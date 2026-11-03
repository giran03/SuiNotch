//! Native-script names for a song the player lists in Latin letters, from Apple's music store in
//! Japan (its public search needs no key): "Kan Saete Kuyashiiwa" by ZUTOMAYO is filed there as
//! 勘冴えて悔しいわ by ずっと真夜中でいいのに。, which is how the lyrics services know it.

//! The same store also answers whether a track is a known recording at all (see `verify`), and lists
//! an album's tracks so the next one's lyrics can be fetched ahead (see `album_track`).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use crate::events::MediaInfo;
use crate::lyrics::{clean_title, encode, primary_artist, Names};
use crate::lyrics_net::get;

const API: &str = "https://itunes.apple.com/search";
const LOOKUP: &str = "https://itunes.apple.com/lookup";
/// A store listing is the same recording when it is within this long of the playing song.
const DURATION_TOLERANCE_MS: u64 = 6000;
/// Closer still for confirming that something playing is a song at all.
const VERIFY_TOLERANCE_MS: u64 = 3000;
/// Album track lists already fetched this run, by album and artist.
static ALBUMS: Mutex<Option<HashMap<String, Vec<AlbumTrack>>>> = Mutex::new(None);
const MAX_ALBUMS: usize = 50;

/// What the store says about something playing.
#[derive(Debug, PartialEq)]
pub enum Verified {
    /// A recording of that name and length exists; with its native-script names when it has them.
    Song(Option<Names>),
    NotSong,
    /// The store could not be asked.
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AlbumTrack {
    pub disc: u64,
    pub number: u64,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
}

/// Whether text is written only in Latin letters (plus digits and punctuation), so a native-script
/// name may exist for it.
pub fn is_latin(s: &str) -> bool {
    s.chars().all(|c| (c as u32) < 0x250 || !c.is_alphanumeric())
}

/// The native-script title and artist of the song, when its names are in Latin letters and the store
/// has it; None otherwise, or when the store can't be reached.
pub fn lookup(names: &Names, duration_ms: u64) -> Option<Names> {
    if !is_latin(&names.title) {
        return None;
    }
    let term = format!("{} {}", names.title, names.artist);
    let url = format!("{API}?media=music&entity=song&limit=8&country=jp&term={}", encode(&term));
    match get(&url) {
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

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether two names are the same once case, spacing and punctuation are set aside, or one holds the other.
fn related(a: &str, b: &str) -> bool {
    let (a, b) = (squash(a), squash(b));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// Whether the store knows a recording with this title (or this artist and a native-script title) and
/// this length, in Japan's store (which also lists the native names) and then the US one.
pub fn verify(info: &MediaInfo) -> Verified {
    let names = Names { title: clean_title(&info.title), artist: primary_artist(&info.artist) };
    let term = format!("{} {}", names.title, names.artist);
    for country in ["jp", "us"] {
        let url = format!("{API}?media=music&entity=song&limit=10&country={country}&term={}", encode(&term));
        // "Not a song" needs both stores to have answered; a failed or cancelled ask decides nothing.
        let Ok(body) = get(&url) else { return Verified::Unknown };
        if let Some(v) = confirms(&body, &names, info.duration_ms) {
            return v;
        }
    }
    Verified::NotSong
}

/// `Song` when a listing matches by length and by title (or by artist, for a native-script title).
fn confirms(body: &str, names: &Names, duration_ms: u64) -> Option<Verified> {
    let v: Value = serde_json::from_str(body).ok()?;
    let results = v.get("results")?.as_array()?;
    let mut native = None;
    let mut matched = false;
    for r in results {
        let (Some(title), Some(artist)) = (r.get("trackName").and_then(Value::as_str), r.get("artistName").and_then(Value::as_str)) else { continue };
        let length = r.get("trackTimeMillis").and_then(Value::as_u64).unwrap_or(0);
        // Without a length, the title has to match, which is too loose alone: an unknown length never confirms.
        let fits = duration_ms > 0 && length > 0 && length.abs_diff(duration_ms) <= VERIFY_TOLERANCE_MS;
        let by_title = related(title, &names.title) && (related(artist, &names.artist) || names.artist.is_empty());
        let by_native = !is_latin(title) && related(artist, &names.artist);
        if fits && (by_title || by_native) {
            matched = true;
            if native.is_none() && !is_latin(title) && is_latin(&names.title) {
                native = Some(Names { title: clean_title(title), artist: primary_artist(artist) });
            }
        }
    }
    matched.then_some(Verified::Song(native))
}

/// The track after `info` on its album, from the store: the next number on the same disc, or the
/// first of the next disc. None when the album is unknown or this is its last track.
pub fn album_track(info: &MediaInfo) -> Option<AlbumTrack> {
    if info.album.is_empty() || info.track_number == 0 {
        return None;
    }
    let artist = if info.album_artist.is_empty() { primary_artist(&info.artist) } else { info.album_artist.clone() };
    let key = format!("{}\u{1}{}", squash(&info.album), squash(&artist));
    let cached = ALBUMS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key).cloned());
    let tracks = match cached {
        Some(t) => t,
        None => {
            let tracks = album_tracks(&info.album, &artist)?;
            let mut albums = ALBUMS.lock().unwrap_or_else(|e| e.into_inner());
            let map = albums.get_or_insert_with(HashMap::new);
            if map.len() >= MAX_ALBUMS {
                map.clear();
            }
            map.insert(key, tracks.clone());
            tracks
        }
    };
    next_on_album(&tracks, info)
}

fn next_on_album(tracks: &[AlbumTrack], info: &MediaInfo) -> Option<AlbumTrack> {
    let here = tracks.iter().find(|t| t.number == info.track_number as u64 && related(&t.title, &clean_title(&info.title)))?;
    tracks
        .iter()
        .filter(|t| (t.disc, t.number) > (here.disc, here.number))
        .min_by_key(|t| (t.disc, t.number))
        .cloned()
}

/// An album's tracks: the album is searched for, then its listing looked up.
fn album_tracks(album: &str, artist: &str) -> Option<Vec<AlbumTrack>> {
    let url = format!("{API}?media=music&entity=album&limit=5&term={}", encode(&format!("{album} {artist}")));
    let v: Value = serde_json::from_str(&get(&url).ok()?).ok()?;
    let id = v.get("results")?.as_array()?.iter().find_map(|r| {
        let name = r.get("collectionName")?.as_str()?;
        let by = r.get("artistName")?.as_str()?;
        (related(name, album) && related(by, artist)).then(|| r.get("collectionId")?.as_u64()).flatten()
    })?;
    let body = get(&format!("{LOOKUP}?id={id}&entity=song")).ok()?;
    Some(tracks_from(&body))
}

fn tracks_from(body: &str) -> Vec<AlbumTrack> {
    let Some(results) = serde_json::from_str::<Value>(body).ok().and_then(|v| v.get("results").and_then(Value::as_array).cloned()) else { return Vec::new() };
    results
        .iter()
        .filter(|r| r.get("wrapperType").and_then(Value::as_str) == Some("track"))
        .filter_map(|r| {
            Some(AlbumTrack {
                disc: r.get("discNumber").and_then(Value::as_u64).unwrap_or(1),
                number: r.get("trackNumber")?.as_u64()?,
                title: r.get("trackName")?.as_str()?.to_string(),
                artist: r.get("artistName")?.as_str()?.to_string(),
                duration_ms: r.get("trackTimeMillis").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect()
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

    fn names(title: &str, artist: &str) -> Names {
        Names { title: title.into(), artist: artist.into() }
    }

    #[test]
    fn a_listing_of_the_same_name_and_length_confirms_a_song() {
        let body = r#"{"results":[{"trackName":"Blinding Lights","artistName":"The Weeknd","trackTimeMillis":200040}]}"#;
        assert_eq!(confirms(body, &names("Blinding Lights", "The Weeknd"), 201_000), Some(Verified::Song(None)));
        // Another length, another title, or no length at all is not confirmation.
        assert_eq!(confirms(body, &names("Blinding Lights", "The Weeknd"), 260_000), None);
        assert_eq!(confirms(body, &names("Episode 12: Lights", "A Podcast"), 200_000), None);
        assert_eq!(confirms(body, &names("Blinding Lights", "The Weeknd"), 0), None);
    }

    #[test]
    fn a_native_listing_by_the_same_artist_confirms_a_romanised_title_and_brings_its_names() {
        let body = r#"{"results":[{"trackName":"夜に駆ける","artistName":"YOASOBI","trackTimeMillis":261000}]}"#;
        let v = confirms(body, &names("Yoru ni Kakeru", "YOASOBI"), 260_000).unwrap();
        assert_eq!(v, Verified::Song(Some(names("夜に駆ける", "YOASOBI"))));
        // A native listing of about the same length by someone unrelated could be anything.
        assert_eq!(confirms(RESULTS, &names("Kan Saete Kuyashiiwa", "ZUTOMAYO"), 237_500), None);
    }

    #[test]
    fn the_next_album_track_follows_disc_and_number() {
        let body = r#"{"results":[
            {"wrapperType":"collection","collectionName":"Album"},
            {"wrapperType":"track","discNumber":1,"trackNumber":1,"trackName":"One","artistName":"A","trackTimeMillis":1000},
            {"wrapperType":"track","discNumber":1,"trackNumber":2,"trackName":"Two (Remastered)","artistName":"A","trackTimeMillis":2000},
            {"wrapperType":"track","discNumber":2,"trackNumber":1,"trackName":"Three","artistName":"A","trackTimeMillis":3000}]}"#;
        let tracks = tracks_from(body);
        assert_eq!(tracks.len(), 3);
        let playing = |title: &str, n: u32| MediaInfo { title: title.into(), album: "Album".into(), track_number: n, ..Default::default() };
        assert_eq!(next_on_album(&tracks, &playing("One", 1)).unwrap().title, "Two (Remastered)");
        assert_eq!(next_on_album(&tracks, &playing("Two", 2)).unwrap().title, "Three");
        assert!(next_on_album(&tracks, &playing("Three", 1)).is_none(), "the last track has no next");
        assert!(next_on_album(&tracks, &playing("Unrelated", 1)).is_none(), "a different album with the same name");
    }
}
