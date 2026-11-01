//! How the current lyric line is shown while the notch is closed. The look is a handful of
//! independent choices (`Look`) that combine freely; the old styles survive as presets that set them.
//! The pure maths behind them lives here; the drawing is in `lyric_draw`.

use crate::config::Config;
use crate::lrc::Line;

pub const PRESET_LABELS: [&str; 6] = ["Pill", "In the notch", "Text only", "Karaoke", "Glow", "Custom"];
/// The preset index that stands for "the choices match no preset".
pub const CUSTOM_PRESET: usize = 5;
pub const PLACE_LABELS: [&str; 2] = ["Under the notch", "In the notch"];
pub const BACKDROP_LABELS: [&str; 3] = ["Black pill", "Album tint", "None"];
pub const HIGHLIGHT_LABELS: [&str; 2] = ["None", "Karaoke fill"];
pub const COLOR_LABELS: [&str; 3] = ["Album tint", "White", "Accent"];
pub const SIZE_LABELS: [&str; 3] = ["Small", "Medium", "Large"];
pub const ENTER_LABELS: [&str; 4] = ["Slide", "Fade", "Typewriter", "None"];
pub const FIT_LABELS: [&str; 3] = ["Shrink", "Scroll", "Cut"];
pub const PAUSED_LABELS: [&str; 2] = ["Hide", "Keep dimmed"];
pub const SECOND_LABELS: [&str; 5] = ["Off", "Next line", "Romaji", "Translation", "Auto"];
pub const PANEL_ALT_LABELS: [&str; 3] = ["Off", "Romaji", "Translation"];

/// Space between the notch and the pill, and below the pill, so the window has some air around it.
pub const ROOM_PAD: f32 = 14.0;
/// Most room the lyrics ever need under the notch (a large pill with two lines); the surface is sized for it.
pub const ROOM_MAX: f32 = 50.0 + ROOM_PAD;
/// Widest a line is drawn, in dips, before it is shrunk, scrolled or cut.
pub const MAX_LINE_W: f32 = 560.0;
/// A last line, with no next line to measure by, fills over this long.
const LAST_LINE_MS: u64 = 5000;
/// Time a line takes to sing per character, for the karaoke fill; it stops there rather than creeping through a break.
const SING_MS_PER_CHAR: u64 = 65;
const MIN_SING_MS: u64 = 1200;
/// How far a line may shrink before it is cut instead.
const MIN_SHRINK: f32 = 0.7;
/// How fast a long line scrolls, in dips per second, and how long it rests at each end.
const SCROLL_SPEED: f32 = 40.0;
const SCROLL_HOLD_S: f32 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A line under the notch.
    Under,
    /// The line takes the song title's place inside the closed notch.
    InNotch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    /// A black pill.
    Pill,
    /// A pill tinted with the colours of the album cover.
    Tint,
    /// The text alone, with a soft shadow.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColor {
    Album,
    White,
    Accent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enter {
    Slide,
    Fade,
    Typewriter,
    Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Shrink,
    Scroll,
    Cut,
}

/// Every choice about how the current lyric line looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub place: Place,
    pub backdrop: Backdrop,
    pub karaoke: bool,
    pub color: TextColor,
    /// Index into `SIZE_LABELS`.
    pub size: usize,
    pub enter: Enter,
    pub fit: Fit,
    /// The line stays, dimmed, while the music is paused.
    pub keep_paused: bool,
}

/// Font size of the line, its pill height with one line and with two, for each size.
const SIZES: [(f32, f32, f32); 3] = [(11.5, 22.0, 38.0), (12.5, 26.0, 44.0), (14.0, 30.0, 50.0)];

impl Look {
    pub fn font(&self) -> f32 {
        SIZES[self.size.min(2)].0
    }

    /// The second line is set a little smaller than the first.
    pub fn second_font(&self) -> f32 {
        self.font() - 1.5
    }

