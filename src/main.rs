mod grid;
mod pty;

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::io::{Read, Write}; // Added Write for PTY writer
use tokio::sync::mpsc;
use portable_ty::MasterPty;
use grid::{TerminalGrid, ExecutionBlock, RichWidget}; // Import new grid types
use pty::MitosPty;

struct MitosTerminalApp {
    grid: Arc<Mutex<TerminalGrid>>,
    input_tx: mpsc::Sender<u8>,
}

impl MitosTerminalApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, mut rx) = mpsc::channel::<u8>(1024);
        let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(1024);

        let grid = Arc::new(Mutex::new(TerminalGrid::new(80, 24)));
        let ui_grid = Arc::clone(&grid);

        // Background Thread: PTY Reader/Writer
        std::thread::spawn(move || {
            let pty = MitosPty::new(80, 24).expect("Failed to create PTY");
            let mut reader = pty.master.try_clone_reader().unwrap();
            let mut writer = pty.master.take_writer();
            
            // Reader Loop
            std::thread::spawn(move || {
                let mut buf = [0; 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => { let _ = pty_tx.blocking_send(buf[..n].to_vec()); }
                        Err(_) => break,
                    }
                }
            });

            // Writer Loop (Async)
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                while let Some(byte) = rx.recv().await {
                    let _ = writer.write_all(&[byte]);
                }
            });
        });

        // Grid Update Loop
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                while let Some(bytes) = pty_rx.recv().await {
                    if let Ok(mut g) = ui_grid.lock() {
                        g.process(&bytes);
                    }
                }
            });
        });

        Self { grid, input_tx: tx }
    }
}

impl eframe::App for MitosTerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let available_width = ui.available_width();
            
            // Full-screen invisible rect to capture focus and keyboard events globally
            let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click());
            response.request_focus();

            egui::ScrollArea::vertical()
                .stick_to_bottom(true) // Auto-scroll to bottom when new output arrives
                .show(ui, |ui| {
                    let grid = self.grid.lock().unwrap();
                    
                    // Render Historical Blocks (Completed Commands)
                    for block in &grid.blocks {
                        render_block(ui, block, available_width, &self.input_tx, false, 0, 0);
                    }
                    
                    // Render Current Active Block (Live Command)
                    render_block(
                        ui, 
                        &grid.current_block, 
                        available_width, 
                        &self.input_tx, 
                        true, 
                        grid.cursor_y, 
                        grid.cursor_x
                    );
                });

            // Keyboard Input Handling
            if response.has_focus() {
                ctx.input(|i| {
                    for event in &i.events {
                        match event {
                            egui::Event::Text(text) => {
                                for c in text.chars() {
                                    let _ = self.input_tx.try_send(c as u8);
                                }
                            }
                            egui::Event::Key { key, pressed: true, .. } => {
                                match key {
                                    egui::Key::Enter => { let _ = self.input_tx.try_send(0x0D); }
                                    egui::Key::Backspace => { let _ = self.input_tx.try_send(0x08); }
                                    egui::Key::Escape => { let _ = self.input_tx.try_send(0x1B); }
                                    egui::Key::ArrowUp => { for b in [27, 91, 65] { let _ = self.input_tx.try_send(b); } }
                                    egui::Key::ArrowDown => { for b in [27, 91, 66] { let _ = self.input_tx.try_send(b); } }
                                    egui::Key::ArrowRight => { for b in [27, 91, 67] { let _ = self.input_tx.try_send(b); } }
                                    egui::Key::ArrowLeft => { for b in [27, 91, 68] { let _ = self.input_tx.try_send(b); } }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        
        // Force continuous redraws (for cursor blinking / active shell output)
        ctx.request_repaint();
    }
}

// --- UI Rendering Helpers ---

fn render_block(
    ui: &mut egui::Ui, 
    block: &ExecutionBlock, 
    available_width: f32, 
    input_tx: &mpsc::Sender<u8>,
    is_active: bool,
    cursor_y: usize,
    cursor_x: usize
) {
    // Draw the "Card" Frame
    egui::Frame::new()
        .fill(egui::Color32::from_gray(22)) // Slightly darker background for cards
        .rounding(6.0)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(45)))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(available_width - 16.0);
            
            // 1. Render Prompt
            ui.label(
                egui::RichText::new(&block.prompt)
                    .color(egui::Color32::from_rgb(85, 255, 85))
                    .strong()
                    .monospace()
            );
            ui.add_space(4.0);

            // 2. Render Cells & Widgets
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let char_size = ui.painter().layout_no_wrap("M".to_string(), font_id.clone(), egui::Color32::WHITE).size();
            let space_width = char_size.x;
            let line_height = char_size.y;

            for (y, row) in block.cells.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    
                    for (x, cell) in row.iter().enumerate() {
                        // Check for MROP Widget at this exact coordinate
                        if let Some(widget) = block.widgets.get(&(y, x)) {
                            render_widget(ui, widget, input_tx);
                            continue;
                        }

                        let is_cursor = is_active && x == cursor_x && y == cursor_y;
                        
                        let mut fg = egui::Color32::from_rgb(cell.fg[0], cell.fg[1], cell.fg[2]);
                        let mut bg = egui::Color32::from_rgb(cell.bg[0], cell.bg[1], cell.bg[2]);
                        
                        // Invert colors for the cursor block
                        if is_cursor {
                            std::mem::swap(&mut fg, &mut bg);
                        }

                        // Allocate exact space for monospace alignment
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(space_width, line_height), 
                            egui::Sense::hover()
                        );

                        // Draw background if it's the cursor or ANSI changed the background color
                        if is_cursor || cell.bg != [20, 20, 25] {
                            ui.painter().rect_filled(rect, 0.0, bg);
                        }

                        // Draw the character
                        if cell.character != ' ' && cell.character != '\0' {
                            ui.painter().text(
                                rect.center(), 
                                egui::Align2::CENTER_CENTER, 
                                cell.character.to_string(), 
                                font_id.clone(), 
                                fg
                            );
                        }
                    }
                });
            }
        });
    ui.add_space(8.0); // Gap between cards
}

fn render_widget(ui: &mut egui::Ui, widget: &RichWidget, input_tx: &mpsc::Sender<u8>) {
    match widget {
        RichWidget::Button { label, cmd } => {
            if ui.button(egui::RichText::new(label).strong()).clicked() {
                // Inject the command into the terminal input stream!
                let cmd_bytes = format!("{}\n", cmd).into_bytes();
                for b in cmd_bytes {
                    let _ = input_tx.try_send(b);
                }
            }
        }
        RichWidget::Progress { percent, color: _ } => {
            ui.add(egui::ProgressBar::new(*percent).show_percentage());
        }
        RichWidget::Sparkline { data } => {
            ui.label("📈 [Sparkline Graph]"); 
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("MITOS Terminal"),
        ..Default::default()
    };
    eframe::run_native(
        "mitos-terminal",
        options,
        Box::new(|cc| Ok(Box::new(MitosTerminalApp::new(cc)))),
    )
}
