//! Synced lyrics lookup on LRCLIB (lrclib.net). Only title, artist, album and length are sent.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use windows::core::{Result, HSTRING};
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;

use crate::events::{post, Event, MediaInfo};
use crate::lrc::{self, Line};

const API: &str = "https://lrclib.net/api";
const USER_AGENT: &str = "SuiNotch/1.0 (a living notch for Windows)";
const DURATION_TOLERANCE_S: f64 = 4.0;

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

/// Looks up lyrics; `attempt` 0 is the best match, each retry moves on to another candidate.
pub fn fetch(info: MediaInfo, attempt: usize) {
    std::thread::spawn(move || {
        let key = info.track_key();
        let found = if attempt == 0 { lookup(&info) } else { lookup_other(&info, attempt) };
        let lines = found.unwrap_or_else(|e| {
            crate::log(&format!("lyrics lookup failed: {e}"));
            Vec::new()
        });
        post(Event::Lyrics { key, lines });
    });
}

fn lookup(info: &MediaInfo) -> Result<Vec<Line>> {
    let client = HttpClient::new()?;
    let _ = client.DefaultRequestHeaders()?.UserAgent()?.TryParseAdd(&HSTRING::from(USER_AGENT));
    let title = clean_title(&info.title);
    let artist = primary_artist(&info.artist);
    let duration_s = info.duration_ms as f64 / 1000.0;

    let mut url = format!("{API}/get?track_name={}&artist_name={}", encode(&title), encode(&artist));
    if !info.album.is_empty() {
        url.push_str(&format!("&album_name={}", encode(&info.album)));
    }
    if duration_s > 0.0 {
        url.push_str(&format!("&duration={}", duration_s.round() as u64));
    }
    if let Ok(body) = get(&client, &url) {
        if let Some(lines) = serde_json::from_str::<Value>(&body).ok().and_then(|v| lines_from(&v, info.duration_ms)) {
            return Ok(lines);
        }
    }

    let url = format!("{API}/search?track_name={}&artist_name={}", encode(&title), encode(&artist));
    let body = get(&client, &url)?;
    let results: Vec<Value> = serde_json::from_str(&body).unwrap_or_default();
    Ok(pick_best(&results, duration_s).and_then(|v| lines_from(v, info.duration_ms)).unwrap_or_default())
}

/// Another candidate for "Search again": the search results in order of fit, then a broader
/// search by title alone.
fn lookup_other(info: &MediaInfo, attempt: usize) -> Result<Vec<Line>> {
    let client = HttpClient::new()?;
    let _ = client.DefaultRequestHeaders()?.UserAgent()?.TryParseAdd(&HSTRING::from(USER_AGENT));
    let title = clean_title(&info.title);
    let artist = primary_artist(&info.artist);
    let duration_s = info.duration_ms as f64 / 1000.0;
    let mut candidates: Vec<Value> = Vec::new();
    for url in [
        format!("{API}/search?track_name={}&artist_name={}", encode(&title), encode(&artist)),
        format!("{API}/search?q={}", encode(&title)),
    ] {
        let body = get(&client, &url).unwrap_or_default();
        let results: Vec<Value> = serde_json::from_str(&body).unwrap_or_default();
        for v in ranked(&results, duration_s) {
            if !candidates.iter().any(|c| c.get("id") == v.get("id")) {
                candidates.push(v.clone());
            }
        }
    }
    let usable: Vec<Vec<Line>> = candidates.iter().filter_map(|v| lines_from(v, info.duration_ms)).collect();
    Ok(if usable.is_empty() { Vec::new() } else { usable[attempt % usable.len()].clone() })
}

/// Results ordered: synced and the right length, synced, the right length, then the rest.
fn ranked(results: &[Value], duration_s: f64) -> Vec<&Value> {
    let fits = |v: &Value| duration_s <= 0.0 || v.get("duration").and_then(Value::as_f64).is_none_or(|d| (d - duration_s).abs() <= DURATION_TOLERANCE_S);
    let synced = |v: &Value| v.get("syncedLyrics").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    let mut v: Vec<&Value> = results.iter().collect();
    v.sort_by_key(|r| (!(synced(r) && fits(r)), !synced(r), !fits(r)));
    v
}

fn get(client: &HttpClient, url: &str) -> Result<String> {
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    Ok(client.GetStringAsync(&uri)?.get()?.to_string())
}

fn lines_from(v: &Value, duration_ms: u64) -> Option<Vec<Line>> {
    if let Some(synced) = v.get("syncedLyrics").and_then(Value::as_str) {
        let lines = lrc::parse(synced);
        if !lines.is_empty() {
            return Some(lines);
        }
    }
    let plain = v.get("plainLyrics").and_then(Value::as_str)?;
    let lines = lrc::from_plain(plain, duration_ms);
    (!lines.is_empty()).then_some(lines)
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
}
