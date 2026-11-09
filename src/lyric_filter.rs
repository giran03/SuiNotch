//! Song eligibility and the UI-thread lifecycle of lyric requests.
use crate::{
    app::App,
    config::Config,
    events::{MediaInfo, MediaKind},
    lyrics::{self, Outcome, Request},
    song_gate::{self, Verdict},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    Automatic,
    Manual,
    Never,
}
impl Policy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Manual => "Manual",
            Self::Never => "Never",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Automatic => Self::Manual,
            Self::Manual => Self::Never,
            Self::Never => Self::Automatic,
        }
    }
}

pub fn is_browser(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    [
        "chrome",
        "chromium",
        "msedge",
        "firefox",
        "opera",
        "brave",
        "vivaldi",
        "browser",
        "waterfox",
        "librewolf",
        "floorp",
    ]
    .iter()
    .any(|name| id.contains(name))
}

fn music_player(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    let music = [
        "spotify",
        "applemusic",
        "foobar",
        "musicbee",
        "aimp",
        "tidal",
        "deezer",
        "amazonmusic",
        "qobuz",
    ]
    .iter()
    .any(|s| id.contains(s));
    music && !is_browser(app_id)
}

pub fn policy(cfg: &Config, info: &MediaInfo) -> Policy {
    let saved = cfg.lyrics_sources.get(&info.app_id).copied();
    if is_browser(&info.app_id) {
        return if saved == Some(Policy::Never) {
            Policy::Never
        } else {
            Policy::Manual
        };
    }
    saved.unwrap_or(Policy::Automatic)
}

/// What an automatic lookup may do for the track playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Fetch,
    /// Ask a music catalog whether it is a song first.
    Verify,
    /// No automatic lookup; with the reason shown when it is the track itself, not the player.
    Skip(Option<&'static str>),
}

/// A player's Music flag is only a hint: browsers use it for videos too. Inside Automatic, the
/// track's metadata is weighed (see `song_gate`); players not made for music that leave doubt are
/// checked against a music catalog when other services may be used.
pub fn gate(cfg: &Config, info: &MediaInfo) -> Gate {
    if !cfg.lyrics_pill || policy(cfg, info) != Policy::Automatic {
        return Gate::Skip(None);
    }
    let known = music_player(&info.app_id);
    if song_gate::remembered(info.track_key()) == Some(true) {
        return Gate::Fetch;
    }
    let a = assessment(info);
    match a.verdict {
        Verdict::NotSong => Gate::Skip(a.why),
        Verdict::Song => Gate::Fetch,
        // A music player is trusted on doubt; so is any player when the catalog may not be asked.
        Verdict::Unsure if known || !cfg.lyrics_fallback => Gate::Fetch,
        Verdict::Unsure => Gate::Verify,
    }
}

/// What `song_gate` makes of the track, counting its player as the gate does.
pub fn assessment(info: &MediaInfo) -> song_gate::Assessment {
    song_gate::assess(info, music_player(&info.app_id))
}

#[cfg(test)]
fn automatic(cfg: &Config, info: &MediaInfo) -> bool {
    !matches!(gate(cfg, info), Gate::Skip(_))
}

#[derive(PartialEq)]
struct Identity {
    track: u64,
    source: String,
}

/// How the current track's lookup ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    Lyrics,
    Instrumental,
    NotFound,
}

#[derive(Default)]
pub struct State {
    signature: Option<Identity>,
    pub generation: u64,
    pub manual: bool,
    pub searching: bool,
    pub started: bool,
    pub settled: Option<Settled>,
    /// Why the track was passed over, when it was for what it is rather than for its player.
    pub skipped: Option<&'static str>,
    /// The id of the lyrics on show, to mark them wrong or keep them.
    pub id: String,
    /// The lyrics on show are kept through later lookups.
    pub pinned: bool,
    /// The song's length when its lookup began. Players often name the new song a moment before they
    /// give its length, so the first lookup may go out with the last song's.
    asked_ms: u64,
    /// The lookup was made again for the song's right length; it is only made again once.
    asked_again: bool,
}

/// A length this much different from the one a lookup was made with means it was made with the wrong
/// one, as when the player had not yet given the new song's length.
const LENGTH_CHANGED_MS: u64 = 3000;

/// Whether a song's lookup went out with a length other than its own, now that the player gives
/// `now_ms`: it is then made again, since the lyrics services match songs by their length.
fn asked_with_wrong_length(asked_ms: u64, now_ms: u64) -> bool {
    now_ms > 0 && asked_ms.abs_diff(now_ms) > LENGTH_CHANGED_MS
}