    pub fn one_line_height(&self) -> f32 {
        SIZES[self.size.min(2)].1
    }

    pub fn two_line_height(&self) -> f32 {
        SIZES[self.size.min(2)].2
    }

    /// Height of the pill (or of the text block) under the notch.
    pub fn pill_height(&self, second_line: bool) -> f32 {
        if second_line {
            self.two_line_height()
        } else {
            self.one_line_height()
        }
    }

    pub fn below_notch(&self) -> bool {
        self.place == Place::Under
    }

    /// Whether the pill shape is drawn behind the text.
    pub fn has_pill(&self) -> bool {
        self.below_notch() && self.backdrop != Backdrop::None
    }

    /// Room the lyrics need under the notch when the pill is `pill_h` tall.
    pub fn room_below(&self, pill_h: f32) -> f32 {
        if self.below_notch() {
            pill_h + ROOM_PAD
        } else {
            0.0
        }
    }
}

impl Config {
    pub fn lyric_look(&self) -> Look {
        Look {
            place: if self.lyrics_place == 1 { Place::InNotch } else { Place::Under },
            backdrop: [Backdrop::Pill, Backdrop::Tint, Backdrop::None][self.lyrics_backdrop.min(2)],
            karaoke: self.lyrics_highlight == 1,
            color: [TextColor::Album, TextColor::White, TextColor::Accent][self.lyrics_color.min(2)],
            size: self.lyrics_size.min(2),
            enter: [Enter::Slide, Enter::Fade, Enter::Typewriter, Enter::Instant][self.lyrics_enter.min(3)],
            fit: [Fit::Shrink, Fit::Scroll, Fit::Cut][self.lyrics_fit.min(2)],
            keep_paused: self.lyrics_paused == 1,
        }
    }

    /// Romaji or a translation is shown somewhere, so lyrics from a service without them should get them.
    pub fn wants_lyric_extras(&self) -> bool {
        (2..=4).contains(&self.lyrics_second) || self.lyrics_panel_alt > 0
    }
}

/// (place, backdrop, highlight, colour) of each preset, in `PRESET_LABELS` order.
const PRESETS: [(usize, usize, usize, usize); 5] = [(0, 0, 0, 0), (1, 0, 0, 0), (0, 2, 0, 0), (0, 0, 1, 0), (0, 1, 0, 1)];

/// The preset the choices add up to, or `CUSTOM_PRESET`.
pub fn preset_of(c: &Config) -> usize {
    let now = (c.lyrics_place, c.lyrics_backdrop, c.lyrics_highlight, c.lyrics_color);
    PRESETS.iter().position(|p| *p == now).unwrap_or(CUSTOM_PRESET)
}

/// Sets the choices a preset stands for; "Custom" leaves them as they are.
pub fn apply_preset(c: &mut Config) {
    if let Some(&(place, backdrop, highlight, color)) = PRESETS.get(c.lyrics_preset) {
        (c.lyrics_place, c.lyrics_backdrop, c.lyrics_highlight, c.lyrics_color) = (place, backdrop, highlight, color);
    }
}

/// Carries a config from before the look was split into choices over: it held one style index, which
/// is applied as a preset once.
pub fn migrate(c: &mut Config) {
    if let Some(old) = c.lyrics_style.take() {
        c.lyrics_preset = old;
        apply_preset(c);
    }
}

