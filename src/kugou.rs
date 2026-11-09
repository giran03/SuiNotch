//! Lyrics and word timing from Kugou Music's lyrics endpoints, a fallback next to NetEase. Kugou's KRC
//! lyrics time every word, so songs whose lyrics have no word timing elsewhere still get a highlight
//! that follows the singer. Unofficial and undocumented, so every response is parsed defensively and
//! a failure is just "no lyrics".
//!
//! A KRC download is base64 of `krc1` followed by zlib data XORed with a fixed key. Inside, each line is
//! `[start,duration]<offset,duration,0>word<offset,duration,0>word`, with word offsets counted from the
//! start of the line, and a `[language:...]` tag may hold a translation and romaji, one row per line.

use serde_json::Value;
use windows::core::{Array, HSTRING};
use windows::Security::Cryptography::CryptographicBuffer;

use crate::lrc::Line;
use crate::lyrics::{encode, Names};
use crate::lyrics_net::get;
use crate::netease::{same_song, without_credits, Lyrics};
use crate::words::{TimedLine, TimedWord};

const API: &str = "https://lyrics.kugou.com";
const KRC_KEY: [u8; 16] = [64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105];
const KRC_MAGIC: &[u8] = b"krc1";
/// A song's lyrics are a few KB; anything inflating past this is not lyrics.
const MAX_KRC_BYTES: usize = 1 << 20;
/// A line at the very start naming the song ("Title - Artist") is a header, not a sung line.
const HEADER_END_MS: u64 = 1000;
/// The `[language:...]` rows: a translation, and romaji split into syllables.
const TRANSLATION: u64 = 1;
const ROMAJI: u64 = 0;

/// A search result that can be downloaded.
struct Candidate {
    id: String,
    key: String,
    off_ms: u64,
}

/// Lyrics for the best-fitting songs, best first, at most `limit` of them, searched under each of the
/// song's names until one finds a match. Songs whose ids are in `skip` (lyrics marked wrong) are passed over.
pub fn lookup(names: &[Names], duration_ms: u64, limit: usize, skip: &[String]) -> Vec<Lyrics> {
    let mut found: Vec<Candidate> = Vec::new();
    for keyword in keywords(names) {
        let url = format!("{API}/search?ver=1&man=yes&client=pc&keyword={}&duration={duration_ms}&hash=", encode(&keyword));
        match get(&url) {
            Ok(body) => {
                for c in candidates(&body, names, duration_ms) {
                    if !found.iter().any(|f| f.id == c.id) && !skip.contains(&format!("kugou:{}", c.id)) {
                        found.push(c);
                    }
                }
            }
            Err(e) => {
                crate::log(&format!("backup lyrics (2) search failed: {e}"));
                break;
            }
        }
        if !found.is_empty() {
            break;
        }
    }
    found.sort_by_key(|c| c.off_ms);
    found
        .into_iter()
        .filter_map(|c| {
            let url = format!("{API}/download?ver=1&client=pc&id={}&accesskey={}&fmt=krc&charset=utf8", encode(&c.id), encode(&c.key));
            let body = get(&url).map_err(|e| crate::log(&format!("backup lyrics (2) download failed: {e}"))).ok()?;
            let mut lyrics = parse_krc(&decode(&body)?)?;
            lyrics.id = format!("kugou:{}", c.id);
            Some(lyrics)
        })
        .take(limit)
        .collect()
}

/// What to search for: "artist - title" under each name, then the title alone.
fn keywords(names: &[Names]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let with_artist = if n.artist.is_empty() { n.title.clone() } else { format!("{} - {}", n.artist, n.title) };
        for q in [with_artist, n.title.clone()] {
            let q = q.trim().to_string();
            if !q.is_empty() && !out.contains(&q) {
                out.push(q);
            }
        }
    }
    out
}

