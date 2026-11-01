//! Lyrics kept on disk, one small file per song, so a song already looked up shows its lyrics
//! at once, offline, and without asking a lyrics service again.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::data_dir;
use crate::lrc::Line;
use crate::lyrics::{Found, Source};

const DIR: &str = "lyrics_cache";
const MAX_FILES: usize = 400;
/// How many of the oldest files go when the limit is passed, so pruning is not done on every save.
const PRUNE_BATCH: usize = 50;
/// A cached song is reused only if its length is this close to the one playing (another edit of the song otherwise).
const DURATION_TOLERANCE_MS: u64 = 4000;

#[derive(Serialize, Deserialize)]
struct Entry {
    source: Source,
    synced: bool,
    duration_ms: u64,
    lines: Vec<(u64, String)>,
    #[serde(default)]
    translation: Vec<(u64, String)>,
    #[serde(default)]
    romaji: Vec<(u64, String)>,
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

pub fn load(key: u64, duration_ms: u64) -> Option<Found> {
    let text = std::fs::read_to_string(path(key)).ok()?;
    let entry: Entry = serde_json::from_str(&text).ok()?;
    if !fits(entry.duration_ms, duration_ms) || entry.lines.is_empty() {
        return None;
    }
    Some(Found {
        lines: from_pairs(entry.lines),
        source: entry.source,
        synced: entry.synced,
        translation: from_pairs(entry.translation),
        romaji: from_pairs(entry.romaji),
    })
}

pub fn store(key: u64, duration_ms: u64, found: &Found) {
    let entry = Entry {
        source: found.source,
        synced: found.synced,
        duration_ms,
        lines: to_pairs(&found.lines),
        translation: to_pairs(&found.translation),
        romaji: to_pairs(&found.romaji),
    };
    let Ok(text) = serde_json::to_string(&entry) else { return };
    if let Err(e) = std::fs::create_dir_all(dir()).and_then(|_| std::fs::write(path(key), text)) {
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
        for (_, p) in list.into_iter().take(PRUNE_BATCH + 1) {
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
            source: Source::Netease,
            synced: true,
            duration_ms: 1000,
            lines: vec![(500, "hi".into())],
            translation: vec![(500, "salut".into())],
            romaji: Vec::new(),
        };
        let back: Entry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!((back.source, back.synced, back.lines), (Source::Netease, true, vec![(500, "hi".to_string())]));
        assert_eq!(back.translation, vec![(500, "salut".to_string())]);
    }

    #[test]
    fn entries_saved_before_translations_still_load() {
        let old = r#"{"source":"Lrclib","synced":true,"duration_ms":1000,"lines":[[1,"a"]]}"#;
        let e: Entry = serde_json::from_str(old).unwrap();
        assert!(e.translation.is_empty() && e.romaji.is_empty());
    }
}
