//! Version markers in song titles. A live, remix, cover, instrumental or karaoke recording has other
//! vocals, and a sped-up or slowed one another clock, so lyrics found for one are wrong for the other:
//! a marker has to be on both titles or on neither.

/// (group, variants). Latin variants match whole words; any variant with a non-ASCII letter matches as
/// text, since those scripts put no spaces between words. Variants are lowercase.
const GROUPS: &[(&str, &[&str])] = &[
    ("live", &["live", "ライブ", "ライヴ", "现场", "現場"]),
    ("remix", &["remix", "remixed", "rmx", "リミックス"]),
    ("cover", &["cover", "covered", "翻唱", "翻自", "カバー"]),
    ("instrumental", &["instrumental", "inst", "伴奏", "纯音乐", "純音樂", "インスト", "off vocal", "off-vocal", "オフボーカル", "karaoke", "カラオケ", "backing track"]),
    ("acoustic", &["acoustic", "unplugged", "アコースティック"]),
    ("sped up", &["sped up", "speed up", "加速版"]),
    ("slowed", &["slowed", "降速", "慢速版"]),
    ("nightcore", &["nightcore"]),
];

/// The marker group that is on exactly one of the two titles, if there is one.
pub fn mismatch(a: &str, b: &str) -> Option<&'static str> {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    GROUPS.iter().find(|(_, variants)| has(&a, variants) != has(&b, variants)).map(|(group, _)| *group)
}

fn has(title: &str, variants: &[&str]) -> bool {
    variants.iter().any(|v| contains(title, v))
}

fn contains(title: &str, needle: &str) -> bool {
    if !needle.is_ascii() {
        return title.contains(needle);
    }
    title.match_indices(needle).any(|(i, _)| {
        let before = title[..i].chars().next_back();
        let after = title[i + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_on_one_title_only_is_a_mismatch() {
        assert_eq!(mismatch("Blinding Lights", "Blinding Lights (Live)"), Some("live"));
        assert_eq!(mismatch("Song - Remix", "Song"), Some("remix"));
        assert_eq!(mismatch("Song", "Song (Instrumental)"), Some("instrumental"));
        assert_eq!(mismatch("曲", "曲 (伴奏)"), Some("instrumental"));
        assert_eq!(mismatch("Song (Sped Up)", "Song"), Some("sped up"));
    }

    #[test]
    fn the_same_marker_on_both_or_none_is_fine() {
        assert_eq!(mismatch("Song (Live)", "Song - Live at Wembley"), None);
        assert_eq!(mismatch("Blinding Lights", "Blinding Lights"), None);
        assert_eq!(mismatch("Song (Remastered 2011)", "Song"), None);
    }

    #[test]
    fn markers_match_whole_words_only() {
        assert_eq!(mismatch("Alive", "Alive"), None);
        assert_eq!(mismatch("Delivered", "Delivered"), None);
        assert_eq!(mismatch("Alive", "Alive (Live)"), Some("live"));
        assert_eq!(mismatch("Live and Let Die", "Live and Let Die"), None);
        assert_eq!(mismatch("Discovery", "Discovery"), None);
    }
}
