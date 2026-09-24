//! Configuration.
//!
//! * `~/.config/mitos/terminal.toml` — everything specific to this terminal
//!   (profiles, fonts, cursor, effects, policy, key bindings). Every table and
//!   key is optional; a partial file is fine and a broken one never crashes the
//!   terminal (the error is reported and the previous config stays active).
//! * `~/.config/mitos/home.conf` — system-wide look & feel shared with the rest
//!   of MITOS (`theme_mode`, `accent_color`, `matrix_rain`). Read for backwards
//!   compatibility; `terminal.toml` wins where both speak.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::Key;
use serde::{Deserialize, Serialize};

use crate::security::{LinkMode, LinkPolicy, PasteConfig};
use crate::term::{CursorShape, CursorStyle, Policy};
use crate::theme::{parse_hex, Rgb, Theme};

pub const MIN_FONT: f32 = 6.0;
pub const MAX_FONT: f32 = 72.0;

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join("mitos")
}

// ---------------------------------------------------------------------------
// terminal.toml
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub default_profile: String,
    /// Ask before closing a pane that still has a foreground job running.
    pub confirm_close_running: bool,
    /// `"close"` the pane/tab when the shell exits, or `"hold"` it open with the exit status.
    pub on_exit: String,
    pub scroll_on_input: bool,
    pub scroll_on_output: bool,
    pub smooth_scroll: bool,
    /// `"auto"` (only with >1 tab), `"always"` or `"never"`.
    pub tab_bar: String,
    /// Inject OSC 133/7 shell integration into bash automatically.
    pub shell_integration: bool,
}

