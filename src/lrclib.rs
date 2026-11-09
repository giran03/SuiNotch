//! LRCLIB (lrclib.net), the main lyrics service: an exact match by name and length, and a search,
//! asked at the same time. Its records can say a song is instrumental, and some carry word timing
//! in their `lyricsfile` (see `lyricsfile`).

use serde_json::Value;

use crate::events::MediaInfo;
use crate::lrc;
use crate::lyrics::{encode, kept, Found, Names};
use crate::{lyrics_net, lyricsfile, words};

const API: &str = "https://lrclib.net/api";
const DURATION_TOLERANCE_S: f64 = 4.0;

/// One LRCLIB record, as far as SuiNotch cares.
#[derive(Clone, Debug)]
pub enum Record {
    Lyrics(Found),
    /// The record says the song has no words.
    Instrumental,
}

impl Record {
    pub fn synced(&self) -> bool {
        matches!(self, Record::Lyrics(f) if f.synced)
    }
}

/// The id lyrics from a record are known by, such as "lrclib:390".
pub fn record_id(v: &Value) -> String {
    match v.get("id") {
        Some(Value::Number(n)) => format!("lrclib:{n}"),
        Some(Value::String(s)) => format!("lrclib:{s}"),
        _ => String::new(),
    }
}

fn blocked(v: &Value, ids: &[String]) -> bool {
    let id = record_id(v);
    !id.is_empty() && ids.contains(&id)
}

/// Keeps what is best of two weak results: plain lyrics over an instrumental from a loose search.
fn better(a: Option<Record>, b: Record) -> Option<Record> {
    match (a, b) {
        (Some(Record::Instrumental), b @ Record::Lyrics(_)) => Some(b),
        (None, b) => Some(b),
        (a, _) => a,
    }
}

/// The song on LRCLIB under each of its names: a synced match, or an exact instrumental one, at once;
/// otherwise the best plain match. Records in `skip` (lyrics the user said were wrong) are passed over.
pub fn best(info: &MediaInfo, names: &[Names], skip: &[String]) -> Option<Record> {
    let duration_s = info.duration_ms as f64 / 1000.0;
    let mut weak: Option<Record> = None;
    for n in names {
        let (exact, results) = exact_and_search(info, n);
        if let Some(r) = exact.filter(|v| !blocked(v, skip)).and_then(|v| record(&v, info.duration_ms)) {
            // An exact match by name and length that says "instrumental" is believed.
            if r.synced() || matches!(r, Record::Instrumental) {
                return Some(r);
            }
            weak = better(weak, r);
        }
        let results: Vec<Value> = results.into_iter().filter(|v| same_version(&n.title, v) && !blocked(v, skip)).collect();
        // An instrumental from a loose search may be another song altogether, so it is not believed.
        if let Some(r) = pick_best(&results, duration_s).and_then(|v| record(v, info.duration_ms)).filter(|r| !matches!(r, Record::Instrumental)) {
            if r.synced() {
                return Some(r);
            }
            weak = better(weak, r);
        }
    }
    weak
}

/// `/api/get` and `/api/search` for one name, asked side by side.
fn exact_and_search(info: &MediaInfo, n: &Names) -> (Option<Value>, Vec<Value>) {
    let ctx = lyrics_net::current();
    std::thread::scope(|s| {
        let exact = s.spawn(|| {
            if let Some(c) = ctx {
                lyrics_net::adopt(c);
            }
            exact(info, n)
        });
        let results = search(&format!("{API}/search?track_name={}&artist_name={}", encode(&n.title), encode(&n.artist)));
        (exact.join().unwrap_or(None), results)
    })
}

fn exact(info: &MediaInfo, n: &Names) -> Option<Value> {
    let mut url = format!("{API}/get?track_name={}&artist_name={}", encode(&n.title), encode(&n.artist));
    if !info.album.is_empty() {
        url.push_str(&format!("&album_name={}", encode(&info.album)));
    }
    let duration_s = (info.duration_ms as f64 / 1000.0).round() as u64;
    if (1..=3600).contains(&duration_s) {
        url.push_str(&format!("&duration={duration_s}"));
    }
    match lyrics_net::get(&url) {
        Ok(body) => serde_json::from_str(&body).ok(),
        Err(e) if e.is_not_found() => None,
        Err(e) => {
            crate::log(&format!("lrclib get failed: {e}"));
            None
        }
    }
}

