//! Word timing for lyric lines, so a highlight can follow the singing word by word. It comes from
//! NetEase's word-by-word lyrics where the song has them (`parse_yrc`, matched to the lines on show
//! by `attach`) and is otherwise estimated from the text (`estimate`).
//!
//! A `Word` does not hold its text: it covers `chars` characters of its line, in order, so the words
//! of a line always add up to the whole line.

use crate::lrc::Line;

/// A line's start may differ this much between two services and still be the same line.
const MATCH_TOLERANCE_MS: u64 = 1500;
/// Words sung up to this long after a line starts are looked at when lining its words up, and a line
/// is taken to last no longer than this when no line follows it.
const MAX_LINE_SPAN_MS: u64 = 10_000;
const WINDOW_AFTER_MS: u64 = 500;
/// At least this many tenths of a line's words must be found among the words sung.
const MIN_ALIGNED_TENTHS: usize = 6;
/// Time given to each word nobody sang a match for, when nothing after it fixes its end.
const UNMATCHED_MS: u64 = 350;
/// The last piece of a line is held a little longer, as sung lines tend to end on a long note.
const LAST_WEIGHT: f32 = 1.3;

/// One timed piece of a lyric line: a word, or a syllable or character where that is how it is timed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    pub start_ms: u64,
    pub dur_ms: u64,
    /// How many characters of the line it covers, spaces and punctuation after it included.
    pub chars: usize,
}

/// A word as the service gives it, with its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedWord {
    pub start_ms: u64,
    pub dur_ms: u64,
    pub text: String,
}

/// A lyric line with its words, as read from word-by-word lyrics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedLine {
    pub start_ms: u64,
    pub text: String,
    pub words: Vec<TimedWord>,
}

// ------------------------------------------------------------------ reading

/// The `(start,duration,0)` tag at the start of `s`: its numbers and its length in bytes.
fn tag_at(s: &str) -> Option<(u64, u64, usize)> {
    let inner = s.strip_prefix('(')?;
    let end = inner.find(')')?;
    let mut parts = inner[..end].split(',');
    let (start, dur, flag) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || ![start, dur, flag].iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((start.parse().ok()?, dur.parse().ok()?, end + 2))
}

/// Reads NetEase word-by-word lyrics: lines like `[start,dur](start,dur,0)word(start,dur,0)word`.
/// The JSON credit lines that open them are skipped. A `(` that is not a tag stays in the text.
pub fn parse_yrc(text: &str) -> Vec<TimedLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let Some(body) = raw.trim().strip_prefix('[') else { continue };
        let Some(close) = body.find(']') else { continue };
        let Some((start, _)) = body[..close].split_once(',') else { continue };
        let Ok(start_ms) = start.trim().parse::<u64>() else { continue };
        let words = parse_words(&body[close + 1..]);
        let line: String = words.iter().map(|w| w.text.as_str()).collect();
        if !line.trim().is_empty() {
            lines.push(TimedLine { start_ms, text: line.trim().to_string(), words });
        }
    }
    lines.sort_by_key(|l| l.start_ms);
    lines
}

