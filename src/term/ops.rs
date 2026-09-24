//! Screen-editing primitives used by the escape-sequence handlers.
//!
//! Everything here upholds the invariants the rest of the crate relies on:
//! * `grid.len() == rows` and every grid row has exactly `cols` cells;
//! * `cursor.x < cols`, `cursor.y < rows`, `scroll_top < scroll_bottom < rows`
//!   (or a 1-row screen);
//! * a `WIDE` cell is always followed by its `WIDE_TAIL` spacer.

use std::mem;

use super::*;

impl Term {
    #[inline]
    pub(super) fn blank(&self) -> Cell {
        self.cursor.pen.blank_bce()
    }

    // ----- printing -------------------------------------------------------

    pub(super) fn put_char(&mut self, c: char) {
        let mut c = c;
        if (c as u32) >= CLUSTER_BASE {
            c = '\u{FFFD}';
        }
        let w = char_width(c);
        if w == 0 {
            self.attach_zero_width(c);
            return;
        }
        let cols = self.cols;

        if self.cursor.wrap_pending {
            if self.modes.autowrap {
                self.wrap_to_next_line();
            } else {
                self.cursor.wrap_pending = false;
            }
        }

        // A wide character that does not fit in the last column.
        if w == 2 && self.cursor.x + 1 >= cols {
            if cols < 2 {
                return;
            }
            if self.modes.autowrap {
                let (y, x) = (self.cursor.y, self.cursor.x);
                let blank = self.blank();
                self.fix_wide_edges(y, x, x + 1);
                self.grid[y].cells[x] = blank;
                self.wrap_to_next_line();
            } else {
                self.cursor.x = cols - 2;
            }
        }

        let (y, x) = (self.cursor.y, self.cursor.x);
        if self.modes.insert {
            self.insert_cells(w);
        }
        self.fix_wide_over(y, x, w);

        let pen = self.cursor.pen;
        let link = self.cur_link;
        {
            let row = &mut self.grid[y];
            row.cells[x] = Cell {
                ch: c,
                fg: pen.fg,
                bg: pen.bg,
                ul: pen.ul,
                flags: pen.flags | if w == 2 { attr::WIDE } else { 0 },
                link,
            };
            if w == 2 {
                row.cells[x + 1] = Cell {
                    ch: ' ',
                    fg: pen.fg,
                    bg: pen.bg,
                    ul: pen.ul,
                    flags: (pen.flags & !attr::WIDE) | attr::WIDE_TAIL,
                    link,
                };
            }
        }
        self.mark_dirty(y);
        if self.glow {
            self.note_fresh(x, y, w);
        }
        self.last_char = Some(c);

        let nx = x + w;
        if nx >= cols {
            self.cursor.x = cols - 1;
            self.cursor.wrap_pending = true;
        } else {
            self.cursor.x = nx;
        }

        if self.ghost_text.is_some() {
            let mut clear = false;
            if let Some(g) = self.ghost_text.as_mut() {
                if g.starts_with(c) {
                    g.remove(0);
                    clear = g.is_empty();
                } else {
                    clear = true;
                }
            }
            if clear {
                self.ghost_text = None;
            }
        }
    }

    /// Combining marks, ZWJ, variation selectors: merge into the previous cell.
    pub(super) fn attach_zero_width(&mut self, c: char) {
        let y = self.cursor.y;
        let mut x = if self.cursor.wrap_pending {
            self.cursor.x
        } else if self.cursor.x > 0 {
            self.cursor.x - 1
        } else {
            return;
        };
        if x > 0 && self.grid[y].cells[x].flags & attr::WIDE_TAIL != 0 {
            x -= 1;
        }
        let base = self.grid[y].cells[x].ch;
        let mut s = String::new();
        match self.cluster_str(base) {
            Some(cs) => s.push_str(cs),
            None => s.push(base),
        }
        if s.chars().count() >= 8 {
            return;
        }
        s.push(c);
        if let Some(nc) = self.intern_cluster(&s) {
            self.grid[y].cells[x].ch = nc;
            self.mark_dirty(y);
        }
    }

