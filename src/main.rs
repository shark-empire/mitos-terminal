mod grid;
mod pty;

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::io::Read;
use tokio::sync::mpsc;
use portable_pty::MasterPty;
use grid::TerminalGrid;
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
            let available_size = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::click());
            
            // Force focus to capture keyboard events globally in the window
            response.request_focus();

            let grid = self.grid.lock().unwrap();
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let char_size = ui.painter().layout_no_wrap("M".to_string(), font_id.clone(), egui::Color32::WHITE).size();

            // Render Grid
            for (y, row) in grid.cells.iter().enumerate() {
                for (x, cell) in row.iter().enumerate() {
                    let pos = egui::pos2(
                        rect.min.x + x as f32 * char_size.x,
                        rect.min.y + y as f32 * char_size.y
                    );
                    
                    let is_cursor = x == grid.cursor_x && y == grid.cursor_y;
                    
                    let (fg, bg) = if is_cursor {
                        (egui::Color32::from_rgb(cell.bg[0], cell.bg[1], cell.bg[2]), 
                         egui::Color32::from_rgb(cell.fg[0], cell.fg[1], cell.fg[2]))
                    } else {
                        (egui::Color32::from_rgb(cell.fg[0], cell.fg[1], cell.fg[2]), 
                         egui::Color32::from_rgb(cell.bg[0], cell.bg[1], cell.bg[2]))
                    };

                    ui.painter().rect_filled(egui::Rect::from_min_size(pos, char_size), 0.0, bg);

                    if cell.character != ' ' {
                        ui.painter().text(
                            pos + egui::vec2(char_size.x / 2.0, char_size.y / 2.0),
                            egui::Align2::CENTER_CENTER,
                            cell.character.to_string(),
                            font_id.clone(),
                            fg
                        );
                    }
                }
            }

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
