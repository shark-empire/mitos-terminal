use vte::{Params, Perform, Parser};
use std::collections::{HashMap, HashSet};

// Import the shared MROP types and constants from mitos-utils
use mitos_utils::ipc::{RichWidget, OSC_WIDGET, OSC_NEW_BLOCK};

// Pure text-pattern detection only (no I/O) — see pkg_bridge.rs. The
// actual mitos-pkgd lookup stays out of TerminalGrid entirely; this
// just reports candidate command names via `process`'s return value
// for main.rs to act on.
use crate::pkg_bridge::detect_missing_command;

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

// --- 2. Execution Block (The "Card" System) ---
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

// --- 3. Main Terminal Grid Engine ---
pub struct TerminalGrid {
    pub cols: usize,
    pub blocks: Vec<ExecutionBlock>, // Historical Blocks
    pub current_block: ExecutionBlock, // Active Block
    pub cursor_x: usize,
    pub cursor_y: usize, 
    parser: Parser,
    current_fg: [u8; 3],
    current_bg: [u8; 3],
    // The theme's resting colors — what an ANSI reset (`\x1b[0m`) and
    // freshly-printed text with no color codes yet applied fall back
    // to. Separate from current_fg/current_bg (which track whatever
    // color is *actively selected* right now) so `apply_theme` has
    // something to change that persists across resets — see that
    // method below.
    default_fg: [u8; 3],
    default_bg: [u8; 3],
    // Command names detected this `process()` call as "not found" —
    // drained and returned by `process`, then acted on by main.rs. See
    // pkg_bridge.rs for why the actual mitos-pkgd lookup doesn't happen
    // here.
    pending_lookups: Vec<String>,
    // Every command already offered an install suggestion this
    // session, so re-running the same missing command doesn't spam a
    // fresh button each time.
    already_suggested: HashSet<String>,
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
            pending_lookups: Vec::new(),
            already_suggested: HashSet::new(),
        }
    }

    /// Applies a new theme going forward: updates both the resting
    /// colors (so a later `\x1b[0m` reset returns to *this* theme, not
    /// the hardcoded original) and the currently-active colors (so text
    /// printed right after this call picks it up immediately, without
    /// needing an explicit reset first).
    ///
    /// Known limitation: this does not repaint characters already on
    /// screen — only new output. Real-time recoloring of existing
    /// history would mean storing a palette index per cell instead of
    /// literal RGB (like this grid does today) and remapping the
    /// palette, a bigger change than this integration needed to solve
    /// yet.
    pub fn apply_theme(&mut self, fg: [u8; 3], bg: [u8; 3]) {
        self.default_fg = fg;
        self.default_bg = bg;
        self.current_fg = fg;
        self.current_bg = bg;
    }

    pub fn process(&mut self, bytes: &[u8]) -> Vec<String> {
        for &byte in bytes {
            self.parser.advance(self, byte);
        }
        std::mem::take(&mut self.pending_lookups)
    }

    // ------------------------------------------------------------------
    // IPC Support: For mitos-system-monitor integration
    // ------------------------------------------------------------------

    /// Serialize the entire visible history into plain text for IPC scraping.
    /// This allows the System Monitor to "read" the terminal's memory over a Unix Socket.
    pub fn snapshot_text(&self) -> String {
        let mut out = String::new();

        // Helper closure to extract text from a specific block
        let extract = |block: &ExecutionBlock, out: &mut String| {
            out.push_str(&format!("── {}{}\n", 
                block.prompt, 
                if block.is_active { " (active)" } else { "" }
            ));
            for row in &block.cells {
                // Extract characters, trim trailing whitespace for clean reading
                let line: String = row.iter().map(|c| c.character).collect();
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out.push('\n');
        };

        // Dump historical blocks
        for block in &self.blocks {
            extract(block, &mut out);
        }
        // Dump current active block
        extract(&self.current_block, &mut out);

        out
    }

    /// Insert a rich widget pushed over IPC (e.g., a Kill button from the monitor).
    /// Each injected widget gets its own fresh row so it never clobbers user output.
    pub fn inject_widget(&mut self, widget: RichWidget) {
        // Move to the next line to avoid overwriting the user's active prompt
        self.cursor_y += 1;
        self.cursor_x = 0;

        // Auto-grow the block if the new row doesn't exist yet
        while self.current_block.cells.len() <= self.cursor_y {
            self.current_block.add_row(self.cols);
        }

        // Insert the widget at the start of the new row
        self.current_block.widgets.insert((self.cursor_y, 0), widget);
    }

    /// Checks the line the cursor is currently on (about to be
    /// completed by the line feed that triggered this call) for a
    /// "command not found"-shaped message, queuing the attempted
    /// command name into `pending_lookups` if so. Pure text matching —
    /// see pkg_bridge.rs for the actual daemon lookup, which happens
    /// later, outside TerminalGrid entirely.
    fn check_line_for_missing_command(&mut self) {
        let Some(row) = self.current_block.cells.get(self.cursor_y) else {
            return;
        };
        let line: String = row.iter().map(|c| c.character).collect();

        if let Some(cmd) = detect_missing_command(line.trim_end()) {
            // HashSet::insert returns true only the first time a value
            // is added — so this only queues a lookup the first time a
            // given missing command is seen this session.
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
            0x08 => { // Backspace
                if self.cursor_x > 0 { 
                    self.cursor_x -= 1; 
                }
            }
            0x0A | 0x0B | 0x0C => { // Line feed
                self.check_line_for_missing_command();
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
    
    // --- THE MAGIC: MROP & Block Management using mitos-utils ---
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() { return; }
        
        if let Ok(ps) = std::str::from_utf8(params[0]) {
            // 1. MROP Widget Injection (using shared OSC_WIDGET constant)
            if ps == OSC_WIDGET && params.len() >= 2 {
                if let Ok(pt) = std::str::from_utf8(params[1]) {
                    // Parse JSON into the shared RichWidget type
                    if let Ok(widget) = serde_json::from_str::<RichWidget>(pt) {
                        self.current_block.widgets.insert(
                            (self.cursor_y, self.cursor_x), 
                            widget
                        );
                    }
                }
            }
            // 2. Execution Block Finalization (using shared OSC_NEW_BLOCK constant)
            else if ps == OSC_NEW_BLOCK {
                let prompt = if params.len() >= 2 {
                    std::str::from_utf8(params[1]).unwrap_or("mitos@user:~$ ").to_string()
                } else {
                    "mitos@user:~$ ".to_string()
                };
                
                self.current_block.is_active = false;
                let old_block = std::mem::replace(
                    &mut self.current_block, 
                    ExecutionBlock::new(prompt, self.cols)
                );
                self.blocks.push(old_block);
                
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
        }
    }
    
    fn esc_dispatch(&mut self, _: &[u8], _: bool, _: u8) {}
}
