//! The VT/xterm screen model.
//!
//! `Term` is deliberately GUI-free and I/O-free: bytes go in through
//! [`Term::process`], state can be inspected through accessors, and anything
//! the outside world must react to (replies for the child, title changes,
//! bells, clipboard writes, notifications…) comes out as [`TermEvent`]s.
//! That makes the whole thing testable without a window or a PTY, and lets
//! the policy layer (see `security`) decide what an application may do.
//!
//! Storage model
//! * `grid`        — the live screen, always exactly `rows` rows of `cols` cells.
//! * `scrollback`  — history rows (trailing blanks trimmed), capped by `scrollback_limit`.
//! * absolute line ids — `dropped + index` into `scrollback ++ grid`. They stay
//!   stable while output scrolls, which is what selection, search hits and
//!   widget anchors are keyed on.
//! * colours are palette references, so a theme change recolours history.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use mitos_utils::ipc::RichWidget;
use vte::Parser;

use crate::theme::Rgb;

mod cell;
mod modes;
mod ops;
mod perform;
mod reflow;
mod search;
mod select;
mod width;

#[cfg(test)]
mod tests;

pub use cell::{attr, mark, Cell, Color, Pen, Row};
pub use modes::{Charset, CharsetState, CursorShape, CursorStyle, Modes, MouseEnc, MouseMode};
pub use search::Hit;
pub use select::{SelMode, SelPoint, Selection, UrlSpan};
pub use width::char_width;

pub const MAX_COLS: usize = 2000;
pub const MAX_ROWS: usize = 1000;
/// Grapheme clusters (base + combining marks) are interned as private-use
/// code points from here up, keeping `Cell` small and `Copy`.
pub const CLUSTER_BASE: u32 = 0x10_0000;
const MAX_CLUSTERS: usize = 60_000;
const MAX_LINKS: usize = 60_000;
const MAX_COMMANDS: usize = 500;
const MAX_TITLE_STACK: usize = 16;
const FRESH_MAX_AGE_MS: u32 = 700;
const FRESH_QUEUE_CAP: usize = 256;
/// Longest OSC payload we will buffer (vte's own limit may be lower).
const MAX_OSC_BYTES: u32 = 256 * 1024;

/// Tracks whether the byte stream is inside an OSC string so a hostile
/// program cannot make us buffer an unterminated one forever.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OscGuard {
    Idle,
    Esc,
    Body,
    Discard,
}

// ---------------------------------------------------------------------------
// Events, policy and small public types
// ---------------------------------------------------------------------------

/// Things the outside world must react to. Drained with [`Term::take_events`].
#[derive(Clone, Debug)]
pub enum TermEvent {
    /// Bytes to write back to the child (DSR/DA/DECRQM/OSC query replies).
    PtyWrite(Vec<u8>),
    Title(String),
    Bell,
    /// OSC 52 write request (already size-checked and decoded).
    ClipboardStore { text: String },
    /// OSC 7 working directory report.
    Cwd(String),
    Notify { title: String, body: String },
    /// OSC 9;4 progress: state 0 = hidden, 1 = normal, 2 = error, 3 = indeterminate, 4 = paused.
    Progress { state: u8, percent: u8 },
    /// OSC 133;C — a command started running.
    CommandStarted { command: String },
    /// OSC 133;D — a command finished.
    CommandFinished { exit: Option<i32>, duration: Duration, command: String },
    /// A MITOS execution block was closed (`OSC MITOS_NEW_BLOCK`).
    BlockClosed { duration: Duration },
    /// A shell "command not found" line was seen — ask mitos-pkgd about it.
    MissingCommand(String),
    /// `OSC MITOS_AUTOCOMPLETE` — ask mitos-file-manager for path suggestions.
    AutocompleteRequest(String),
    /// Palette / default colours were changed by the application.
    ColorsChanged,
    /// A MROP widget's button was clicked (already policy/sanitize-checked);
    /// the session layer writes `command + "\n"` to the child.
    WidgetCommand { id: u64, command: String },
}