/// The results of a search that are the song playing.
fn candidates(body: &str, names: &[Names], duration_ms: u64) -> Vec<Candidate> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    let Some(list) = v.get("candidates").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|c| {
            let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default();
            let (id, key) = (text("id"), text("accesskey"));
            if id.is_empty() || key.is_empty() {
                return None;
            }
            let length = c.get("duration").and_then(Value::as_u64).unwrap_or(0);
            let off_ms = same_song(names, text("song"), &[text("singer")], length, duration_ms)?;
            Some(Candidate { id: id.to_string(), key: key.to_string(), off_ms })
        })
        .collect()
}

/// The KRC text in a download response, None when it holds none or is not what it should be.
fn decode(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let content = v.get("content").and_then(Value::as_str).filter(|c| !c.is_empty())?;
    decrypt(&base64_bytes(content)?)
}

fn base64_bytes(b64: &str) -> Option<Vec<u8>> {
    let buffer = CryptographicBuffer::DecodeFromBase64String(&HSTRING::from(b64)).ok()?;
    let mut raw = Array::<u8>::new();
    CryptographicBuffer::CopyToByteArray(&buffer, &mut raw).ok()?;
    Some(raw.to_vec())
}

/// `krc1` and the XORed zlib data after it, as text.
fn decrypt(raw: &[u8]) -> Option<String> {
    let data = raw.strip_prefix(KRC_MAGIC)?;
    let plain: Vec<u8> = data.iter().enumerate().map(|(i, b)| b ^ KRC_KEY[i % KRC_KEY.len()]).collect();
    let text = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&plain, MAX_KRC_BYTES).ok()?;
    let text = String::from_utf8(text).ok()?;
    Some(text.trim_start_matches('\u{feff}').to_string())
}

/// The lines, word timing, translation and romaji in KRC text; None when it has no sung lines.
fn parse_krc(text: &str) -> Option<Lyrics> {
    let (translation_rows, romaji_rows) = language_rows(text);
    let rows: Vec<TimedLine> = text.lines().filter_map(krc_line).collect();
    // The rows of the language tag match the lines one to one; when their count differs they are not used.
    let translation_rows = if translation_rows.len() == rows.len() { translation_rows } else { Vec::new() };
    let romaji_rows = if romaji_rows.len() == rows.len() { romaji_rows } else { Vec::new() };
    let title = text.lines().find_map(|l| l.trim().strip_prefix("[ti:")?.strip_suffix(']')).unwrap_or_default().trim().to_lowercase();
    let is_header = |t: &TimedLine| t.start_ms < HEADER_END_MS && !title.is_empty() && t.text.to_lowercase().contains(&title);
    let (mut lines, mut timed, mut translation, mut romaji) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, t) in rows.into_iter().enumerate() {
        if t.text.is_empty() || is_header(&t) {
            continue;
        }
        let translated = translation_rows.get(i).map(|p| joined(p)).filter(|s| !s.is_empty());
        let romanised = romaji_rows.get(i).map(|p| romaji_line(&t.words, p)).filter(|s| !s.is_empty());
        for (text, out) in [(translated, &mut translation), (romanised, &mut romaji)] {
            if let Some(text) = text {
                out.push(Line { at_ms: t.start_ms, text });
            }
        }
        lines.push(Line { at_ms: t.start_ms, text: t.text.clone() });
        timed.push(t);
    }
    let lines = without_credits(lines);
    let first = lines.first()?.at_ms;
    timed.retain(|t| t.start_ms >= first);
    translation.retain(|l| l.at_ms >= first);
    romaji.retain(|l| l.at_ms >= first);
    Some(Lyrics { lines, translation: crate::lrc::english_only(translation), romaji, timed, id: String::new() })
}

