//! Lyrics kept on disk, one small file per song, so a song already looked up shows its lyrics
//! at once, offline, and without asking a lyrics service again. A song the services had nothing for
//! is kept too, for a few days, so it is not searched for on every play; an instrumental for good.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::data_dir;
use crate::lrc::Line;
use crate::lyrics::{Found, Source};
use crate::words::Word;

const DIR: &str = "lyrics_cache";
const MAX_FILES: usize = 400;
/// How many of the oldest files go when the limit is passed, so pruning is not done on every save.
const PRUNE_BATCH: usize = 50;
/// A cached song is reused only if its length is this close to the one playing (another edit of the song otherwise).
const DURATION_TOLERANCE_MS: u64 = 4000;
/// How long "nothing found" is believed. LRCLIB fills in missing songs in the background, so not forever.
const MISS_TTL_S: i64 = 3 * 24 * 3600;

/// What a cache file holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Lyrics,
    /// The services had nothing for the song when it was last looked up.
    Miss,
    /// The song has no words.
    Instrumental,
}

/// A song as the cache knows it.
#[derive(Debug)]
pub enum Cached {
    Lyrics(Found),
    Instrumental,
    Miss,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    kind: Kind,
    /// Unix seconds when it was saved; 0 in files from before misses were kept.
    #[serde(default)]
    at: i64,
    /// Kept with "Keep these": never replaced by another lookup, nor pruned.
    #[serde(default)]
    pinned: bool,
    /// The service's id for the lyrics, such as "lrclib:390"; empty when unknown.
    #[serde(default)]
    id: String,
    /// For a miss: whether the fallback services were asked too. A miss from LRCLIB alone says
    /// nothing once the fallback is turned on.
    #[serde(default)]
    fallback: bool,
    source: Source,
    synced: bool,
    duration_ms: u64,
    lines: Vec<(u64, String)>,
    #[serde(default)]
    translation: Vec<(u64, String)>,
    #[serde(default)]
    romaji: Vec<(u64, String)>,
    /// Per line, its words as (start ms, duration ms, characters); empty when the song has no word timing.
    #[serde(default)]
    words: Vec<Vec<(u64, u64, usize)>>,
}

fn words_to_tuples(words: &[Vec<Word>]) -> Vec<Vec<(u64, u64, usize)>> {
    words.iter().map(|line| line.iter().map(|w| (w.start_ms, w.dur_ms, w.chars)).collect()).collect()
}

fn words_from_tuples(words: Vec<Vec<(u64, u64, usize)>>) -> Vec<Vec<Word>> {
    words.into_iter().map(|line| line.into_iter().map(|(start_ms, dur_ms, chars)| Word { start_ms, dur_ms, chars }).collect()).collect()
}

fn to_pairs(lines: &[Line]) -> Vec<(u64, String)> {
    lines.iter().map(|l| (l.at_ms, l.text.clone())).collect()
}

fn from_pairs(pairs: Vec<(u64, String)>) -> Vec<Line> {
    pairs.into_iter().map(|(at_ms, text)| Line { at_ms, text }).collect()
}

fn dir() -> PathBuf {
    data_dir().join(DIR)
}

fn path(key: u64) -> PathBuf {
    dir().join(format!("{key:016x}.json"))
}

fn fits(cached_ms: u64, playing_ms: u64) -> bool {
    cached_ms == 0 || playing_ms == 0 || cached_ms.abs_diff(playing_ms) <= DURATION_TOLERANCE_MS
}

fn read(key: u64) -> Option<Entry> {
    serde_json::from_str(&std::fs::read_to_string(path(key)).ok()?).ok()
}

/// The song as cached, if it is there, fits the length playing and (for a miss) is still recent and
/// was looked for everywhere a lookup with `fallback` would look.
pub fn load(key: u64, duration_ms: u64, fallback: bool) -> Option<Cached> {
    cached(read(key)?, duration_ms, crate::store::now(), fallback)
}