fn parse_words(mut rest: &str) -> Vec<TimedWord> {
    let mut words: Vec<TimedWord> = Vec::new();
    while let Some((start_ms, dur_ms, used)) = tag_at(rest) {
        rest = &rest[used..];
        // The word runs to the next tag; a bracket that is not one belongs to the text.
        let mut end = rest.len();
        for (i, _) in rest.match_indices('(') {
            if tag_at(&rest[i..]).is_some() {
                end = i;
                break;
            }
        }
        if !rest[..end].is_empty() {
            words.push(TimedWord { start_ms, dur_ms, text: rest[..end].to_string() });
        }
        rest = &rest[end..];
    }
    words
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// At most this share of a line's letters may differ between two services and it still be the same
/// line: one service may write a kanji in its Simplified Chinese form (間 as 间, 無 as 无).
const MAX_DIFFERENT_PERCENT: usize = 15;
/// Lines shorter than this must match exactly, as one letter is too large a share of them.
const MIN_CHARS_TO_DIFFER: usize = 6;

/// Whether two lines are the same line, letter for letter but for case, punctuation and a few letters
/// spelled differently. Their letters line up one to one, so a service's words can be laid over the other's.
pub(crate) fn same_text(a: &str, b: &str) -> bool {
    let (a, b) = (norm_of(a), norm_of(b));
    if a == b {
        return true;
    }
    let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    a.len() == b.len() && a.len() >= MIN_CHARS_TO_DIFFER && differ * 100 <= a.len() * MAX_DIFFERENT_PERCENT
}

/// Gives each timed word the stretch of `line` it covers, in characters. The words' letters and digits
/// must be those of the line, in order, and the stretches then add up to the whole line.
fn cover(line: &str, words: &[TimedWord]) -> Vec<Word> {
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    let mut out: Vec<Word> = Vec::new();
    for w in words {
        let need = w.text.chars().filter(|c| is_word_char(*c)).count();
        let from = at;
        let mut seen = 0;
        while at < chars.len() && seen < need {
            seen += is_word_char(chars[at]) as usize;
            at += 1;
        }
        // The spaces and punctuation the service ends the word with belong to it.
        let trailing = w.text.chars().rev().take_while(|c| !is_word_char(*c)).count();
        for _ in 0..trailing {
            if at < chars.len() && !is_word_char(chars[at]) {
                at += 1;
            }
        }
        if at > from {
            out.push(Word { start_ms: w.start_ms, dur_ms: w.dur_ms, chars: at - from });
        }
    }
    if let Some(last) = out.last_mut() {
        last.chars += chars.len() - at;
    }
    out
}

/// Word timing for each of `lines`, from the word-by-word `timed` lines. A line that is the same as one
/// of them, starting about when it does, takes its words as they are. Any other line (services break
/// and spell lines differently) is lined up with the words sung around its time, where enough of its
/// words are among them. Lines with no such match get an empty list.
pub fn attach(lines: &[Line], timed: &[TimedLine]) -> Vec<Vec<Word>> {
    let stream = flatten(timed);
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let same = timed
                .iter()
                .filter(|t| t.start_ms.abs_diff(line.at_ms) <= MATCH_TOLERANCE_MS && same_text(&t.text, &line.text))
                .min_by_key(|t| t.start_ms.abs_diff(line.at_ms));
            let words = match same {
                Some(t) => cover(&line.text, &t.words),
                None => align(line, lines.get(i + 1).map(|n| n.at_ms), &stream),
            };
            starting_with(words, line.at_ms)
        })
        .collect()
}

/// Words that another service has starting before the line does start with it instead: the two
/// services' clocks for a line differ by a few hundred ms either way, and the line on show is this one's.
fn starting_with(mut words: Vec<Word>, line_ms: u64) -> Vec<Word> {
    for w in words.iter_mut().filter(|w| w.start_ms < line_ms) {
        w.dur_ms = (w.start_ms + w.dur_ms).saturating_sub(line_ms);
        w.start_ms = line_ms;
    }
    words
}

// ------------------------------------------------------------------ vetting

/// Above this many words a second, a line is not physically sung; ordinary singing is 1-3 and the fastest rap about 9.
const IMPOSSIBLE_UNITS_PER_S: f64 = 8.0;
/// The same for lines mostly of Chinese, Japanese or Korean characters, which are sung faster per unit.
const IMPOSSIBLE_CJK_UNITS_PER_S: f64 = 12.0;
/// Lines shorter than this say nothing about the rate ("Ooh, ah" lands inside a few hundred ms).
const MIN_UNITS_TO_JUDGE: usize = 3;
/// A song needs this many judgeable lines before its rate condemns it.
const MIN_JUDGED_LINES: usize = 5;
/// A word may start this much before its line, or after the next line begins, and still belong to it.
const WORD_SLACK_MS: u64 = 200;
/// Share of lines with words that may be incoherent before the whole song's word timing is dropped.
const INCOHERENT_SHARE: f64 = 0.30;
const MIN_COHERENCE_LINES: usize = 3;
/// Lines at an exactly even step, from this many of them, are generated rather than transcribed.
const UNIFORM_MIN_LINES: usize = 20;
/// Timing running past the song by this much (15 % and 5 s) belongs to another recording.
const OVERRUN_PERCENT: u64 = 115;
const OVERRUN_SLACK_MS: u64 = 5000;

/// Why a song's word timing was thrown away, when it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// Most lines would have to be sung faster than anyone sings.
    TooFast,
    /// Words outside their own line or running backwards on many lines: another clock than the lines.
    Incoherent,
    /// Evenly spaced lines, or timing that runs past the song: made up, or for another recording.
    Decoy,
}

/// Sung units of a line, as (all units, CJK units): each Chinese, Japanese or Korean character is one, and
/// the rest of the text counts its words.
fn units(text: &str) -> (usize, usize) {
    let (mut cjk, mut words, mut in_word) = (0, 0, false);
    for c in text.chars() {
        if matches!(kind(c), Kind::Kana | Kind::Ideograph) {
            cjk += 1;
            in_word = false;
        } else if c.is_whitespace() {
            in_word = false;
        } else if c.is_alphanumeric() && !in_word {
            words += 1;
            in_word = true;
        }
    }
    (cjk + words, cjk)
}

