//! Settings search: an index of every setting by the page it lives on, matched against what is typed.
//! A test keeps the labels here in step with the ones the pages draw.

use super::{ABOUT_PAGE, CATEGORIES, DIAG_PAGE, SOUNDS_PAGE};

pub(super) struct Entry {
    pub page: usize,
    pub label: &'static str,
    /// Words people may look for that the label doesn't contain.
    pub keywords: &'static str,
}

const fn e(page: usize, label: &'static str, keywords: &'static str) -> Entry {
    Entry { page, label, keywords }
}

pub(super) const INDEX: &[Entry] = &[
    e(SOUNDS_PAGE, "Sound volume", "audio loudness mute"),
    e(SOUNDS_PAGE, "Import sound", "custom upload wav mp3 ogg file ringtone"),
    e(SOUNDS_PAGE, "Repeat timer sound", "alarm loop ringing"),
    e(SOUNDS_PAGE, "Pause between rings", "alarm interval gap"),
    e(SOUNDS_PAGE, "Stop ringing after", "alarm maximum duration"),
    e(SOUNDS_PAGE, "Gradual volume", "alarm fade increase"),
    e(SOUNDS_PAGE, "Restore sound defaults", "reset assignments presets"),
    e(0, "Sound library", "audio notifications presets custom"),
    e(0, "Start with Windows", "startup boot login launch autostart"),
    e(0, "Music", "page tab notch show hide"),
    e(0, "Focus", "page tab notch show hide pomodoro"),
    e(0, "Water", "page tab notch show hide"),
    e(0, "Shelf", "page tab notch show hide files"),
    e(0, "Clipboard", "page tab notch show hide history"),
    e(0, "To-do", "page tab notch show hide tasks"),
    e(0, "Calendar", "page tab notch show hide"),
    e(0, "Stats", "page tab notch show hide streaks"),
    e(0, "Size", "notch bigger smaller scale"),
    e(0, "Minimal notch", "thin line idle"),
    e(0, "Tooltips", "hints hover help labels buttons"),
    e(0, "Sui mascot", "droplet character"),
    e(0, "Sui reactions", "cheer mascot celebrate"),
    e(0, "Reminder sounds", "chime audio mute"),
    e(0, "Respect Do Not Disturb", "focus assist quiet gaming presenting"),
    e(0, "Hide in full screen", "games borderless videos slides auto hide"),
    e(1, "Main display", "monitor screen primary"),
    e(1, "Follow the active window", "monitor screen move multi"),
    e(1, "Follow the mouse", "monitor screen cursor pointer move multi"),
    e(1, "Open on hover", "click expand"),
    e(1, "Hover delay", "wait expand"),
    e(2, "Accent colour", "color theme swatch"),
    e(2, "Use the Windows accent colour", "color system theme"),
    e(2, "Notch style", "docked floating flat"),
    e(2, "Floating gap", "space edge distance"),
    e(2, "Corner roundness", "radius soft round"),
    e(2, "Frosted glass", "translucent blur transparent"),
    e(2, "Shadow", "drop"),
    e(2, "Animation style", "motion spring bouncy"),
    e(2, "Time format", "clock 12 24 hour"),
    e(ABOUT_PAGE, "Preview Updates tab", "debug test download install simulation"),
    e(ABOUT_PAGE, "Test volume HUD", "debug preview audio"),
    e(ABOUT_PAGE, "Test notification", "debug preview message"),
    e(3, "Open lyrics automatically", "panel"),
    e(3, "Show lyrics in the closed notch", "pill line under"),
    e(3, "Per-player lyric searches", "automatic manual never song video browser source filter"),
    e(3, "Keep searching for", "retry again seconds wait lyrics"),
    e(3, "Fetch the next song's lyrics ahead", "prefetch preload next track queue faster"),
    e(3, "Learn what plays next", "history playlist album predict next track"),
    e(3, "Your own lyric files", "lrc local folder custom offline"),
    e(3, "Lyric preset", "style look pill in the notch text only karaoke glow display custom"),
    e(3, "Placement", "lyrics under in the notch title position"),
    e(3, "Backdrop", "lyrics pill background tint black none shadow"),
    e(3, "Highlight", "lyrics karaoke fill sung colour words word by word syllable line"),
    e(3, "Text colour", "lyrics color album accent white"),
    e(3, "Text size", "lyrics font small large"),
    e(3, "New lines", "lyrics animation slide fade typewriter enter"),
    e(3, "Long lines", "lyrics shrink scroll cut marquee wide overflow"),
    e(3, "When paused", "lyrics keep hide dimmed"),
    e(3, "Second line", "romaji translation next line pill lyrics auto"),
    e(3, "Under each line in the panel", "romaji translation lyrics panel"),
    e(3, "Equalizer animation", "bars visualizer"),
    e(3, "Artwork glow", "cover album tint colour"),
    e(3, "Show player name", "spotify apple music source"),
    e(3, "Lyrics come from LRCLIB", "privacy source service"),
    e(3, "Fall back to other services", "privacy source service backup lyrics apple"),
    e(3, "Keep lyrics on this PC", "cache offline save disk"),
    e(3, "Clear saved lyrics", "cache delete remove"),
    e(ABOUT_PAGE, "Reset all settings", "default factory restore fresh"),
    e(3, "Clear recently played", "history delete remove songs"),
    e(4, "Snooze for", "reminder time's up"),
    e(4, "Show completed tasks", "done finished crossed"),
    e(4, "Clear completed tasks daily", "done finished remove"),
    e(5, "Week starts on", "monday sunday"),
    e(5, "Day dots", "mark tasks events"),
    e(5, "Add a calendar link", "ical ics outlook google paste url"),
    e(5, "Heads-up before events", "reminder notify"),
    e(6, "Focus", "pomodoro minutes work"),
    e(6, "Short break", "pomodoro minutes rest"),
    e(6, "Long break", "pomodoro minutes rest"),
    e(6, "Sessions before a long break", "pomodoro cycles"),
    e(6, "Start breaks automatically", "pomodoro auto"),
    e(6, "Start focus automatically", "pomodoro auto"),
    e(7, "Water reminders", "drink hydrate nudge"),
    e(7, "Remind every", "water interval minutes"),
    e(7, "From", "water hours start"),
    e(7, "Until", "water hours end"),
    e(7, "Glass size", "water ml amount"),
    e(7, "Daily goal", "water litres target"),
    e(7, "Reset today's log", "water clear"),
    e(8, "Open the shelf when dragging", "files drop"),
    e(8, "Remove after dragging out", "files"),
    e(8, "Show file sizes", "files"),
    e(8, "Clear the shelf", "files delete remove"),
    e(9, "Keep clipboard history", "copies"),
    e(9, "Items to keep", "clipboard limit"),
    e(9, "Keep after a restart", "clipboard remember"),
    e(9, "Show a popup", "clipboard copy hud"),
    e(9, "Clear clipboard history", "delete remove"),
    e(10, "Volume", "hud overlay popup"),
    e(10, "Brightness", "hud overlay popup"),
    e(10, "Caps Lock", "hud overlay popup"),
    e(10, "Keyboard language", "hud overlay popup input"),
    e(10, "Battery and charger", "hud overlay popup power"),
    e(10, "Wi-Fi", "hud overlay popup network"),
    e(10, "Headphones", "hud overlay popup bluetooth audio"),
    e(10, "Layout", "hud compact wide"),
    e(10, "Stays for", "hud duration"),
    e(ABOUT_PAGE, "Support", "help email contact"),
    e(ABOUT_PAGE, "Support the developers", "donate coffee"),
    e(ABOUT_PAGE, "What's new", "changelog release notes"),
    e(ABOUT_PAGE, "Automatic updates", "install version"),
    e(ABOUT_PAGE, "Stored only on this PC", "data folder privacy"),
    e(DIAG_PAGE, "Diagnostics", "debug log troubleshoot problem bug fullscreen monitor"),
];

