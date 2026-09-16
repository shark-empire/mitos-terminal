mod grid;
mod pty;
mod pkg_bridge;
mod fx; // <-- ADDED: Cinematic Background FX

// --- Standard Library ---
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};

// --- Third-Party ---
use arboard::Clipboard; // Semantic Clipboard (text/uri-list)
use eframe::egui;
use tokio::sync::mpsc;
use tokio::net::{UnixListener, UnixStream};
use notify::{Watcher, RecursiveMode, Config, RecommendedWatcher};

// --- Local / System Crates ---
use grid::{TerminalGrid, ExecutionBlock};
use pty::MitosPty;
use mitos_utils::ipc::{self, RichWidget, IpcRequest, IpcResponse};

// --- Constants ---
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_BG: [u8; 3] = [20, 20, 25];
const DEFAULT_FG: [u8; 3] = [200, 200, 200];
const DEFAULT_PROMPT: [u8; 3] = [85, 255, 85];

// ============================================================================
// APP STATE
// ============================================================================

struct MitosTerminalApp {
    grid: Arc<Mutex<TerminalGrid>>,
    input_tx: mpsc::Sender<u8>,
    resize_tx: std::sync::mpsc::Sender<(u16, u16)>,
    last_cols: u16,
    last_rows: u16,
    last_t: f64, // <-- ADDED: Frame clock for animations
}

// ============================================================================
// SYSTEM DAEMONS & IPC
// ============================================================================

fn spawn_ipc_server(grid: Arc<Mutex<TerminalGrid>>) {
    let socket_path = ipc::terminal_socket(std::process::id());
    let _ = std::fs::remove_file(&socket_path); 

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("ipc runtime");
        rt.block_on(async move {
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => { eprintln!("[mitos-terminal] IPC bind failed: {e}"); return; }
            };
            eprintln!("[mitos-terminal] IPC listening on {:?}", socket_path);

            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                
                let grid = Arc::clone(&grid);
                tokio::spawn(async move {
                    while let Ok(Some(req)) = ipc::ipc_recv::<IpcRequest>(&mut stream).await {
                        let resp = match req {
                            IpcRequest::GetTerminalBuffer => {
                                let g = grid.lock().unwrap();
                                IpcResponse::BufferData {
                                    pid: std::process::id(),
                                    prompt: g.current_block.prompt.clone(),
                                    text: g.snapshot_text(),
                                }
                            }
                            IpcRequest::InjectWidget { widget } => {
                                grid.lock().unwrap().inject_widget(widget);
                                IpcResponse::Ack
                            }
                            IpcRequest::ThemeChanged { bg, fg } => {
                                grid.lock().unwrap().apply_theme(fg, bg, DEFAULT_PROMPT);
                                IpcResponse::Ack
                            }
                            IpcRequest::AutoCompletePath { .. } => {
                                IpcResponse::AutoCompleteResult { suggestions: vec![] }
                            }
                            _ => IpcResponse::Ack,
                        };
                        
                        if ipc::ipc_send(&mut stream, &resp).await.is_err() { break; }
                    }
                });
            }
        });
    });
}

fn spawn_network_poller(grid: Arc<Mutex<TerminalGrid>>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("network poller runtime");
        rt.block_on(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
            loop {
                interval.tick().await;
                let socket = ipc::network_socket();
                
                if let Ok(mut stream) = UnixStream::connect(&socket).await {
                    let req = IpcRequest::GetNetworkStatus;
                    if ipc::ipc_send(&mut stream, &req).await.is_ok() {
                        if let Ok(Some(IpcResponse::NetworkStatus { is_captive_portal, .. })) = ipc::ipc_recv::<IpcResponse>(&mut stream).await {
                            if is_captive_portal {
                                inject_captive_portal_widget(&grid);
                            }
                        }
                    }
                }
            }
        });
    });
}

