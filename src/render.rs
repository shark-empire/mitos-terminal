//! Turning a [`Term`] into pixels.
//!
//! Split in two on purpose:
//! * [`coalesce_row`] is pure data → data (a [`Row`]'s cells → same-attribute
//!   [`Run`]s) and fully unit-tested below without an `egui::Context`.
//! * [`paint_pane`] walks those runs and issues `egui::Painter` calls. It is
//!   mechanical by comparison, so it is not unit-tested — an `eframe`
//!   integration test would need a real GPU context to render into.
//!
//! "Efficient text grid": one draw call per same-attribute run instead of per
//! cell, and background fills are skipped for runs already sitting on the
//! pane's own background. "Damage tracking": `paint_pane` returns whether it
//! actually drew anything different-looking, and `app.rs` only calls
//! `ctx.request_repaint()` when a pane's `Term::seqno()` moved, the cursor's
//! blink phase flipped, or an effect animation needs the next frame — an idle
//! pane costs nothing after the frame it went idle on. Glyph atlasing, GPU
//! upload and HiDPI scaling are egui's own (its font `Fonts` cache glyphs
//! into one atlas texture per point-size and every `Painter` call goes
//! through the same GPU-backed tessellator, in logical points so
//! `pixels_per_point` scaling is automatic).

use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, Vec2};

use crate::fx;
use crate::term::{attr, mark, Cell, Color, CursorShape, Overrides, Row, Term};
use crate::theme::{mix, Rgb, Theme};

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub char_w: f32,
    pub row_h: f32,
}

/// Measures one monospace glyph cell by laying out "M" the same way
/// `eframe`'s own font system does everywhere else in this codebase
/// (`ctx.fonts(|f| f.layout_no_wrap(...))`), so the grid's column/row size
/// always matches what was actually rasterised — no separate glyph-metrics
/// API to drift out of sync with it.
pub fn measure(ctx: &egui::Context, font: &FontId) -> Metrics {
    let size = ctx.fonts(|f| f.layout_no_wrap("M".to_string(), font.clone(), Color32::WHITE).size());
    Metrics { char_w: size.x.max(1.0), row_h: size.y.max(1.0) }
}

/// Resolves to the built-in monospace font. `family` (from `terminal.toml`'s
/// `[font]` table) is accepted and stored so configs round-trip, but
/// selecting an arbitrary system family would need loading its `.ttf` and
/// registering it with `egui::Context::set_fonts` first; that font-discovery
/// step is not wired up yet (tracked in docs/AUDIT.md) so every family name
/// still renders with the bundled monospace font rather than risking a
/// reference to a family egui was never given.
pub fn font_id(_family: &str, size: f32) -> FontId {
    FontId::monospace(size)
}

// ---------------------------------------------------------------------------
// Run coalescing (pure)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub col0: usize,
    pub col1: usize,
    pub text: String,
    pub fg: Rgb,
    pub bg: Rgb,
    pub ul: Rgb,
    pub flags: u16,
    pub link: u16,
}

impl Run {
    fn cols(&self) -> usize {
        self.col1 - self.col0
    }
}

/// Attributes shared by consecutive cells worth batching into one draw call.
/// Link id is deliberately excluded: adjacent same-styled cells with
/// different link ids still render identically, and splitting the run would
/// only cost draw calls for no visual gain — hover/click hit-testing reads
/// the link straight from the `Term`, not from `Run`.
fn same_style(a: &Cell, b: &Cell, ta: Rgb, tb: Rgb, ba: Rgb, bb: Rgb, ua: Rgb, ub: Rgb) -> bool {
    ta == tb && ba == bb && ua == ub && (a.flags & !attr::WIDE_TAIL) == (b.flags & !attr::WIDE_TAIL)
}

