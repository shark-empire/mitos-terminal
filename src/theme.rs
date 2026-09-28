//! The Theme Engine.
//!
//! This module is the one place in the crate that decides what MITOS
//! Terminal *looks* like. It deliberately knows nothing about PTYs, shells,
//! panes or the VT parser (see `term/`, `pty.rs`, `layout.rs` for that) —
//! swapping a [`Theme`] never touches terminal behaviour, only how it's
//! painted. `render.rs`, `fx.rs` and `app.rs` are the only other modules
//! that read a `Theme`; none of them hard-code a palette or an effect.
//!
//! A theme is more than a palette: alongside the terminal-content colours
//! (`fg`/`bg`/`ansi`/...) it carries the "personality" knobs described in the
//! design brief — glow intensity, simulated glass blur, corner radius,
//! scanlines, a grid overlay, vignette, HUD corner accents, and the tab/status
//! chrome style — as *suggested defaults*. Switching presets seeds those into
//! the user's live, independently-adjustable settings (`config::AppearanceCfg`)
//! rather than owning them outright, so "I love Cyberpunk but want less glow"
//! is just moving a slider, never a reason to fork a theme.
//!
//! Every theme is designed to the brief's restraint requirement first:
//! comfortable for hours of real use, not a poster. Nothing here blinks,
//! saturates, or fights the text for attention.

use crate::term::{Color, Overrides};

pub type Rgb = [u8; 3];

/// Visual style of the tab strip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabStyle {
    /// Soft rounded pill per tab.
    Rounded,
    /// Flat, with a coloured underline on the active tab.
    Underline,
    /// Square-cornered, bordered box per tab (traditional).
    Boxed,
}

impl TabStyle {
    pub fn parse(s: &str) -> TabStyle {
        match s.trim().to_ascii_lowercase().as_str() {
            "underline" => TabStyle::Underline,
            "boxed" | "box" => TabStyle::Boxed,
            _ => TabStyle::Rounded,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TabStyle::Rounded => "rounded",
            TabStyle::Underline => "underline",
            TabStyle::Boxed => "boxed",
        }
    }
}

/// Visual style of the per-pane status/breadcrumb strip (user@host, cwd).
/// This is terminal-drawn chrome, independent of the shell's own `$PS1` —
/// it never touches what the shell prints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusStyle {
    Hidden,
    /// A single small line, low visual weight.
    Minimal,
    /// `[ user@host ]-[ ~/path ]` breadcrumb, closest to a classic prompt.
    Breadcrumb,
    /// Coloured pill segments (powerline-adjacent) for user/host/path.
    Segmented,
}

