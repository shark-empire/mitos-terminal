use vte::{Params, Perform, Parser};
use std::time::Duration; 
use std::collections::{HashMap, HashSet, VecDeque};
use mitos_utils::ipc::{RichWidget, OSC_WIDGET, OSC_NEW_BLOCK};
use crate::pkg_bridge::detect_missing_command;

const MAX_SCROLLBACK_BLOCKS: usize = 5000; 

#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub character: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub intensity: f32,
    pub is_error: bool,
    pub link_id: u32,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            fg: [200, 200, 200],
            bg: [20, 20, 25],
            intensity: 0.0,
            is_error: false,
            link_id: 0,
        }
    }
}

#[derive(Clone)]
pub struct ExecutionBlock {
    pub prompt: String,
    pub cells: Vec<Vec<Cell>>, 
    pub widgets: HashMap<(usize, usize), RichWidget>, 
    pub is_active: bool, 
    pub start_time: std::time::Instant,
    pub ghost_text: Option<String>, 
    pub duration: Option<Duration>, 
    pub link_table: Vec<String>,
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
            link_table: Vec::new(),
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
    pub blocks: VecDeque<ExecutionBlock>, 
    pub current_block: ExecutionBlock, 
    pub cursor_x: usize,
    pub cursor_y: usize, 
    // FIX: Wrapped in Option to allow taking it out during `process()` 
    // to satisfy the borrow checker (vte requires &mut Parser and &mut Performer simultaneously)
    parser: Option<Parser>,
    current_fg: [u8; 3],
    current_bg: [u8; 3],
    default_fg: [u8; 3],
    default_bg: [u8; 3],
    pub prompt_color: [u8; 3],
    pending_lookups: Vec<String>,
    already_suggested: HashSet<String>,
    pending_autocomplete: Option<String>, 
    pending_closed_blocks: Vec<ExecutionBlock>,
    pub any_hot: bool,
    current_link_id: u32,
}

impl TerminalGrid {
    pub fn new(cols: usize, _rows: usize) -> Self {
        let initial_block = ExecutionBlock::new("mitos@user:~$ ".to_string(), cols);
        let default_fg = [200, 200, 200];
        let default_bg = [20, 20, 25];
        Self {
            cols,
            blocks: VecDeque::with_capacity(MAX_SCROLLBACK_BLOCKS),
            current_block: initial_block,
            cursor_x: 0,
            cursor_y: 0,
            parser: Some(Parser::new()), // FIX: Initialize as Some
            current_fg: default_fg,
            current_bg: default_bg,
            default_fg,
            default_bg,
            prompt_color: [85, 255, 85], 
            pending_lookups: Vec::new(),
            already_suggested: HashSet::new(),
            pending_autocomplete: None,
            pending_closed_blocks: Vec::new(),
            any_hot: false,
            current_link_id: 0,
        }
    }

    fn push_block(&mut self, block: ExecutionBlock) {
        if self.blocks.len() >= MAX_SCROLLBACK_BLOCKS {
            self.blocks.pop_front(); 
        }
        self.blocks.push_back(block);
    }

    pub fn apply_theme(&mut self, fg: [u8; 3], bg: [u8; 3], prompt: [u8; 3]) {
        let old_fg = self.default_fg;
        let old_bg = self.default_bg;
        self.default_fg = fg;
        self.default_bg = bg;
        self.current_fg = fg;
        self.current_bg = bg;
        self.prompt_color = prompt;
        
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
        // FIX: Take the parser out to bypass the borrow checker, then put it back
        let mut parser = self.parser.take().unwrap();
        for &byte in bytes {
            parser.advance(self, byte);
        }
        self.parser = Some(parser);

        ProcessResult {
            missing_commands: std::mem::take(&mut self.pending_lookups),
            autocomplete_request: self.pending_autocomplete.take(),
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

    pub fn tick(&mut self, dt: f32) {
        if !self.any_hot { return; }
        let cool = dt * 2.2; 
        let mut still_hot = false;
        for block in self.blocks.iter_mut().chain(std::iter::once(&mut self.current_block)) {
            for row in block.cells.iter_mut() {
                for cell in row.iter_mut() {
                    if cell.intensity > 0.0 {
                        cell.intensity = (cell.intensity - cool).max(0.0);
                        still_hot |= cell.intensity > 0.0;
                    }
                }
            }
        }
        self.any_hot = still_hot;
    }

    pub fn spike_row_error(&mut self, row: usize) {
        if let Some(r) = self.current_block.cells.get_mut(row) {
            for c in r.iter_mut() {
                if c.character != ' ' { 
                    c.intensity = 1.0; 
                    c.is_error = true;
                }
            }
            self.any_hot = true;
        }
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
                self.spike_row_error(self.cursor_y); 
            }
        }
    }