/// Name of the page an entry lives on, as the sidebar shows it.
pub(super) fn page_name(page: usize) -> &'static str {
    if page == DIAG_PAGE {
        "About"
    } else {
        CATEGORIES.get(page).map_or("", |c| c.name)
    }
}

/// Entries that contain every word typed (in the label, its keywords or its page's name), best first:
/// those matching in the label itself come before those matching only in the keywords.
pub(super) fn matches(query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(usize, usize)> = INDEX
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            let label = entry.label.to_lowercase();
            let all = format!("{label} {} {}", entry.keywords, page_name(entry.page).to_lowercase());
            words.iter().all(|w| all.contains(w.as_str())).then(|| (words.iter().filter(|w| label.contains(w.as_str())).count(), i))
        })
        .collect();
    hits.sort_by_key(|(in_label, _)| std::cmp::Reverse(*in_label));
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_is_one_the_pages_draw() {
        let drawn = [include_str!("pages.rs"), include_str!("sounds.rs"), include_str!("../notch/kinds.rs")].join("\n");
        for entry in INDEX {
            assert!(drawn.contains(&format!("\"{}\"", entry.label)), "no page draws \"{}\"", entry.label);
            assert!(entry.page < CATEGORIES.len() || entry.page == DIAG_PAGE, "{} points at a missing page", entry.label);
        }
    }

    #[test]
    fn finds_by_label_keyword_and_page_name() {
        let labels = |q: &str| matches(q).iter().map(|&i| INDEX[i].label).collect::<Vec<_>>();
        assert!(labels("lyrics").contains(&"Keep lyrics on this PC"));
        assert!(labels("borderless").contains(&"Hide in full screen"));
        assert!(labels("MOUSE").contains(&"Follow the mouse"));
        assert!(labels("hud brightness").contains(&"Brightness"));
        assert!(labels("zzzz").is_empty());
        assert!(labels("   ").is_empty());
    }

    #[test]
    fn label_matches_rank_above_keyword_matches() {
        let hits = matches("clipboard");
        assert_eq!(hits.first().map(|&i| INDEX[i].label), Some("Clipboard"));
    }
}
