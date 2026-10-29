//! Release notes of the running version, from changelogs/v<version>.md (picked by build.rs),
//! shown on the "What's new" page.

pub const TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/changelog.md"));

#[derive(Debug, PartialEq)]
pub enum Block {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// Reads the small Markdown subset the release script writes. The top `#` title is skipped,
/// since the page has its own.
pub fn parse(md: &str) -> Vec<Block> {
    md.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !(l.starts_with("# ") || *l == "#"))
        .map(|l| {
            if let Some(h) = l.strip_prefix("### ").or_else(|| l.strip_prefix("## ")) {
                Block::Heading(plain(h))
            } else if let Some(b) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
                Block::Bullet(plain(b))
            } else {
                Block::Text(plain(l))
            }
        })
        .collect()
}

/// Drops inline Markdown marks the page cannot style.
fn plain(s: &str) -> String {
    s.replace("**", "").replace('`', "").trim().to_string()
}

/// Greedy word wrap; `width_of` measures a candidate line.
pub fn wrap(text: &str, max_w: f32, width_of: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && width_of(&candidate) > max_w {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_notes() {
        let md = "# SuiNotch 1.2.0\n\nReleased 2026-11-02\n\n### Added\n- **Updates** from `GitHub`\n* Shelf sizes\n\n## Fixed\n- Delete button\n";
        assert_eq!(
            parse(md),
            vec![
                Block::Text("Released 2026-11-02".into()),
                Block::Heading("Added".into()),
                Block::Bullet("Updates from GitHub".into()),
                Block::Bullet("Shelf sizes".into()),
                Block::Heading("Fixed".into()),
                Block::Bullet("Delete button".into()),
            ]
        );
    }

    #[test]
    fn wraps_on_word_boundaries() {
        let w = |s: &str| s.len() as f32;
        assert_eq!(wrap("one two three four", 9.0, w), vec!["one two", "three", "four"]);
        assert_eq!(wrap("averyveryverylongword x", 5.0, w), vec!["averyveryverylongword", "x"]);
        assert!(wrap("   ", 5.0, w).is_empty());
    }

    #[test]
    fn embedded_notes_are_not_empty() {
        assert!(!parse(TEXT).is_empty());
    }
}