fn span_ms(words: &[Word]) -> u64 {
    match (words.first(), words.last()) {
        (Some(first), Some(last)) => (last.start_ms + last.dur_ms).saturating_sub(first.start_ms),
        _ => 0,
    }
}

fn impossibly_fast(text: &str, words: &[Word]) -> bool {
    let span = span_ms(words);
    let (n, cjk) = units(text);
    if span == 0 || n < MIN_UNITS_TO_JUDGE {
        return false;
    }
    let limit = if cjk * 2 > n { IMPOSSIBLE_CJK_UNITS_PER_S } else { IMPOSSIBLE_UNITS_PER_S };
    n as f64 * 1000.0 / span as f64 > limit
}

/// A line whose words contradict the line itself: one starting well before it, one starting after the
/// next line has begun, or starts that run backwards.
fn incoherent(line: &Line, next_at: Option<u64>, words: &[Word]) -> bool {
    let mut prev = 0;
    for (k, w) in words.iter().enumerate() {
        let before = w.start_ms + WORD_SLACK_MS < line.at_ms;
        let after = next_at.is_some_and(|n| w.start_ms > n + WORD_SLACK_MS);
        if before || after || (k > 0 && w.start_ms < prev) {
            return true;
        }
        prev = w.start_ms;
    }
    false
}

fn evenly_spaced(lines: &[Line]) -> bool {
    if lines.len() < UNIFORM_MIN_LINES {
        return false;
    }
    let step = lines[1].at_ms.abs_diff(lines[0].at_ms);
    lines.windows(2).all(|p| p[1].at_ms.abs_diff(p[0].at_ms).abs_diff(step) <= 1)
}

/// Checks a song's word timing the way a listener would: is it physically singable, does it sit on its
/// own lines, and is it for this recording? Some such timing is wrong in every way a line start is right:
/// all the words burst out in the first fraction of a second, or on another clock. A song whose timing
/// fails loses all of it (its lines keep their starts and fall back to estimated words); on a song that passes, a
/// single bad line loses only its own words. `duration_ms` is the song's length, 0 when unknown.
pub fn vet(lines: &[Line], mut words: Vec<Vec<Word>>, duration_ms: u64) -> Result<Vec<Vec<Word>>, Rejection> {
    words.resize(lines.len(), Vec::new());
    let (mut judged, mut fast, mut with_words, mut incoherent_lines) = (0, 0, 0, 0);
    for (i, line) in lines.iter().enumerate() {
        let w = &words[i];
        if w.is_empty() {
            continue;
        }
        with_words += 1;
        if incoherent(line, lines.get(i + 1).map(|n| n.at_ms), w) {
            incoherent_lines += 1;
        }
        let (n, _) = units(&line.text);
        if span_ms(w) > 0 && n >= MIN_UNITS_TO_JUDGE {
            judged += 1;
            fast += impossibly_fast(&line.text, w) as usize;
        }
    }
    if with_words == 0 {
        return Ok(Vec::new());
    }
    let overrun = duration_ms > 0
        && words.iter().flatten().map(|w| w.start_ms + w.dur_ms).max().is_some_and(|end| end > duration_ms * OVERRUN_PERCENT / 100 + OVERRUN_SLACK_MS);
    if overrun || evenly_spaced(lines) {
        return Err(Rejection::Decoy);
    }
    if judged >= MIN_JUDGED_LINES && fast * 2 >= judged {
        return Err(Rejection::TooFast);
    }
    if with_words >= MIN_COHERENCE_LINES && incoherent_lines as f64 >= with_words as f64 * INCOHERENT_SHARE {
        return Err(Rejection::Incoherent);
    }
    for (i, line) in lines.iter().enumerate() {
        if impossibly_fast(&line.text, &words[i]) || incoherent(line, lines.get(i + 1).map(|n| n.at_ms), &words[i]) {
            words[i].clear();
        }
    }
    Ok(words)
}

/// A word of the song and when it is sung, with the letters it is matched by.
struct Sung {
    norm: Vec<char>,
    start_ms: u64,
    dur_ms: u64,
}

fn norm_of(s: &str) -> Vec<char> {
    s.chars().filter(|c| is_word_char(*c)).flat_map(char::to_lowercase).collect()
}

fn flatten(timed: &[TimedLine]) -> Vec<Sung> {
    let mut all: Vec<Sung> = timed
        .iter()
        .flat_map(|l| l.words.iter())
        .map(|w| Sung { norm: norm_of(&w.text), start_ms: w.start_ms, dur_ms: w.dur_ms })
        .filter(|w| !w.norm.is_empty())
        .collect();
    all.sort_by_key(|w| w.start_ms);
    all
}

