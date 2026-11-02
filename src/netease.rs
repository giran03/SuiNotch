//! Fallback lyrics from NetEase Cloud Music's public web endpoints, used when LRCLIB has none.
//! Unofficial and undocumented, so every response is parsed defensively and a failure is just "no lyrics".

use serde_json::Value;
use windows::core::{Result, HSTRING};
use windows::Foundation::Uri;
use windows::Security::Cryptography::Core::{CryptographicEngine, HashAlgorithmNames, HashAlgorithmProvider, SymmetricAlgorithmNames, SymmetricKeyAlgorithmProvider};
use windows::Security::Cryptography::{BinaryStringEncoding, CryptographicBuffer};
use windows::Storage::Streams::UnicodeEncoding;
use windows::Web::Http::{HttpClient, HttpStringContent};

use crate::lrc::{self, Line};
use crate::lyrics::{encode, get, Names};
use crate::versions;
use crate::words::{self, TimedLine};

const API: &str = "https://music.163.com/api";
/// The word-by-word lyrics are only served by the app's own encrypted endpoint; the public one has none.
const EAPI: &str = "https://interface.music.163.com/eapi/song/lyric/v1";
const EAPI_PATH: &str = "/api/song/lyric/v1";
const EAPI_KEY: &[u8] = b"e82ckenh8dichen8";
const EAPI_SEPARATOR: &str = "36cd479b6b5";
const SEARCH_LIMIT: u32 = 8;
const DURATION_TOLERANCE_MS: u64 = 4000;
/// Credit lines ("作词 : ...", "Lyrics by : ...") come before the first sung line, in the opening seconds.
const CREDITS_END_MS: u64 = 20_000;

/// One song's lyrics: the original, and when NetEase has them a translation and a romanised version.
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub translation: Vec<Line>,
    pub romaji: Vec<Line>,
    /// The lines with each word's timing, when asked for and the song has it; empty otherwise.
    pub timed: Vec<TimedLine>,
}

/// Lyrics for the best-fitting songs, best first, at most `limit` of them. The song is searched for
/// under every name it goes by (as the player lists it, and in its native script), and a result
/// counts if it fits any of them. With `word_timing`, each song's word-by-word lyrics are asked for too.
pub fn lookup(client: &HttpClient, names: &[Names], duration_ms: u64, limit: usize, word_timing: bool) -> Vec<Lyrics> {
    let mut songs: Vec<Value> = Vec::new();
    for query in queries(names) {
        let url = format!("{API}/search/get?s={}&type=1&offset=0&limit={SEARCH_LIMIT}", encode(&query));
        match get(client, &url) {
            Ok(body) => {
                for song in songs_from(&body) {
                    if !songs.iter().any(|s| s.get("id") == song.get("id")) {
                        songs.push(song);
                    }
                }
            }
            Err(e) => {
                crate::log(&format!("netease search failed: {e}"));
                break;
            }
        }
    }
    pick_ids(&songs, names, duration_ms)
        .into_iter()
        .filter_map(|id| {
            let mut lyrics = get(client, &format!("{API}/song/lyric?id={id}&lv=1&tv=-1&rv=-1")).ok().and_then(|b| parse_lyric(&b))?;
            if word_timing {
                lyrics.timed = word_by_word(id).unwrap_or_else(|e| {
                    crate::log(&format!("netease word timing failed: {e}"));
                    Vec::new()
                });
            }
            Some(lyrics)
        })
        .take(limit)
        .collect()
}

/// The `params` value of an encrypted app request: the body signed with its path, then AES-128-ECB
/// encrypted with the app's fixed key, in capital hex.
fn eapi_params(path: &str, body: &str) -> Result<String> {
    let binary = |s: String| CryptographicBuffer::ConvertStringToBinary(&HSTRING::from(s), BinaryStringEncoding::Utf8);
    let md5 = HashAlgorithmProvider::OpenAlgorithm(&HashAlgorithmNames::Md5()?)?;
    let hash = md5.HashData(&binary(format!("nobody{path}use{body}md5forencrypt"))?)?;
    let digest = CryptographicBuffer::EncodeToHexString(&hash)?.to_string().to_ascii_lowercase();
    let message = format!("{path}-{EAPI_SEPARATOR}-{body}-{EAPI_SEPARATOR}-{digest}");
    let key = SymmetricKeyAlgorithmProvider::OpenAlgorithm(&SymmetricAlgorithmNames::AesEcbPkcs7()?)?.CreateSymmetricKey(&CryptographicBuffer::CreateFromByteArray(EAPI_KEY)?)?;
    let encrypted = CryptographicEngine::Encrypt(&key, &binary(message)?, None)?;
    Ok(CryptographicBuffer::EncodeToHexString(&encrypted)?.to_string().to_ascii_uppercase())
}