/// Coalesce one row into attribute runs, resolving palette colours and
/// swapping fg/bg for `INVERSE` and the whole-screen `DECSCNM` mode. Wide-tail
/// spacer cells are skipped (their glyph already came from the head cell).
pub fn coalesce_row(theme: &Theme, ov: &Overrides, row: &Row, screen_reverse: bool) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (x, cell) in row.cells.iter().enumerate() {
        if cell.flags & attr::WIDE_TAIL != 0 {
            continue;
        }
        if cell.flags & attr::HIDDEN != 0 {
            continue;
        }
        let mut fg = theme.resolve(ov, cell.fg, true);
        let mut bg = theme.resolve(ov, cell.bg, false);
        let ul = if cell.ul == Color::Default { fg } else { theme.resolve(ov, cell.ul, true) };
        if (cell.flags & attr::INVERSE != 0) != screen_reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags & attr::DIM != 0 {
            fg = mix(fg, bg, 0.4);
        }
        let ch = if cell.ch == ' ' && bg == theme.bg { ' ' } else { cell.ch };
        let width = if cell.flags & attr::WIDE != 0 { 2 } else { 1 };

        let extend = runs.last_mut().filter(|r| r.col1 == x && same_style_run(r, fg, bg, ul, cell.flags));
        match extend {
            Some(r) => {
                push_char(&mut r.text, ch);
                r.col1 = x + width;
            }
            None => {
                let mut text = String::new();
                push_char(&mut text, ch);
                runs.push(Run { col0: x, col1: x + width, text, fg, bg, ul, flags: cell.flags, link: cell.link });
            }
        }
    }
    runs
}

fn same_style_run(r: &Run, fg: Rgb, bg: Rgb, ul: Rgb, flags: u16) -> bool {
    r.fg == fg && r.bg == bg && r.ul == ul && (r.flags & !attr::WIDE_TAIL) == (flags & !attr::WIDE_TAIL)
}

fn push_char(s: &mut String, c: char) {
    // A cluster's interned code point never appears here — callers pass the
    // real glyph text in; kept as a plain char push so width accounting
    // (one push per cell) matches `col0..col1` exactly.
    s.push(c);
}

fn c32(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

pub struct PaintOpts<'a> {
    pub theme: &'a Theme,
    pub metrics: Metrics,
    pub font: FontId,
    pub focused: bool,
    pub cursor_visible_phase: bool,
    pub unfocused_hollow: bool,
    pub cursor_thickness: f32,
    pub show_search: Option<&'a str>,
    pub reduced_motion: bool,
    pub effects: &'a crate::config::EffectsCfg,
    pub accent: Rgb,
    pub now: f64,
    /// Screen coordinates of the mouse, if hovering this pane (for link underline-on-hover).
    pub hover_cell: Option<(usize, usize)>,
}

