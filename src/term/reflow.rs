//! Resize and text reflow.
//!
//! Primary screen: `scrollback ++ grid` is cut into *logical lines* (rows
//! joined while `wrapped` is set), each logical line is re-split at the new
//! width, and the cursor is tracked through the operation by its offset
//! inside its logical line. The alternate screen is simply cropped/padded —
//! full-screen applications redraw on SIGWINCH anyway.
//!
//! Height-only changes take a cheap path that pushes/pulls whole rows.

use std::collections::VecDeque;
use std::mem;

use super::*;

impl Term {
    /// Resize the screen model. Call whenever the pane size in cells changes.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.clamp(1, MAX_COLS);
        let rows = rows.clamp(1, MAX_ROWS);
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.selection = None;
        self.display_offset = 0;
        self.cmd_input_start = None;
        self.fresh.clear();
        let old_cols = self.cols;

        if self.in_alt {
            crop_pad(&mut self.grid, cols, rows);
            let mut stash = mem::take(&mut self.stash);
            let cur = self.primary_cursor;
            let (cx, cy, inc) = reflow_primary(
                &mut self.scrollback,
                &mut stash,
                (cur.x, cur.y),
                old_cols,
                cols,
                rows,
                self.scrollback_limit,
            );
            self.stash = stash;
            self.primary_cursor.x = cx;
            self.primary_cursor.y = cy;
            self.dropped += inc;
            self.cursor.x = self.cursor.x.min(cols - 1);
            self.cursor.y = self.cursor.y.min(rows - 1);
        } else {
            let (cx, cy, inc) = reflow_primary(
                &mut self.scrollback,
                &mut self.grid,
                (self.cursor.x, self.cursor.y),
                old_cols,
                cols,
                rows,
                self.scrollback_limit,
            );
            self.cursor.x = cx;
            self.cursor.y = cy;
            self.dropped += inc;
        }

        self.cols = cols;
        self.rows = rows;
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.cursor.wrap_pending = false;

        let old_tabs = mem::take(&mut self.tabs);
        self.tabs = default_tabs(cols);
        for (i, t) in old_tabs.iter().enumerate().take(cols) {
            self.tabs[i] = *t;
        }
        for c in self.saved_cursor.iter_mut().flatten() {
            c.x = c.x.min(cols - 1);
            c.y = c.y.min(rows - 1);
        }

        self.reanchor_widgets();
        self.after_drop();

        self.dirty = vec![true; rows];
        self.dirty_all = true;
        self.last_dirty_row = usize::MAX;
        self.changed = true;
    }

    /// After a reflow abs line ids changed; rows keep their WIDGET mark, so
    /// re-assign widgets to the marked rows in order.
    fn reanchor_widgets(&mut self) {
        if self.widgets.is_empty() {
            return;
        }
        self.widgets.sort_by_key(|w| w.line);
        let mut lines: Vec<u64> = Vec::new();
        for i in 0..self.combined_len() {
            if self.combined_row(i).marks & mark::WIDGET != 0 {
                lines.push(self.dropped + i as u64);
            }
        }
        let n = lines.len().min(self.widgets.len());
        self.widgets.truncate(n);
        for (w, l) in self.widgets.iter_mut().zip(lines) {
            w.line = l;
        }
    }
}

/// Crop or pad every row to `cols` and the grid to `rows` (alternate screen).
fn crop_pad(grid: &mut Vec<Row>, cols: usize, rows: usize) {
    for row in grid.iter_mut() {
        row.cells.resize(cols, Cell::BLANK);
        row.wrapped = false;
        sanitize_wide(row);
    }
    grid.truncate(rows);
    while grid.len() < rows {
        grid.push(Row::new(cols));
    }
}

