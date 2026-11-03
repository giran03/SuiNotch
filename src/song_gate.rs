//! Whether what is playing is likely a song, from what the player says about it. Each sign adds or
//! takes points; a few settle it outright (a video, a podcast genre, an ad). In between, the answer
//! is "unsure", and for players not known for music a music catalog is asked (see `localized::verify`).

use std::sync::Mutex;

use crate::events::{MediaInfo, MediaKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Song,
    Unsure,
    NotSong,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assessment {
    pub verdict: Verdict,
    pub score: i32,
    /// Every sign that counted, for the diagnostics.
    pub signs: Vec<&'static str>,
    /// Why it is not a song, in words for the lyrics panel; set when the verdict is NotSong.
    pub why: Option<&'static str>,
}

pub const NOT_IN_CATALOG: &str = "Not found in a music catalog";
const SONG_AT: i32 = 2;
const NOT_SONG_AT: i32 = -3;
/// Shorter than this is a jingle or an ad; longer than the other a mix, an episode or a stream.
const SHORT_MS: u64 = 30_000;
const LONG_MS: u64 = 20 * 60_000;
const LONG_TITLE: usize = 90;
/// Video-title signs together take at most this much.
const VIDEO_CAP: i32 = -4;

const PODCAST: &str = "Looks like a podcast episode";
const VIDEO: &str = "Looks like a video";
const AD: &str = "Looks like an ad";
const AUDIOBOOK: &str = "Looks like an audiobook";
const SPOKEN: &str = "Looks like spoken word";
const SHORT: &str = "Too short to be a song";
const MISSING: &str = "No title or artist";
const DOUBT: &str = "Doesn't look like a song";

/// Catalog answers already had this run, by track, so a track is checked once.
static VERIFIED: Mutex<Vec<(u64, bool)>> = Mutex::new(Vec::new());
const MAX_VERIFIED: usize = 500;

pub fn remember(key: u64, song: bool) {
    let mut v = VERIFIED.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|(k, _)| *k != key);
    if v.len() >= MAX_VERIFIED {
        v.remove(0);
    }
    v.push((key, song));
}