/// A search's records; LRCLIB being down must not stop the fallback, so a failure is only logged.
fn search(url: &str) -> Vec<Value> {
    match lyrics_net::get(url) {
        Ok(body) => serde_json::from_str(&body).unwrap_or_default(),
        Err(e) => {
            if !e.is_not_found() {
                crate::log(&format!("lrclib search failed: {e}"));
            }
            Vec::new()
        }
    }
}

/// Every usable record for the song, best fit first, for "Search again": the searches by title and
/// artist, then broader ones by title alone.
pub fn candidates(info: &MediaInfo, names: &[Names], skip: &[String]) -> Vec<Found> {
    let duration_s = info.duration_ms as f64 / 1000.0;
    let mut seen: Vec<Value> = Vec::new();
    for n in names {
        for url in [
            format!("{API}/search?track_name={}&artist_name={}", encode(&n.title), encode(&n.artist)),
            format!("{API}/search?q={}", encode(&n.title)),
        ] {
            let results = search(&url);
            for v in ranked(&results, duration_s).into_iter().filter(|v| same_version(&n.title, v) && !blocked(v, skip)) {
                if !seen.iter().any(|c| c.get("id") == v.get("id")) {
                    seen.push(v.clone());
                }
            }
        }
    }
    seen.iter()
        .filter_map(|v| match record(v, info.duration_ms) {
            Some(Record::Lyrics(f)) => Some(f),
            _ => None,
        })
        .collect()
}

/// Whether a record is the same version of the song as `title` (not its live, remix or cover).
/// A record with no name says nothing, so it passes.
fn same_version(title: &str, record: &Value) -> bool {
    record.get("trackName").and_then(Value::as_str).is_none_or(|name| crate::versions::mismatch(title, name).is_none())
}

fn fits(v: &Value, duration_s: f64) -> bool {
    duration_s <= 0.0 || v.get("duration").and_then(Value::as_f64).is_none_or(|d| (d - duration_s).abs() <= DURATION_TOLERANCE_S)
}