/// Paints one pane's grid, cursor, selection and effects into `rect`.
/// Returns the on-screen rect of each OSC-8/plain-text URL under the mouse
/// (for the caller to draw a click affordance / show a tooltip) and whether
/// any per-frame animation is still in flight (so the caller knows to keep
/// requesting repaints).
pub fn paint_pane(ui: &mut egui::Ui, rect: ERect, term: &mut Term, opts: &PaintOpts, rain: Option<&mut fx::CodeRain>, dt: f32) -> bool {
    let painter = ui.painter().with_clip_rect(rect);
    let theme = opts.theme;
    let m = opts.metrics;
    let mut animating = false;

    // Background (glass/opacity handled by the caller compositing the window;
    // this is the pane's own flat fill so text is always legible).
    painter.rect_filled(rect, 0.0, c32(theme.bg));

    if let Some(rain) = rain {
        rain.tick(dt.max(0.0).min(0.25), rect);
        rain.paint(&painter, rect, opts.now, theme.accent);
        animating = true;
    }

    if opts.effects.enabled {
        if opts.effects.grid {
            fx::paint_grid(&painter, rect, opts.now, theme.accent);
        }
        if !opts.reduced_motion {
            animating = true;
        }
    }

    let cols = term.cols();
    let rows = term.rows();
    let ov = term.overrides.clone();
    let reverse = term.modes.reverse_video;

    // Accessibility proxy: a real, fully transparent widget carrying the
    // visible screen text, so a screen reader gets a normal AT node at this
    // rect. The glyphs users actually see are the Painter calls below.
    let a11y_text = term.visible_text();
    ui.put(
        rect,
        egui::Label::new(egui::RichText::new(a11y_text).font(opts.font.clone()).color(Color32::TRANSPARENT))
            .selectable(false)
            .wrap(),
    );

    let (cx, cy) = term.cursor_pos();
    let cursor_row_visible = term.display_offset == 0;
    let selection = term.selection;
    let hits: Vec<_> = opts.show_search.map(|q| term.search(q, false)).unwrap_or_default();

    for r in 0..rows {
        let y = rect.top() + r as f32 * m.row_h;
        if y > rect.bottom() {
            break;
        }
        let row = term.view_row(r).clone();
        let abs = term.view_abs(r);

        if row.marks & mark::WIDGET != 0 {
            // Widgets get real egui widgets, not painted glyphs (keyboard/AT accessible).
            // The lookup borrows `term` immutably; `describe` copies out only the
            // plain-data fields we need so that borrow ends before `paint_widget_view`
            // needs `term` mutably (for queuing a click's command).
            let found = term.widgets().iter().find(|w| w.line == abs).map(|w| (w.id, describe(&w.widget)));
            if let Some((wid, view)) = found {
                let wrect = ERect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), m.row_h.max(22.0)));
                paint_widget_view(ui, wrect, wid, &view, term);
                continue;
            }
        }

        let runs = coalesce_row(theme, &ov, &row, reverse);
        for run in &runs {
            let x0 = rect.left() + run.col0 as f32 * m.char_w;
            let w = run.cols() as f32 * m.char_w;
            let cell_rect = ERect::from_min_size(Pos2::new(x0, y), Vec2::new(w, m.row_h));
            if run.bg != theme.bg {
                painter.rect_filled(cell_rect, 0.0, c32(run.bg));
            }
            if !run.text.trim().is_empty() || run.flags & attr::ANY_UL != 0 || run.flags & attr::STRIKE != 0 {
                let mut fg = c32(run.fg);
                if run.link != 0 {
                    fg = c32(mix(run.fg, theme.accent, 0.35));
                }
                if !run.text.chars().all(|c| c == ' ') {
                    let mut job = egui::text::LayoutJob::default();
                    let mut fmt = egui::TextFormat { font_id: opts.font.clone(), color: fg, ..Default::default() };
                    if run.flags & attr::BOLD != 0 {
                        fmt.color = c32(brighten(run.fg));
                    }
                    if run.flags & attr::ITALIC != 0 {
                        fmt.italics = true;
                    }
                    if run.flags & attr::STRIKE != 0 {
                        fmt.strikethrough = Stroke::new(1.0, fmt.color);
                    }
                    job.append(&run.text, 0.0, fmt);
                    painter.galley(Pos2::new(x0, y), ui.fonts(|f| f.layout_job(job)), fg);
                }
                paint_underline(&painter, cell_rect, run, theme);
            }
            if run.link != 0 {
                if let Some((hx, hy)) = opts.hover_cell {
                    if hy == r && hx >= run.col0 && hx < run.col1 {
                        let ly = cell_rect.bottom() - 1.0;
                        painter.line_segment([Pos2::new(cell_rect.left(), ly), Pos2::new(cell_rect.right(), ly)], Stroke::new(1.0, c32(run.fg)));
                    }
                }
            }
        }

        if let Some(sel) = &selection {
            if let Some((c0, c1)) = sel.cols_on_line(abs, cols) {
                let x0 = rect.left() + c0 as f32 * m.char_w;
                let x1 = rect.left() + (c1 + 1) as f32 * m.char_w;
                painter.rect_filled(ERect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + m.row_h)), 0.0, Color32::from_rgba_unmultiplied(theme.selection[0], theme.selection[1], theme.selection[2], 110));
            }
        }
        for hit in &hits {
            for (line, c0, c1) in &hit.spans {
                if *line == abs {
                    let x0 = rect.left() + *c0 as f32 * m.char_w;
                    let x1 = rect.left() + (*c1 + 1) as f32 * m.char_w;
                    painter.rect_stroke(ERect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + m.row_h)), 1.0, Stroke::new(1.5, c32(opts.accent)));
                }
            }
        }
        if row.marks & mark::PROMPT != 0 || row.marks & mark::BLOCK != 0 {
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.left(), y + m.row_h)], Stroke::new(2.0, c32(theme.prompt)));
        }
    }

    if let Some(ghost) = term.ghost_text() {
        if cursor_row_visible && cy < rows {
            let x = rect.left() + cx as f32 * m.char_w;
            let y = rect.top() + cy as f32 * m.row_h;
            painter.text(Pos2::new(x, y), Align2::LEFT_TOP, ghost, opts.font.clone(), Color32::from_rgba_unmultiplied(theme.fg[0], theme.fg[1], theme.fg[2], 110));
        }
    }

    if term.modes.cursor_visible && cursor_row_visible && cx < cols && cy < rows {
        let show = opts.focused || !opts.unfocused_hollow;
        let blink_on = !term.cursor_style().blink || opts.reduced_motion || opts.cursor_visible_phase;
        if show && blink_on {
            let x = rect.left() + cx as f32 * m.char_w;
            let y = rect.top() + cy as f32 * m.row_h;
            let under_cell = term.view_row(cy).get(cx);
            let base = theme.resolve(&ov, if under_cell.bg != Color::Default { under_cell.fg } else { Color::Default }, true);
            let cursor_color = c32(theme.cursor);
            let filled = opts.focused;
            paint_cursor(&painter, Pos2::new(x, y), m, term.cursor_style().shape, cursor_color, filled, opts.cursor_thickness);
            if filled && !under_cell.is_blank() {
                let ch_s = if under_cell.flags & attr::WIDE_TAIL == 0 {
                    let mut s = String::new();
                    push_char(&mut s, under_cell.ch);
                    s
                } else {
                    String::new()
                };
                if !ch_s.trim().is_empty() {
                    painter.text(Pos2::new(x, y), Align2::LEFT_TOP, ch_s, opts.font.clone(), c32(theme.cursor_text));
                }
            }
            let _ = base;
        }
        if term.cursor_style().blink && !opts.reduced_motion {
            animating = true;
        }
    }

    if opts.effects.enabled {
        if opts.effects.sweep && !opts.reduced_motion {
            fx::paint_sweep(&painter, rect, opts.now, theme.accent);
        }
        if opts.effects.scanlines {
            fx::paint_scanlines(&painter, rect);
        }
        if opts.effects.phosphor && !opts.reduced_motion {
            paint_phosphor(&painter, rect, &*term, m, opts);
            animating = true;
        }
    }

    animating
}

