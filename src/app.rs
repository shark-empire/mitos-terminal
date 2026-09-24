//! The running application: one `TerminalApp` per OS window, owning every
//! pane's [`Session`], the [`Layout`] of tabs/splits, the resolved
//! [`Theme`]/[`Config`], and the small bits of UI state (dialogs, search,
//! pending confirmations) that don't belong in any lower module.
//!
//! `update()` follows the same shape proven out in the pre-rewrite build of
//! this app (settings shortcut → settings window → search bar → central
//! panel with a painter + an `if response.has_focus() { for event in
//! i.events { ... } }` keyboard loop) extended to multiple panes: the loop
//! over `i.events` still runs once per frame, but now only for whichever
//! pane's interact-response currently has focus, and mouse/keyboard/paste
//! routing all key off that same focused pane.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};

use egui::{Color32, Pos2, Rect as ERect, Sense, Vec2};
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};

use crate::a11y::AnnounceQueue;
use crate::config::{self, Action, Chord, Config, HomeConf};
use crate::fx;
use crate::input::{self, Mods};
use crate::layout::{Axis, FocusDir, Layout, PaneId, Rect as LRect};
use crate::platform::SystemClipboard;
use crate::pty::SpawnOptions;
use crate::render::{self, Metrics, PaintOpts};
use crate::security::{self, LinkDecision, Paste};
use crate::session::{Session, SessionState};
use crate::term::{SelMode, SelPoint, Term, TermEvent};
use crate::theme::Theme;

const MIN_COLS: usize = 4;
const MIN_ROWS: usize = 2;

struct PendingPaste {
    pane: PaneId,
    paste: Paste,
    reason: String,
}

struct PendingLink {
    pane: PaneId,
    url: String,
    reason: String,
}

struct PaneState {
    session: Session,
    rain: Option<fx::CodeRain>,
    blink_on: bool,
    last_blink_flip: f64,
    hover_cell: Option<(usize, usize)>,
    drag_active: bool,
    last_seqno: u64,
    notified_finish: bool,
    notified_long_running: bool,
}

pub struct TerminalApp {
    config: Config,
    home: HomeConf,
    theme: Theme,
    keymap: Vec<(Chord, Action)>,
    panes: HashMap<PaneId, PaneState>,
    layout: Layout,
    next_pane_id: PaneId,
    registry: Arc<crate::ipc::Registry>,
    socket_path: PathBuf,
    clipboard: SystemClipboard,
    announce: AnnounceQueue,
    speech_checked: bool,
    speech_on: bool,
    last_time: f64,
    show_settings: bool,
    show_palette: bool,
    palette_query: String,
    search_active: bool,
    search_query: String,
    font: egui::FontId,
    metrics: Metrics,
    metrics_font_size: f32,
    config_dirty: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
    pending_paste: Option<PendingPaste>,
    pending_link: Option<PendingLink>,
    pending_close: Option<PaneId>,
    startup_errors: Vec<String>,
    frame_count: u64,
    fullscreen: bool,
    notify_times: std::collections::VecDeque<f64>,
    last_focused_pane: Option<PaneId>,
}

fn new_pane(app_ctx: &egui::Context, config: &Config, home: &HomeConf, cols: usize, rows: usize, profile_name: Option<&str>) -> PaneState {
    let profile = config.profile(profile_name);
    let opts = SpawnOptions {
        shell: profile.shell.clone(),
        args: profile.args.clone(),
        cwd: profile.cwd.as_ref().map(PathBuf::from),
        env: profile.env.clone(),
        cols: cols.clamp(MIN_COLS, crate::term::MAX_COLS) as u16,
        rows: rows.clamp(MIN_ROWS, crate::term::MAX_ROWS) as u16,
        pixel_width: 0,
        pixel_height: 0,
        shell_integration: config.general.shell_integration,
    };
    let session = Session::spawn(opts, config.scrollback.lines, config.term_policy(), app_ctx.clone());
    let rain = if config.rain_enabled(home) { Some(fx::CodeRain::new(10.0, 0.55)) } else { None };
    PaneState {
        session,
        rain,
        blink_on: true,
        last_blink_flip: 0.0,
        hover_cell: None,
        drag_active: false,
        last_seqno: 0,
        notified_finish: false,
        notified_long_running: false,
    }
}

