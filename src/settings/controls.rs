//! Immediate-mode controls for the settings pages: cards, toggles, segments, steppers, buttons.

use super::*;

pub(super) struct Ui<'a, 'b> {
    pub(super) cv: &'a mut Canvas<'b>,
    pub(super) s: &'a mut SettingsWindow,
    pub(super) y: f32,
    pub(super) x: f32,
    pub(super) w: f32,
    pub(super) row_index: usize,
    pub(super) moving: bool,
    pub(super) dt: f32,
    /// Corner roundness from Appearance, applied to cards and controls.
    pub(super) round: f32,
}

pub(super) const ROW: f32 = 48.0;
pub(super) const ROW_SUB: f32 = 56.0;
/// A row with a label above a button group, with room between them.
pub(super) const ROW_SEG: f32 = 96.0;
/// Line height of release notes on the "What's new" page.
pub(super) const NOTE_LINE: f32 = 19.0;

impl Ui<'_, '_> {
    pub(super) fn accent(cfg: &Config) -> C {
        C::hex(cfg.accent_rgb())
    }

    pub(super) fn title(&mut self, t: &str) {
        self.cv.display(t, R::new(self.x, self.y, self.w, 34.0), 22.0, 700, C::WHITE, Align::Left);
        self.y += 46.0;
    }

    pub(super) fn section(&mut self, label: &str) {
        self.cv.text(label, R::new(self.x + 4.0, self.y, self.w, 18.0), 12.0, 600, DIM, Align::Left);
        self.y += 22.0;
    }

    /// Draws a card of `rows` heights and returns their rects.
    pub(super) fn card(&mut self, heights: &[f32]) -> Vec<R> {
        let total: f32 = heights.iter().sum();
        let r = R::new(self.x, self.y, self.w, total);
        self.cv.fill_rrect(r, 12.0 * self.round, CARD);
        let mut rows = Vec::new();
        let mut y = self.y;
        for (i, h) in heights.iter().enumerate() {
            if i > 0 {
                self.cv.line(self.x + 16.0, y, self.x + self.w - 16.0, y, LINE, 1.0);
            }
            rows.push(R::new(self.x, y, self.w, *h));
            y += h;
        }
        self.y = y + 18.0;
        rows
    }

    pub(super) fn label(&mut self, r: R, label: &str, sub: Option<&str>) {
        match sub {
            Some(sub) => {
                self.cv.text(label, R::new(r.x + 16.0, r.y + 9.0, r.w * 0.62, 20.0), 13.5, 550, C::WHITE, Align::Left);
                self.cv.text(sub, R::new(r.x + 16.0, r.y + 29.0, r.w * 0.62, 17.0), 11.5, 500, DIM, Align::Left);
            }
            None => self.cv.text(label, R::new(r.x + 16.0, r.y, r.w * 0.62, r.h), 13.5, 550, C::WHITE, Align::Left),
        }
    }

    pub(super) fn toggle(&mut self, cfg: &Config, r: R, label: &str, sub: Option<&str>, f: BoolField) {
        self.label(r, label, sub);
        let on = *f(&mut cfg.clone());
        let id = f as usize;
        let knob = self.s.knobs.entry(id).or_insert_with(|| Spring::new(if on { 1.0 } else { 0.0 }, 0.28, 0.72));
        knob.set(if on { 1.0 } else { 0.0 });
        self.moving |= knob.step(self.dt);
        let t = knob.value;
        let sw = R::new(r.right() - 16.0 - 44.0, r.cy() - 12.0, 44.0, 24.0);
        let act = Act::Toggle(f);
        self.s.hits.push((r, act));
        let off = C::WHITE.a(if hovered(self.s, act) { 0.22 } else { 0.16 });
        self.cv.fill_rrect(sw, 12.0, off.mix(Self::accent(cfg), t.clamp(0.0, 1.0)));
        let kx = sw.x + 12.0 + t * 20.0;
        let stretch = if self.s.pressed == Some(act) { 4.0 } else { 0.0 };
        self.cv.fill_rrect(R::new(kx - 9.0 - stretch * (1.0 - t), sw.cy() - 9.0, 18.0 + stretch, 18.0), 9.0, C::WHITE);
    }

