//! The live preview at the top of the lyric look settings: a miniature notch drawn by the same
//! code as the real one, replaying a sample line so every choice can be seen as it is made.

use super::*;
use crate::lyric_draw::{self, Fill, Frame};
use crate::lyric_style::Enter;
use crate::words;

/// Height of the preview's row.
pub(super) const ROW_PREVIEW: f32 = 120.0;
const NOTCH_W: f32 = 360.0;
const NOTCH_H: f32 = 28.0;
const SAMPLE: &str = "A line of the song, as it is sung";
/// What the second line shows for each `Second line` choice; the first is unused (Off).
const SAMPLE_SECOND: [&str; 5] = ["", "The next line of the song", "A romanised line", "A translated line", "A translated line"];
/// The sample replays over this many seconds.
const LOOP_S: f32 = 4.0;
/// When the sample line starts to be sung, and how long it takes, in the preview.
const SING_START_MS: u64 = 400;
const SING_MS: u64 = 2800;
/// How long a new line takes to arrive in the preview.
const ENTER_S: f32 = 0.9;

/// Push the preview out with the end of Second line, before Now playing enters.
pub(super) fn section_y(normal_y: f32, lyrics_end: f32, height: f32) -> f32 {
    normal_y.max(TITLE_H).min(lyrics_end - height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_stays_in_flow_until_it_reaches_the_top() {
        assert_eq!(section_y(200.0, 1400.0, 160.0), 200.0);
        assert_eq!(section_y(-100.0, 1000.0, 160.0), TITLE_H);
        assert_eq!(section_y(-200.0, 900.0, 160.0), TITLE_H);
    }

    #[test]
    fn preview_leaves_before_now_playing() {
        assert_eq!(section_y(-1000.0, TITLE_H + 80.0, 160.0), TITLE_H - 80.0);
        let end = TITLE_H - 8.0;
        assert_eq!(section_y(-1200.0, end, 160.0) + 160.0, end);
    }

    #[test]
    fn preview_returns_and_stays_pinned_when_scrolling_up() {
        let upward_positions = [(-1200.0, 20.0), (-1000.0, 220.0), (-800.0, 420.0), (-100.0, 1120.0)];
        let positions: Vec<f32> = upward_positions.iter()
            .map(|&(normal, end)| section_y(normal, end, 160.0))
            .collect();
        assert_eq!(positions, vec![-140.0, TITLE_H, TITLE_H, TITLE_H]);
        assert_eq!(section_y(60.0, 1280.0, 160.0), 60.0);
    }
}

impl Ui<'_, '_> {
    pub(super) fn lyric_preview_section(&mut self, c: &Config, r: R) {
        // Cover the padding up to the page's clip edge too, so scrolling text
        // cannot peek through the strip above the pinned section label.
        let top = (r.y - 8.0).max(TITLE_H - 8.0);
        if r.bottom() <= top || r.y >= H {
            return;
        }
        // The backdrop spans the content column, including the card margins.
        self.cv.fill_rect(R::new(SIDEBAR + 1.0, top, W - SIDEBAR - 1.0, r.bottom() - top), BG);
        self.cv.text("Lyrics preview", R::new(r.x + 4.0, r.y, r.w, 18.0), 12.0, 600, DIM, Align::Left);
        let card = R::new(r.x, r.y + 22.0, r.w, ROW_PREVIEW);
        self.cv.fill_rrect(card, 12.0 * self.round, CARD);
        self.lyric_preview(c, card);
        // Clip covered hit areas as well as painting over them. Partly exposed
        // controls remain usable below the preview, with no clicks through it.
        self.s.hits.retain_mut(|(hit, _)| {
            if hit.right() <= SIDEBAR || hit.bottom() <= top || hit.y >= r.bottom() {
                return true;
            }
            let bottom = hit.bottom();
            hit.y = r.bottom();
            hit.h = (bottom - hit.y).max(0.0);
            hit.h > 0.0
        });
        self.s.hover = self.s.hits.iter().rev()
            .find(|(hit, _)| hit.contains(self.s.mouse.0, self.s.mouse.1))
            .map(|(_, act)| *act);
    }

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
            fill: if look.words {
                // Words are timed as an estimate, the way a song without word timing is shown.
                let words = words::estimate(SAMPLE, SING_START_MS, SING_MS);
                Fill::Chars(words::sung_chars(&words, (t * 1000.0) as u64))
            } else {
                Fill::Even(((t - 0.4) / (LOOP_S - 1.2)).clamp(0.0, 1.0))
            },
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
        if look.in_notch() {
            lyric_draw::draw_in_notch(self.cv, slot, &frame);
        } else {
            self.cv.text("Song title", slot, 12.5, 600, C::WHITE, Align::Left);
        }
        if look.below_notch() {
            let h = look.pill_height(second.is_some());
            let widest = second.map_or(0.0, |s| self.cv.gfx.text_width(s, look.second_font(), 500)).max(self.cv.gfx.text_width(SAMPLE, look.font(), 600));
            let w = (widest + 34.0).min(stage.w - 16.0);
            lyric_draw::draw_under(self.cv, R::centered(stage.cx(), body.bottom() + 8.0 + h / 2.0, w, h), &frame);
        }
        self.cv.pop();
        // The sample replays, so the page keeps drawing while it is on screen.
        self.moving = true;
    }
}
