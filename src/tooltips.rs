//! Hint text for the notch's buttons. Only buttons that are not self-explanatory have one: play,
//! previous, next, the settings gear and plus/minus steppers speak for themselves.

use crate::app::App;
use crate::notch::Hit;
use crate::lyric_style::LyricsMode;

/// The state a hint's wording depends on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub lyrics_mode: LyricsMode,
    pub recent_view: bool,
    /// What the lyrics panel shows under each line: 0 nothing, 1 romaji, 2 translation.
    pub panel_alt: usize,
    pub muted: bool,
    /// The lyrics on show are kept for their song.
    pub lyrics_kept: bool,
}

impl Context {
    pub fn of(app: &App) -> Context {
        Context {
            lyrics_mode: app.cfg.lyrics_mode(),
            recent_view: app.notch.recent_view,
            panel_alt: app.cfg.lyrics_panel_alt,
            muted: app.audio.as_ref().is_some_and(|a| a.get().1),
            lyrics_kept: app.lyric_request.pinned,
        }
    }
}

pub fn text(app: &App, hit: Hit) -> Option<String> {
    describe(hit, &Context::of(app))
}

/// What to say about a button, or None when it needs no explanation.
pub fn describe(hit: Hit, ctx: &Context) -> Option<String> {
    let text = match hit {
        Hit::PageDot(page) => page.label(),
        Hit::Source => "Switch to the next player",
        Hit::LyricsToggle => match ctx.lyrics_mode {
            LyricsMode::Off => "Lyrics off; click to show outside",
            LyricsMode::Outside => "Lyrics outside; click to show inside",
            LyricsMode::Inside => "Lyrics inside; click to show both",
            LyricsMode::Both => "Lyrics inside and outside; click to turn off",
        },
        Hit::RecentToggle if ctx.recent_view => "Hide recently played",
        Hit::RecentToggle => "Recently played",
        Hit::LyricsEarlier => "Show lyrics earlier",
        Hit::LyricsLater => "Show lyrics later",
        Hit::LyricsAlt => match ctx.panel_alt {
            0 => "Show romaji or a translation",
            1 => "Romaji. Click for the next view",
            _ => "Translation. Click for the next view",
        },
        Hit::LyricsWrong => "Wrong lyrics: never show these for this song, and look for others",
        Hit::LyricsKeep if ctx.lyrics_kept => "Kept for this song. Click to stop keeping them",
        Hit::LyricsKeep => "Keep these lyrics for this song, whatever later searches find",
        Hit::LyricLine(_) => "Jump to this line",
        Hit::Recent(_) => "Copy the song name",
        Hit::Volume if ctx.muted => "Unmute",
        Hit::Volume => "Mute",
        Hit::WaterUndo => "Undo the last drink",
        Hit::NewColor | Hit::TodoColor(_) => "Task colour",
        Hit::DueClear => "Turn off reminder; keep the scheduled day",
        Hit::DueEnable => "Add a reminder on this day",
        Hit::DueTime => "Edit reminder time; Enter to apply, Escape to cancel",
        Hit::DueStep(-1) => "15 minutes earlier",
        Hit::DueStep(_) => "15 minutes later",
        Hit::TodoDayStep(-1) => "Previous day",
        Hit::TodoDayStep(_) => "Next day",
        Hit::TodoCalendar => "Return to the selected day in Calendar",
        Hit::TodoAll => "Show all tasks; keep this day's draft",
        Hit::CalToday => "Go to today",
        Hit::CalDay(_) => "Select day; double-click to add tasks",
        Hit::ClipItem(_) => "Click to copy it back",
        _ => return None,
    };
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notch::Page;

    #[test]
    fn obvious_buttons_have_no_hint() {
        let ctx = Context::default();
        for hit in [Hit::PlayPause, Hit::Prev, Hit::Next, Hit::Settings, Hit::TimerStep(1), Hit::TodoDelete(1)] {
            assert_eq!(describe(hit, &ctx), None, "{hit:?}");
        }
    }

    #[test]
    fn wording_follows_the_state_of_a_toggle() {
        let on = Context { lyrics_mode: LyricsMode::Inside, muted: true, ..Context::default() };
        let off = Context::default();
        assert_eq!(describe(Hit::LyricsToggle, &on).as_deref(), Some("Lyrics inside; click to show both"));
        let both = Context { lyrics_mode: LyricsMode::Both, ..Context::default() };
        assert_eq!(describe(Hit::LyricsToggle, &both).as_deref(), Some("Lyrics inside and outside; click to turn off"));
        assert_eq!(describe(Hit::LyricsToggle, &off).as_deref(), Some("Lyrics off; click to show outside"));
        let outside = Context { lyrics_mode: LyricsMode::Outside, ..Context::default() };
        assert_eq!(describe(Hit::LyricsToggle, &outside).as_deref(), Some("Lyrics outside; click to show inside"));
        assert_eq!(describe(Hit::Volume, &on).as_deref(), Some("Unmute"));
        assert_eq!(describe(Hit::Volume, &off).as_deref(), Some("Mute"));
    }

    #[test]
    fn page_dots_name_their_page() {
        assert_eq!(describe(Hit::PageDot(Page::Water), &Context::default()).as_deref(), Some("Water"));
    }
}
