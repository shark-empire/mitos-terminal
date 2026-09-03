use vte::{Params, Perform, Parser};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --- 1. Standard Cell Definition ---
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

// --- 2. MROP (MITOS Rich Output Protocol) Definitions ---
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum RichWidget {
    #[serde(rename = "button")]
    Button { label: String, cmd: String },
    #[serde(rename = "progress")]
    Progress { percent: f32, color: Option<String> },
    #[serde(rename = "sparkline")]
    Sparkline { data: Vec<f32> },
}

// --- 3. Execution Block (The "Card" System) ---
pub struct ExecutionBlock {
    pub prompt: String,
    pub cells: Vec<Vec<Cell>>, 
    pub widgets: HashMap<(usize, usize), RichWidget>, 
    pub is_active: bool, 
    pub start_time: std::time::Instant,
}

impl ExecutionBlock {
    pub fn new(prompt: String, cols: usize) -> Self {
        Self {
            prompt,
            cells: vec![vec![Cell::default(); cols]], // Start with one empty row
            widgets: HashMap::new(),
            is_active: true,
            start_time: std::time::Instant::now(),
        }
    }
    
    pub fn add_row(&mut self, cols: usize) {
        self.cells.push(vec![Cell::default(); cols]);
    }
}

// --- 4. Main Terminal Grid Engine ---
pub struct TerminalGrid {
    pub cols: usize,
    pub blocks: Vec<ExecutionBlock>, // Historical Blocks
    pub current_block: ExecutionBlock, // Active Block
    pub cursor_x: usize,
    pub cursor_y: usize, 
    parser: Parser,
    current_fg: [u8; 3],
    current_bg: [u8; 3],
}

impl TerminalGrid {
    // Note: Kept `rows` in signature so your main.rs doesn't break, 
    // but blocks now grow dynamically based on output!
    pub fn new(cols: usize, _rows: usize) -> Self {
        let initial_block = ExecutionBlock::new("mitos@user:~$ ".to_string(), cols);
        Self {
            cols,
            blocks: Vec::new(),
            current_block: initial_block,
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
        }

        // Auto-grow the block if the cursor moves past existing rows
        while self.current_block.cells.len() <= self.cursor_y {
            self.current_block.add_row(self.cols);
        }

        self.current_block.cells[self.cursor_y][self.cursor_x] = Cell {
            character: c,
            fg: self.current_fg,
            bg: self.current_bg,
        };
        self.cursor_x += 1;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => { // Backspace
                if self.cursor_x > 0 { 
                    self.cursor_x -= 1; 
                }
            }
            0x0A | 0x0B | 0x0C => { // Line feed
                self.cursor_y += 1;
                while self.current_block.cells.len() <= self.cursor_y {
                    self.current_block.add_row(self.cols);
                }
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
                        35 => self.current_fg = [255, 85, 255],  // Magenta
                        36 => self.current_fg = [85, 255, 255],  // Cyan
                        37 => self.current_fg = [200, 200, 200], // White/Light Grey
                        _ => {}
                    }
                }
            }
        } 
        // Handle Cursor Movement (e.g., \x1b[H)
        else if action == 'H' || action == 'f' {
            let y = params.iter().next().and_then(|p| p.get(0)).unwrap_or(&1).saturating_sub(1) as usize;
            let x = params.iter().nth(1).and_then(|p| p.get(0)).unwrap_or(&1).saturating_sub(1) as usize;
            
            self.cursor_y = y;
            self.cursor_x = x.min(self.cols.saturating_sub(1));
            
            while self.current_block.cells.len() <= self.cursor_y {
                self.current_block.add_row(self.cols);
            }
        }
        // Handle Erase in Display (e.g., the `clear` command)
        else if action == 'J' {
            let mode = params.iter().next().and_then(|p| p.get(0)).unwrap_or(&0);
            if *mode == 2 || *mode == 3 {
                self.current_block.cells.clear();
                self.current_block.add_row(self.cols);
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
        }
    }

    fn hook(&mut self, _: &Params, _: &[u8], _: bool, _: char) {}
    fn put(&mut self, _: u8) {}
    fn unhook(&mut self) {}
    
    // --- THE MAGIC: MROP & Block Management ---
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() { return; }
        
        if let Ok(ps) = std::str::from_utf8(params[0]) {
            // 1. MROP Widget Injection
            if ps == "MITOS_WIDGET" && params.len() >= 2 {
                if let Ok(pt) = std::str::from_utf8(params[1]) {
                    if let Ok(widget) = serde_json::from_str::<RichWidget>(pt) {
                        self.current_block.widgets.insert(
                            (self.cursor_y, self.cursor_x), 
                            widget
                        );
                    }
                }
            }
            // 2. Execution Block Finalization
            else if ps == "MITOS_NEW_BLOCK" {
                let prompt = if params.len() >= 2 {
                    std::str::from_utf8(params[1]).unwrap_or("mitos@user:~$ ").to_string()
                } else {
                    "mitos@user:~$ ".to_string()
                };
                
                // Move current block to history
                self.current_block.is_active = false;
                let old_block = std::mem::replace(
                    &mut self.current_block, 
                    ExecutionBlock::new(prompt, self.cols)
                );
                self.blocks.push(old_block);
                
                // Reset cursor for the new block
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
        }
    }
    
    fn esc_dispatch(&mut self, _: &[u8], _: bool, _: u8) {}
}