    pub(super) fn repeat_last_char(&mut self, n: usize) {
        if let Some(c) = self.last_char {
            for _ in 0..n.min(65_536) {
                self.put_char(c);
            }
        }
    }

    /// Before writing `w` cells at `x`: blank the orphaned half of any wide
    /// character we are about to overwrite.
    fn fix_wide_over(&mut self, y: usize, x: usize, w: usize) {
        let cols = self.cols;
        let blank = self.blank();
        let cells = &mut self.grid[y].cells;
        if x > 0 && cells[x].flags & attr::WIDE_TAIL != 0 {
            cells[x - 1] = blank;
        }
        let last = (x + w - 1).min(cols - 1);
        if last + 1 < cols && cells[last].flags & attr::WIDE != 0 {
            cells[last + 1] = blank;
        }
    }

    /// Blank orphaned wide halves at the edges of the cell range `[x0, x1)`.
    fn fix_wide_edges(&mut self, y: usize, x0: usize, x1: usize) {
        let cols = self.cols;
        let blank = self.blank();
        let cells = &mut self.grid[y].cells;
        if x0 > 0 && x0 < cols && cells[x0].flags & attr::WIDE_TAIL != 0 {
            cells[x0 - 1] = blank;
        }
        if x1 > 0 && x1 < cols && cells[x1 - 1].flags & attr::WIDE != 0 {
            cells[x1] = blank;
        }
    }

    // ----- line feed / wrap ----------------------------------------------

    pub(super) fn carriage_return(&mut self) {
        self.cursor.x = 0;
        self.cursor.wrap_pending = false;
    }

    /// IND: down one row, scrolling at the bottom margin.
    pub(super) fn index_down(&mut self) {
        self.cursor.wrap_pending = false;
        if self.cursor.y == self.scroll_bottom {
            let (t, b) = (self.scroll_top, self.scroll_bottom);
            self.scroll_up_region(t, b, 1, true);
        } else if self.cursor.y + 1 < self.rows {
            self.cursor.y += 1;
        }
    }

    /// LF / VT / FF as received from the child.
    pub(super) fn linefeed(&mut self) {
        if self.scan_budget > 0 && !self.in_alt {
            self.scan_budget -= 1;
            self.scan_missing_command();
        }
        self.index_down();
        if self.modes.lnm {
            self.cursor.x = 0;
        }
        self.ghost_text = None;
    }

    /// RI: up one row, scrolling down at the top margin.
    pub(super) fn reverse_index(&mut self) {
        self.cursor.wrap_pending = false;
        if self.cursor.y == self.scroll_top {
            let (t, b) = (self.scroll_top, self.scroll_bottom);
            self.scroll_down_region(t, b, 1);
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
        }
    }

    pub(super) fn wrap_to_next_line(&mut self) {
        let y = self.cursor.y;
        self.grid[y].wrapped = true;
        self.cursor.x = 0;
        self.cursor.wrap_pending = false;
        self.index_down();
    }

    /// Cheap per-line check for "command not found" (armed by Enter / OSC 133;C).
    fn scan_missing_command(&mut self) {
        let y = self.cursor.y;
        let mut s = mem::take(&mut self.scratch);
        s.clear();
        if let Some(row) = self.grid.get(y) {
            for c in &row.cells {
                if c.flags & attr::WIDE_TAIL == 0 {
                    s.push(c.ch);
                }
            }
        }
        if s.contains("not found") {
            if let Some(cmd) = crate::pkg_bridge::detect_missing_command(s.trim_end()) {
                if self.already_suggested.len() > 512 {
                    self.already_suggested.clear();
                }
                if self.already_suggested.insert(cmd.clone()) {
                    self.events.push(TermEvent::MissingCommand(cmd));
                    self.spike_error_row(y);
                }
            }
        }
        self.scratch = s;
    }

    // ----- scrolling ------------------------------------------------------