impl TerminalApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (config, cfg_err) = Config::load();
        let home = HomeConf::load();
        let theme = config.theme(&home);
        let (keymap, keymap_problems) = config::build_keymap(&config.keybindings);

        let mut startup_errors: Vec<String> = Vec::new();
        if let Some(e) = cfg_err {
            startup_errors.push(e);
        }
        startup_errors.extend(keymap_problems);

        let font = render::font_id(&config.font.family, config.font.size);
        let metrics = render::measure(&cc.egui_ctx, &font);

        let first = new_pane(&cc.egui_ctx, &config, &home, 80, 24, None);
        let pane_id: PaneId = 1;
        let registry = crate::ipc::Registry::new();
        registry.add(pane_id, first.session.term());
        registry.set_active(pane_id);
        registry.set_allow_buffer_read(config.security.ipc_buffer_read);
        let socket_path = crate::ipc::spawn_server(Arc::clone(&registry), cc.egui_ctx.clone());
        crate::ipc::spawn_network_poller(Arc::clone(&registry), cc.egui_ctx.clone());

        let mut panes = HashMap::new();
        panes.insert(pane_id, first);

        let config_dirty = Arc::new(AtomicBool::new(false));
        let watcher = spawn_config_watcher(Arc::clone(&config_dirty), cc.egui_ctx.clone());

        TerminalApp {
            config,
            home,
            theme,
            keymap,
            panes,
            layout: Layout::new(pane_id),
            next_pane_id: pane_id + 1,
            registry,
            socket_path,
            clipboard: SystemClipboard::new(),
            announce: AnnounceQueue::new(),
            speech_checked: false,
            speech_on: false,
            last_time: 0.0,
            show_settings: false,
            show_palette: false,
            palette_query: String::new(),
            search_active: false,
            search_query: String::new(),
            font,
            metrics,
            metrics_font_size: config.font.size,
            config_dirty,
            _watcher: watcher,
            pending_paste: None,
            pending_link: None,
            pending_close: None,
            startup_errors,
            frame_count: 0,
            fullscreen: false,
            notify_times: std::collections::VecDeque::new(),
            last_focused_pane: None,
        }
    }

    fn reload_config(&mut self, ctx: &egui::Context) {
        let (config, err) = Config::load();
        self.home = HomeConf::load();
        if let Some(e) = err {
            self.startup_errors.push(e);
            return; // keep the previous, valid config
        }
        let (keymap, problems) = config::build_keymap(&config.keybindings);
        self.startup_errors.extend(problems);
        self.config = config;
        self.theme = self.config.theme(&self.home);
        self.keymap = keymap;
        self.font = render::font_id(&self.config.font.family, self.config.font.size);
        self.metrics = render::measure(ctx, &self.font);
        self.metrics_font_size = self.config.font.size;
        self.registry.set_allow_buffer_read(self.config.security.ipc_buffer_read);
        let want_rain = self.config.rain_enabled(&self.home);
        for pane in self.panes.values_mut() {
            match (want_rain, &pane.rain) {
                (true, None) => pane.rain = Some(fx::CodeRain::new(10.0, 0.55)),
                (false, Some(_)) => pane.rain = None,
                _ => {}
            }
            let policy = self.config.term_policy();
            if let Ok(mut t) = pane.session.term().lock() {
                t.set_policy(policy);
            }
        }
    }

    fn resolve_theme_overrides(&mut self) {
        if let Some((fg, bg)) = self.registry.take_theme_override() {
            self.theme.fg = fg;
            self.theme.bg = bg;
        }
    }

    fn set_zoom(&mut self, delta: f32, ctx: &egui::Context) {
        let new_size = (self.metrics_font_size + delta).clamp(config::MIN_FONT, config::MAX_FONT);
        if (new_size - self.metrics_font_size).abs() > 0.01 {
            self.metrics_font_size = new_size;
            self.font = render::font_id(&self.config.font.family, new_size);
            self.metrics = render::measure(ctx, &self.font);
        }
    }

    fn new_pane_in_active_tab(&mut self, ctx: &egui::Context, axis: Option<Axis>) {
        if self.layout.all_panes().len() >= self.config.security.max_panes {
            return;
        }
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let pane = new_pane(ctx, &self.config, &self.home, 80, 24, None);
        self.registry.add(id, pane.session.term());
        self.panes.insert(id, pane);
        match axis {
            Some(a) => self.layout.split(a, id),
            None => {
                self.layout.new_tab(id);
            }
        }
        self.registry.set_active(self.layout.focused_pane());
    }

    fn close_pane(&mut self, pane_id: PaneId) {
        self.layout.close_pane(pane_id);
        if let Some(mut p) = self.panes.remove(&pane_id) {
            p.session.shutdown();
        }
        self.registry.remove(pane_id);
        if self.layout.all_panes().is_empty() {
            // The very last pane of the very last tab: keep the window alive
            // with a fresh shell rather than leaving an empty layout.
            let id = self.next_pane_id;
            self.next_pane_id += 1;
            self.layout = Layout::new(id);
        }
        self.registry.set_active(self.layout.focused_pane());
    }

    fn request_close_pane(&mut self, ctx: &egui::Context, pane_id: PaneId) {
        let running_job = self.panes.get(&pane_id).map(|p| p.session.has_foreground_job()).unwrap_or(false);
        if self.config.general.confirm_close_running && running_job {
            self.pending_close = Some(pane_id);
        } else {
            self.close_pane(pane_id);
        }
        let _ = ctx;
    }

    fn dispatch_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::NewTab => self.new_pane_in_active_tab(ctx, None),
            Action::NewWindow => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
            }
            Action::ClosePane => self.request_close_pane(ctx, self.layout.focused_pane()),
            Action::CloseTab => {
                for p in self.layout.active_tab().panes() {
                    self.request_close_pane(ctx, p);
                }
            }
            Action::NextTab => self.layout.next_tab(),
            Action::PrevTab => self.layout.prev_tab(),
            Action::GotoTab(n) => self.layout.goto_tab(n as usize),
            Action::SplitRight => self.new_pane_in_active_tab(ctx, Some(Axis::X)),
            Action::SplitDown => self.new_pane_in_active_tab(ctx, Some(Axis::Y)),
            Action::FocusLeft => self.layout.focus_direction(FocusDir::Left),
            Action::FocusRight => self.layout.focus_direction(FocusDir::Right),
            Action::FocusUp => self.layout.focus_direction(FocusDir::Up),
            Action::FocusDown => self.layout.focus_direction(FocusDir::Down),
            Action::ZoomPane => self.layout.toggle_zoom(),
            Action::Copy => self.copy_focused(),
            Action::Paste => self.start_paste(ctx, self.clipboard.get_text().unwrap_or_default()),
            Action::SemanticCopy => self.copy_focused(),
            Action::SelectAll => self.with_focused_term(|t| t.sel_all()),
            Action::Find => {
                self.search_active = !self.search_active;
                if !self.search_active {
                    self.search_query.clear();
                }
            }
            Action::FindNext | Action::FindPrev => {} // handled inline by the search bar
            Action::ZoomIn => self.set_zoom(1.0, ctx),
            Action::ZoomOut => self.set_zoom(-1.0, ctx),
            Action::ZoomReset => self.set_zoom(self.config.font.size - self.metrics_font_size, ctx),
            Action::ScrollPageUp => self.with_focused_term(|t| { let r = t.rows(); t.scroll_display(r as isize); }),
            Action::ScrollPageDown => self.with_focused_term(|t| { let r = t.rows(); t.scroll_display(-(r as isize)); }),
            Action::ScrollTop => self.with_focused_term(|t| { let n = t.scrollback_len(); t.scroll_display(n as isize); }),
            Action::ScrollBottom => self.with_focused_term(|t| t.scroll_to_bottom()),
            Action::ScrollLineUp => self.with_focused_term(|t| { t.scroll_display(1); }),
            Action::ScrollLineDown => self.with_focused_term(|t| { t.scroll_display(-1); }),
            Action::PrevPrompt => self.with_focused_term(|t| {
                let cur = t.view_abs(0);
                if let Some(l) = t.jump_prompt(cur, true) {
                    t.scroll_to_abs(l);
                }
            }),
            Action::NextPrompt => self.with_focused_term(|t| {
                let cur = t.view_abs(0);
                if let Some(l) = t.jump_prompt(cur, false) {
                    t.scroll_to_abs(l);
                }
            }),
            Action::ClearScrollback => self.with_focused_term(|t| t.clear_scrollback()),
            Action::ResetTerminal => self.with_focused_term(|t| t.reset()),
            Action::CommandPalette => {
                self.show_palette = !self.show_palette;
                self.palette_query.clear();
            }
            Action::CommandHistory => self.show_palette = true,
            Action::Settings => self.show_settings = !self.show_settings,
            Action::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            Action::ReloadConfig => self.reload_config(ctx),
        }
    }

    fn send_focus_report(&self, pane_id: PaneId, focused: bool) {
        let wants = self.with_pane_term_ret(pane_id, |t| t.modes.focus_events).unwrap_or(false);
        if wants {
            if let Some(p) = self.panes.get(&pane_id) {
                p.session.write(input::focus_report(focused).to_vec());
            }
        }
    }

    fn with_focused_term(&mut self, f: impl FnOnce(&mut Term)) {
        if let Some(p) = self.panes.get(&self.layout.focused_pane()) {
            if let Ok(mut t) = p.session.term().lock() {
                f(&mut t);
            }
        }
    }

    fn copy_focused(&mut self) {
        let text = self.panes.get(&self.layout.focused_pane()).and_then(|p| p.session.term().lock().ok().and_then(|t| t.selection_text()));
        if let Some(t) = text {
            self.clipboard.set_text(t);
        }
    }

    fn start_paste(&mut self, _ctx: &egui::Context, text: String) {
        if text.is_empty() {
            return;
        }
        let pane = self.layout.focused_pane();
        let bracketed = self.panes.get(&pane).map(|p| p.session.term().lock().map(|t| t.modes.bracketed_paste).unwrap_or(false)).unwrap_or(false);
        let paste = security::prepare_paste(&text, bracketed, &self.config.paste_config());
        if paste.refused {
            self.announce.push("Paste was too large and was not sent", true);
            return;
        }
        match &paste.needs_confirm {
            Some(reason) => self.pending_paste = Some(PendingPaste { pane, reason: reason.clone(), paste }),
            None => {
                if let Some(p) = self.panes.get(&pane) {
                    p.session.write(paste.bytes);
                }
            }
        }
    }

    fn open_link(&mut self, pane: PaneId, url: String, visible_text: Option<String>) {
        let decision = security::evaluate_link(&url, visible_text.as_deref(), &self.config.link_policy());
        match decision {
            LinkDecision::Open => {
                crate::platform::open_external(&url);
            }
            LinkDecision::Confirm(reason) => self.pending_link = Some(PendingLink { pane, url, reason }),
            LinkDecision::Deny(_) => {}
        }
    }

    // ----- per-frame pane update -------------------------------------------

    /// `NotifyCfg::rate_limit_per_min`: true at most that many times within
    /// any trailing 60s window; also records the call so it counts toward
    /// the next check.
    fn notify_allowed(&mut self, now: f64) -> bool {
        while let Some(&t) = self.notify_times.front() {
            if now - t > 60.0 {
                self.notify_times.pop_front();
            } else {
                break;
            }
        }
        if self.notify_times.len() >= self.config.notifications.rate_limit_per_min as usize {
            return false;
        }
        self.notify_times.push_back(now);
        true
    }

    fn drain_pane_events(&mut self, ctx: &egui::Context, now: f64, id: PaneId) {
        let (events, exited_now, foreground_name) = {
            let pane = match self.panes.get(&id) {
                Some(p) => p,
                None => return,
            };
            // Named binding (rather than chaining `.term().lock()` straight
            // into the match) so the cloned `Arc` lives long enough for the
            // `MutexGuard` it hands out — chaining would drop the `Arc`
            // temporary at the end of this statement while `t` is still
            // borrowing from it.
            let term_arc = pane.session.term();
            let mut t = match term_arc.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            (t.take_events(), !pane.session.is_running(), pane.session.foreground_name())
        };
        let long_running = self.config.notifications.long_command_secs;
        for ev in events {
            match ev {
                TermEvent::PtyWrite(bytes) => {
                    if let Some(p) = self.panes.get(&id) {
                        p.session.write(bytes);
                    }
                }
                TermEvent::Bell => {
                    if self.config.bell.visual {
                        self.announce.push("Bell", false);
                    }
                    if self.config.bell.audible {
                        crate::a11y::ring_bell();
                    }
                    if self.config.bell.urgent {
                        crate::platform::request_urgent("MITOS Terminal");
                    }
                }
                TermEvent::Notify { title, body } => {
                    self.announce.push(format!("{title}: {body}"), false);
                    if self.notify_allowed(now) {
                        crate::ipc::notify_user(title, body);
                    }
                }
                TermEvent::MissingCommand(cmd) => {
                    let term_arc = self.panes.get(&id).map(|p| p.session.term());
                    if let Some(term_arc) = term_arc {
                        spawn_pkg_suggestion(term_arc, ctx.clone(), cmd);
                    }
                }
                TermEvent::AutocompleteRequest(partial) => {
                    if let Some(p) = self.panes.get(&id) {
                        crate::ipc::request_autocomplete(p.session.term(), ctx.clone(), partial);
                    }
                }
                TermEvent::WidgetCommand { command, .. } => {
                    if let Some(p) = self.panes.get(&id) {
                        p.session.write(format!("{command}\n").into_bytes());
                    }
                }
                TermEvent::CommandFinished { duration, command, exit } => {
                    if duration.as_secs() >= long_running {
                        let ok = exit == Some(0);
                        self.announce.push(format!("{} finished{}", short_cmd(&command), if ok { "" } else { " with an error" }), !ok);
                        if self.config.notifications.only_when_unfocused && self.notify_allowed(now) {
                            crate::ipc::notify_user("Command finished".into(), short_cmd(&command));
                        }
                    }
                }
                TermEvent::ClipboardStore { text } => {
                    // OSC 52. `take_events()` already drained this from the
                    // term above, so it is handled here rather than via a
                    // second drain (which would find nothing left to take).
                    self.clipboard.set_text(text);
                }
                TermEvent::ColorsChanged | TermEvent::Title(_) | TermEvent::Cwd(_) | TermEvent::Progress { .. } | TermEvent::CommandStarted { .. } | TermEvent::BlockClosed { .. } => {}
            }
        }
        if exited_now {
            if let Some(pane) = self.panes.get_mut(&id) {
                if !pane.notified_finish {
                    pane.notified_finish = true;
                    if let SessionState::Exited(info) = pane.session.state() {
                        self.announce.push(format!("Shell {}", info.describe()), false);
                    }
                }
            }
        }
        let _ = foreground_name;
    }

    fn tick_pane(&mut self, now: f64, dt: f32, id: PaneId) -> bool {
        let interval = (self.config.cursor.blink_interval_ms as f64 / 1000.0).max(0.05);
        let reduced = self.config.reduced_motion(&self.home);
        let mut animating = false;
        if let Some(pane) = self.panes.get_mut(&id) {
            if !reduced && now - pane.last_blink_flip >= interval {
                pane.blink_on = !pane.blink_on;
                pane.last_blink_flip = now;
            }
            if let Ok(t) = pane.session.term().lock() {
                if t.seqno() != pane.last_seqno {
                    pane.last_seqno = t.seqno();
                    animating = true;
                }
                if !t.fresh().is_empty() {
                    animating = true;
                }
            }
            let _ = dt;
        }
        animating
    }
}