pub fn clear(app: &mut App) {
    crate::lyrics::cancel();
    app.lyric_request.generation = 0;
    app.lyric_request.searching = false;
    app.lyric_request.settled = None;
    app.lyric_request.id.clear();
    app.lyric_request.pinned = false;
    app.lyrics.clear();
    app.lyrics_translation.clear();
    app.lyrics_romaji.clear();
    app.lyrics_words.clear();
    app.lyrics_key = 0;
    app.lyrics_source = None;
}

/// Stop work without losing an already-loaded song or its manual-search consent.
fn cancel_pending(app: &mut App) {
    crate::lyrics::cancel();
    app.lyric_request.generation = 0;
    app.lyric_request.searching = false;
    if app
        .media
        .as_ref()
        .is_none_or(|m| app.lyrics_key != m.info.track_key())
    {
        app.lyric_request.started = false;
    }
}

/// Only a different track or player changes identity. Placement and Off are presentation choices.
pub fn reconcile(app: &mut App) {
    let Some(info) = app.media.as_ref().map(|m| m.info.clone()) else {
        clear(app);
        crate::lyrics_prefetch::cancel();
        app.lyric_request = State::default();
        return;
    };
    let signature = Identity {
        track: info.track_key(),
        source: info.app_id.clone(),
    };
    if app.lyric_request.signature.as_ref() != Some(&signature) {
        clear(app);
        // What was being fetched ahead is either this track, now fetched in its own right, or stale.
        crate::lyrics_prefetch::cancel();
        app.lyrics_attempt = 0;
        app.lyric_request = State {
            signature: Some(signature),
            ..Default::default()
        };
    }
    if policy(&app.cfg, &info) == Policy::Never {
        clear(app);
        app.lyric_request.started = false;
        app.lyric_request.manual = false;
        return;
    }
    let gate = gate(&app.cfg, &info);
    if let Gate::Skip(Some(why)) = gate {
        app.lyric_request.skipped = Some(why);
    }
    let eligible = !matches!(gate, Gate::Skip(_));
    if !app.cfg.lyrics_pill || (!app.lyric_request.manual && (!eligible || !info.playing)) {
        if app.lyric_request.searching {
            cancel_pending(app);
        }
        return;
    }
    if !app.lyric_request.started {
        if app.lyric_request.manual {
            request(app, true, false);
        } else if eligible && info.playing {
            request(app, false, false);
        }
    } else if !app.lyric_request.manual
        && !app.lyric_request.pinned
        && !app.lyric_request.asked_again
        && asked_with_wrong_length(app.lyric_request.asked_ms, info.duration_ms)
    {
        crate::log("lyrics looked up again: the player gave the song's length after its name");
        app.lyric_request.asked_again = true;
        request(app, false, false);
    }
}

pub fn manual_allowed(app: &App) -> bool {
    app.cfg.lyrics_pill
        && app.media.as_ref().is_some_and(|m| {
            policy(&app.cfg, &m.info) != Policy::Never && !m.info.title.trim().is_empty()
        })
}

/// Explicit searches bypass song classification, but respect Off and Never.
pub fn request(app: &mut App, manual: bool, extras: bool) {
    let Some(info) = app.media.as_ref().map(|m| m.info.clone()) else {
        return;
    };
    // Changing a display option must not launch another browser search.
    if extras && is_browser(&info.app_id) {
        return;
    }
    let gate = gate(&app.cfg, &info);
    if manual {
        if !manual_allowed(app) {
            return;
        }
    } else if matches!(gate, Gate::Skip(_)) || !info.playing {
        return;
    }
    if !extras {
        clear(app);
        if manual && app.lyric_request.started {
            app.lyrics_attempt += 1;
        }
    }
    app.lyric_request.manual = manual;
    app.lyric_request.started = true;
    app.lyric_request.searching = true;
    app.lyric_request.skipped = None;
    app.lyric_request.asked_ms = info.duration_ms;
    let skip = app.shifts.blocked(info.track_key());
    app.lyric_request.generation = lyrics::fetch(Request {
        info,
        attempt: app.lyrics_attempt,
        opts: lyrics::Options::from_config(&app.cfg),
        automatic: !manual,
        verify: !manual && gate == Gate::Verify,
        skip,
        alias: false,
    });
}