    /// Scroll `[top, bottom]` up by `n`. With `to_scrollback`, rows leaving the
    /// top of a full-screen primary scroll go into history.
    pub(super) fn scroll_up_region(&mut self, top: usize, bottom: usize, n: usize, to_scrollback: bool) {
        let n = n.min(bottom + 1 - top);
        if n == 0 {
            return;
        }
        let full = to_scrollback && top == 0 && bottom + 1 == self.rows && !self.in_alt;
        let blank = self.blank();
        self.grid[top..=bottom].rotate_left(n);
        for i in (bottom + 1 - n)..=bottom {
            if full {
                let fresh = Row::filled(self.cols, blank);
                let old = mem::replace(&mut self.grid[i], fresh);
                self.push_scrollback(old);
            } else {
                self.grid[i].reset(blank);
            }
        }
        if !full {
            self.selection = None;
        }
        self.mark_all_dirty();
    }

    pub(super) fn scroll_down_region(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        if n == 0 {
            return;
        }
        let blank = self.blank();
        self.grid[top..=bottom].rotate_right(n);
        for i in top..top + n {
            self.grid[i].reset(blank);
        }
        self.selection = None;
        self.mark_all_dirty();
    }

    fn push_scrollback(&mut self, mut row: Row) {
        if !row.wrapped {
            row.trim_blank();
        }
        if row.cells.capacity() > row.cells.len() {
            row.cells.shrink_to_fit();
        }
        if self.scrollback_limit == 0 {
            self.dropped += 1;
            self.after_drop();
            return;
        }
        let mut dropped_any = false;
        while self.scrollback.len() >= self.scrollback_limit {
            self.scrollback.pop_front();
            self.dropped += 1;
            dropped_any = true;
        }
        self.scrollback.push_back(row);
        if self.display_offset > 0 {
            self.display_offset = (self.display_offset + 1).min(self.scrollback.len());
        }
        if dropped_any {
            self.after_drop();
        }
    }

    pub(super) fn after_drop(&mut self) {
        let d = self.dropped;
        let clear = self.selection.as_ref().map(|s| s.lo().line < d).unwrap_or(false);
        if clear {
            self.selection = None;
        }
        if !self.widgets.is_empty() {
            self.widgets.retain(|w| w.line >= d);
        }
    }

    // ----- erase ----------------------------------------------------------

    /// Blank `[x0, x1)` of row `y` (background-colour-erase).
    pub(super) fn erase_cells(&mut self, y: usize, x0: usize, x1: usize) {
        let x1 = x1.min(self.cols);
        if x0 >= x1 || y >= self.rows {
            return;
        }
        let blank = self.blank();
        self.fix_wide_edges(y, x0, x1);
        for c in &mut self.grid[y].cells[x0..x1] {
            *c = blank;
        }
        self.mark_dirty(y);
    }

    pub(super) fn erase_row(&mut self, y: usize) {
        if y >= self.rows {
            return;
        }
        let blank = self.blank();
        if self.grid[y].marks & mark::WIDGET != 0 {
            let line = self.screen_top_abs() + y as u64;
            self.widgets.retain(|w| w.line != line);
        }
        self.grid[y].reset(blank);
        self.mark_dirty(y);
    }

    /// ED.
    pub(super) fn erase_display(&mut self, mode: u16) {
        let (x, y) = (self.cursor.x, self.cursor.y);
        match mode {
            0 => {
                self.erase_cells(y, x, self.cols);
                for r in (y + 1)..self.rows {
                    self.erase_row(r);
                }
            }
            1 => {
                for r in 0..y {
                    self.erase_row(r);
                }
                self.erase_cells(y, 0, x + 1);
            }
            2 => {
                for r in 0..self.rows {
                    self.erase_row(r);
                }
            }
            3 => self.clear_scrollback(),
            _ => {}
        }
        self.selection = None;
    }

