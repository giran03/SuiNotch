//! Synced lyrics lookup on LRCLIB (lrclib.net), with NetEase (see `netease`) as an optional fallback
//! and a cache on disk (see `lyrics_cache`). Only title, artist, album and length are sent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use windows::core::{Result, HSTRING};
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;

use crate::events::{post, Event, MediaInfo};
use crate::lrc::{self, Line};
use crate::{localized, lyrics_cache, netease};

const API: &str = "https://lrclib.net/api";
const USER_AGENT: &str = "SuiNotch/1.0 (a living notch for Windows)";
const DURATION_TOLERANCE_S: f64 = 4.0;

/// Where a song's lyrics came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Lrclib,
    Netease,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Lrclib => "LRCLIB",
            Source::Netease => "NetEase",
        }
    }
}

/// Lyrics for one song. Unsynced ones are spread evenly over the song so they still scroll.
#[derive(Clone, Debug)]
pub struct Found {
    pub lines: Vec<Line>,
    pub source: Source,
    pub synced: bool,
    /// The lines in another language, timed like `lines`; empty when there are none.
    pub translation: Vec<Line>,
    /// The lines in Latin letters, for songs in other scripts; empty when there are none.
    pub romaji: Vec<Line>,
}

impl Found {
    fn from_netease(n: netease::Lyrics) -> Found {
        Found { lines: n.lines, source: Source::Netease, synced: true, translation: n.translation, romaji: n.romaji }
    }
}

/// One name a song goes by: its title and main artist.
#[derive(Clone, Debug, PartialEq)]
pub struct Names {
    pub title: String,
    pub artist: String,
}

/// How far each song's lyrics were nudged, in ms (positive shows lines later). Kept in lyrics.json.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Shifts {
    pub ms: HashMap<String, i64>,
}

pub const SHIFT_STEP_MS: i64 = 500;
const MAX_SHIFT_MS: i64 = 30_000;

impl Shifts {
    pub fn get(&self, key: u64) -> i64 {
        self.ms.get(&key.to_string()).copied().unwrap_or(0)
    }

    pub fn nudge(&mut self, key: u64, delta: i64) {
        let v = (self.get(key) + delta).clamp(-MAX_SHIFT_MS, MAX_SHIFT_MS);
        if v == 0 {
            self.ms.remove(&key.to_string());
        } else {
            self.ms.insert(key.to_string(), v);
        }
    }
}

/// What a lookup may use besides LRCLIB.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub fallback: bool,
    pub cache: bool,
    /// How many more times a first search is repeated when it finds nothing.
    pub retries: u32,
    /// Romaji or a translation is wanted, so LRCLIB lyrics (which have none) get them from NetEase.
    pub extras: bool,
}

impl Options {
    pub fn from_config(cfg: &crate::config::Config) -> Options {
        Options { fallback: cfg.lyrics_fallback, cache: cfg.lyrics_cache, retries: cfg.lyrics_retries, extras: cfg.wants_lyric_extras() }
    }
}

/// Songs already asked NetEase for romaji and a translation this run, so one without any is not asked again.
static EXTRAS_ASKED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// Pause between one search round and the next.
const RETRY_PAUSE: Duration = Duration::from_secs(1);
/// Counts requests, so a search that a newer one (another song, or "Search again") has replaced
/// stops trying and says nothing.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Looks up lyrics; `attempt` 0 is the best match, each retry moves on to another candidate.
/// A first search that finds nothing is repeated up to `opts.retries` more times, since a service
/// may time out, or the player may still be filling in the song's details. One round searches
/// under every name the song goes by, so the original and the native-script title count as one.
pub fn fetch(info: MediaInfo, attempt: usize, opts: Options) {
    let key = info.track_key();
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        let cached = if attempt == 0 && opts.cache { lyrics_cache::load(key, info.duration_ms) } else { None };
        let found = cached.or_else(|| {
            let found = search_until(&info, attempt, opts, generation);
            // Unsynced lyrics are not kept: a synced version may turn up later, from either service.
            if let (true, Some(f)) = (opts.cache, found.as_ref().filter(|f| f.synced)) {
                if GENERATION.load(Ordering::SeqCst) == generation {
                    lyrics_cache::store(key, info.duration_ms, f);
                }
            }
            found
        });
        let found = found.map(|f| with_extras(key, &info, f, opts, generation));
        if GENERATION.load(Ordering::SeqCst) == generation {
            post(Event::Lyrics { key, found });
        }
    });
}

