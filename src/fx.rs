//! Painted effects that give each theme its personality, beyond plain
//! colour: soft glow borders, a simulated frosted-glass surface, a CRT-style
//! vignette, small HUD corner accents, an ambient backdrop for glass
//! surfaces, and the pre-existing cinematic layer (Matrix rain, a drifting
//! grid, a radar sweep, scanlines). Every function here is a `Theme`
//! consumer, never a `Theme` author — see `theme.rs` for what each preset
//! actually sets these to, and `render.rs`/`app.rs` for where they're called.
//!
//! Kept deliberately cheap: everything below is a handful of stroke/fill
//! calls per pane per frame, never per row or per cell, so switching a
//! theme's effects on costs nothing proportional to how much text is on
//! screen.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

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
    density: f32,   // drops per 100px of width
    opacity: f32,   // master alpha 0.0..=1.0
    max_drops: f32, // most drops there ever are, however wide the pane
    glyph_size: f32,
    last_rect: Rect,
}

/// What the single 0..=1 "rain intensity" knob (`[effects] rain_intensity`, the slider in Settings)
/// stands for: `(drops per 100 px of width, brightness, most drops at once)`. 0.5 is the
/// long-standing look (10 per 100 px, at most 64, 55% bright); 0.05 is a few faint streams and
/// 1.0 is dense and bright. Out-of-range values are clamped.
fn rain_params(intensity: f32) -> (f32, f32, f32) {
    let i = intensity.clamp(0.0_f32, 1.0_f32);
    (20.0_f32 * i, 0.2_f32 + 0.7_f32 * i, 128.0_f32 * i)
}

impl CodeRain {
    pub fn new(density: f32, opacity: f32) -> Self {
        Self {
            drops: Vec::new(),
            density,
            opacity,
            max_drops: 64.0_f32,
            glyph_size: 14.0_f32,
            last_rect: Rect::NOTHING,
        }
    }

    /// Rain at a user-chosen intensity (0..=1; 0.5 is the default look).
    pub fn with_intensity(intensity: f32) -> Self {
        let mut rain = Self::new(10.0_f32, 0.55_f32);
        rain.set_intensity(intensity);
        rain
    }

    /// Change the intensity of rain that is already falling. Streams are added or removed on the
    /// next `tick`; the ones that stay keep falling where they are.
    pub fn set_intensity(&mut self, intensity: f32) {
        let (density, opacity, max_drops) = rain_params(intensity);
        self.density = density;
        self.opacity = opacity;
        self.max_drops = max_drops;
    }

    fn target_count(&self, rect: Rect) -> usize {
        ((rect.width() / 100.0_f32) * self.density)
            .clamp(8.0_f32, self.max_drops.max(8.0_f32)) as usize
    }

    /// Drop number `i`, scattered above/inside `rect` so the first frame isn't empty.
    fn spawn(i: usize, rect: Rect) -> Drop {
        let seed = hash(i as u64 + 0x5EED);
        let x = rect.left() + unit(hash(seed)) * rect.width();
        let head =
            rect.top() + unit(hash(seed ^ 0xA5A5)) * rect.height() * 2.0_f32 - rect.height();
        let speed = 40.0_f32 + unit(hash(seed ^ 0x0F0F)) * 160.0_f32;
        let trail = 60.0_f32 + unit(hash(seed ^ 0x1234)) * 200.0_f32;
        Drop {
            x,
            head,
            speed,
            trail,
            seed,
        }
    }

    fn rebuild(&mut self, rect: Rect) {
        let count = self.target_count(rect);
        self.drops.clear();
        self.drops.reserve(count);
        for i in 0..count {
            self.drops.push(Self::spawn(i, rect));
        }
        self.last_rect = rect;
    }

    /// Add or remove streams to match the current intensity without restarting the rain.
    fn fit_count(&mut self, rect: Rect) {
        let want = self.target_count(rect);
        self.drops.truncate(want);
        while self.drops.len() < want {
            let i = self.drops.len();
            self.drops.push(Self::spawn(i, rect));
        }
    }

