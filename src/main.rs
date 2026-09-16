mod grid;
mod pty;
mod pkg_bridge;
mod fx; 

// --- Standard Library ---
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};


// --- Third-Party ---
use arboard::Clipboard; 
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
const MATRIX_GREEN: [u8; 3] = [0x33, 0xFF, 0x66]; // <-- ADD THIS (matches the image tint)

// ============================================================================
// APP STATE & SELECTION
// ============================================================================

#[derive(Clone, Copy, PartialEq)]
struct Selection {
    start_block: usize,
    start_row: usize,
    start_col: usize,
    end_block: usize,
    end_row: usize,
    end_col: usize,
}

struct MitosTerminalApp {
    grid: Arc<Mutex<TerminalGrid>>,
    input_tx: mpsc::Sender<u8>,
    resize_tx: std::sync::mpsc::Sender<(u16, u16)>,
    last_cols: u16,
    last_rows: u16,
    last_t: f64,
    selection: Option<Selection>,
    search_active: bool,
    search_query: String,
    rain: fx::CodeRain,
    rain_enabled: Arc<AtomicBool>,
    show_settings: bool,
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

fn spawn_settings_watcher(grid: Arc<Mutex<TerminalGrid>>, rain_enabled: Arc<AtomicBool>) {
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

        // Parse settings on startup
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            apply_home_conf_theme(&grid, &content);
            
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("matrix_rain=") {
                    rain_enabled.store(val.trim() == "true" || val.trim() == "1", Ordering::Relaxed);
                }
            }
        }

        let watch_dir = config_path.parent().unwrap().to_path_buf();
        let _ = watcher.watch(&watch_dir, RecursiveMode::NonRecursive);

        for res in rx {
            if let Ok(event) = res {
                let is_home_conf = event.paths.iter().any(|p| p.file_name() == Some(std::ffi::OsStr::new("home.conf")));
                if is_home_conf {
                    if let Ok(content) = std::fs::read_to_string(&config_path) {
                        apply_home_conf_theme(&grid, &content);
                        
                        // Update settings when the file changes externally
                        for line in content.lines() {
                            if let Some(val) = line.strip_prefix("matrix_rain=") {
                                rain_enabled.store(val.trim() == "true" || val.trim() == "1", Ordering::Relaxed);
                            }
                        }
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
// SELECTION HELPER
// ============================================================================

fn extract_selection_text(grid: &TerminalGrid, sel: &Selection) -> String {
    let mut out = String::new();
    
    let get_block = |idx: usize| -> Option<&ExecutionBlock> {
        if idx < grid.blocks.len() {
            Some(&grid.blocks[idx])
        } else if idx == grid.blocks.len() {
            Some(&grid.current_block)
        } else {
            None
        }
    };

    let (min_b, max_b) = if sel.start_block <= sel.end_block {
        (sel.start_block, sel.end_block)
    } else {
        (sel.end_block, sel.start_block)
    };

    let (min_r, max_r, min_c, max_c) = if sel.start_block < sel.end_block || (sel.start_block == sel.end_block && sel.start_row <= sel.end_row) {
        (sel.start_row, sel.end_row, sel.start_col, sel.end_col)
    } else {
        (sel.end_row, sel.start_row, sel.end_col, sel.start_col)
    };

    for b_idx in min_b..=max_b {
        if let Some(block) = get_block(b_idx) {
            let r_start = if b_idx == min_b { min_r } else { 0 };
            let r_end = if b_idx == max_b { max_r } else { block.cells.len().saturating_sub(1) };
            
            for r_idx in r_start..=r_end {
                if let Some(row) = block.cells.get(r_idx) {
                    let c_start = if b_idx == min_b && r_idx == min_r { min_c } else { 0 };
                    let c_end = if b_idx == max_b && r_idx == max_r { max_c } else { row.len().saturating_sub(1) };
                    
                    let mut line = String::new();
                    for c_idx in c_start..=c_end {
                        if let Some(cell) = row.get(c_idx) {
                            if cell.character != '\0' { 
                               line.push(cell.character);
                            }
                        }
                    }
                    out.push_str(&line);
                }
                if r_idx < r_end || b_idx < max_b {
                    out.push('\n');
                }
            }
        }
    }
    out.trim_end().to_string()
}

// ============================================================================
// APP IMPLEMENTATION
// ============================================================================

impl MitosTerminalApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (input_tx, input_rx) = mpsc::channel::<u8>(1024);
        let (pty_tx, pty_rx) = mpsc::channel::<Vec<u8>>(1024);
        let (resize_tx, resize_rx) = std::sync::mpsc::channel::<(u16, u16)>(); 

        // FIX: Cast u16 constants to usize
        let grid = Arc::new(Mutex::new(TerminalGrid::new(DEFAULT_COLS as usize, DEFAULT_ROWS as usize)));

        let rain_enabled = Arc::new(AtomicBool::new(true));
        let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
        let config_path = config_dir.join("mitos").join("home.conf");
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("matrix_rain=") {
                    rain_enabled.store(val.trim() == "true" || val.trim() == "1", Ordering::Relaxed);
                }
            }
        }
        spawn_ipc_server(Arc::clone(&grid));
        spawn_settings_watcher(Arc::clone(&grid), Arc::clone(&rain_enabled)); 
        spawn_network_poller(Arc::clone(&grid));

        Self::spawn_pty_handler(pty_tx, input_rx, resize_rx);
        Self::spawn_grid_processor(Arc::clone(&grid), pty_rx);

        Self { 
            grid, 
            input_tx,
            resize_tx,
            last_cols: DEFAULT_COLS,
            last_rows: DEFAULT_ROWS,
            last_t: 0.0,
            selection: None,
            search_active: false,
            search_query: String::new(),
            rain: fx::CodeRain::new(10.0, 0.55),
            rain_enabled,
            show_settings: false,
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
            
            // FIX: Unwrap the Result from take_writer()
            let mut writer = pty.master.take_writer().expect("Failed to take PTY writer");

            std::thread::spawn(move || {
                let mut buf = [0; 1024];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 { break; }
                    let _ = pty_tx.blocking_send(buf[..n].to_vec());
                }
            });

            tokio::runtime::Runtime::new().unwrap().block_on(async {
                while let Some(byte) = input_rx.recv().await {
                    let _ = writer.write_all(&[byte]);
                }
            });

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
                        // FIX: Clone the string so we can print it after moving it into the clipboard
                        let _ = clipboard.set_text(uri.clone());
                        eprintln!("[mitos-terminal] Copied URI to clipboard: {}", uri);
                    }
                }
            }
        }
    }
}