fn inject_captive_portal_widget(grid: &Arc<Mutex<TerminalGrid>>) {
    if let Ok(mut g_lock) = grid.lock() {
        let widget = RichWidget::Button {
            label: "🌐 Open Login Page".to_string(),
            cmd: "xdg-open http://captive.apple.com".to_string(),
        };
        let already_has = g_lock.current_block.widgets.values().any(|w| {
            matches!(w, RichWidget::Button { label, .. } if label.contains("Login Page"))
        });
        if !already_has {
            g_lock.inject_widget(widget);
        }
    }
}

// ============================================================================
// SETTINGS & THEME SYNC
// ============================================================================

fn spawn_settings_watcher(grid: Arc<Mutex<TerminalGrid>>) {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    let config_path = config_dir.join("mitos").join("home.conf");
    
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = RecommendedWatcher::new(move |res| {
            let _ = tx.send(res);
        }, Config::default()).expect("Failed to create file watcher");

        if let Ok(content) = std::fs::read_to_string(&config_path) {
            apply_home_conf_theme(&grid, &content);
        }

        let watch_dir = config_path.parent().unwrap().to_path_buf();
        let _ = watcher.watch(&watch_dir, RecursiveMode::NonRecursive);

        for res in rx {
            if let Ok(event) = res {
                let is_home_conf = event.paths.iter().any(|p| p.file_name() == Some(std::ffi::OsStr::new("home.conf")));
                if is_home_conf {
                    if let Ok(content) = std::fs::read_to_string(&config_path) {
                        apply_home_conf_theme(&grid, &content);
                    }
                }
            }
        }
    });
}

fn apply_home_conf_theme(grid: &Arc<Mutex<TerminalGrid>>, content: &str) {
    let (theme_mode, accent_color) = parse_home_conf(content);
    
    let (fg, bg) = match theme_mode.as_deref() {
        Some("light") => ([30, 30, 30], [245, 245, 245]),
        _ => (DEFAULT_FG, DEFAULT_BG), 
    };
    
    let prompt = parse_color(accent_color.as_deref(), DEFAULT_PROMPT);
    grid.lock().unwrap().apply_theme(fg, bg, prompt);
}

fn parse_home_conf(content: &str) -> (Option<String>, Option<String>) {
    let mut theme_mode = None;
    let mut accent_color = None;
    
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        
        let mut parts = line.splitn(2, '=');
        let key = parts.next().map(|s| s.trim());
        let val = parts.next().map(|s| s.trim());
        
        match key {
            Some("theme_mode") => theme_mode = val.map(String::from),
            Some("accent_color") => accent_color = val.map(String::from),
            _ => {}
        }
    }
    (theme_mode, accent_color)
}

fn parse_color(s: Option<&str>, default: [u8; 3]) -> [u8; 3] {
    let Some(s) = s else { return default; };
    let s = s.trim().trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(default[0]);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(default[1]);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(default[2]);
        return [r, g, b];
    }
    default
}

// ============================================================================
// APP IMPLEMENTATION
// ============================================================================