    /// Advance the simulation. Call once per frame.
    pub fn tick(&mut self, dt: f32, rect: Rect) {
        if rect != self.last_rect || self.drops.is_empty() {
            self.rebuild(rect);
        } else {
            self.fit_count(rect);
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
    /// `tint` colours the trail; `head` is the "white-hot" leading glyph —
    /// kept as a separate parameter (rather than hardcoded) because a fixed
    /// near-white head is only actually "hot" against a dark background. On
    /// a light theme the caller passes a dark, saturated tone instead, so
    /// the head glyph stays the brightest thing in the column either way.
    pub fn paint(&self, p: &Painter, rect: Rect, now: f64, tint: [u8; 3], head: [u8; 3]) {
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
                    // The leading glyph: brightest against whatever
                    // background this theme actually has (see `head`'s doc).
                    (
                        glyph(hash3(d.seed, tick, 0)),
                        Color32::from_rgba_unmultiplied(
                            head[0],
                            head[1],
                            head[2],
                            (190.0_f32 * self.opacity) as u8,
                        ),
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
    let line = Stroke::new(
        1.0_f32,
        Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], 9),
    );

    let mut y = rect.top() - step + drift as f32;
    while y < rect.bottom() {
        p.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            line,
        );
        y += step;
    }

    let mut x = rect.left();
    while x < rect.right() {
        p.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            line,
        );
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
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], a),
            ),
        );
    }

    p.line_segment(
        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
        Stroke::new(
            1.5_f32,
            Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], 55),
        ),
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

// ---------------------------------------------------------------------------
// THEME CHROME EFFECTS (glow / glass / vignette / HUD accents / ambient backdrop)
// ---------------------------------------------------------------------------

/// Soft outer glow around a rounded rect, built from a handful of
/// progressively larger, fainter strokes rather than a real blur — cheap,
/// and tasteful at the low `intensity` values every shipped theme uses (see
/// `theme.rs`'s `brief_mandated_restraint_is_actually_restrained` test).
pub fn paint_glow_border(p: &Painter, rect: Rect, radius: f32, color: [u8; 3], intensity: f32) {
    let intensity = intensity.clamp(0.0, 1.0);
    if intensity <= 0.001 {
        return;
    }
    const PASSES: i32 = 4;
    let peak = 70.0 * intensity;
    for i in (0..PASSES).rev() {
        let grow = i as f32 * 1.6 + 1.0;
        let a = (peak * (1.0 - i as f32 / PASSES as f32)).max(4.0) as u8;
        let r = rect.expand(grow);
        let stroke = Stroke::new(
            1.0 + grow * 0.4,
            Color32::from_rgba_unmultiplied(color[0], color[1], color[2], a),
        );
        p.rect_stroke(r, radius + grow, stroke);
    }
    // A crisp inner line ties the glow to an actual edge instead of just haze.
    let edge_a = (peak + 60.0).min(255.0) as u8;
    p.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(color[0], color[1], color[2], edge_a),
        ),
    );
}

/// The frosted-glass surface fill: a translucent base plus one restrained
/// highlight band near the top (a hint of "gentle reflection", not a glossy
/// button). `base_alpha` is 0..=255. What shows "through" the glass is
/// `paint_ambient_backdrop`'s job, painted first, underneath this.
pub fn paint_glass_surface(p: &Painter, rect: Rect, radius: f32, base: [u8; 3], base_alpha: u8) {
    p.rect_filled(
        rect,
        radius,
        Color32::from_rgba_unmultiplied(base[0], base[1], base[2], base_alpha),
    );
    let highlight_h = (rect.height() * 0.16).min(40.0);
    if highlight_h > 2.0 {
        let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), highlight_h));
        p.rect_filled(
            band,
            radius,
            Color32::from_rgba_unmultiplied(255, 255, 255, 10),
        );
    }
}