pub fn remembered(key: u64) -> Option<bool> {
    VERIFIED.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

fn not_song(why: &'static str, signs: Vec<&'static str>) -> Assessment {
    Assessment { verdict: Verdict::NotSong, score: i32::MIN, signs, why: Some(why) }
}

/// What the metadata says. `known_player` is a player made for music (Spotify, Apple Music, ...).
pub fn assess(info: &MediaInfo, known_player: bool) -> Assessment {
    let title = info.title.trim().to_lowercase();
    let artist = info.artist.trim().to_lowercase();
    if info.kind == MediaKind::Video {
        return not_song(VIDEO, vec!["the player says video"]);
    }
    if title.is_empty() || artist.is_empty() {
        return not_song(MISSING, vec!["no title or artist"]);
    }
    if ["advertisement", "advertising", "sponsored message", "spotify"].contains(&title.as_str()) {
        return not_song(AD, vec!["an ad title"]);
    }
    for g in info.genres.iter().map(|g| g.to_lowercase()) {
        let why = if g.contains("podcast") {
            PODCAST
        } else if g.contains("audiobook") || g.contains("audio book") {
            AUDIOBOOK
        } else if ["spoken word", "talk", "news", "comedy", "speech"].iter().any(|s| g.contains(s)) {
            SPOKEN
        } else {
            continue;
        };
        return not_song(why, vec!["genre"]);
    }

    let mut score = 0;
    let mut signs = Vec::new();
    let mut worst: Option<(i32, &'static str)> = None;
    let mut add = |points: i32, sign: &'static str, why: Option<&'static str>| {
        score += points;
        signs.push(sign);
        if let (Some(why), true) = (why, points < 0) {
            if worst.is_none_or(|(p, _)| points < p) {
                worst = Some((points, why));
            }
        }
    };
    let album = !info.album.trim().is_empty();
    if album && info.track_number > 0 {
        add(3, "track number on an album", None);
    }
    if album {
        add(1, "album", None);
    }
    if !info.album_artist.trim().is_empty() {
        add(1, "album artist", None);
    }
    if artist.ends_with(" - topic") || artist.contains("vevo") {
        add(2, "a music channel", None);
    }
    if known_player {
        add(2, "a music player", None);
    }
    // Enough to outweigh a music player alone: podcasts play in Spotify too.
    if episode_like(&title) {
        add(-5, "an episode title", Some(PODCAST));
    }
    let video = video_signs(&info.title);
    if video < 0 {
        add(video.max(VIDEO_CAP), "a video title", Some(VIDEO));
    }
    match info.duration_ms {
        0 => add(-2, "no length (a stream)", None),
        d if d < SHORT_MS => add(-3, "very short", Some(SHORT)),
        d if d > LONG_MS => add(-2, "very long", None),
        _ => {}
    }
    let verdict = if score >= SONG_AT {
        Verdict::Song
    } else if score <= NOT_SONG_AT {
        Verdict::NotSong
    } else {
        Verdict::Unsure
    };
    let why = (verdict == Verdict::NotSong).then(|| worst.map_or(DOUBT, |(_, w)| w));
    Assessment { verdict, score, signs, why }
}

/// "Episode 12", "Ep. 4", "#123", "S2E5", "Chapter 3", "Part 2 of 5", "... podcast".
fn episode_like(t: &str) -> bool {
    if t.contains("episode") || t.contains("podcast") || t.contains("chapter ") {
        return true;
    }
    let words: Vec<&str> = t.split(|c: char| c.is_whitespace() || c == ':' || c == '-' || c == '|').filter(|w| !w.is_empty()).collect();
    let numeric = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit());
    words.windows(2).any(|w| matches!(w[0], "ep" | "ep." | "eps" | "part") && numeric(w[1].trim_end_matches(['.', ','])) && (w[0] != "part" || t.contains(" of ")))
        || words.iter().any(|w| {
            let w = w.trim_end_matches(['.', ',']);
            // Two digits at least, so "#1 Crush" stays a song.
            (w.starts_with('#') && w.len() > 2 && numeric(&w[1..]))
                || (w.starts_with("ep") && numeric(&w[2..]) && w.len() > 2)
                || season_episode(w)
        })
}

/// "s2e5", "s02e11".
fn season_episode(w: &str) -> bool {
    let Some(rest) = w.strip_prefix('s') else { return false };
    let Some((season, episode)) = rest.split_once('e') else { return false };
    !season.is_empty() && !episode.is_empty() && season.chars().all(|c| c.is_ascii_digit()) && episode.chars().all(|c| c.is_ascii_digit())
}

/// Points off for title habits of videos rather than songs.
fn video_signs(title: &str) -> i32 {
    let t = title.to_lowercase();
    let mut points = 0;
    if t.contains(" | ") {
        points -= 2;
    }
    // A hashtag: '#' straight before a letter.
    if t.char_indices().any(|(i, c)| c == '#' && t[i + 1..].chars().next().is_some_and(char::is_alphabetic)) {
        points -= 2;
    }
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    if has("vs") {
        points -= 2;
    }
    if ["reaction", "tutorial", "trailer", "highlights", "livestream", "gameplay", "unboxing", "walkthrough", "vlog"].iter().any(|w| has(w)) || t.contains("live stream") {
        points -= 2;
    }
    if title.chars().count() > LONG_TITLE {
        points -= 2;
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(title: &str, artist: &str) -> MediaInfo {
        MediaInfo { title: title.into(), artist: artist.into(), kind: MediaKind::Music, duration_ms: 210_000, ..Default::default() }
    }

    fn verdict(i: &MediaInfo, known: bool) -> Verdict {
        assess(i, known).verdict
    }

    #[test]
    fn real_world_metadata_gets_the_expected_verdict() {
        let album = |title: &str, artist: &str, album: &str, n: u32| MediaInfo { album: album.into(), track_number: n, ..info(title, artist) };
        let cases: Vec<(MediaInfo, bool, Verdict)> = vec![
            // Music apps and local players with full tags.
            (album("Blinding Lights", "The Weeknd", "After Hours", 9), true, Verdict::Song),
            (album("Paper Sky", "Lumen Coast", "Coastlines", 3), false, Verdict::Song),
            (MediaInfo { album_artist: "Various".into(), ..album("Track", "Someone", "Compilation", 0) }, false, Verdict::Song),
            (info("Ordinary Song", "Band - Topic"), false, Verdict::Song),
            (info("Ordinary Song", "BandVEVO"), false, Verdict::Song),
            (info("Ordinary Song", "Band"), true, Verdict::Song),
            // Bare metadata from a general player: not enough to tell.
            (info("Ordinary Song", "Band"), false, Verdict::Unsure),
            (MediaInfo { duration_ms: 0, ..info("Song", "Artist") }, false, Verdict::Unsure),
            // Podcasts, videos, ads and jingles.
            (info("Episode 112: The long interview", "Talk Show"), false, Verdict::NotSong),
            (info("Ep. 4 - Getting started", "Learning Pod"), false, Verdict::NotSong),
            (info("#123 with a guest", "Some Show"), false, Verdict::NotSong),
            (info("S2E5 Finale", "Series"), false, Verdict::NotSong),
            (info("Funny cats compilation | try not to laugh #cats", "Cat Channel"), false, Verdict::NotSong),
            (info("Team A vs Team B highlights", "Sports"), false, Verdict::NotSong),
            (MediaInfo { duration_ms: 15_000, ..info("Jingle", "Radio") }, false, Verdict::NotSong),
            (MediaInfo { genres: vec!["Podcast".into()], ..album("Anything", "Host", "Show", 1) }, true, Verdict::NotSong),
            (MediaInfo { genres: vec!["Audiobooks".into()], ..info("Chapter one", "Narrator") }, true, Verdict::NotSong),
            (MediaInfo { kind: MediaKind::Video, ..album("Song", "Artist", "Album", 1) }, true, Verdict::NotSong),
            (info("Advertisement", "Spotify"), true, Verdict::NotSong),
            (info("Song", ""), true, Verdict::NotSong),
            // A podcast in a music app still reads as one.
            (info("Episode 3: Origins", "History Pod"), true, Verdict::NotSong),
            // Song titles that only look a little like the signs stay songs in a music app.
            (album("Part of Me", "Katy Perry", "Teenage Dream", 4), true, Verdict::Song),
            (album("Review", "Band", "Album", 2), true, Verdict::Song),
            (MediaInfo { duration_ms: 25 * 60_000, ..album("Long Suite", "Orchestra", "Works", 1) }, true, Verdict::Song),
        ];
        for (i, known, expected) in cases {
            assert_eq!(verdict(&i, known), expected, "{} / {} ({:?})", i.title, i.artist, assess(&i, known));
        }
    }

    #[test]
    fn the_reason_shown_is_the_strongest_sign_against() {
        assert_eq!(assess(&info("Episode 9", "Pod"), false).why, Some(PODCAST));
        assert_eq!(assess(&info("Clip | thing #fun", "Channel"), false).why, Some(VIDEO));
        assert_eq!(assess(&MediaInfo { duration_ms: 10_000, ..info("Ding", "Radio") }, false).why, Some(SHORT));
        assert_eq!(assess(&info("Song", "Band"), true).why, None);
    }

    #[test]
    fn episode_and_video_patterns() {
        for t in ["episode 4", "ep 12", "ep. 3", "ep12", "#45 guest", "s01e02", "part 2 of 5", "the daily podcast"] {
            assert!(episode_like(t), "{t}");
        }
        for t in ["part of me", "#1 crush", "septembers", "epic", "step 2", "s.o.s"] {
            assert!(!episode_like(t), "{t}");
        }
        assert_eq!(video_signs("Plain Song"), 0);
        assert!(video_signs("Boss fight gameplay | part 3") <= -4);
    }

    #[test]
    fn catalog_answers_are_remembered_per_track() {
        remember(42, false);
        assert_eq!(remembered(42), Some(false));
        remember(42, true);
        assert_eq!(remembered(42), Some(true));
        assert_eq!(remembered(43), None);
    }
}
