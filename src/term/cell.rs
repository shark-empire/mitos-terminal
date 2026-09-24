//! Cell, colour, attribute and row types.

/// A colour reference. `Default` means "theme foreground/background";
/// `Indexed` is a palette slot (0-255); `Rgb` is 24-bit.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Cell attribute bits (`Cell::flags`).
pub mod attr {
    pub const BOLD: u16 = 1 << 0;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const BLINK: u16 = 1 << 4;
    pub const INVERSE: u16 = 1 << 5;
    pub const HIDDEN: u16 = 1 << 6;
    pub const STRIKE: u16 = 1 << 7;
    /// Left half of a double-width character.
    pub const WIDE: u16 = 1 << 8;
    /// Right half (spacer) of a double-width character.
    pub const WIDE_TAIL: u16 = 1 << 9;
    pub const DOUBLE_UL: u16 = 1 << 10;
    pub const CURLY_UL: u16 = 1 << 11;
    pub const DOTTED_UL: u16 = 1 << 12;
    pub const DASHED_UL: u16 = 1 << 13;
    pub const ANY_UL: u16 = UNDERLINE | DOUBLE_UL | CURLY_UL | DOTTED_UL | DASHED_UL;
}

/// Row-level marks (`Row::marks`). They travel with the row through
/// scrollback and reflow.
pub mod mark {
    /// First row of a MITOS execution block (`OSC MITOS_NEW_BLOCK`).
    pub const BLOCK: u8 = 1;
    /// OSC 133;A — shell prompt starts here.
    pub const PROMPT: u8 = 2;
    /// OSC 133;C — command output starts here.
    pub const OUTPUT: u8 = 4;
    /// A MROP widget is anchored to this row.
    pub const WIDGET: u8 = 8;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    /// Underline colour (SGR 58); `Default` = use the foreground.
    pub ul: Color,
    pub flags: u16,
    /// Index into the terminal's hyperlink table (0 = none).
    pub link: u16,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: Color::Default,
        bg: Color::Default,
        ul: Color::Default,
        flags: 0,
        link: 0,
    };

    /// True for a default-coloured, attribute-free space (safe to trim).
    #[inline]
    pub fn is_blank(&self) -> bool {
        *self == Cell::BLANK
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

/// The "pen": attributes applied to newly printed characters.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub ul: Color,
    pub flags: u16,
}

impl Pen {
    #[inline]
    pub fn cell(&self, ch: char, link: u16) -> Cell {
        Cell { ch, fg: self.fg, bg: self.bg, ul: self.ul, flags: self.flags, link }
    }

    /// A blank cell that keeps the pen's background (BCE — background colour erase).
    #[inline]
    pub fn blank_bce(&self) -> Cell {
        Cell { ch: ' ', fg: Color::Default, bg: self.bg, ul: Color::Default, flags: 0, link: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Soft-wrapped: this row continues on the next one.
    pub wrapped: bool,
    pub marks: u8,
    /// Command-record id (0 = none) for rows that start a shell prompt.
    pub tag: u32,
}

impl Row {
    pub fn new(cols: usize) -> Row {
        Row { cells: vec![Cell::BLANK; cols], wrapped: false, marks: 0, tag: 0 }
    }

    pub fn filled(cols: usize, cell: Cell) -> Row {
        Row { cells: vec![cell; cols], wrapped: false, marks: 0, tag: 0 }
    }

    /// Drop trailing default blanks (used when a row enters scrollback).
    pub fn trim_blank(&mut self) {
        while let Some(last) = self.cells.last() {
            if last.is_blank() {
                self.cells.pop();
            } else {
                break;
            }
        }
    }

    /// Cell at `x`, treating anything past the end (trimmed rows) as blank.
    #[inline]
    pub fn get(&self, x: usize) -> Cell {
        self.cells.get(x).copied().unwrap_or(Cell::BLANK)
    }

    pub fn reset(&mut self, blank: Cell) {
        for c in self.cells.iter_mut() {
            *c = blank;
        }
        self.wrapped = false;
        self.marks = 0;
        self.tag = 0;
    }
}