impl MitosTerminalApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Channels
        let (input_tx, mut input_rx) = mpsc::channel::<u8>(1024);
        let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(1024);
        let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u16, u16)>(); 

        // State
        let grid = Arc::new(Mutex::new(TerminalGrid::new(DEFAULT_COLS, DEFAULT_ROWS)));
        
        // Daemons
        spawn_ipc_server(Arc::clone(&grid));
        spawn_settings_watcher(Arc::clone(&grid)); 
        spawn_network_poller(Arc::clone(&grid));

        // PTY Threads
        Self::spawn_pty_handler(pty_tx, input_rx, resize_rx);
        Self::spawn_grid_processor(Arc::clone(&grid), pty_rx);

        Self { 
            grid, 
            input_tx,
            resize_tx,
            last_cols: DEFAULT_COLS,
            last_rows: DEFAULT_ROWS,
            last_t: 0.0, // <-- ADDED
        }
    }

    fn spawn_pty_handler(
        pty_tx: mpsc::Sender<Vec<u8>>, 
        mut input_rx: mpsc::Receiver<u8>, 
        resize_rx: std::sync::mpsc::Receiver<(u16, u16)>
    ) {
        std::thread::spawn(move || {
            let pty = MitosPty::new(DEFAULT_COLS, DEFAULT_ROWS).expect("Failed to create PTY");
            let mut reader = pty.master.try_clone_reader().unwrap();
            let mut writer = pty.master.take_writer();

            // Reader
            std::thread::spawn(move || {
                let mut buf = [0; 1024];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 { break; }
                    let _ = pty_tx.blocking_send(buf[..n].to_vec());
                }
            });

            // Writer
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                while let Some(byte) = input_rx.recv().await {
                    let _ = writer.write_all(&[byte]);
                }
            });

            // Resizer
            while let Ok((cols, rows)) = resize_rx.recv() {
                if let Err(e) = pty.resize(cols, rows) {
                    eprintln!("[mitos-terminal] Resize failed: {e}");
                }
            }
        });
    }

    fn spawn_grid_processor(ui_grid: Arc<Mutex<TerminalGrid>>, mut pty_rx: mpsc::Receiver<Vec<u8>>) {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                while let Some(bytes) = pty_rx.recv().await {
                    let result = {
                        let Ok(mut g) = ui_grid.lock() else { continue };
                        g.process(&bytes)
                    };

                    // 1. mitos-pkg lookup
                    for cmd in result.missing_commands {
                        let grid_for_lookup = Arc::clone(&ui_grid);
                        tokio::task::spawn_blocking(move || {
                            if let Some(widget) = pkg_bridge::suggest_install(&cmd) {
                                if let Ok(mut g) = grid_for_lookup.lock() {
                                    g.inject_widget(widget);
                                }
                            }
                        });
                    }

                    // 2. File Manager Autocomplete
                    if let Some(partial) = result.autocomplete_request {
                        let grid_for_ac = Arc::clone(&ui_grid);
                        tokio::spawn(async move {
                            if let Ok(mut stream) = UnixStream::connect(ipc::file_manager_socket()).await {
                                let req = IpcRequest::AutoCompletePath { partial_path: partial };
                                if ipc::ipc_send(&mut stream, &req).await.is_ok() {
                                    if let Ok(Some(IpcResponse::AutoCompleteResult { suggestions })) = ipc::ipc_recv::<IpcResponse>(&mut stream).await {
                                        if let Some(first) = suggestions.into_iter().next() {
                                            if let Ok(mut g) = grid_for_ac.lock() {
                                                g.set_ghost_text(Some(first));
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }

                    // 3. Long-Running Task Notifications
                    for block in result.closed_blocks {
                        if let Some(dur) = block.duration {
                            if dur.as_secs() >= 5 {
                                tokio::spawn(async move {
                                    if let Ok(mut stream) = UnixStream::connect(ipc::gui_socket()).await {
                                        let req = IpcRequest::NotifyUser { 
                                            title: "MITOS Terminal".to_string(), 
                                            body: format!("Command finished in {:.1}s", dur.as_secs_f32()) 
                                        };
                                        let _ = ipc::ipc_send(&mut stream, &req).await;
                                    }
                                });
                            }
                        }
                    }
                }
            });
        });
    }

    fn handle_semantic_clipboard(&self) {
        let grid = self.grid.lock().unwrap();
        if let Some(row) = grid.current_block.cells.get(grid.cursor_y) {
            let line: String = row.iter().map(|c| c.character).collect();
            
            let mut start = grid.cursor_x;
            let mut end = grid.cursor_x;
            let chars: Vec<char> = line.chars().collect();
            
            while start > 0 && !chars[start-1].is_whitespace() && chars[start-1] != '\0' { start -= 1; }
            while end < chars.len() && !chars[end].is_whitespace() && chars[end] != '\0' { end += 1; }
            
            let word: String = chars[start..end].iter().collect();
            let path = Path::new(&word);
            
            if path.exists() {
                if let Ok(mut clipboard) = Clipboard::new() {
                    if let Ok(abs_path) = std::fs::canonicalize(path) {
                        let uri = format!("file://{}", abs_path.display());
                        let _ = clipboard.set_text(uri);
                        eprintln!("[mitos-terminal] Copied URI to clipboard: {}", uri);
                    }
                }
            }
        }
    }
}

impl eframe::App for MitosTerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- CINEMATIC CLOCK ADDED ---
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_t).clamp(0.0, 0.1) as f32;
        self.last_t = now;
        let frame = (now * 20.0) as u64; // 20 Hz clock for glitch jitter

        // Window Resize Handling
        let font_id = egui::TextStyle::Monospace.resolve(ctx.style());
        let char_size = ctx.graphics(|gfx| {
            gfx.layout_no_wrap("M".to_string(), font_id, egui::Color32::WHITE).size()
        });
        
        let screen_rect = ctx.screen_rect();
        let new_cols = ((screen_rect.width() - 32.0) / char_size.x).max(10.0) as u16;
        let new_rows = ((screen_rect.height() - 64.0) / char_size.y).max(5.0) as u16;

        if new_cols != self.last_cols || new_rows != self.last_rows {
            self.last_cols = new_cols;
            self.last_rows = new_rows;
            
            let _ = self.resize_tx.send((new_cols, new_rows));
            if let Ok(mut g) = self.grid.lock() { 
                g.resize(new_cols as usize); // <-- REPLACE `g.cols = ...` WITH THIS
            }
        }


        // --- TICK GRID DECAY ADDED ---
        if let Ok(mut g) = self.grid.try_lock() {
            g.tick(dt);
        }

        // UI Rendering
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(4, 10, 18))) // Deep Aurora Void
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let p = ui.painter().with_clip_rect(rect);

                // --- BACKGROUND FX ADDED ---
                fx::paint_grid(&p, rect, now, [0x1E, 0x90, 0xC8]);
                fx::paint_sweep(&p, rect, now, [0x1E, 0x90, 0xC8]);

                let available_width = ui.available_width();
                let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click());
                response.request_focus();

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let grid = self.grid.lock().unwrap();
                        let prompt_color = grid.prompt_color; 

                        for block in &grid.blocks {
                            render_block(ui, block, available_width, &self.input_tx, false, 0, 0, prompt_color, now, frame);
                        }

                        render_block(
                            ui,
                            &grid.current_block,
                            available_width,
                            &self.input_tx,
                            true,
                            grid.cursor_y,
                            grid.cursor_x,
                            prompt_color,
                            now,
                            frame,
                        );
                    });

                // --- CRT SCANLINES ADDED ---
                fx::paint_scanlines(&p, rect);

                // Input Handling
                if response.has_focus() {
                    ctx.input(|i| {
                        for event in &i.events {
                            match event {
egui::Event::Text(text) => {
    let mut buf = [0; 4];
    for c in text.chars() {
        let s = c.encode_utf8(&mut buf);
        for b in s.as_bytes() {
            let _ = self.input_tx.try_send(*b);
        }
    }
}

                                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                                    if *key == egui::Key::C && modifiers.ctrl && modifiers.shift {
                                        self.handle_semantic_clipboard();
                                        continue;
                                    }

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
                                },
                                _ => {}
                            }
                        }
                    });
                }
            });
        ctx.request_repaint();
    }
}