    pub fn resize(&mut self, new_cols: usize) {
        if new_cols == self.cols || new_cols == 0 { return; }
        
        for block in &mut self.blocks {
            Self::reflow_block(block, new_cols);
        }
        Self::reflow_block(&mut self.current_block, new_cols);
        
        self.cols = new_cols;
        
        self.cursor_y = self.cursor_y.min(self.current_block.cells.len().saturating_sub(1));
        self.cursor_x = self.cursor_x.min(new_cols.saturating_sub(1));
    }

    fn reflow_block(block: &mut ExecutionBlock, new_cols: usize) {
        let old_cols = block.cells.first().map(|r| r.len()).unwrap_or(0);
        let mut flat: Vec<Cell> = Vec::new();
        
        for row in &block.cells {
            let mut end = row.len();
            while end > 0 && row[end-1].character == ' ' {
                end -= 1;
            }
            flat.extend_from_slice(&row[..end]);
            
            if end < old_cols {
                flat.push(Cell { character: '\n', ..Default::default() });
            }
        }
        
        block.cells.clear();
        let mut current_row = Vec::with_capacity(new_cols);
        
        for cell in flat {
            if cell.character == '\n' {
                while current_row.len() < new_cols {
                    current_row.push(Cell::default());
                }
                block.cells.push(std::mem::take(&mut current_row));
                current_row = Vec::with_capacity(new_cols);
            } else {
                current_row.push(cell);
                if current_row.len() == new_cols {
                    block.cells.push(std::mem::take(&mut current_row));
                    current_row = Vec::with_capacity(new_cols);
                }
            }
        }
        
        if !current_row.is_empty() {
            while current_row.len() < new_cols {
                current_row.push(Cell::default());
            }
            block.cells.push(current_row);
        }
        
        if block.cells.is_empty() {
            block.cells.push(vec![Cell::default(); new_cols]);
        }
    }
}

