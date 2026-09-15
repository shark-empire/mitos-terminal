mod grid;
mod pty;
mod pkg_bridge;

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::io::{Read, Write};
use tokio::sync::mpsc;
use grid::{TerminalGrid, ExecutionBlock, ProcessResult};
use pty::MitosPty;
use mitos_utils::ipc::{self, RichWidget, IpcRequest, IpcResponse};
use arboard::Clipboard; // NEW: For Semantic Clipboard (text/uri-list)

struct MitosTerminalApp {
    grid: Arc<Mutex<TerminalGrid>>,
    input_tx: mpsc::Sender<u8>,
}

fn spawn_ipc_server(grid: Arc<Mutex<TerminalGrid>>) {
    let socket_path = ipc::terminal_socket(std::process::id());
    let _ = std::fs::remove_file(&socket_path); 

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("ipc runtime");
        rt.block_on(async move {
            let listener = match tokio::net::UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => { eprintln!("[mitos-terminal] IPC bind failed: {e}"); return; }
            };
            eprintln!("[mitos-terminal] IPC listening on {socket_path}");

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
                                grid.lock().unwrap().apply_theme(fg, bg, [85, 255, 85]); // Fallback prompt color
                                IpcResponse::Ack
                            }
                            IpcRequest::AutoCompletePath { partial_path: _ } => {
                                IpcResponse::AutoCompleteResult { suggestions: vec![] }
                            }
                            // We don't strictly need to handle these requests here as we are the ones SENDING them,
                            // but the enum requires us to match exhaustively if we add more variants later.
                            _ => IpcResponse::Ack,
                        };
                        
                        if ipc::ipc_send(&mut stream, &resp).await.is_err() { break; }
                    }
                });
            }
        });
    });
}