fn short_cmd(cmd: &str) -> String {
    let mut s = cmd.split_whitespace().next().unwrap_or(cmd).to_string();
    s.truncate(40);
    s
}

fn spawn_pkg_suggestion(term: Arc<Mutex<Term>>, ctx: egui::Context, cmd: String) {
    std::thread::spawn(move || {
        if let Some(widget) = crate::pkg_bridge::suggest_install(&cmd) {
            if let Ok(mut t) = term.lock() {
                t.inject_widget(widget, true);
            }
            ctx.request_repaint();
        }
    });
}

fn spawn_config_watcher(dirty: Arc<AtomicBool>, ctx: egui::Context) -> Option<RecommendedWatcher> {
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let (tx, rx) = std_mpsc::channel();
    let mut watcher = match RecommendedWatcher::new(move |res| { let _ = tx.send(res); }, NotifyConfig::default()) {
        Ok(w) => w,
        Err(_) => return None,
    };
    if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
        return None;
    }
    std::thread::spawn(move || {
        for event in rx.into_iter().flatten() {
            let relevant = event.paths.iter().any(|p| {
                matches!(p.file_name().and_then(|n| n.to_str()), Some("terminal.toml") | Some("home.conf"))
            });
            if relevant {
                dirty.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
        }
    });
    Some(watcher)
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

impl eframe::App for TerminalApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_count += 1;
        if self.config_dirty.swap(false, Ordering::Relaxed) {
            self.reload_config(ctx);
        }
        self.resolve_theme_overrides();

        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = now;

        // ----- global keyboard shortcuts (menu-style actions) --------------
        let focused_pane = self.layout.focused_pane();
        // DECSET 1004: tell whichever pane opted in that it gained/lost
        // focus, whenever moving between panes/tabs changes which one that is.
        if self.last_focused_pane != Some(focused_pane) {
            if let Some(old) = self.last_focused_pane {
                self.send_focus_report(old, false);
            }
            self.send_focus_report(focused_pane, true);
            self.last_focused_pane = Some(focused_pane);
        }
        let actions: Vec<Action> = ctx.input(|i| {
            let mods = i.modifiers;
            self.keymap
                .iter()
                .filter(|(c, _)| i.key_pressed(c.key) && c.matches(c.key, mods.ctrl, mods.shift, mods.alt))
                .map(|(_, a)| *a)
                .collect()
        });
        for a in actions {
            self.dispatch_action(ctx, a);
        }

        // ----- dialogs -------------------------------------------------
        self.show_settings_window(ctx);
        self.show_command_palette(ctx);
        self.show_paste_confirm(ctx);
        self.show_link_confirm(ctx);
        self.show_close_confirm(ctx);

        if self.search_active {
            egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("\u{1F50D}");
                    let resp = ui.text_edit_singleline(&mut self.search_query);
                    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        resp.request_focus();
                    }
                    if ui.button("\u{2716}").clicked() {
                        self.search_active = false;
                        self.search_query.clear();
                    }
                });
            });
        }

        // ----- tab bar -------------------------------------------------
        if self.config.general.tab_bar == "always" || (self.config.general.tab_bar == "auto" && self.layout.tabs().len() > 1) {
            egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let active = self.layout.active_index();
                    let mut goto: Option<usize> = None;
                    let mut close: Option<PaneId> = None;
                    for (i, tab) in self.layout.tabs().iter().enumerate() {
                        let label = tab.title.clone().unwrap_or_else(|| {
                            self.pane_title(tab.focused()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Tab {}", i + 1))
                        });
                        if ui.selectable_label(i == active, label).clicked() {
                            goto = Some(i + 1);
                        }
                        if ui.small_button("\u{2716}").clicked() {
                            close = Some(tab.focused());
                        }
                        ui.separator();
                    }
                    if ui.button("+").clicked() {
                        self.new_pane_in_active_tab(ctx, None);
                    }
                    if let Some(g) = goto {
                        self.layout.goto_tab(g);
                        self.registry.set_active(self.layout.focused_pane());
                    }
                    if let Some(p) = close {
                        self.request_close_pane(ctx, p);
                    }
                });
            });
        }

        // ----- settings gear + status ------------------------------------
        egui::Area::new(egui::Id::new("gear_area"))
            .fixed_pos(Pos2::new(ctx.screen_rect().right() - 36.0, ctx.screen_rect().top() + 6.0))
            .show(ctx, |ui| {
                if ui.button("\u{2699}").on_hover_text("Settings (Ctrl+,)").clicked() {
                    self.show_settings = !self.show_settings;
                }
            });

        // ----- central panel: the pane grid --------------------------------
        let bg = self.theme.bg;
        let panel_alpha = (self.config.window.opacity.clamp(0.1, 1.0) * 255.0) as u8;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::from_rgba_unmultiplied(bg[0], bg[1], bg[2], panel_alpha)))
            .show(ctx, |ui| {
                let area = ui.max_rect();
                let cell = self.metrics;
                let rects = self.layout.active_tab().rects(LRect::FULL);
                let mut any_animating = false;
                let hover_pos = ctx.input(|i| i.pointer.hover_pos());
                let effects = self.config.effects.clone();
                let accent = self.theme.accent;
                let reduced = self.config.reduced_motion(&self.home);
                let unfocused_hollow = self.config.cursor.unfocused == "hollow";
                let thickness = self.config.cursor.thickness;

                for (pane_id, nr) in rects {
                    let px = ERect::from_min_max(
                        Pos2::new(area.left() + nr.x0 * area.width(), area.top() + nr.y0 * area.height()),
                        Pos2::new(area.left() + nr.x1 * area.width(), area.top() + nr.y1 * area.height()),
                    );
                    if px.width() < 1.0 || px.height() < 1.0 {
                        continue;
                    }

                    let cols = ((px.width() / cell.char_w).floor() as usize).max(MIN_COLS);
                    let rows = ((px.height() / cell.row_h).floor() as usize).max(MIN_ROWS);
                    if let Some(pane) = self.panes.get(&pane_id) {
                        pane.session.resize(cols, rows, 0, 0);
                    }

                    self.drain_pane_events(ctx, now, pane_id);
                    if self.tick_pane(now, dt, pane_id) {
                        any_animating = true;
                    }

                    let resp = ui.interact(px, egui::Id::new(("pane", pane_id)), Sense::click_and_drag());
                    if resp.clicked() || resp.drag_started() {
                        self.layout.focus(pane_id);
                        self.registry.set_active(pane_id);
                        resp.request_focus();
                    }
                    let focused = self.layout.focused_pane() == pane_id;

                    let hover_cell = hover_pos.filter(|_| px.contains(hover_pos.unwrap())).map(|p| {
                        let c = ((p.x - px.left()) / cell.char_w).floor().max(0.0) as usize;
                        let r = ((p.y - px.top()) / cell.row_h).floor().max(0.0) as usize;
                        (c.min(cols.saturating_sub(1)), r.min(rows.saturating_sub(1)))
                    });
                    if let Some(pane) = self.panes.get_mut(&pane_id) {
                        pane.hover_cell = hover_cell;
                    }

                    self.handle_pane_mouse(ctx, &resp, pane_id, px, cell, cols, rows);
                    if focused {
                        self.handle_pane_keyboard(ctx, pane_id);
                    }

                    if let Some(pane) = self.panes.get_mut(&pane_id) {
                        let term_arc = pane.session.term();
                        if let Ok(mut term) = term_arc.lock() {
                            let opts = PaintOpts {
                                theme: &self.theme,
                                metrics: cell,
                                font: self.font.clone(),
                                focused,
                                cursor_visible_phase: pane.blink_on,
                                unfocused_hollow,
                                cursor_thickness: thickness,
                                show_search: if self.search_active && !self.search_query.is_empty() { Some(self.search_query.as_str()) } else { None },
                                reduced_motion: reduced,
                                effects: &effects,
                                accent,
                                now,
                                hover_cell: pane.hover_cell,
                            };
                            if render::paint_pane(ui, px, &mut term, &opts, pane.rain.as_mut(), dt) {
                                any_animating = true;
                            }
                        }
                    }

                    if rects_len(&self.layout, pane_id) > 1 {
                        ui.painter().rect_stroke(px, 0.0, egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(accent[0], accent[1], accent[2], 40)));
                    }
                }

                if any_animating || self.pending_paste.is_some() || self.pending_link.is_some() {
                    ctx.request_repaint();
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(250));
                }
            });

        for a in self.announce.drain() {
            let speak_on = self.config.accessibility.speak_output && self.speech_on;
            crate::a11y::speak(&a.text, speak_on, a.urgent);
        }
        if !self.speech_checked {
            self.speech_checked = true;
            self.speech_on = crate::a11y::speech_available();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = std::fs::remove_file(&self.socket_path);
        for (_, mut pane) in self.panes.drain() {
            pane.session.shutdown();
        }
    }
}