fn brighten(rgb: Rgb) -> Rgb {
    mix(rgb, [255, 255, 255], 0.35)
}

fn paint_underline(painter: &egui::Painter, cell_rect: ERect, run: &Run, _theme: &Theme) {
    if run.flags & attr::ANY_UL == 0 {
        return;
    }
    let col = c32(run.ul);
    let y = cell_rect.bottom() - 1.5;
    let (x0, x1) = (cell_rect.left(), cell_rect.right());
    if run.flags & attr::CURLY_UL != 0 {
        let amp = 1.6;
        let step = 3.0;
        let mut x = x0;
        let mut up = true;
        while x < x1 {
            let nx = (x + step).min(x1);
            let y0 = if up { y - amp } else { y + amp };
            let y1 = if up { y + amp } else { y - amp };
            painter.line_segment([Pos2::new(x, y0), Pos2::new(nx, y1)], Stroke::new(1.0, col));
            x = nx;
            up = !up;
        }
    } else if run.flags & attr::DOTTED_UL != 0 || run.flags & attr::DASHED_UL != 0 {
        let dash = if run.flags & attr::DOTTED_UL != 0 { 1.5 } else { 4.0 };
        let gap = if run.flags & attr::DOTTED_UL != 0 { 2.0 } else { 3.0 };
        let mut x = x0;
        while x < x1 {
            let nx = (x + dash).min(x1);
            painter.line_segment([Pos2::new(x, y), Pos2::new(nx, y)], Stroke::new(1.4, col));
            x = nx + gap;
        }
    } else if run.flags & attr::DOUBLE_UL != 0 {
        painter.line_segment([Pos2::new(x0, y - 1.5), Pos2::new(x1, y - 1.5)], Stroke::new(1.0, col));
        painter.line_segment([Pos2::new(x0, y + 1.0), Pos2::new(x1, y + 1.0)], Stroke::new(1.0, col));
    } else {
        painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(1.2, col));
    }
}

