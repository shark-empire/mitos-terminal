// mitos-terminal/src/fx/rain.rs
use std::time::{Duration, Instant};
use egui::{Align2, Color32, Context, FontId, Painter, Pos2, Rect};
use rand::{rngs::ThreadRng, Rng};

const GLYPHS: &[char] = &[
    'ｱ','ｲ','ｳ','ｴ','ｵ','ｶ','ｷ','ｸ','ｹ','ｺ','ｻ','ｼ','ｽ','ｾ','ｿ','ﾀ','ﾁ','ﾂ','ﾃ','ﾄ',
    'ﾅ','ﾆ','ﾇ','ﾈ','ﾉ','ﾊ','ﾋ','ﾌ','ﾍ','ﾎ','ﾏ','ﾐ','ﾑ','ﾒ','ﾓ','ﾔ','ﾕ','ﾖ','ﾗ','ﾘ',
    'ﾙ','ﾚ','ﾛ','ﾜ','ﾝ','0','1','2','3','4','5','6','7','8','9',':','・','=','*','+','<','>','|'],
];

struct Column { head: f32, speed: f32, trail: usize, cells: Vec<char> }

pub struct Rain {
    cols: Vec<Column>,
    last: Instant,
    pub intensity: f32,        // current (eased)
    target: f32,               // where we're heading
}

impl Column {
    fn spawn(rng: &mut ThreadRng, anywhere: bool) -> Self {
        let trail = rng.gen_range(8..24);
        Column {
            head: if anywhere { rng.gen_range(0.0..60.0) } else { -rng.gen_range(0.0..20.0) },
            speed: rng.gen_range(6.0..22.0),
            trail,
            cells: (0..trail).map(|_| GLYPHS[rng.gen_range(0..GLYPHS.len())]).collect(),
        }
    }
}

impl Rain {
    pub fn new(ncols: usize) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            cols: (0..ncols).map(|_| Column::spawn(&mut rng, true)).collect(),
            last: Instant::now(),
            intensity: 0.0,
            target: 0.22,
        }
    }

    /// Feed normalized PTY throughput (0.0..=1.0). Rain surges while text prints.
    pub fn feed_activity(&mut self, io: f32) {
        self.target = 0.16 + io.clamp(0.0, 1.0) * 0.60;
    }

    pub fn paint(&mut self, painter: &Painter, rect: Rect, cell: [f32; 2], ctx: &Context) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;

        // Ease intensity — cinematic, never snappy
        self.intensity += (self.target - self.intensity) * (dt * 3.0).min(1.0);

        let rows = (rect.height() / cell[1]) as i32 + 2;
        let mut rng = rand::thread_rng();

        for (i, col) in self.cols.iter_mut().enumerate() {
            col.head += col.speed * dt;
            if col.head as i32 - col.trail as i32 > rows {
                *col = Column::spawn(&mut rng, false);
            }
            // Flicker: mutate a random trail glyph
            if rng.gen_bool(0.3) {
                let t = rng.gen_range(0..col.cells.len());
                col.cells[t] = GLYPHS[rng.gen_range(0..GLYPHS.len())];
            }

            let x = rect.min.x + i as f32 * cell[0];
            let head = col.head as i32;
            for t in 0..col.trail as i32 {
                let row = head - t;
                if row < 0 || row >= rows { continue; }
                let fade = 1.0 - t as f32 / col.trail as f32;
                let a = fade * fade * self.intensity;
                let color = if t == 0 {
                    // White-hot head
                    Color32::from_rgba_unmultiplied(210, 255, 235, (self.intensity * 235.0) as u8)
                } else {
                    Color32::from_rgba_unmultiplied(0, 255, 140, (a * 200.0) as u8)
                };
                painter.text(
                    Pos2::new(x, rect.min.y + row as f32 * cell[1]),
                    Align2::LEFT_TOP,
                    col.cells[t as usize % col.cells.len()].to_string(),
                    FontId::monospace(cell[1] * 0.95),
                    color,
                );
            }
        }
        // Keep the film rolling at ~30fps
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