/// CRT-style darkened-corner falloff, built from nested inset strokes
/// (a poor-man's radial gradient — no mesh/shader access from here).
pub fn paint_vignette(p: &Painter, rect: Rect, strength: f32) {
    let strength = strength.clamp(0.0, 1.0);
    if strength <= 0.001 {
        return;
    }
    const PASSES: i32 = 6;
    let max_inset = rect.width().min(rect.height()) * 0.16;
    for i in 0..PASSES {
        let t = i as f32 / (PASSES - 1) as f32; // 0 at the very edge .. 1 innermost
        let inset = t * max_inset;
        let a = (strength * 46.0 * (1.0 - t)) as u8;
        if a == 0 {
            continue;
        }
        let width = (max_inset / PASSES as f32) + 1.0;
        p.rect_stroke(
            rect.shrink(inset),
            0.0,
            Stroke::new(width, Color32::from_rgba_unmultiplied(0, 0, 0, a)),
        );
    }
}

/// Small corner tick-mark accents (viewfinder-style), one short L per
/// corner — the whole of "HUD accents": no crosshairs, no readouts, no
/// clutter, just enough to feel technical.
pub fn paint_hud_accents(p: &Painter, rect: Rect, color: [u8; 3], alpha: u8) {
    let len = 12.0_f32.min(rect.width() * 0.1).min(rect.height() * 0.1);
    if len < 3.0 {
        return;
    }
    let stroke = Stroke::new(
        1.2_f32,
        Color32::from_rgba_unmultiplied(color[0], color[1], color[2], alpha),
    );
    for (c, sx, sy) in [
        (rect.left_top(), 1.0_f32, 1.0_f32),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        p.line_segment([c, Pos2::new(c.x + len * sx, c.y)], stroke);
        p.line_segment([c, Pos2::new(c.x, c.y + len * sy)], stroke);
    }
}

/// A soft, low-alpha colour wash behind glass surfaces — what shows
/// "through" the glass when the user hasn't set a real wallpaper image (see
/// `AppearanceCfg::wallpaper_path`) and the desktop compositor isn't (or
/// can't be) blurring the real desktop behind this window.
///
/// Deliberately time-independent: `seed` (a pane id works well) picks fixed
/// blob positions once and they never move. The brief asks to avoid
/// constant animation, and a static backdrop also means painting it never
/// forces a repaint the way an animated one would — the two goals turned
/// out to want the same answer.
///
/// `opacity` (0..=1) is the window opacity: the base wash and the blobs are scaled by it, so the
/// backdrop never turns a see-through window opaque (it used to paint a fully opaque base here,
/// which made the Opacity setting a no-op whenever the glass look was on).
pub fn paint_ambient_backdrop(
    p: &Painter,
    rect: Rect,
    seed: u64,
    base: [u8; 3],
    accent_a: [u8; 3],
    accent_b: [u8; 3],
    opacity: f32,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    p.rect_filled(
        rect,
        0.0,
        Color32::from_rgba_unmultiplied(base[0], base[1], base[2], (opacity * 255.0).round() as u8),
    );
    let short = rect.width().min(rect.height());
    for (i, (color, size_frac, peak_alpha)) in [
        (accent_a, 0.55, 26u8),
        (accent_b, 0.42, 22),
        (base, 0.30, 30),
    ]
    .into_iter()
    .enumerate()
    {
        let h = hash3(seed, i as u64, 0x0B1);
        let fx = 0.15 + 0.7 * unit(h);
        let fy = 0.15 + 0.7 * unit(hash(h));
        let center = Pos2::new(
            rect.left() + rect.width() * fx,
            rect.top() + rect.height() * fy,
        );
        let peak = (peak_alpha as f32 * opacity).round() as u8;
        paint_soft_blob(p, center, short * size_frac, color, peak);
    }
}

