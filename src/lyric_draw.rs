//! Draws the current lyric line, under the notch or inside it, for a `Look`. The notch and the
//! settings preview both call this, so what the preview shows is what the notch does.

use std::sync::OnceLock;
use std::time::Instant;

use crate::anim::lerp;
use crate::gfx::{Align, Canvas, C, R};
use crate::lyric_style::{self, Backdrop, Enter, Look, TextColor};

const FIRST_WEIGHT: i32 = 600;
const SECOND_WEIGHT: i32 = 500;
/// Line height as a multiple of the font size, so a second line sits snugly under the first.
const LINE_HEIGHT: f32 = 1.45;
const SIDE_PAD: f32 = 14.0;
/// Space between a lyric inside the notch and the second text after it.
pub const SUFFIX_GAP: f32 = 10.0;
/// How far a new line slides up as it fades in.
const SLIDE: f32 = 5.0;
/// How much dimmer the second line is than the first.
const SECOND_ALPHA: f32 = 0.55;

/// Seconds since lyrics were first drawn; the clock a scrolling line is timed by.
pub fn clock_secs() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// How much of a line has been sung, for the karaoke fill.
#[derive(Clone, Copy, Debug)]
pub enum Fill {
    /// This fraction (0 to 1) of the line's width, spread evenly.
    Even(f32),
    /// This many characters of the line, with a fraction for the one being sung.
    Chars(f32),
}

/// One lyric line, as it is to be drawn this frame.
pub struct Frame<'a> {
    pub text: &'a str,
    pub second: Option<&'a str>,
    pub look: Look,
    /// How far the line has entered, 0 to 1.
    pub fade: f32,
    pub fill: Fill,
    /// Seconds since the line appeared, for scrolling.
    pub secs: f32,
    /// The album's colour, when there is a cover, and the accent.
    pub art: Option<C>,
    pub accent: C,
    /// A translucent black pill rather than a solid one.
    pub glass: bool,
}

impl Frame<'_> {
    fn text_color(&self) -> C {
        match self.look.color {
            TextColor::Album => self.art.map(|c| c.mix(C::WHITE, 0.55)).unwrap_or(C::WHITE),
            TextColor::White => C::WHITE,
            TextColor::Accent => self.accent.mix(C::WHITE, 0.35),
        }
    }

    fn pill_color(&self) -> C {
        match self.look.backdrop {
            Backdrop::Tint => self.art.unwrap_or(self.accent).mix(C::BLACK, 0.78).a(0.96),
            _ if self.glass => C(0.05, 0.055, 0.07, 0.9),
            _ => C::BLACK,
        }
    }

    fn alpha(&self) -> f32 {
        match self.look.enter {
            Enter::Slide | Enter::Fade => self.fade.clamp(0.0, 1.0),
            Enter::Typewriter | Enter::Instant => 1.0,
        }
    }

    fn slide(&self) -> f32 {
        if self.look.enter == Enter::Slide {
            (1.0 - self.fade.clamp(0.0, 1.0)) * SLIDE
        } else {
            0.0
        }
    }
}

/// How one run of text is set.
struct Run {
    size: f32,
    weight: i32,
    color: C,
    /// Left-aligned in its rectangle rather than centred.
    left: bool,
    /// The karaoke fill, when this run has one.
    sung: Option<Fill>,
    /// A soft shadow, for text with no pill behind it.
    shadow: bool,
    /// Extra transparency on top of the frame's.
    alpha: f32,
}

/// The font size and the width a run takes once shrunk to fit `avail`, if the look shrinks.
fn fitted(cv: &Canvas, f: &Frame, text: &str, run: &Run, avail: f32) -> (f32, f32) {
    let full = cv.gfx.text_width(text, run.size, run.weight);
    let scale = lyric_style::shrink_scale(full, avail, f.look.fit);
    (run.size * scale, full * scale)
}

fn draw_run(cv: &mut Canvas, f: &Frame, rect: R, text: &str, run: &Run) {
    let (size, tw) = fitted(cv, f, text, run, rect.w);
    let scrolling = f.look.fit == lyric_style::Fit::Scroll && tw > rect.w + 0.5;
    let visible = tw.min(rect.w);
    let x0 = if scrolling {
        rect.x - lyric_style::scroll_offset(tw - rect.w, f.secs)
    } else if run.left {
        rect.x
    } else {
        rect.cx() - visible / 2.0
    };
    let width = if scrolling { tw + 8.0 } else { visible + if tw <= rect.w { 8.0 } else { 0.0 } };
    let shown = if f.look.enter == Enter::Typewriter { lyric_style::typed(text, f.fade) } else { text };
    let at = R::new(x0, rect.y + f.slide(), width, rect.h);
    let a = f.alpha() * run.alpha;
    let band = R::new(rect.x, rect.y - 4.0, rect.w, rect.h + 8.0);
    if scrolling {
        cv.push_clip_rect(band);
    }
    if run.shadow {
        // No pill to read against, so the text carries its own soft shadow.
        for (dx, dy) in [(0.0, 2.0), (-1.0, 1.0), (1.0, 1.0)] {
            cv.text(shown, at.offset(dx, dy), size, run.weight, C::BLACK.a(0.45 * a), Align::Left);
        }
    }
    match run.sung {
        Some(sung) => {
            // The line sits dim, and a bright copy is revealed from the left as it is sung.
            cv.text(shown, at, size, run.weight, C::WHITE.a(0.4 * a), Align::Left);
            let whole = if scrolling { tw } else { visible };
            let reach = match sung {
                Fill::Even(f) => whole * f.clamp(0.0, 1.0),
                Fill::Chars(n) => chars_width(cv, text, n, size, run.weight).min(whole),
            };
            cv.push_clip_rect(R::new(x0 - 2.0, band.y, reach + 2.0, band.h));
            cv.text(shown, at, size, run.weight, run.color.a(a), Align::Left);
            cv.pop();
        }
        None => cv.text(shown, at, size, run.weight, run.color.a(a), Align::Left),
    }
    if scrolling {
        cv.pop();
    }
}