/// Whether synced lyrics lack the romaji and translation that are wanted, and NetEase has not been asked yet.
fn needs_extras(f: &Found, opts: Options) -> bool {
    opts.extras && opts.fallback && f.synced && f.translation.is_empty() && f.romaji.is_empty()
}

/// LRCLIB has no romaji or translations, so for lyrics from there they are taken from the same song
/// on NetEase. Only those lines are used: they are matched to the LRCLIB ones by time.
fn with_extras(key: u64, info: &MediaInfo, mut found: Found, opts: Options, generation: u64) -> Found {
    if !needs_extras(&found, opts) {
        return found;
    }
    {
        let mut asked = EXTRAS_ASKED.lock().unwrap_or_else(|e| e.into_inner());
        if asked.contains(&key) {
            return found;
        }
        asked.push(key);
    }
    let Ok(client) = client() else { return found };
    let names = names_for(&client, info, true);
    let Some(n) = netease::lookup(&client, &names, info.duration_ms, 1).into_iter().next() else { return found };
    if n.translation.is_empty() && n.romaji.is_empty() {
        return found;
    }
    found.translation = n.translation;
    found.romaji = n.romaji;
    if opts.cache && GENERATION.load(Ordering::SeqCst) == generation {
        lyrics_cache::store(key, info.duration_ms, &found);
    }
    found
}

