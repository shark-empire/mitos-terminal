//! Escape-sequence interpretation (`vte::Perform`).
//!
//! Security notes (see also `security.rs`):
//! * There is deliberately **no** title report (CSI 21 t), no answerback
//!   (ENQ), no DECRQSS and no window-resize/move sequences: those are the
//!   classic "make the terminal type attacker-controlled text into the shell"
//!   primitives.
//! * OSC 52 can *write* the clipboard (size-capped, policy-gated) but a read
//!   request (`?`) is never answered.
//! * Every OSC that reaches the outside world goes through `Policy`.

use std::time::Instant;

use vte::{Params, Perform};

use super::*;
use crate::theme::{xterm_extended, Rgb};

const VERSION: &str = env!("CARGO_PKG_VERSION");

impl Perform for Term {
    fn print(&mut self, c: char) {
        if self.cursor.charset.is_plain() {
            self.put_char(c);
        } else {
            let m = self.cursor.charset.map(c);
            self.put_char(m);
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {
                if self.events.len() < 4096 {
                    self.events.push(TermEvent::Bell);
                }
            }
            0x08 => {
                self.move_left(1);
                self.ghost_text = None;
            }
            0x09 => self.tab_forward(1),
            0x0A | 0x0B | 0x0C => self.linefeed(),
            0x0D => self.carriage_return(),
            0x0E => self.cursor.charset.gl = 1,
            0x0F => self.cursor.charset.gl = 0,
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([b'#'], b'8') => self.screen_alignment(),
            ([], b'D') => self.index_down(),
            ([], b'E') => {
                self.carriage_return();
                self.index_down();
            }
            ([], b'H') => {
                let x = self.cursor.x;
                self.tabs[x] = true;
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([], b'n') => self.cursor.charset.gl = 2,
            ([], b'o') => self.cursor.charset.gl = 3,
            ([b'('], f) => self.designate(0, f),
            ([b')'], f) => self.designate(1, f),
            ([b'*'], f) => self.designate(2, f),
            ([b'+'], f) => self.designate(3, f),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        // First sub-parameter of every parameter, without allocating.
        let mut ps = [0u16; 32];
        let mut n = 0usize;
        for p in params.iter() {
            if n < 32 {
                ps[n] = p.first().copied().unwrap_or(0);
                n += 1;
            }
        }
        // Parameter `i`, with 0/absent replaced by `def`.
        let get = |i: usize, def: usize| -> usize {
            if i < n && ps[i] != 0 {
                ps[i] as usize
            } else {
                def
            }
        };

        match (intermediates, action) {
            ([], '@') => self.insert_cells(get(0, 1)),
            ([], 'A') => self.move_up(get(0, 1)),
            ([], 'B') | ([], 'e') => self.move_down(get(0, 1)),
            ([], 'C') | ([], 'a') => self.move_right(get(0, 1)),
            ([], 'D') => self.move_left(get(0, 1)),
            ([], 'E') => {
                self.move_down(get(0, 1));
                self.set_col(0);
            }
            ([], 'F') => {
                self.move_up(get(0, 1));
                self.set_col(0);
            }
            ([], 'G') | ([], '`') => self.set_col(get(0, 1) - 1),
            ([], 'H') | ([], 'f') => self.goto(get(0, 1) - 1, get(1, 1) - 1),
            ([], 'I') => self.tab_forward(get(0, 1)),
            ([], 'J') | ([b'?'], 'J') => self.erase_display(ps[0]),
            ([], 'K') | ([b'?'], 'K') => self.erase_line(ps[0]),
            ([], 'L') => self.insert_lines(get(0, 1)),
            ([], 'M') => self.delete_lines(get(0, 1)),
            ([], 'P') => self.delete_cells(get(0, 1)),
            ([], 'S') => {
                let (t, b) = (self.scroll_top, self.scroll_bottom);
                self.scroll_up_region(t, b, get(0, 1), true);
            }
            ([], 'T') => {
                // `CSI Pn T` scrolls down; the 5-parameter form is mouse-tracking init: ignore.
                if n <= 1 {
                    let (t, b) = (self.scroll_top, self.scroll_bottom);
                    self.scroll_down_region(t, b, get(0, 1));
                }
            }
            ([], 'X') => self.erase_chars(get(0, 1)),
            ([], 'Z') => self.tab_backward(get(0, 1)),
            ([], 'b') => self.repeat_last_char(get(0, 1)),
            ([], 'c') => {
                if ps[0] == 0 {
                    self.reply(b"\x1b[?62;22c".to_vec());
                }
            }
            ([b'>'], 'c') => {
                if ps[0] == 0 {
                    self.reply(b"\x1b[>1;1000;0c".to_vec());
                }
            }
            ([], 'd') => self.set_row(get(0, 1) - 1),
            ([], 'g') => match ps[0] {
                0 => {
                    let x = self.cursor.x;
                    self.tabs[x] = false;
                }
                3 => {
                    for t in self.tabs.iter_mut() {
                        *t = false;
                    }
                }
                _ => {}
            },
            ([], 'h') => {
                for i in 0..n {
                    self.set_ansi_mode(ps[i], true);
                }
            }
            ([], 'l') => {
                for i in 0..n {
                    self.set_ansi_mode(ps[i], false);
                }
            }
            ([b'?'], 'h') => {
                for i in 0..n {
                    self.set_private_mode(ps[i], true);
                }
            }
            ([b'?'], 'l') => {
                for i in 0..n {
                    self.set_private_mode(ps[i], false);
                }
            }
            ([b'?'], 's') => {
                for i in 0..n {
                    if let Some(v) = self.private_mode_state(ps[i]) {
                        self.saved_modes.insert(ps[i], v);
                    }
                }
            }
            ([b'?'], 'r') => {
                for i in 0..n {
                    if let Some(v) = self.saved_modes.get(&ps[i]).copied() {
                        self.set_private_mode(ps[i], v);
                    }
                }
            }
            ([], 'm') => self.sgr(params),
            ([], 'n') => self.device_status(ps[0]),
            ([], 'r') => {
                let top = get(0, 1) - 1;
                let bottom = get(1, self.rows).min(self.rows) - 1;
                self.set_scroll_region(top, bottom);
            }
            ([], 's') => {
                if n == 0 {
                    self.save_cursor();
                }
            }
            ([], 'u') => self.restore_cursor(),
            ([], 't') => self.window_ops(&ps[..n]),
            ([b' '], 'q') => self.set_cursor_style(ps[0]),
            ([b'!'], 'p') => self.soft_reset(),
            ([b'>'], 'q') => {
                let s = format!("\x1bP>|mitos-terminal({})\x1b\\", VERSION);
                self.reply(s.into_bytes());
            }
            ([b'?'], 'u') => self.reply(b"\x1b[?0u".to_vec()),
            ([b'$'], 'p') => {
                let state = match ps[0] {
                    4 => Some(self.modes.insert),
                    20 => Some(self.modes.lnm),
                    _ => None,
                };
                let pm = match state {
                    Some(true) => 1,
                    Some(false) => 2,
                    None => 0,
                };
                self.reply(format!("\x1b[{};{}$y", ps[0], pm).into_bytes());
            }
            ([b'?', b'$'], 'p') => {
                let pm = match self.private_mode_state(ps[0]) {
                    Some(true) => 1,
                    Some(false) => 2,
                    None => 0,
                };
                self.reply(format!("\x1b[?{};{}$y", ps[0], pm).into_bytes());
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        let term: &'static [u8] = if bell_terminated { b"\x07" } else { b"\x1b\\" };
        let cmd = params[0];

        match cmd {
            b"0" | b"2" => {
                let t = lossy(&join_params(params, 1));
                self.set_title(&t);
            }
            b"1" => {}
            b"4" => self.osc_palette(params, term),
            b"104" => {
                if !self.policy.color_changes {
                    return;
                }
                if params.len() <= 1 {
                    for p in self.overrides.palette.iter_mut() {
                        *p = None;
                    }
                } else {
                    for p in &params[1..] {
                        if let Some(i) = std::str::from_utf8(p).ok().and_then(|s| s.parse::<usize>().ok()) {
                            if i < 256 {
                                self.overrides.palette[i] = None;
                            }
                        }
                    }
                }
                self.colors_changed();
            }
            b"10" | b"11" | b"12" => {
                let first = match cmd {
                    b"10" => 0usize,
                    b"11" => 1,
                    _ => 2,
                };
                for (k, spec) in params[1..].iter().enumerate() {
                    let target = first + k;
                    if target > 2 {
                        break;
                    }
                    self.osc_dynamic_color(target, spec, term);
                }
            }
            b"110" | b"111" | b"112" => {
                if self.policy.color_changes {
                    match cmd {
                        b"110" => self.overrides.fg = None,
                        b"111" => self.overrides.bg = None,
                        _ => self.overrides.cursor = None,
                    }
                    self.colors_changed();
                }
            }
            b"7" => {
                if !self.policy.cwd || params.len() < 2 {
                    return;
                }
                let uri = lossy(&join_params(params, 1));
                if let Some(path) = parse_file_uri(&uri) {
                    self.cwd = Some(path.clone());
                    self.events.push(TermEvent::Cwd(path));
                }
            }
            b"8" => {
                if params.len() < 3 {
                    return;
                }
                let uri_bytes = join_params(params, 2);
                let uri = lossy(&uri_bytes);
                if uri.is_empty() {
                    self.cur_link = 0;
                } else if !self.policy.hyperlinks
                    || uri.len() > self.policy.max_uri
                    || uri.chars().any(|c| c.is_control())
                {
                    self.cur_link = 0;
                } else {
                    self.cur_link = self.intern_link(&uri);
                }
            }
            b"52" => {
                if params.len() < 3 || !self.policy.clipboard_write {
                    return;
                }
                let data = join_params(params, 2);
                // A read request (`?`) is never answered: that would leak the clipboard.
                if data == b"?" {
                    return;
                }
                if data.len() / 4 * 3 > self.policy.max_clipboard {
                    return;
                }
                if let Some(bytes) = base64_decode(&data) {
                    if bytes.len() <= self.policy.max_clipboard {
                        let text = String::from_utf8_lossy(&bytes).replace('\0', "");
                        self.events.push(TermEvent::ClipboardStore { text });
                    }
                }
            }
            b"9" => self.osc_9(params),
            b"777" => {
                if self.policy.notifications
                    && params.len() >= 3
                    && params[1] == b"notify"
                {
                    let title = clean_text(&lossy(params[2]), 96);
                    let body = if params.len() > 3 { clean_text(&lossy(&join_params(params, 3)), 300) } else { String::new() };
                    self.events.push(TermEvent::Notify { title, body });
                }
            }
            b"133" => self.osc_133(params),
            _ => self.osc_mitos(params),
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

impl Term {
    fn designate(&mut self, slot: usize, final_byte: u8) {
        self.cursor.charset.g[slot] = match final_byte {
            b'0' => Charset::DecGraphics,
            b'A' => Charset::Uk,
            _ => Charset::Ascii,
        };
    }

    fn set_ansi_mode(&mut self, mode: u16, on: bool) {
        match mode {
            4 => self.modes.insert = on,
            20 => self.modes.lnm = on,
            _ => {}
        }
    }

    pub(super) fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.app_cursor = on,
            5 => {
                self.modes.reverse_video = on;
                self.mark_all_dirty();
            }
            6 => {
                self.modes.origin = on;
                self.goto(0, 0);
            }
            7 => self.modes.autowrap = on,
            9 => self.modes.mouse = if on { MouseMode::X10 } else { MouseMode::Off },
            12 => {
                self.cursor_style.blink = on;
                self.mark_all_dirty();
            }
            25 => {
                self.modes.cursor_visible = on;
                self.mark_all_dirty();
            }
            47 | 1047 => {
                if on {
                    self.enter_alt();
                } else {
                    self.leave_alt(false);
                }
            }
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    self.enter_alt();
                } else {
                    self.leave_alt(true);
                }
            }
            1000 => self.modes.mouse = if on { MouseMode::Normal } else { MouseMode::Off },
            1002 => self.modes.mouse = if on { MouseMode::Button } else { MouseMode::Off },
            1003 => self.modes.mouse = if on { MouseMode::Any } else { MouseMode::Off },
            1004 => self.modes.focus_events = on,
            1005 => self.modes.mouse_enc = if on { MouseEnc::Utf8 } else { MouseEnc::Default },
            1006 => self.modes.mouse_enc = if on { MouseEnc::Sgr } else { MouseEnc::Default },
            1015 => self.modes.mouse_enc = if on { MouseEnc::Urxvt } else { MouseEnc::Default },
            1007 => self.modes.alt_scroll = on,
            2004 => self.modes.bracketed_paste = on,
            2026 => {
                self.modes.sync_output = on;
                if !on {
                    self.mark_all_dirty();
                }
            }
            _ => {}
        }
    }

    pub(super) fn private_mode_state(&self, mode: u16) -> Option<bool> {
        let m = &self.modes;
        Some(match mode {
            1 => m.app_cursor,
            5 => m.reverse_video,
            6 => m.origin,
            7 => m.autowrap,
            9 => m.mouse == MouseMode::X10,
            12 => self.cursor_style.blink,
            25 => m.cursor_visible,
            47 | 1047 | 1049 => self.in_alt,
            1000 => m.mouse == MouseMode::Normal,
            1002 => m.mouse == MouseMode::Button,
            1003 => m.mouse == MouseMode::Any,
            1004 => m.focus_events,
            1005 => m.mouse_enc == MouseEnc::Utf8,
            1006 => m.mouse_enc == MouseEnc::Sgr,
            1015 => m.mouse_enc == MouseEnc::Urxvt,
            1007 => m.alt_scroll,
            2004 => m.bracketed_paste,
            2026 => m.sync_output,
            _ => return None,
        })
    }

    fn set_cursor_style(&mut self, ps: u16) {
        let (shape, blink) = match ps {
            0 => (self.default_cursor_style.shape, self.default_cursor_style.blink),
            1 => (CursorShape::Block, true),
            2 => (CursorShape::Block, false),
            3 => (CursorShape::Underline, true),
            4 => (CursorShape::Underline, false),
            5 => (CursorShape::Bar, true),
            6 => (CursorShape::Bar, false),
            _ => return,
        };
        self.cursor_style = CursorStyle { shape, blink };
        self.mark_all_dirty();
    }

    fn device_status(&mut self, ps: u16) {
        match ps {
            5 => self.reply(b"\x1b[0n".to_vec()),
            6 => {
                let base = if self.modes.origin {
                    self.cursor.y.saturating_sub(self.scroll_top)
                } else {
                    self.cursor.y
                };
                let s = format!("\x1b[{};{}R", base + 1, self.cursor.x + 1);
                self.reply(s.into_bytes());
            }
            _ => {}
        }
    }

    /// xterm window manipulation — only the harmless subset.
    fn window_ops(&mut self, ps: &[u16]) {
        let op = ps.first().copied().unwrap_or(0);
        match op {
            18 | 19 => {
                let code = if op == 18 { 8 } else { 9 };
                let s = format!("\x1b[{};{};{}t", code, self.rows, self.cols);
                self.reply(s.into_bytes());
            }
            22 => self.push_title(),
            23 => self.pop_title(),
            // 21 (report title), 8 (resize), 3 (move)…: intentionally unsupported.
            _ => {}
        }
    }

    // ----- SGR ------------------------------------------------------------

    fn sgr(&mut self, params: &Params) {
        let mut pen = self.cursor.pen;
        let mut it = params.iter();
        let mut any = false;
        while let Some(p) = it.next() {
            any = true;
            let code = p.first().copied().unwrap_or(0);
            match code {
                0 => pen = Pen::default(),
                1 => pen.flags |= attr::BOLD,
                2 => pen.flags |= attr::DIM,
                3 => pen.flags |= attr::ITALIC,
                4 => set_underline(&mut pen, p.get(1).copied().unwrap_or(1)),
                5 | 6 => pen.flags |= attr::BLINK,
                7 => pen.flags |= attr::INVERSE,
                8 => pen.flags |= attr::HIDDEN,
                9 => pen.flags |= attr::STRIKE,
                21 => set_underline(&mut pen, 2),
                22 => pen.flags &= !(attr::BOLD | attr::DIM),
                23 => pen.flags &= !attr::ITALIC,
                24 => pen.flags &= !attr::ANY_UL,
                25 => pen.flags &= !attr::BLINK,
                27 => pen.flags &= !attr::INVERSE,
                28 => pen.flags &= !attr::HIDDEN,
                29 => pen.flags &= !attr::STRIKE,
                30..=37 => pen.fg = Color::Indexed((code - 30) as u8),
                38 => {
                    if let Some(c) = parse_ext_color(p, &mut it) {
                        pen.fg = c;
                    }
                }
                39 => pen.fg = Color::Default,
                40..=47 => pen.bg = Color::Indexed((code - 40) as u8),
                48 => {
                    if let Some(c) = parse_ext_color(p, &mut it) {
                        pen.bg = c;
                    }
                }
                49 => pen.bg = Color::Default,
                58 => {
                    if let Some(c) = parse_ext_color(p, &mut it) {
                        pen.ul = c;
                    }
                }
                59 => pen.ul = Color::Default,
                90..=97 => pen.fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => pen.bg = Color::Indexed((code - 100 + 8) as u8),
                _ => {}
            }
        }
        if !any {
            pen = Pen::default();
        }
        self.cursor.pen = pen;
    }

    // ----- OSC helpers ----------------------------------------------------

    fn colors_changed(&mut self) {
        self.events.push(TermEvent::ColorsChanged);
        self.mark_all_dirty();
    }

    fn palette_color(&self, idx: u8) -> Rgb {
        if let Some(Some(c)) = self.overrides.palette.get(idx as usize) {
            return *c;
        }
        if idx < 16 {
            self.theme_ansi[idx as usize]
        } else {
            xterm_extended(idx)
        }
    }

    fn color_reply(&mut self, prefix: &str, rgb: Rgb, term: &[u8]) {
        let mut s = format!(
            "\x1b]{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}",
            prefix, rgb[0], rgb[0], rgb[1], rgb[1], rgb[2], rgb[2]
        )
        .into_bytes();
        s.extend_from_slice(term);
        self.reply(s);
    }

    fn osc_palette(&mut self, params: &[&[u8]], term: &[u8]) {
        let mut i = 1;
        let mut changed = false;
        while i + 1 < params.len() {
            let idx = std::str::from_utf8(params[i]).ok().and_then(|s| s.parse::<usize>().ok());
            if let Some(idx) = idx {
                if idx < 256 {
                    let spec = params[i + 1];
                    if spec == b"?" {
                        if self.policy.color_queries {
                            let rgb = self.palette_color(idx as u8);
                            self.color_reply(&format!("4;{}", idx), rgb, term);
                        }
                    } else if self.policy.color_changes {
                        if let Some(rgb) = parse_osc_color(spec) {
                            self.overrides.palette[idx] = Some(rgb);
                            changed = true;
                        }
                    }
                }
            }
            i += 2;
        }
        if changed {
            self.colors_changed();
        }
    }

    /// `target`: 0 = foreground (OSC 10), 1 = background (11), 2 = cursor (12).
    fn osc_dynamic_color(&mut self, target: usize, spec: &[u8], term: &[u8]) {
        if spec == b"?" {
            if !self.policy.color_queries {
                return;
            }
            let rgb = match target {
                0 => self.overrides.fg.unwrap_or(self.theme_fg),
                1 => self.overrides.bg.unwrap_or(self.theme_bg),
                _ => self.overrides.cursor.unwrap_or(self.theme_cursor),
            };
            self.color_reply(&format!("{}", 10 + target), rgb, term);
        } else if self.policy.color_changes {
            if let Some(rgb) = parse_osc_color(spec) {
                match target {
                    0 => self.overrides.fg = Some(rgb),
                    1 => self.overrides.bg = Some(rgb),
                    _ => self.overrides.cursor = Some(rgb),
                }
                self.colors_changed();
            }
        }
    }

    fn osc_9(&mut self, params: &[&[u8]]) {
        // ConEmu / Windows Terminal progress: OSC 9;4;state;percent
        if params.len() >= 2 && params[1] == b"4" {
            let state = params.get(2).and_then(|p| std::str::from_utf8(p).ok()).and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);
            let pct = params.get(3).and_then(|p| std::str::from_utf8(p).ok()).and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);
            self.events.push(TermEvent::Progress { state: state.min(4), percent: pct.min(100) });
            return;
        }
        // iTerm2 style notification: OSC 9;message
        if self.policy.notifications && params.len() >= 2 {
            let body = clean_text(&lossy(&join_params(params, 1)), 300);
            if !body.is_empty() {
                self.events.push(TermEvent::Notify { title: "Terminal".to_string(), body });
            }
        }
    }

    /// OSC 133 (FinalTerm / shell integration): A prompt, B input, C output, D finished.
    fn osc_133(&mut self, params: &[&[u8]]) {
        let kind = match params.get(1).and_then(|p| p.first()) {
            Some(k) => *k,
            None => return,
        };
        match kind {
            b'A' => {
                let y = self.cursor.y;
                self.grid[y].marks |= mark::PROMPT;
                self.pending_prompt_tag = true;
            }
            b'B' => {
                let line = self.screen_top_abs() + self.cursor.y as u64;
                self.cmd_input_start = Some((line, self.cursor.x));
            }
            b'C' => {
                let end = SelPoint { line: self.screen_top_abs() + self.cursor.y as u64, col: self.cursor.x };
                let input_start = self.cmd_input_start.take();
                let command = match input_start {
                    Some((line, col)) => {
                        let text = self.text_range(SelPoint { line, col }, end);
                        clean_text(text.trim(), 1024)
                    }
                    None => String::new(),
                };
                // the row the command was typed on carries the record id
                let prompt_line = input_start.map(|(l, _)| l).unwrap_or(end.line);
                let id = self.next_cmd_id;
                self.next_cmd_id = self.next_cmd_id.wrapping_add(1).max(1);
                if self.commands.len() >= MAX_COMMANDS {
                    self.commands.pop_front();
                }
                self.commands.push_back(CommandRecord {
                    id,
                    command: command.clone(),
                    cwd: self.cwd.clone(),
                    exit: None,
                    started: Instant::now(),
                    duration: None,
                    prompt_line,
                });
                self.open_cmd = Some(id);
                // tag the prompt row (the row the command was typed on)
                if let Some(row) = self.row_mut_by_abs(prompt_line) {
                    row.tag = id;
                    row.marks |= mark::OUTPUT;
                }
                self.arm_command_scan();
                self.events.push(TermEvent::CommandStarted { command });
            }
            b'D' => {
                let exit = params
                    .get(2)
                    .and_then(|p| std::str::from_utf8(p).ok())
                    .and_then(|s| s.trim().parse::<i32>().ok());
                if let Some(id) = self.open_cmd.take() {
                    if let Some(rec) = self.commands.iter_mut().rev().find(|r| r.id == id) {
                        let d = rec.started.elapsed();
                        rec.exit = exit;
                        rec.duration = Some(d);
                        let command = rec.command.clone();
                        self.events.push(TermEvent::CommandFinished { exit, duration: d, command });
                    }
                }
            }
            _ => {}
        }
    }

    /// MITOS private OSCs: widgets (MROP), execution blocks, autocomplete.
    fn osc_mitos(&mut self, params: &[&[u8]]) {
        let cmd = params[0];
        if cmd == mitos_utils::ipc::OSC_WIDGET.as_bytes() {
            if params.len() >= 2 {
                let payload = join_params(params, 1);
                if let Ok(w) = serde_json::from_slice::<RichWidget>(&payload) {
                    self.inject_widget(w, false);
                }
            }
        } else if cmd == mitos_utils::ipc::OSC_NEW_BLOCK.as_bytes() {
            let prompt = if params.len() >= 2 {
                lossy(&join_params(params, 1))
            } else {
                "mitos@user:~$ ".to_string()
            };
            self.new_block(&prompt);
        } else if cmd == b"MITOS_AUTOCOMPLETE" && params.len() >= 2 {
            let partial = clean_text(&lossy(params[1]), 1024);
            if !partial.is_empty() {
                self.events.push(TermEvent::AutocompleteRequest(partial));
            }
        }
    }

    /// Close the running MITOS execution block and start a new one on a fresh
    /// line. The prompt text is written into the grid (so it is selectable,
    /// searchable and survives reflow); the renderer draws BLOCK-marked rows
    /// in the theme's prompt colour.
    pub(super) fn new_block(&mut self, prompt: &str) {
        if let Some(started) = self.block_started.take() {
            self.events.push(TermEvent::BlockClosed { duration: started.elapsed() });
        }
        self.ghost_text = None;
        if self.cursor.x != 0 || self.cursor.wrap_pending {
            self.carriage_return();
            self.linefeed();
        }
        let y = self.cursor.y;
        self.grid[y].marks |= mark::BLOCK;

        let saved_pen = self.cursor.pen;
        let saved_link = self.cur_link;
        self.cursor.pen = Pen::default();
        self.cur_link = 0;
        let clean = clean_text(prompt, 256);
        for c in clean.chars() {
            self.put_char(c);
        }
        self.cursor.pen = saved_pen;
        self.cur_link = saved_link;
        self.carriage_return();
        self.linefeed();
        self.block_prompt = clean.trim_end().to_string();
        self.block_started = Some(Instant::now());
    }

    /// Let the embedding app tell the term which colours the theme uses, so
    /// OSC 4/10/11/12 *queries* are answered truthfully (vim, bat, delta…
    /// use OSC 11 to detect light vs dark backgrounds).
    pub fn set_theme_colors(&mut self, fg: Rgb, bg: Rgb, cursor: Rgb, ansi: [Rgb; 16]) {
        self.theme_fg = fg;
        self.theme_bg = bg;
        self.theme_cursor = cursor;
        self.theme_ansi = ansi;
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn set_underline(pen: &mut Pen, style: u16) {
    pen.flags &= !attr::ANY_UL;
    match style {
        0 => {}
        2 => pen.flags |= attr::DOUBLE_UL,
        3 => pen.flags |= attr::CURLY_UL,
        4 => pen.flags |= attr::DOTTED_UL,
        5 => pen.flags |= attr::DASHED_UL,
        _ => pen.flags |= attr::UNDERLINE,
    }
}

/// `38;5;n`, `38;2;r;g;b`, `38:5:n`, `38:2::r:g:b`, `38:2:r:g:b`.
fn parse_ext_color<'a, I: Iterator<Item = &'a [u16]>>(p: &[u16], it: &mut I) -> Option<Color> {
    let c8 = |v: u16| -> u8 { v.min(255) as u8 };
    if p.len() > 1 {
        match p[1] {
            5 => p.get(2).map(|&n| Color::Indexed(c8(n))),
            2 => {
                let off = if p.len() >= 6 { 3 } else { 2 };
                if p.len() >= off + 3 {
                    Some(Color::Rgb(c8(p[off]), c8(p[off + 1]), c8(p[off + 2])))
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
        let kind = it.next()?.first().copied().unwrap_or(0);
        match kind {
            5 => {
                let n = it.next()?.first().copied().unwrap_or(0);
                Some(Color::Indexed(c8(n)))
            }
            2 => {
                let r = it.next()?.first().copied().unwrap_or(0);
                let g = it.next()?.first().copied().unwrap_or(0);
                let b = it.next()?.first().copied().unwrap_or(0);
                Some(Color::Rgb(c8(r), c8(g), c8(b)))
            }
            _ => None,
        }
    }
}

/// vte splits OSC on every `;`; payloads (URIs, JSON, titles) may contain them.
fn join_params(params: &[&[u8]], start: usize) -> Vec<u8> {
    let mut out = Vec::new();
    if start >= params.len() {
        return out;
    }
    for (i, part) in params[start..].iter().enumerate() {
        if i > 0 {
            out.push(b';');
        }
        out.extend_from_slice(part);
    }
    out
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Strip control characters and cap the length (titles, notification text, prompts).
pub(crate) fn clean_text(s: &str, max_chars: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max_chars).collect()
}

pub(crate) fn hex_component(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    let max = (1u32 << (4 * s.len() as u32)) - 1;
    Some(((v * 255 + max / 2) / max) as u8)
}

/// `rgb:RR/GG/BB` (1–4 hex digits per channel) or `#RGB` / `#RRGGBB` / `#RRRGGGBBB` / `#RRRRGGGGBBBB`.
pub(crate) fn parse_osc_color(spec: &[u8]) -> Option<Rgb> {
    let s = std::str::from_utf8(spec).ok()?.trim();
    if let Some(rest) = s.strip_prefix("rgb:") {
        let mut it = rest.split('/');
        let r = hex_component(it.next()?)?;
        let g = hex_component(it.next()?)?;
        let b = hex_component(it.next()?)?;
        if it.next().is_some() {
            return None;
        }
        return Some([r, g, b]);
    }
    if let Some(hex) = s.strip_prefix('#') {
        let n = hex.len();
        if !hex.is_ascii() || n == 0 || n % 3 != 0 || n > 12 {
            return None;
        }
        let d = n / 3;
        return Some([
            hex_component(&hex[0..d])?,
            hex_component(&hex[d..2 * d])?,
            hex_component(&hex[2 * d..3 * d])?,
        ]);
    }
    None
}

pub(crate) fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in input {
        let v: u8 = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

pub(crate) fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return None;
            }
            let hi = hexval(b[i + 1])?;
            let lo = hexval(b[i + 2])?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn local_hostname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname").ok().map(|s| s.trim().to_string())
}

/// `file://host/path` → local path. Remote hosts (SSH sessions) are rejected so a
/// remote cwd is never used to start a *local* shell.
pub(crate) fn parse_file_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    let local = host.is_empty()
        || host == "localhost"
        || local_hostname().map(|h| h == host).unwrap_or(false);
    if !local {
        return None;
    }
    let path = percent_decode(&rest[slash..])?;
    if path.len() > 4096 || path.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(path)
}
