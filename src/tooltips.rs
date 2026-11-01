//! Hint text for the notch's buttons. Only buttons that are not self-explanatory have one: play,
//! previous, next, the settings gear and plus/minus steppers speak for themselves.

use crate::app::App;
use crate::notch::Hit;

/// The state a hint's wording depends on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub lyrics_view: bool,
    pub recent_view: bool,
    /// What the lyrics panel shows under each line: 0 nothing, 1 romaji, 2 translation.
    pub panel_alt: usize,
    pub muted: bool,
}

impl Context {
    pub fn of(app: &App) -> Context {
        Context {
            lyrics_view: app.notch.lyrics_view,
            recent_view: app.notch.recent_view,
            panel_alt: app.cfg.lyrics_panel_alt,
            muted: app.audio.as_ref().is_some_and(|a| a.get().1),
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
        Hit::LyricsToggle if ctx.lyrics_view => "Hide lyrics",
        Hit::LyricsToggle => "Show lyrics",
        Hit::RecentToggle if ctx.recent_view => "Hide recently played",
        Hit::RecentToggle => "Recently played",
        Hit::LyricsEarlier => "Show lyrics earlier",
        Hit::LyricsLater => "Show lyrics later",
        Hit::LyricsAlt => match ctx.panel_alt {
            0 => "Show romaji or a translation",
            1 => "Romaji. Click for the next view",
            _ => "Translation. Click for the next view",
        },
        Hit::LyricLine(_) => "Jump to this line",
        Hit::Recent(_) => "Copy the song name",
        Hit::Volume if ctx.muted => "Unmute",
        Hit::Volume => "Mute",
        Hit::WaterUndo => "Undo the last drink",
        Hit::NewColor | Hit::TodoColor(_) => "Task colour",
        Hit::DueClear => "Remove the due time",
        Hit::CalToday => "Go to today",
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
        let on = Context { lyrics_view: true, muted: true, ..Context::default() };
        let off = Context::default();
        assert_eq!(describe(Hit::LyricsToggle, &on).as_deref(), Some("Hide lyrics"));
        assert_eq!(describe(Hit::LyricsToggle, &off).as_deref(), Some("Show lyrics"));
        assert_eq!(describe(Hit::Volume, &on).as_deref(), Some("Unmute"));
        assert_eq!(describe(Hit::Volume, &off).as_deref(), Some("Mute"));
    }

    #[test]
    fn page_dots_name_their_page() {
        assert_eq!(describe(Hit::PageDot(Page::Water), &Context::default()).as_deref(), Some("Water"));
    }
}