fn rects_len(layout: &Layout, pane: PaneId) -> usize {
    layout.tabs().iter().find(|t| t.panes().contains(&pane)).map(|t| t.panes().len()).unwrap_or(1)
}

impl TerminalApp {
    fn handle_pane_mouse(&mut self, ctx: &egui::Context, resp: &egui::Response, pane_id: PaneId, px: ERect, cell: Metrics, cols: usize, _rows: usize) {
        let cell_of = |p: Pos2| -> (usize, usize) {
            let c = ((p.x - px.left()) / cell.char_w).floor().max(0.0) as usize;
            let r = ((p.y - px.top()) / cell.row_h).floor().max(0.0) as usize;
            (c.min(cols.saturating_sub(1)), r)
        };
        let sel_cfg = self.config.selection.clone();
        let links_cfg = self.config.links.clone();

        if resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (c, r) = cell_of(pos);
                self.with_pane_term(pane_id, |t| {
                    let p = t.view_point(r, c);
                    t.sel_begin(SelMode::Word, p);
                });
            }
        } else if resp.triple_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (c, r) = cell_of(pos);
                self.with_pane_term(pane_id, |t| {
                    let p = t.view_point(r, c);
                    t.sel_begin(SelMode::Line, p);
                });
            }
        } else if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (c, r) = cell_of(pos);
                let mode = if ctx.input(|i| i.modifiers.alt) { crate::term::SelMode::Block } else { SelMode::Simple };
                self.with_pane_term(pane_id, |t| {
                    let p = t.view_point(r, c);
                    t.sel_begin(mode, p);
                });
                if let Some(p) = self.panes.get_mut(&pane_id) {
                    p.drag_active = true;
                }
            }
        } else if resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (c, r) = cell_of(pos);
                self.with_pane_term(pane_id, |t| {
                    let p = t.view_point(r, c);
                    t.sel_update(p);
                });
            }
        } else if resp.drag_stopped() {
            if let Some(p) = self.panes.get_mut(&pane_id) {
                p.drag_active = false;
            }
            if sel_cfg.copy_on_select {
                let text = self.pane_selection_text(pane_id);
                if let Some(t) = text {
                    if !t.is_empty() {
                        self.clipboard.set_text(t.clone());
                        if sel_cfg.primary_selection {
                            self.clipboard.set_primary(t);
                        }
                    }
                }
            } else if sel_cfg.primary_selection {
                if let Some(t) = self.pane_selection_text(pane_id) {
                    if !t.is_empty() {
                        self.clipboard.set_primary(t);
                    }
                }
            }
        }

        if resp.clicked() && !resp.double_clicked() && !resp.triple_clicked() {
            let want_link = ctx.input(|i| match links_cfg.mode.as_str() {
                "click" => true,
                "off" => false,
                _ => i.modifiers.ctrl,
            });
            if want_link {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let (c, r) = cell_of(pos);
                    let hit = self.with_pane_term_ret(pane_id, |t| {
                        let line = t.view_abs(r);
                        t.url_at(line, c).map(|u| u.url)
                    });
                    if let Some(url) = hit.flatten() {
                        self.open_link(pane_id, url, None);
                    }
                }
            } else {
                self.with_pane_term(pane_id, |t| t.sel_clear());
            }
        }

        if resp.clicked_by(egui::PointerButton::Middle) && sel_cfg.middle_click_paste {
            if let Some(text) = self.clipboard.get_primary() {
                self.start_paste(ctx, text);
            }
        }

        if resp.hovered() {
            let scroll = ctx.input(|i| i.raw_scroll_delta.y);
            if scroll.abs() > 0.01 {
                let in_alt = self.with_pane_term_ret(pane_id, |t| t.in_alt_screen()).unwrap_or(false);
                let lines = ((scroll / cell.row_h.max(1.0)) * self.config.scrollback.wheel_lines).round() as isize;
                if in_alt && self.config.scrollback.wheel_lines > 0.0 {
                    let app_cursor = self.with_pane_term_ret(pane_id, |t| t.modes.app_cursor).unwrap_or(false);
                    if lines != 0 {
                        let bytes = input::wheel_as_arrows(lines > 0, lines.unsigned_abs(), app_cursor);
                        if let Some(p) = self.panes.get(&pane_id) {
                            p.session.write(bytes);
                        }
                    }
                } else if lines != 0 {
                    self.with_pane_term(pane_id, |t| {
                        t.scroll_display(lines);
                    });
                }
            }
        }
    }

    fn handle_pane_keyboard(&mut self, ctx: &egui::Context, pane_id: PaneId) {
        let app_cursor = self.with_pane_term_ret(pane_id, |t| t.modes.app_cursor).unwrap_or(false);
        let ctrl_v_pastes = self.config.paste.ctrl_v_pastes;
        let ctrl_c_copies = self.config.selection.ctrl_c_copies;
        let has_selection = self.with_pane_term_ret(pane_id, |t| t.has_selection()).unwrap_or(false);

        let mut outgoing: Vec<u8> = Vec::new();
        let mut do_copy = false;
        let mut do_paste: Option<String> = None;

        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        if text.chars().any(|c| c.is_control()) {
                            continue;
                        }
                        outgoing.extend(input::encode_text(text, i.modifiers.alt && !i.modifiers.ctrl));
                    }
                    egui::Event::Key { key, pressed: true, modifiers, .. } => {
                        let mods = Mods::from_egui(modifiers);
                        if *key == egui::Key::C && mods.ctrl && mods.shift {
                            do_copy = true;
                            continue;
                        }
                        if *key == egui::Key::C && mods.ctrl && ctrl_c_copies && has_selection {
                            do_copy = true;
                            continue;
                        }
                        if *key == egui::Key::V && mods.ctrl && ctrl_v_pastes {
                            continue; // handled via Event::Paste below
                        }
                        if let Some(bytes) = input::encode_key(*key, mods, app_cursor) {
                            outgoing.extend(bytes);
                        }
                    }
                    egui::Event::Paste(text) => do_paste = Some(text.clone()),
                    egui::Event::Copy => do_copy = true,
                    _ => {}
                }
            }
        });

        if do_copy {
            self.copy_focused();
        }
        if let Some(text) = do_paste {
            self.start_paste(ctx, text);
        }
        if !outgoing.is_empty() {
            if let Some(p) = self.panes.get(&pane_id) {
                p.session.write(outgoing);
            }
            if self.config.general.scroll_on_input {
                self.with_pane_term(pane_id, |t| t.scroll_to_bottom());
            }
        }
    }

    fn with_pane_term(&self, pane_id: PaneId, f: impl FnOnce(&mut Term)) {
        if let Some(p) = self.panes.get(&pane_id) {
            if let Ok(mut t) = p.session.term().lock() {
                f(&mut t);
            }
        }
    }

    fn with_pane_term_ret<R>(&self, pane_id: PaneId, f: impl FnOnce(&mut Term) -> R) -> Option<R> {
        self.panes.get(&pane_id).and_then(|p| p.session.term().lock().ok().map(|mut t| f(&mut t)))
    }

    fn pane_selection_text(&self, pane_id: PaneId) -> Option<String> {
        self.with_pane_term_ret(pane_id, |t| t.selection_text()).flatten()
    }

    fn pane_title(&self, pane_id: PaneId) -> Option<String> {
        self.with_pane_term_ret(pane_id, |t| t.title().to_string())
    }

    // ----- dialogs -----------------------------------------------------

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut save = false;
        egui::Window::new("\u{2699}\u{FE0F} MITOS Terminal Settings").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.heading("Appearance");
            ui.separator();
            egui::ComboBox::from_label("Theme").selected_text(self.config.theme.name.clone()).show_ui(ui, |ui| {
                for name in Theme::names() {
                    if ui.selectable_label(self.config.theme.name == *name, *name).clicked() {
                        self.config.theme.name = name.to_string();
                        save = true;
                    }
                }
            });
            if ui.add(egui::Slider::new(&mut self.config.font.size, config::MIN_FONT..=config::MAX_FONT).text("Font size")).changed() {
                save = true;
            }
            ui.separator();
            ui.heading("Effects");
            let mut rain_on = self.config.effects.rain.unwrap_or(true);
            if ui.checkbox(&mut rain_on, "Matrix code rain").changed() {
                self.config.effects.rain = Some(rain_on);
                crate::config::HomeConf::save_matrix_rain(rain_on);
                save = true;
            }
            if ui.checkbox(&mut self.config.effects.scanlines, "CRT scanlines").changed() {
                save = true;
            }
            if ui.checkbox(&mut self.config.effects.phosphor, "Phosphor glow on new text").changed() {
                save = true;
            }
            ui.separator();
            ui.heading("Accessibility");
            if ui.checkbox(&mut self.config.accessibility.high_contrast, "High contrast").changed() {
                save = true;
            }
            if ui.checkbox(&mut self.config.accessibility.reduced_motion, "Reduce motion").changed() {
                save = true;
            }
            if ui.checkbox(&mut self.config.accessibility.speak_output, "Speak notifications (needs speech-dispatcher)").changed() {
                save = true;
            }
            ui.separator();
            ui.heading("Security");
            if ui.checkbox(&mut self.config.security.osc52_write, "Allow programs to write the clipboard (OSC 52)").changed() {
                save = true;
            }
            if ui.checkbox(&mut self.config.security.widgets, "Allow programs to show interactive widgets").changed() {
                save = true;
            }
            ui.separator();
            if !self.startup_errors.is_empty() {
                ui.colored_label(Color32::from_rgb(230, 120, 90), "Configuration warnings:");
                for e in &self.startup_errors {
                    ui.label(format!("\u{2022} {e}"));
                }
            }
            ui.add_space(6.0);
            ui.label("Saved automatically to ~/.config/mitos/terminal.toml");
        });
        self.show_settings = open;
        if save {
            self.config.sanitize();
            let _ = self.config.save();
            self.theme = self.config.theme(&self.home);
            self.font = render::font_id(&self.config.font.family, self.config.font.size);
            self.metrics = render::measure(ctx, &self.font);
            self.metrics_font_size = self.config.font.size;
            for pane in self.panes.values_mut() {
                let policy = self.config.term_policy();
                if let Ok(mut t) = pane.session.term().lock() {
                    t.set_policy(policy);
                }
            }
        }
    }

    fn show_command_palette(&mut self, ctx: &egui::Context) {
        if !self.show_palette {
            return;
        }
        let mut open = true;
        let mut chosen: Option<Action> = None;
        egui::Window::new("Command Palette").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_TOP, [0.0, 60.0]).show(ctx, |ui| {
            let resp = ui.text_edit_singleline(&mut self.palette_query);
            resp.request_focus();
            let q = self.palette_query.to_ascii_lowercase();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (action, _name, label) in config::ACTIONS {
                    if !q.is_empty() && !label.to_ascii_lowercase().contains(&q) {
                        continue;
                    }
                    let shortcut = config::shortcut_for(&self.keymap, *action).unwrap_or_default();
                    if ui.selectable_label(false, format!("{label}    {shortcut}")).clicked() {
                        chosen = Some(*action);
                    }
                }
            });
        });
        self.show_palette = open;
        if let Some(a) = chosen {
            self.show_palette = false;
            self.dispatch_action(ctx, a);
        }
    }

    fn show_paste_confirm(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_paste else { return };
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Confirm paste").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("This paste needs confirmation: {}", pending.reason));
            let preview = security::paste_preview(&String::from_utf8_lossy(&pending.paste.bytes), 6, 80);
            ui.add(egui::Label::new(egui::RichText::new(preview).monospace()).wrap());
            ui.horizontal(|ui| {
                if ui.button("Paste anyway").clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if go {
            if let Some(pending) = self.pending_paste.take() {
                if let Some(p) = self.panes.get(&pending.pane) {
                    p.session.write(pending.paste.bytes);
                }
            }
        } else if cancel {
            self.pending_paste = None;
        }
    }

    fn show_link_confirm(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_link else { return };
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Open link?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(&pending.url);
            ui.label(&pending.reason);
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if go {
            if let Some(pending) = self.pending_link.take() {
                crate::platform::open_external(&pending.url);
            }
        } else if cancel {
            self.pending_link = None;
        }
    }

    fn show_close_confirm(&mut self, ctx: &egui::Context) {
        let Some(pane_id) = self.pending_close else { return };
        let name = self.panes.get(&pane_id).and_then(|p| p.session.foreground_name()).unwrap_or_else(|| "a running program".to_string());
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Close pane?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("{name} is still running. Close this pane anyway?"));
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if go {
            self.pending_close = None;
            self.close_pane(pane_id);
        } else if cancel {
            self.pending_close = None;
        }
    }
}
