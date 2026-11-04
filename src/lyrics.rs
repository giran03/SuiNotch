//! Synced lyrics for a song: your own files first (see `lyrics_local`), then the cache on disk (see
//! `lyrics_cache`), then LRCLIB (see `lrclib`) with NetEase (see `netease`) as an optional fallback.
//! Only title, artist, album and length are sent.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::events::{post, Event, MediaInfo};
use crate::localized::{self, Verified};
use crate::lrc::Line;
use crate::lrclib::{self, Record};
use crate::lyrics_cache::{self, Cached, Kind};
use crate::lyrics_net::{self, Lane};
use crate::words::{self, Word};
use crate::{lyrics_local, netease};

/// Where a song's lyrics came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Lrclib,
    Netease,
    /// A lyric file of your own (see `lyrics_local`).
    Local,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Lrclib => "LRCLIB",
            Source::Netease => "NetEase",
            Source::Local => "Your file",
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
    /// The timing of each word of each line, one list per line; a line without any has an empty list,
    /// and a song without any has none at all.
    pub words: Vec<Vec<Word>>,
    /// The service's id for these lyrics ("lrclib:390", "netease:123", "local:file.lrc"), so they can
    /// be marked wrong or kept; empty when unknown.
    pub id: String,
}

impl Found {
    fn from_netease(n: netease::Lyrics, duration_ms: u64) -> Found {
        let words = kept(&n.lines, words::attach(&n.lines, &n.timed), duration_ms);
        Found { lines: n.lines, source: Source::Netease, synced: true, translation: n.translation, romaji: n.romaji, words, id: n.id }
    }

    pub(crate) fn from_lrclib(lines: Vec<Line>, synced: bool, id: String) -> Found {
        Found { lines, source: Source::Lrclib, synced, translation: Vec::new(), romaji: Vec::new(), words: Vec::new(), id }
    }

    pub fn has_words(&self) -> bool {
        self.words.iter().any(|w| !w.is_empty())
    }
}

/// How a lookup ended.
#[derive(Clone, Debug)]
pub enum Outcome {
    Found(Found),
    /// The song has no words.
    Instrumental,
    NotFound,
}

/// Word lists as they are kept: vetted (see `words::vet`), and none at all when no line has any left.
pub(crate) fn kept(lines: &[Line], words: Vec<Vec<Word>>, duration_ms: u64) -> Vec<Vec<Word>> {
    match words::vet(lines, words, duration_ms) {
        Ok(words) if words.iter().any(|w| !w.is_empty()) => words,
        Ok(_) => Vec::new(),
        Err(why) => {
            crate::log(&format!("word timing thrown away: {why:?}"));
            Vec::new()
        }
    }
}

/// One name a song goes by: its title and main artist.
#[derive(Clone, Debug, PartialEq)]
pub struct Names {
    pub title: String,
    pub artist: String,
}

/// How far each song's lyrics were nudged, in ms (positive shows lines later), and which lyrics were
/// marked wrong for it. Kept in lyrics.json.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Shifts {
    pub ms: HashMap<String, i64>,
    /// Per song, the ids of lyrics marked wrong for it, never offered for it again.
    pub blocked: HashMap<String, Vec<String>>,
}

pub const SHIFT_STEP_MS: i64 = 500;
const MAX_SHIFT_MS: i64 = 30_000;
/// The oldest wrong lyrics are forgotten past this many for one song.
const MAX_BLOCKED: usize = 20;

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

    pub fn blocked(&self, key: u64) -> Vec<String> {
        self.blocked.get(&key.to_string()).cloned().unwrap_or_default()
    }

    /// Marks lyrics as wrong for a song; false when there is nothing to mark.
    pub fn block(&mut self, key: u64, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        let list = self.blocked.entry(key.to_string()).or_default();
        if !list.iter().any(|b| b == id) {
            list.push(id.to_string());
        }
        let extra = list.len().saturating_sub(MAX_BLOCKED);
        list.drain(..extra);
        true
    }
}

