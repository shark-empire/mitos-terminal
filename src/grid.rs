use vte::{Params, Perform, Parser};
use std::time::{Duration, Instant}; 
use std::collections::{HashMap, HashSet};
use mitos_utils::ipc::{RichWidget, OSC_WIDGET, OSC_NEW_BLOCK};
use crate::pkg_bridge::detect_missing_command;

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
            fg: [200, 200, 200], 
            bg: [20, 20, 25],    
        }
    }
}

// Added Clone so we can push to both history and the notification queue
#[derive(Clone)]
pub struct ExecutionBlock {
    pub prompt: String,
    pub cells: Vec<Vec<Cell>>, 
    pub widgets: HashMap<(usize, usize), RichWidget>, 
    pub is_active: bool, 
    pub start_time: std::time::Instant,
    pub ghost_text: Option<String>, 
    pub duration: Option<Duration>, 
}

impl ExecutionBlock {
    pub fn new(prompt: String, cols: usize) -> Self {
        Self {
            prompt,
            cells: vec![vec![Cell::default(); cols]], 
            widgets: HashMap::new(),
            is_active: true,
            start_time: std::time::Instant::now(),
            ghost_text: None,
            duration: None,
        }
    }
    
    pub fn add_row(&mut self, cols: usize) {
        self.cells.push(vec![Cell::default(); cols]);
    }
}

pub struct ProcessResult {
    pub missing_commands: Vec<String>,
    pub autocomplete_request: Option<String>,
    pub closed_blocks: Vec<ExecutionBlock>, 
}

pub struct TerminalGrid {
    pub cols: usize,
    pub blocks: Vec<ExecutionBlock>, 
    pub current_block: ExecutionBlock, 
    pub cursor_x: usize,
    pub cursor_y: usize, 
    parser: Parser,
    current_fg: [u8; 3],
    current_bg: [u8; 3],
    default_fg: [u8; 3],
    default_bg: [u8; 3],
    pub prompt_color: [u8; 3],
    pending_lookups: Vec<String>,
    already_suggested: HashSet<String>,
    pending_autocomplete: Option<String>, 
    pending_closed_blocks: Vec<ExecutionBlock>, // Added missing field
}

impl TerminalGrid {
    pub fn new(cols: usize, _rows: usize) -> Self {
        let initial_block = ExecutionBlock::new("mitos@user:~$ ".to_string(), cols);
        let default_fg = [200, 200, 200];
        let default_bg = [20, 20, 25];
        Self {
            cols,
            blocks: Vec::new(),
            current_block: initial_block,
            cursor_x: 0,
            cursor_y: 0,
            parser: Parser::new(),
            current_fg: default_fg,
            current_bg: default_bg,
            default_fg,
            default_bg,
            prompt_color: [85, 255, 85], // Default MITOS Green
            pending_lookups: Vec::new(),
            already_suggested: HashSet::new(),
            pending_autocomplete: None,
            pending_closed_blocks: Vec::new(), // Initialized here
        }
    }

    /// Fully applies a new theme, retroactively updating all existing cells
    /// that match the old default colors, as well as setting the new defaults.
    pub fn apply_theme(&mut self, fg: [u8; 3], bg: [u8; 3], prompt: [u8; 3]) {
        let old_fg = self.default_fg;
        let old_bg = self.default_bg;
        self.default_fg = fg;
        self.default_bg = bg;
        self.current_fg = fg;
        self.current_bg = bg;
        self.prompt_color = prompt;
        
        // Retroactively repaint existing history
        for block in &mut self.blocks {
            for row in &mut block.cells {
                for cell in row.iter_mut() {
                    if cell.fg == old_fg { cell.fg = fg; }
                    if cell.bg == old_bg { cell.bg = bg; }
                }
            }
        }
        for row in &mut self.current_block.cells {
            for cell in row.iter_mut() {
                if cell.fg == old_fg { cell.fg = fg; }
                if cell.bg == old_bg { cell.bg = bg; }
            }
        }
    }

    pub fn set_ghost_text(&mut self, text: Option<String>) {
        self.current_block.ghost_text = text;
    }

    pub fn process(&mut self, bytes: &[u8]) -> ProcessResult {
        for &byte in bytes {
            self.parser.advance(self, byte);
        }
        ProcessResult {
            missing_commands: std::mem::take(&mut self.pending_lookups),
            autocomplete_request: self.pending_autocomplete.take(),
            // Fixed typo: was sod::mem::take
            closed_blocks: std::mem::take(&mut self.pending_closed_blocks), 
        }
    }