/// Rejoin OSC params from `start` onward with `;`. vte's OSC parser
/// splits the whole sequence on every semicolon, but OSC payloads (a
/// RichWidget's JSON, a hyperlink URI, a prompt string) can
/// legitimately contain one -- without this, anything after the
/// first embedded `;` silently vanishes.
fn rejoin_osc_params(params: &[&[u8]], start: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, part) in params[start..].iter().enumerate() {
        if i > 0 {
            out.push(b';');
        }
        out.extend_from_slice(part);
    }
    out
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
            intensity: 1.0, 
            is_error: false, 
            link_id: self.current_link_id,
        };
        self.any_hot = true;
        self.cursor_x += 1;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 | 0x7F => { 
                if self.cursor_x > 0 { 
                    self.cursor_x -= 1; 
                    if let Some(row) = self.current_block.cells.get_mut(self.cursor_y) {
                        if self.cursor_x < row.len() {
                            row[self.cursor_x] = Cell::default();
                        }
                    }
                    if let Some(g) = &mut self.current_block.ghost_text {
                        g.pop();
                        if g.is_empty() { self.current_block.ghost_text = None; }
                    }
                }
            }
            // Line 360
             0x0A..=0x0C => { 
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
            let y = params.iter().next().and_then(|p| p.first()).unwrap_or(&1).saturating_sub(1) as usize;
            let x = params.iter().nth(1).and_then(|p| p.first()).unwrap_or(&1).saturating_sub(1) as usize;
            
            self.cursor_y = y;
            self.cursor_x = x.min(self.cols.saturating_sub(1));
            
            while self.current_block.cells.len() <= self.cursor_y {
                self.current_block.add_row(self.cols);
            }
        }
        else if action == 'K' { 
            let mode = params.iter().next().and_then(|p| p.first()).unwrap_or(&0);
            if let Some(row) = self.current_block.cells.get_mut(self.cursor_y) {
                match mode {
                    0 => { 
                        // Lines 409 & 426
                        for cell in &mut row[self.cursor_x..] { 
                        *cell = Cell::default(); 
                           }
                    }
                    1 => { 
                        for i in 0..=self.cursor_x.min(row.len().saturating_sub(1)) { row[i] = Cell::default(); }
                    }
                    2 => { 
                        for cell in row.iter_mut() { *cell = Cell::default(); }
                    }
                    _ => {}
                }
            }
        }
        else if action == 'J' { 
            let mode = params.iter().next().and_then(|p| p.first()).unwrap_or(&0);
            match mode {
                0 => { 
                    if let Some(row) = self.current_block.cells.get_mut(self.cursor_y) {
                        // Lines 409 & 426
for cell in &mut row[self.cursor_x..] { 
    *cell = Cell::default(); 
}

                    }
                    self.current_block.cells.truncate(self.cursor_y + 1);
                }
                1 => { 
                    for i in 0..self.cursor_y {
                        if let Some(row) = self.current_block.cells.get_mut(i) {
                            for cell in row.iter_mut() { *cell = Cell::default(); }
                        }
                    }
                    if let Some(row) = self.current_block.cells.get_mut(self.cursor_y) {
                        for i in 0..=self.cursor_x.min(row.len().saturating_sub(1)) { row[i] = Cell::default(); }
                    }
                }
                2 | 3 => { 
                    if *mode == 3 {
                        self.blocks.clear(); 
                    }
                    self.current_block.cells.clear();
                    self.current_block.add_row(self.cols);
                    self.cursor_x = 0;
                    self.cursor_y = 0;
                }
                _ => {}
            }
        }
    }

    fn hook(&mut self, _: &Params, _: &[u8], _: bool, _: char) {}
    fn put(&mut self, _: u8) {}
    fn unhook(&mut self) {}
    
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() { return; }
        
        if params.len() >= 3 {
            if let Ok(cmd) = std::str::from_utf8(params[0]) {
                if cmd == "8" {
                    let uri_bytes = rejoin_osc_params(params, 2);
                    let uri = std::str::from_utf8(&uri_bytes).unwrap_or("");
                    if uri.is_empty() {
                        self.current_link_id = 0; 
                    } else {
                        self.current_link_id = (self.current_block.link_table.len() as u32) + 1;
                        self.current_block.link_table.push(uri.to_string());
                    }
                    return; 
                }
            }
        }

        if let Ok(ps) = std::str::from_utf8(params[0]) {
            if ps == OSC_WIDGET && params.len() >= 2 {
                let payload = rejoin_osc_params(params, 1);
                if let Ok(pt) = std::str::from_utf8(&payload) {
                    if let Ok(widget) = serde_json::from_str::<RichWidget>(pt) {
                        self.current_block.widgets.insert((self.cursor_y, self.cursor_x), widget);
                    }
                }
            }
            else if ps == OSC_NEW_BLOCK {
                let prompt = if params.len() >= 2 {
                    let prompt_bytes = rejoin_osc_params(params, 1);
                    std::str::from_utf8(&prompt_bytes).unwrap_or("mitos@user:~$ ").to_string()
                } else {
                    "mitos@user:~$ ".to_string()
                };
                
                self.current_block.is_active = false;
                self.current_block.duration = Some(self.current_block.start_time.elapsed());
                
                let old_block = std::mem::replace(&mut self.current_block, ExecutionBlock::new(prompt, self.cols));
                
                self.push_block(old_block.clone()); 
                self.pending_closed_blocks.push(old_block);
                
                self.cursor_x = 0;
                self.cursor_y = 0;
                self.current_link_id = 0; 
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

#[inline]
pub fn hash3(a: u64, b: u64, c: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ b.wrapping_mul(0x517C_C1B7_2722_0A95)
        ^ c.wrapping_mul(0x2545_F491_4F6C_DD1D);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}