fn is_cjk(c: char) -> bool {
    matches!(kind(c), Kind::Kana | Kind::Ideograph)
}

/// A line's words as (letters, characters the word and the spaces after it take). Punctuation on its
/// own joins the word before it. Chinese, Japanese and Korean are written without spaces and timed a
/// character at a time, so each of their characters is a word of its own.
fn tokens(text: &str) -> Vec<(Vec<char>, usize)> {
    let mut out: Vec<(Vec<char>, usize)> = Vec::new();
    let mut lead = 0;
    let mut push = |piece: &str, out: &mut Vec<(Vec<char>, usize)>| {
        let len = piece.chars().count();
        let norm = norm_of(piece);
        match (norm.is_empty(), out.last_mut()) {
            (true, Some(prev)) => prev.1 += len,
            (true, None) => lead += len,
            (false, _) => {
                out.push((norm, len + lead));
                lead = 0;
            }
        }
    };
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, first)) = chars.peek() {
        // A character of its own, or a run of other non-space characters, then the spaces after it.
        let mut end = start + first.len_utf8();
        chars.next();
        if !is_cjk(first) {
            while let Some((i, c)) = chars.next_if(|&(_, c)| !c.is_whitespace() && !is_cjk(c)) {
                end = i + c.len_utf8();
            }
        }
        while let Some((i, c)) = chars.next_if(|&(_, c)| c.is_whitespace()) {
            end = i + c.len_utf8();
        }
        push(&text[start..end], &mut out);
    }
    out
}

/// Lines up one line's words with the words sung around its time: the longest run of its words found
/// in order, each taking its timing, and those between them sharing the time between their neighbours.
fn align(line: &Line, next_at: Option<u64>, stream: &[Sung]) -> Vec<Word> {
    let toks = tokens(&line.text);
    let from = line.at_ms.saturating_sub(MATCH_TOLERANCE_MS);
    let to = next_at.unwrap_or(line.at_ms + MAX_LINE_SPAN_MS).min(line.at_ms + MAX_LINE_SPAN_MS) + WINDOW_AFTER_MS;
    let near: Vec<&Sung> = stream.iter().filter(|w| w.start_ms >= from && w.start_ms <= to).collect();
    if toks.is_empty() || near.is_empty() {
        return Vec::new();
    }
    // Longest common subsequence of the line's words and the words sung.
    let (n, m) = (toks.len(), near.len());
    let mut len = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[i][j] = if toks[i].0 == near[j].norm { len[i + 1][j + 1] + 1 } else { len[i + 1][j].max(len[i][j + 1]) };
        }
    }
    if (len[0][0] as usize) * 10 < n * MIN_ALIGNED_TENTHS {
        return Vec::new();
    }
    let mut hit: Vec<Option<&Sung>> = vec![None; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        // A word sung before the line starts may end the line before; when passing it over loses
        // nothing, the same word sung later is taken instead.
        let early = near[j].start_ms + WORD_SLACK_MS < line.at_ms && len[i][j + 1] == len[i][j];
        if toks[i].0 == near[j].norm && !early {
            hit[i] = Some(near[j]);
            i += 1;
            j += 1;
        } else if len[i + 1][j] >= len[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let mut out: Vec<Word> = Vec::with_capacity(n);
    let mut k = 0;
    while k < n {
        if let Some(w) = hit[k] {
            out.push(Word { start_ms: w.start_ms, dur_ms: w.dur_ms, chars: toks[k].1 });
            k += 1;
            continue;
        }
        // A run of words nobody sang a match for shares the time from the last one's end to the next one's start.
        let run_end = (k..n).find(|&x| hit[x].is_some()).unwrap_or(n);
        let mut begin = out.last().map_or(line.at_ms, |w| w.start_ms + w.dur_ms);
        let count = (run_end - k) as u64;
        let finish = match hit.get(run_end).copied().flatten() {
            Some(next_word) => next_word.start_ms.max(begin + count),
            // Words left over at the end fit in before the next line, rather than running into it:
            // there they would take a line that is no longer on show.
            None => {
                let last_start = out.last().map_or(line.at_ms, |w| w.start_ms);
                let limit = next_at.map_or(u64::MAX, |n| n.max(last_start));
                begin = begin.min(limit);
                (begin + count * UNMATCHED_MS).min(limit).max(begin)
            }
        };
        let share = (finish - begin) / count;
        for (step, x) in (k..run_end).enumerate() {
            out.push(Word { start_ms: begin + share * step as u64, dur_ms: share, chars: toks[x].1 });
        }
        k = run_end;
    }
    out
}

// ------------------------------------------------------------------ estimating

#[derive(PartialEq)]
enum Kind {
    Kana,
    Ideograph,
    Letters,
    Other,
}

fn kind(c: char) -> Kind {
    match c as u32 {
        0x3040..=0x30FF => Kind::Kana,
        0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0x20000..=0x2FFFF => Kind::Ideograph,
        _ if c.is_alphanumeric() => Kind::Letters,
        _ => Kind::Other,
    }
}

/// Small kana join the sound before them; the sokuon and the long-vowel mark are sounds of their own.
fn is_small_kana(c: char) -> bool {
    "ぁぃぅぇぉゃゅょゎァィゥェォャュョヮ".contains(c)
}

fn is_vowel(w: &[char], i: usize) -> bool {
    let c = w[i].to_lowercase().next().unwrap_or(w[i]);
    "aeiouàáâãäåæèéêëìíîïòóôõöøùúûüœ".contains(c) || (c == 'y' && i > 0)
}

/// Splits a word into syllables, as character counts: one at each run of vowels, with a lone
/// consonant between two runs going to the second and the first of two staying with the first.
fn syllables(w: &[char]) -> Vec<usize> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < w.len() {
        if is_vowel(w, i) {
            let s = i;
            while i < w.len() && is_vowel(w, i) {
                i += 1;
            }
            runs.push((s, i));
        } else {
            i += 1;
        }
    }
    // A final silent e ("love") makes no syllable, but "-le" ("table") does.
    if let Some(&(s, e)) = runs.last() {
        let silent = runs.len() > 1 && e == w.len() && e - s == 1 && w[s].to_lowercase().next() == Some('e') && w.len() > 2 && w[s - 1].to_lowercase().next() != Some('l');
        if silent {
            runs.pop();
        }
    }
    if runs.len() < 2 {
        return vec![w.len()];
    }
    let mut cuts: Vec<usize> = Vec::new();
    for pair in runs.windows(2) {
        let between = pair[1].0 - pair[0].1;
        cuts.push(if between >= 2 { pair[0].1 + 1 } else { pair[0].1 });
    }
    let mut lens = Vec::new();
    let mut prev = 0;
    for c in cuts.into_iter().chain([w.len()]) {
        lens.push(c - prev);
        prev = c;
    }
    lens
}