/// What a lookup may use besides LRCLIB.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub fallback: bool,
    pub cache: bool,
    /// How many more times a first search is repeated when services fail to answer.
    pub retries: u32,
    /// Romaji or a translation is wanted, so LRCLIB lyrics (which have none) get them from NetEase.
    pub extras: bool,
    /// The highlight follows words, so lyrics without word timing look for it on NetEase.
    pub words: bool,
}

impl Options {
    pub fn from_config(cfg: &crate::config::Config) -> Options {
        Options {
            fallback: cfg.lyrics_fallback,
            cache: cfg.lyrics_cache,
            retries: cfg.lyrics_retries,
            extras: cfg.wants_lyric_extras(),
            words: cfg.wants_word_timing(),
        }
    }
}

/// One lookup.
#[derive(Clone, Debug)]
pub struct Request {
    pub info: MediaInfo,
    /// 0 for the best match; each "Search again" moves on to another candidate.
    pub attempt: usize,
    pub opts: Options,
    /// Started by SuiNotch rather than by a click: waits a moment before using the network.
    pub automatic: bool,
    /// The player is not known for songs and its metadata leaves doubt: ask a music catalog first.
    pub verify: bool,
    /// Lyrics marked wrong for this song.
    pub skip: Vec<String>,
    /// Keep what is found under the song's alias key (see `lyrics_cache::alias_key`): for songs named
    /// by a store rather than the player, fetched ahead.
    pub alias: bool,
}

/// Automatic lookups wait this long before using the network, so a quick skip costs nothing.
const AUTOMATIC_DELAY: Duration = Duration::from_secs(2);
/// Pause before the second round of a search whose services failed, doubling each round up to the most.
const RETRY_PAUSE: Duration = Duration::from_secs(1);
const RETRY_PAUSE_MAX: Duration = Duration::from_secs(8);

/// Songs already asked NetEase for romaji, translations and word timing this run, and whether word
/// timing was asked for, so a song that has none is not asked again.
static EXTRAS_ASKED: Mutex<Vec<(u64, bool)>> = Mutex::new(Vec::new());

/// Invalidates pending searches for the song playing and prevents their further requests.
pub fn cancel() {
    lyrics_net::cancel_foreground();
}

/// Looks up the song playing in the background and posts the outcome. Automatic requests wait two
/// seconds before using the network; a song already known shows at once.
pub fn fetch(req: Request) -> u64 {
    let key = req.info.track_key();
    let generation = lyrics_net::next_foreground();
    std::thread::spawn(move || {
        let _lane = lyrics_net::enter(Lane::Foreground(generation));
        let started = Instant::now();
        let Some(outcome) = resolve(&req, AUTOMATIC_DELAY) else { return };
        crate::lyrics_log::note(&req.info, "Now playing", &outcome, started);
        if lyrics_net::live() {
            post(Event::Lyrics { key, generation, outcome });
        }
    });
    generation
}

/// Works out a song's lyrics on this thread's lane, keeping what it learns in the cache. None when
/// the lookup was replaced on the way.
pub fn resolve(req: &Request, delay: Duration) -> Option<Outcome> {
    let key = req.info.track_key();
    let alias = lyrics_cache::alias_key(&req.info.title, &req.info.artist);
    let store_key = if req.alias { alias } else { key };
    let (opts, first) = (req.opts, req.attempt == 0);
    if first {
        if let Some(f) = lyrics_local::find(&req.info) {
            return Some(Outcome::Found(f));
        }
        if opts.cache {
            let load = |k: u64| lyrics_cache::load(k, req.info.duration_ms, opts.fallback);
            // Lyrics marked wrong are not shown again, even from the cache.
            let cached = load(store_key).or_else(|| load(alias)).filter(|c| !matches!(c, Cached::Lyrics(f) if req.skip.contains(&f.id)));
            match cached {
                Some(Cached::Lyrics(f)) => return Some(Outcome::Found(with_extras(store_key, &req.info, f, opts))),
                Some(Cached::Instrumental) => return Some(Outcome::Instrumental),
                // A previous miss must not prevent the current song's configured search rounds.
                _ => {}
            }
        }
    }
    if req.automatic && !delay.is_zero() && !lyrics_net::sleep(delay) {
        return None;
    }
    let mut native = None;
    if req.verify && opts.fallback {
        let verified = localized::verify(&req.info);
        if !lyrics_net::live() {
            return None;
        }
        match verified {
            Verified::Song(names) => {
                crate::song_gate::remember(key, true);
                native = names;
            }
            Verified::NotSong => {
                // Catalog coverage is incomplete; the lyrics services may still have this song.
            }
            // The catalog could not be asked: search as for any song rather than give up.
            Verified::Unknown => {}
        }
    }
    let outcome = search_until(req, native)?;
    if opts.cache && lyrics_net::live() {
        match &outcome {
            // Unsynced lyrics are not kept: a synced version may turn up later, from either service.
            Outcome::Found(f) if f.synced => lyrics_cache::store(store_key, req.info.duration_ms, f),
            Outcome::Instrumental => lyrics_cache::store_empty(store_key, req.info.duration_ms, Kind::Instrumental, opts.fallback),
            // Only when every service answered: a failed request says nothing about the song.
            Outcome::NotFound if first && !lyrics_net::any_failed() => lyrics_cache::store_empty(store_key, req.info.duration_ms, Kind::Miss, opts.fallback),
            _ => {}
        }
    }
    if !lyrics_net::live() {
        return None;
    }
    Some(match outcome {
        Outcome::Found(f) => Outcome::Found(with_extras(store_key, &req.info, f, opts)),
        other => other,
    })
}