/// One `[start,duration]<offset,duration,0>word...` line, its words placed in the song's time.
fn krc_line(raw: &str) -> Option<TimedLine> {
    let body = raw.trim().strip_prefix('[')?;
    let (head, mut rest) = body.split_once(']')?;
    let (start, _) = head.split_once(',')?;
    let start_ms: u64 = start.trim().parse().ok()?;
    let mut words: Vec<TimedWord> = Vec::new();
    while let Some(tag) = rest.strip_prefix('<') {
        let (inner, after) = tag.split_once('>')?;
        let mut parts = inner.split(',');
        let offset: u64 = parts.next()?.trim().parse().ok()?;
        let dur_ms: u64 = parts.next()?.trim().parse().ok()?;
        let end = after.find('<').unwrap_or(after.len());
        if !after[..end].is_empty() {
            words.push(TimedWord { start_ms: start_ms + offset, dur_ms, text: after[..end].to_string() });
        }
        rest = &after[end..];
    }
    let text = words.iter().map(|w| w.text.as_str()).collect::<String>().trim().to_string();
    Some(TimedLine { start_ms, text, words })
}

/// The translation and romaji rows of the `[language:...]` tag (base64 JSON), one per line, each as the
/// pieces the tag gives it: a whole translated line, or the romaji of each timed word. Empty when absent.
fn language_rows(text: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let tag = text.lines().find_map(|l| l.trim().strip_prefix("[language:")?.strip_suffix(']'));
    let json = tag.and_then(base64_bytes).and_then(|b| String::from_utf8(b).ok());
    let Some(v) = json.and_then(|j| serde_json::from_str::<Value>(&j).ok()) else { return (Vec::new(), Vec::new()) };
    let rows_of = |kind: u64| -> Vec<Vec<String>> {
        let entry = v.get("content").and_then(Value::as_array).and_then(|list| list.iter().find(|e| e.get("type").and_then(Value::as_u64) == Some(kind)));
        let Some(rows) = entry.and_then(|e| e.get("lyricContent")).and_then(Value::as_array) else { return Vec::new() };
        rows.iter()
            .map(|row| row.as_array().map(|r| r.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default())
            .collect()
    };
    (rows_of(TRANSLATION), rows_of(ROMAJI))
}

/// A row's pieces as one line, single-spaced.
fn joined(pieces: &[String]) -> String {
    pieces.concat().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A line's romaji. Kugou gives it a syllable at a time, one piece per timed word of the line; with the
/// line's own words beside it, the syllables are put together into words.
fn romaji_line(words: &[TimedWord], pieces: &[String]) -> String {
    if pieces.len() != words.len() {
        return joined(pieces);
    }
    let units: Vec<(&str, &str)> = words.iter().map(|w| w.text.as_str()).zip(pieces.iter().map(String::as_str)).collect();
    crate::romaji::words(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(title: &str, artist: &str) -> Vec<Names> {
        vec![Names { title: title.into(), artist: artist.into() }]
    }

    fn b64(s: &str) -> String {
        CryptographicBuffer::EncodeToBase64String(&CryptographicBuffer::CreateFromByteArray(s.as_bytes()).unwrap()).unwrap().to_string()
    }

    const KRC: &str = "[ti:Blinding Lights]\n[ar:The Weeknd]\n\
        [0,1756]<0,439,0>Blinding <439,439,0>Lights - <878,439,0>The <1317,439,0>Weeknd\n\
        [1757,4829]<0,439,0>Lyrics <439,439,0>by：<878,439,0>Max\n\
        [14075,13597]<0,177,0>Yeah\n\
        [27672,2784]<0,200,0>I've <200,144,0>been <344,176,0>tryna <520,632,0>call\n";

    #[test]
    fn lines_are_read_with_each_word_in_song_time() {
        let l = parse_krc(KRC).unwrap();
        assert_eq!(l.lines.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), vec!["Yeah", "I've been tryna call"]);
        let call = &l.timed.last().unwrap().words;
        assert_eq!((call[0].start_ms, call[3].start_ms, call[3].dur_ms), (27_672, 28_192, 632));
        assert_eq!(call.iter().map(|w| w.text.as_str()).collect::<String>(), "I've been tryna call");
        // The title header and credit lines are not lyrics, timed or not.
        assert_eq!(l.timed.len(), 2);
    }

    #[test]
    fn translation_and_romaji_follow_their_lines() {
        let lang = r#"{"content":[{"type":1,"lyricContent":[["Title"],["Words"],["Yes"],["I keep trying to call"]]},{"type":0,"lyricContent":[[""],[""],["ie "],["a ","i ","ki ta"]]}]}"#;
        let body: String = KRC.lines().skip(2).collect::<Vec<_>>().join("\n");
        let l = parse_krc(&format!("[ti:Blinding Lights]\n[language:{}]\n{body}", b64(lang))).unwrap();
        assert_eq!(l.translation.iter().map(|x| (x.at_ms, x.text.as_str())).collect::<Vec<_>>(), vec![(14_075, "Yes"), (27_672, "I keep trying to call")]);
        assert_eq!(l.romaji.last().unwrap().text, "a i ki ta");
        // Only English translations are kept; romaji stays either way.
        let chinese = r#"{"content":[{"type":1,"lyricContent":[["标题"],["词"],["耶"],["我一直想打电话"]]},{"type":0,"lyricContent":[[""],[""],["ie "],["a ","i ","ki ta"]]}]}"#;
        let l = parse_krc(&format!("[ti:Blinding Lights]\n[language:{}]\n{body}", b64(chinese))).unwrap();
        assert!(l.translation.is_empty() && !l.romaji.is_empty());
        // Rows that do not line up with the lines are left out rather than put on the wrong lines.
        let short = r#"{"content":[{"type":1,"lyricContent":[["x"]]}]}"#;
        assert!(parse_krc(&format!("[language:{}]\n{KRC}", b64(short))).unwrap().translation.is_empty());
    }

    #[test]
    fn encrypted_content_is_decoded() {
        let plain = b"[ti:x]\n[100,500]<0,500,0>hi";
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(plain, 6);
        let mut raw = KRC_MAGIC.to_vec();
        raw.extend(packed.iter().enumerate().map(|(i, b)| b ^ KRC_KEY[i % KRC_KEY.len()]));
        assert_eq!(decrypt(&raw).unwrap().as_bytes(), plain);
        assert!(decrypt(b"nope").is_none());
        assert!(decrypt(b"krc1garbage").is_none());
        assert!(decode(r#"{"status":200,"content":""}"#).is_none());
        assert!(decode("not json").is_none());
    }

    #[test]
    fn only_the_same_song_is_a_candidate() {
        let body = r#"{"candidates":[
            {"id":"1","accesskey":"A","song":"Blinding Lights (Live)","singer":"The Weeknd","duration":260000},
            {"id":"2","accesskey":"B","song":"Blinding Lights","singer":"The Weeknd","duration":200097},
            {"id":"3","accesskey":"C","song":"Blinding Lights","singer":"Cover Band","duration":200000},
            {"id":"4","accesskey":"","song":"Blinding Lights","singer":"The Weeknd","duration":200000}]}"#;
        let c = candidates(body, &names("Blinding Lights", "The Weeknd"), 200_000);
        assert_eq!(c.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["2"]);
        assert!(candidates("{}", &names("a", "b"), 0).is_empty());
    }

    #[test]
    fn each_name_is_searched_with_and_without_its_artist() {
        assert_eq!(keywords(&names("Lemon", "米津玄師")), vec!["米津玄師 - Lemon", "Lemon"]);
        assert_eq!(keywords(&names("Song", "")), vec!["Song"]);
    }

    /// Talks to Kugou: `cargo test -- --ignored live_kugou --nocapture`.
    #[test]
    #[ignore]
    fn live_kugou() {
        let found = lookup(&names("夜に駆ける", "YOASOBI"), 261_041, 1, &[]);
        let l = found.first().expect("kugou has this song");
        println!("{}: {} lines, {} timed, {} translated, {} romaji; first {:?}", l.id, l.lines.len(), l.timed.len(), l.translation.len(), l.romaji.len(), l.lines.first());
        assert!(!l.timed.is_empty() && l.timed.iter().all(|t| !t.words.is_empty()));
    }
}