/// A line's sounds as (characters, weight): a syllable of a Latin word, a mora of kana, a character of
/// Chinese or Korean. Spaces and punctuation add to the sound before them and weigh nothing.
fn pieces(text: &str) -> Vec<(usize, f32)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(usize, f32)> = Vec::new();
    // Characters before the first sound join it.
    let mut pending = 0;
    let push = |out: &mut Vec<(usize, f32)>, n: usize, pending: &mut usize| {
        out.push((n + *pending, 1.0));
        *pending = 0;
    };
    let mut i = 0;
    while i < chars.len() {
        match kind(chars[i]) {
            Kind::Kana => {
                let s = i;
                i += 1;
                while i < chars.len() && is_small_kana(chars[i]) {
                    i += 1;
                }
                push(&mut out, i - s, &mut pending);
            }
            Kind::Ideograph => {
                push(&mut out, 1, &mut pending);
                i += 1;
            }
            Kind::Letters => {
                let s = i;
                while i < chars.len() && (kind(chars[i]) == Kind::Letters || (matches!(chars[i], '\'' | '’') && i > s && chars.get(i + 1).is_some_and(|n| kind(*n) == Kind::Letters))) {
                    i += 1;
                }
                for n in syllables(&chars[s..i]) {
                    push(&mut out, n, &mut pending);
                }
            }
            Kind::Other => {
                match out.last_mut() {
                    Some(last) => last.0 += 1,
                    None => pending += 1,
                }
                i += 1;
            }
        }
    }
    out
}

/// How many sounds a line has: syllables of Latin words, and each kana mora or Chinese or Korean character.
pub fn sounds(text: &str) -> usize {
    pieces(text).len()
}

/// Words for a line that starts at `start_ms` and is sung over `sing_ms`, made up from its text: each
/// sound gets a share of the time as long as it is. Not what the singer did, but it moves like it.
pub fn estimate(text: &str, start_ms: u64, sing_ms: u64) -> Vec<Word> {
    let mut parts = pieces(text);
    if let Some(last) = parts.last_mut() {
        last.1 *= LAST_WEIGHT;
    }
    let total: f32 = parts.iter().map(|p| p.1).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut done = 0.0;
    parts
        .iter()
        .map(|&(chars, weight)| {
            let word = Word { start_ms: start_ms + (done / total * sing_ms as f32) as u64, dur_ms: (weight / total * sing_ms as f32) as u64, chars };
            done += weight;
            word
        })
        .collect()
}