    /// EL.
    pub(super) fn erase_line(&mut self, mode: u16) {
        let (x, y) = (self.cursor.x, self.cursor.y);
        match mode {
            0 => self.erase_cells(y, x, self.cols),
            1 => self.erase_cells(y, 0, x + 1),
            2 => self.erase_cells(y, 0, self.cols),
            _ => {}
        }
    }

    /// ECH.
    pub(super) fn erase_chars(&mut self, n: usize) {
        let (x, y) = (self.cursor.x, self.cursor.y);
        self.erase_cells(y, x, x.saturating_add(n));
    }

    // ----- insert / delete -----------------------------------------------

    /// ICH (also used by insert mode).
    pub(super) fn insert_cells(&mut self, n: usize) {
        let (x, y) = (self.cursor.x, self.cursor.y);
        let cols = self.cols;
        let n = n.min(cols - x);
        if n == 0 {
            return;
        }
        let blank = self.blank();
        {
            // Inserting between a wide head and its tail destroys the pair.
            let cells = &mut self.grid[y].cells;
            if x > 0 && cells[x].flags & attr::WIDE_TAIL != 0 {
                cells[x - 1] = blank;
                cells[x] = blank;
            }
        }
        let cells = &mut self.grid[y].cells;
        cells[x..cols].rotate_right(n);
        for c in &mut cells[x..x + n] {
            *c = blank;
        }
        if cells[cols - 1].flags & attr::WIDE != 0 {
            cells[cols - 1] = blank;
        }
        self.mark_dirty(y);
    }

    /// DCH.
    pub(super) fn delete_cells(&mut self, n: usize) {
        let (x, y) = (self.cursor.x, self.cursor.y);
        let cols = self.cols;
        let n = n.min(cols - x);
        if n == 0 {
            return;
        }
        let blank = self.blank();
        self.fix_wide_edges(y, x, x + n);
        let cells = &mut self.grid[y].cells;
        cells[x..cols].rotate_left(n);
        for c in &mut cells[cols - n..cols] {
            *c = blank;
        }
        self.mark_dirty(y);
    }

    /// IL.
    pub(super) fn insert_lines(&mut self, n: usize) {
        let y = self.cursor.y;
        if y < self.scroll_top || y > self.scroll_bottom {
            return;
        }
        let b = self.scroll_bottom;
        self.scroll_down_region(y, b, n);
        self.cursor.x = 0;
        self.cursor.wrap_pending = false;
    }

    /// DL.
    pub(super) fn delete_lines(&mut self, n: usize) {
        let y = self.cursor.y;
        if y < self.scroll_top || y > self.scroll_bottom {
            return;
        }
        let b = self.scroll_bottom;
        self.scroll_up_region(y, b, n, false);
        self.cursor.x = 0;
        self.cursor.wrap_pending = false;
    }

    // ----- cursor movement -----------------------------------------------

    /// CUP/HVP (0-based), honouring origin mode.
    pub(super) fn goto(&mut self, row: usize, col: usize) {
        let (top, bottom) = if self.modes.origin {
            (self.scroll_top, self.scroll_bottom)
        } else {
            (0, self.rows - 1)
        };
        self.cursor.y = top.saturating_add(row).min(bottom);
        self.cursor.x = col.min(self.cols - 1);
        self.cursor.wrap_pending = false;
    }

    pub(super) fn set_row(&mut self, row: usize) {
        let x = self.cursor.x;
        self.goto(row, x);
    }

    pub(super) fn set_col(&mut self, col: usize) {
        self.cursor.x = col.min(self.cols - 1);
        self.cursor.wrap_pending = false;
    }

    pub(super) fn move_up(&mut self, n: usize) {
        let top = if self.cursor.y >= self.scroll_top { self.scroll_top } else { 0 };
        self.cursor.y = self.cursor.y.saturating_sub(n).max(top);
        self.cursor.wrap_pending = false;
    }

    pub(super) fn move_down(&mut self, n: usize) {
        let bottom = if self.cursor.y <= self.scroll_bottom { self.scroll_bottom } else { self.rows - 1 };
        self.cursor.y = self.cursor.y.saturating_add(n).min(bottom);
        self.cursor.wrap_pending = false;
    }