/// A song's word-by-word lyrics, empty when NetEase has none for it.
fn word_by_word(id: u64) -> Result<Vec<TimedLine>> {
    let body = format!(r#"{{"id":"{id}","lv":-1,"kv":-1,"tv":-1,"rv":-1,"yv":-1,"ytv":-1,"yrv":-1}}"#);
    let params = eapi_params(EAPI_PATH, &body)?;
    let client = HttpClient::new()?;
    let headers = client.DefaultRequestHeaders()?;
    let _ = headers.UserAgent()?.TryParseAdd(&HSTRING::from("Mozilla/5.0"));
    let _ = headers.TryAppendWithoutValidation(&HSTRING::from("Cookie"), &HSTRING::from("os=pc; appver=2.10.8"));
    let content = HttpStringContent::CreateFromStringWithEncodingAndMediaType(
        &HSTRING::from(format!("params={params}")),
        UnicodeEncoding::Utf8,
        &HSTRING::from("application/x-www-form-urlencoded"),
    )?;
    let reply = client.PostAsync(&Uri::CreateUri(&HSTRING::from(EAPI))?, &content)?.get()?;
    let text = reply.Content()?.ReadAsStringAsync()?.get()?.to_string();
    Ok(timed_lines(&text))
}

/// The word-by-word lines in an `eapi` lyric response.
fn timed_lines(body: &str) -> Vec<TimedLine> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/yrc/lyric").and_then(Value::as_str).map(words::parse_yrc))
        .unwrap_or_default()
}

/// What to search for: each name's title with its artist, then the title alone in case the artist
/// is spelled differently over there.
fn queries(names: &[Names]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        for q in [format!("{} {}", n.title, n.artist), n.title.clone()] {
            let q = q.trim().to_string();
            if !q.is_empty() && !out.contains(&q) {
                out.push(q);
            }
        }
    }
    out
}

