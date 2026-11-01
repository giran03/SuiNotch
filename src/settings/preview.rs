//! The live preview at the top of the lyric look settings: a miniature notch drawn by the same
//! code as the real one, replaying a sample line so every choice can be seen as it is made.

use super::*;
use crate::lyric_draw::{self, Frame};
use crate::lyric_style::Enter;

/// Height of the preview's row.
pub(super) const ROW_PREVIEW: f32 = 120.0;
const NOTCH_W: f32 = 360.0;
const NOTCH_H: f32 = 28.0;
const SAMPLE: &str = "A line of the song, as it is sung";
/// What the second line shows for each `Second line` choice; the first is unused (Off).
const SAMPLE_SECOND: [&str; 5] = ["", "The next line of the song", "A romanised line", "A translated line", "A translated line"];
/// The sample replays over this many seconds.
const LOOP_S: f32 = 4.0;
/// How long a new line takes to arrive in the preview.
const ENTER_S: f32 = 0.9;

impl Ui<'_, '_> {
    pub(super) fn lyric_preview(&mut self, c: &Config, r: R) {
        let look = c.lyric_look();
        let stage = R::new(r.x + 16.0, r.y + 10.0, r.w - 32.0, r.h - 20.0);
        self.cv.fill_rrect(stage, 10.0 * self.round, BG);
        self.cv.push_clip_rect(stage);
        let accent = Self::accent(c);
        let t = lyric_draw::clock_secs() % LOOP_S;
        let fade = if look.enter == Enter::Instant { 1.0 } else { (t / ENTER_S).clamp(0.0, 1.0) };
        let second = (c.lyrics_second > 0).then(|| SAMPLE_SECOND[c.lyrics_second.min(SAMPLE_SECOND.len() - 1)]);
        let frame = Frame {
            text: SAMPLE,
            second,
            look,
            fade,
            sung: ((t - 0.4) / (LOOP_S - 1.2)).clamp(0.0, 1.0),
            secs: t,
            art: None,
            accent,
            glass: c.glass,
        };
        // A notch hanging from the top of the stage, with a stand-in cover and the song title.
        let body = R::new(stage.cx() - NOTCH_W / 2.0, stage.y, NOTCH_W, NOTCH_H);
        self.cv.fill_rrect(R::new(body.x, body.y - 16.0, body.w, body.h + 16.0), 13.0, C::BLACK);
        self.cv.fill_rrect(R::centered(body.x + 20.0, body.cy(), 14.0, 14.0), 4.0, accent.a(0.8));
        let slot = R::new(body.x + 38.0, body.y, body.w - 38.0 - 40.0, body.h);
        if look.below_notch() {
            self.cv.text("Song title", slot, 12.5, 600, C::WHITE, Align::Left);
            let h = look.pill_height(second.is_some());
            let widest = second.map_or(0.0, |s| self.cv.gfx.text_width(s, look.second_font(), 500)).max(self.cv.gfx.text_width(SAMPLE, look.font(), 600));
            let w = (widest + 34.0).min(stage.w - 16.0);
            lyric_draw::draw_under(self.cv, R::centered(stage.cx(), body.bottom() + 8.0 + h / 2.0, w, h), &frame);
        } else {
            lyric_draw::draw_in_notch(self.cv, slot, &frame);
        }
        self.cv.pop();
        // The sample replays, so the page keeps drawing while it is on screen.
        self.moving = true;
    }
}