    pub(super) fn move_right(&mut self, n: usize) {
        self.cursor.x = self.cursor.x.saturating_add(n).min(self.cols - 1);
        self.cursor.wrap_pending = false;
    }

    pub(super) fn move_left(&mut self, n: usize) {
        self.cursor.x = self.cursor.x.saturating_sub(n);
        self.cursor.wrap_pending = false;
    }

    pub(super) fn tab_forward(&mut self, n: usize) {
        for _ in 0..n.min(self.cols) {
            let mut x = self.cursor.x + 1;
            while x < self.cols && !self.tabs[x] {
                x += 1;
            }
            self.cursor.x = x.min(self.cols - 1);
        }
        self.cursor.wrap_pending = false;
    }

    pub(super) fn tab_backward(&mut self, n: usize) {
        for _ in 0..n.min(self.cols) {
            let mut x = self.cursor.x;
            while x > 0 {
                x -= 1;
                if self.tabs[x] {
                    break;
                }
            }
            self.cursor.x = x;
        }
        self.cursor.wrap_pending = false;
    }

    /// DECSTBM with 0-based inclusive margins. Invalid regions are ignored.
    pub(super) fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        if top < bottom && bottom < self.rows {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.goto(0, 0);
        }
    }

    // ----- save / restore cursor -----------------------------------------

    pub(super) fn save_cursor(&mut self) {
        let mut c = self.cursor;
        c.origin = self.modes.origin;
        self.saved_cursor[self.in_alt as usize] = Some(c);
    }

    pub(super) fn restore_cursor(&mut self) {
        match self.saved_cursor[self.in_alt as usize] {
            Some(c) => {
                self.cursor = c;
                self.modes.origin = c.origin;
                self.cursor.x = self.cursor.x.min(self.cols - 1);
                self.cursor.y = self.cursor.y.min(self.rows - 1);
            }
            None => {
                self.cursor = Cursor::home();
                self.modes.origin = false;
            }
        }
    }

    // ----- alternate screen ----------------------------------------------

    pub(super) fn enter_alt(&mut self) {
        if self.in_alt {
            return;
        }
        self.primary_cursor = self.cursor;
        let blank_grid: Vec<Row> = (0..self.rows).map(|_| Row::new(self.cols)).collect();
        self.stash = mem::replace(&mut self.grid, blank_grid);
        self.in_alt = true;
        self.selection = None;
        self.display_offset = 0;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.mark_all_dirty();
    }

    pub(super) fn leave_alt(&mut self, restore_cursor: bool) {
        if !self.in_alt {
            return;
        }
        self.grid = mem::take(&mut self.stash);
        self.in_alt = false;
        self.selection = None;
        self.display_offset = 0;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        if restore_cursor {
            self.cursor = self.primary_cursor;
        }
        self.cursor.x = self.cursor.x.min(self.cols - 1);
        self.cursor.y = self.cursor.y.min(self.rows - 1);
        self.mark_all_dirty();
    }

    pub(super) fn leave_alt_discard(&mut self) {
        if self.in_alt {
            self.leave_alt(true);
        }
    }

    // ----- misc -----------------------------------------------------------

    /// DECALN: fill the screen with `E`.
    pub(super) fn screen_alignment(&mut self) {
        for y in 0..self.rows {
            for c in self.grid[y].cells.iter_mut() {
                *c = Cell { ch: 'E', ..Cell::BLANK };
            }
            self.grid[y].wrapped = false;
        }
        self.mark_all_dirty();
    }

    /// Soft reset (DECSTR): modes and attributes, but keep the screen.
    pub(super) fn soft_reset(&mut self) {
        self.modes = Modes::default();
        self.cursor.pen = Pen::default();
        self.cursor.wrap_pending = false;
        self.cursor.charset = CharsetState::default();
        self.cursor_style = self.default_cursor_style;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.saved_cursor = [None, None];
        self.cur_link = 0;
        self.mark_all_dirty();
    }
}
