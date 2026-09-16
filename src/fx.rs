//! Cinematic background FX for MITOS Terminal.
//! Reproduces the reference look: Matrix code-rain columns with white-hot
//! heads, a faint drifting holographic grid, a slow radar sweep band,
//! and CRT scanlines on top.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke};

// ---------------------------------------------------------------------------
// Deterministic hashing (stable across frames, no RNG dependency)
// ---------------------------------------------------------------------------

#[inline]
pub fn hash(n: u64) -> u64 {
    let mut x = n;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 29;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 32;
    x
}

#[inline]
pub fn hash3(a: u64, b: u64, c: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ b.wrapping_mul(0x517C_C1B7_2722_0A95)
        ^ c.wrapping_mul(0x2545_F491_4F6C_DD1D);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Glyph pool for the rain: hex + terminal symbols, like the reference shot
const GLYPHS: &[u8] = b"0123456789ABCDEF<>/\\|[]{}=+*#%&@$?:;^~";

#[inline]
pub fn glyph(h: u64) -> char {
    GLYPHS[(h as usize) % GLYPHS.len()] as char
}

/// Normalizes a hash to 0.0..1.0
#[inline]
fn unit(h: u64) -> f32 {
    (h as f64 / u64::MAX as f64) as f32
}

// ---------------------------------------------------------------------------
// CODE RAIN (the falling glyph columns from the image)
// ---------------------------------------------------------------------------

struct Drop {
    x: f32,     // horizontal position in pixels
    head: f32,  // y position of the falling head
    speed: f32, // pixels per second
    trail: f32, // trail length in pixels
    seed: u64,  // per-drop seed for glyph mutation
}

pub struct CodeRain {
    drops: Vec<Drop>,
    density: f32,    // drops per 100px of width
    opacity: f32,    // master alpha 0.0..=1.0
    glyph_size: f32,
    last_rect: Rect,
}

impl CodeRain {
    pub fn new(density: f32, opacity: f32) -> Self {
        Self {
            drops: Vec::new(),
            density,
            opacity,
            glyph_size: 14.0_f32,
            last_rect: Rect::NOTHING,
        }
    }

    fn rebuild(&mut self, rect: Rect) {
        let count = ((rect.width() / 100.0_f32) * self.density).clamp(8.0_f32, 64.0_f32) as usize;
        self.drops.clear();
        self.drops.reserve(count);
        for i in 0..count {
            let seed = hash(i as u64 + 0x5EED);
            let x = rect.left() + unit(hash(seed)) * rect.width();
            // Start scattered above/inside the screen so the first frame isn't empty
            let head = rect.top() + unit(hash(seed ^ 0xA5A5)) * rect.height() * 2.0_f32 - rect.height();
            let speed = 40.0_f32 + unit(hash(seed ^ 0x0F0F)) * 160.0_f32;
            let trail = 60.0_f32 + unit(hash(seed ^ 0x1234)) * 200.0_f32;
            self.drops.push(Drop { x, head, speed, trail, seed });
        }
        self.last_rect = rect;
    }

    /// Advance the simulation. Call once per frame.
    pub fn tick(&mut self, dt: f32, rect: Rect) {
        if rect != self.last_rect || self.drops.is_empty() {
            self.rebuild(rect);
        }
        for d in &mut self.drops {
            d.head += d.speed * dt;
            // Respawn above the top edge once the whole trail has exited
            if d.head - d.trail > rect.bottom() {
                let r = unit(hash(d.seed.wrapping_add(d.head as u64)));
                d.head = rect.top() - r * rect.height() * 0.5_f32;
                d.speed = 40.0_f32 + r * 160.0_f32;
                d.trail = 60.0_f32 + unit(hash(d.seed ^ 0x77)) * 200.0_f32;
                d.seed = hash(d.seed ^ 0xBEEF);
            }
        }
    }

    /// Paint the rain behind the terminal content.
    pub fn paint(&self, p: &Painter, rect: Rect, now: f64, tint: [u8; 3]) {
        let font = FontId::monospace(self.glyph_size);
        let tick = (now * 10.0) as u64; // glyph mutation clock (~10 Hz flicker)

        for d in &self.drops {
            let rows = (d.trail / self.glyph_size) as usize;
            for i in 0..rows {
                let y = d.head - i as f32 * self.glyph_size;
                if y < rect.top() || y > rect.bottom() {
                    continue;
                }

                // Quadratic fade: bright near the head, invisible at the tail
                let fade = 1.0_f32 - (i as f32 / rows.max(1) as f32);
                let alpha = (fade * fade * 130.0_f32 * self.opacity) as u8;

                let (ch, color) = if i == 0 {
                    // White-hot head glyph, like the reference image
                    (
                        glyph(hash3(d.seed, tick, 0)),
                        Color32::from_rgba_unmultiplied(210, 255, 210, (190.0_f32 * self.opacity) as u8),
                    )
                } else {
                    (
                        glyph(hash3(d.seed, tick, i as u64)),
                        Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], alpha),
                    )
                };

                p.text(
                    Pos2::new(d.x, y),
                    Align2::CENTER_CENTER,
                    ch.to_string(),
                    font.clone(),
                    color,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BACKGROUND LAYERS (grid / sweep / scanlines)
// ---------------------------------------------------------------------------

/// Faint holographic grid that slowly drifts downward
pub fn paint_grid(p: &Painter, rect: Rect, now: f64, tint: [u8; 3]) {
    let step = 48.0_f32;
    let drift = (now * 6.0) % step as f64;
    let line = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], 9));

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

/// Slow horizontal sweep band with a fading trail and a brighter head line
pub fn paint_sweep(p: &Painter, rect: Rect, now: f64, tint: [u8; 3]) {
    let period = 7.0_f64; // seconds per full pass
    let t = (now % period) / period;
    let y = rect.top() + rect.height() * t as f32;

    let trail_rows = 24;
    for i in 0..trail_rows {
        let yy = y - i as f32 * 3.0_f32;
        if yy < rect.top() {
            break;
        }
        let a = (28.0_f32 * (1.0_f32 - i as f32 / trail_rows as f32)) as u8;
        p.line_segment(
            [Pos2::new(rect.left(), yy), Pos2::new(rect.right(), yy)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], a)),
        );
    }

    p.line_segment(
        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
        Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], 55)),
    );
}

/// CRT scanlines: thin dark lines every 3px across the whole surface
pub fn paint_scanlines(p: &Painter, rect: Rect) {
    let dark = Color32::from_rgba_unmultiplied(0, 0, 0, 28);
    let mut y = rect.top();
    while y < rect.bottom() {
        p.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0_f32, dark),
        );
        y += 3.0_f32;
    }
}
