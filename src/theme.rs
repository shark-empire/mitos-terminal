//! Themes, palette resolution and contrast helpers.
//!
//! Cells store *palette references* (`Color::Default` / `Color::Indexed`), not
//! baked RGB, so switching theme repaints the whole scrollback instantly —
//! this is what lets `ThemeChanged` recolour text that is already on screen.

use crate::term::{Color, Overrides};

pub type Rgb = [u8; 3];

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection: Rgb,
    pub prompt: Rgb,
    pub accent: Rgb,
    pub ansi: [Rgb; 16],
    pub light: bool,
}

pub const THEME_NAMES: &[&str] = &[
    "mitos-dark",
    "mitos-light",
    "high-contrast-dark",
    "high-contrast-light",
    "solarized-dark",
];

impl Theme {
    pub fn names() -> &'static [&'static str] {
        THEME_NAMES
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mitos-dark" | "dark" => Some(Self::dark()),
            "mitos-light" | "light" => Some(Self::light()),
            "high-contrast-dark" | "high-contrast" => Some(Self::high_contrast_dark()),
            "high-contrast-light" => Some(Self::high_contrast_light()),
            "solarized-dark" | "solarized" => Some(Self::solarized_dark()),
            _ => None,
        }
    }

    /// The default MITOS look: deep navy, phosphor green accent.
    pub fn dark() -> Theme {
        Theme {
            name: "mitos-dark".into(),
            fg: [200, 200, 200],
            bg: [4, 10, 18],
            cursor: [85, 255, 85],
            cursor_text: [4, 10, 18],
            selection: [0, 90, 170],
            prompt: [85, 255, 85],
            accent: [0x33, 0xFF, 0x66],
            ansi: [
                [24, 26, 34],
                [255, 85, 85],
                [85, 255, 85],
                [255, 255, 85],
                [85, 85, 255],
                [255, 85, 255],
                [85, 255, 255],
                [200, 200, 200],
                [96, 100, 118],
                [255, 140, 140],
                [140, 255, 140],
                [255, 255, 150],
                [140, 150, 255],
                [255, 150, 255],
                [150, 255, 255],
                [245, 245, 245],
            ],
            light: false,
        }
    }

    pub fn light() -> Theme {
        Theme {
            name: "mitos-light".into(),
            fg: [30, 30, 30],
            bg: [245, 245, 245],
            cursor: [20, 120, 40],
            cursor_text: [245, 245, 245],
            selection: [160, 200, 250],
            prompt: [20, 120, 40],
            accent: [20, 120, 40],
            ansi: [
                [40, 40, 40],
                [190, 30, 30],
                [20, 130, 40],
                [160, 110, 0],
                [30, 60, 200],
                [150, 40, 160],
                [0, 130, 140],
                [200, 200, 200],
                [110, 110, 110],
                [220, 60, 60],
                [40, 160, 60],
                [190, 140, 0],
                [60, 90, 230],
                [180, 70, 190],
                [20, 160, 170],
                [255, 255, 255],
            ],
            light: true,
        }
    }

    pub fn high_contrast_dark() -> Theme {
        Theme {
            name: "high-contrast-dark".into(),
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
            light: false,
        }
    }

    pub fn high_contrast_light() -> Theme {
        Theme {
            name: "high-contrast-light".into(),
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
            light: true,
        }
    }

    pub fn solarized_dark() -> Theme {
        Theme {
            name: "solarized-dark".into(),
            fg: [0x83, 0x94, 0x96],
            bg: [0x00, 0x2b, 0x36],
            cursor: [0x93, 0xa1, 0xa1],
            cursor_text: [0x00, 0x2b, 0x36],
            selection: [0x07, 0x36, 0x42],
            prompt: [0x85, 0x99, 0x00],
            accent: [0x2a, 0xa1, 0x98],
            ansi: [
                [0x07, 0x36, 0x42],
                [0xdc, 0x32, 0x2f],
                [0x85, 0x99, 0x00],
                [0xb5, 0x89, 0x00],
                [0x26, 0x8b, 0xd2],
                [0xd3, 0x36, 0x82],
                [0x2a, 0xa1, 0x98],
                [0xee, 0xe8, 0xd5],
                [0x00, 0x2b, 0x36],
                [0xcb, 0x4b, 0x16],
                [0x58, 0x6e, 0x75],
                [0x65, 0x7b, 0x83],
                [0x83, 0x94, 0x96],
                [0x6c, 0x71, 0xc4],
                [0x93, 0xa1, 0xa1],
                [0xfd, 0xf6, 0xe3],
            ],
            light: false,
        }
    }

    /// Re-tint the accent-derived colours (prompt, cursor, accent).
    pub fn with_accent(mut self, accent: Rgb) -> Theme {
        self.accent = accent;
        self.prompt = accent;
        self.cursor = accent;
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

    #[test]
    fn cube_and_grey_ramp() {
        let t = Theme::dark();
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
        // already fine => untouched
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
        for n in Theme::names() {
            assert!(Theme::by_name(n).is_some(), "{n}");
        }
    }
}
