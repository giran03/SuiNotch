//! Parsing of LRC synced lyrics as returned by LRCLIB, including the word stamps of enhanced LRC
//! (`[mm:ss.xx]word <mm:ss.xx>word`).

use crate::words::{TimedLine, TimedWord};

/// The last word of a line is taken to last until the next line starts, but no longer than this.
const LAST_WORD_MAX_MS: u64 = 1500;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at_ms: u64,
    pub text: String,
}

/// Parses `[mm:ss.xx] text` lines; lines with several stamps are repeated, metadata tags skipped.
/// Word stamps are left out of the text (see `parse_word_timing` for them).
pub fn parse(lrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in lrc.lines() {
        let (stamps, rest) = line_stamps(raw);
        let text = strip_word_stamps(rest);
        for ms in stamps {
            lines.push(Line { at_ms: ms, text: text.clone() });
        }
    }
    lines.sort_by_key(|l| l.at_ms);
    lines
}

/// The word timing of enhanced LRC, one timed line per line that has word stamps. A line's text is the
/// one `parse` gives it. Lines shown at several times are skipped, as their word stamps fit only one.
pub fn parse_word_timing(lrc: &str) -> Vec<TimedLine> {
    let mut starts = Vec::new();
    let mut timed = Vec::new();
    for raw in lrc.lines() {
        let (stamps, rest) = line_stamps(raw);
        starts.extend_from_slice(&stamps);
        if let [at] = stamps[..] {
            let words = timed_words(at, &pieces(rest));
            if !words.is_empty() {
                timed.push((at, words));
            }
        }
    }
    starts.sort_unstable();
    timed.sort_by_key(|(at, _)| *at);
    timed
        .into_iter()
        .map(|(at, words)| {
            let words: Vec<TimedWord> = words
                .into_iter()
                .map(|(start_ms, dur, text)| {
                    let dur_ms = dur.unwrap_or_else(|| {
                        let next = starts.iter().find(|&&s| s > start_ms);
                        next.map_or(LAST_WORD_MAX_MS, |n| (n - start_ms).min(LAST_WORD_MAX_MS))
                    });
                    TimedWord { start_ms, dur_ms, text }
                })
                .collect();
            let text = words.iter().map(|w| w.text.as_str()).collect::<String>().trim().to_string();
            TimedLine { start_ms: at, text, words }
        })
        .collect()
}

/// A translation is taken to be English when at least this share of its letters are Latin.
const ENGLISH_MIN_LATIN_PERCENT: usize = 90;

/// A song's translation when it is in English, and none when it is in another language: only English
/// translations are shown. The whole translation is judged at once, so a song never shows half of one.
pub fn english_only(translation: Vec<Line>) -> Vec<Line> {
    let latin = |c: char| c.is_ascii_alphabetic() || ('\u{C0}'..='\u{24F}').contains(&c);
    let (mut letters, mut latin_letters) = (0usize, 0usize);
    for c in translation.iter().flat_map(|l| l.text.chars()).filter(|c| c.is_alphabetic()) {
        letters += 1;
        latin_letters += latin(c) as usize;
    }
    if letters > 0 && latin_letters * 100 >= letters * ENGLISH_MIN_LATIN_PERCENT { translation } else { Vec::new() }
}

/// Whether a line's text still holds word stamps, as lines read before they were left out do.
pub fn has_word_stamps(text: &str) -> bool {
    pieces(text).len() > 1
}

/// A line's leading `[mm:ss.xx]` stamps and the text after them.
fn line_stamps(raw: &str) -> (Vec<u64>, &str) {
    let mut rest = raw.trim();
    let mut stamps = Vec::new();
    while let Some(stripped) = rest.strip_prefix('[') {
        let Some(end) = stripped.find(']') else { break };
        let Some(ms) = parse_stamp(&stripped[..end]) else { break };
        stamps.push(ms);
        rest = stripped[end + 1..].trim_start();
    }
    (stamps, rest)
}