    pub(super) fn segmented(&mut self, cfg: &Config, r: R, labels: &[&str], f: IndexField) {
        let sel = *f(&mut cfg.clone());
        let id = f as usize;
        let spring = self.s.segs.entry(id).or_insert_with(|| Spring::new(sel as f32, 0.3, 0.82));
        spring.set(sel as f32);
        self.moving |= spring.step(self.dt);
        let pos = spring.value;
        let bar = R::new(r.x, r.y, r.w, 30.0);
        self.cv.fill_rrect(bar, 8.0 * self.round, C::WHITE.a(0.06));
        let seg_w = bar.w / labels.len() as f32;
        self.cv.fill_rrect(R::new(bar.x + 2.0 + pos * seg_w, bar.y + 2.0, seg_w - 4.0, bar.h - 4.0), 6.5 * self.round, C::WHITE.a(0.16));
        for (i, l) in labels.iter().enumerate() {
            let sr = R::new(bar.x + i as f32 * seg_w, bar.y, seg_w, bar.h);
            let act = Act::Seg(f, i);
            self.s.hits.push((sr, act));
            if hovered(self.s, act) && i != sel {
                self.cv.fill_rrect(sr.inset(2.0, 2.0), 6.5 * self.round, C::WHITE.a(0.05));
            }
            self.cv.text(l, sr, 12.0, if i == sel { 650 } else { 500 }, if i == sel { C::WHITE } else { DIM }, Align::Center);
        }
    }

    pub(super) fn seg_row(&mut self, cfg: &Config, r: R, label: &str, sub: Option<&str>, labels: &[&str], f: IndexField) {
        let lr = R::new(r.x, r.y, r.w, if sub.is_some() { 50.0 } else { 38.0 });
        self.label(lr.offset(0.0, if sub.is_some() { 0.0 } else { 4.0 }), label, sub);
        self.segmented(cfg, R::new(r.x + 16.0, r.bottom() - 42.0, r.w - 32.0, 30.0), labels, f);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn stepper(&mut self, cfg: &Config, r: R, label: &str, sub: Option<&str>, f: NumField, step: i32, lo: u32, hi: u32, fmt: &dyn Fn(u32) -> String) {
        self.label(r, label, sub);
        let v = *f(&mut cfg.clone());
        let plus = R::centered(r.right() - 32.0, r.cy(), 28.0, 28.0);
        let value_w = 92.0;
        let value = R::new(plus.x - 4.0 - value_w, r.cy() - 14.0, value_w, 28.0);
        let minus = R::centered(value.x - 18.0, r.cy(), 28.0, 28.0);
        for (rect, d, g) in [(minus, -step, '\u{E738}'), (plus, step, icon::ADD)] {
            let act = Act::Step(f, d, lo, hi);
            self.s.hits.push((rect, act));
            let disabled = (d < 0 && v <= lo) || (d > 0 && v >= hi);
            self.cv.fill_circle(rect.cx(), rect.cy(), 14.0, C::WHITE.a(if hovered(self.s, act) && !disabled { 0.16 } else { 0.08 }));
            self.cv.glyph(g, rect.cx(), rect.cy(), 10.0, if disabled { C::WHITE.a(0.25) } else { C::WHITE });
        }
        self.cv.text(&fmt(v), value, 13.0, 650, C::WHITE, Align::Center);
    }

    pub(super) fn button(&mut self, r: R, label: &str, sub: Option<&str>, text: &str, act: Act, danger: bool) {
        self.label(r, label, sub);
        let bw = self.cv.gfx.text_width(text, 12.5, 600) + 30.0;
        let b = R::new(r.right() - 16.0 - bw, r.cy() - 14.0, bw, 28.0);
        self.s.hits.push((b, act));
        let base = if danger { C::hex(0xFF453A).a(0.2) } else { C::WHITE.a(0.1) };
        self.cv.fill_rrect(b, 14.0, if hovered(self.s, act) { base.mix(C::WHITE, 0.08) } else { base });
        self.cv.text(text, b, 12.5, 600, if danger { C::hex(0xFF8A80) } else { C::WHITE }, Align::Center);
    }
}
