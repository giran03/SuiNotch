//! Native-script names for a song the player lists in Latin letters, from Apple's music store in
//! Japan (its public search needs no key): "Kan Saete Kuyashiiwa" by ZUTOMAYO is filed there as
//! 勘冴えて悔しいわ by ずっと真夜中でいいのに。, which is how the lyrics services know it.

use serde_json::Value;
use windows::Web::Http::HttpClient;

use crate::lyrics::{clean_title, encode, get, primary_artist, Names};

const API: &str = "https://itunes.apple.com/search";
/// A store listing is the same recording when it is within this long of the playing song.
const DURATION_TOLERANCE_MS: u64 = 6000;

/// Whether text is written only in Latin letters (plus digits and punctuation), so a native-script
/// name may exist for it.
pub fn is_latin(s: &str) -> bool {
    s.chars().all(|c| (c as u32) < 0x250 || !c.is_alphanumeric())
}

/// The native-script title and artist of the song, when its names are in Latin letters and the store
/// has it; None otherwise, or when the store can't be reached.
pub fn lookup(client: &HttpClient, names: &Names, duration_ms: u64) -> Option<Names> {
    if !is_latin(&names.title) {
        return None;
    }
    let term = format!("{} {}", names.title, names.artist);
    let url = format!("{API}?media=music&entity=song&limit=8&country=jp&term={}", encode(&term));
    match get(client, &url) {
        Ok(body) => pick(&body, duration_ms),
        Err(e) => {
            crate::log(&format!("name lookup failed: {e}"));
            None
        }
    }
}

/// The first listing in a native script that is the right length.
fn pick(body: &str, duration_ms: u64) -> Option<Names> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("results")?.as_array()?.iter().find_map(|r| {
        let title = r.get("trackName")?.as_str()?;
        let artist = r.get("artistName")?.as_str()?;
        let length = r.get("trackTimeMillis").and_then(Value::as_u64).unwrap_or(0);
        let fits = duration_ms == 0 || length == 0 || length.abs_diff(duration_ms) <= DURATION_TOLERANCE_MS;
        (fits && !is_latin(title)).then(|| Names { title: clean_title(title), artist: primary_artist(artist) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULTS: &str = r#"{"results":[
        {"trackName":"Kan Saete Kuyashiiwa (Live)","artistName":"Someone","trackTimeMillis":300000},
        {"trackName":"勘冴えて悔しいわ","artistName":"ずっと真夜中でいいのに。","trackTimeMillis":236960},
        {"trackName":"勘冴えて悔しいわ ORIGINAL COVER","artistName":"NIYARI計画","trackTimeMillis":131166}]}"#;

    #[test]
    fn latin_text_is_told_from_native_script() {
        assert!(is_latin("Kan Saete Kuyashiiwa (Live) - 2020!"));
        assert!(is_latin("Beyoncé"));
        assert!(!is_latin("勘冴えて悔しいわ"));
        assert!(!is_latin("Lemon 米津玄師"));
    }

    #[test]
    fn takes_the_native_listing_of_the_right_length() {
        let n = pick(RESULTS, 238_000).unwrap();
        assert_eq!((n.title.as_str(), n.artist.as_str()), ("勘冴えて悔しいわ", "ずっと真夜中でいいのに。"));
        // A length that matches nothing, or nothing in a native script, finds no names.
        assert!(pick(RESULTS, 90_000).is_none());
        assert!(pick(r#"{"results":[]}"#, 0).is_none());
        assert!(pick("not json", 0).is_none());
    }

    #[test]
    fn unknown_length_takes_the_first_native_listing() {
        assert_eq!(pick(RESULTS, 0).unwrap().title, "勘冴えて悔しいわ");
    }
}