/// Takes a finished lookup, if it is still the one wanted, and looks ahead to the next song.
pub fn apply(app: &mut App, key: u64, generation: u64, outcome: Outcome) {
    if generation != app.lyric_request.generation || app.media.as_ref().is_none_or(|m| m.info.track_key() != key) {
        return;
    }
    app.lyric_request.searching = false;
    let (settled, found) = match outcome {
        Outcome::Found(f) => (Settled::Lyrics, Some(f)),
        Outcome::Instrumental => (Settled::Instrumental, None),
        Outcome::NotFound => (Settled::NotFound, None),
    };
    app.lyric_request.settled = Some(settled);
    app.lyrics_source = found.as_ref().map(|f| (f.source, f.synced));
    app.lyric_request.id = found.as_ref().map(|f| f.id.clone()).unwrap_or_default();
    // Kept only if these very lyrics are the kept ones: "Search again" may show others meanwhile.
    app.lyric_request.pinned = app.cfg.lyrics_cache && crate::lyrics_cache::is_pinned(key, &app.lyric_request.id);
    let f = found.unwrap_or_else(|| lyrics::Found { lines: Vec::new(), source: lyrics::Source::Lrclib, synced: false, translation: Vec::new(), romaji: Vec::new(), words: Vec::new(), id: String::new() });
    app.lyrics = f.lines;
    app.lyrics_translation = f.translation;
    app.lyrics_romaji = f.romaji;
    app.lyrics_words = f.words;
    app.lyrics_key = key;
    if let Some(plan) = plan_ahead(app) {
        crate::lyrics_prefetch::start(plan);
    }
}

/// What to fetch ahead while this song plays, if anything.
pub fn plan_ahead(app: &App) -> Option<crate::lyrics_prefetch::Plan> {
    use crate::lyrics_prefetch::{wait_for, Plan, Target};
    let cfg = &app.cfg;
    let m = app.media.as_ref()?;
    let offline = app.network.as_ref().is_some_and(|(connected, _)| !connected);
    if !cfg.lyrics_prefetch || !cfg.lyrics_cache || !cfg.lyrics_pill || offline || !m.info.playing || app.lyric_request.manual {
        return None;
    }
    if matches!(gate(cfg, &m.info), Gate::Skip(_)) {
        return None;
    }
    let learned = cfg.lyrics_learn.then(|| app.history.predict(&m.info, crate::store::now())).flatten().map(|n| MediaInfo {
        title: n.title.clone(),
        artist: n.artist.clone(),
        album: n.album.clone(),
        duration_ms: n.duration_ms,
        app_id: m.info.app_id.clone(),
        kind: MediaKind::Music,
        playing: true,
        ..Default::default()
    });
    let album_next = cfg.lyrics_fallback && m.info.track_number > 0 && !m.info.album.is_empty() && (m.info.track_count == 0 || m.info.track_number < m.info.track_count);
    let target = match learned {
        // Only what would be fetched anyway when it plays: never a catalog check ahead.
        Some(next) if gate(cfg, &next) == Gate::Fetch => Target::Known(next),
        Some(_) => return None,
        None if album_next => Target::AlbumNext(m.info.clone()),
        None => return None,
    };
    Some(Plan { target, wait: wait_for(m.position_ms(), m.info.duration_ms), opts: lyrics::Options::from_config(cfg), blocked: app.shifts.blocked.clone() })
}

/// "Wrong lyrics": these are never offered for this song again, and the best other match is looked up.
pub fn reject(app: &mut App) -> bool {
    let Some(key) = app.media.as_ref().map(|m| m.info.track_key()) else { return false };
    let id = app.lyric_request.id.clone();
    if app.lyrics_key != key || !manual_allowed(app) || !app.shifts.block(key, &id) {
        return false;
    }
    app.save_shifts();
    // Only these lyrics go: other lyrics kept for the song stay.
    crate::lyrics_cache::remove_if(key, &id);
    app.lyrics_attempt = 0;
    app.lyric_request.started = false;
    request(app, true, false);
    true
}

/// "Keep these": the lyrics on show stay for this song through later lookups, or stop staying.
/// None when they cannot be kept (nothing cached, or the cache is off).
pub fn toggle_keep(app: &mut App) -> Option<bool> {
    let key = app.media.as_ref().map(|m| m.info.track_key())?;
    if app.lyrics_key != key || !app.cfg.lyrics_cache || app.lyrics.is_empty() {
        return None;
    }
    let keep = !app.lyric_request.pinned;
    crate::lyrics_cache::set_pinned(key, keep).then(|| {
        app.lyric_request.pinned = keep;
        keep
    })
}

