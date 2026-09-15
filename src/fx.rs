//! mitos-terminal/src/fx.rs
//! Cinematic background layers: digital rain, drifting grid, scan sweep, scanlines.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke};

// --------------------------------------------------------------- hash utils
#[inline]
pub fn hash(n: u64) -> u64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}
#[inline]
pub fn hash3(a: u64, b: u64, c: u64) -> u64 {
    hash(a ^ b.wrapping_mul(0x517C_C1B7_2722_0A95) ^ c.wrapping_mul(0x2545_F491_4F6C_DD1D))
}

const GLYPHS: &[u8] = b"0123456789ABCDEF<>/\\|[]{}=+*#%&@$?:;^~";
#[inline]
pub fn glyph(h: u64) -> char { GLYPHS[(h as usize) % GLYPHS.len()] as char }

// --------------------------------------------------------------- digital rain
struct Drop { x: f32, head: f32, speed: f32, trail: f32, seed: u64 }

pub struct CodeRain {
    drops: Vec<Drop>,
    built_for: (f32, f32),
    pub density: f32,   // 0..=1  (columns per pixel)
    pub opacity: f32,   // 0..=1  master alpha — keep <= 0.35 for readability
    font: FontId,
    cell: f32,
}

impl CodeRain {
    pub fn new(density: f32, opacity: f32) -> Self {
        Self {
            drops: Vec::new(),
            built_for: (0.0, 0.0),
            density,
            opacity,
            font: FontId::monospace(11.0),
            cell: 14.0,
        }
    }

    fn rebuild(&mut self, rect: Rect) {
        let step = 14.0 + (1.0 - self.density) * 26.0;
        let n = (rect.width() / step) as usize;
        self.drops = (0..n).map(|i| {
            let seed = hash(i as u64 + 0xDEAD_BEEF);
            Drop {
                x: rect.left() + i as f32 * step + (seed >> 8 & 5) as f32,
                head: rect.top() - (seed >> 16 & 1023) as f32,
                speed: 40.0 + (seed >> 24 & 127) as f32,
                trail: 90.0 + (seed >> 32 & 150) as f32,
                seed,
            }
        }).collect();
        self.built_for = (rect.width(), rect.height());
    }

    pub fn tick(&mut self, dt: f32, rect: Rect) {
        if (rect.width(), rect.height()) != self.built_for { self.rebuild(rect); }
        for d in &mut self.drops {
            d.head += d.speed * dt;
            if d.head - d.trail > rect.bottom() {
                d.seed = hash(d.seed);                    // new personality on respawn
                d.head = rect.top() - (d.seed & 300) as f32;
                d.speed = 40.0 + (d.seed & 127) as f32;
            }
        }
    }

    pub fn paint(&self, p: &Painter, rect: Rect, now: f64, tint: [u8; 3]) {
        let shimmer = (now * 9.0) as u64;                 // glyphs mutate at ~9 Hz
        for d in &self.drops {
            let mut row = 0.0;
            while row <= d.trail {
                let y = d.head - row;
                if y > rect.top() && y < rect.bottom() {
                    let fade = 1.0 - row / d.trail;
                    let a = (fade * fade * 80.0 * self.opacity) as u8;
                    if a > 6 {
                        let ch = glyph(hash3(d.seed, (y / self.cell) as u64, shimmer));
                        let col = if row < self.cell {
                            // white-hot head of the drop
                            Color32::from_rgba_unmultiplied(
                                tint[0].max(180), tint[1].max(240), tint[2].max(220),
                                (150.0 * self.opacity) as u8)
                        } else {
                            Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], a)
                        };
                        p.text(Pos2::new(d.x, y), Align2::LEFT_TOP,
                               ch.to_string(), self.font.clone(), col);
                    }
                }
                row += self.cell;
            }
        }
    }
}

// --------------------------------------------------------------- drifting grid
pub fn paint_grid(p: &Painter, rect: Rect, now: f64, tint: [u8; 3]) {
    let step = 56.0;
    let drift = (now * 8.0) % step;                       // grid breathes downward
    let line = Stroke::new(1.0, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], 9));
    let mut y = rect.top() - step + drift as f32;
    while y < rect.bottom() {
        p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], line);
        y += step;
    }
    let mut x = rect.left();
    while x < rect.right() {
        p.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], line);
        x += step;
    }
}

// --------------------------------------------------------------- scan sweep
pub fn paint_sweep(p: &Painter, rect: Rect, now: f64, tint: [u8; 3]) {
    let t = (now % 9.0) / 9.0;                            // one pass every 9 s
    let y = rect.top() + t as f32 * rect.height();
    for i in 0..24 {                                      // soft trailing wake
        let a = (16.0 * (1.0 - i as f32 / 24.0)) as u8;
        p.line_segment(
            [Pos2::new(rect.left(), y - i as f32 * 2.0), Pos2::new(rect.right(), y - i as f32 * 2.0)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], a)));
    }
    p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
        Stroke::new(1.5, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], 55)));
}

// --------------------------------------------------------------- CRT scanlines
pub fn paint_scanlines(p: &Painter, rect: Rect) {
    let dark = Color32::from_black_alpha(14);
    let mut y = rect.top();
    while y < rect.bottom() {
        p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                       Stroke::new(1.0, dark));
        y += 3.0;
    }
}
