//! Parsing of LRC synced lyrics as returned by LRCLIB.

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at_ms: u64,
    pub text: String,
}

/// Parses `[mm:ss.xx] text` lines; lines with several stamps are repeated, metadata tags skipped.
pub fn parse(lrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_stamp(&stripped[..end]) {
                Some(ms) => stamps.push(ms),
                None => break,
            }
            rest = stripped[end + 1..].trim_start();
        }
        for ms in stamps {
            lines.push(Line { at_ms: ms, text: rest.trim().to_string() });
        }
    }
    lines.sort_by_key(|l| l.at_ms);
    lines
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
    fn current_line_lookup() {
        let l = parse("[00:01.00]a\n[00:03.00]b\n[00:05.00]c");
        assert_eq!(current(&l, 500), None);
        assert_eq!(current(&l, 1000), Some(0));
        assert_eq!(current(&l, 4999), Some(1));
        assert_eq!(current(&l, 99_000), Some(2));
    }

    #[test]
    fn plain_lyrics_spread_over_duration() {
        let l = from_plain("one\n\ntwo\nthree", 4000);
        assert_eq!(l.iter().map(|x| x.at_ms).collect::<Vec<_>>(), vec![1000, 2000, 3000]);
    }
}
