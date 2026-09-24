//! Character cell width (0, 1 or 2 columns).

use unicode_width::UnicodeWidthChar;

/// Number of terminal columns `c` occupies. Zero-width characters
/// (combining marks, ZWJ, variation selectors) return 0.
#[inline]
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x7f {
        return if u >= 0x20 { 1 } else { 0 };
    }
    if u < 0xa0 {
        return 0;
    }
    match c.width() {
        Some(w) => w.min(2),
        None => 0,
    }
}