/// Returns `(cursor_x, cursor_y, lines_dropped_from_scrollback)`.
fn reflow_primary(
    scrollback: &mut VecDeque<Row>,
    grid: &mut Vec<Row>,
    cursor: (usize, usize),
    old_cols: usize,
    new_cols: usize,
    new_rows: usize,
    limit: usize,
) -> (usize, usize, u64) {
    if new_cols == old_cols {
        return resize_rows_only(scrollback, grid, cursor, new_rows, limit);
    }

    let old_rows = grid.len();
    let sb_len = scrollback.len();
    let mut lines: Vec<Row> = Vec::with_capacity(sb_len + old_rows);
    lines.extend(scrollback.drain(..));
    lines.extend(grid.drain(..));
    let cursor_abs = sb_len + cursor.1.min(old_rows.saturating_sub(1));

    let mut out: Vec<Row> = Vec::with_capacity(lines.len());
    let mut new_cursor = (0usize, 0usize);
    let mut i = 0usize;
    while i < lines.len() {
        // Logical line = rows i..=j.
        let mut j = i;
        while j + 1 < lines.len() && lines[j].wrapped {
            j += 1;
        }
        let has_cursor = cursor_abs >= i && cursor_abs <= j;

        let mut cells: Vec<Cell> = Vec::new();
        let mut marks = 0u8;
        let mut tag = 0u32;
        for (k, row) in lines[i..=j].iter().enumerate() {
            marks |= row.marks;
            if tag == 0 {
                tag = row.tag;
            }
            cells.extend_from_slice(&row.cells);
            let is_last = i + k == j;
            if !is_last && row.cells.len() < old_cols {
                let pad = old_cols - row.cells.len();
                cells.resize(cells.len() + pad, Cell::BLANK);
            }
        }

        let cursor_off = if has_cursor { Some((cursor_abs - i) * old_cols + cursor.0) } else { None };
        let keep_min = cursor_off.map(|o| o + 1).unwrap_or(0);
        while cells.len() > keep_min && cells.last().map(|c| c.is_blank()).unwrap_or(false) {
            cells.pop();
        }
        if let Some(o) = cursor_off {
            if cells.len() < o + 1 {
                cells.resize(o + 1, Cell::BLANK);
            }
        }

        // Split at the new width, never separating a wide head from its tail.
        let first_out = out.len();
        let mut start = 0usize;
        loop {
            let mut end = (start + new_cols).min(cells.len());
            if end < cells.len() && end > start + 1 && cells[end - 1].flags & attr::WIDE != 0 {
                end -= 1;
            }
            let wrapped = end < cells.len();
            out.push(Row { cells: cells[start..end].to_vec(), wrapped, marks: 0, tag: 0 });
            start = end;
            if start >= cells.len() {
                break;
            }
        }
        out[first_out].marks = marks;
        out[first_out].tag = tag;

        if let Some(o) = cursor_off {
            let mut acc = 0usize;
            let mut idx = out.len() - 1;
            let mut col = 0usize;
            for r in first_out..out.len() {
                let len_r = out[r].cells.len();
                if o < acc + len_r || r == out.len() - 1 {
                    idx = r;
                    col = o.saturating_sub(acc);
                    break;
                }
                acc += len_r;
            }
            new_cursor = (col.min(new_cols - 1), idx);
        }
        i = j + 1;
    }

    // Blank rows *below* the cursor are not content: dropping them keeps text
    // anchored to the top of a mostly-empty screen instead of pushing it into history.
    let cy_pre = new_cursor.1.min(out.len().saturating_sub(1));
    while out.len() > cy_pre + 1 {
        let droppable = match out.last() {
            Some(r) => r.marks == 0 && r.tag == 0 && !r.wrapped && r.cells.iter().all(|c| c.is_blank()),
            None => false,
        };
        if droppable {
            out.pop();
        } else {
            break;
        }
    }
    for r in out.iter_mut() {
        sanitize_wide(r);
    }

    // Choose the screen window: show as much of the bottom as possible while
    // keeping the cursor visible.
    let total = out.len();
    let cy = new_cursor.1.min(total.saturating_sub(1));
    let top = total.saturating_sub(new_rows).min(cy);
    let mut new_grid: Vec<Row> = out.split_off(top);
    new_grid.truncate(new_rows);
    while new_grid.len() < new_rows {
        new_grid.push(Row::new(new_cols));
    }
    for row in new_grid.iter_mut() {
        row.cells.resize(new_cols, Cell::BLANK);
        sanitize_wide(row);
    }

    let mut inc = 0u64;
    let mut sb: VecDeque<Row> = out.into();
    while sb.len() > limit {
        sb.pop_front();
        inc += 1;
    }
    *scrollback = sb;
    *grid = new_grid;
    (new_cursor.0, cy - top, inc)
}

/// Only the number of rows changed: push rows into / pull rows out of history.
fn resize_rows_only(
    scrollback: &mut VecDeque<Row>,
    grid: &mut Vec<Row>,
    cursor: (usize, usize),
    new_rows: usize,
    limit: usize,
) -> (usize, usize, u64) {
    let old_rows = grid.len();
    let cols = grid.first().map(|r| r.cells.len()).unwrap_or(1);
    let mut cy = cursor.1.min(old_rows.saturating_sub(1));

    if new_rows < old_rows {
        let need = (cy + 1).saturating_sub(new_rows);
        let remove = old_rows - new_rows;
        let push_top = need.min(remove);
        for _ in 0..push_top {
            let mut row = grid.remove(0);
            if !row.wrapped {
                row.trim_blank();
            }
            row.cells.shrink_to_fit();
            scrollback.push_back(row);
            cy -= 1;
        }
        grid.truncate(new_rows);
    } else if new_rows > old_rows {
        let add = new_rows - old_rows;
        let pull = add.min(scrollback.len());
        for _ in 0..pull {
            if let Some(mut row) = scrollback.pop_back() {
                row.cells.resize(cols, Cell::BLANK);
                grid.insert(0, row);
                cy += 1;
            }
        }
        while grid.len() < new_rows {
            grid.push(Row::new(cols));
        }
    }

    let mut inc = 0u64;
    while scrollback.len() > limit {
        scrollback.pop_front();
        inc += 1;
    }
    (cursor.0, cy.min(new_rows - 1), inc)
}

/// Restore the wide-character invariant: every head is followed by its tail
/// and every tail is preceded by a head (cropping or a 1-column reflow can
/// separate them).
fn sanitize_wide(row: &mut Row) {
    let n = row.cells.len();
    for i in 0..n {
        let f = row.cells[i].flags;
        if f & attr::WIDE != 0 {
            let ok = i + 1 < n && row.cells[i + 1].flags & attr::WIDE_TAIL != 0;
            if !ok {
                row.cells[i] = Cell::BLANK;
            }
        } else if f & attr::WIDE_TAIL != 0 {
            let ok = i > 0 && row.cells[i - 1].flags & attr::WIDE != 0;
            if !ok {
                row.cells[i] = Cell::BLANK;
            }
        }
    }
}
