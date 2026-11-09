//! Romaji written as words. Lyrics services give romaji a syllable at a time ("a na ta ga su ki"), one
//! piece for each character of the Japanese line. With those characters beside it, the syllables are put
//! back together into words ("anata ga suki"): a word starts where the writing changes from kana to
//! kanji, or from one script to another, and the common particles stand apart. It reads well for most
//! lines, without a dictionary; a word written all in hiragana may still be split or joined oddly.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Hiragana,
    Katakana,
    Kanji,
    Latin,
    Other,
}

fn script_of(text: &str) -> Script {
    let Some(c) = text.chars().next() else { return Script::Other };
    match c as u32 {
        0x3041..=0x309F => Script::Hiragana,
        // The long-vowel mark ー sits in the katakana block and continues a katakana word.
        0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Script::Katakana,
        0x3005 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Script::Kanji,
        _ if c.is_alphanumeric() => Script::Latin,
        _ => Script::Other,
    }
}

/// The first halves of あの, この, その and どの ("that", "this", "which"), whose の is not a particle.
const DEMONSTRATIVES: [&str; 4] = ["あ", "こ", "そ", "ど"];

/// Where a piece sits in its line, for telling a particle from a letter of a word.
struct Around<'a> {
    prev: Option<Script>,
    next: Option<Script>,
    /// The piece before, when it began a word on its own.
    word_start: Option<&'a str>,
}

/// Whether a piece of the line is a particle, standing apart from the words around it. を always is;
/// は, が and へ are before anything but more hiragana. と, に, で, の and も are right after kanji or
/// katakana, or before them, but not inside hiragana words (こと, もの, なに), nor the の of あの.
fn is_particle(text: &str, at: &Around) -> bool {
    let kanji_or_katakana = |s: Option<Script>| matches!(s, Some(Script::Kanji | Script::Katakana));
    match text {
        "を" => true,
        "は" | "が" | "へ" => at.next != Some(Script::Hiragana),
        "の" if at.word_start.is_some_and(|w| DEMONSTRATIVES.contains(&w)) => false,
        "と" | "に" | "で" | "の" | "も" => kanji_or_katakana(at.prev) || kanji_or_katakana(at.next),
        _ => false,
    }
}

/// Whether a word starts with a piece in `cur` after one in `prev`: kanji after kana starts one (the
/// kana before it end a word), as does any change of script but kanji followed by its kana.
fn starts_word(prev: Script, cur: Script) -> bool {
    match (prev, cur) {
        (Script::Hiragana, Script::Kanji) => true,
        (Script::Kanji, Script::Hiragana) => false,
        (a, b) => a != b,
    }
}

/// The romaji of a line as words, from its pieces as (the line's text for the piece, its romaji).
pub fn words(units: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut prev: Option<Script> = None;
    let mut break_next = false;
    let mut word_start: Option<&str> = None;
    for (k, (text, roma)) in units.iter().enumerate() {
        let shown = text.trim();
        let script = script_of(shown);
        let at = Around { prev, next: units.get(k + 1).map(|(t, _)| script_of(t.trim())), word_start };
        let particle = is_particle(shown, &at);
        let new_word = break_next || particle || prev.is_some_and(|p| starts_word(p, script));
        // The small っ doubles the next sound; some services write it with an apostrophe ("'t").
        let syllables: String = roma.split_whitespace().collect::<String>().replace('\'', "");
        if !syllables.is_empty() {
            if new_word && !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&syllables);
        }
        word_start = (new_word || prev.is_none()).then_some(shown);
        // A space or punctuation in the line, or a particle, ends the word it is in.
        break_next = particle || text.ends_with(char::is_whitespace) || script == Script::Other;
        if script != Script::Other {
            prev = Some(script);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pairs a line's characters with romaji given a syllable at a time, as the services give it.
    fn line(text: &str, romaji: &[&str]) -> String {
        let chars: Vec<String> = text.chars().map(String::from).collect();
        assert_eq!(chars.len(), romaji.len(), "one piece of romaji per character");
        let units: Vec<(&str, &str)> = chars.iter().map(String::as_str).zip(romaji.iter().copied()).collect();
        words(&units)
    }

    #[test]
    fn syllables_become_words_with_particles_apart() {
        let r = line("あなたが好きだと言ってくれた私を", &["a ", "na ", "ta ", "ga ", "su ", "ki ", "da ", "to ", "i ", "t ", "te ", "ku ", "re ", "ta ", "wa ta shi ", "wo "]);
        assert_eq!(r, "anata ga sukida to ittekureta watashi wo");
        assert_eq!(line("君の目に映りたかった", &["ki mi ", "no ", "me ", "ni ", "u tsu ", "ri ", "ta ", "ka ", "t ", "ta "]), "kimi no me ni utsuritakatta");
    }

    #[test]
    fn kanji_with_their_kana_are_one_word() {
        let r = line("命短し恋する乙女だ", &["i no chi ", "mi ji ka ", "shi ", "ko i ", "su ", "ru ", "o to ", "me ", "da "]);
        assert_eq!(r, "inochimijikashi koisuru otomeda");
    }

    #[test]
    fn katakana_words_and_demonstratives_keep_together() {
        assert_eq!(line("ウワサのあの子にぞっこん", &["u ", "wa ", "sa ", "no ", "a ", "no ", "ko ", "ni ", "zo ", "k ", "ko ", "n "]), "uwasa no ano ko ni zokkon");
        // The small っ written "'t" doubles the next sound without an apostrophe.
        assert_eq!(line("分かった", &["wa ", "ka ", "'t ", "ta "]), "wakatta");
    }

    #[test]
    fn spaces_in_the_line_part_words_and_hiragana_words_hold() {
        assert_eq!(words(&[("有", "u "), ("無 ", "mu "), ("う", "u "), ("ら", "ra ")]), "umu ura");
        assert_eq!(line("こともの", &["ko ", "to ", "mo ", "no "]), "kotomono");
        assert_eq!(words(&[]), "");
    }
}
