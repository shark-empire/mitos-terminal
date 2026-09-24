//! Selection, text extraction, word/line boundaries and URL detection.
//!
//! Selection endpoints are *absolute line ids* (see `term/mod.rs`), so a
//! selection survives scrolling and streaming output. Endpoints are inclusive.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SelPoint {
    pub line: u64,
    pub col: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelMode {
    Simple,
    Word,
    Line,
    Block,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: SelPoint,
    pub head: SelPoint,
    pub mode: SelMode,
    origin_lo: SelPoint,
    origin_hi: SelPoint,
}

impl Selection {
    pub fn lo(&self) -> SelPoint {
        if self.anchor <= self.head {
            self.anchor
        } else {
            self.head
        }
    }

    pub fn hi(&self) -> SelPoint {
        if self.anchor <= self.head {
            self.head
        } else {
            self.anchor
        }
    }

    /// Inclusive column range highlighted on `line`, if any.
    pub fn cols_on_line(&self, line: u64, cols: usize) -> Option<(usize, usize)> {
        let (lo, hi) = (self.lo(), self.hi());
        if line < lo.line || line > hi.line {
            return None;
        }
        let last = cols.saturating_sub(1);
        if self.mode == SelMode::Block {
            let c0 = self.anchor.col.min(self.head.col);
            let c1 = self.anchor.col.max(self.head.col);
            return Some((c0, c1.min(last)));
        }
        let start = if line == lo.line { lo.col } else { 0 };
        let end = if line == hi.line { hi.col.min(last) } else { last };
        Some((start, end))
    }
}

/// A URL found in plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlSpan {
    pub c0: usize,
    pub c1: usize,
    pub url: String,
}

const URL_SCHEMES: &[&str] = &["https://", "http://", "file://", "ftp://", "ssh://", "mailto:"];

impl Term {
    pub(super) fn row_mut_by_abs(&mut self, abs: u64) -> Option<&mut Row> {
        if abs < self.dropped {
            return None;
        }
        let idx = (abs - self.dropped) as usize;
        let sb = self.scrollback.len();
        if idx < sb {
            self.scrollback.get_mut(idx)
        } else if idx - sb < self.rows {
            self.grid.get_mut(idx - sb)
        } else {
            None
        }
    }

    /// The selection point for view row `row`, column `col`.
    pub fn view_point(&self, row: usize, col: usize) -> SelPoint {
        SelPoint { line: self.view_abs(row.min(self.rows - 1)), col: col.min(self.cols - 1) }
    }