impl eframe::App for MitosTerminalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::Comma) && i.modifiers.ctrl) {
            self.show_settings = !self.show_settings;
        }
        egui::Area::new(egui::Id::new("settings_btn_area"))
            .fixed_pos(egui::Pos2::new(ctx.screen_rect().right() - 40.0, ctx.screen_rect().top() + 10.0))
            .show(ctx, |ui| {
                if ui.button("⚙️").on_hover_text("Settings (Ctrl+,)").clicked() {
                    self.show_settings = !self.show_settings;
                }
            });

        // --- NEW: Settings Window ---
        if self.show_settings {
            egui::Window::new("⚙️ MITOS Settings")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading("Visual Effects");
                    ui.separator();
                    
                    let mut is_rain_on = self.rain_enabled.load(Ordering::Relaxed);
                    if ui.checkbox(&mut is_rain_on, "Matrix Code Rain").changed() {
                        self.rain_enabled.store(is_rain_on, Ordering::Relaxed);
                        save_matrix_rain_setting(is_rain_on);
                    }
                    
                    ui.add_space(10.0);
                    ui.label("Changes are saved automatically to ~/.config/mitos/home.conf");
                });
        }
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_t).clamp(0.0, 0.1) as f32;
        self.last_t = now;
        let frame = (now * 20.0) as u64; 

        if ctx.input(|i| i.key_pressed(egui::Key::F) && i.modifiers.ctrl) {
            self.search_active = !self.search_active;
            if !self.search_active { self.search_query.clear(); }
        }

        if self.search_active {
            egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.text_edit_singleline(&mut self.search_query);
                    if ui.button("✖").clicked() {
                        self.search_active = false;
                        self.search_query.clear();
                    }
                });
            });
        }

        // FIX: egui 0.28 requires dereferencing the Arc<Style>
        let font_id = egui::TextStyle::Monospace.resolve(&*ctx.style());
        
        // FIX: egui 0.28 moved layout_no_wrap to the Fonts API
        let char_size = ctx.fonts(|f| {
            f.layout_no_wrap("M".to_string(), font_id, egui::Color32::WHITE).size()
        });
        
        let screen_rect = ctx.screen_rect();
                if self.rain_enabled.load(Ordering::Relaxed) {
            self.rain.tick(dt, screen_rect);
        }
        let new_cols = ((screen_rect.width() - 32.0) / char_size.x).max(10.0) as u16;
        let new_rows = ((screen_rect.height() - 64.0) / char_size.y).max(5.0) as u16;

        if new_cols != self.last_cols || new_rows != self.last_rows {
            self.last_cols = new_cols;
            self.last_rows = new_rows;
            
            let _ = self.resize_tx.send((new_cols, new_rows));
            if let Ok(mut g) = self.grid.lock() { 
                g.resize(new_cols as usize);
            }
        }

        if let Ok(mut g) = self.grid.try_lock() {
            g.tick(dt);
        }

        egui::CentralPanel::default()
            // FIX: egui 0.28 renamed Frame::NONE to Frame::none()
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(4, 10, 18))) 
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let p = ui.painter().with_clip_rect(rect);

                // --- NEW: Only paint rain if enabled ---
                if self.rain_enabled.load(Ordering::Relaxed) {
                    self.rain.paint(&p, rect, now, MATRIX_GREEN);
                }
                // ---------------------------------------
                
                fx::paint_grid(&p, rect, now, MATRIX_GREEN);
                fx::paint_sweep(&p, rect, now, MATRIX_GREEN);



                let available_width = ui.available_width();
                let response = ui.allocate_rect(ui.max_rect(), egui::Sense::click());
                response.request_focus();

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let grid = self.grid.lock().unwrap();
                        let prompt_color = grid.prompt_color; 

                        for (block_idx, block) in grid.blocks.iter().enumerate() {
                            render_block(ui, block, block_idx, available_width, &self.input_tx, false, 0, 0, prompt_color, now, frame, &self.search_query, &mut self.selection);
                        }

                        render_block(
                            ui,
                            &grid.current_block,
                            grid.blocks.len(),
                            available_width,
                            &self.input_tx,
                            true,
                            grid.cursor_y,
                            grid.cursor_x,
                            prompt_color,
                            now,
                            frame,
                            &self.search_query,
                            &mut self.selection,
                        );
                    });

                fx::paint_scanlines(&p, rect);

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
                                    
                                    if *key == egui::Key::C && modifiers.ctrl {
                                        if let Some(sel) = self.selection {
                                            let g = self.grid.lock().unwrap();
                                            let text = extract_selection_text(&g, &sel);
                                            if !text.is_empty() {
                                                if let Ok(mut clipboard) = Clipboard::new() {
                                                    let _ = clipboard.set_text(text);
                                                }
                                            }
                                        } else {
                                            self.handle_semantic_clipboard();
                                        }
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
    block_idx: usize,
    available_width: f32,
    input_tx: &mpsc::Sender<u8>,
    is_active: bool,
    cursor_y: usize,
    cursor_x: usize,
    prompt_color: [u8; 3],
    _now: f64,
    frame: u64,
    search_query: &str,
    selection: &mut Option<Selection>,
) {
    // FIX: egui 0.28 renamed Frame::new() to Frame::none()
    egui::Frame::none()
        .fill(egui::Color32::from_rgba_unmultiplied(10, 15, 20, 180)) 
        .rounding(6.0)
        // FIX: Explicit f32 type for stroke width
        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_gray(45)))
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

            let cells_start_pos = ui.cursor().min;
            let block_response = ui.interact(ui.max_rect(), egui::Id::new(("block", block_idx)), egui::Sense::click_and_drag());

            if block_response.drag_started() || block_response.dragged() {
                if let Some(pos) = block_response.interact_pointer_pos() {
                    let rel_x = pos.x - cells_start_pos.x;
                    let rel_y = pos.y - cells_start_pos.y;
                    let col = (rel_x / space_width).floor().max(0.0) as usize;
                    let row = (rel_y / line_height).floor().max(0.0) as usize;
                    
                    let row = row.min(block.cells.len().saturating_sub(1));
                    let max_col = block.cells.get(row).map(|r| r.len().saturating_sub(1)).unwrap_or(0);
                    let col = col.min(max_col);
                    
                    if block_response.drag_started() {
                        *selection = Some(Selection {
                            start_block: block_idx, start_row: row, start_col: col,
                            end_block: block_idx, end_row: row, end_col: col,
                        });
                    } else if let Some(sel) = selection {
                        *selection = Some(Selection {
                            start_block: sel.start_block, start_row: sel.start_row, start_col: sel.start_col,
                            end_block: block_idx, end_row: row, end_col: col,
                        });
                    }
                }
            }

            for (y, row) in block.cells.iter().enumerate() {
                let lower_q = search_query.to_lowercase();
                let row_text: String = row.iter().map(|c| c.character).collect();
                let lower_row = row_text.to_lowercase();
                let mut matches = vec![false; row.len()];

                if !lower_q.is_empty() {
                    let q_len = lower_q.chars().count();
                    let mut start = 0;
                    while let Some(idx) = lower_row[start..].find(&lower_q) {
                        let abs_idx = start + idx;
                        for i in 0..q_len {
                            if abs_idx + i < matches.len() { matches[abs_idx + i] = true; }
                        }
                        start = abs_idx + 1;
                    }
                }

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                    for (x, cell) in row.iter().enumerate() {
                        if let Some(widget) = block.widgets.get(&(y, x)) {
                            render_widget(ui, widget, input_tx);
                            continue;
                        }

                        let is_match = matches.get(x).copied().unwrap_or(false);
                        
                        let is_selected = if let Some(sel) = selection {
                            let (min_b, max_b) = if sel.start_block <= sel.end_block { (sel.start_block, sel.end_block) } else { (sel.end_block, sel.start_block) };
                            let (min_r, max_r, min_c, max_c) = if sel.start_block < sel.end_block || (sel.start_block == sel.end_block && sel.start_row <= sel.end_row) {
                                (sel.start_row, sel.end_row, sel.start_col, sel.end_col)
                            } else {
                                (sel.end_row, sel.start_row, sel.end_col, sel.start_col)
                            };

                            if block_idx > min_b && block_idx < max_b {
                                true
                            } else if block_idx == min_b && block_idx == max_b {
                                (y > min_r && y < max_r) || (y == min_r && x >= min_c && (y != max_r || x <= max_c)) || (y == max_r && x <= max_c && y != min_r)
                            } else if block_idx == min_b {
                                y > min_r || (y == min_r && x >= min_c)
                            } else if block_idx == max_b {
                                y < max_r || (y == max_r && x <= max_c)
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        let is_cursor = is_active && x == cursor_x && y == cursor_y;
                        let mut fg = egui::Color32::from_rgb(cell.fg[0], cell.fg[1], cell.fg[2]);
                        let mut bg = egui::Color32::from_rgb(cell.bg[0], cell.bg[1], cell.bg[2]);

                        if is_selected {
                            bg = egui::Color32::from_rgba_unmultiplied(0, 120, 215, 100);
                        } else if is_match {
                            bg = egui::Color32::from_rgba_unmultiplied(255, 200, 0, 150);
                        }

                        if is_cursor { std::mem::swap(&mut fg, &mut bg); }

                        let (rect, _) = ui.allocate_exact_size(egui::vec2(space_width, line_height), egui::Sense::hover());

                        if is_cursor || cell.bg != DEFAULT_BG || is_selected || is_match {
                            ui.painter().rect_filled(rect, 0.0, bg);
                        }
                        
                        if cell.link_id > 0 {
                            let link_response = ui.interact(rect, egui::Id::new(("link", y, x, cell.link_id)), egui::Sense::click());
                            if link_response.clicked() {
                                if let Some(url) = block.link_table.get((cell.link_id - 1) as usize) {
                                    let _ = open::that(url);
                                }
                            }
                            if link_response.hovered() {
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                            }
                        }

                        if cell.character != ' ' && cell.character != '\0' {
                            let text_rgb = if is_cursor { cell.bg } else { cell.fg };
                            let t = cell.intensity;
                            let is_err = cell.is_error; 

                            let jx = if is_err && t > 0.05 {
                                ((grid::hash3(x as u64, y as u64, frame) % 3) as f32 - 1.0) * 1.5
                            } else { 0.0 };

                            let center = rect.center();
                            let text_pos = egui::pos2(center.x + jx, center.y);

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

                            let core = if t > 0.0 {
                                let k = t * 0.85;
                                egui::Color32::from_rgb(
                                    (text_rgb[0] as f32 + (255.0 - text_rgb[0] as f32) * k) as u8,
                                    (text_rgb[1] as f32 + (255.0 - text_rgb[1] as f32) * k) as u8,
                                    (text_rgb[2] as f32 + (255.0 - text_rgb[2] as f32) * k) as u8)
                            } else {
                                fg 
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

fn save_matrix_rain_setting(enabled: bool) {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    let config_path = config_dir.join("mitos").join("home.conf");
    let _ = std::fs::create_dir_all(&config_dir);

    let val = if enabled { "true" } else { "false" };
    let mut content = std::fs::read_to_string(&config_path).unwrap_or_default();

    let mut found = false;
    let lines: Vec<String> = content.lines().map(|l| {
        if l.starts_with("matrix_rain=") {
            found = true;
            format!("matrix_rain={}", val)
        } else {
            l.to_string()
        }
    }).collect();

    if found {
        content = lines.join("\n");
    } else {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("matrix_rain={}\n", val));
    }

    let _ = std::fs::write(&config_path, content);
}

