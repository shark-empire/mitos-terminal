//! Incremental search over scrollback + screen.
//!
//! Matching works on *logical lines* (soft-wrapped rows joined), so a match
//! that wraps across rows is found; each hit records one span per row.

use super::*;

/// One search hit; usually one span, more if it wraps across rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// `(absolute line, first column, last column inclusive)`.
    pub spans: Vec<(u64, usize, usize)>,
}

impl Hit {
    pub fn first_line(&self) -> u64 {
        self.spans.first().map(|s| s.0).unwrap_or(0)
    }
}

impl Term {
    /// All hits for `query`, oldest first. An empty query yields nothing.
    /// Case-insensitive search folds one char at a time, so byte offsets
    /// never drift.
    pub fn search(&self, query: &str, case_sensitive: bool) -> Vec<Hit> {
        if query.is_empty() {
            return Vec::new();
        }
        let fold = |c: char| -> char {
            if case_sensitive {
                c
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        };
        let needle: Vec<char> = query.chars().map(fold).collect();
        let mut hits: Vec<Hit> = Vec::new();
        let total = self.combined_len();
        let mut i = 0usize;
        while i < total {
            // gather one logical line
            let mut chars: Vec<char> = Vec::new();
            let mut pos: Vec<(u64, usize)> = Vec::new();
            let mut j = i;
            loop {
                let row = self.combined_row(j);
                let abs = self.dropped + j as u64;
                for (x, cell) in row.cells.iter().enumerate() {
                    if cell.flags & attr::WIDE_TAIL != 0 {
                        continue;
                    }
                    match self.cluster_str(cell.ch) {
                        Some(s) => {
                            for (k, ch) in s.chars().enumerate() {
                                let _ = k;
                                chars.push(fold(ch));
                                pos.push((abs, x));
                            }
                        }
                        None => {
                            chars.push(fold(cell.ch));
                            pos.push((abs, x));
                        }
                    }
                }
                if row.wrapped && j + 1 < total {
                    j += 1;
                } else {
                    break;
                }
            }
            if chars.len() >= needle.len() {
                let mut s = 0usize;
                while s + needle.len() <= chars.len() {
                    if chars[s..s + needle.len()] == needle[..] {
                        hits.push(make_hit(&pos[s..s + needle.len()]));
                        s += needle.len().max(1);
                    } else {
                        s += 1;
                    }
                }
            }
            i = j + 1;
        }
        hits
    }
}

fn make_hit(pos: &[(u64, usize)]) -> Hit {
    let mut spans: Vec<(u64, usize, usize)> = Vec::new();
    for &(line, col) in pos {
        match spans.last_mut() {
            Some(last) if last.0 == line => {
                if col > last.2 {
                    last.2 = col;
                }
            }
            _ => spans.push((line, col, col)),
        }
    }
    Hit { spans }
}
