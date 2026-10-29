//! Synced lyrics lookup on LRCLIB (lrclib.net). Only title, artist, album and length are sent.

use serde_json::Value;
use windows::core::{Result, HSTRING};
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;

use crate::events::{post, Event, MediaInfo};
use crate::lrc::{self, Line};

const API: &str = "https://lrclib.net/api";
const USER_AGENT: &str = "SuiNotch/1.0 (a living notch for Windows)";
const DURATION_TOLERANCE_S: f64 = 4.0;

pub fn fetch(info: MediaInfo) {
    std::thread::spawn(move || {
        let key = info.track_key();
        let lines = lookup(&info).unwrap_or_else(|e| {
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
    fn best_result_prefers_synced_matching_length() {
        let r = vec![
            json!({"duration": 200.0, "plainLyrics": "x"}),
            json!({"duration": 400.0, "syncedLyrics": "[00:01.00]far"}),
            json!({"duration": 201.0, "syncedLyrics": "[00:01.00]near"}),
        ];
        assert_eq!(pick_best(&r, 200.0).unwrap()["syncedLyrics"], "[00:01.00]near");
    }
}
