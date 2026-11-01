//! Songs played lately, newest first, for the Music page's "Recently played" list.

use serde::{Deserialize, Serialize};

const MAX_KEPT: usize = 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Played {
    pub title: String,
    pub artist: String,
    /// Unix seconds when it started.
    pub at: i64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Recent {
    pub items: Vec<Played>,
}

impl Recent {
    /// Notes that a song started. A song already in the list moves to the top instead of repeating;
    /// returns false when it was already the latest, so nothing needs saving.
    pub fn played(&mut self, title: &str, artist: &str, at: i64) -> bool {
        if title.is_empty() {
            return false;
        }
        if self.items.first().is_some_and(|p| p.title == title && p.artist == artist) {
            return false;
        }
        self.items.retain(|p| !(p.title == title && p.artist == artist));
        self.items.insert(0, Played { title: title.to_string(), artist: artist.to_string(), at });
        self.items.truncate(MAX_KEPT);
        true
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_repeats() {
        let mut r = Recent::default();
        assert!(r.played("A", "x", 1));
        assert!(r.played("B", "x", 2));
        assert!(!r.played("B", "x", 3));
        assert!(r.played("A", "x", 4));
        let titles: Vec<&str> = r.items.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["A", "B"]);
        assert_eq!(r.items[0].at, 4);
    }

    #[test]
    fn keeps_a_bounded_list_and_ignores_untitled() {
        let mut r = Recent::default();
        for i in 0..30 {
            r.played(&format!("song {i}"), "x", i);
        }
        assert_eq!(r.items.len(), MAX_KEPT);
        assert_eq!(r.items[0].title, "song 29");
        assert!(!r.played("", "x", 99));
    }
}