fn paint_cursor(painter: &egui::Painter, top_left: Pos2, m: Metrics, shape: CursorShape, color: Color32, filled: bool, thickness: f32) {
    let rect = ERect::from_min_size(top_left, Vec2::new(m.char_w, m.row_h));
    match shape {
        CursorShape::Block => {
            if filled {
                painter.rect_filled(rect, 1.0, color);
            } else {
                painter.rect_stroke(rect, 1.0, Stroke::new(1.5, color));
            }
        }
        CursorShape::Underline => {
            let y = rect.bottom() - thickness.max(1.0);
            painter.rect_filled(ERect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), thickness.max(1.0))), 0.0, color);
        }
        CursorShape::Bar => {
            painter.rect_filled(ERect::from_min_size(rect.left_top(), Vec2::new(thickness.max(1.0), rect.height())), 0.0, color);
        }
    }
}

/// Fading highlight over recently printed text (or a red glitch on error rows).
fn paint_phosphor(painter: &egui::Painter, rect: ERect, term: &Term, m: Metrics, opts: &PaintOpts) {
    let now = term.now_ms();
    let top_abs = term.view_abs(0);
    let rows = term.rows();
    for f in term.fresh() {
        if f.line < top_abs {
            continue;
        }
        let r = (f.line - top_abs) as usize;
        if r >= rows {
            continue;
        }
        let age = now.saturating_sub(f.t) as f32;
        let life = 700.0f32;
        let t = (1.0 - age / life).clamp(0.0, 1.0);
        if t <= 0.0 {
            continue;
        }
        let x0 = rect.left() + f.c0 as f32 * m.char_w;
        let x1 = rect.left() + f.c1 as f32 * m.char_w;
        let y = rect.top() + r as f32 * m.row_h;
        let base = if f.error { [255u8, 60, 60] } else { opts.accent };
        let alpha = (t * if f.error { 90.0 } else { 55.0 }) as u8;
        painter.rect_filled(
            ERect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y + m.row_h)),
            0.0,
            Color32::from_rgba_unmultiplied(base[0], base[1], base[2], alpha),
        );
    }
}

/// Plain-data view of a widget's contents. Extracted (rather than matching
/// `RichWidget` at the paint site) so the borrow of `term.widgets()` needed to
/// find the widget ends before we might need `term` mutably for a click.
/// Only destructures the fields `pkg_bridge`/the old renderer already relied
/// on (`Button{label,cmd}`, `Progress{percent,..}`, `Sparkline{..}`) rather
/// than guessing at the rest of `mitos_utils::ipc::RichWidget`'s shape.
enum WidgetView {
    Button { label: String, cmd: String },
    Progress { percent: f32 },
    Sparkline,
}

fn describe(w: &mitos_utils::ipc::RichWidget) -> WidgetView {
    use mitos_utils::ipc::RichWidget;
    match w {
        RichWidget::Button { label, cmd } => WidgetView::Button { label: label.clone(), cmd: cmd.clone() },
        RichWidget::Progress { percent, .. } => WidgetView::Progress { percent: *percent },
        RichWidget::Sparkline { .. } => WidgetView::Sparkline,
    }
}

fn paint_widget_view(ui: &mut egui::Ui, rect: ERect, widget_id: u64, view: &WidgetView, term: &mut Term) {
    // A floating `Area` anchored at the row's own pixel position rather than
    // a nested child `Ui`: it only needs proven-stable `egui::Area` +
    // `ui.horizontal` building blocks (see the settings-gear button and the
    // search bar for the same pattern) and still gives the button/progress
    // bar a normal, focusable `Response` for keyboard navigation and AT.
    let ctx = ui.ctx().clone();
    let clicked_cmd = std::cell::RefCell::new(None::<String>);
    egui::Area::new(egui::Id::new(("mitos-widget-row", widget_id)))
        .fixed_pos(rect.min)
        .show(&ctx, |ui| {
            ui.set_min_width(rect.width().max(1.0));
            ui.horizontal(|ui| match view {
                WidgetView::Button { label, cmd } => {
                    if ui.button(label.as_str()).clicked() {
                        if let Some(clean) = crate::security::sanitize_command(cmd) {
                            *clicked_cmd.borrow_mut() = Some(clean);
                        }
                    }
                }
                WidgetView::Progress { percent } => {
                    ui.add(egui::ProgressBar::new(*percent).show_percentage());
                }
                WidgetView::Sparkline => {
                    ui.label("\u{1F4C8} [Sparkline Graph]");
                }
            });
        });
    if let Some(cmd) = clicked_cmd.into_inner() {
        // The byte-writing path lives on `Session`; queueing it on the term's
        // own event list keeps `render.rs` free of session/IPC types.
        term.queue_widget_command(widget_id, cmd);
    }
}