/// A line's text cut at its word stamps: each piece with the stamp before it, the first with none.
/// A `<` that does not open a stamp stays in the text.
fn pieces(text: &str) -> Vec<(Option<u64>, &str)> {
    let mut out = Vec::new();
    let (mut stamp, mut from, mut search) = (None, 0, 0);
    while let Some(off) = text[search..].find('<') {
        let start = search + off;
        let after = &text[start + 1..];
        match after.find('>').and_then(|end| parse_stamp(&after[..end]).map(|ms| (end, ms))) {
            Some((end, ms)) => {
                out.push((stamp, &text[from..start]));
                stamp = Some(ms);
                from = start + end + 2;
                search = from;
            }
            None => search = start + 1,
        }
    }
    out.push((stamp, &text[from..]));
    out
}

/// Joins a piece onto the text so far, keeping one space where words met.
fn join_piece<'a>(so_far: &str, piece: &'a str) -> &'a str {
    if so_far.is_empty() || so_far.ends_with(char::is_whitespace) { piece.trim_start() } else { piece }
}

/// Drops enhanced-LRC word stamps (`<mm:ss.xx>`) from a line's text, keeping one space where words met.
pub fn strip_word_stamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (_, piece) in pieces(text) {
        out.push_str(join_piece(&out, piece));
    }
    out.trim().to_string()
}

/// A line's words as (start, duration, text): each runs until the next stamp, and the last one's
/// duration is left open when no stamp ends it. Text before the first stamp starts with the line.
/// Spaces alone between two stamps end the word before them and join its text.
fn timed_words(line_ms: u64, pieces: &[(Option<u64>, &str)]) -> Vec<(u64, Option<u64>, String)> {
    if pieces.iter().all(|(stamp, _)| stamp.is_none()) {
        return Vec::new();
    }
    let mut words: Vec<(u64, Option<u64>, String)> = Vec::new();
    let mut so_far = String::new();
    for &(stamp, raw) in pieces {
        let start = stamp.unwrap_or(line_ms);
        let piece = join_piece(&so_far, raw);
        so_far.push_str(piece);
        if let Some(prev) = words.last_mut() {
            prev.1.get_or_insert(start.saturating_sub(prev.0));
        }
        if piece.trim().is_empty() {
            if let Some(prev) = words.last_mut() {
                prev.2.push_str(piece);
            }
            continue;
        }
        words.push((start, None, piece.to_string()));
    }
    if let Some(last) = words.last_mut() {
        last.2 = last.2.trim_end().to_string();
    }
    words
}

fn parse_stamp(s: &str) -> Option<u64> {
    let (min, sec) = s.split_once(':')?;
    let min: u64 = min.trim().parse().ok()?;
    let (whole, frac) = sec.split_once(['.', ':']).unwrap_or((sec, "0"));
    let whole: u64 = whole.trim().parse().ok()?;
    let frac_digits: String = frac.chars().take(3).collect();
    let frac_ms = match frac_digits.len() {
        0 => 0,
        1 => frac_digits.parse::<u64>().ok()? * 100,
        2 => frac_digits.parse::<u64>().ok()? * 10,
        _ => frac_digits.parse::<u64>().ok()?,
    };
    Some(min * 60_000 + whole * 1000 + frac_ms)
}

/// Index of the line being sung at `pos_ms`, if any line has started.
pub fn current(lines: &[Line], pos_ms: u64) -> Option<usize> {
    match lines.partition_point(|l| l.at_ms <= pos_ms) {
        0 => None,
        n => Some(n - 1),
    }
}

/// The line starting at `at_ms`, give or take `tolerance_ms`: a translation is timed alongside its original.
pub fn line_at(lines: &[Line], at_ms: u64, tolerance_ms: u64) -> Option<&Line> {
    let i = lines.partition_point(|l| l.at_ms < at_ms.saturating_sub(tolerance_ms));
    lines.get(i).filter(|l| l.at_ms <= at_ms + tolerance_ms)
}