// ------------------------------------------------------------------
// NEW: Network Captive Portal Poller (mitos-network integration)
// ------------------------------------------------------------------
fn spawn_network_poller(grid: Arc<Mutex<TerminalGrid>>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("network poller runtime");
        rt.block_on(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
            loop {
                interval.tick().await;
                let socket = ipc::network_socket();
                
                // Attempt to connect to mitos-network daemon
                if let Ok(mut stream) = tokio::net::UnixStream::connect(&socket).await {
                    let req = IpcRequest::GetNetworkStatus;
                    if ipc::ipc_send(&mut stream, &req).await.is_ok() {
                        if let Ok(Some(IpcResponse::NetworkStatus { is_captive_portal, .. })) = ipc::ipc_recv::<IpcResponse>(&mut stream).await {
                            if is_captive_portal {
                                let g = Arc::clone(&grid);
                                if let Ok(mut g_lock) = g.lock() {
                                    // Inject Captive Portal Button if not already present
                                    let widget = RichWidget::Button {
                                        label: "🌐 Open Login Page".to_string(),
                                        cmd: "xdg-open http://captive.apple.com".to_string(),
                                    };
                                    // Simple check to avoid spamming the button
                                    let already_has = g_lock.current_block.widgets.values().any(|w| {
                                        matches!(w, RichWidget::Button { label, .. } if label.contains("Login Page"))
                                    });
                                    if !already_has {
                                        g_lock.inject_widget(widget);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    });
}

// ------------------------------------------------------------------
// Settings & Theme Sync (mitos-settings integration)
// ------------------------------------------------------------------
fn parse_home_conf(content: &str) -> (Option<String>, Option<String>) {
    let mut theme_mode = None;
    let mut accent_color = None;
    
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
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

fn apply_home_conf_theme(grid: &Arc<Mutex<TerminalGrid>>, content: &str) {
    let (theme_mode, accent_color) = parse_home_conf(content);
    
    // Map theme_mode to actual RGB values
    let (fg, bg) = match theme_mode.as_deref() {
        Some("light") => ([30, 30, 30], [245, 245, 245]),
        _ => ([200, 200, 200], [20, 20, 25]), // Default MITOS Dark
    };
    
    let prompt = parse_color(accent_color.as_deref(), [85, 255, 85]);
    
    grid.lock().unwrap().apply_theme(fg, bg, prompt);
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

fn spawn_settings_watcher(grid: Arc<Mutex<TerminalGrid>>) {
    let config_dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".config"));
    let config_path = config_dir.join("mitos").join("home.conf");
    
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::thread::spawn(move || {
        use notify::{Watcher, RecursiveMode, Config, RecommendedWatcher};
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher = RecommendedWatcher::new(move |res| {
            let _ = tx.send(res);
        }, Config::default()).expect("Failed to create file watcher");

        // Initial load
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            apply_home_conf_theme(&grid, &content);
        }

        // Watch the parent directory (~/.config/mitos/) so we catch atomic renames
        // (temp file + rename) used by mitos-settings.
        let watch_dir = config_path.parent().unwrap().to_path_buf();
        let _ = watcher.watch(&watch_dir, RecursiveMode::NonRecursive);

        for res in rx {
            if let Ok(event) = res {
                // Check if the event involves home.conf
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

impl MitosTerminalApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, mut rx) = mpsc::channel::<u8>(1024);
        let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(1024);

        let grid = Arc::new(Mutex::new(TerminalGrid::new(80, 24)));
        let ui_grid = Arc::clone(&grid);

        spawn_ipc_server(Arc::clone(&grid));
        spawn_settings_watcher(Arc::clone(&grid)); 
        spawn_network_poller(Arc::clone(&grid)); // Start network captive portal listener

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
                    let result = {
                        let Ok(mut g) = ui_grid.lock() else { continue };
                        g.process(&bytes)
                    };

                    // 1. Handle mitos-pkg "command not found" lookups
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

                    // 2. Handle File Manager Autocomplete
                    if let Some(partial) = result.autocomplete_request {
                        let grid_for_ac = Arc::clone(&ui_grid);
                        tokio::spawn(async move {
                            let socket_path = ipc::file_manager_socket();
                            if let Ok(mut stream) = tokio::net::UnixStream::connect(&socket_path).await {
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

                    // 3. Handle Long-Running Task Notifications (mitos-gui)
                    for block in result.closed_blocks {
                        if let Some(dur) = block.duration {
                            if dur.as_secs() >= 5 {
                                let title = "MITOS Terminal".to_string();
                                let body = format!("Command finished in {:.1}s", dur.as_secs_f32());
                                tokio::spawn(async move {
                                    let socket = ipc::gui_socket();
                                    if let Ok(mut stream) = tokio::net::UnixStream::connect(&socket).await {
                                        let req = IpcRequest::NotifyUser { title, body };
                                        let _ = ipc::ipc_send(&mut stream, &req).await;
                                    }
                                });
                            }
                        }
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
            let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click());
            response.request_focus();

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let grid = self.grid.lock().unwrap();
                    let prompt_color = grid.prompt_color; 

                    for block in &grid.blocks {
                        render_block(ui, block, available_width, &self.input_tx, false, 0, 0, prompt_color);
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
                    );
                });

            if response.has_focus() {
                ctx.input(|i| {
                    for event in &i.events {
                        match event {
                            egui::Event::Text(text) => {
                                for c in text.chars() {
                                    let _ = self.input_tx.try_send(c as u8);
                                }
                            }
                            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                                // --- SEMANTIC CLIPBOARD (Ctrl+Shift+C) ---
                                if *key == egui::Key::C && pressed && modifiers.ctrl && modifiers.shift {
                                    let grid = self.grid.lock().unwrap();
                                    if let Some(row) = grid.current_block.cells.get(grid.cursor_y) {
                                        let line: String = row.iter().map(|c| c.character).collect();
                                        
                                        // Extract word under cursor
                                        let mut start = grid.cursor_x;
                                        let mut end = grid.cursor_x;
                                        let chars: Vec<char> = line.chars().collect();
                                        
                                        while start > 0 && !chars[start-1].is_whitespace() && chars[start-1] != '\0' { 
                                            start -= 1; 
                                        }
                                        while end < chars.len() && !chars[end].is_whitespace() && chars[end] != '\0' { 
                                            end += 1; 
                                        }
                                        
                                        let word: String = chars[start..end].iter().collect();
                                        let path = std::path::Path::new(&word);
                                        
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
                                    continue; // Don't send Ctrl+Shift+C to the PTY
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

fn render_block(
    ui: &mut egui::Ui,
    block: &ExecutionBlock,
    available_width: f32,
    input_tx: &mpsc::Sender<u8>,
    is_active: bool,
    cursor_y: usize,
    cursor_x: usize,
    prompt_color: [u8; 3],
) {
    egui::Frame::new()
        .fill(egui::Color32::from_gray(22))
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

                        if is_cursor || cell.bg != [20, 20, 25] {
                            ui.painter().rect_filled(rect, 0.0, bg);
                        }

                        if cell.character != ' ' && cell.character != '\0' {
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                cell.character.to_string(),
                                font_id.clone(),
                                fg,
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
        RichWidget::Progress { percent, color: _ } => {
            ui.add(egui::ProgressBar::new(*percent).show_percentage());
        }
        RichWidget::Sparkline { data: _ } => {
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