    fn clamp_point(&self, p: SelPoint) -> SelPoint {
        let first = self.dropped;
        let last = self.screen_top_abs() + self.rows as u64 - 1;
        SelPoint { line: p.line.clamp(first, last), col: p.col.min(self.cols - 1) }
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    pub fn sel_clear(&mut self) {
        if self.selection.take().is_some() {
            self.mark_all_dirty();
        }
    }

    pub fn sel_begin(&mut self, mode: SelMode, p: SelPoint) {
        let p = self.clamp_point(p);
        let (lo, hi) = match mode {
            SelMode::Word => {
                let (a, b) = self.word_bounds(p.line, p.col);
                (SelPoint { line: p.line, col: a }, SelPoint { line: p.line, col: b })
            }
            SelMode::Line => {
                let (l0, l1) = self.logical_line_bounds(p.line);
                (SelPoint { line: l0, col: 0 }, SelPoint { line: l1, col: self.cols - 1 })
            }
            _ => (p, p),
        };
        self.selection = Some(Selection { anchor: lo, head: hi, mode, origin_lo: lo, origin_hi: hi });
        self.mark_all_dirty();
    }

    /// Extend the current selection to `p` (mouse drag).
    pub fn sel_update(&mut self, p: SelPoint) {
        let mut sel = match self.selection {
            Some(s) => s,
            None => return,
        };
        let p = self.clamp_point(p);
        match sel.mode {
            SelMode::Simple | SelMode::Block => sel.head = p,
            SelMode::Word => {
                let (a, b) = self.word_bounds(p.line, p.col);
                let ps = SelPoint { line: p.line, col: a };
                let pe = SelPoint { line: p.line, col: b };
                if pe < sel.origin_lo {
                    sel.anchor = sel.origin_hi;
                    sel.head = ps;
                } else if ps > sel.origin_hi {
                    sel.anchor = sel.origin_lo;
                    sel.head = pe;
                } else {
                    sel.anchor = sel.origin_lo;
                    sel.head = sel.origin_hi;
                }
            }
            SelMode::Line => {
                let (l0, l1) = self.logical_line_bounds(p.line);
                let ps = SelPoint { line: l0, col: 0 };
                let pe = SelPoint { line: l1, col: self.cols - 1 };
                if pe < sel.origin_lo {
                    sel.anchor = sel.origin_hi;
                    sel.head = ps;
                } else if ps > sel.origin_hi {
                    sel.anchor = sel.origin_lo;
                    sel.head = pe;
                } else {
                    sel.anchor = sel.origin_lo;
                    sel.head = sel.origin_hi;
                }
            }
        }
        if self.selection.map(|s| (s.anchor, s.head)) != Some((sel.anchor, sel.head)) {
            self.selection = Some(sel);
            self.mark_all_dirty();
        }
    }

    pub fn sel_all(&mut self) {
        let lo = SelPoint { line: self.dropped, col: 0 };
        let hi = SelPoint { line: self.screen_top_abs() + self.rows as u64 - 1, col: self.cols - 1 };
        self.selection = Some(Selection { anchor: lo, head: hi, mode: SelMode::Simple, origin_lo: lo, origin_hi: hi });
        self.mark_all_dirty();
    }

    /// Text of the current selection (`None` if there is none or it is empty).
    pub fn selection_text(&self) -> Option<String> {
        let sel = self.selection?;
        let text = self.extract(sel.lo(), sel.hi(), sel.mode == SelMode::Block, true, sel.anchor.col, sel.head.col);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Text between two points, end exclusive (used for OSC 133 command capture).
    pub(super) fn text_range(&self, a: SelPoint, b: SelPoint) -> String {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        self.extract(lo, hi, false, false, 0, 0)
    }

    fn extract(&self, lo: SelPoint, hi: SelPoint, block: bool, inclusive: bool, bc0: usize, bc1: usize) -> String {
        let mut out = String::new();
        let (blo, bhi) = (bc0.min(bc1), bc0.max(bc1));
        let mut line = lo.line.max(self.dropped);
        while line <= hi.line {
            let row = match self.row_by_abs(line) {
                Some(r) => r,
                None => break,
            };
            let row_len = row.cells.len();
            let (x0, x1) = if block {
                (blo, bhi + 1)
            } else {
                let s = if line == lo.line { lo.col } else { 0 };
                let e = if line == hi.line {
                    if inclusive {
                        hi.col + 1
                    } else {
                        hi.col
                    }
                } else {
                    self.cols
                };
                (s, e)
            };
            let reached_end = x1 >= row_len;
            let x1c = x1.min(row_len);
            let mut seg = String::new();
            if x0 < x1c {
                let mut start = x0;
                if start > 0 && row.cells[start].flags & attr::WIDE_TAIL != 0 {
                    start -= 1;
                }
                for x in start..x1c {
                    self.push_cell_text(&row.cells[x], &mut seg);
                }
            }
            let joins_next = row.wrapped && !block && line != hi.line;
            if !joins_next && reached_end {
                let t = seg.trim_end_matches(' ').len();
                seg.truncate(t);
            }
            out.push_str(&seg);
            if line != hi.line && !joins_next {
                out.push('\n');
            }
            line += 1;
        }
        out
    }

    // ----- word / line boundaries ----------------------------------------

    /// Inclusive column range of the "word" at `(line, col)`. Words are runs of
    /// characters that are all separators or all non-separators.
    pub fn word_bounds(&self, line: u64, col: usize) -> (usize, usize) {
        let row = match self.row_by_abs(line) {
            Some(r) => r,
            None => return (col, col),
        };
        let len = row.cells.len();
        if len == 0 || col >= len {
            return (col, col);
        }
        let mut c = col;
        if row.cells[c].flags & attr::WIDE_TAIL != 0 && c > 0 {
            c -= 1;
        }
        let is_sep = |cell: &Cell| -> bool {
            let ch = cell.ch;
            ch == ' ' || ch == '\0' || self.word_separators.contains(ch)
        };
        let class = is_sep(&row.cells[c]);
        let mut s = c;
        while s > 0 && is_sep(&row.cells[s - 1]) == class {
            s -= 1;
        }
        let mut e = c;
        while e + 1 < len && is_sep(&row.cells[e + 1]) == class {
            e += 1;
        }
        (s, e)
    }

    /// First and last absolute line of the logical (soft-wrap-joined) line containing `line`.
    pub fn logical_line_bounds(&self, line: u64) -> (u64, u64) {
        let mut l0 = line;
        while l0 > self.dropped {
            match self.row_by_abs(l0 - 1) {
                Some(r) if r.wrapped => l0 -= 1,
                _ => break,
            }
        }
        let mut l1 = line;
        loop {
            match self.row_by_abs(l1) {
                Some(r) if r.wrapped => {
                    if self.row_by_abs(l1 + 1).is_some() {
                        l1 += 1;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        (l0, l1)
    }

    // ----- URLs -----------------------------------------------------------

    /// Plain-text URLs on one row (for Ctrl+click and hover).
    pub fn urls_in_row(&self, row: &Row) -> Vec<UrlSpan> {
        // (char, column) pairs, skipping wide-char spacers
        let mut chars: Vec<(char, usize)> = Vec::with_capacity(row.cells.len());
        for (x, cell) in row.cells.iter().enumerate() {
            if cell.flags & attr::WIDE_TAIL != 0 {
                continue;
            }
            let ch = if self.cluster_str(cell.ch).is_some() { '\u{FFFD}' } else { cell.ch };
            chars.push((ch, x));
        }
        let text: Vec<char> = chars.iter().map(|(c, _)| *c).collect();
        let mut spans = Vec::new();
        let mut i = 0usize;
        while i < text.len() {
            let mut matched = false;
            for scheme in URL_SCHEMES {
                let sc: Vec<char> = scheme.chars().collect();
                if i + sc.len() <= text.len()
                    && text[i..i + sc.len()].iter().zip(sc.iter()).all(|(a, b)| a.to_ascii_lowercase() == *b)
                    && (i == 0 || !text[i - 1].is_alphanumeric())
                {
                    let mut end = i + sc.len();
                    while end < text.len() && is_url_char(text[end]) {
                        end += 1;
                    }
                    end = trim_url_end(&text, i, end);
                    if end > i + sc.len() {
                        let url: String = text[i..end].iter().collect();
                        spans.push(UrlSpan { c0: chars[i].1, c1: chars[end - 1].1, url });
                        i = end;
                        matched = true;
                        break;
                    }
                }
            }
            if !matched {
                i += 1;
            }
        }
        spans
    }

    /// The URL under `(line, col)`: an OSC 8 hyperlink first, else a plain-text URL.
    pub fn url_at(&self, line: u64, col: usize) -> Option<UrlSpan> {
        let row = self.row_by_abs(line)?;
        if col < row.cells.len() {
            let mut c = col;
            if row.cells[c].flags & attr::WIDE_TAIL != 0 && c > 0 {
                c -= 1;
            }
            let link = row.cells[c].link;
            if link != 0 {
                let uri = self.link_uri(link)?.to_string();
                // extent: the contiguous run of cells sharing this link id
                let mut c0 = c;
                while c0 > 0 && row.cells[c0 - 1].link == link {
                    c0 -= 1;
                }
                let mut c1 = c;
                while c1 + 1 < row.cells.len() && row.cells[c1 + 1].link == link {
                    c1 += 1;
                }
                return Some(UrlSpan { c0, c1, url: uri });
            }
        }
        self.urls_in_row(row).into_iter().find(|s| col >= s.c0 && col <= s.c1)
    }
}

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !matches!(c, '<' | '>' | '"' | '\'' | '`' | '\u{FFFD}' | '\0')
}

/// Drop trailing punctuation and unbalanced closing brackets.
fn trim_url_end(text: &[char], start: usize, mut end: usize) -> usize {
    while end > start {
        let c = text[end - 1];
        if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') {
            end -= 1;
            continue;
        }
        let open = match c {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        };
        if let Some(o) = open {
            let opens = text[start..end].iter().filter(|x| **x == o).count();
            let closes = text[start..end].iter().filter(|x| **x == c).count();
            if closes > opens {
                end -= 1;
                continue;
            }
        }
        break;
    }
    end
}