// ------------------------------------------------------------------ following

/// How many characters of the line have been sung at `pos_ms`, with a fraction for the word being
/// sung. Between words the count holds.
pub fn sung_chars(words: &[Word], pos_ms: u64) -> f32 {
    let mut sung = 0.0;
    for w in words {
        if pos_ms <= w.start_ms {
            break;
        }
        if w.dur_ms == 0 || pos_ms >= w.start_ms + w.dur_ms {
            sung += w.chars as f32;
        } else {
            sung += w.chars as f32 * (pos_ms - w.start_ms) as f32 / w.dur_ms as f32;
            break;
        }
    }
    sung
}

#[cfg(test)]
mod tests {
    use super::*;

    const YRC: &str = concat!(
        "{\"t\":0,\"c\":[{\"tx\":\"作词: \"},{\"tx\":\"X\"}]}\n",
        "[14100,3570](14100,420,0)White (14520,480,0)shirt(15000,240,0), (15240,480,0)now\n",
        "[20000,2000](20000,300,0)(oh) (20300,500,0)you"
    );

    fn line(at_ms: u64, text: &str) -> Line {
        Line { at_ms, text: text.into() }
    }

    #[test]
    fn word_by_word_lines_are_read_and_credits_skipped() {
        let l = parse_yrc(YRC);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].start_ms, l[0].text.as_str()), (14100, "White shirt, now"));
        assert_eq!(l[0].words.len(), 4);
        assert_eq!(l[0].words[1], TimedWord { start_ms: 14520, dur_ms: 480, text: "shirt".into() });
        // A bracket that is not a timing tag stays in the text.
        assert_eq!(l[1].text, "(oh) you");
        assert!(parse_yrc("not lyrics\n[bad](1,2,0)x").is_empty());
    }

    #[test]
    fn words_cover_the_whole_line_they_are_attached_to() {
        let l = parse_yrc(YRC);
        let lines = [line(14200, "White shirt, now"), line(99_000, "unrelated")];
        let words = attach(&lines, &l);
        assert_eq!(words[0].iter().map(|w| w.chars).collect::<Vec<_>>(), vec![6, 5, 2, 3]);
        assert_eq!(words[0].iter().map(|w| w.chars).sum::<usize>(), "White shirt, now".chars().count());
        assert!(words[1].is_empty());
    }

    #[test]
    fn a_line_with_other_words_or_another_time_gets_none() {
        let l = parse_yrc(YRC);
        assert!(attach(&[line(14100, "Black coat, then")], &l)[0].is_empty());
        assert!(attach(&[line(20_000 + 5000, "(oh) you")], &l)[0].is_empty());
        // Case and punctuation do not matter.
        assert!(!attach(&[line(14100, "white SHIRT now")], &l)[0].is_empty());
    }

    #[test]
    fn lines_broken_differently_are_lined_up_by_their_words() {
        let timed = parse_yrc(concat!(
            "[9520,2000](9520,210,0)If (9730,120,0)you (9850,300,0)wanna (10150,180,0)run (10330,210,0)away (10540,180,0)with (10720,150,0)me\n",
            "[10870,2500](10870,100,0)I (10970,200,0)know (11170,100,0)a (11270,500,0)galaxy"
        ));
        let text = "If you wanna run away with me, I know a galaxy";
        let w = attach(&[line(9520, text), line(14_000, "Another line")], &timed);
        assert_eq!(w[0].len(), 11);
        assert_eq!((w[0][0].start_ms, w[0][6].start_ms, w[0][10].start_ms), (9520, 10_720, 11_270));
        assert_eq!(w[0].iter().map(|x| x.chars).sum::<usize>(), text.chars().count());
        assert!(w[1].is_empty());
    }

    #[test]
    fn a_line_with_a_kanji_spelled_another_way_still_takes_its_words() {
        // The service writes 間 in its Simplified Chinese form 间.
        let timed = parse_yrc("[15288,3000](15288,200,0)い(15488,200,0)つ(15688,200,0)の(15888,200,0)间(16088,200,0)に(16288,200,0)か(16488,200,0)そ(16688,200,0)れ(16888,200,0)は");
        let w = attach(&[line(14890, "いつの間にかそれは")], &timed);
        assert_eq!(w[0].len(), 9);
        assert_eq!((w[0][3].start_ms, w[0][3].chars), (15_888, 1));
        // Short lines must still match exactly.
        assert!(!same_text("間に", "间に"));
        assert!(same_text("いつの間にかそれは", "いつの间にかそれは"));
    }

    #[test]
    fn japanese_lines_broken_differently_are_lined_up_by_character() {
        let timed = parse_yrc("[21700,3000](21700,200,0)物(21900,200,0)差(22100,200,0)し(22300,200,0)だ\n[24711,2000](24711,224,0)何(24935,800,0)か");
        let text = "物差しだ 何か";
        let w = attach(&[line(21_440, text), line(27_880, "next")], &timed);
        assert_eq!(w[0].iter().map(|x| x.start_ms).collect::<Vec<_>>(), vec![21_700, 21_900, 22_100, 22_300, 24_711, 24_935]);
        assert_eq!(w[0].iter().map(|x| x.chars).sum::<usize>(), text.chars().count());
    }

    #[test]
    fn a_word_from_the_line_before_is_not_taken_when_the_same_word_comes_later() {
        // "た" ends the line before (at 9900) and is sung again in this line.
        let timed = parse_yrc("[9000,1000](9900,100,0)た\n[10000,2000](10000,300,0)君(10300,300,0)と(10600,300,0)た(10900,300,0)び");
        let w = attach(&[line(10_000, "君とたび")], &timed);
        assert_eq!(w[0].iter().map(|x| x.start_ms).collect::<Vec<_>>(), vec![10_000, 10_300, 10_600, 10_900]);
    }

    #[test]
    fn words_left_over_at_the_end_fit_in_before_the_next_line() {
        // The service breaks the line in two and sings "dd" long after this line's slot ends.
        let timed = parse_yrc("[1000,1000](1000,300,0)aa (1300,300,0)bb (1600,300,0)cc\n[9000,1000](9000,300,0)dd");
        let lines = [line(1000, "aa bb cc dd"), line(2000, "next")];
        let w = attach(&lines, &timed);
        assert_eq!(w[0].len(), 4);
        assert!(w[0].iter().all(|x| x.start_ms <= 2000), "{:?}", w[0]);
        assert!(!incoherent(&lines[0], Some(2000), &w[0]));
    }

    #[test]
    fn a_line_another_service_starts_early_starts_with_its_own_line() {
        let timed = parse_yrc("[98216,3000](98216,921,0)仆(99137,371,0)の(99508,195,0)元(99703,232,0)に(99935,211,0)も(100146,252,0)お");
        let lines = [line(98_710, "僕の元にもお")];
        let w = attach(&lines, &timed);
        assert_eq!((w[0][0].start_ms, w[0][0].dur_ms), (98_710, 427));
        assert_eq!(w[0][1].start_ms, 99_137);
        assert!(!incoherent(&lines[0], None, &w[0]));
    }

    #[test]
    fn a_word_nobody_matched_takes_the_gap_between_its_neighbours() {
        let timed = parse_yrc("[1000,1000](1000,300,0)Hello (1700,300,0)world");
        let w = attach(&[line(1000, "Hello big world")], &timed);
        assert_eq!(w[0].len(), 3);
        assert_eq!((w[0][1].start_ms, w[0][1].dur_ms), (1300, 400));
        assert_eq!(w[0].iter().map(|x| x.chars).sum::<usize>(), 15);
    }

    /// `count` lines of five words each, the words of each spread over `span` ms from `offset` after its start.
    fn song(count: usize, span: u64, offset: i64) -> (Vec<Line>, Vec<Vec<Word>>) {
        let lines: Vec<Line> = (0..count).map(|i| line(10_000 + i as u64 * 4000 + (i as u64 % 3) * 100, "aa bb cc dd ee")).collect();
        let words = lines
            .iter()
            .map(|l| {
                let start = (l.at_ms as i64 + offset).max(0) as u64;
                (0..5).map(|k| Word { start_ms: start + k * span / 5, dur_ms: span / 5, chars: 3 }).collect()
            })
            .collect();
        (lines, words)
    }

    #[test]
    fn singable_word_timing_passes_untouched() {
        let (lines, words) = song(8, 3000, 0);
        assert_eq!(vet(&lines, words.clone(), 200_000), Ok(words));
        assert_eq!(vet(&lines, vec![Vec::new(); 8], 200_000), Ok(Vec::new()));
    }

    #[test]
    fn timing_that_bursts_out_is_thrown_away() {
        // Five words in 400 ms is 12.5 words a second.
        let (lines, words) = song(8, 400, 0);
        assert_eq!(vet(&lines, words, 200_000), Err(Rejection::TooFast));
    }

    #[test]
    fn one_bad_line_loses_only_its_own_words() {
        let (lines, mut words) = song(8, 3000, 0);
        words[3] = (0..5).map(|k| Word { start_ms: lines[3].at_ms + k * 50, dur_ms: 50, chars: 3 }).collect();
        let vetted = vet(&lines, words, 200_000).unwrap();
        assert!(vetted[3].is_empty());
        assert_eq!(vetted.iter().filter(|w| !w.is_empty()).count(), 7);
    }

    #[test]
    fn words_on_another_clock_are_thrown_away() {
        let (lines, words) = song(6, 3000, -6000);
        assert_eq!(vet(&lines, words, 200_000), Err(Rejection::Incoherent));
    }

    #[test]
    fn made_up_or_overrunning_timing_is_thrown_away() {
        let even: Vec<Line> = (0..24).map(|i| line(i * 4000, "aa bb cc dd ee")).collect();
        let words = even.iter().map(|l| vec![Word { start_ms: l.at_ms, dur_ms: 2000, chars: 14 }]).collect();
        assert_eq!(vet(&even, words, 200_000), Err(Rejection::Decoy));
        let (lines, words) = song(8, 3000, 0);
        assert_eq!(vet(&lines, words.clone(), 20_000), Err(Rejection::Decoy));
        // Not knowing the length never condemns.
        assert!(vet(&lines, words, 0).is_ok());
    }

    #[test]
    fn units_count_words_and_cjk_characters() {
        assert_eq!(units("If you wanna run"), (4, 0));
        assert_eq!(units("夜空中最亮"), (5, 5));
        assert_eq!(units("Oh 夜空 yeah"), (4, 2));
        assert_eq!(units("  ...  "), (0, 0));
    }

    #[test]
    fn chinese_is_timed_per_character() {
        let l = parse_yrc("[35720,900](35720,520,0)夜(36240,250,0)空(36490,270,0)中");
        let w = attach(&[line(35720, "夜空中")], &l);
        assert_eq!(w[0].iter().map(|w| w.chars).collect::<Vec<_>>(), vec![1, 1, 1]);
    }

    #[test]
    fn english_words_split_into_syllables() {
        let s = |w: &str| syllables(&w.chars().collect::<Vec<_>>());
        assert_eq!(s("wanna"), vec![3, 2]);
        assert_eq!(s("levitating"), vec![2, 2, 2, 4]);
        assert_eq!(s("love"), vec![4]);
        assert_eq!(s("table"), vec![3, 2]);
        assert_eq!(s("away"), vec![1, 3]);
        assert_eq!(s("rhythm").iter().sum::<usize>(), 6);
        assert_eq!(s("123"), vec![3]);
    }

    #[test]
    fn estimates_cover_the_line_and_the_time() {
        for text in ["If you wanna run away with me", "勘冴えて悔しいわ", "きゃりーぱみゅぱみゅ", "(Ooh) yeah, yeah!", "안녕하세요 세계", "  "] {
            let w = estimate(text, 1000, 4000);
            if text.trim().is_empty() {
                assert!(w.is_empty());
                continue;
            }
            assert_eq!(w.iter().map(|w| w.chars).sum::<usize>(), text.chars().count(), "{text}");
            assert_eq!(w[0].start_ms, 1000);
            assert!(w.windows(2).all(|p| p[0].start_ms <= p[1].start_ms));
            assert!(w.last().unwrap().start_ms + w.last().unwrap().dur_ms <= 5000 + 1);
        }
    }

    #[test]
    fn kana_are_timed_per_mora() {
        let w = estimate("きゃりー", 0, 1000);
        assert_eq!(w.iter().map(|w| w.chars).collect::<Vec<_>>(), vec![2, 1, 1]);
    }

    #[test]
    fn longer_sounds_take_longer_and_the_last_is_held() {
        let w = estimate("ab cd", 0, 1000);
        assert_eq!(w.iter().map(|w| w.chars).collect::<Vec<_>>(), vec![3, 2]);
        assert!(w[1].dur_ms > w[0].dur_ms);
    }

    #[test]
    fn the_sung_count_follows_each_word_and_holds_between_them() {
        let w = [Word { start_ms: 1000, dur_ms: 500, chars: 4 }, Word { start_ms: 2000, dur_ms: 1000, chars: 6 }];
        assert_eq!(sung_chars(&w, 500), 0.0);
        assert_eq!(sung_chars(&w, 1250), 2.0);
        assert_eq!(sung_chars(&w, 1800), 4.0);
        assert_eq!(sung_chars(&w, 2500), 7.0);
        assert_eq!(sung_chars(&w, 9000), 10.0);
        assert_eq!(sung_chars(&[Word { start_ms: 5, dur_ms: 0, chars: 3 }], 6), 3.0);
        assert_eq!(sung_chars(&[], 100), 0.0);
    }
}