// ============================================================================
// UI RENDERING HELPERS
// ============================================================================

fn render_block(
    ui: &mut egui::Ui,
    block: &ExecutionBlock,
    available_width: f32,
    input_tx: &mpsc::Sender<u8>,
    is_active: bool,
    cursor_y: usize,
    cursor_x: usize,
    prompt_color: [u8; 3],
    now: f64,   // <-- ADDED
    frame: u64, // <-- ADDED
) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(10, 15, 20, 180)) // Semi-transparent to show grid
        .rounding(6.0)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(45)))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(available_width - 16.0);

            ui.label(
                egui::RichText::new(&block.prompt)
                    .color(egui::Color32::from_rgb(prompt_color[0], prompt_color[1], prompt_color[2]))
                    .strong()
                    .monospace(),
            );
            ui.add_space(4.0);

            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let char_size = ui.painter().layout_no_wrap("M".to_string(), font_id.clone(), egui::Color32::WHITE).size();
            let space_width = char_size.x;
            let line_height = char_size.y;

            for (y, row) in block.cells.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                    for (x, cell) in row.iter().enumerate() {
                        if let Some(widget) = block.widgets.get(&(y, x)) {
                            render_widget(ui, widget, input_tx);
                            continue;
                        }

                        let is_cursor = is_active && x == cursor_x && y == cursor_y;
                        let mut fg = egui::Color32::from_rgb(cell.fg[0], cell.fg[1], cell.fg[2]);
                        let mut bg = egui::Color32::from_rgb(cell.bg[0], cell.bg[1], cell.bg[2]);

                        if is_cursor { std::mem::swap(&mut fg, &mut bg); }

                        let (rect, _) = ui.allocate_exact_size(egui::vec2(space_width, line_height), egui::Sense::hover());

                        if is_cursor || cell.bg != DEFAULT_BG {
                            ui.painter().rect_filled(rect, 0.0, bg);
                        }

                        if cell.character != ' ' && cell.character != '\0' {
                            // --- CINEMATIC TEXT RENDERING ---
                            let text_rgb = if is_cursor { cell.bg } else { cell.fg };
                            let t = cell.intensity;
                            let is_err = cell.is_error; 

                            // 1. GLITCH JITTER
                            let jx = if is_err && t > 0.05 {
                                ((grid::hash3(x as u64, y as u64, frame) % 3) as f32 - 1.0) * 1.5
                            } else { 0.0 };

                            let center = rect.center();
                            let text_pos = egui::pos2(center.x + jx, center.y);

                            // 2. PHOSPHOR HALO
                            if t > 0.05 {
                                let halo = egui::Color32::from_rgba_unmultiplied(
                                    text_rgb[0], text_rgb[1], text_rgb[2], (t * 90.0) as u8);
                                let offsets = [[-1.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.0, 1.0]];
                                for [dx, dy] in offsets {
                                    ui.painter().text(
                                        egui::pos2(center.x + dx + jx, center.y + dy),
                                        egui::Align2::CENTER_CENTER,
                                        cell.character.to_string(),
                                        font_id.clone(),
                                        halo,
                                    );
                                }
                            }

                            // 3. CORE COLOR (white-hot cooling)
                            let core = if t > 0.0 {
                                let k = t * 0.85;
                                egui::Color32::from_rgb(
                                    (text_rgb[0] as f32 + (255.0 - text_rgb[0] as f32) * k) as u8,
                                    (text_rgb[1] as f32 + (255.0 - text_rgb[1] as f32) * k) as u8,
                                    (text_rgb[2] as f32 + (255.0 - text_rgb[2] as f32) * k) as u8)
                            } else {
                                fg // This is already swapped if is_cursor
                            };

                            ui.painter().text(
                                text_pos,
                                egui::Align2::CENTER_CENTER,
                                cell.character.to_string(),
                                font_id.clone(),
                                core,
                            );
                        }
                    }
                    
                    if is_active && y == cursor_y {
                        if let Some(ghost) = &block.ghost_text {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(ghost)
                                        .color(egui::Color32::from_gray(80))
                                        .monospace()
                                ).selectable(false)
                            );
                        }
                    }
                });
            }
        });
    ui.add_space(8.0); 
}

fn render_widget(ui: &mut egui::Ui, widget: &RichWidget, input_tx: &mpsc::Sender<u8>) {
    match widget {
        RichWidget::Button { label, cmd } => {
            if ui.button(egui::RichText::new(label).strong()).clicked() {
                let cmd_bytes = format!("{}\n", cmd).into_bytes();
                for b in cmd_bytes {
                    let _ = input_tx.try_send(b);
                }
            }
        }
        RichWidget::Progress { percent, .. } => {
            ui.add(egui::ProgressBar::new(*percent).show_percentage());
        }
        RichWidget::Sparkline { .. } => {
            ui.label("📈 [Sparkline Graph]");
        }
    }
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|arg| arg == "--tty") {
        eprintln!("[mitos-terminal] TTY recovery mode not yet fully wired to crossterm backend. Starting eframe anyway.");
    }

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