impl StatusStyle {
    pub fn parse(s: &str) -> StatusStyle {
        match s.trim().to_ascii_lowercase().as_str() {
            "hidden" | "none" | "off" => StatusStyle::Hidden,
            "breadcrumb" => StatusStyle::Breadcrumb,
            "segmented" | "powerline" => StatusStyle::Segmented,
            _ => StatusStyle::Minimal,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatusStyle::Hidden => "hidden",
            StatusStyle::Minimal => "minimal",
            StatusStyle::Breadcrumb => "breadcrumb",
            StatusStyle::Segmented => "segmented",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// Stable slug used in `terminal.toml` and `Theme::by_name`. Never shown to the user.
    pub id: String,
    /// Human-readable name shown in the theme picker.
    pub label: String,
    /// One-line description shown under the preset in the picker.
    pub tagline: String,
    pub light: bool,

    // ----- terminal content palette ---------------------------------
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection: Rgb,
    pub prompt: Rgb,
    pub accent: Rgb,
    pub ansi: [Rgb; 16],

    // ----- chrome / surface palette (tabs, status bar, dialogs) -----
    /// Primary chrome surface (tab bar, dialogs).
    pub surface: Rgb,
    /// Secondary surface: hover states, the status/breadcrumb strip.
    pub surface_alt: Rgb,
    /// Hairline border colour (panes, dialogs, dividers).
    pub border: Rgb,
    /// Colour of the soft outer glow on focused panes/borders. Usually the
    /// accent, occasionally a deliberately different hue (see Cyberpunk).
    pub glow: Rgb,

    // ----- suggested effect defaults (seed config; see module docs) -
    /// 0 = fully opaque .. 1 = as transparent as the theme intends.
    pub opacity: f32,
    /// 0 = flat solid surfaces .. 1 = full frosted-glass treatment.
    pub blur: f32,
    /// 0 = no glow .. 1 = the theme's strongest tasteful glow.
    pub glow_intensity: f32,
    pub corner_radius: f32,
    pub border_width: f32,
    /// CRT-style horizontal scanlines over the text area.
    pub scanlines: bool,
    /// Faint technical grid over the background.
    pub grid_overlay: bool,
    /// 0 = none .. 1 = strong darkened-corner falloff (CRT curvature).
    pub vignette: f32,
    /// Small corner tick-mark accents on focused panes.
    pub hud_accents: bool,
    pub tab_style: TabStyle,
    pub status_style: StatusStyle,
}

/// `(slug, label)` for every built-in preset, in the order they should be
/// offered in the picker. Accessibility variants are listed last, separately
/// labelled, rather than mixed into the main aesthetic choices.
pub const THEME_PRESETS: &[(&str, &str)] = &[
    ("futuristic-scifi", "Futuristic Sci-Fi"),
    ("cyberpunk", "Cyberpunk"),
    ("glass-neon", "Glass Neon"),
    ("minimal-dark", "Minimal Dark"),
    ("light", "Light"),
    ("classic", "Classic Terminal"),
    ("retro-crt", "Retro CRT"),
    ("high-contrast-dark", "High Contrast (Dark)"),
    ("high-contrast-light", "High Contrast (Light)"),
    ("custom", "Custom"),
];

impl Theme {
    pub fn names() -> &'static [(&'static str, &'static str)] {
        THEME_PRESETS
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        match name.trim().to_ascii_lowercase().as_str() {
            "futuristic-scifi" | "futuristic" | "scifi" | "mitos-dark" | "dark" => Some(Self::futuristic_scifi()),
            "cyberpunk" => Some(Self::cyberpunk()),
            "glass-neon" | "glass" | "neon" => Some(Self::glass_neon()),
            "minimal-dark" | "minimal" => Some(Self::minimal_dark()),
            "light" | "mitos-light" => Some(Self::light()),
            "classic" | "classic-terminal" => Some(Self::classic()),
            "retro-crt" | "retro" | "crt" => Some(Self::retro_crt()),
            "high-contrast-dark" | "high-contrast" => Some(Self::high_contrast_dark()),
            "high-contrast-light" => Some(Self::high_contrast_light()),
            // "Custom" has no fixed palette of its own: it resolves to a
            // quiet, neutral base that `Config::theme` then applies the
            // user's saved custom overrides on top of (the same generic
            // override mechanism every other preset supports too).
            "custom" => Some(Self::custom_base()),
            _ => None,
        }
    }

    // ----- presets --------------------------------------------------

    /// Alias for [`Self::futuristic_scifi`] — the MITOS default. Kept as a
    /// short, memorable name for callers that just want "the" dark theme
    /// (e.g. a safe fallback when nothing else is known yet) without caring
    /// which preset that resolves to.
    pub fn dark() -> Theme {
        Self::futuristic_scifi()
    }

    /// The MITOS default: dark blue-black, restrained cyan glow, soft glass.
    pub fn futuristic_scifi() -> Theme {
        Theme {
            id: "futuristic-scifi".into(),
            label: "Futuristic Sci-Fi".into(),
            tagline: "Calm, premium, and quietly technical — the MITOS default.".into(),
            light: false,
            fg: [196, 210, 226],
            bg: [5, 9, 17],
            cursor: [95, 212, 255],
            cursor_text: [5, 9, 17],
            selection: [22, 58, 84],
            prompt: [95, 212, 255],
            accent: [70, 184, 232],
            ansi: [
                [18, 24, 34],
                [230, 90, 90],
                [110, 220, 160],
                [230, 190, 110],
                [80, 140, 235],
                [170, 130, 235],
                [70, 200, 235],
                [196, 210, 226],
                [92, 108, 130],
                [255, 130, 130],
                [150, 255, 195],
                [255, 220, 150],
                [120, 175, 255],
                [205, 170, 255],
                [120, 230, 255],
                [235, 242, 250],
            ],
            surface: [10, 16, 28],
            surface_alt: [15, 23, 38],
            border: [45, 110, 150],
            glow: [95, 212, 255],
            opacity: 0.94,
            blur: 0.35,
            glow_intensity: 0.35,
            corner_radius: 10.0,
            border_width: 1.0,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.08,
            hud_accents: true,
            tab_style: TabStyle::Rounded,
            status_style: StatusStyle::Breadcrumb,
        }
    }

    /// Dark charcoal, controlled magenta + cyan, subtle grid — sophisticated, not chaotic.
    pub fn cyberpunk() -> Theme {
        Theme {
            id: "cyberpunk".into(),
            label: "Cyberpunk".into(),
            tagline: "Controlled magenta and cyan on charcoal. Bold without the noise.".into(),
            light: false,
            fg: [214, 214, 226],
            bg: [13, 13, 18],
            cursor: [45, 212, 207],
            cursor_text: [13, 13, 18],
            selection: [58, 26, 58],
            prompt: [217, 70, 196],
            accent: [217, 70, 196],
            ansi: [
                [20, 18, 26],
                [255, 70, 110],
                [80, 230, 150],
                [235, 200, 80],
                [90, 140, 255],
                [230, 80, 220],
                [60, 225, 220],
                [210, 210, 224],
                [100, 96, 118],
                [255, 120, 150],
                [140, 255, 190],
                [255, 225, 130],
                [140, 175, 255],
                [255, 130, 245],
                [130, 245, 240],
                [240, 240, 250],
            ],
            surface: [19, 19, 26],
            surface_alt: [26, 22, 34],
            border: [180, 60, 170],
            glow: [45, 212, 207],
            opacity: 0.97,
            blur: 0.15,
            glow_intensity: 0.5,
            corner_radius: 6.0,
            border_width: 1.25,
            scanlines: false,
            grid_overlay: true,
            vignette: 0.15,
            hud_accents: true,
            tab_style: TabStyle::Underline,
            status_style: StatusStyle::Segmented,
        }
    }

    /// Frosted glass over a subdued ambient backdrop. The most transparent preset.
    pub fn glass_neon() -> Theme {
        Theme {
            id: "glass-neon".into(),
            label: "Glass Neon".into(),
            tagline: "Frosted glass with low-intensity neon. Modern and premium.".into(),
            light: false,
            fg: [228, 233, 245],
            bg: [16, 20, 42],
            cursor: [135, 150, 255],
            cursor_text: [16, 20, 42],
            selection: [54, 58, 110],
            prompt: [120, 180, 255],
            accent: [138, 124, 255],
            ansi: [
                [26, 28, 48],
                [255, 110, 140],
                [120, 230, 190],
                [240, 205, 140],
                [125, 150, 255],
                [190, 140, 255],
                [110, 215, 255],
                [220, 224, 240],
                [104, 112, 150],
                [255, 150, 175],
                [160, 255, 215],
                [255, 225, 170],
                [165, 190, 255],
                [215, 175, 255],
                [160, 235, 255],
                [245, 247, 255],
            ],
            surface: [20, 24, 50],
            surface_alt: [26, 30, 60],
            border: [110, 120, 220],
            glow: [95, 201, 255],
            opacity: 0.82,
            blur: 0.75,
            glow_intensity: 0.3,
            corner_radius: 14.0,
            border_width: 1.0,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.05,
            hud_accents: false,
            tab_style: TabStyle::Rounded,
            status_style: StatusStyle::Minimal,
        }
    }

    /// Clean, flat, no effects. Everyday use.
    pub fn minimal_dark() -> Theme {
        Theme {
            id: "minimal-dark".into(),
            label: "Minimal Dark".into(),
            tagline: "Clean and simple, with nothing competing for attention.".into(),
            light: false,
            fg: [212, 212, 216],
            bg: [20, 22, 27],
            cursor: [140, 170, 204],
            cursor_text: [20, 22, 27],
            selection: [45, 55, 68],
            prompt: [140, 170, 204],
            accent: [110, 140, 174],
            ansi: [
                [30, 32, 38],
                [205, 100, 100],
                [130, 180, 130],
                [200, 175, 110],
                [110, 150, 200],
                [170, 130, 180],
                [110, 175, 180],
                [200, 200, 205],
                [104, 108, 118],
                [230, 140, 140],
                [165, 210, 165],
                [225, 200, 140],
                [145, 185, 230],
                [200, 160, 210],
                [145, 205, 210],
                [230, 230, 234],
            ],
            surface: [27, 29, 35],
            surface_alt: [33, 35, 42],
            border: [60, 66, 78],
            glow: [110, 140, 174],
            opacity: 1.0,
            blur: 0.0,
            glow_intensity: 0.0,
            corner_radius: 6.0,
            border_width: 1.0,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.0,
            hud_accents: false,
            tab_style: TabStyle::Underline,
            status_style: StatusStyle::Minimal,
        }
    }

    /// Fresh and calm; the daytime counterpart to the dark presets.
    pub fn light() -> Theme {
        Theme {
            id: "light".into(),
            label: "Light".into(),
            tagline: "Soft colours on a bright background. Built for daytime use.".into(),
            light: true,
            fg: [35, 35, 38],
            bg: [250, 250, 250],
            cursor: [30, 100, 220],
            cursor_text: [250, 250, 250],
            selection: [190, 215, 250],
            prompt: [30, 100, 220],
            accent: [47, 111, 224],
            ansi: [
                [45, 45, 48],
                [190, 35, 35],
                [25, 130, 55],
                [160, 115, 0],
                [35, 90, 205],
                [150, 45, 165],
                [0, 135, 145],
                [90, 90, 96],
                [110, 110, 115],
                [220, 65, 65],
                [40, 160, 80],
                [180, 132, 0],
                [60, 115, 230],
                [180, 70, 195],
                [0, 150, 160],
                [140, 140, 146],
            ],
            surface: [240, 241, 243],
            surface_alt: [232, 233, 236],
            border: [205, 208, 214],
            glow: [47, 111, 224],
            opacity: 1.0,
            blur: 0.0,
            glow_intensity: 0.0,
            corner_radius: 8.0,
            border_width: 1.0,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.0,
            hud_accents: false,
            tab_style: TabStyle::Rounded,
            status_style: StatusStyle::Minimal,
        }
    }

    /// Plain black-on-grey, standard 16-colour palette, zero decoration.
    /// The option for someone who just wants a normal terminal.
    pub fn classic() -> Theme {
        Theme {
            id: "classic".into(),
            label: "Classic Terminal".into(),
            tagline: "Plain black background, standard colours. No effects at all.".into(),
            light: false,
            fg: [229, 229, 229],
            bg: [0, 0, 0],
            cursor: [78, 154, 6],
            cursor_text: [0, 0, 0],
            selection: [60, 80, 60],
            prompt: [78, 154, 6],
            accent: [78, 154, 6],
            ansi: [
                [0, 0, 0],
                [205, 0, 0],
                [0, 205, 0],
                [205, 205, 0],
                [64, 110, 240],
                [205, 0, 205],
                [0, 205, 205],
                [229, 229, 229],
                [118, 118, 118],
                [255, 0, 0],
                [0, 255, 0],
                [255, 255, 0],
                [92, 92, 255],
                [255, 0, 255],
                [0, 255, 255],
                [255, 255, 255],
            ],
            surface: [10, 10, 10],
            surface_alt: [18, 18, 18],
            border: [60, 60, 60],
            glow: [78, 154, 6],
            opacity: 1.0,
            blur: 0.0,
            glow_intensity: 0.0,
            corner_radius: 3.0,
            border_width: 1.0,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.0,
            hud_accents: false,
            tab_style: TabStyle::Boxed,
            status_style: StatusStyle::Minimal,
        }
    }

    /// Green phosphor, scanlines, soft screen curvature. Nostalgic, not harsh.
    pub fn retro_crt() -> Theme {
        Theme {
            id: "retro-crt".into(),
            label: "Retro CRT".into(),
            tagline: "Green phosphor and soft scanlines. Old-school, easy on the eyes.".into(),
            light: false,
            fg: [57, 211, 83],
            bg: [3, 10, 3],
            cursor: [130, 255, 150],
            cursor_text: [3, 10, 3],
            selection: [20, 60, 25],
            prompt: [130, 255, 150],
            accent: [57, 211, 83],
            ansi: [
                [3, 12, 4],
                [150, 90, 70],
                [57, 211, 83],
                [170, 195, 80],
                [80, 160, 140],
                [150, 130, 150],
                [70, 200, 160],
                [190, 225, 190],
                [66, 112, 74],
                [190, 120, 95],
                [110, 255, 140],
                [200, 225, 110],
                [110, 200, 175],
                [190, 165, 190],
                [110, 235, 195],
                [220, 245, 220],
            ],
            surface: [5, 15, 6],
            surface_alt: [8, 20, 9],
            border: [40, 110, 50],
            glow: [130, 255, 150],
            opacity: 1.0,
            blur: 0.0,
            glow_intensity: 0.45,
            corner_radius: 4.0,
            border_width: 1.0,
            scanlines: true,
            grid_overlay: false,
            vignette: 0.35,
            hud_accents: false,
            tab_style: TabStyle::Boxed,
            status_style: StatusStyle::Minimal,
        }
    }

    pub fn high_contrast_dark() -> Theme {
        Theme {
            id: "high-contrast-dark".into(),
            label: "High Contrast (Dark)".into(),
            tagline: "Maximum legibility on black. No decorative effects.".into(),
            light: false,
            fg: [255, 255, 255],
            bg: [0, 0, 0],
            cursor: [255, 255, 0],
            cursor_text: [0, 0, 0],
            selection: [0, 70, 200],
            prompt: [0, 255, 0],
            accent: [255, 255, 0],
            ansi: [
                [0, 0, 0],
                [255, 90, 90],
                [0, 255, 0],
                [255, 255, 0],
                [110, 150, 255],
                [255, 110, 255],
                [0, 255, 255],
                [230, 230, 230],
                [150, 150, 150],
                [255, 130, 130],
                [110, 255, 110],
                [255, 255, 120],
                [150, 185, 255],
                [255, 160, 255],
                [110, 255, 255],
                [255, 255, 255],
            ],
            surface: [0, 0, 0],
            surface_alt: [20, 20, 20],
            border: [255, 255, 255],
            glow: [255, 255, 0],
            opacity: 1.0,
            blur: 0.0,
            glow_intensity: 0.0,
            corner_radius: 2.0,
            border_width: 1.5,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.0,
            hud_accents: false,
            tab_style: TabStyle::Underline,
            status_style: StatusStyle::Minimal,
        }
    }

    pub fn high_contrast_light() -> Theme {
        Theme {
            id: "high-contrast-light".into(),
            label: "High Contrast (Light)".into(),
            tagline: "Maximum legibility on white. No decorative effects.".into(),
            light: true,
            fg: [0, 0, 0],
            bg: [255, 255, 255],
            cursor: [0, 0, 200],
            cursor_text: [255, 255, 255],
            selection: [140, 190, 255],
            prompt: [0, 90, 0],
            accent: [0, 0, 200],
            ansi: [
                [0, 0, 0],
                [160, 0, 0],
                [0, 100, 0],
                [110, 80, 0],
                [0, 0, 190],
                [130, 0, 130],
                [0, 100, 110],
                [90, 90, 90],
                [60, 60, 60],
                [190, 0, 0],
                [0, 120, 0],
                [130, 100, 0],
                [0, 0, 230],
                [160, 0, 160],
                [0, 120, 130],
                [0, 0, 0],
            ],
            surface: [255, 255, 255],
            surface_alt: [235, 235, 235],
            border: [0, 0, 0],
            glow: [0, 0, 200],
            opacity: 1.0,
            blur: 0.0,
            glow_intensity: 0.0,
            corner_radius: 2.0,
            border_width: 1.5,
            scanlines: false,
            grid_overlay: false,
            vignette: 0.0,
            hud_accents: false,
            tab_style: TabStyle::Underline,
            status_style: StatusStyle::Minimal,
        }
    }

    /// Neutral starting point for a from-scratch Custom theme (quiet slate,
    /// every field meant to be overridden — see `Config::theme`).
    pub fn custom_base() -> Theme {
        let mut t = Self::minimal_dark();
        t.id = "custom".into();
        t.label = "Custom".into();
        t.tagline = "Your own colours and effects, built from a blank slate.".into();
        t
    }

    // ----- derived variants ------------------------------------------

    /// This theme, re-derived for a light background — same accent/hue
    /// character, recomputed luminance relationships, contrast re-verified.
    /// A no-op if the theme is already light. Used by
    /// `[accessibility] color_scheme` so every preset works in both
    /// readings without hand-authoring a second palette for each.
    pub fn as_light_variant(&self) -> Theme {
        if self.light {
            return self.clone();
        }
        let mut t = self.clone();
        t.light = true;
        t.id = format!("{}-light", self.id);
        t.label = format!("{} (Light)", self.label);
        t.bg = mix(self.bg, [255, 255, 255], 0.94);
        t.surface = mix(t.bg, [0, 0, 0], 0.04);
        t.surface_alt = mix(t.bg, [0, 0, 0], 0.08);
        t.fg = ensure_contrast(mix(self.fg, [0, 0, 0], 0.75), t.bg, 7.0);
        t.border = ensure_contrast(mix(self.border, [0, 0, 0], 0.3), t.bg, 1.6);
        t.selection = mix(self.selection, [255, 255, 255], 0.55);
        t.cursor_text = t.bg;
        t.cursor = ensure_contrast(mix(self.cursor, [0, 0, 0], 0.2), t.bg, 3.0);
        t.prompt = ensure_contrast(mix(self.prompt, [0, 0, 0], 0.2), t.bg, 3.0);
        t.accent = ensure_contrast(mix(self.accent, [0, 0, 0], 0.15), t.bg, 2.5);
        t.glow = t.accent;
        for c in t.ansi.iter_mut() {
            *c = ensure_contrast(mix(*c, [0, 0, 0], 0.25), t.bg, 3.0);
        }
        t
    }

    /// Symmetric counterpart of [`Self::as_light_variant`]: a no-op if
    /// already dark, otherwise a re-derived dark reading of a light theme.
    pub fn as_dark_variant(&self) -> Theme {
        if !self.light {
            return self.clone();
        }
        let mut t = self.clone();
        t.light = false;
        t.id = format!("{}-dark", self.id);
        t.label = format!("{} (Dark)", self.label);
        t.bg = mix(self.bg, [0, 0, 0], 0.94);
        t.surface = mix(t.bg, [255, 255, 255], 0.04);
        t.surface_alt = mix(t.bg, [255, 255, 255], 0.08);
        t.fg = ensure_contrast(mix(self.fg, [255, 255, 255], 0.75), t.bg, 7.0);
        t.border = ensure_contrast(mix(self.border, [255, 255, 255], 0.3), t.bg, 1.6);
        t.selection = mix(self.selection, [0, 0, 0], 0.55);
        t.cursor_text = t.bg;
        t.cursor = ensure_contrast(mix(self.cursor, [255, 255, 255], 0.2), t.bg, 3.0);
        t.prompt = ensure_contrast(mix(self.prompt, [255, 255, 255], 0.2), t.bg, 3.0);
        t.accent = ensure_contrast(mix(self.accent, [255, 255, 255], 0.15), t.bg, 2.5);
        t.glow = t.accent;
        for c in t.ansi.iter_mut() {
            *c = ensure_contrast(mix(*c, [255, 255, 255], 0.25), t.bg, 3.0);
        }
        t
    }

    /// Re-tint every accent-derived colour (prompt, cursor, glow — not the
    /// base palette) to a user-chosen accent.
    pub fn with_accent(mut self, accent: Rgb) -> Theme {
        self.accent = accent;
        self.prompt = accent;
        self.cursor = accent;
        self.glow = accent;
        self
    }

    /// Palette entry without any OSC 4 overrides: 0-15 theme, 16-231 cube, 232-255 grey ramp.
    pub fn indexed(&self, i: u8) -> Rgb {
        if (i as usize) < 16 {
            self.ansi[i as usize]
        } else {
            xterm_extended(i)
        }
    }

    /// Resolve a cell colour, honouring OSC 4/10/11 overrides set by the application.
    pub fn resolve(&self, ov: &Overrides, c: Color, is_fg: bool) -> Rgb {
        match c {
            Color::Default => {
                if is_fg {
                    ov.fg.unwrap_or(self.fg)
                } else {
                    ov.bg.unwrap_or(self.bg)
                }
            }
            Color::Rgb(r, g, b) => [r, g, b],
            Color::Indexed(i) => ov
                .palette
                .get(i as usize)
                .copied()
                .flatten()
                .unwrap_or_else(|| self.indexed(i)),
        }
    }
}

/// xterm's fixed 6x6x6 colour cube (16..=231) and 24-step grey ramp (232..=255).
pub fn xterm_extended(i: u8) -> Rgb {
    let i = i as usize;
    if i < 16 {
        return [0, 0, 0];
    }
    if i < 232 {
        let n = (i - 16) as u8;
        let comp = |v: u8| -> u8 {
            if v == 0 {
                0
            } else {
                55 + 40 * v
            }
        };
        [comp(n / 36), comp((n / 6) % 6), comp(n % 6)]
    } else {
        let g = 8 + 10 * (i as u8 - 232);
        [g, g, g]
    }
}

// ---------------------------------------------------------------------------
// Colour maths
// ---------------------------------------------------------------------------

fn lin(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.03928 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance.
pub fn luminance(c: Rgb) -> f32 {
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio, 1.0 (none) ..= 21.0 (black on white).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// Nudge `fg` towards white/black until it meets `min` contrast against `bg`.
pub fn ensure_contrast(fg: Rgb, bg: Rgb, min: f32) -> Rgb {
    if contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let target: Rgb = if luminance(bg) > 0.5 { [0, 0, 0] } else { [255, 255, 255] };
    for step in 1..=20 {
        let c = mix(fg, target, step as f32 / 20.0);
        if contrast_ratio(c, bg) >= min {
            return c;
        }
    }
    target
}

/// Parses `#rrggbb`, `rrggbb` or `#rgb`.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    let hex = |t: &str| u8::from_str_radix(t, 16).ok();
    match s.len() {
        6 => Some([hex(&s[0..2])?, hex(&s[2..4])?, hex(&s[4..6])?]),
        3 => {
            let d = |i: usize| -> Option<u8> { hex(&s[i..i + 1]).map(|v| v * 17) };
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

pub fn to_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_presets() -> Vec<Theme> {
        THEME_PRESETS.iter().map(|(id, _)| Theme::by_name(id).unwrap()).collect()
    }

    #[test]
    fn cube_and_grey_ramp() {
        let t = Theme::futuristic_scifi();
        assert_eq!(t.indexed(16), [0, 0, 0]);
        assert_eq!(t.indexed(231), [255, 255, 255]);
        assert_eq!(t.indexed(196), [255, 0, 0]);
        assert_eq!(t.indexed(232), [8, 8, 8]);
        assert_eq!(t.indexed(255), [238, 238, 238]);
    }

    #[test]
    fn contrast_bounds() {
        assert!((contrast_ratio([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 0.01);
        assert!((contrast_ratio([9, 9, 9], [9, 9, 9]) - 1.0).abs() < 0.001);
    }

    #[test]
    fn ensure_contrast_improves_low_contrast() {
        let bg = [20, 20, 25];
        let fg = [40, 40, 50];
        let out = ensure_contrast(fg, bg, 4.5);
        assert!(contrast_ratio(out, bg) >= 4.5);
        assert_eq!(ensure_contrast([255, 255, 255], bg, 4.5), [255, 255, 255]);
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("#55FF55"), Some([0x55, 0xff, 0x55]));
        assert_eq!(parse_hex("f0a"), Some([0xff, 0x00, 0xaa]));
        assert_eq!(parse_hex("zzz"), None);
        assert_eq!(to_hex([1, 2, 255]), "#0102ff");
    }

    #[test]
    fn every_builtin_resolves() {
        for (id, _label) in Theme::names() {
            assert!(Theme::by_name(id).is_some(), "{id}");
        }
    }

    #[test]
    fn preset_ids_and_light_flag_are_internally_consistent() {
        for (id, _) in THEME_PRESETS {
            if *id == "custom" {
                continue;
            }
            let t = Theme::by_name(id).unwrap();
            assert_eq!(&t.id, id, "constructor id must match its THEME_PRESETS slug");
            let expect_light = *id == "light" || *id == "high-contrast-light";
            assert_eq!(t.light, expect_light, "{id}");
        }
    }

    /// Every non-decorative preset must clear WCAG AA (4.5:1) for normal
    /// text out of the box — the whole point of a "restrained" design brief
    /// is that the *default* reading is comfortable, not just the
    /// accessibility variants.
    #[test]
    fn every_preset_meets_aa_contrast_for_body_text() {
        for t in all_presets() {
            let ratio = contrast_ratio(t.fg, t.bg);
            assert!(ratio >= 4.5, "{} fg/bg contrast only {:.2}:1", t.label, ratio);
        }
    }

    /// "Every theme must prioritize readability": no ANSI colour except
    /// black (index 0 — used mostly as a *background*) may sink into the
    /// theme's own background. This is what catches things like "bright
    /// white on a light theme" or an unreadably dark "bright black".
    #[test]
    fn every_ansi_colour_stays_legible_against_its_background() {
        let mut themes = all_presets();
        let variants: Vec<Theme> = themes.iter().map(|t| if t.light { t.as_dark_variant() } else { t.as_light_variant() }).collect();
        themes.extend(variants);
        for t in themes {
            for i in 1..16 {
                let r = contrast_ratio(t.ansi[i], t.bg);
                assert!(r >= 3.0, "{}: ANSI {} {:?} on bg {:?} is only {:.2}:1", t.label, i, t.ansi[i], t.bg, r);
            }
        }
    }

    #[test]
    fn effect_parameters_stay_in_their_documented_ranges() {
        for t in all_presets() {
            for (name, v) in [
                ("opacity", t.opacity),
                ("blur", t.blur),
                ("glow_intensity", t.glow_intensity),
                ("vignette", t.vignette),
            ] {
                assert!((0.0..=1.0).contains(&v), "{}.{name} = {v} out of 0..=1", t.label);
            }
            assert!(t.corner_radius >= 0.0 && t.border_width >= 0.0, "{}", t.label);
        }
    }

    #[test]
    fn brief_mandated_restraint_is_actually_restrained() {
        // "Must NOT be overly bright... avoid excessive neon / blinding glow
        // / thick glowing borders." None of the atmospheric knobs should
        // ever reach full intensity on a shipped preset.
        for t in all_presets() {
            assert!(t.glow_intensity <= 0.6, "{}: glow_intensity {} too strong", t.label, t.glow_intensity);
            assert!(t.border_width <= 2.0, "{}: border_width {} too thick", t.label, t.border_width);
            assert!(t.vignette <= 0.4, "{}: vignette {} too strong", t.label, t.vignette);
        }
        // Glass Neon specifically: "wallpaper remains visible but heavily
        // subdued... text remains extremely readable" — opacity must drop
        // (glass, not a flat panel) but never so far that legibility suffers.
        let glass = Theme::glass_neon();
        assert!(glass.opacity < 0.9 && glass.opacity > 0.6);
    }

    #[test]
    fn light_and_dark_variants_round_trip_and_preserve_hue_identity() {
        for t in all_presets() {
            let flipped = if t.light { t.as_dark_variant() } else { t.as_light_variant() };
            assert_ne!(flipped.light, t.light);
            assert!(contrast_ratio(flipped.fg, flipped.bg) >= 4.5, "{}", flipped.label);
            // flipping preserves *some* relationship to the original accent
            // hue rather than collapsing to grey (a crude but effective
            // check: the derived accent must still differ from pure fg/bg).
            assert_ne!(flipped.accent, flipped.fg);
            assert_ne!(flipped.accent, flipped.bg);
        }
        // idempotent: flipping an already-light theme light-ward is a no-op
        let light = Theme::light();
        assert_eq!(light.as_light_variant().bg, light.bg);
        let dark = Theme::futuristic_scifi();
        assert_eq!(dark.as_dark_variant().bg, dark.bg);
    }

    #[test]
    fn with_accent_retints_every_accent_derived_field_consistently() {
        let t = Theme::minimal_dark().with_accent([255, 0, 128]);
        assert_eq!(t.accent, [255, 0, 128]);
        assert_eq!(t.cursor, [255, 0, 128]);
        assert_eq!(t.prompt, [255, 0, 128]);
        assert_eq!(t.glow, [255, 0, 128]);
        // the base palette (fg/bg/ansi) is untouched by an accent change
        assert_eq!(t.fg, Theme::minimal_dark().fg);
    }

    #[test]
    fn custom_base_is_a_neutral_quiet_starting_point() {
        let c = Theme::custom_base();
        assert_eq!(c.id, "custom");
        assert_eq!(c.glow_intensity, 0.0);
        assert_eq!(c.blur, 0.0);
    }

    #[test]
    fn tab_and_status_style_parse_round_trip() {
        for s in [TabStyle::Rounded, TabStyle::Underline, TabStyle::Boxed] {
            assert_eq!(TabStyle::parse(s.as_str()), s);
        }
        for s in [StatusStyle::Hidden, StatusStyle::Minimal, StatusStyle::Breadcrumb, StatusStyle::Segmented] {
            assert_eq!(StatusStyle::parse(s.as_str()), s);
        }
        assert_eq!(TabStyle::parse("bogus"), TabStyle::Rounded);
        assert_eq!(StatusStyle::parse("bogus"), StatusStyle::Minimal);
    }
}