/// A second key for a song, from its names with case, punctuation and decorations set aside, so
/// lyrics fetched ahead under a store's spelling ("Song (Remastered)", "Artist feat. X") are found
/// under the player's.
pub fn alias_key(title: &str, artist: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let squash = |s: String| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    "alias".hash(&mut h);
    squash(crate::lyrics::clean_title(title)).hash(&mut h);
    squash(crate::lyrics::primary_artist(artist)).hash(&mut h);
    h.finish()
}

fn cached(entry: Entry, duration_ms: u64, now: i64, fallback: bool) -> Option<Cached> {
    if !fits(entry.duration_ms, duration_ms) {
        return None;
    }
    match entry.kind {
        Kind::Miss => (now - entry.at < MISS_TTL_S && (entry.fallback || !fallback)).then_some(Cached::Miss),
        Kind::Instrumental => Some(Cached::Instrumental),
        Kind::Lyrics if entry.lines.is_empty() => None,
        Kind::Lyrics => Some(Cached::Lyrics(Found {
            lines: from_pairs(entry.lines),
            source: entry.source,
            synced: entry.synced,
            translation: from_pairs(entry.translation),
            romaji: from_pairs(entry.romaji),
            words: words_from_tuples(entry.words),
            id: entry.id,
        })),
    }
}

/// Keeps a song's lyrics, unless it holds other lyrics the user chose to keep.
pub fn store(key: u64, duration_ms: u64, found: &Found) {
    let pinned = read(key).filter(|e| e.pinned);
    if pinned.as_ref().is_some_and(|e| found.id.is_empty() || e.id != found.id) {
        return;
    }
    write(key, &Entry {
        kind: Kind::Lyrics,
        at: crate::store::now(),
        pinned: pinned.is_some(),
        id: found.id.clone(),
        fallback: false,
        source: found.source,
        synced: found.synced,
        duration_ms,
        lines: to_pairs(&found.lines),
        translation: to_pairs(&found.translation),
        romaji: to_pairs(&found.romaji),
        words: words_to_tuples(&found.words),
    });
}

/// Notes that the services had nothing for a song, or that it has no words. Lyrics already kept
/// for it stay: a later search finding nothing is no reason to forget them.
pub fn store_empty(key: u64, duration_ms: u64, kind: Kind, fallback: bool) {
    if read(key).is_some_and(|e| e.kind == Kind::Lyrics && !e.lines.is_empty()) {
        return;
    }
    write(key, &Entry { fallback, ..empty_entry(kind, crate::store::now(), duration_ms) });
}

fn empty_entry(kind: Kind, at: i64, duration_ms: u64) -> Entry {
    Entry { kind, at, pinned: false, id: String::new(), fallback: false, source: Source::Lrclib, synced: false, duration_ms, lines: Vec::new(), translation: Vec::new(), romaji: Vec::new(), words: Vec::new() }
}

/// Keeps (or stops keeping) a song's cached lyrics through later lookups; false when none are cached.
pub fn set_pinned(key: u64, pinned: bool) -> bool {
    let Some(mut e) = read(key).filter(|e| e.kind == Kind::Lyrics) else { return false };
    e.pinned = pinned;
    write(key, &e);
    true
}

/// Whether these very lyrics (by id) are the ones kept for the song.
pub fn is_pinned(key: u64, id: &str) -> bool {
    !id.is_empty() && read(key).is_some_and(|e| e.pinned && e.id == id)
}

/// Forgets a song's cached lyrics if they are the ones with this id, so they are not shown again.
pub fn remove_if(key: u64, id: &str) {
    if read(key).is_some_and(|e| e.id == id) {
        let _ = std::fs::remove_file(path(key));
    }
}

#[cfg(test)]
pub fn remove(key: u64) {
    let _ = std::fs::remove_file(path(key));
}

/// Writes through a temporary file, so a crash mid-write never leaves half an entry (and a lost pin).
fn write(key: u64, entry: &Entry) {
    let Ok(text) = serde_json::to_string(entry) else { return };
    let tmp = path(key).with_extension("tmp");
    if let Err(e) = std::fs::create_dir_all(dir()).and_then(|_| std::fs::write(&tmp, text)).and_then(|_| std::fs::rename(&tmp, path(key))) {
        crate::log(&format!("cannot cache lyrics: {e}"));
        return;
    }
    prune();
}