/// Keeps every choice inside what the settings offer and the preset label in step with them.
pub fn sanitize(c: &mut Config) {
    migrate(c);
    c.lyrics_place = c.lyrics_place.min(PLACE_LABELS.len() - 1);
    c.lyrics_backdrop = c.lyrics_backdrop.min(BACKDROP_LABELS.len() - 1);
    c.lyrics_highlight = c.lyrics_highlight.min(HIGHLIGHT_LABELS.len() - 1);
    c.lyrics_color = c.lyrics_color.min(COLOR_LABELS.len() - 1);
    c.lyrics_size = c.lyrics_size.min(SIZE_LABELS.len() - 1);
    c.lyrics_enter = c.lyrics_enter.min(ENTER_LABELS.len() - 1);
    c.lyrics_fit = c.lyrics_fit.min(FIT_LABELS.len() - 1);
    c.lyrics_paused = c.lyrics_paused.min(PAUSED_LABELS.len() - 1);
    c.lyrics_second = c.lyrics_second.min(SECOND_LABELS.len() - 1);
    c.lyrics_panel_alt = c.lyrics_panel_alt.min(PANEL_ALT_LABELS.len() - 1);
    c.lyrics_preset = preset_of(c);
}

/// How far through line `i` playback is, from 0 to 1. The line is sung over the time its length
/// suggests, up to the time until the next line, so a break between lines does not stretch the fill.
pub fn progress(lines: &[Line], i: usize, pos_ms: u64) -> f32 {
    let Some(line) = lines.get(i) else { return 0.0 };
    let span = match lines.get(i + 1) {
        Some(next) => next.at_ms.saturating_sub(line.at_ms),
        None => LAST_LINE_MS,
    };
    if span == 0 {
        return 1.0;
    }
    let sing = (line.text.chars().count() as u64 * SING_MS_PER_CHAR).max(MIN_SING_MS).min(span);
    (pos_ms.saturating_sub(line.at_ms) as f32 / sing as f32).clamp(0.0, 1.0)
}

/// How much a line `text_w` wide is scaled to fit in `avail`: 1 when it fits or the fit is not Shrink.
pub fn shrink_scale(text_w: f32, avail: f32, fit: Fit) -> f32 {
    if fit != Fit::Shrink || text_w <= avail || text_w <= 0.0 {
        return 1.0;
    }
    (avail / text_w).clamp(MIN_SHRINK, 1.0)
}

/// How far a line that overflows by `overflow` dips has scrolled `secs` after it appeared: it rests,
/// scrolls to its end, rests and scrolls back, over and over.
pub fn scroll_offset(overflow: f32, secs: f32) -> f32 {
    if overflow <= 0.0 {
        return 0.0;
    }
    let travel = overflow / SCROLL_SPEED;
    let cycle = 2.0 * (SCROLL_HOLD_S + travel);
    let t = secs.max(0.0) % cycle;
    if t < SCROLL_HOLD_S {
        0.0
    } else if t < SCROLL_HOLD_S + travel {
        (t - SCROLL_HOLD_S) / travel * overflow
    } else if t < 2.0 * SCROLL_HOLD_S + travel {
        overflow
    } else {
        (1.0 - (t - 2.0 * SCROLL_HOLD_S - travel) / travel) * overflow
    }
}