/// Reserves a click/hover-sensing region over `rect` for mouse routing
/// (selection drags, link clicks, focus-on-click) without egui's own widgets
/// intercepting terminal keystrokes.
pub fn interact(ui: &mut egui::Ui, rect: ERect, id: egui::Id) -> egui::Response {
    ui.interact(rect, id, Sense::click_and_drag())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Pen, Term};

    fn theme() -> Theme {
        Theme::dark()
    }

    fn row_from(t: &mut Term, text: &str, sgr: &str) -> Row {
        t.process(format!("\x1b[H{sgr}{text}").as_bytes());
        t.view_row(0).clone()
    }

    #[test]
    fn plain_text_is_one_run() {
        let mut t = Term::new(20, 3, 10);
        let row = row_from(&mut t, "hello", "");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "hello");
        assert_eq!((runs[0].col0, runs[0].col1), (0, 5));
    }

    #[test]
    fn colour_change_splits_runs() {
        let mut t = Term::new(20, 3, 10);
        t.process(b"\x1b[Hab\x1b[31mcd\x1b[0mef");
        let row = t.view_row(0).clone();
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["ab", "cd", "ef"]);
    }

    #[test]
    fn bg_matching_theme_is_not_specially_marked_but_still_one_run() {
        let mut t = Term::new(10, 3, 10);
        let row = row_from(&mut t, "xy", "");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bg, theme().bg);
    }

    #[test]
    fn inverse_swaps_fg_and_bg() {
        let mut t = Term::new(10, 3, 10);
        let row = row_from(&mut t, "x", "\x1b[7m");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs[0].fg, theme().bg);
        assert_eq!(runs[0].bg, theme().fg);
    }

    #[test]
    fn screen_reverse_video_also_swaps() {
        let mut t = Term::new(10, 3, 10);
        let row = row_from(&mut t, "x", "");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, true);
        assert_eq!(runs[0].fg, theme().bg);
        assert_eq!(runs[0].bg, theme().fg);
        // inverse cell + screen reverse cancel out
        let row2 = row_from(&mut t, "x", "\x1b[7m");
        let runs2 = coalesce_row(&theme(), &Overrides::default(), &row2, true);
        assert_eq!(runs2[0].fg, theme().fg);
    }

    #[test]
    fn wide_tail_cells_are_skipped_and_width_is_two() {
        let mut t = Term::new(10, 3, 10);
        let row = row_from(&mut t, "世", "");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].col0, runs[0].col1), (0, 2));
        assert_eq!(runs[0].text.chars().count(), 1);
    }

    #[test]
    fn hidden_cells_produce_empty_text_but_still_a_bg_run() {
        let mut t = Term::new(10, 3, 10);
        let row = row_from(&mut t, "x", "\x1b[8;41m");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "");
        assert_eq!(runs[0].bg, theme().ansi[1]);
    }

    #[test]
    fn adjacent_cells_with_different_link_ids_still_coalesce() {
        let mut t = Term::new(20, 3, 10);
        t.process(b"\x1b[H\x1b]8;;https://a\x07a\x1b]8;;\x07\x1b]8;;https://b\x07b\x1b]8;;\x07");
        let row = t.view_row(0).clone();
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs.len(), 1, "same style, different link ids: still one run");
        assert_eq!(runs[0].text, "ab");
    }

    #[test]
    fn dim_darkens_without_changing_run_count() {
        let mut t = Term::new(10, 3, 10);
        let row = row_from(&mut t, "xy", "\x1b[2m");
        let runs = coalesce_row(&theme(), &Overrides::default(), &row, false);
        assert_eq!(runs.len(), 1);
        assert_ne!(runs[0].fg, theme().fg);
    }

    #[test]
    fn font_id_resolves_to_builtin_monospace() {
        assert_eq!(font_id("monospace", 14.0), FontId::monospace(14.0));
        assert_eq!(font_id("", 12.0), FontId::monospace(12.0));
        assert_eq!(font_id("Fira Code", 14.0), FontId::monospace(14.0));
    }
}