impl Default for General {
    fn default() -> Self {
        General {
            default_profile: "default".into(),
            confirm_close_running: true,
            on_exit: "close".into(),
            scroll_on_input: true,
            scroll_on_output: false,
            smooth_scroll: true,
            tab_bar: "auto".into(),
            shell_integration: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontCfg {
    /// `"monospace"` = built-in font, otherwise a fontconfig family name.
    pub family: String,
    pub fallbacks: Vec<String>,
    pub size: f32,
    pub bold_is_bright: bool,
    /// Requires an OpenType shaper; see docs/AUDIT.md. Accepted so configs are forward-compatible.
    pub ligatures: bool,
    pub line_height: f32,
}

impl Default for FontCfg {
    fn default() -> Self {
        FontCfg { family: "monospace".into(), fallbacks: Vec::new(), size: 14.0, bold_is_bright: false, ligatures: true, line_height: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowCfg {
    pub opacity: f32,
    /// Frosted-glass look: tinted translucent background with a soft highlight.
    pub glass: bool,
    /// Ask the compositor to blur what is behind the window (X11/KWin-style; see docs).
    pub blur: bool,
    pub padding: f32,
    pub decorations: bool,
    pub width: f32,
    pub height: f32,
}

impl Default for WindowCfg {
    fn default() -> Self {
        WindowCfg { opacity: 1.0, glass: false, blur: false, padding: 6.0, decorations: true, width: 1000.0, height: 650.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorCfg {
    /// `"block"`, `"underline"` or `"bar"`.
    pub style: String,
    pub blink: bool,
    pub blink_interval_ms: u64,
    /// Shape used while the window is unfocused: `"hollow"` or `"same"`.
    pub unfocused: String,
    pub thickness: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Default for CursorCfg {
    fn default() -> Self {
        CursorCfg { style: "block".into(), blink: true, blink_interval_ms: 530, unfocused: "hollow".into(), thickness: 2.0, color: None }
    }
}

impl CursorCfg {
    pub fn style(&self) -> CursorStyle {
        let shape = match self.style.trim().to_ascii_lowercase().as_str() {
            "underline" => CursorShape::Underline,
            "bar" | "beam" | "ibeam" => CursorShape::Bar,
            _ => CursorShape::Block,
        };
        CursorStyle { shape, blink: self.blink }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScrollCfg {
    pub lines: usize,
    /// Lines per mouse-wheel notch.
    pub wheel_lines: f32,
}

impl Default for ScrollCfg {
    fn default() -> Self {
        ScrollCfg { lines: 10_000, wheel_lines: 3.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeCfg {
    pub name: String,
    /// Let `home.conf`'s `theme_mode` switch between mitos-dark / mitos-light.
    pub follow_system: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Optional 16 `#rrggbb` entries replacing the ANSI palette.
    pub palette: Vec<String>,
}

impl Default for ThemeCfg {
    fn default() -> Self {
        ThemeCfg { name: "mitos-dark".into(), follow_system: true, accent: None, foreground: None, background: None, selection: None, palette: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectsCfg {
    /// Master switch for all cinematic effects.
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rain: Option<bool>,
    pub grid: bool,
    pub sweep: bool,
    pub scanlines: bool,
    pub phosphor: bool,
    pub glitch: bool,
    /// Animation frame-rate cap for background effects.
    pub max_fps: u32,
    pub pause_unfocused: bool,
}

impl Default for EffectsCfg {
    fn default() -> Self {
        EffectsCfg { enabled: true, rain: None, grid: true, sweep: true, scanlines: true, phosphor: true, glitch: true, max_fps: 30, pause_unfocused: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BellCfg {
    pub visual: bool,
    pub audible: bool,
    /// Ask the window manager for attention when the bell rings in an unfocused window.
    pub urgent: bool,
    pub min_interval_ms: u64,
}

impl Default for BellCfg {
    fn default() -> Self {
        BellCfg { visual: true, audible: false, urgent: true, min_interval_ms: 200 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectionCfg {
    pub word_separators: String,
    pub copy_on_select: bool,
    /// Also publish the selection as the X11/Wayland PRIMARY selection.
    pub primary_selection: bool,
    pub middle_click_paste: bool,
    /// With a selection, Ctrl+C copies instead of interrupting.
    pub ctrl_c_copies: bool,
}

impl Default for SelectionCfg {
    fn default() -> Self {
        SelectionCfg {
            word_separators: " \t\"'`|()[]{}<>,;".into(),
            copy_on_select: false,
            primary_selection: true,
            middle_click_paste: true,
            ctrl_c_copies: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PasteCfg {
    pub confirm_multiline: bool,
    pub confirm_bytes: usize,
    pub max_bytes: usize,
    pub strip_controls: bool,
    /// Plain Ctrl+V pastes (Windows style). Off by default: Ctrl+V is literal-next in shells/vim.
    pub ctrl_v_pastes: bool,
}

impl Default for PasteCfg {
    fn default() -> Self {
        let d = PasteConfig::default();
        PasteCfg { confirm_multiline: d.confirm_multiline, confirm_bytes: d.confirm_bytes, max_bytes: d.max_bytes, strip_controls: d.strip_controls, ctrl_v_pastes: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinksCfg {
    /// `"ctrl_click"`, `"click"` or `"off"`.
    pub mode: String,
    pub allowed_schemes: Vec<String>,
    pub confirm_mismatch: bool,
    pub confirm_all: bool,
    pub detect_plain_urls: bool,
}

impl Default for LinksCfg {
    fn default() -> Self {
        let d = LinkPolicy::default();
        LinksCfg { mode: "ctrl_click".into(), allowed_schemes: d.allowed_schemes, confirm_mismatch: true, confirm_all: false, detect_plain_urls: true }
    }
}

/// What programs running inside the terminal may ask it to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityCfg {
    /// OSC 52 clipboard *writes* (reads are never answered).
    pub osc52_write: bool,
    pub title_changes: bool,
    pub notifications: bool,
    /// MROP widgets sent as escape sequences by programs.
    pub widgets: bool,
    pub cwd_reports: bool,
    pub hyperlinks: bool,
    pub color_changes: bool,
    pub color_queries: bool,
    /// Ask before running the command of a widget that a *program* (not MITOS itself) created.
    pub mrop_confirm: bool,
    /// Commands starting with one of these run without asking.
    pub mrop_trusted_prefixes: Vec<String>,
    /// Let local MITOS daemons read the terminal buffer over the IPC socket.
    pub ipc_buffer_read: bool,
    pub max_title: usize,
    pub max_clipboard_kb: usize,
    pub max_widgets: usize,
    pub max_panes: usize,
}

impl Default for SecurityCfg {
    fn default() -> Self {
        SecurityCfg {
            osc52_write: true,
            title_changes: true,
            notifications: true,
            widgets: true,
            cwd_reports: true,
            hyperlinks: true,
            color_changes: true,
            color_queries: true,
            mrop_confirm: true,
            mrop_trusted_prefixes: Vec::new(),
            ipc_buffer_read: true,
            max_title: 256,
            max_clipboard_kb: 100,
            max_widgets: 256,
            max_panes: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyCfg {
    /// Notify when a command that ran at least this long finishes.
    pub long_command_secs: u64,
    pub only_when_unfocused: bool,
    pub rate_limit_per_min: u32,
}

impl Default for NotifyCfg {
    fn default() -> Self {
        NotifyCfg { long_command_secs: 10, only_when_unfocused: true, rate_limit_per_min: 12 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct A11yCfg {
    pub high_contrast: bool,
    /// Minimum WCAG contrast ratio enforced on every run of text (0 = off; 4.5 = AA, 7 = AAA).
    pub min_contrast: f32,
    pub reduced_motion: bool,
    /// Publish the visible screen text to the platform accessibility tree (AT-SPI/Orca).
    pub screen_reader: bool,
    /// Speak new output through `spd-say` (speech-dispatcher), if installed.
    pub speak_output: bool,
    pub announce_bell: bool,
}

impl Default for A11yCfg {
    fn default() -> Self {
        A11yCfg { high_contrast: false, min_contrast: 0.0, reduced_motion: false, screen_reader: false, speak_output: false, announce_bell: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { name: "default".into(), shell: None, args: Vec::new(), cwd: None, env: BTreeMap::new(), theme: None, font_size: None, title: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub font: FontCfg,
    pub window: WindowCfg,
    pub cursor: CursorCfg,
    pub scrollback: ScrollCfg,
    pub theme: ThemeCfg,
    pub effects: EffectsCfg,
    pub bell: BellCfg,
    pub selection: SelectionCfg,
    pub paste: PasteCfg,
    pub links: LinksCfg,
    pub security: SecurityCfg,
    pub notifications: NotifyCfg,
    pub accessibility: A11yCfg,
    /// `"ctrl+shift+t" = "new_tab"`; the value `"none"` removes a default binding.
    pub keybindings: BTreeMap<String, String>,
    #[serde(rename = "profile")]
    pub profiles: Vec<Profile>,
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("terminal.toml")
    }

    /// Parse a TOML document; range-check the result.
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut cfg: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        cfg.sanitize();
        Ok(cfg)
    }

    /// Load `terminal.toml`. Returns the config and, if the file exists but
    /// could not be parsed, an error message (the defaults are used then).
    pub fn load() -> (Config, Option<String>) {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => match Config::parse(&text) {
                Ok(c) => (c, None),
                Err(e) => (Config::default(), Some(format!("terminal.toml: {e}"))),
            },
            Err(_) => (Config::default(), None),
        }
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = self.to_toml()?;
        // write-then-rename so a crash never leaves a truncated file behind
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// Clamp every numeric setting into a sane range.
    pub fn sanitize(&mut self) {
        self.font.size = self.font.size.clamp(MIN_FONT, MAX_FONT);
        self.font.line_height = self.font.line_height.clamp(0.8, 2.0);
        self.window.opacity = self.window.opacity.clamp(0.1, 1.0);
        self.window.padding = self.window.padding.clamp(0.0, 64.0);
        self.window.width = self.window.width.clamp(200.0, 10_000.0);
        self.window.height = self.window.height.clamp(150.0, 10_000.0);
        self.cursor.blink_interval_ms = self.cursor.blink_interval_ms.clamp(100, 5_000);
        self.cursor.thickness = self.cursor.thickness.clamp(1.0, 8.0);
        self.scrollback.lines = self.scrollback.lines.min(1_000_000);
        self.scrollback.wheel_lines = self.scrollback.wheel_lines.clamp(0.25, 50.0);
        self.effects.max_fps = self.effects.max_fps.clamp(5, 120);
        self.bell.min_interval_ms = self.bell.min_interval_ms.clamp(50, 60_000);
        self.accessibility.min_contrast = self.accessibility.min_contrast.clamp(0.0, 21.0);
        self.security.max_title = self.security.max_title.clamp(16, 4096);
        self.security.max_clipboard_kb = self.security.max_clipboard_kb.clamp(1, 10_240);
        self.security.max_widgets = self.security.max_widgets.clamp(1, 4096);
        self.security.max_panes = self.security.max_panes.clamp(1, 512);
        self.paste.max_bytes = self.paste.max_bytes.clamp(1024, 64 * 1024 * 1024);
        self.notifications.rate_limit_per_min = self.notifications.rate_limit_per_min.clamp(1, 600);
        if self.general.on_exit != "hold" {
            self.general.on_exit = "close".into();
        }
    }

    // ----- derived settings ---------------------------------------------

    /// The profile named `name`, else the default profile, else a built-in default.
    pub fn profile(&self, name: Option<&str>) -> Profile {
        let wanted = name.unwrap_or(self.general.default_profile.as_str());
        self.profiles
            .iter()
            .find(|p| p.name == wanted)
            .or_else(|| self.profiles.first().filter(|_| name.is_none() && self.general.default_profile == "default"))
            .cloned()
            .unwrap_or_default()
    }

    pub fn term_policy(&self) -> Policy {
        let s = &self.security;
        Policy {
            title: s.title_changes,
            clipboard_write: s.osc52_write,
            notifications: s.notifications,
            widgets: s.widgets,
            cwd: s.cwd_reports,
            hyperlinks: s.hyperlinks,
            color_changes: s.color_changes,
            color_queries: s.color_queries,
            max_title: s.max_title,
            max_uri: crate::security::MAX_URI_LEN,
            max_clipboard: s.max_clipboard_kb * 1024,
            max_widgets: s.max_widgets,
        }
    }

    pub fn link_policy(&self) -> LinkPolicy {
        LinkPolicy {
            mode: LinkMode::parse(&self.links.mode),
            allowed_schemes: self.links.allowed_schemes.clone(),
            confirm_mismatch: self.links.confirm_mismatch,
            confirm_all: self.links.confirm_all,
        }
    }

    pub fn paste_config(&self) -> PasteConfig {
        PasteConfig {
            confirm_multiline: self.paste.confirm_multiline,
            confirm_bytes: self.paste.confirm_bytes,
            max_bytes: self.paste.max_bytes,
            strip_controls: self.paste.strip_controls,
        }
    }

    /// Resolve the theme: named theme → `home.conf` light/dark follow → accent → per-colour overrides → high contrast.
    pub fn theme(&self, home: &HomeConf) -> Theme {
        let mut t = Theme::by_name(&self.theme.name).unwrap_or_else(Theme::dark);
        if self.theme.follow_system && (self.theme.name == "mitos-dark" || self.theme.name == "mitos-light") {
            match home.theme_mode.as_deref() {
                Some("light") => t = Theme::light(),
                Some("dark") => t = Theme::dark(),
                _ => {}
            }
        }
        if self.accessibility.high_contrast {
            t = if t.light { Theme::high_contrast_light() } else { Theme::high_contrast_dark() };
        }
        let accent = self.theme.accent.as_deref().and_then(parse_hex).or(home.accent);
        if let Some(a) = accent {
            t = t.with_accent(a);
        }
        if let Some(c) = self.theme.foreground.as_deref().and_then(parse_hex) {
            t.fg = c;
        }
        if let Some(c) = self.theme.background.as_deref().and_then(parse_hex) {
            t.bg = c;
        }
        if let Some(c) = self.theme.selection.as_deref().and_then(parse_hex) {
            t.selection = c;
        }
        if let Some(c) = self.cursor.color.as_deref().and_then(parse_hex) {
            t.cursor = c;
        }
        if self.theme.palette.len() == 16 {
            let parsed: Vec<Option<Rgb>> = self.theme.palette.iter().map(|s| parse_hex(s)).collect();
            if parsed.iter().all(|p| p.is_some()) {
                for (i, p) in parsed.into_iter().enumerate() {
                    t.ansi[i] = p.unwrap_or(t.ansi[i]);
                }
            }
        }
        t
    }

    pub fn rain_enabled(&self, home: &HomeConf) -> bool {
        self.effects.enabled && self.effects.rain.or(home.matrix_rain).unwrap_or(true)
    }

    pub fn reduced_motion(&self, home: &HomeConf) -> bool {
        let env = std::env::var("MITOS_REDUCED_MOTION").map(|v| v == "1" || v == "true").unwrap_or(false);
        self.accessibility.reduced_motion || env || home.reduced_motion.unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// home.conf
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HomeConf {
    pub theme_mode: Option<String>,
    pub accent: Option<Rgb>,
    pub matrix_rain: Option<bool>,
    pub reduced_motion: Option<bool>,
}

impl HomeConf {
    pub fn path() -> PathBuf {
        config_dir().join("home.conf")
    }

    pub fn parse(content: &str) -> HomeConf {
        let mut h = HomeConf::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(2, '=');
            let key = parts.next().map(|s| s.trim());
            let val = parts.next().map(|s| s.trim());
            let truthy = |v: &str| v == "true" || v == "1" || v == "yes";
            match (key, val) {
                (Some("theme_mode"), Some(v)) => h.theme_mode = Some(v.to_ascii_lowercase()),
                (Some("accent_color"), Some(v)) => h.accent = parse_hex(v),
                (Some("matrix_rain"), Some(v)) => h.matrix_rain = Some(truthy(v)),
                (Some("reduced_motion"), Some(v)) => h.reduced_motion = Some(truthy(v)),
                _ => {}
            }
        }
        h
    }

    pub fn load() -> HomeConf {
        std::fs::read_to_string(Self::path()).map(|c| HomeConf::parse(&c)).unwrap_or_default()
    }

    /// Persist `matrix_rain=` in home.conf, keeping every other line untouched.
    pub fn save_matrix_rain(enabled: bool) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let val = if enabled { "true" } else { "false" };
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let mut found = false;
        let mut lines: Vec<String> = content
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("matrix_rain=") {
                    found = true;
                    format!("matrix_rain={val}")
                } else {
                    l.to_string()
                }
            })
            .collect();
        if !found {
            lines.push(format!("matrix_rain={val}"));
        }
        let mut out = lines.join("\n");
        out.push('\n');
        let _ = std::fs::write(&path, out);
    }
}

// ---------------------------------------------------------------------------
// Actions and key bindings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Action {
    NewTab,
    NewWindow,
    ClosePane,
    CloseTab,
    NextTab,
    PrevTab,
    GotoTab(u8),
    SplitRight,
    SplitDown,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ZoomPane,
    Copy,
    Paste,
    SemanticCopy,
    SelectAll,
    Find,
    FindNext,
    FindPrev,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    ScrollLineUp,
    ScrollLineDown,
    PrevPrompt,
    NextPrompt,
    ClearScrollback,
    ResetTerminal,
    CommandPalette,
    CommandHistory,
    Settings,
    Fullscreen,
    ReloadConfig,
}

/// `(action, config name, human label)` — also feeds the command palette.
pub const ACTIONS: &[(Action, &str, &str)] = &[
    (Action::NewTab, "new_tab", "New tab"),
    (Action::NewWindow, "new_window", "New window"),
    (Action::ClosePane, "close_pane", "Close pane"),
    (Action::CloseTab, "close_tab", "Close tab"),
    (Action::NextTab, "next_tab", "Next tab"),
    (Action::PrevTab, "prev_tab", "Previous tab"),
    (Action::GotoTab(1), "goto_tab_1", "Go to tab 1"),
    (Action::GotoTab(2), "goto_tab_2", "Go to tab 2"),
    (Action::GotoTab(3), "goto_tab_3", "Go to tab 3"),
    (Action::GotoTab(4), "goto_tab_4", "Go to tab 4"),
    (Action::GotoTab(5), "goto_tab_5", "Go to tab 5"),
    (Action::GotoTab(6), "goto_tab_6", "Go to tab 6"),
    (Action::GotoTab(7), "goto_tab_7", "Go to tab 7"),
    (Action::GotoTab(8), "goto_tab_8", "Go to tab 8"),
    (Action::GotoTab(9), "goto_tab_9", "Go to last tab"),
    (Action::SplitRight, "split_right", "Split pane right"),
    (Action::SplitDown, "split_down", "Split pane down"),
    (Action::FocusLeft, "focus_left", "Focus pane left"),
    (Action::FocusRight, "focus_right", "Focus pane right"),
    (Action::FocusUp, "focus_up", "Focus pane up"),
    (Action::FocusDown, "focus_down", "Focus pane down"),
    (Action::ZoomPane, "zoom_pane", "Maximise / restore pane"),
    (Action::Copy, "copy", "Copy selection"),
    (Action::Paste, "paste", "Paste"),
    (Action::SemanticCopy, "semantic_copy", "Copy as path / file URI"),
    (Action::SelectAll, "select_all", "Select all"),
    (Action::Find, "find", "Find in scrollback"),
    (Action::FindNext, "find_next", "Find next"),
    (Action::FindPrev, "find_prev", "Find previous"),
    (Action::ZoomIn, "zoom_in", "Increase font size"),
    (Action::ZoomOut, "zoom_out", "Decrease font size"),
    (Action::ZoomReset, "zoom_reset", "Reset font size"),
    (Action::ScrollPageUp, "scroll_page_up", "Scroll page up"),
    (Action::ScrollPageDown, "scroll_page_down", "Scroll page down"),
    (Action::ScrollTop, "scroll_top", "Scroll to top"),
    (Action::ScrollBottom, "scroll_bottom", "Scroll to bottom"),
    (Action::ScrollLineUp, "scroll_line_up", "Scroll line up"),
    (Action::ScrollLineDown, "scroll_line_down", "Scroll line down"),
    (Action::PrevPrompt, "prev_prompt", "Jump to previous prompt"),
    (Action::NextPrompt, "next_prompt", "Jump to next prompt"),
    (Action::ClearScrollback, "clear_scrollback", "Clear scrollback"),
    (Action::ResetTerminal, "reset_terminal", "Reset terminal"),
    (Action::CommandPalette, "command_palette", "Command palette"),
    (Action::CommandHistory, "command_history", "Command history"),
    (Action::Settings, "settings", "Settings"),
    (Action::Fullscreen, "fullscreen", "Toggle fullscreen"),
    (Action::ReloadConfig, "reload_config", "Reload configuration"),
];

impl Action {
    pub fn name(self) -> &'static str {
        ACTIONS.iter().find(|(a, _, _)| *a == self).map(|(_, n, _)| *n).unwrap_or("?")
    }

    pub fn label(self) -> &'static str {
        ACTIONS.iter().find(|(a, _, _)| *a == self).map(|(_, _, l)| *l).unwrap_or("?")
    }

    pub fn from_name(name: &str) -> Option<Action> {
        let n = name.trim().to_ascii_lowercase();
        ACTIONS.iter().find(|(_, an, _)| *an == n).map(|(a, _, _)| *a)
    }
}

/// A key plus exact modifier state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

fn is_plus_or_equals(k: Key) -> bool {
    matches!(k.name(), "Plus" | "Equals")
}

impl Chord {
    /// `+` and `=` share a physical key, so Ctrl+= and Ctrl++ are the same chord.
    pub fn matches(&self, key: Key, ctrl: bool, shift: bool, alt: bool) -> bool {
        let key_ok = self.key == key || (is_plus_or_equals(self.key) && is_plus_or_equals(key));
        key_ok && self.ctrl == ctrl && self.alt == alt && (self.shift == shift || (is_plus_or_equals(self.key) && shift && ctrl))
    }
}

const KEY_NAMES: &[(&str, Key)] = &[
    ("a", Key::A),
    ("b", Key::B),
    ("c", Key::C),
    ("d", Key::D),
    ("e", Key::E),
    ("f", Key::F),
    ("g", Key::G),
    ("h", Key::H),
    ("i", Key::I),
    ("j", Key::J),
    ("k", Key::K),
    ("l", Key::L),
    ("m", Key::M),
    ("n", Key::N),
    ("o", Key::O),
    ("p", Key::P),
    ("q", Key::Q),
    ("r", Key::R),
    ("s", Key::S),
    ("t", Key::T),
    ("u", Key::U),
    ("v", Key::V),
    ("w", Key::W),
    ("x", Key::X),
    ("y", Key::Y),
    ("z", Key::Z),
    ("0", Key::Num0),
    ("1", Key::Num1),
    ("2", Key::Num2),
    ("3", Key::Num3),
    ("4", Key::Num4),
    ("5", Key::Num5),
    ("6", Key::Num6),
    ("7", Key::Num7),
    ("8", Key::Num8),
    ("9", Key::Num9),
    ("f1", Key::F1),
    ("f2", Key::F2),
    ("f3", Key::F3),
    ("f4", Key::F4),
    ("f5", Key::F5),
    ("f6", Key::F6),
    ("f7", Key::F7),
    ("f8", Key::F8),
    ("f9", Key::F9),
    ("f10", Key::F10),
    ("f11", Key::F11),
    ("f12", Key::F12),
    ("up", Key::ArrowUp),
    ("down", Key::ArrowDown),
    ("left", Key::ArrowLeft),
    ("right", Key::ArrowRight),
    ("home", Key::Home),
    ("end", Key::End),
    ("pageup", Key::PageUp),
    ("pagedown", Key::PageDown),
    ("insert", Key::Insert),
    ("delete", Key::Delete),
    ("tab", Key::Tab),
    ("enter", Key::Enter),
    ("space", Key::Space),
    ("backspace", Key::Backspace),
    ("escape", Key::Escape),
    ("minus", Key::Minus),
    ("equals", Key::Equals),
    ("comma", Key::Comma),
    ("period", Key::Period),
    ("slash", Key::Slash),
    ("backslash", Key::Backslash),
    ("semicolon", Key::Semicolon),
    ("backtick", Key::Backtick),
    ("openbracket", Key::OpenBracket),
    ("closebracket", Key::CloseBracket),
];

pub fn key_from_name(name: &str) -> Option<Key> {
    let n = name.trim().to_ascii_lowercase();
    let n = match n.as_str() {
        "esc" => "escape",
        "return" => "enter",
        "pgup" => "pageup",
        "pgdn" | "pgdown" => "pagedown",
        "ins" => "insert",
        "del" => "delete",
        "plus" | "=" => "equals",
        "-" => "minus",
        "," => "comma",
        "." => "period",
        "/" => "slash",
        "\\" => "backslash",
        ";" => "semicolon",
        "`" => "backtick",
        "[" => "openbracket",
        "]" => "closebracket",
        other => other,
    };
    KEY_NAMES.iter().find(|(k, _)| *k == n).map(|(_, key)| *key)
}

fn key_name(key: Key) -> &'static str {
    KEY_NAMES.iter().find(|(_, k)| *k == key).map(|(n, _)| *n).unwrap_or("?")
}

/// `"ctrl+shift+t"` → chord. Modifier order and case do not matter.
pub fn parse_chord(s: &str) -> Option<Chord> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key: Option<Key> = None;
    for part in s.split('+') {
        let p = part.trim().to_ascii_lowercase();
        match p.as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            "" => {
                // a literal '+' key at the end ("ctrl++")
                key = key_from_name("equals");
                shift = true;
            }
            other => key = Some(key_from_name(other)?),
        }
    }
    Some(Chord { key: key?, ctrl, shift, alt })
}

pub fn chord_to_string(c: &Chord) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if c.ctrl {
        parts.push("ctrl");
    }
    if c.shift {
        parts.push("shift");
    }
    if c.alt {
        parts.push("alt");
    }
    parts.push(key_name(c.key));
    parts.join("+")
}

/// The built-in bindings. (Copy/paste are handled specially — egui-winit turns
/// Ctrl+C/V/X into copy/paste events — so they are not listed here.)
pub fn default_bindings() -> Vec<(&'static str, Action)> {
    let mut v: Vec<(&'static str, Action)> = vec![
        ("ctrl+shift+t", Action::NewTab),
        ("ctrl+shift+n", Action::NewWindow),
        ("ctrl+shift+w", Action::ClosePane),
        ("ctrl+tab", Action::NextTab),
        ("ctrl+shift+tab", Action::PrevTab),
        ("ctrl+pagedown", Action::NextTab),
        ("ctrl+pageup", Action::PrevTab),
        ("ctrl+shift+d", Action::SplitRight),
        ("ctrl+shift+e", Action::SplitDown),
        ("alt+shift+left", Action::FocusLeft),
        ("alt+shift+right", Action::FocusRight),
        ("alt+shift+up", Action::FocusUp),
        ("alt+shift+down", Action::FocusDown),
        ("ctrl+shift+z", Action::ZoomPane),
        ("ctrl+shift+a", Action::SelectAll),
        ("ctrl+shift+f", Action::Find),
        ("ctrl+f", Action::Find),
        ("ctrl+g", Action::FindNext),
        ("ctrl+shift+g", Action::FindPrev),
        ("ctrl+equals", Action::ZoomIn),
        ("ctrl+minus", Action::ZoomOut),
        ("ctrl+0", Action::ZoomReset),
        ("shift+pageup", Action::ScrollPageUp),
        ("shift+pagedown", Action::ScrollPageDown),
        ("shift+home", Action::ScrollTop),
        ("shift+end", Action::ScrollBottom),
        ("ctrl+shift+up", Action::PrevPrompt),
        ("ctrl+shift+down", Action::NextPrompt),
        ("ctrl+shift+k", Action::ClearScrollback),
        ("ctrl+shift+r", Action::ResetTerminal),
        ("ctrl+shift+p", Action::CommandPalette),
        ("ctrl+shift+h", Action::CommandHistory),
        ("ctrl+comma", Action::Settings),
        ("f11", Action::Fullscreen),
    ];
    for n in 1u8..=9 {
        let s: &'static str = match n {
            1 => "alt+1",
            2 => "alt+2",
            3 => "alt+3",
            4 => "alt+4",
            5 => "alt+5",
            6 => "alt+6",
            7 => "alt+7",
            8 => "alt+8",
            _ => "alt+9",
        };
        v.push((s, Action::GotoTab(n)));
    }
    v
}

/// Defaults overlaid with the user's `[keybindings]`. A user entry replaces
/// every default binding of the same chord; `"none"` just removes it.
/// Returns the table plus human-readable problems (unknown key or action).
pub fn build_keymap(user: &BTreeMap<String, String>) -> (Vec<(Chord, Action)>, Vec<String>) {
    let mut map: Vec<(Chord, Action)> = default_bindings()
        .into_iter()
        .filter_map(|(s, a)| parse_chord(s).map(|c| (c, a)))
        .collect();
    let mut problems = Vec::new();
    for (chord_s, action_s) in user {
        let chord = match parse_chord(chord_s) {
            Some(c) => c,
            None => {
                problems.push(format!("keybindings: cannot parse key `{chord_s}`"));
                continue;
            }
        };
        map.retain(|(c, _)| *c != chord);
        if action_s.trim().eq_ignore_ascii_case("none") {
            continue;
        }
        match Action::from_name(action_s) {
            Some(a) => map.push((chord, a)),
            None => problems.push(format!("keybindings: unknown action `{action_s}`")),
        }
    }
    (map, problems)
}

/// The first binding for `action`, formatted for menus (`"ctrl+shift+t"`).
pub fn shortcut_for(map: &[(Chord, Action)], action: Action) -> Option<String> {
    map.iter().find(|(_, a)| *a == action).map(|(c, _)| chord_to_string(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_file_overrides_only_what_it_names() {
        let c = Config::parse("[font]\nsize = 18\n[cursor]\nstyle = \"bar\"\nblink = false\n").unwrap();
        assert_eq!(c.font.size, 18.0);
        assert_eq!(c.font.family, "monospace");
        assert_eq!(c.cursor.style().shape, CursorShape::Bar);
        assert!(!c.cursor.style().blink);
        assert!(c.effects.enabled);
    }

    #[test]
    fn errors_are_reported_not_fatal() {
        assert!(Config::parse("[font]\nsize = \"huge\"").is_err());
        assert!(Config::parse("this is not toml").is_err());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        assert!(Config::parse("[future]\nthing = 1\n[font]\nsize = 12\nfuture_key = true\n").is_ok());
    }

    #[test]
    fn values_are_clamped() {
        let c = Config::parse("[font]\nsize = 500\n[window]\nopacity = 0.0\n[scrollback]\nlines = 999999999\n").unwrap();
        assert_eq!(c.font.size, MAX_FONT);
        assert_eq!(c.window.opacity, 0.1);
        assert_eq!(c.scrollback.lines, 1_000_000);
    }

    #[test]
    fn roundtrip_through_toml() {
        let mut c = Config::default();
        c.font.size = 17.0;
        c.profiles.push(Profile { name: "work".into(), shell: Some("/bin/zsh".into()), args: vec!["-l".into()], ..Profile::default() });
        c.keybindings.insert("ctrl+shift+x".into(), "new_tab".into());
        let text = c.to_toml().unwrap();
        assert_eq!(Config::parse(&text).unwrap(), c);
    }

    #[test]
    fn profiles_resolve_by_name_with_fallback() {
        let c = Config::parse("[[profile]]\nname = \"default\"\nshell = \"/bin/dash\"\n[[profile]]\nname = \"fish\"\nshell = \"/usr/bin/fish\"\n").unwrap();
        assert_eq!(c.profile(None).shell.as_deref(), Some("/bin/dash"));
        assert_eq!(c.profile(Some("fish")).shell.as_deref(), Some("/usr/bin/fish"));
        assert_eq!(c.profile(Some("missing")).name, "default");
        assert_eq!(Config::default().profile(None).shell, None);
    }

    #[test]
    fn home_conf_parsing_matches_the_old_format() {
        let h = HomeConf::parse("# comment\ntheme_mode=light\naccent_color=#FF8800\nmatrix_rain=false\n");
        assert_eq!(h.theme_mode.as_deref(), Some("light"));
        assert_eq!(h.accent, Some([0xff, 0x88, 0x00]));
        assert_eq!(h.matrix_rain, Some(false));
    }

    #[test]
    fn theme_resolution_order() {
        let home = HomeConf::parse("theme_mode=light\naccent_color=#112233\n");
        let cfg = Config::default();
        let t = cfg.theme(&home);
        assert!(t.light, "home.conf light mode is followed");
        assert_eq!(t.accent, [0x11, 0x22, 0x33]);
        let pinned = Config::parse("[theme]\nname = \"solarized-dark\"\n").unwrap();
        assert_eq!(pinned.theme(&home).name, "solarized-dark");
        let hc = Config::parse("[accessibility]\nhigh_contrast = true\n").unwrap();
        assert!(hc.theme(&HomeConf::default()).name.starts_with("high-contrast"));
        let custom = Config::parse("[theme]\nbackground = \"#010203\"\n").unwrap();
        assert_eq!(custom.theme(&HomeConf::default()).bg, [1, 2, 3]);
    }

    #[test]
    fn rain_setting_precedence() {
        let cfg = Config::default();
        assert!(cfg.rain_enabled(&HomeConf::default()));
        assert!(!cfg.rain_enabled(&HomeConf::parse("matrix_rain=false")));
        let forced = Config::parse("[effects]\nrain = true\n").unwrap();
        assert!(forced.rain_enabled(&HomeConf::parse("matrix_rain=false")));
        let off = Config::parse("[effects]\nenabled = false\n").unwrap();
        assert!(!off.rain_enabled(&HomeConf::default()));
    }

    #[test]
    fn chord_parsing_and_formatting() {
        let c = parse_chord("Ctrl+Shift+T").unwrap();
        assert_eq!((c.ctrl, c.shift, c.alt, c.key), (true, true, false, Key::T));
        assert_eq!(chord_to_string(&c), "ctrl+shift+t");
        assert_eq!(parse_chord("alt+1").unwrap().key, Key::Num1);
        assert!(parse_chord("ctrl+nonsense").is_none());
        assert!(parse_chord("ctrl+").is_some());
    }

    #[test]
    fn plus_and_equals_are_the_same_chord() {
        let c = parse_chord("ctrl+equals").unwrap();
        assert!(c.matches(Key::Equals, true, false, false));
        assert!(c.matches(Key::Equals, true, true, false), "Ctrl+Shift+= is Ctrl++");
        assert!(!c.matches(Key::Minus, true, false, false));
        let t = parse_chord("ctrl+shift+t").unwrap();
        assert!(!t.matches(Key::T, true, false, false), "modifiers must match exactly");
    }

    #[test]
    fn every_action_has_a_unique_name_that_round_trips() {
        let mut seen = std::collections::HashSet::new();
        for (a, name, _) in ACTIONS {
            assert!(seen.insert(*name), "duplicate {name}");
            assert_eq!(Action::from_name(name), Some(*a));
            assert_eq!(a.name(), *name);
        }
    }

    #[test]
    fn default_bindings_all_parse_and_do_not_collide() {
        let mut seen = std::collections::HashMap::new();
        for (s, a) in default_bindings() {
            let c = parse_chord(s).unwrap_or_else(|| panic!("unparsable default binding {s}"));
            if let Some(prev) = seen.insert(chord_to_string(&c), a) {
                assert_eq!(prev, a, "{s} bound twice to different actions");
            }
        }
    }

    #[test]
    fn user_bindings_override_and_unbind() {
        let mut user = BTreeMap::new();
        user.insert("ctrl+shift+t".to_string(), "new_window".to_string());
        user.insert("ctrl+f".to_string(), "none".to_string());
        user.insert("ctrl+shift+q".to_string(), "bogus".to_string());
        user.insert("ctrl+shift+nope".to_string(), "new_tab".to_string());
        let (map, problems) = build_keymap(&user);
        let find = |s: &str| {
            let c = parse_chord(s).unwrap();
            map.iter().find(|(k, _)| *k == c).map(|(_, a)| *a)
        };
        assert_eq!(find("ctrl+shift+t"), Some(Action::NewWindow));
        assert_eq!(find("ctrl+f"), None);
        assert_eq!(find("ctrl+shift+f"), Some(Action::Find));
        assert_eq!(problems.len(), 2);
    }
}
