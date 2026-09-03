use vte::{Params, Perform, Parser};

#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub character: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            fg: [200, 200, 200], // Default MITOS light grey
            bg: [20, 20, 25],    // Default MITOS dark background
        }
    }
}

pub struct TerminalGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Vec<Cell>>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    parser: Parser,
    current_fg: [u8; 3],
    current_bg: [u8; 3],
}

impl TerminalGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cells = vec![vec![Cell::default(); cols]; rows];
        Self {
            cols,
            rows,
            cells,
            cursor_x: 0,
            cursor_y: 0,
            parser: Parser::new(),
            current_fg: [200, 200, 200],
            current_bg: [20, 20, 25],
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.parser.advance(self, byte);
        }
    }
}

impl Perform for TerminalGrid {
    fn print(&mut self, c: char) {
        if self.cursor_x >= self.cols {
            self.cursor_x = 0;
            self.cursor_y += 1;
            if self.cursor_y >= self.rows {
                self.cursor_y = self.rows - 1;
                // TODO: Implement scrollback buffer logic here
            }
        }
        self.cells[self.cursor_y][self.cursor_x] = Cell {
            character: c,
            fg: self.current_fg,
            bg: self.current_bg,
        };
        self.cursor_x += 1;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => { // Backspace
                if self.cursor_x > 0 { self.cursor_x -= 1; }
            }
            0x0A | 0x0B | 0x0C => { // Line feed
                self.cursor_y += 1;
                if self.cursor_y >= self.rows { self.cursor_y = self.rows - 1; }
            }
            0x0D => { // Carriage return
                self.cursor_x = 0;
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Handle ANSI colors (SGR)
        if intermediates.is_empty() && action == 'm' {
            for param in params.iter() {
                for subparam in param {
                    match subparam {
                        0 => { self.current_fg = [200, 200, 200]; self.current_bg = [20, 20, 25]; } // Reset
                        31 => self.current_fg = [255, 85, 85],   // Red
                        32 => self.current_fg = [85, 255, 85],   // Green
                        33 => self.current_fg = [255, 255, 85],  // Yellow
                        34 => self.current_fg = [85, 85, 255],   // Blue
                        _ => {}
                    }
                }
            }
        } 
        // Handle Cursor Movement (e.g., \x1b[H)
        else if action == 'H' || action == 'f' {
            let y = params.iter().next().and_then(|p| p.get(0)).unwrap_or(&1).saturating_sub(1) as usize;
            let x = params.iter().nth(1).and_then(|p| p.get(0)).unwrap_or(&1).saturating_sub(1) as usize;
            self.cursor_y = y.min(self.rows - 1);
            self.cursor_x = x.min(self.cols - 1);
        }
    }

    fn hook(&mut self, _: &Params, _: &[u8], _: bool, _: char) {}
    fn put(&mut self, _: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _: &[&[u8]], _: bool) {}
    fn esc_dispatch(&mut self, _: &[u8], _: bool, _: u8) {}
}