/// What an application running inside the terminal is allowed to do.
/// Filled from `[security]` in `terminal.toml`.
#[derive(Clone, Debug)]
pub struct Policy {
    pub title: bool,
    pub clipboard_write: bool,
    pub notifications: bool,
    pub widgets: bool,
    pub cwd: bool,
    pub hyperlinks: bool,
    pub color_changes: bool,
    pub color_queries: bool,
    pub max_title: usize,
    pub max_uri: usize,
    pub max_clipboard: usize,
    pub max_widgets: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            title: true,
            clipboard_write: true,
            notifications: true,
            widgets: true,
            cwd: true,
            hyperlinks: true,
            color_changes: true,
            color_queries: true,
            max_title: 256,
            max_uri: 2048,
            max_clipboard: 100_000,
            max_widgets: 256,
        }
    }
}

/// Colours the application changed at runtime (OSC 4/10/11/12).
#[derive(Clone, Debug)]
pub struct Overrides {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub palette: Vec<Option<Rgb>>,
}

impl Default for Overrides {
    fn default() -> Self {
        Overrides { fg: None, bg: None, cursor: None, palette: vec![None; 256] }
    }
}

/// A MROP widget anchored to an absolute line.
#[derive(Clone)]
pub struct Widget {
    pub id: u64,
    pub line: u64,
    /// `true` when injected by the terminal itself or an authenticated local
    /// daemon; `false` when it arrived as an escape sequence from the child.
    pub trusted: bool,
    pub widget: RichWidget,
}

#[derive(Clone, Debug)]
pub struct CommandRecord {
    pub id: u32,
    pub command: String,
    pub cwd: Option<String>,
    pub exit: Option<i32>,
    pub started: Instant,
    pub duration: Option<Duration>,
    pub prompt_line: u64,
}

/// A run of freshly printed cells (drives the phosphor glow / error glitch).
#[derive(Clone, Copy, Debug)]
pub struct Fresh {
    pub line: u64,
    pub c0: u16,
    pub c1: u16,
    pub t: u32,
    pub error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    None,
    Full,
    Rows(Vec<usize>),
}

#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    pub x: usize,
    pub y: usize,
    pub pen: Pen,
    pub wrap_pending: bool,
    pub origin: bool,
    pub charset: CharsetState,
}

