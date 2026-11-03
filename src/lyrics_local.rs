//! Lyric files of your own: `.lrc` (or plain `.txt`) files in the `lyrics` folder of the data folder,
//! named "Artist - Title" or just "Title". They come before any service, work offline and send nothing.

use std::path::{Path, PathBuf};

use crate::events::MediaInfo;
use crate::lrc;
use crate::lyrics::{clean_title, primary_artist, Found, Source};

pub fn dir() -> PathBuf {
    crate::config::data_dir().join("lyrics")
}

/// A name reduced to its letters and digits, so "AC/DC - T.N.T." finds "acdc - tnt.lrc".
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// File names (without extension, squashed) that would hold this song's lyrics, best first.
fn wanted(info: &MediaInfo) -> Vec<String> {
    let mut out = Vec::new();
    for title in [info.title.clone(), clean_title(&info.title)] {
        for artist in [info.artist.clone(), primary_artist(&info.artist)] {
            out.push(squash(&format!("{artist} - {title}")));
        }
    }
    out.push(squash(&info.title));
    out.push(squash(&clean_title(&info.title)));
    out.retain(|s| !s.is_empty());
    out.dedup();
    out
}

/// The song's own lyric file in `folder`, if there is one.
fn find_in(folder: &Path, info: &MediaInfo) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lrc") || x.eq_ignore_ascii_case("txt")))
        .collect();
    let stems: Vec<String> = files.iter().map(|p| squash(&p.file_stem().unwrap_or_default().to_string_lossy())).collect();
    // .lrc before .txt for the same name.
    wanted(info).iter().find_map(|w| {
        let mut hits: Vec<&PathBuf> = files.iter().zip(&stems).filter(|(_, s)| *s == w).map(|(p, _)| p).collect();
        hits.sort_by_key(|p| !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lrc")));
        hits.first().map(|p| (*p).clone())
    })
}

pub fn find(info: &MediaInfo) -> Option<Found> {
    let path = find_in(&dir(), info)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    parse(&text, info.duration_ms, &name)
}

fn parse(text: &str, duration_ms: u64, name: &str) -> Option<Found> {
    let synced = lrc::parse(text);
    let (lines, timed) = if synced.is_empty() { (lrc::from_plain(text, duration_ms), false) } else { (synced, true) };
    (!lines.is_empty()).then(|| Found {
        lines,
        source: Source::Local,
        synced: timed,
        translation: Vec::new(),
        romaji: Vec::new(),
        words: Vec::new(),
        id: format!("local:{name}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, artist: &str) -> MediaInfo {
        MediaInfo { title: title.into(), artist: artist.into(), duration_ms: 100_000, ..Default::default() }
    }

    #[test]
    fn files_are_found_by_artist_and_title_or_title_alone() {
        let dir = std::env::temp_dir().join(format!("suinotch-local-lrc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AC_DC - T.N.T..txt"), "plain").unwrap();
        std::fs::write(dir.join("ac dc - tnt.lrc"), "[00:01.00]x").unwrap();
        std::fs::write(dir.join("Lonely Title.lrc"), "[00:01.00]y").unwrap();
        let hit = find_in(&dir, &song("T.N.T. (Official Video)", "AC/DC")).unwrap();
        assert!(hit.extension().unwrap() == "lrc", ".lrc wins over .txt: {hit:?}");
        assert!(find_in(&dir, &song("Lonely Title", "Anyone, Else")).is_some());
        assert!(find_in(&dir, &song("Missing", "Nobody")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn synced_and_plain_files_are_read() {
        let f = parse("[00:01.00]one\n[00:02.00]two", 100_000, "a.lrc").unwrap();
        assert!(f.synced && f.source == Source::Local && f.id == "local:a.lrc");
        let plain = parse("one\ntwo", 100_000, "b.txt").unwrap();
        assert!(!plain.synced && plain.lines.len() == 2);
        assert!(parse("", 100_000, "c.lrc").is_none());
    }
}