/// The network came back: a track whose automatic lookup found nothing is tried once more, since the
/// services may just not have been reachable.
pub fn reconnected(app: &mut App) {
    let unanswered = app.lyric_request.settled == Some(Settled::NotFound) && !app.lyric_request.manual && !app.lyric_request.searching;
    if unanswered && app.lyrics.is_empty() {
        app.lyric_request.started = false;
        request(app, false, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn song(id: &str) -> MediaInfo {
        MediaInfo {
            title: "Song".into(),
            artist: "Artist".into(),
            app_id: id.into(),
            playing: true,
            kind: MediaKind::Music,
            ..Default::default()
        }
    }
    #[test]
    fn browsers_are_manual_and_all_other_players_default_to_automatic() {
        let cfg = Config::default();
        for id in [
            "chrome.exe",
            "msedge.exe",
            "firefox",
            "brave.exe",
            "Spotify browser",
        ] {
            assert!(!automatic(&cfg, &song(id)), "{id}");
        }
        for id in [
            "Spotify.exe",
            "AppleInc.AppleMusicWin!App",
            "foobar2000.exe",
            "MusicBee.exe",
            "vlc.exe",
            "unknown",
            "Wavee.exe",
        ] {
            assert_eq!(policy(&cfg, &song(id)), Policy::Automatic, "{id}");
            assert!(automatic(&cfg, &song(id)), "{id}");
        }
    }
    #[test]
    fn source_preferences_off_and_non_song_metadata_are_respected() {
        let mut cfg = Config::default();
        let mut info = song("chrome.exe");
        cfg.lyrics_sources
            .insert(info.app_id.clone(), Policy::Automatic);
        assert!(!automatic(&cfg, &info)); // saved browser Automatic never bypasses the gate
        assert_eq!(policy(&cfg, &info), Policy::Manual);
        info = song("mpv.exe");
        cfg.lyrics_sources
            .insert(info.app_id.clone(), Policy::Automatic);
        assert!(automatic(&cfg, &info));
        info.kind = MediaKind::Video;
        assert!(!automatic(&cfg, &info));
        info.kind = MediaKind::Music;
        info.artist.clear();
        assert!(!automatic(&cfg, &info));
        info = song("Spotify.exe");
        info.title = "Advertisement".into();
        assert!(!automatic(&cfg, &info));
        info.title = "Song".into();
        for mode in [Policy::Manual, Policy::Never] {
            cfg.lyrics_sources.insert(info.app_id.clone(), mode);
            assert!(!automatic(&cfg, &info));
        }
        cfg.lyrics_sources.clear();
        cfg.lyrics_pill = false;
        assert!(!automatic(&cfg, &info));
    }
    #[test]
    fn music_players_skip_podcasts_and_doubtful_players_are_checked_first() {
        let mut cfg = Config::default();
        let episode = MediaInfo { title: "Episode 41: Interview".into(), duration_ms: 3_000_000, ..song("Spotify.exe") };
        assert_eq!(gate(&cfg, &episode), Gate::Skip(Some("Looks like a podcast episode")));
        // A general player with bare metadata is checked against a catalog, unless other
        // services are off, in which case it is fetched as before.
        let bare = MediaInfo { duration_ms: 200_000, ..song("mpv.exe") };
        assert_eq!(gate(&cfg, &bare), Gate::Verify);
        cfg.lyrics_fallback = false;
        assert_eq!(gate(&cfg, &bare), Gate::Fetch);
        cfg.lyrics_fallback = true;
        let tagged = MediaInfo { album: "Album".into(), track_number: 3, ..bare.clone() };
        assert_eq!(gate(&cfg, &tagged), Gate::Fetch);
        // A catalog miss cannot block a lyrics search, even if remembered by an older lookup.
        let checked = MediaInfo { title: "Checked once".into(), ..bare };
        crate::song_gate::remember(checked.track_key(), false);
        assert_eq!(gate(&cfg, &checked), Gate::Verify);
    }
    #[test]
    fn browser_gate_overrides_saved_automatic_and_preserves_never() {
        let mut cfg = Config::default();
        for id in [
            "chrome.exe",
            "Microsoft.msedge",
            "Firefox",
            "Brave.exe",
            "Opera",
            "Vivaldi",
            "Chromium",
            "Waterfox",
        ] {
            let info = song(id);
            cfg.lyrics_sources.insert(id.into(), Policy::Automatic);
            assert_eq!(policy(&cfg, &info), Policy::Manual, "{id}");
            assert!(!automatic(&cfg, &info), "{id}");
            cfg.lyrics_sources.insert(id.into(), Policy::Never);
            assert_eq!(policy(&cfg, &info), Policy::Never);
        }
    }
    #[test]
    fn old_config_defaults_and_preferences_round_trip() {
        let mut cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.lyrics_sources.is_empty());
        cfg.lyrics_sources
            .insert("chrome.exe".into(), Policy::Manual);
        let restored: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(cfg.lyrics_sources, restored.lyrics_sources);
    }

    #[test]
    fn a_lookup_made_with_the_last_songs_length_is_made_again() {
        // "JET" (4:45) was named while the player still gave "Jet Lag"'s 3:03.
        assert!(asked_with_wrong_length(183_080, 285_107));
        assert!(asked_with_wrong_length(0, 285_107));
        // The same length, a little rounding, or no length yet is no reason to.
        assert!(!asked_with_wrong_length(285_107, 285_000));
        assert!(!asked_with_wrong_length(285_107, 0));
    }
}