impl Cursor {
    fn home() -> Cursor {
        Cursor {
            x: 0,
            y: 0,
            pen: Pen::default(),
            wrap_pending: false,
            origin: false,
            charset: CharsetState::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Term
// ---------------------------------------------------------------------------

pub struct Term {
    cols: usize,
    rows: usize,

    grid: Vec<Row>,
    /// The inactive screen (the primary grid while the alternate is shown).
    stash: Vec<Row>,
    in_alt: bool,
    primary_cursor: Cursor,
    scrollback: VecDeque<Row>,
    pub scrollback_limit: usize,
    /// Characters that delimit words for double-click selection.
    pub word_separators: String,
    dropped: u64,
    /// Lines scrolled back from the bottom (0 = live view).
    pub display_offset: usize,

    cursor: Cursor,
    saved_cursor: [Option<Cursor>; 2],
    scroll_top: usize,
    scroll_bottom: usize,
    tabs: Vec<bool>,
    pub modes: Modes,
    cursor_style: CursorStyle,
    default_cursor_style: CursorStyle,
    saved_modes: HashMap<u16, bool>,

    cur_link: u16,
    last_char: Option<char>,
    title: String,
    title_stack: Vec<String>,
    cwd: Option<String>,

    links: Vec<String>,
    link_map: HashMap<String, u16>,
    clusters: Vec<String>,
    cluster_map: HashMap<String, u32>,
    pub overrides: Overrides,
    theme_fg: Rgb,
    theme_bg: Rgb,
    theme_cursor: Rgb,
    theme_ansi: [Rgb; 16],

    parser: Option<Parser>,
    osc_state: OscGuard,
    osc_len: u32,
    events: Vec<TermEvent>,

    pub selection: Option<Selection>,

    seqno: u64,
    changed: bool,
    dirty: Vec<bool>,
    dirty_all: bool,
    last_dirty_row: usize,

    pub policy: Policy,
    ghost_text: Option<String>,
    widgets: Vec<Widget>,
    next_widget_id: u64,
    commands: VecDeque<CommandRecord>,
    next_cmd_id: u32,
    cmd_input_start: Option<(u64, usize)>,
    open_cmd: Option<u32>,
    pending_prompt_tag: bool,
    block_started: Option<Instant>,
    block_prompt: String,
    scan_budget: u8,
    already_suggested: HashSet<String>,
    scratch: String,

    epoch: Instant,
    now_ms: u32,
    fresh: VecDeque<Fresh>,
    /// Record freshly printed spans for the phosphor-glow effect.
    pub glow: bool,
    pub bytes_processed: u64,
}

impl Term {
    pub fn new(cols: usize, rows: usize, scrollback_limit: usize) -> Term {
        let cols = cols.clamp(1, MAX_COLS);
        let rows = rows.clamp(1, MAX_ROWS);
        let style = CursorStyle { shape: CursorShape::Block, blink: true };
        Term {
            cols,
            rows,
            grid: (0..rows).map(|_| Row::new(cols)).collect(),
            stash: Vec::new(),
            in_alt: false,
            primary_cursor: Cursor::home(),
            scrollback: VecDeque::new(),
            scrollback_limit,
            word_separators: " \t\"'`|()[]{}<>,;".to_string(),
            dropped: 0,
            display_offset: 0,
            cursor: Cursor::home(),
            saved_cursor: [None, None],
            scroll_top: 0,
            scroll_bottom: rows - 1,
            tabs: default_tabs(cols),
            modes: Modes::default(),
            cursor_style: style,
            default_cursor_style: style,
            saved_modes: HashMap::new(),
            cur_link: 0,
            last_char: None,
            title: String::new(),
            title_stack: Vec::new(),
            cwd: None,
            links: Vec::new(),
            link_map: HashMap::new(),
            clusters: Vec::new(),
            cluster_map: HashMap::new(),
            overrides: Overrides::default(),
            theme_fg: [200, 200, 200],
            theme_bg: [4, 10, 18],
            theme_cursor: [85, 255, 85],
            theme_ansi: crate::theme::Theme::dark().ansi,
            parser: Some(Parser::new()),
            osc_state: OscGuard::Idle,
            osc_len: 0,
            events: Vec::new(),
            selection: None,
            seqno: 1,
            changed: false,
            dirty: vec![true; rows],
            dirty_all: true,
            last_dirty_row: usize::MAX,
            policy: Policy::default(),
            ghost_text: None,
            widgets: Vec::new(),
            next_widget_id: 1,
            commands: VecDeque::new(),
            next_cmd_id: 1,
            cmd_input_start: None,
            open_cmd: None,
            pending_prompt_tag: false,
            block_started: None,
            block_prompt: String::new(),
            scan_budget: 0,
            already_suggested: HashSet::new(),
            scratch: String::new(),
            epoch: Instant::now(),
            now_ms: 0,
            fresh: VecDeque::new(),
            glow: true,
            bytes_processed: 0,
        }
    }

    // ----- input ----------------------------------------------------------

    /// Feed bytes from the child. This is the hot path.
    pub fn process(&mut self, bytes: &[u8]) {
        let mut parser = self.parser.take().unwrap_or_else(Parser::new);
        self.now_ms = self.epoch.elapsed().as_millis().min(u32::MAX as u128) as u32;
        self.prune_fresh();

        let n = bytes.len();
        let mut i = 0usize;
        while i < n {
            if self.osc_state == OscGuard::Idle {
                // Fast path: hand everything up to and including the next ESC to the parser.
                let end = match bytes[i..].iter().position(|&b| b == 0x1b) {
                    Some(p) => i + p + 1,
                    None => n,
                };
                for &b in &bytes[i..end] {
                    parser.advance(self, b);
                }
                if bytes[end - 1] == 0x1b {
                    self.osc_state = OscGuard::Esc;
                }
                i = end;
                continue;
            }
            let b = bytes[i];
            i += 1;
            match self.osc_state {
                OscGuard::Esc => {
                    self.osc_state = match b {
                        b']' => {
                            self.osc_len = 0;
                            OscGuard::Body
                        }
                        0x1b => OscGuard::Esc,
                        _ => OscGuard::Idle,
                    };
                    parser.advance(self, b);
                }
                OscGuard::Body => match b {
                    0x07 | 0x18 | 0x1a => {
                        self.osc_state = OscGuard::Idle;
                        parser.advance(self, b);
                    }
                    0x1b => {
                        self.osc_state = OscGuard::Esc;
                        parser.advance(self, b);
                    }
                    _ => {
                        self.osc_len += 1;
                        if self.osc_len > MAX_OSC_BYTES {
                            // Cancel the runaway OSC (CAN) and swallow the rest of it.
                            parser.advance(self, 0x18);
                            self.osc_state = OscGuard::Discard;
                        } else {
                            parser.advance(self, b);
                        }
                    }
                },
                OscGuard::Discard => match b {
                    0x07 | 0x18 | 0x1a => self.osc_state = OscGuard::Idle,
                    0x1b => {
                        self.osc_state = OscGuard::Esc;
                        parser.advance(self, b);
                    }
                    _ => {}
                },
                OscGuard::Idle => {}
            }
        }

        self.parser = Some(parser);
        self.bytes_processed = self.bytes_processed.wrapping_add(n as u64);
        if self.changed {
            self.seqno = self.seqno.wrapping_add(1);
            self.changed = false;
        }
    }

    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut self.events)
    }

    // ----- simple accessors ----------------------------------------------

    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn seqno(&self) -> u64 {
        self.seqno
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }
    pub fn in_alt_screen(&self) -> bool {
        self.in_alt
    }
    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.cursor.x, self.cursor.y)
    }
    pub fn cursor_wrap_pending(&self) -> bool {
        self.cursor.wrap_pending
    }
    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }
    pub fn set_default_cursor_style(&mut self, style: CursorStyle) {
        self.default_cursor_style = style;
        self.cursor_style = style;
        self.mark_all_dirty();
    }
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn ghost_text(&self) -> Option<&str> {
        self.ghost_text.as_deref()
    }
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }
    pub fn commands(&self) -> &VecDeque<CommandRecord> {
        &self.commands
    }
    pub fn block_prompt(&self) -> &str {
        &self.block_prompt
    }
    pub fn fresh(&self) -> &VecDeque<Fresh> {
        &self.fresh
    }
    pub fn now_ms(&self) -> u32 {
        self.epoch.elapsed().as_millis().min(u32::MAX as u128) as u32
    }
    pub fn link_uri(&self, id: u16) -> Option<&str> {
        if id == 0 {
            None
        } else {
            self.links.get(id as usize - 1).map(|s| s.as_str())
        }
    }
    /// The full text of an interned grapheme cluster, if `ch` is one.
    pub fn cluster_str(&self, ch: char) -> Option<&str> {
        let u = ch as u32;
        if u >= CLUSTER_BASE {
            self.clusters.get((u - CLUSTER_BASE) as usize).map(|s| s.as_str())
        } else {
            None
        }
    }
    pub fn set_ghost_text(&mut self, text: Option<String>) {
        self.ghost_text = text;
        self.changed = true;
    }
    /// Called when the user presses Enter: the next few lines are scanned for
    /// a "command not found" message (keeps the per-line cost at zero otherwise).
    pub fn arm_command_scan(&mut self) {
        self.scan_budget = 4;
    }

    // ----- view / addressing ---------------------------------------------

    /// Absolute id of screen row 0.
    pub fn screen_top_abs(&self) -> u64 {
        self.dropped + self.scrollback.len() as u64
    }

    /// Index (into `scrollback ++ grid`) of the first row of the current view.
    pub fn view_top_combined(&self) -> usize {
        self.scrollback.len().saturating_sub(self.display_offset)
    }

    pub fn combined_len(&self) -> usize {
        self.scrollback.len() + self.rows
    }

    pub fn combined_row(&self, idx: usize) -> &Row {
        let sb = self.scrollback.len();
        if idx < sb {
            &self.scrollback[idx]
        } else {
            &self.grid[(idx - sb).min(self.rows - 1)]
        }
    }

    /// Row `r` (0..rows) of the current view (honours `display_offset`).
    pub fn view_row(&self, r: usize) -> &Row {
        self.combined_row(self.view_top_combined() + r)
    }

    pub fn view_abs(&self, r: usize) -> u64 {
        self.dropped + (self.view_top_combined() + r) as u64
    }

    pub fn row_by_abs(&self, abs: u64) -> Option<&Row> {
        if abs < self.dropped {
            return None;
        }
        let idx = (abs - self.dropped) as usize;
        let sb = self.scrollback.len();
        if idx < sb {
            self.scrollback.get(idx)
        } else if idx - sb < self.rows {
            self.grid.get(idx - sb)
        } else {
            None
        }
    }

    /// Scroll the view by `delta` lines (positive = towards older output).
    /// Returns `true` if the offset changed.
    pub fn scroll_display(&mut self, delta: isize) -> bool {
        let max = self.scrollback.len() as isize;
        let new = (self.display_offset as isize + delta).clamp(0, max) as usize;
        let changed = new != self.display_offset;
        self.display_offset = new;
        if changed {
            self.mark_all_dirty();
        }
        changed
    }

    pub fn scroll_to_bottom(&mut self) {
        if self.display_offset != 0 {
            self.display_offset = 0;
            self.mark_all_dirty();
        }
    }

    /// Scroll so absolute line `abs` is visible (roughly centred).
    pub fn scroll_to_abs(&mut self, abs: u64) {
        if abs < self.dropped {
            return;
        }
        let idx = (abs - self.dropped) as usize;
        let sb = self.scrollback.len();
        if idx >= sb {
            self.scroll_to_bottom();
            return;
        }
        let from_bottom = sb - idx; // rows between the line and the live screen
        let target = from_bottom + self.rows / 2;
        self.display_offset = target.min(sb);
        self.mark_all_dirty();
    }

    // ----- damage ---------------------------------------------------------

    #[inline]
    pub(crate) fn mark_dirty(&mut self, y: usize) {
        self.changed = true;
        if y != self.last_dirty_row && y < self.dirty.len() {
            self.dirty[y] = true;
            self.last_dirty_row = y;
        }
    }

    pub(crate) fn mark_all_dirty(&mut self) {
        self.changed = true;
        self.dirty_all = true;
    }

    /// Rows changed since the last call (screen rows, not view rows).
    pub fn take_damage(&mut self) -> Damage {
        self.last_dirty_row = usize::MAX;
        if self.dirty_all {
            self.dirty_all = false;
            for d in self.dirty.iter_mut() {
                *d = false;
            }
            return Damage::Full;
        }
        let rows: Vec<usize> =
            self.dirty.iter().enumerate().filter(|(_, d)| **d).map(|(i, _)| i).collect();
        for d in self.dirty.iter_mut() {
            *d = false;
        }
        if rows.is_empty() {
            Damage::None
        } else {
            Damage::Rows(rows)
        }
    }

    // ----- text helpers ---------------------------------------------------

    pub(crate) fn push_cell_text(&self, cell: &Cell, out: &mut String) {
        if cell.flags & attr::WIDE_TAIL != 0 {
            return;
        }
        if let Some(s) = self.cluster_str(cell.ch) {
            out.push_str(s);
        } else {
            out.push(cell.ch);
        }
    }

    /// Text of one row, trailing blanks trimmed.
    pub(crate) fn row_string(&self, row: &Row) -> String {
        let mut s = String::with_capacity(row.cells.len());
        for c in &row.cells {
            self.push_cell_text(c, &mut s);
        }
        let trimmed = s.trim_end_matches(' ').len();
        s.truncate(trimmed);
        s
    }

    /// Text of screen row `y` (test / accessibility helper).
    pub fn screen_row_text(&self, y: usize) -> String {
        self.grid.get(y).map(|r| self.row_string(r)).unwrap_or_default()
    }

    /// The live screen as text, one line per row.
    pub fn screen_text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.rows {
            out.push_str(&self.screen_row_text(y));
            out.push('\n');
        }
        out
    }

    /// Text of the *current view* (honours scrollback offset).
    pub fn visible_text(&self) -> String {
        let mut out = String::new();
        for r in 0..self.rows {
            out.push_str(&self.row_string(self.view_row(r)));
            out.push('\n');
        }
        out
    }

    /// Last `max_lines` lines of history + screen as plain text
    /// (soft-wrapped rows are joined).
    pub fn snapshot_text(&self, max_lines: usize) -> String {
        let total = self.combined_len();
        // ignore trailing blank rows of the live screen
        let mut end = total;
        while end > self.scrollback.len() {
            if self.row_string(self.combined_row(end - 1)).is_empty() {
                end -= 1;
            } else {
                break;
            }
        }
        let start = end.saturating_sub(max_lines);
        let mut out = String::new();
        for i in start..end {
            let row = self.combined_row(i);
            let s = self.row_string(row);
            if row.wrapped {
                // keep interior spaces of full-width rows
                let mut full = String::new();
                for c in &row.cells {
                    self.push_cell_text(c, &mut full);
                }
                out.push_str(&full);
            } else {
                out.push_str(&s);
                out.push('\n');
            }
        }
        out
    }

    // ----- prompt marks (OSC 133) ----------------------------------------

    /// Absolute line ids of every row marked as a shell prompt, oldest first.
    pub fn prompt_lines(&self) -> Vec<u64> {
        let mut v = Vec::new();
        let total = self.combined_len();
        for i in 0..total {
            if self.combined_row(i).marks & mark::PROMPT != 0 {
                v.push(self.dropped + i as u64);
            }
        }
        v
    }

    /// Nearest prompt above (`backwards`) or below `from_abs`.
    pub fn jump_prompt(&self, from_abs: u64, backwards: bool) -> Option<u64> {
        let lines = self.prompt_lines();
        if backwards {
            lines.into_iter().rev().find(|l| *l < from_abs)
        } else {
            lines.into_iter().find(|l| *l > from_abs)
        }
    }

    // ----- widgets --------------------------------------------------------

    /// Anchor a MROP widget on its own row at the cursor. `trusted` widgets
    /// come from the terminal itself or an authenticated local daemon.
    pub fn inject_widget(&mut self, widget: RichWidget, trusted: bool) -> bool {
        if !trusted && !self.policy.widgets {
            return false;
        }
        while self.widgets.len() >= self.policy.max_widgets.max(1) {
            self.widgets.remove(0);
        }
        if self.cursor.x != 0 || self.cursor.wrap_pending {
            self.carriage_return();
            self.linefeed();
        }
        let y = self.cursor.y;
        self.grid[y].marks |= mark::WIDGET;
        let line = self.screen_top_abs() + y as u64;
        let id = self.next_widget_id;
        self.next_widget_id += 1;
        self.widgets.push(Widget { id, line, trusted, widget });
        self.mark_dirty(y);
        self.linefeed();
        true
    }

    pub fn remove_widget(&mut self, id: u64) {
        self.widgets.retain(|w| w.id != id);
        self.mark_all_dirty();
    }

    /// Called by the renderer when a widget's button is clicked.
    pub fn queue_widget_command(&mut self, id: u64, command: String) {
        self.events.push(TermEvent::WidgetCommand { id, command });
    }

    // ----- fx (phosphor glow) --------------------------------------------

    fn prune_fresh(&mut self) {
        let now = self.now_ms;
        while let Some(f) = self.fresh.front() {
            if f.t.saturating_add(FRESH_MAX_AGE_MS) < now {
                self.fresh.pop_front();
            } else {
                break;
            }
        }
    }

    pub(crate) fn note_fresh(&mut self, x: usize, y: usize, w: usize) {
        let line = self.screen_top_abs() + y as u64;
        let (x0, x1) = (x as u16, (x + w) as u16);
        if let Some(last) = self.fresh.back_mut() {
            if !last.error && last.line == line && last.c1 == x0 && last.t == self.now_ms {
                last.c1 = x1;
                return;
            }
        }
        if self.fresh.len() >= FRESH_QUEUE_CAP {
            self.fresh.pop_front();
        }
        self.fresh.push_back(Fresh { line, c0: x0, c1: x1, t: self.now_ms, error: false });
    }

    /// Flag the text on screen row `y` as an error (red glitch effect).
    pub(crate) fn spike_error_row(&mut self, y: usize) {
        let end = match self.grid.get(y) {
            Some(row) => row.cells.iter().rposition(|c| !c.is_blank()).map(|i| i + 1),
            None => None,
        };
        if let Some(end) = end {
            let line = self.screen_top_abs() + y as u64;
            if self.fresh.len() >= FRESH_QUEUE_CAP {
                self.fresh.pop_front();
            }
            self.fresh.push_back(Fresh { line, c0: 0, c1: end as u16, t: self.now_ms, error: true });
        }
    }

    // ----- interned tables ------------------------------------------------

    pub(crate) fn intern_link(&mut self, uri: &str) -> u16 {
        if let Some(&id) = self.link_map.get(uri) {
            return id;
        }
        if self.links.len() >= MAX_LINKS {
            self.compact_links();
            if self.links.len() >= MAX_LINKS {
                return 0;
            }
        }
        self.links.push(uri.to_string());
        let id = self.links.len() as u16;
        self.link_map.insert(uri.to_string(), id);
        id
    }

    /// Drop link-table entries no cell references any more.
    fn compact_links(&mut self) {
        let mut used = vec![false; self.links.len() + 1];
        let mut mark_row = |row: &Row, used: &mut Vec<bool>| {
            for c in &row.cells {
                if c.link != 0 && (c.link as usize) < used.len() {
                    used[c.link as usize] = true;
                }
            }
        };
        for r in self.scrollback.iter() {
            mark_row(r, &mut used);
        }
        for r in self.grid.iter() {
            mark_row(r, &mut used);
        }
        for r in self.stash.iter() {
            mark_row(r, &mut used);
        }
        if self.cur_link != 0 && (self.cur_link as usize) < used.len() {
            used[self.cur_link as usize] = true;
        }
        let mut remap = vec![0u16; self.links.len() + 1];
        let mut new_links: Vec<String> = Vec::new();
        for (old, u) in used.iter().enumerate().skip(1) {
            if *u {
                new_links.push(self.links[old - 1].clone());
                remap[old] = new_links.len() as u16;
            }
        }
        let fix = |row: &mut Row, remap: &Vec<u16>| {
            for c in row.cells.iter_mut() {
                if c.link != 0 {
                    c.link = remap.get(c.link as usize).copied().unwrap_or(0);
                }
            }
        };
        for r in self.scrollback.iter_mut() {
            fix(r, &remap);
        }
        for r in self.grid.iter_mut() {
            fix(r, &remap);
        }
        for r in self.stash.iter_mut() {
            fix(r, &remap);
        }
        self.cur_link = remap.get(self.cur_link as usize).copied().unwrap_or(0);
        self.link_map.clear();
        for (i, l) in new_links.iter().enumerate() {
            self.link_map.insert(l.clone(), (i + 1) as u16);
        }
        self.links = new_links;
    }

    pub(crate) fn intern_cluster(&mut self, s: &str) -> Option<char> {
        if let Some(&id) = self.cluster_map.get(s) {
            return char::from_u32(CLUSTER_BASE + id);
        }
        if self.clusters.len() >= MAX_CLUSTERS {
            return None;
        }
        let id = self.clusters.len() as u32;
        self.clusters.push(s.to_string());
        self.cluster_map.insert(s.to_string(), id);
        char::from_u32(CLUSTER_BASE + id)
    }

    // ----- title ----------------------------------------------------------

    pub(crate) fn set_title(&mut self, t: &str) {
        if !self.policy.title {
            return;
        }
        let clean: String = t
            .chars()
            .filter(|c| !c.is_control())
            .take(self.policy.max_title)
            .collect();
        if clean != self.title {
            self.title = clean.clone();
            self.events.push(TermEvent::Title(clean));
        }
    }

    pub(crate) fn push_title(&mut self) {
        if self.title_stack.len() < MAX_TITLE_STACK {
            self.title_stack.push(self.title.clone());
        }
    }

    pub(crate) fn pop_title(&mut self) {
        if let Some(t) = self.title_stack.pop() {
            self.set_title(&t);
        }
    }

    pub(crate) fn reply(&mut self, bytes: Vec<u8>) {
        self.events.push(TermEvent::PtyWrite(bytes));
    }

    /// Recompute palette-derived state after the policy changed.
    pub fn set_policy(&mut self, p: Policy) {
        self.policy = p;
    }

    /// Reset the interpreter (`RIS`, `reset`, "Reset terminal" menu item).
    pub fn reset(&mut self) {
        self.leave_alt_discard();
        let blank = Cell::BLANK;
        for row in self.grid.iter_mut() {
            row.reset(blank);
        }
        self.cursor = Cursor::home();
        self.saved_cursor = [None, None];
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.tabs = default_tabs(self.cols);
        self.modes = Modes::default();
        self.cursor_style = self.default_cursor_style;
        self.cur_link = 0;
        self.last_char = None;
        self.overrides = Overrides::default();
        self.selection = None;
        self.ghost_text = None;
        self.display_offset = 0;
        self.mark_all_dirty();
        self.events.push(TermEvent::ColorsChanged);
    }

    /// Erase history but keep the visible screen (abs ids of the screen stay valid).
    pub fn clear_scrollback(&mut self) {
        self.dropped += self.scrollback.len() as u64;
        self.scrollback.clear();
        self.display_offset = 0;
        self.selection = None;
        let d = self.dropped;
        self.widgets.retain(|w| w.line >= d);
        self.mark_all_dirty();
    }
}

pub(crate) fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c % 8 == 0 && c != 0).collect()
}