    pub fn snapshot_text(&self) -> String {
        let mut out = String::new();
        let extract = |block: &ExecutionBlock, out: &mut String| {
            out.push_str(&format!("── {}{}\n", 
                block.prompt, 
                if block.is_active { " (active)" } else { "" }
            ));
            for row in &block.cells {
                let line: String = row.iter().map(|c| c.character).collect();
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out.push('\n');
        };

        for block in &self.blocks {
            extract(block, &mut out);
        }
        extract(&self.current_block, &mut out);
        out
    }

    pub fn inject_widget(&mut self, widget: RichWidget) {
        self.cursor_y += 1;
        self.cursor_x = 0;

        while self.current_block.cells.len() <= self.cursor_y {
            self.current_block.add_row(self.cols);
        }

        self.current_block.widgets.insert((self.cursor_y, 0), widget);
    }

    fn check_line_for_missing_command(&mut self) {
        let Some(row) = self.current_block.cells.get(self.cursor_y) else {
            return;
        };
        let line: String = row.iter().map(|c| c.character).collect();

        if let Some(cmd) = detect_missing_command(line.trim_end()) {
            if self.already_suggested.insert(cmd.clone()) {
                self.pending_lookups.push(cmd);
            }
        }
    }
}

impl Perform for TerminalGrid {
    fn print(&mut self, c: char) {
        if self.cursor_x >= self.cols {
            self.cursor_x = 0;
            self.cursor_y += 1;
        }

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
            0x08 => { 
                if self.cursor_x > 0 { 
                    self.cursor_x -= 1; 
                    if let Some(g) = &mut self.current_block.ghost_text {
                        g.pop();
                        if g.is_empty() { self.current_block.ghost_text = None; }
                    }
                }
            }
            0x0A | 0x0B | 0x0C => { 
                self.check_line_for_missing_command();
                self.cursor_y += 1;
                while self.current_block.cells.len() <= self.cursor_y {
                    self.current_block.add_row(self.cols);
                }
                self.current_block.ghost_text = None; 
            }
            0x0D => { 
                self.cursor_x = 0;
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        if intermediates.is_empty() && action == 'm' {
            for param in params.iter() {
                for subparam in param {
                    match subparam {
                        0 => { self.current_fg = self.default_fg; self.current_bg = self.default_bg; }
                        31 => self.current_fg = [255, 85, 85],
                        32 => self.current_fg = [85, 255, 85],
                        33 => self.current_fg = [255, 255, 85],
                        34 => self.current_fg = [85, 85, 255],
                        35 => self.current_fg = [255, 85, 255],
                        36 => self.current_fg = [85, 255, 255],
                        37 => self.current_fg = [200, 200, 200],
                        _ => {}
                    }
                }
            }
        } 
        else if action == 'H' || action == 'f' {
            let y = params.iter().next().and_then(|p| p.get(0)).unwrap_or(&1).saturating_sub(1) as usize;
            let x = params.iter().nth(1).and_then(|p| p.get(0)).unwrap_or(&1).saturating_sub(1) as usize;
            
            self.cursor_y = y;
            self.cursor_x = x.min(self.cols.saturating_sub(1));
            
            while self.current_block.cells.len() <= self.cursor_y {
                self.current_block.add_row(self.cols);
            }
        }
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
    
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() { return; }
        
        if let Ok(ps) = std::str::from_utf8(params[0]) {
            if ps == OSC_WIDGET && params.len() >= 2 {
                if let Ok(pt) = std::str::from_utf8(params[1]) {
                    if let Ok(widget) = serde_json::from_str::<RichWidget>(pt) {
                        self.current_block.widgets.insert((self.cursor_y, self.cursor_x), widget);
                    }
                }
            }
            else if ps == OSC_NEW_BLOCK {
                let prompt = if params.len() >= 2 {
                    std::str::from_utf8(params[1]).unwrap_or("mitos@user:~$ ").to_string()
                } else {
                    "mitos@user:~$ ".to_string()
                };
                
                self.current_block.is_active = false;
                // Calculate how long the command took before we move it
                self.current_block.duration = Some(self.current_block.start_time.elapsed());
                
                let old_block = std::mem::replace(&mut self.current_block, ExecutionBlock::new(prompt, self.cols));
                
                // Push a clone to the visual history
                self.blocks.push(old_block.clone());
                // Queue the original for main.rs to process (e.g., for notifications)
                self.pending_closed_blocks.push(old_block);
                
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
            else if ps == "MITOS_AUTOCOMPLETE" && params.len() >= 2 {
                if let Ok(pt) = std::str::from_utf8(params[1]) {
                    self.pending_autocomplete = Some(pt.to_string());
                }
            }
        }
    }
    
    fn esc_dispatch(&mut self, _: &[u8], _: bool, _: u8) {}
}