/// The part of `text` a typewriter has typed when it is `f` (0 to 1) of the way through.
pub fn typed(text: &str, f: f32) -> &str {
    let total = text.chars().count();
    let shown = ((total as f32 * f.clamp(0.0, 1.0)).ceil() as usize).min(total);
    match text.char_indices().nth(shown) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<Line> {
        vec![Line { at_ms: 1000, text: "a".into() }, Line { at_ms: 5000, text: "b".into() }]
    }

    #[test]
    fn the_old_styles_are_presets_of_the_new_choices() {
        let mut c = Config { lyrics_style: Some(3), ..Config::default() };
        sanitize(&mut c);
        assert!(c.lyric_look().karaoke && c.lyric_look().has_pill());
        assert_eq!(c.lyrics_preset, 3);
        let mut c = Config { lyrics_style: Some(1), ..Config::default() };
        sanitize(&mut c);
        assert_eq!(c.lyric_look().place, Place::InNotch);
        // Read once: a later sanitise leaves the choices alone.
        c.lyrics_color = 2;
        sanitize(&mut c);
        assert_eq!((c.lyrics_color, c.lyrics_preset), (2, CUSTOM_PRESET));
    }

    #[test]
    fn choices_combine_beyond_the_presets() {
        let c = Config { lyrics_place: 1, lyrics_highlight: 1, ..Config::default() };
        let look = c.lyric_look();
        assert!(look.karaoke && !look.below_notch() && !look.has_pill());
        assert_eq!(preset_of(&c), CUSTOM_PRESET);
        assert_eq!(look.room_below(30.0), 0.0);
    }

    #[test]
    fn out_of_range_choices_are_clamped_and_presets_apply() {
        let mut c = Config { lyrics_size: 9, lyrics_enter: 9, lyrics_fit: 9, lyrics_second: 9, lyrics_preset: 4, ..Config::default() };
        apply_preset(&mut c);
        sanitize(&mut c);
        assert_eq!((c.lyrics_size, c.lyrics_enter, c.lyrics_fit, c.lyrics_second), (2, 3, 2, 4));
        assert_eq!((c.lyrics_backdrop, c.lyrics_color, c.lyrics_preset), (1, 1, 4));
    }

    #[test]
    fn the_pill_grows_only_for_a_second_line_and_with_the_size() {
        let small = Config { lyrics_size: 0, ..Config::default() }.lyric_look();
        let large = Config { lyrics_size: 2, ..Config::default() }.lyric_look();
        assert!(small.pill_height(false) < small.pill_height(true));
        assert!(large.pill_height(true) > small.pill_height(true));
        assert!(large.room_below(large.pill_height(true)) <= ROOM_MAX);
    }

    #[test]
    fn progress_runs_over_the_time_the_line_takes_to_sing() {
        let l = lines();
        assert_eq!(progress(&l, 0, 500), 0.0);
        // A one-letter line is sung in the minimum time, then holds through the gap to the next line.
        assert_eq!(progress(&l, 0, 1000 + MIN_SING_MS / 2), 0.5);
        assert_eq!(progress(&l, 0, 3000), 1.0);
        assert_eq!(progress(&l, 1, 5000 + MIN_SING_MS / 2), 0.5);
        assert_eq!(progress(&l, 7, 100), 0.0);
        // Never longer than the gap itself.
        let tight = vec![Line { at_ms: 0, text: "long line of words here".into() }, Line { at_ms: 1000, text: "x".into() }];
        assert_eq!(progress(&tight, 0, 1000), 1.0);
    }

    #[test]
    fn only_shrink_scales_and_never_below_the_floor() {
        assert_eq!(shrink_scale(300.0, 400.0, Fit::Shrink), 1.0);
        assert_eq!(shrink_scale(500.0, 400.0, Fit::Shrink), 0.8);
        assert_eq!(shrink_scale(2000.0, 400.0, Fit::Shrink), MIN_SHRINK);
        assert_eq!(shrink_scale(500.0, 400.0, Fit::Cut), 1.0);
    }

    #[test]
    fn a_long_line_scrolls_to_its_end_and_back() {
        assert_eq!(scroll_offset(0.0, 3.0), 0.0);
        assert_eq!(scroll_offset(80.0, 0.5), 0.0);
        let mid = scroll_offset(80.0, SCROLL_HOLD_S + 1.0);
        assert!(mid > 0.0 && mid < 80.0);
        assert_eq!(scroll_offset(80.0, SCROLL_HOLD_S + 2.0 + 0.5), 80.0);
        let cycle = 2.0 * (SCROLL_HOLD_S + 2.0);
        assert!(scroll_offset(80.0, cycle - 0.1) < 80.0 / 4.0);
    }

    #[test]
    fn the_typewriter_types_whole_characters() {
        assert_eq!(typed("héllo", 0.0), "");
        assert_eq!(typed("héllo", 0.4), "hé");
        assert_eq!(typed("héllo", 1.0), "héllo");
        assert_eq!(typed("勘冴え", 0.5), "勘冴");
    }
}
