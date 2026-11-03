//! LRCLIB's `lyricsfile`: a small YAML document holding each line, and for word-synced songs each
//! word, with start and end times in ms. Only the shape LRCLIB writes is read: a top-level `lines:`
//! list whose items have `text`, `start_ms`, `end_ms` and optionally a `words:` list of the same keys.
//! Anything else is skipped, and what comes out is vetted by `words::vet` before it is used.

use crate::words::{TimedLine, TimedWord};

#[derive(Default)]
struct Item {
    text: String,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
}

/// The word-timed lines of a lyricsfile; lines without words are left out.
pub fn timed_lines(doc: &str) -> Vec<TimedLine> {
    let mut out = Vec::new();
    let mut in_lines = false;
    // Where line items start, and where the current line's `words:` key sits.
    let mut line_dash: Option<usize> = None;
    let mut words_key: Option<usize> = None;
    let mut line: Option<(Item, Vec<TimedWord>)> = None;
    let mut word: Option<Item> = None;
    let finish_word = |word: &mut Option<Item>, line: &mut Option<(Item, Vec<TimedWord>)>| {
        if let (Some(w), Some((_, words))) = (word.take(), line.as_mut()) {
            if let (Some(start), Some(end)) = (w.start_ms, w.end_ms) {
                words.push(TimedWord { start_ms: start, dur_ms: end.saturating_sub(start), text: w.text });
            }
        }
    };
    let finish_line = |line: &mut Option<(Item, Vec<TimedWord>)>, out: &mut Vec<TimedLine>| {
        if let Some((l, words)) = line.take() {
            if let (Some(start), false) = (l.start_ms, words.is_empty()) {
                out.push(TimedLine { start_ms: start, text: l.text, words });
            }
        }
    };
    for raw in doc.lines() {
        let trimmed = raw.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw.len() - trimmed.len();
        if indent == 0 && !trimmed.starts_with("- ") {
            finish_word(&mut word, &mut line);
            finish_line(&mut line, &mut out);
            in_lines = trimmed == "lines:";
            continue;
        }
        if !in_lines {
            continue;
        }
        let (dash, body, at) = match trimmed.strip_prefix("- ") {
            Some(rest) => (true, rest, indent + 2),
            None => (false, trimmed, indent),
        };
        if dash && line_dash.is_none_or(|d| indent <= d) {
            line_dash = Some(indent);
            finish_word(&mut word, &mut line);
            finish_line(&mut line, &mut out);
            line = Some((Item::default(), Vec::new()));
            words_key = None;
        } else if dash && words_key.is_some_and(|k| indent >= k) {
            finish_word(&mut word, &mut line);
            word = Some(Item::default());
        }
        let Some((key, value)) = body.split_once(':') else { continue };
        let value = value.trim();
        let in_word = words_key.is_some_and(|k| at > k) && word.is_some();
        if !in_word && words_key.is_some_and(|k| at <= k) && key != "words" {
            // Back at the line's own keys after its words.
            finish_word(&mut word, &mut line);
            words_key = None;
        }
        if key == "words" && !in_word {
            words_key = Some(at);
            continue;
        }
        let target = if in_word { word.as_mut() } else { line.as_mut().map(|(l, _)| l) };
        let Some(item) = target else { continue };
        match key {
            "text" => item.text = scalar(value),
            "start_ms" => item.start_ms = value.parse().ok(),
            "end_ms" => item.end_ms = value.parse().ok(),
            _ => {}
        }
    }
    finish_word(&mut word, &mut line);
    finish_line(&mut line, &mut out);
    out
}

/// A YAML scalar's text: quotes taken off ('' inside single quotes is one '; the usual escapes in double).
fn scalar(v: &str) -> String {
    if let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    if let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(c);
                    }
                }
                Some(other) => out.push(other),
                None => {}
            }
        }
        return out;
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "version: '1.0'
metadata:
  title: Song
  duration_ms: 200000
lines:
- text: 'It''s here'
  start_ms: 1000
  end_ms: 2000
  words:
  - text: 'It''s '
    start_ms: 1000
    end_ms: 1400
  - text: here
    start_ms: 1400
    end_ms: 2000
- text: No words on this one
  start_ms: 3000
  end_ms: 4000
- text: \"Last \\\"one\\\"\"
  start_ms: 5000
  words:
    - text: Last
      start_ms: 5000
      end_ms: 5300
    - text: ' \"one\"'
      start_ms: 5300
      end_ms: 6000
  end_ms: 6000
";

    #[test]
    fn word_timed_lines_are_read_and_plain_ones_left_out() {
        let lines = timed_lines(DOC);
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].start_ms, lines[0].text.as_str()), (1000, "It's here"));
        assert_eq!(lines[0].words, vec![
            TimedWord { start_ms: 1000, dur_ms: 400, text: "It's ".into() },
            TimedWord { start_ms: 1400, dur_ms: 600, text: "here".into() },
        ]);
        assert_eq!(lines[1].text, "Last \"one\"");
        assert_eq!(lines[1].words.len(), 2);
        assert_eq!(lines[1].words[1].dur_ms, 700);
    }

    #[test]
    fn a_document_without_words_or_lines_gives_nothing() {
        assert!(timed_lines("version: '1.0'\nlines:\n- text: a\n  start_ms: 1\n  end_ms: 2\n").is_empty());
        assert!(timed_lines("not yaml at all").is_empty());
        assert!(timed_lines("").is_empty());
    }

    #[test]
    fn scalars_lose_their_quotes() {
        assert_eq!(scalar("'a''b'"), "a'b");
        assert_eq!(scalar("\"a\\\"b\\u00e9\""), "a\"bé");
        assert_eq!(scalar("plain"), "plain");
    }
}