/// The width of the first `n` characters of `text` (a fraction of one counting part of its width), at `size`.
fn chars_width(cv: &Canvas, text: &str, n: f32, size: f32, weight: i32) -> f32 {
    let whole = n.max(0.0).floor() as usize;
    let upto = |k: usize| cv.gfx.text_width(lyric_style::first_chars(text, k), size, weight);
    let before = if whole == 0 { 0.0 } else { upto(whole) };
    if whole >= text.chars().count() {
        return before;
    }
    before + (n - whole as f32) * (upto(whole + 1) - before)
}

/// A left-aligned line of text with `sung` characters of it filled in `color` over a dim copy: the
/// word highlight on the lyrics panel's current line.
pub fn draw_filled(cv: &mut Canvas, rect: R, text: &str, size: f32, weight: i32, color: C, sung: f32) {
    let reach = chars_width(cv, text, sung, size, weight).min(rect.w);
    cv.text(text, rect, size, weight, C::WHITE.a(0.4), Align::Left);
    cv.push_clip_rect(R::new(rect.x - 2.0, rect.y - 4.0, reach + 2.0, rect.h + 8.0));
    cv.text(text, rect, size, weight, color, Align::Left);
    cv.pop();
}

fn first_run(f: &Frame, shadow: bool, left: bool) -> Run {
    Run {
        size: f.look.font(),
        weight: FIRST_WEIGHT,
        color: f.text_color(),
        left,
        sung: f.look.karaoke.then_some(f.fill),
        shadow,
        alpha: 1.0,
    }
}

fn second_run(f: &Frame, shadow: bool, left: bool, alpha: f32) -> Run {
    Run { size: f.look.second_font(), weight: SECOND_WEIGHT, color: C::WHITE, left, sung: None, shadow, alpha: SECOND_ALPHA * alpha }
}

/// The line in a pill (or on its own) at `r`. A second line is drawn when there is one and the
/// rectangle is tall enough for it, easing in as the pill grows.
pub fn draw_under(cv: &mut Canvas, r: R, f: &Frame) {
    let look = f.look;
    if look.has_pill() {
        cv.fill_rrect(r, (r.h / 2.0).min(16.0), f.pill_color());
    }
    let shadow = look.backdrop == Backdrop::None;
    let (one, two) = (look.one_line_height(), look.two_line_height());
    let grown = ((r.h - one) / (two - one)).clamp(0.0, 1.0);
    let (first_h, second_h) = (look.font() * LINE_HEIGHT, look.second_font() * LINE_HEIGHT);
    let first_y = lerp(r.cy() - first_h / 2.0, r.y + (r.h - first_h - second_h) / 2.0, grown);
    let first = R::new(r.x + SIDE_PAD, first_y, r.w - 2.0 * SIDE_PAD, first_h);
    draw_run(cv, f, first, f.text, &first_run(f, shadow, false));
    if let (Some(text), true) = (f.second, grown > 0.02) {
        let under = R::new(first.x, first.y + first_h - 1.0, first.w, second_h);
        draw_run(cv, f, under, text, &second_run(f, shadow, false, grown));
    }
}

/// The line in the closed notch, left-aligned in the space the song title takes; the second text,
/// when it fits after it, follows in a dimmer colour.
pub fn draw_in_notch(cv: &mut Canvas, r: R, f: &Frame) {
    let first = first_run(f, false, true);
    draw_run(cv, f, r, f.text, &first);
    let Some(second) = f.second else { return };
    let (_, tw) = fitted(cv, f, f.text, &first, r.w);
    let rest = r.w - tw - SUFFIX_GAP;
    let run = second_run(f, false, true, 1.0);
    let (_, sw) = fitted(cv, f, second, &run, f32::MAX);
    if sw <= rest {
        draw_run(cv, f, R::new(r.x + tw + SUFFIX_GAP, r.y, rest, r.h), second, &run);
    }
}
