//! How the current lyric line is shown while the notch is closed. The look is a handful of
//! independent choices (`Look`) that combine freely; the old styles survive as presets that set them.
//! The pure maths behind them lives here; the drawing is in `lyric_draw`.

use crate::config::Config;
use crate::lrc::Line;

pub const PRESET_LABELS: [&str; 6] = ["Pill", "In the notch", "Text only", "Karaoke", "Glow", "Custom"];
/// The preset index that stands for "the choices match no preset".
pub const CUSTOM_PRESET: usize = 5;
pub const PLACE_LABELS: [&str; 3] = ["Under the notch", "In the notch", "Both"];
pub const BACKDROP_LABELS: [&str; 3] = ["Black pill", "Album tint", "None"];
pub const HIGHLIGHT_LABELS: [&str; 3] = ["None", "Line", "Words"];
pub const COLOR_LABELS: [&str; 3] = ["Album tint", "White", "Accent"];
pub const SIZE_LABELS: [&str; 3] = ["Small", "Medium", "Large"];
pub const ENTER_LABELS: [&str; 4] = ["Slide", "Fade", "Typewriter", "None"];
pub const FIT_LABELS: [&str; 3] = ["Shrink", "Scroll", "Cut"];
pub const PAUSED_LABELS: [&str; 2] = ["Hide", "Keep dimmed"];
pub const SECOND_LABELS: [&str; 5] = ["Off", "Next line", "Romaji", "Translation", "Auto"];
pub const PANEL_ALT_LABELS: [&str; 3] = ["Off", "Romaji", "Translation"];

/// The quick lyrics button cycles the existing visibility and placement settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LyricsMode {
    #[default]
    Off,
    Auto,
    Outside,
    Inside,
    Both,
}