fn has_synced(v: &Value) -> bool {
    v.get("syncedLyrics").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// Results ordered: synced and the right length, synced, the right length, then the rest.
fn ranked(results: &[Value], duration_s: f64) -> Vec<&Value> {
    let mut v: Vec<&Value> = results.iter().collect();
    v.sort_by_key(|r| (!(has_synced(r) && fits(r, duration_s)), !has_synced(r), !fits(r, duration_s)));
    v
}

/// Prefers synced lyrics whose length matches the playing track.
fn pick_best(results: &[Value], duration_s: f64) -> Option<&Value> {
    results
        .iter()
        .find(|v| fits(v, duration_s) && has_synced(v))
        .or_else(|| results.iter().find(|v| has_synced(v)))
        .or_else(|| results.iter().find(|v| fits(v, duration_s)))
}

/// What a record holds: synced lines (with word timing when its lyricsfile has some), plain lines
/// spread over the song, or the word that the song has none.
pub fn record(v: &Value, duration_ms: u64) -> Option<Record> {
    if v.get("instrumental").and_then(Value::as_bool) == Some(true) {
        return Some(Record::Instrumental);
    }
    let id = record_id(v);
    if let Some(synced) = v.get("syncedLyrics").and_then(Value::as_str) {
        let lines = lrc::parse(synced);
        if !lines.is_empty() {
            let mut f = Found::from_lrclib(lines, true, id);
            if v.get("hasWordSync").and_then(Value::as_bool) == Some(true) {
                if let Some(doc) = v.get("lyricsfile").and_then(Value::as_str) {
                    f.words = kept(&f.lines, words::attach(&f.lines, &lyricsfile::timed_lines(doc)), duration_ms);
                }
            }
            if !f.has_words() {
                f.words = crate::lyrics::lrc_words(&f.lines, synced, duration_ms);
            }
            return Some(Record::Lyrics(f));
        }
    }
    let plain = v.get("plainLyrics").and_then(Value::as_str)?;
    let lines = lrc::from_plain(plain, duration_ms);
    (!lines.is_empty()).then(|| Record::Lyrics(Found::from_lrclib(lines, false, id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::Source;
    use serde_json::json;

    fn lyrics(r: Option<Record>) -> Found {
        match r {
            Some(Record::Lyrics(f)) => f,
            other => panic!("expected lyrics, got {other:?}"),
        }
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
    fn records_say_whether_they_are_synced_and_carry_their_id() {
        let synced = lyrics(record(&json!({"id": 390, "syncedLyrics": "[00:01.00]a", "plainLyrics": "a"}), 100_000));
        assert!(synced.synced && synced.source == Source::Lrclib);
        assert_eq!(synced.id, "lrclib:390");
        assert_eq!(lyrics(record(&json!({"id": "7", "plainLyrics": "a\nb"}), 100_000)).id, "lrclib:7");
        assert!(!lyrics(record(&json!({"plainLyrics": "a\nb"}), 100_000)).synced);
        assert!(record(&json!({}), 100_000).is_none());
    }

    #[test]
    fn instrumental_records_are_told_apart() {
        let r = record(&json!({"instrumental": true, "plainLyrics": null, "syncedLyrics": null}), 100_000);
        assert!(matches!(r, Some(Record::Instrumental)));
        // A loose instrumental hit gives way to plain lyrics found later.
        let plain = record(&json!({"plainLyrics": "a"}), 100_000).unwrap();
        assert!(matches!(better(Some(Record::Instrumental), plain.clone()), Some(Record::Lyrics(_))));
        assert!(matches!(better(Some(plain), Record::Instrumental), Some(Record::Lyrics(_))));
    }

    #[test]
    fn word_timing_comes_from_a_word_synced_lyricsfile() {
        let doc = "lines:\n- text: Hello world\n  start_ms: 1000\n  end_ms: 3000\n  words:\n  - text: 'Hello '\n    start_ms: 1000\n    end_ms: 1800\n  - text: world\n    start_ms: 1800\n    end_ms: 3000\n";
        let v = json!({"id": 1, "syncedLyrics": "[00:01.00]Hello world\n[00:04.00]Next", "hasWordSync": true, "lyricsfile": doc});
        let f = lyrics(record(&v, 200_000));
        assert_eq!(f.words.first().map(Vec::len), Some(2));
        // Without the flag the document is not read.
        let v = json!({"id": 1, "syncedLyrics": "[00:01.00]Hello world", "hasWordSync": false, "lyricsfile": doc});
        assert!(!lyrics(record(&v, 200_000)).has_words());
    }

    /// Talks to LRCLIB: `cargo test -- --ignored live_lrclib --nocapture`.
    #[test]
    #[ignore]
    fn live_lrclib() {
        let info = MediaInfo { title: "Blinding Lights".into(), artist: "The Weeknd".into(), album: "After Hours".into(), duration_ms: 200_000, ..Default::default() };
        let names = [Names { title: info.title.clone(), artist: info.artist.clone() }];
        let _lane = lyrics_net::enter(lyrics_net::Lane::Foreground(lyrics_net::next_foreground()));
        let r = best(&info, &names, &[]);
        println!("{:?} after {:?} requests", r.as_ref().map(Record::synced), lyrics_net::counts());
        assert!(r.is_some_and(|r| r.synced()));
        // A song no one has: a 404 is an answer, not a failure. (LRCLIB is sometimes busy and answers
        // 503, which rightly counts as one, so that is reported rather than asserted.)
        let none = MediaInfo { title: "zzqx nonexistent song 93817".into(), artist: "nobody qqz".into(), duration_ms: 123_000, ..Default::default() };
        lyrics_net::reset_failed();
        assert!(best(&none, &[Names { title: none.title.clone(), artist: none.artist.clone() }], &[]).is_none());
        println!("missing song, a service failed to answer: {}", lyrics_net::any_failed());
    }

    #[test]
    fn records_the_user_rejected_are_skipped() {
        let v = json!({"id": 5});
        assert!(blocked(&v, &["lrclib:5".to_string()]));
        assert!(!blocked(&v, &["lrclib:6".to_string()]));
        assert!(!blocked(&json!({}), &["".to_string()]));
    }
}