/// Whether synced lyrics lack the romaji and translation, or the word timing, that are wanted.
/// Your own files never send anything, so they never ask.
fn needs_extras(f: &Found, opts: Options) -> bool {
    let lacks_extras = opts.extras && f.translation.is_empty() && f.romaji.is_empty();
    let lacks_words = opts.words && !f.has_words();
    opts.fallback && f.synced && f.source != Source::Local && (lacks_extras || lacks_words)
}

/// LRCLIB has no romaji or translations, and word timing only for some songs, so for lyrics from there
/// those are taken from the same song on NetEase. Only those are used: the lines stay as they are, and
/// the extras and words are matched to them by time (and, for words, by text).
fn with_extras(key: u64, info: &MediaInfo, mut found: Found, opts: Options) -> Found {
    if !needs_extras(&found, opts) {
        return found;
    }
    if EXTRAS_ASKED.lock().unwrap_or_else(|e| e.into_inner()).contains(&(key, opts.words)) {
        return found;
    }
    let names = names_for(info, true);
    let looked_up = netease::lookup(&names, info.duration_ms, 1, opts.words && !found.has_words(), &[]).into_iter().next();
    // Counted as asked only when the services answered: a failure or a skip may be asked again.
    if lyrics_net::live() && !lyrics_net::any_failed() {
        EXTRAS_ASKED.lock().unwrap_or_else(|e| e.into_inner()).push((key, opts.words));
    }
    let Some(n) = looked_up else { return found };
    merge_extras(&mut found, n, opts, info.duration_ms);
    if opts.cache && lyrics_net::live() {
        lyrics_cache::store(key, info.duration_ms, &found);
    }
    found
}

/// Takes what `found` lacks and `n` has: romaji and a translation when they are wanted, and word timing.
fn merge_extras(found: &mut Found, n: netease::Lyrics, opts: Options, duration_ms: u64) {
    if opts.extras && found.translation.is_empty() && found.romaji.is_empty() {
        found.translation = n.translation;
        found.romaji = n.romaji;
    }
    if opts.words && !found.has_words() {
        found.words = kept(&found.lines, words::attach(&found.lines, &n.timed), duration_ms);
    }
}

/// The pause before search round `round` (1 = the first retry): doubling, with a little jitter so
/// many copies of the app do not ask in step.
fn retry_pause(round: u32, jitter_ms: u64) -> Duration {
    let doubled = RETRY_PAUSE.saturating_mul(1 << round.saturating_sub(1).min(5));
    doubled.min(RETRY_PAUSE_MAX) + Duration::from_millis(jitter_ms % 250)
}

fn jitter() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_millis() as u64)
}