/// One soft radial blob, built from nested circles of falling alpha — the
/// same "fake blur via many cheap passes" trick as `paint_glow_border`.
fn paint_soft_blob(p: &Painter, center: Pos2, radius: f32, color: [u8; 3], peak_alpha: u8) {
    const PASSES: i32 = 8;
    for i in (0..PASSES).rev() {
        let t = i as f32 / PASSES as f32;
        let r = radius * (0.3 + 0.7 * t);
        let a = (peak_alpha as f32 * (1.0 - t) * (1.0 - t)) as u8;
        if a == 0 {
            continue;
        }
        p.circle_filled(
            center,
            r,
            Color32::from_rgba_unmultiplied(color[0], color[1], color[2], a),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_intensity_half_is_the_original_look() {
        let (density, opacity, max_drops) = rain_params(0.5);
        assert!((density - 10.0).abs() < 1e-4);
        assert!((opacity - 0.55).abs() < 1e-4);
        assert!((max_drops - 64.0).abs() < 1e-4);
    }

    #[test]
    fn rain_gets_denser_and_brighter_with_intensity() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 600.0));
        let count = |i: f32| CodeRain::with_intensity(i).target_count(rect);
        assert_eq!(count(0.5), 64, "the default look: 64 streams");
        assert_eq!(count(1.0), 128);
        assert_eq!(count(0.25), 32);
        assert_eq!(count(0.0), 8, "never fewer than 8 streams");
        assert!(count(0.05) < count(0.25) && count(0.25) < count(0.5) && count(0.5) < count(1.0));
        assert!(rain_params(0.1).1 < rain_params(0.9).1, "brighter at high intensity");
        // Out-of-range input is clamped rather than trusted.
        assert_eq!(count(5.0), count(1.0));
        assert_eq!(count(-1.0), count(0.0));
    }

    #[test]
    fn changing_intensity_keeps_the_streams_that_stay() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 600.0));
        let mut rain = CodeRain::with_intensity(0.5);
        rain.tick(0.0, rect);
        assert_eq!(rain.drops.len(), 64);
        rain.tick(0.5, rect); // let it fall for a moment
        let before: Vec<f32> = rain.drops.iter().take(10).map(|d| d.head).collect();

        rain.set_intensity(1.0);
        rain.tick(0.0, rect);
        assert_eq!(rain.drops.len(), 128, "more streams are added");
        let after: Vec<f32> = rain.drops.iter().take(10).map(|d| d.head).collect();
        assert_eq!(before, after, "the streams that stay are not restarted");

        rain.set_intensity(0.1);
        rain.tick(0.0, rect);
        assert_eq!(rain.drops.len(), 12, "streams are removed again");
    }

    #[test]
    fn hash_is_deterministic_and_spreads_out() {
        assert_eq!(hash(42), hash(42));
        assert_ne!(hash(1), hash(2));
        assert_eq!(hash3(1, 2, 3), hash3(1, 2, 3));
        assert_ne!(hash3(1, 2, 3), hash3(3, 2, 1));
    }

    #[test]
    fn unit_stays_in_zero_one() {
        for n in [0u64, 1, 12345, u64::MAX / 2, u64::MAX] {
            let u = unit(hash(n));
            assert!((0.0..=1.0).contains(&u), "{n} -> {u}");
        }
    }

    #[test]
    fn glyph_never_indexes_out_of_the_pool() {
        for n in [0u64, 1, u64::MAX, hash(999)] {
            let c = glyph(n);
            assert!(GLYPHS.contains(&(c as u8)));
        }
    }

    #[test]
    fn ambient_backdrop_blob_placement_is_stable_for_a_given_seed() {
        // paint_ambient_backdrop's positions come from hash3(seed, i, 0x0B1)
        // and hash(...) — same inputs, same outputs, every call. This is
        // what makes the backdrop genuinely static rather than "animated
        // but so slow you can't tell": there is no time input to it at all.
        let a = hash3(7, 0, 0x0B1);
        let b = hash3(7, 0, 0x0B1);
        assert_eq!(a, b);
        assert_eq!(unit(a), unit(b));
    }
}