fn songs_from(body: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/result/songs").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

/// Ids of results that are the same song: a title and an artist that match one of the names, and the right length.
fn pick_ids(songs: &[Value], names: &[Names], duration_ms: u64) -> Vec<u64> {
    let artists: Vec<String> = names.iter().map(|n| n.artist.to_lowercase()).collect();
    let related = |a: &str, b: &str| !a.is_empty() && !b.is_empty() && (a.contains(b) || b.contains(a));
    let mut hits: Vec<(u64, u64)> = Vec::new();
    for song in songs {
        let Some(id) = song.get("id").and_then(Value::as_u64) else { continue };
        let original = song.get("name").and_then(Value::as_str).unwrap_or_default();
        let name = original.to_lowercase();
        let length = song.get("duration").and_then(Value::as_u64).unwrap_or(0);
        let off = if duration_ms > 0 && length > 0 { length.abs_diff(duration_ms) } else { 0 };
        let by_artist = song.get("artists").and_then(Value::as_array).is_some_and(|list| {
            list.iter().filter_map(|x| x.get("name")?.as_str()).any(|n| artists.iter().any(|a| related(&n.to_lowercase(), a)))
        });
        // A live, remix, cover or instrumental recording has other vocals: its lyrics are not this song's.
        let same_version = names.iter().any(|n| related(&name, &n.title.to_lowercase()) && versions::mismatch(&n.title, original).is_none());
        if same_version && by_artist && off <= DURATION_TOLERANCE_MS {
            hits.push((off, id));
        }
    }
    hits.sort_by_key(|&(off, _)| off);
    hits.into_iter().map(|(_, id)| id).collect()
}

/// Synced lines from a `song/lyric` response, without the credit header; None for instrumentals.
fn parse_lyric(body: &str) -> Option<Lyrics> {
    let v: Value = serde_json::from_str(body).ok()?;
    let text = v.pointer("/lrc/lyric")?.as_str()?;
    if text.contains("纯音乐") {
        return None;
    }
    let lines = without_credits(lrc::parse(text));
    if lines.is_empty() {
        return None;
    }
    let extra = |pointer: &str| v.pointer(pointer).and_then(Value::as_str).map(|t| without_credits(lrc::parse(t))).unwrap_or_default();
    Some(Lyrics { lines, translation: extra("/tlyric/lyric"), romaji: extra("/romalrc/lyric"), timed: Vec::new() })
}

fn without_credits(mut lines: Vec<Line>) -> Vec<Line> {
    let credits = lines.iter().take_while(|l| l.at_ms < CREDITS_END_MS && (l.text.contains(" : ") || l.text.contains('：'))).count();
    lines.drain(..credits);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = r#"{"result":{"songs":[
        {"id":1,"name":"Blinding Lights (Live)","duration":260000,"artists":[{"name":"The Weeknd"}]},
        {"id":2,"name":"Blinding Lights","duration":200045,"artists":[{"name":"The Weeknd"}]},
        {"id":3,"name":"Blinding Lights","duration":199000,"artists":[{"name":"Cover Band"}]},
        {"id":4,"name":"Blinding Lights","duration":201500,"artists":[{"name":"The Weeknd"}]}]}}"#;

    fn names(title: &str, artist: &str) -> Vec<Names> {
        vec![Names { title: title.into(), artist: artist.into() }]
    }

    #[test]
    fn picks_same_song_closest_in_length_first() {
        let songs = songs_from(SEARCH);
        assert_eq!(pick_ids(&songs, &names("Blinding Lights", "The Weeknd"), 200_000), vec![2, 4]);
    }

    #[test]
    fn unknown_length_accepts_any_duration() {
        let songs = songs_from(SEARCH);
        // Not the live recording, whose lyrics are another take.
        assert_eq!(pick_ids(&songs, &names("blinding lights", "the weeknd"), 0), vec![2, 4]);
        assert_eq!(pick_ids(&songs, &names("Blinding Lights (Live)", "The Weeknd"), 0), vec![1]);
    }

    #[test]
    fn bad_search_body_gives_nothing() {
        assert!(songs_from("not json").is_empty());
        assert!(songs_from(r#"{"result":{}}"#).is_empty());
    }

    #[test]
    fn a_native_script_name_finds_a_song_listed_romanised() {
        // The player says "Kan Saete Kuyashiiwa"; NetEase lists the song only in Japanese.
        let songs = songs_from(
            r#"{"result":{"songs":[
                {"id":7,"name":"勘冴えて悔しいわ","duration":243408,"artists":[{"name":"然倉桃果"}]},
                {"id":8,"name":"勘冴えて悔しいわ","duration":237000,"artists":[{"name":"ずっと真夜中でいいのに。"}]}]}}"#,
        );
        let romanised = names("Kan Saete Kuyashiiwa", "ZUTOMAYO");
        assert!(pick_ids(&songs, &romanised, 237_000).is_empty());
        let mut both = romanised;
        both.push(Names { title: "勘冴えて悔しいわ".into(), artist: "ずっと真夜中でいいのに。".into() });
        assert_eq!(pick_ids(&songs, &both, 237_000), vec![8]);
    }

    #[test]
    fn each_name_is_searched_with_and_without_its_artist() {
        let mut both = names("Kan Saete Kuyashiiwa", "ZUTOMAYO");
        both.push(Names { title: "勘冴えて悔しいわ".into(), artist: "ずっと真夜中でいいのに。".into() });
        assert_eq!(
            queries(&both),
            vec!["Kan Saete Kuyashiiwa ZUTOMAYO", "Kan Saete Kuyashiiwa", "勘冴えて悔しいわ ずっと真夜中でいいのに。", "勘冴えて悔しいわ"]
        );
        assert_eq!(queries(&names("Song", "")), vec!["Song"]);
    }

    #[test]
    fn credit_header_is_dropped() {
        let body = r#"{"lrc":{"lyric":"[00:00.00] 作词 : A\n[00:01.00] 作曲 : B\n[00:18.85]first line\n[00:25.00]Note: not a credit"}}"#;
        let lines = parse_lyric(body).unwrap().lines;
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["first line", "Note: not a credit"]);
    }

    #[test]
    fn translation_and_romaji_come_along_when_present() {
        let body = r#"{"lrc":{"lyric":"[00:10.00]一"},"tlyric":{"lyric":"[00:10.00]one"},"romalrc":{"lyric":"[00:10.00]ichi"}}"#;
        let l = parse_lyric(body).unwrap();
        assert_eq!((l.translation[0].text.as_str(), l.romaji[0].text.as_str()), ("one", "ichi"));
        let bare = parse_lyric(r#"{"lrc":{"lyric":"[00:10.00]x"}}"#).unwrap();
        assert!(bare.translation.is_empty() && bare.romaji.is_empty());
    }

    #[test]
    fn the_signed_request_matches_the_app_s() {
        // Computed with an independent implementation (Python's cryptography package).
        let body = r#"{"id":"25706282","lv":-1,"kv":-1,"tv":-1,"rv":-1,"yv":-1,"ytv":-1,"yrv":-1}"#;
        let params = eapi_params(EAPI_PATH, body).unwrap();
        assert!(params.starts_with("04AE33D34A93FE3EC22DA8FA305D290AB337D0FE5F36D211"));
        assert!(params.ends_with("C72C17E46C51BBB12087E266434FEF98B5"));
        assert_eq!(params.len(), 320);
    }

    #[test]
    fn word_timing_is_read_from_the_response_when_there_is_any() {
        let body = r#"{"yrc":{"lyric":"[1000,900](1000,400,0)Hel(1400,500,0)lo"}}"#;
        assert_eq!(timed_lines(body)[0].words.len(), 2);
        assert!(timed_lines(r#"{"lrc":{"lyric":"x"}}"#).is_empty());
        assert!(timed_lines("nope").is_empty());
    }

    /// Talks to NetEase: `cargo test -- --ignored live_word_timing --nocapture`.
    #[test]
    #[ignore]
    fn live_word_timing() {
        let lines = word_by_word(25706282).unwrap();
        println!("{} lines; first: {:?}", lines.len(), lines.first());
        assert!(!lines.is_empty());
    }

    #[test]
    fn instrumentals_and_missing_lyrics_give_nothing() {
        assert!(parse_lyric(r#"{"lrc":{"lyric":"[00:00.00]纯音乐，请欣赏"}}"#).is_none());
        assert!(parse_lyric(r#"{"nolyric":true}"#).is_none());
    }
}