/// Searches until lyrics are found or the configured retry limit is reached.
fn search_until(req: &Request, native: Option<Names>) -> Option<Outcome> {
    // "Search again" (attempt > 0) looks once; a song's first search gets its retries.
    let rounds = if req.attempt == 0 { 1 + req.opts.retries } else { 1 };
    search_rounds(rounds, || {
        if !lyrics_net::live() {
            return None;
        }
        lyrics_net::reset_failed();
        let outcome = if req.attempt == 0 { lookup(req, native.clone()) } else { lookup_other(req) };
        if matches!(outcome, Outcome::NotFound) && lyrics_net::any_failed() {
            crate::log("lyrics lookup incomplete; services failed to answer");
        }
        Some(outcome)
    }, |round| lyrics_net::sleep(retry_pause(round, jitter())))
}

/// Shared retry lifecycle; injected lookup and wait keep regression checks off the network.
fn search_rounds(rounds: u32, mut lookup: impl FnMut() -> Option<Outcome>, mut wait: impl FnMut(u32) -> bool) -> Option<Outcome> {
    for round in 0..rounds {
        if round > 0 && !wait(round) {
            return None;
        }
        match lookup()? {
            Outcome::NotFound => {}
            other => return Some(other),
        }
    }
    Some(Outcome::NotFound)
}

/// The names to search under: the song as the player lists it and, when `resolve` is on and the
/// title is in Latin letters, its native-script names from Apple's store in Japan.
fn names_for(info: &MediaInfo, resolve: bool) -> Vec<Names> {
    let original = Names { title: clean_title(&info.title), artist: primary_artist(&info.artist) };
    let mut names = vec![original.clone()];
    if resolve {
        if let Some(native) = localized::lookup(&original, info.duration_ms).filter(|n| *n != original) {
            names.push(native);
        }
    }
    names
}

fn settled(r: &Option<Record>) -> bool {
    r.as_ref().is_some_and(|r| r.synced() || matches!(r, Record::Instrumental))
}

fn outcome_of(r: Option<Record>) -> Outcome {
    match r {
        Some(Record::Lyrics(f)) => Outcome::Found(f),
        Some(Record::Instrumental) => Outcome::Instrumental,
        None => Outcome::NotFound,
    }
}

/// Best lyrics for the song. LRCLIB is asked under the player's names first; the native-script names
/// are looked up only when that finds nothing synced. Synced lyrics win: plain LRCLIB lyrics are kept
/// only when nothing synced turns up, from LRCLIB or the fallback.
fn lookup(req: &Request, native: Option<Names>) -> Outcome {
    let (info, opts) = (&req.info, req.opts);
    let original = Names { title: clean_title(&info.title), artist: primary_artist(&info.artist) };
    let mut names = vec![original.clone()];
    let resolved = native.is_some();
    if let Some(n) = native.filter(|n| *n != original) {
        names.push(n);
    }
    let mut best = lrclib::best(info, &names, &req.skip);
    if !settled(&best) && opts.fallback && !resolved {
        if let Some(n) = localized::lookup(&original, info.duration_ms).filter(|n| *n != original) {
            if let Some(r) = lrclib::best(info, std::slice::from_ref(&n), &req.skip) {
                best = if r.synced() || matches!(r, Record::Instrumental) || best.is_none() { Some(r) } else { best };
            }
            names.push(n);
        }
    }
    if settled(&best) || !opts.fallback {
        return outcome_of(best);
    }
    match netease::lookup(&names, info.duration_ms, 1, opts.words, &req.skip).into_iter().next() {
        Some(n) => Outcome::Found(Found::from_netease(n, info.duration_ms)),
        None => outcome_of(best),
    }
}

