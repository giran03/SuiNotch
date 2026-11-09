//! Romaji and translations taken from another service's copy of a song's lyrics. They are only taken
//! when that copy is the same lyrics as the ones shown, so a song with a similar name never lends its
//! romaji, and each row is moved onto the line with the same words rather than the line at about the
//! same time, so the two services timing a line differently never put a row under the wrong line.

use crate::lrc::{self, Line};
use crate::words;

/// At least this share of the lines shown must be in the other copy for it to count as the same lyrics.
const MIN_SHARED_PERCENT: usize = 50;
/// How far a romaji or translation row may sit from its line in the other copy, in time.
const ROW_TOLERANCE_MS: u64 = 300;

/// Another copy's translation and romaji, placed on the lines shown.
pub struct Extras {
    pub translation: Vec<Line>,
    pub romaji: Vec<Line>,
}

/// The translation and romaji of `theirs` (another service's copy of the lyrics) placed on `ours`, each
/// row at the time of the line with the same words; None when `theirs` is not the same lyrics. A line
/// of ours that is not in `theirs`, such as one broken in two there, gets no row.
pub fn for_lines(ours: &[Line], theirs: &[Line], translation: &[Line], romaji: &[Line]) -> Option<Extras> {
    let found = matches(ours, theirs);
    if !same_lyrics(ours, &found) {
        return None;
    }
    Some(Extras { translation: moved(ours, theirs, &found, translation), romaji: moved(ours, theirs, &found, romaji) })
}

/// Whether `theirs` is the same lyrics as `ours`.
pub fn is_same(ours: &[Line], theirs: &[Line]) -> bool {
    same_lyrics(ours, &matches(ours, theirs))
}

/// For each of our lines, the same line in `theirs`, read in order, so a repeated chorus is matched to
/// its next time round rather than its first.
fn matches(ours: &[Line], theirs: &[Line]) -> Vec<Option<usize>> {
    let mut next = 0;
    ours.iter()
        .map(|line| {
            if line.text.trim().is_empty() {
                return None;
            }
            let same = |j: &usize| words::same_text(&line.text, &theirs[*j].text);
            let j = (next..theirs.len()).find(same).or_else(|| (0..next.min(theirs.len())).find(same))?;
            next = j + 1;
            Some(j)
        })
        .collect()
}

fn same_lyrics(ours: &[Line], found: &[Option<usize>]) -> bool {
    let wanted = ours.iter().filter(|l| !l.text.trim().is_empty()).count();
    wanted > 0 && found.iter().flatten().count() * 100 >= wanted * MIN_SHARED_PERCENT
}

/// The rows of `rows` (romaji or translation of `theirs`) for our lines, at our lines' times.
fn moved(ours: &[Line], theirs: &[Line], found: &[Option<usize>], rows: &[Line]) -> Vec<Line> {
    ours.iter()
        .zip(found)
        .filter_map(|(line, j)| {
            let row = lrc::line_at(rows, theirs[(*j)?].at_ms, ROW_TOLERANCE_MS)?;
            Some(Line { at_ms: line.at_ms, text: row.text.clone() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(rows: &[(u64, &str)]) -> Vec<Line> {
        rows.iter().map(|(at_ms, text)| Line { at_ms: *at_ms, text: text.to_string() }).collect()
    }

    #[test]
    fn another_songs_romaji_is_not_taken() {
        // A song by the same band with a similar name: none of its lines are ours.
        let ours = lines(&[(2000, "姿形の見えない魔物"), (7000, "どこへでも連れて行くよ"), (11000, "街を見下ろして")]);
        let theirs = lines(&[(3800, "もうきっとさよならがすぐそこさ"), (9600, "終わりともさよならとも")]);
        let romaji = lines(&[(3800, "moukitto sayonara ga sugu soko sa"), (9600, "owari to mo sayonara to mo")]);
        assert!(for_lines(&ours, &theirs, &[], &romaji).is_none());
        assert!(!is_same(&ours, &theirs));
    }

    #[test]
    fn rows_follow_the_line_with_the_same_words_not_the_same_time() {
        // The other service times every line a second and a half later.
        let ours = lines(&[(2000, "姿形の見えない魔物"), (7000, "どこへでも連れて行くよ"), (11000, "街を見下ろして")]);
        let theirs = lines(&[(3500, "姿形の見えない魔物"), (8500, "どこへでも連れて行くよ"), (12500, "街を見下ろして")]);
        let romaji = lines(&[(3500, "sugata katachi no mienai mamono"), (8500, "doko e demo tsurete iku yo"), (12500, "machi wo mioroshite")]);
        let e = for_lines(&ours, &theirs, &[], &romaji).unwrap();
        assert_eq!(e.romaji, lines(&[(2000, "sugata katachi no mienai mamono"), (7000, "doko e demo tsurete iku yo"), (11000, "machi wo mioroshite")]));
        assert!(e.translation.is_empty());
    }

    #[test]
    fn a_line_broken_differently_gets_no_row_and_a_chorus_follows_its_turn() {
        let ours = lines(&[(1000, "la la"), (2000, "first half second half"), (3000, "la la"), (4000, "end")]);
        let theirs = lines(&[(1000, "la la"), (2000, "first half"), (2500, "second half"), (3000, "la la"), (4000, "end")]);
        let romaji = lines(&[(1000, "r1"), (2000, "r2"), (2500, "r3"), (3000, "r4"), (4000, "r5")]);
        let e = for_lines(&ours, &theirs, &[], &romaji).unwrap();
        assert_eq!(e.romaji, lines(&[(1000, "r1"), (3000, "r4"), (4000, "r5")]));
    }
}