/// Files in the cache, oldest first.
fn files() -> Vec<(std::time::SystemTime, PathBuf)> {
    let mut list: Vec<_> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    list.sort();
    list
}

fn prune() {
    let list = files();
    if list.len() > MAX_FILES {
        let kept = |p: &PathBuf| std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Entry>(&t).ok()).is_some_and(|e| e.pinned);
        for (_, p) in list.into_iter().filter(|(_, p)| !kept(p)).take(PRUNE_BATCH + 1) {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn count() -> usize {
    files().len()
}

/// Deletes every cached song; returns how many there were.
pub fn clear() -> usize {
    let list = files();
    let n = list.len();
    for (_, p) in list {
        let _ = std::fs::remove_file(p);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_song_lengths_fit_and_other_edits_do_not() {
        assert!(fits(200_000, 202_500));
        assert!(!fits(200_000, 260_000));
        assert!(fits(0, 260_000));
        assert!(fits(200_000, 0));
    }

    #[test]
    fn entries_round_trip_through_json() {
        let e = Entry {
            kind: Kind::Lyrics,
            at: 0,
            pinned: false,
            id: "netease:1".into(),
            fallback: false,
            source: Source::Netease,
            synced: true,
            duration_ms: 1000,
            lines: vec![(500, "hi".into())],
            translation: vec![(500, "salut".into())],
            romaji: Vec::new(),
            words: words_to_tuples(&[vec![Word { start_ms: 500, dur_ms: 120, chars: 2 }]]),
        };
        let back: Entry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(words_from_tuples(back.words.clone()), vec![vec![Word { start_ms: 500, dur_ms: 120, chars: 2 }]]);
        assert_eq!((back.source, back.synced, back.lines), (Source::Netease, true, vec![(500, "hi".to_string())]));
        assert_eq!(back.translation, vec![(500, "salut".to_string())]);
    }

    #[test]
    fn entries_saved_before_translations_still_load() {
        let old = r#"{"source":"Lrclib","synced":true,"duration_ms":1000,"lines":[[1,"a"]]}"#;
        let e: Entry = serde_json::from_str(old).unwrap();
        assert!(e.translation.is_empty() && e.romaji.is_empty() && e.words.is_empty());
        assert_eq!((e.kind, e.at, e.pinned, e.id.as_str()), (Kind::Lyrics, 0, false, ""));
    }

    #[test]
    fn misses_are_believed_for_a_few_days_and_instrumentals_for_good() {
        let now = 10_000_000;
        assert!(matches!(cached(empty_entry(Kind::Miss, now - 3600, 200_000), 200_000, now, false), Some(Cached::Miss)));
        assert!(cached(empty_entry(Kind::Miss, now - MISS_TTL_S - 1, 200_000), 200_000, now, false).is_none());
        assert!(matches!(cached(empty_entry(Kind::Instrumental, 0, 200_000), 200_000, now, true), Some(Cached::Instrumental)));
        // Another edit of the song is not this one, whatever was found for it.
        assert!(cached(empty_entry(Kind::Miss, now, 200_000), 300_000, now, false).is_none());
        assert!(cached(empty_entry(Kind::Lyrics, now, 200_000), 200_000, now, false).is_none(), "lyrics without lines are no lyrics");
    }

    #[test]
    fn a_miss_from_lrclib_alone_is_not_believed_once_the_fallback_is_on() {
        let now = 10_000_000;
        let lrclib_only = empty_entry(Kind::Miss, now, 200_000);
        assert!(cached(lrclib_only, 200_000, now, true).is_none());
        let everywhere = Entry { fallback: true, ..empty_entry(Kind::Miss, now, 200_000) };
        assert!(matches!(cached(everywhere, 200_000, now, true), Some(Cached::Miss)));
    }

    #[test]
    fn alias_keys_ignore_spelling_and_decorations() {
        assert_eq!(alias_key("Two (Remastered)", "Band feat. Guest"), alias_key("two", "BAND"));
        assert_ne!(alias_key("Two", "Band"), alias_key("Three", "Band"));
    }
}