/// Another candidate for "Search again": LRCLIB's records in order of fit, then a broader search by
/// title alone, then the fallback's songs.
fn lookup_other(req: &Request) -> Outcome {
    let info = &req.info;
    let names = names_for(info, req.opts.fallback);
    let mut usable = lrclib::candidates(info, &names, &req.skip);
    if req.opts.fallback {
        usable.extend(netease::lookup(&names, info.duration_ms, 3, req.opts.words, &req.skip).into_iter().map(|n| Found::from_netease(n, info.duration_ms)));
    }
    if usable.is_empty() {
        Outcome::NotFound
    } else {
        Outcome::Found(usable[req.attempt % usable.len()].clone())
    }
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

    fn synced(text: &str) -> Found {
        Found::from_lrclib(crate::lrc::parse(text), true, "lrclib:1".into())
    }

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
    fn wrong_lyrics_are_remembered_per_song_once_and_bounded() {
        let mut s = Shifts::default();
        assert!(!s.block(1, ""), "lyrics without an id cannot be marked");
        assert!(s.block(1, "lrclib:5"));
        s.block(1, "lrclib:5");
        assert_eq!(s.blocked(1), vec!["lrclib:5".to_string()]);
        assert!(s.blocked(2).is_empty());
        for i in 0..30 {
            s.block(1, &format!("netease:{i}"));
        }
        let list = s.blocked(1);
        assert_eq!(list.len(), MAX_BLOCKED);
        assert_eq!(list.last().unwrap(), "netease:29");
        // Old lyrics.json files, with shifts only, still load.
        let old: Shifts = serde_json::from_str(r#"{"ms":{"7":500}}"#).unwrap();
        assert_eq!((old.get(7), old.blocked(7).len()), (500, 0));
    }

    #[test]
    fn retry_pauses_double_up_to_a_ceiling_with_a_little_jitter() {
        assert_eq!(retry_pause(1, 0), Duration::from_secs(1));
        assert_eq!(retry_pause(2, 0), Duration::from_secs(2));
        assert_eq!(retry_pause(3, 0), Duration::from_secs(4));
        assert_eq!(retry_pause(9, 0), RETRY_PAUSE_MAX);
        assert_eq!(retry_pause(1, 999), Duration::from_millis(1249));
    }

    #[test]
    fn searches_retry_misses_until_found_or_the_configured_limit() {
        let mut calls = 0;
        let mut waits = Vec::new();
        let result = search_rounds(6, || {
            calls += 1;
            Some(if calls == 3 { Outcome::Found(synced("[00:01.00]found")) } else { Outcome::NotFound })
        }, |round| { waits.push(round); true });
        assert!(matches!(result, Some(Outcome::Found(_))));
        assert_eq!(calls, 3, "stop as soon as lyrics are found");
        assert_eq!(waits, vec![1, 2]);

        for retries in [0, 1, 5, crate::config::MAX_LYRICS_RETRIES] {
            let mut calls = 0;
            let result = search_rounds(1 + retries, || { calls += 1; Some(Outcome::NotFound) }, |_| true);
            assert!(matches!(result, Some(Outcome::NotFound)));
            assert_eq!(calls, 1 + retries, "initial round plus the saved retry count");
        }
    }

    #[test]
    fn retrying_stops_for_instrumentals_or_cancellation() {
        let result = search_rounds(6, || Some(Outcome::Instrumental), |_| panic!("instrumentals must not retry"));
        assert!(matches!(result, Some(Outcome::Instrumental)));
        let mut calls = 0;
        let result = search_rounds(6, || { calls += 1; Some(Outcome::NotFound) }, |_| false);
        assert!(result.is_none());
        assert_eq!(calls, 1, "cancelling during the wait prevents the next lookup");
        assert!(search_rounds(6, || None, |_| panic!("a cancelled lookup must not retry")).is_none());
    }

    #[test]
    fn only_synced_lyrics_without_extras_ask_for_them_when_wanted() {
        let opts = Options { fallback: true, cache: true, retries: 0, extras: true, words: false };
        let bare = synced("[00:01.00]a");
        assert!(needs_extras(&bare, opts));
        assert!(!needs_extras(&bare, Options { extras: false, ..opts }));
        // Word timing alone is reason enough, until the song has it.
        let words_only = Options { extras: false, words: true, ..opts };
        assert!(needs_extras(&bare, words_only));
        let timed = Found { words: vec![vec![Word { start_ms: 1000, dur_ms: 100, chars: 1 }]], ..bare.clone() };
        assert!(!needs_extras(&timed, words_only));
        assert!(!needs_extras(&bare, Options { fallback: false, ..opts }));
        let plain = Found::from_lrclib(crate::lrc::from_plain("a\nb", 100_000), false, String::new());
        assert!(!needs_extras(&plain, opts));
        let with = Found { translation: vec![Line { at_ms: 1000, text: "x".into() }], ..bare.clone() };
        assert!(!needs_extras(&with, opts));
        // Your own files send nothing anywhere.
        assert!(!needs_extras(&Found { source: Source::Local, ..bare }, opts));
    }

    #[test]
    fn extras_and_words_are_taken_only_where_missing_and_wanted() {
        let opts = Options { fallback: true, cache: true, retries: 0, extras: true, words: true };
        let lyrics = || netease::Lyrics {
            lines: Vec::new(),
            translation: vec![Line { at_ms: 1000, text: "hola".into() }],
            romaji: Vec::new(),
            timed: crate::words::parse_yrc("[1000,500](1000,250,0)hel(1250,250,0)lo"),
            id: "netease:1".into(),
        };
        let mut f = synced("[00:01.00]hello");
        merge_extras(&mut f, lyrics(), opts, 100_000);
        assert_eq!(f.translation.len(), 1);
        assert!(f.has_words());
        // What the song already has is left alone.
        let mut g = synced("[00:01.00]hello");
        merge_extras(&mut g, lyrics(), Options { extras: false, words: false, ..opts }, 100_000);
        assert!(g.translation.is_empty() && !g.has_words());
    }

    /// Uses a disposable data folder: `$env:APPDATA = ...\target\lyrics-toggle-test`, then
    /// `cargo test -- --ignored known_songs_resolve --test-threads=1`.
    #[test]
    #[ignore = "writes the lyrics cache; requires isolated APPDATA ending in lyrics-toggle-test"]
    fn known_songs_resolve_at_once_without_the_network() {
        assert!(crate::config::data_dir().parent().unwrap().ends_with("lyrics-toggle-test"));
        let opts = Options { fallback: true, cache: true, retries: 5, extras: false, words: false };
        let info = |title: &str| MediaInfo { title: title.into(), artist: "Cache Test".into(), duration_ms: 200_000, ..Default::default() };
        let req = |info: MediaInfo, automatic: bool| Request { info, attempt: 0, opts, automatic, verify: true, skip: Vec::new(), alias: false };
        let _lane = lyrics_net::enter(Lane::Foreground(lyrics_net::next_foreground()));
        let cached = info("Cached song");
        lyrics_cache::store(cached.track_key(), cached.duration_ms, &synced("[00:01.00]from the cache"));
        let missed = info("Song with nothing");
        lyrics_cache::remove(missed.track_key());
        lyrics_cache::store_empty(missed.track_key(), missed.duration_ms, Kind::Miss, true);
        // A song fetched ahead under a store's spelling is found under the player's.
        let ahead = info("cached song ahead");
        lyrics_cache::store(lyrics_cache::alias_key("Cached song ahead (Remastered)", "CACHE TEST"), 200_000, &synced("[00:01.00]ahead"));
        let started = Instant::now();
        let hit = resolve(&req(cached, true), AUTOMATIC_DELAY);
        assert!(matches!(hit, Some(Outcome::Found(ref f)) if f.lines[0].text == "from the cache"), "{hit:?}");
        assert!(matches!(resolve(&req(ahead, true), AUTOMATIC_DELAY), Some(Outcome::Found(ref f)) if f.lines[0].text == "ahead"));
        assert!(started.elapsed() < AUTOMATIC_DELAY, "no delay before what is already known");
        assert_eq!(lyrics_net::counts().0, 0, "and no request");
        // Lyrics marked wrong are not taken from the cache: the lookup goes on to the wait, and stops
        // there because the lane was cancelled, before any request.
        let wrong = info("Marked wrong");
        lyrics_cache::store(wrong.track_key(), 200_000, &synced("[00:01.00]wrong"));
        lyrics_net::cancel_foreground();
        assert!(resolve(&req(missed, true), AUTOMATIC_DELAY).is_none(), "a cached miss proceeds to the network wait, which cancellation stops");
        assert!(resolve(&Request { skip: vec!["lrclib:1".into()], ..req(wrong.clone(), true) }, AUTOMATIC_DELAY).is_none());
        assert_eq!(lyrics_net::counts().0, 0);
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
