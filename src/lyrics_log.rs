//! The last few lyric lookups, for Settings > Diagnostics: what was looked up, why, how it ended, how
//! many requests it took and how long. Kept in memory only.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use crate::events::MediaInfo;
use crate::lyrics::Outcome;

const KEPT: usize = 10;

#[derive(Clone, Debug)]
pub struct Lookup {
    pub at: i64,
    pub title: String,
    /// "Now playing" or "Ahead".
    pub kind: &'static str,
    pub result: String,
    pub requests: u32,
    pub ms: u64,
    pub waited_ms: u64,
}

static LOG: Mutex<VecDeque<Lookup>> = Mutex::new(VecDeque::new());

/// Words for how a lookup ended.
pub fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Found(f) => {
            let timing = if f.has_words() { "word-synced" } else if f.synced { "synced" } else { "unsynced" };
            format!("{} · {timing}", f.source.name())
        }
        Outcome::Instrumental => "Instrumental".to_string(),
        Outcome::NotFound if crate::lyrics_net::any_failed() => "Nothing found (a service did not answer)".to_string(),
        Outcome::NotFound => "Nothing found".to_string(),
    }
}

/// Notes a finished lookup run on this thread's lane.
pub fn note(info: &MediaInfo, kind: &'static str, outcome: &Outcome, started: Instant) {
    let (requests, waited_ms) = crate::lyrics_net::counts();
    push(Lookup {
        at: crate::store::now(),
        title: format!("{} - {}", info.artist, info.title),
        kind,
        result: describe(outcome),
        requests,
        ms: started.elapsed().as_millis() as u64,
        waited_ms,
    });
}

/// Notes a lookup ahead that was not made, and why.
pub fn skipped(title: String, why: &str) {
    push(Lookup { at: crate::store::now(), title, kind: "Ahead", result: why.to_string(), requests: 0, ms: 0, waited_ms: 0 });
}

fn push(entry: Lookup) {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if log.len() >= KEPT {
        log.pop_front();
    }
    log.push_back(entry);
}

/// Newest first.
pub fn recent() -> Vec<Lookup> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_few_are_kept_newest_first() {
        for i in 0..KEPT + 3 {
            skipped(format!("song {i}"), "test");
        }
        let r = recent();
        assert!(r.len() <= KEPT);
        assert_eq!(r[0].title, format!("song {}", KEPT + 2));
    }

    #[test]
    fn outcomes_read_naturally() {
        assert_eq!(describe(&Outcome::Instrumental), "Instrumental");
        let f = crate::lyrics::Found { lines: Vec::new(), source: crate::lyrics::Source::Lrclib, synced: true, translation: Vec::new(), romaji: Vec::new(), words: Vec::new(), id: String::new() };
        assert_eq!(describe(&Outcome::Found(f)), "LRCLIB · synced");
    }
}