fn search_until(info: &MediaInfo, attempt: usize, opts: Options, generation: u64) -> Option<Found> {
    // "Search again" (attempt > 0) looks once; a song's first search gets its retries.
    let rounds = if attempt == 0 { 1 + opts.retries } else { 1 };
    for round in 0..rounds {
        if round > 0 {
            std::thread::sleep(RETRY_PAUSE);
        }
        if GENERATION.load(Ordering::SeqCst) != generation {
            return None;
        }
        let looked_up = if attempt == 0 { lookup(info, opts.fallback) } else { lookup_other(info, attempt, opts.fallback) };
        let found = looked_up.unwrap_or_else(|e| {
            crate::log(&format!("lyrics lookup failed: {e}"));
            None
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn client() -> Result<HttpClient> {
    let client = HttpClient::new()?;
    let _ = client.DefaultRequestHeaders()?.UserAgent()?.TryParseAdd(&HSTRING::from(USER_AGENT));
    Ok(client)
}

/// The names to search under: the song as the player lists it and, when `resolve` is on and the
/// title is in Latin letters, its native-script names from Apple's store in Japan. One search round
/// tries them all.
fn names_for(client: &HttpClient, info: &MediaInfo, resolve: bool) -> Vec<Names> {
    let original = Names { title: clean_title(&info.title), artist: primary_artist(&info.artist) };
    let mut names = vec![original.clone()];
    if resolve {
        if let Some(native) = localized::lookup(client, &original, info.duration_ms) {
            if native != original {
                names.push(native);
            }
        }
    }
    names
}

/// The song on LRCLIB under each of its names: a synced match at once, otherwise the first plain one.
fn lrclib_best(client: &HttpClient, info: &MediaInfo, names: &[Names]) -> Option<Found> {
    let duration_s = info.duration_ms as f64 / 1000.0;
    let mut plain: Option<Found> = None;
    for n in names {
        let mut url = format!("{API}/get?track_name={}&artist_name={}", encode(&n.title), encode(&n.artist));
        if !info.album.is_empty() {
            url.push_str(&format!("&album_name={}", encode(&info.album)));
        }
        if duration_s > 0.0 {
            url.push_str(&format!("&duration={}", duration_s.round() as u64));
        }
        if let Ok(body) = get(client, &url) {
            if let Some(f) = serde_json::from_str::<Value>(&body).ok().and_then(|v| found_from(&v, info.duration_ms)) {
                if f.synced {
                    return Some(f);
                }
                plain = plain.or(Some(f));
            }
        }
        let url = format!("{API}/search?track_name={}&artist_name={}", encode(&n.title), encode(&n.artist));
        // LRCLIB being down must not stop the fallback, so a failed search is logged rather than returned.
        let body = get(client, &url).unwrap_or_else(|e| {
            crate::log(&format!("lrclib search failed: {e}"));
            String::new()
        });
        let results: Vec<Value> = serde_json::from_str(&body).unwrap_or_default();
        if let Some(f) = pick_best(&results, duration_s).and_then(|v| found_from(v, info.duration_ms)) {
            if f.synced {
                return Some(f);
            }
            plain = plain.or(Some(f));
        }
    }
    plain
}

/// Best lyrics for the song. Synced ones win: plain LRCLIB lyrics are kept only when
/// nothing synced turns up, from LRCLIB or the fallback.
fn lookup(info: &MediaInfo, fallback: bool) -> Result<Option<Found>> {
    let client = client()?;
    let names = names_for(&client, info, fallback);
    let lrclib = lrclib_best(&client, info, &names);
    if lrclib.as_ref().is_some_and(|f| f.synced) {
        return Ok(lrclib);
    }
    if fallback {
        if let Some(n) = netease::lookup(&client, &names, info.duration_ms, 1).into_iter().next() {
            return Ok(Some(Found::from_netease(n)));
        }
    }
    Ok(lrclib)
}

/// Another candidate for "Search again": the search results in order of fit, then a broader
/// search by title alone, then the fallback's songs.
fn lookup_other(info: &MediaInfo, attempt: usize, fallback: bool) -> Result<Option<Found>> {
    let client = client()?;
    let names = names_for(&client, info, fallback);
    let duration_s = info.duration_ms as f64 / 1000.0;
    let mut candidates: Vec<Value> = Vec::new();
    for n in &names {
        for url in [
            format!("{API}/search?track_name={}&artist_name={}", encode(&n.title), encode(&n.artist)),
            format!("{API}/search?q={}", encode(&n.title)),
        ] {
            let body = get(&client, &url).unwrap_or_default();
            let results: Vec<Value> = serde_json::from_str(&body).unwrap_or_default();
            for v in ranked(&results, duration_s) {
                if !candidates.iter().any(|c| c.get("id") == v.get("id")) {
                    candidates.push(v.clone());
                }
            }
        }
    }
    let mut usable: Vec<Found> = candidates.iter().filter_map(|v| found_from(v, info.duration_ms)).collect();
    if fallback {
        usable.extend(netease::lookup(&client, &names, info.duration_ms, 3).into_iter().map(Found::from_netease));
    }
    Ok(if usable.is_empty() { None } else { Some(usable[attempt % usable.len()].clone()) })
}

/// Results ordered: synced and the right length, synced, the right length, then the rest.
fn ranked(results: &[Value], duration_s: f64) -> Vec<&Value> {
    let fits = |v: &Value| duration_s <= 0.0 || v.get("duration").and_then(Value::as_f64).is_none_or(|d| (d - duration_s).abs() <= DURATION_TOLERANCE_S);
    let synced = |v: &Value| v.get("syncedLyrics").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    let mut v: Vec<&Value> = results.iter().collect();
    v.sort_by_key(|r| (!(synced(r) && fits(r)), !synced(r), !fits(r)));
    v
}

pub(crate) fn get(client: &HttpClient, url: &str) -> Result<String> {
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    Ok(client.GetStringAsync(&uri)?.get()?.to_string())
}

/// Lyrics from an LRCLIB record, synced when it has timestamps and otherwise spread over the song.
fn found_from(v: &Value, duration_ms: u64) -> Option<Found> {
    if let Some(synced) = v.get("syncedLyrics").and_then(Value::as_str) {
        let lines = lrc::parse(synced);
        if !lines.is_empty() {
            return Some(Found { lines, source: Source::Lrclib, synced: true, translation: Vec::new(), romaji: Vec::new() });
        }
    }
    let plain = v.get("plainLyrics").and_then(Value::as_str)?;
    let lines = lrc::from_plain(plain, duration_ms);
    (!lines.is_empty()).then_some(Found { lines, source: Source::Lrclib, synced: false, translation: Vec::new(), romaji: Vec::new() })
}

/// Prefers synced lyrics whose length matches the playing track.
pub fn pick_best(results: &[Value], duration_s: f64) -> Option<&Value> {
    let fits = |v: &&Value| {
        duration_s <= 0.0
            || v.get("duration").and_then(Value::as_f64).is_none_or(|d| (d - duration_s).abs() <= DURATION_TOLERANCE_S)
    };
    let synced = |v: &&Value| v.get("syncedLyrics").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    results
        .iter()
        .find(|v| fits(v) && synced(v))
        .or_else(|| results.iter().find(synced))
        .or_else(|| results.iter().find(fits))
}

/// Drops decorations such as "(Official Video)" or "- Remastered 2011" that confuse the search.
pub fn clean_title(title: &str) -> String {
    let mut t = title.to_string();
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let (Some(a), Some(b)) = (t.find(open), t.find(close)) {
            if b <= a {
                break;
            }
            let inner = t[a + 1..b].to_ascii_lowercase();
            let noise = ["official", "video", "audio", "lyric", "remaster", "visualizer", "hd", "4k", "mv"];
            if noise.iter().any(|n| inner.contains(n)) {
                t.replace_range(a..=b, "");
            } else {
                break;
            }
        }
    }
    if let Some(i) = t.to_ascii_lowercase().find(" - remaster") {
        t.truncate(i);
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn primary_artist(artist: &str) -> String {
    let lower = artist.to_ascii_lowercase();
    let cut = [", ", " & ", " feat.", " ft.", " x "].iter().filter_map(|s| lower.find(s)).min();
    let a = match cut {
        Some(i) => &artist[..i],
        None => artist,
    };
    a.trim().trim_end_matches(" - Topic").to_string()
}

pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn percent_encoding() {
        assert_eq!(encode("Paper Sky & Co"), "Paper%20Sky%20%26%20Co");
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn cleans_video_decorations() {
        assert_eq!(clean_title("Summer Flows (Official Video)"), "Summer Flows");
        assert_eq!(clean_title("Song [Lyric Video] (Live)"), "Song (Live)");
        assert_eq!(clean_title("Track - Remastered 2011"), "Track");
    }

    #[test]
    fn first_artist_only() {
        assert_eq!(primary_artist("Lumen Coast, Paper Sky"), "Lumen Coast");
        assert_eq!(primary_artist("A feat. B"), "A");
        assert_eq!(primary_artist("Band - Topic"), "Band");
    }

    #[test]
    fn shifts_are_kept_per_song_and_bounded() {
        let mut s = Shifts::default();
        s.nudge(7, 500);
        s.nudge(7, 500);
        s.nudge(8, -500);
        assert_eq!((s.get(7), s.get(8), s.get(9)), (1000, -500, 0));
        s.nudge(8, 500);
        assert!(!s.ms.contains_key("8"));
        s.nudge(7, 99_000);
        assert_eq!(s.get(7), 30_000);
    }

    #[test]
    fn retries_rank_synced_matches_first() {
        let r = vec![json!({"id": 1, "duration": 400.0, "plainLyrics": "x"}), json!({"id": 2, "duration": 201.0, "syncedLyrics": "[00:01.00]a"}), json!({"id": 3, "duration": 400.0, "syncedLyrics": "[00:01.00]b"})];
        let ids: Vec<i64> = ranked(&r, 200.0).iter().map(|v| v["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![2, 3, 1]);
    }

    #[test]
    fn best_result_prefers_synced_matching_length() {
        let r = vec![
            json!({"duration": 200.0, "plainLyrics": "x"}),
            json!({"duration": 400.0, "syncedLyrics": "[00:01.00]far"}),
            json!({"duration": 201.0, "syncedLyrics": "[00:01.00]near"}),
        ];
        assert_eq!(pick_best(&r, 200.0).unwrap()["syncedLyrics"], "[00:01.00]near");
    }

    #[test]
    fn records_say_whether_they_are_synced() {
        let synced = found_from(&json!({"syncedLyrics": "[00:01.00]a", "plainLyrics": "a"}), 100_000).unwrap();
        assert!(synced.synced && synced.source == Source::Lrclib);
        let plain = found_from(&json!({"plainLyrics": "a\nb"}), 100_000).unwrap();
        assert!(!plain.synced);
        assert!(found_from(&json!({}), 100_000).is_none());
    }

    #[test]
    fn only_synced_lyrics_without_extras_ask_for_them_when_wanted() {
        let opts = Options { fallback: true, cache: true, retries: 0, extras: true };
        let bare = found_from(&json!({"syncedLyrics": "[00:01.00]a"}), 100_000).unwrap();
        assert!(needs_extras(&bare, opts));
        assert!(!needs_extras(&bare, Options { extras: false, ..opts }));
        assert!(!needs_extras(&bare, Options { fallback: false, ..opts }));
        let plain = found_from(&json!({"plainLyrics": "a\nb"}), 100_000).unwrap();
        assert!(!needs_extras(&plain, opts));
        let with = Found { translation: vec![Line { at_ms: 1000, text: "x".into() }], ..bare };
        assert!(!needs_extras(&with, opts));
    }

    #[test]
    fn extras_are_wanted_only_when_something_shows_them() {
        let mut c = crate::config::Config::default();
        assert!(!c.wants_lyric_extras());
        c.lyrics_second = 1;
        assert!(!c.wants_lyric_extras(), "the next line needs no service");
        c.lyrics_second = 4;
        assert!(c.wants_lyric_extras());
        c.lyrics_second = 0;
        c.lyrics_panel_alt = 2;
        assert!(c.wants_lyric_extras());
    }
}
