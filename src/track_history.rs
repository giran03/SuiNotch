//! What tends to play after what, learned from your own listening and kept on this PC only
//! (transitions.json), so the next song's lyrics can be fetched before it starts (see `lyrics_prefetch`).
//! Albums and playlists played in order teach it quickly; shuffle teaches it nothing, by design.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::events::MediaInfo;

/// Most tracks remembered; the ones heard longest ago go first.
const MAX_TRACKS: usize = 3000;
/// Most next songs remembered per track.
const MAX_NEXT: usize = 4;
/// Scores halve over this long, so old playlists fade.
const HALF_LIFE_S: f64 = 90.0 * 24.0 * 3600.0;
/// A song played to here (or with this little left) ended on its own, which counts double.
const NATURAL_SHARE: f64 = 0.8;
const NATURAL_LEFT_MS: u64 = 15_000;
/// A prediction needs this score, and this lead over the runner-up.
const MIN_SCORE: f64 = 2.0;
const MIN_LEAD: f64 = 1.5;
const SLACK: f64 = 0.01;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Next {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub score: f64,
    /// Unix seconds the score stands at; it decays from then.
    pub last: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Track {
    pub last: i64,
    pub next: Vec<Next>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct History {
    /// By the earlier track's key, in hex.
    pub after: HashMap<String, Track>,
}

fn decayed(score: f64, since: i64, now: i64) -> f64 {
    score * 0.5f64.powf((now - since).max(0) as f64 / HALF_LIFE_S)
}

fn key_of(info: &MediaInfo) -> String {
    format!("{:016x}", info.track_key())
}

impl History {
    /// Notes that `next` started after `prev`, which played for `played_ms`. Returns true when
    /// something changed and should be saved.
    pub fn record(&mut self, prev: &MediaInfo, played_ms: u64, next: &MediaInfo, now: i64) -> bool {
        if prev.title.trim().is_empty() || next.title.trim().is_empty() || prev.track_key() == next.track_key() {
            return false;
        }
        let natural = prev.duration_ms > 0
            && (played_ms as f64 >= prev.duration_ms as f64 * NATURAL_SHARE || prev.duration_ms.saturating_sub(played_ms) <= NATURAL_LEFT_MS);
        let weight = if natural { 2.0 } else { 1.0 };
        let track = self.after.entry(key_of(prev)).or_default();
        track.last = now;
        for n in &mut track.next {
            n.score = decayed(n.score, n.last, now);
            n.last = now;
        }
        match track.next.iter_mut().find(|n| n.title == next.title && n.artist == next.artist) {
            Some(n) => {
                n.score += weight;
                n.last = now;
                n.album = next.album.clone();
                n.duration_ms = next.duration_ms;
            }
            None => track.next.push(Next { title: next.title.clone(), artist: next.artist.clone(), album: next.album.clone(), duration_ms: next.duration_ms, score: weight, last: now }),
        }
        track.next.sort_by(|a, b| b.score.total_cmp(&a.score));
        track.next.truncate(MAX_NEXT);
        self.prune();
        true
    }

    fn prune(&mut self) {
        if self.after.len() <= MAX_TRACKS {
            return;
        }
        let mut by_age: Vec<(i64, String)> = self.after.iter().map(|(k, t)| (t.last, k.clone())).collect();
        by_age.sort();
        for (_, k) in by_age.into_iter().take(self.after.len() - MAX_TRACKS) {
            self.after.remove(&k);
        }
    }

    /// The song that clearly tends to follow `info`, if one does.
    pub fn predict(&self, info: &MediaInfo, now: i64) -> Option<&Next> {
        let track = self.after.get(&key_of(info))?;
        let mut scored: Vec<(f64, &Next)> = track.next.iter().map(|n| (decayed(n.score, n.last, now), n)).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (top, next) = *scored.first()?;
        let second = scored.get(1).map_or(0.0, |s| s.0);
        // A little slack, so two plays minutes apart (decayed by a hair) still count as two.
        (top >= MIN_SCORE - SLACK && top >= second * MIN_LEAD).then_some(next)
    }

    pub fn clear(&mut self) {
        self.after.clear();
    }

    pub fn len(&self) -> usize {
        self.after.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str) -> MediaInfo {
        MediaInfo { title: title.into(), artist: "Band".into(), duration_ms: 200_000, ..Default::default() }
    }

    const DAY: i64 = 24 * 3600;

    #[test]
    fn songs_played_through_teach_faster_than_skips() {
        let mut h = History::default();
        let (a, b) = (song("A"), song("B"));
        h.record(&a, 190_000, &b, 0);
        assert_eq!(h.predict(&a, 0).map(|n| n.title.as_str()), Some("B"), "one natural transition is enough");
        let mut skips = History::default();
        skips.record(&a, 20_000, &b, 0);
        assert!(skips.predict(&a, 0).is_none(), "one skip is not");
        skips.record(&a, 20_000, &b, 10);
        assert!(skips.predict(&a, 10).is_some(), "two are");
    }

    #[test]
    fn shuffle_predicts_nothing() {
        let mut h = History::default();
        let a = song("A");
        for (i, next) in ["B", "C", "D", "B", "C", "D"].iter().enumerate() {
            h.record(&a, 195_000, &song(next), i as i64);
        }
        assert!(h.predict(&a, 10).is_none());
        // One clear favourite does predict.
        for i in 0..4 {
            h.record(&a, 195_000, &song("B"), 20 + i);
        }
        assert_eq!(h.predict(&a, 30).unwrap().title, "B");
    }

    #[test]
    fn old_habits_fade_and_lists_stay_small() {
        let mut h = History::default();
        let a = song("A");
        h.record(&a, 195_000, &song("Old"), 0);
        assert!(h.predict(&a, 200 * DAY).is_none(), "faded below the bar");
        for (i, n) in ["P", "Q", "R", "S", "T"].iter().enumerate() {
            h.record(&a, 195_000, &song(n), 300 * DAY + i as i64);
        }
        assert_eq!(h.after.values().next().unwrap().next.len(), MAX_NEXT);
        assert!(!h.record(&a, 195_000, &a, 0), "a song repeating is no transition");
        assert!(!h.record(&song(""), 0, &a, 0));
    }

    #[test]
    fn the_least_recent_tracks_go_first_past_the_limit() {
        let mut h = History::default();
        for i in 0..MAX_TRACKS + 5 {
            h.record(&song(&format!("t{i}")), 195_000, &song("x"), i as i64);
        }
        assert_eq!(h.len(), MAX_TRACKS);
        assert!(h.predict(&song("t0"), 0).is_none());
        assert!(h.after.contains_key(&key_of(&song(&format!("t{}", MAX_TRACKS + 4)))));
    }

    #[test]
    fn history_round_trips_and_old_files_load() {
        let mut h = History::default();
        h.record(&song("A"), 195_000, &song("B"), 5);
        let back: History = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back.after, h.after);
        assert!(serde_json::from_str::<History>("{}").unwrap().after.is_empty());
    }
}