/// Space between the notch and the pill, and below the pill, so the window has some air around it.
pub const ROOM_PAD: f32 = 14.0;
/// Most room the lyrics ever need under the notch (a large pill with two lines); the surface is sized for it.
pub const ROOM_MAX: f32 = 50.0 + ROOM_PAD;
/// Widest a line is drawn, in dips, before it is shrunk, scrolled or cut.
pub const MAX_LINE_W: f32 = 560.0;
/// A last line, with no next line to measure by, fills over this long.
const LAST_LINE_MS: u64 = 5000;
/// Time a line takes to sing per sound (a syllable, or a kana or Chinese character), for the karaoke
/// fill; it stops there rather than creeping through a break. Sung lines run at about 4 to 5 sounds a
/// second in both English and Japanese, where a character count would rush a Japanese line.
const SING_MS_PER_SOUND: u64 = 220;
const MIN_SING_MS: u64 = 1200;
/// How far a line may shrink before it is cut instead.
const MIN_SHRINK: f32 = 0.7;
/// How fast a long line scrolls at least, in dips per second, and how long it rests at its start.
const SCROLL_SPEED: f32 = 40.0;
const SCROLL_HOLD_S: f32 = 1.2;
/// How long before the line is sung a scrolling line reaches its end, so its last words can be read.
const SCROLL_END_EARLY_S: f32 = 0.4;
/// How fast a long song title goes round, in dips per second, and how long it rests at its start.
const MARQUEE_SPEED: f32 = 30.0;
const MARQUEE_HOLD_S: f32 = 2.0;
/// Space between the end of a song title going round and its start following it.
pub const MARQUEE_GAP: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A line under the notch.
    Under,
    /// The line takes the song title's place inside the closed notch.
    InNotch,
    /// The current line appears inside the notch and below it simultaneously.
    Both,
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
    /// The line fills with colour as it is sung (the Line or Words highlight).
    pub karaoke: bool,
    /// The fill follows each word, rather than spreading evenly over the line.
    pub words: bool,
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
        matches!(self.place, Place::Under | Place::Both)
    }

    pub fn in_notch(&self) -> bool {
        matches!(self.place, Place::InNotch | Place::Both)
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
    pub fn lyrics_mode(&self) -> LyricsMode {
        if !self.lyrics_pill { LyricsMode::Off }
        else if self.lyrics_auto_mode { LyricsMode::Auto }
        else if self.lyrics_place == 2 { LyricsMode::Both }
        else if self.lyrics_place == 1 { LyricsMode::Inside }
        else { LyricsMode::Outside }
    }

    pub fn cycle_lyrics_mode(&mut self) -> LyricsMode {
        match self.lyrics_mode() {
            LyricsMode::Auto => { self.lyrics_auto_mode = false; self.lyrics_place = 0; }
            LyricsMode::Off => { self.lyrics_auto_mode = false; self.lyrics_pill = true; self.lyrics_place = 0; }
            LyricsMode::Outside => { self.lyrics_auto_mode = false; self.lyrics_place = 1; }
            LyricsMode::Inside => { self.lyrics_auto_mode = false; self.lyrics_place = 2; }
            LyricsMode::Both => { self.lyrics_auto_mode = false; self.lyrics_pill = false; }
        }
        self.lyrics_mode()
    }

    pub fn lyric_look(&self) -> Look {
        Look {
            place: match self.lyrics_place { 1 => Place::InNotch, 2 => Place::Both, _ => Place::Under },
            backdrop: [Backdrop::Pill, Backdrop::Tint, Backdrop::None][self.lyrics_backdrop.min(2)],
            karaoke: self.lyrics_highlight >= 1,
            words: self.lyrics_highlight == 2,
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

    /// The highlight follows words, so lyrics should come with word timing where it exists.
    pub fn wants_word_timing(&self) -> bool {
        self.lyrics_highlight == 2
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
    c.lyrics_selected_preset = c.lyrics_preset;
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
    c.lyrics_selected_preset = c.lyrics_selected_preset.min(PRESET_LABELS.len() - 1);
}

/// How far through line `i` playback is, from 0 to 1. The line is sung over the time its length
/// suggests, up to the time until the next line, so a break between lines does not stretch the fill.
pub fn progress(lines: &[Line], i: usize, pos_ms: u64) -> f32 {
    let Some(line) = lines.get(i) else { return 0.0 };
    let sing = sing_ms(lines, i);
    if sing == 0 {
        return 1.0;
    }
    (pos_ms.saturating_sub(line.at_ms) as f32 / sing as f32).clamp(0.0, 1.0)
}

/// How long line `i` takes to sing, judged from how many sounds it has, and no longer than the time
/// until the next line; 0 for a line that is gone before it starts.
pub fn sing_ms(lines: &[Line], i: usize) -> u64 {
    let Some(line) = lines.get(i) else { return 0 };
    let span = match lines.get(i + 1) {
        Some(next) => next.at_ms.saturating_sub(line.at_ms),
        None => LAST_LINE_MS,
    };
    (crate::words::sounds(&line.text) as u64 * SING_MS_PER_SOUND).max(MIN_SING_MS).min(span)
}

/// The first `n` characters of `text`.
pub fn first_chars(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

/// How much a line `text_w` wide is scaled to fit in `avail`: 1 when it fits or the fit is not Shrink.
pub fn shrink_scale(text_w: f32, avail: f32, fit: Fit) -> f32 {
    if fit != Fit::Shrink || text_w <= avail || text_w <= 0.0 {
        return 1.0;
    }
    (avail / text_w).clamp(MIN_SHRINK, 1.0)
}

/// How far a line that overflows by `overflow` dips has scrolled `secs` after it appeared: it rests at
/// its start, scrolls to its end once and stays there. It keeps pace with the singing, speeding up so
/// it reaches its end by the time the line has been sung (`sung_s`), but never moves slower than usual.
pub fn scroll_offset(overflow: f32, secs: f32, sung_s: f32) -> f32 {
    if overflow <= 0.0 {
        return 0.0;
    }
    let hold = SCROLL_HOLD_S.min(sung_s / 4.0).max(0.0);
    let window = sung_s - hold - SCROLL_END_EARLY_S;
    let speed = if window > 0.0 { (overflow / window).max(SCROLL_SPEED) } else { SCROLL_SPEED };
    ((secs - hold).max(0.0) * speed).min(overflow)
}

/// Whether a line that overflows by `overflow` is still on its way to its end `secs` after it appeared.
pub fn still_scrolling(overflow: f32, secs: f32, sung_s: f32) -> bool {
    overflow > 0.0 && scroll_offset(overflow, secs, sung_s) < overflow
}

/// How far a text that goes round and round has moved `secs` after it appeared: it rests at its start,
/// moves left by one `lap` (its width and the gap after it), where its start has come round again,
/// and begins over.
pub fn marquee_offset(lap: f32, secs: f32) -> f32 {
    if lap <= 0.0 {
        return 0.0;
    }
    let t = secs.max(0.0) % (MARQUEE_HOLD_S + lap / MARQUEE_SPEED);
    ((t - MARQUEE_HOLD_S).max(0.0) * MARQUEE_SPEED).min(lap)
}

/// The part of `text` a typewriter has typed when it is `f` (0 to 1) of the way through.
pub fn typed(text: &str, f: f32) -> &str {
    let total = text.chars().count();
    first_chars(text, ((total as f32 * f.clamp(0.0, 1.0)).ceil() as usize).min(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_modes_cycle_in_order_and_persist_without_changing_the_look() {
        let mut c = Config { lyrics_pill: false, lyrics_highlight: 1, lyrics_color: 2, lyrics_size: 2, ..Config::default() };
        for expected in [LyricsMode::Outside, LyricsMode::Inside, LyricsMode::Both, LyricsMode::Off, LyricsMode::Outside] {
            assert_eq!(c.cycle_lyrics_mode(), expected);
            let restored: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
            c = restored.sanitized();
            assert_eq!(c.lyrics_mode(), expected);
            assert_eq!((c.lyrics_highlight, c.lyrics_color, c.lyrics_size), (1, 2, 2));
        }
    }

    #[test]
    fn both_placement_enables_two_renderers_and_reserves_pill_room() {
        let c = Config { lyrics_place: 2, ..Config::default() }.sanitized();
        let look = c.lyric_look();
        assert_eq!(c.lyrics_mode(), LyricsMode::Both);
        assert!(look.in_notch() && look.below_notch());
        assert!(look.has_pill());
        assert_eq!(look.room_below(44.0), 44.0 + ROOM_PAD);
    }

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
    fn auto_mode_applies_settings_preset_and_cycles_cleanly() {
        let mut c = Config::default();
        assert_eq!(c.lyrics_mode(), LyricsMode::Outside);
        // Select Karaoke (preset 3) on the settings.
        c.lyrics_preset = 3;
        apply_preset(&mut c);
        assert_eq!(c.lyrics_selected_preset, 3);
        assert_eq!(c.lyrics_highlight, 1);
        assert_eq!(c.lyrics_place, 0);

        // Manually change to Inside mode.
        c.lyrics_place = 1;
        c.lyrics_auto_mode = false;
        assert_eq!(c.lyrics_mode(), LyricsMode::Inside);

        // Turn on Auto mode: reapplies the selected preset (Karaoke).
        c.lyrics_auto_mode = true;
        c.lyrics_preset = c.lyrics_selected_preset;
        apply_preset(&mut c);
        assert_eq!(c.lyrics_mode(), LyricsMode::Auto);
        assert_eq!(c.lyrics_place, 0);
        assert_eq!(c.lyrics_highlight, 1);

        // Cycling from Auto transitions to Outside.
        let next = c.cycle_lyrics_mode();
        assert_eq!(next, LyricsMode::Outside);
        assert!(!c.lyrics_auto_mode);
        assert_eq!(c.lyrics_place, 0);
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
    fn a_long_line_scrolls_to_its_end_once_and_stays() {
        assert_eq!(scroll_offset(0.0, 3.0, 10.0), 0.0);
        assert_eq!(scroll_offset(80.0, 0.5, 10.0), 0.0);
        let mid = scroll_offset(80.0, SCROLL_HOLD_S + 1.0, 10.0);
        assert!(mid > 0.0 && mid < 80.0);
        // At the usual speed when the line is sung slowly, and it stays at its end.
        assert_eq!(scroll_offset(80.0, SCROLL_HOLD_S + 2.0, 10.0), 80.0);
        assert_eq!(scroll_offset(80.0, 60.0, 10.0), 80.0);
        assert!(still_scrolling(80.0, SCROLL_HOLD_S + 1.0, 10.0));
        assert!(!still_scrolling(80.0, 60.0, 10.0));
        assert!(!still_scrolling(0.0, 1.0, 10.0));
    }

    #[test]
    fn a_long_line_sung_quickly_scrolls_faster_to_finish_in_time() {
        // 400 dips at the usual 40 a second would take 10 s; the line is sung in 3 s.
        let end = 3.0 - SCROLL_END_EARLY_S;
        assert!((scroll_offset(400.0, end, 3.0) - 400.0).abs() < 0.01);
        assert!(scroll_offset(400.0, end - 0.5, 3.0) < 400.0);
    }

    #[test]
    fn a_long_title_goes_round_with_a_rest_at_its_start() {
        let lap = 300.0;
        assert_eq!(marquee_offset(lap, 0.0), 0.0);
        assert_eq!(marquee_offset(lap, MARQUEE_HOLD_S), 0.0);
        let one_lap = MARQUEE_HOLD_S + lap / MARQUEE_SPEED;
        let half = marquee_offset(lap, MARQUEE_HOLD_S + lap / MARQUEE_SPEED / 2.0);
        assert!((half - lap / 2.0).abs() < 0.01);
        // Round again: back at the start, resting.
        assert!(marquee_offset(lap, one_lap + 0.1) < 0.01);
        assert_eq!(marquee_offset(0.0, 5.0), 0.0);
    }

    #[test]
    fn the_highlight_is_none_line_or_words() {
        let look = |h| Config { lyrics_highlight: h, ..Config::default() }.lyric_look();
        assert!(!look(0).karaoke && !look(0).words);
        assert!(look(1).karaoke && !look(1).words);
        assert!(look(2).karaoke && look(2).words);
        assert!(Config { lyrics_highlight: 2, ..Config::default() }.wants_word_timing());
        assert!(!Config { lyrics_highlight: 1, ..Config::default() }.wants_word_timing());
        let mut c = Config { lyrics_highlight: 9, ..Config::default() };
        sanitize(&mut c);
        assert_eq!(c.lyrics_highlight, 2);
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(first_chars("勘冴えて", 2), "勘冴");
        assert_eq!(first_chars("ab", 5), "ab");
        assert_eq!(first_chars("ab", 0), "");
        assert_eq!(sing_ms(&lines(), 0), MIN_SING_MS);
        assert_eq!(sing_ms(&lines(), 9), 0);
    }

    #[test]
    fn japanese_lines_are_given_the_time_their_sounds_take() {
        // 14 characters, 13 sounds: sung over about 3 s, not rushed through in 1.2 s.
        let l = vec![Line { at_ms: 0, text: "有無も言えずに うらめしいや".into() }, Line { at_ms: 3220, text: "x".into() }];
        assert_eq!(sing_ms(&l, 0), 13 * SING_MS_PER_SOUND);
        let en = vec![Line { at_ms: 0, text: "I've been on my own for long enough".into() }, Line { at_ms: 9000, text: "x".into() }];
        // I've·been·on·my·own·for·long·e·nough
        assert_eq!(sing_ms(&en, 0), 9 * SING_MS_PER_SOUND);
    }

    #[test]
    fn the_typewriter_types_whole_characters() {
        assert_eq!(typed("héllo", 0.0), "");
        assert_eq!(typed("héllo", 0.4), "hé");
        assert_eq!(typed("héllo", 1.0), "héllo");
        assert_eq!(typed("勘冴え", 0.5), "勘冴");
    }
}