/// Plain lyrics without timing get spread evenly across the track so they still scroll.
pub fn from_plain(text: &str, duration_ms: u64) -> Vec<Line> {
    let rows: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if rows.is_empty() || duration_ms == 0 {
        return Vec::new();
    }
    let step = duration_ms / (rows.len() as u64 + 1);
    rows.iter().enumerate().map(|(i, t)| Line { at_ms: step * (i as u64 + 1), text: t.to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stamps_and_skips_tags() {
        let l = parse("[ar:Someone]\n[00:12.50] first\n[01:02.123]second\n[00:05]intro");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0], Line { at_ms: 5000, text: "intro".into() });
        assert_eq!(l[1].at_ms, 12_500);
        assert_eq!(l[2].at_ms, 62_123);
    }

    #[test]
    fn repeated_stamps_duplicate_the_line() {
        let l = parse("[00:01.00][00:30.00] chorus");
        assert_eq!(l.iter().map(|x| x.at_ms).collect::<Vec<_>>(), vec![1000, 30_000]);
    }

    #[test]
    fn word_stamps_are_stripped_from_text() {
        let l = parse("[01:54.000]私の人生は<01:54.955>狂い出し\n[00:02.00]<00:02.00> hello <00:02.50> world <00:03.00>\n[00:04.00]a <b> c");
        assert_eq!(l[0].text, "hello world");
        assert_eq!(l[1].text, "a <b> c");
        assert_eq!(l[2].text, "私の人生は狂い出し");
    }

    #[test]
    fn word_stamps_become_word_timing() {
        let t = parse_word_timing("[ar:x]\n[00:02.00]<00:02.00> hello <00:02.50> world <00:03.20>\n[00:04.00]plain line\n[01:54.000]私の人生は<01:54.955>狂い出し\n[01:57.000]next");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].text, "hello world");
        let w: Vec<_> = t[0].words.iter().map(|w| (w.start_ms, w.dur_ms, w.text.as_str())).collect();
        assert_eq!(w, vec![(2000, 500, "hello "), (2500, 700, "world")]);
        // Text before the first word stamp starts with the line; the last word runs to the next line, within a limit.
        let w: Vec<_> = t[1].words.iter().map(|w| (w.start_ms, w.dur_ms, w.text.as_str())).collect();
        assert_eq!(w, vec![(114_000, 955, "私の人生は"), (114_955, LAST_WORD_MAX_MS, "狂い出し")]);
        assert_eq!(t[1].text, parse("[01:54.000]私の人生は<01:54.955>狂い出し")[0].text);
    }

    #[test]
    fn lines_without_word_stamps_have_no_word_timing() {
        assert!(parse_word_timing("[00:01.00]a <b> c\n[00:01.00][00:09.00]<00:01.00>twice").is_empty());
        assert!(has_word_stamps("a<00:01.00>b"));
        assert!(!has_word_stamps("a <b> c"));
    }

    #[test]
    fn only_english_translations_are_kept() {
        let lines = |t: &[&str]| t.iter().enumerate().map(|(i, s)| Line { at_ms: i as u64 * 1000, text: s.to_string() }).collect::<Vec<_>>();
        assert_eq!(english_only(lines(&["I only left a goodbye", "That was all"])).len(), 2);
        assert!(english_only(lines(&["你只留下了一句再见", "简短的一句话"])).is_empty());
        // A stray foreign word does not condemn an English translation, but a mostly foreign one goes whole.
        assert_eq!(english_only(lines(&["Tokyo at night, 東京", "we run until the morning comes"])).len(), 2);
        assert!(english_only(lines(&["夜に駆ける夜に駆ける", "ok"])).is_empty());
        assert!(english_only(lines(&["...", "!!"])).is_empty());
    }

    #[test]
    fn current_line_lookup() {
        let l = parse("[00:01.00]a\n[00:03.00]b\n[00:05.00]c");
        assert_eq!(current(&l, 500), None);
        assert_eq!(current(&l, 1000), Some(0));
        assert_eq!(current(&l, 4999), Some(1));
        assert_eq!(current(&l, 99_000), Some(2));
    }

    #[test]
    fn finds_the_line_that_starts_about_now() {
        let l = parse("[00:01.00]a\n[00:03.00]b\n[00:05.00]c");
        assert_eq!(line_at(&l, 3000, 0).map(|x| x.text.as_str()), Some("b"));
        assert_eq!(line_at(&l, 3400, 500).map(|x| x.text.as_str()), Some("b"));
        assert_eq!(line_at(&l, 4000, 500), None);
        assert_eq!(line_at(&[], 0, 500), None);
    }

    #[test]
    fn plain_lyrics_spread_over_duration() {
        let l = from_plain("one\n\ntwo\nthree", 4000);
        assert_eq!(l.iter().map(|x| x.at_ms).collect::<Vec<_>>(), vec![1000, 2000, 3000]);
    }
}
