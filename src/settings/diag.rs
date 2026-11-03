//! The Diagnostics page: a snapshot of what SuiNotch sees (monitors, full-screen state, players,
//! lyrics) and the end of the log, for troubleshooting and for "Copy debug info".
//! The snapshot is taken when the page opens or is refreshed, never while painting, because
//! some of it reads files and walks the window list.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::{config, lyrics_cache, media, quiet, sys};

const LOG_LINES: usize = 10;
const LOG_TAIL_BYTES: u64 = 8192;

#[derive(Default, Clone)]
pub(super) struct Diag {
    pub groups: Vec<(&'static str, Vec<(String, String)>)>,
    pub log: Vec<String>,
}

pub fn log_path() -> PathBuf {
    config::data_dir().join("suinotch.log")
}

fn row(key: &str, value: impl Into<String>) -> (String, String) {
    (key.to_string(), value.into())
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "Yes"
    } else {
        "No"
    }
}

fn uptime(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", s / 60),
        s => format!("{} h {} min", s / 3600, (s % 3600) / 60),
    }
}

pub(super) fn snapshot(app: &App) -> Diag {
    let cfg = &app.cfg;
    let app_group = vec![
        row("Version", env!("CARGO_PKG_VERSION")),
        row("Build", if cfg!(debug_assertions) { "Debug" } else { "Release" }),
        row("Running for", uptime(app.started.elapsed().as_secs())),
        row("Data folder", config::data_dir().display().to_string()),
    ];

    let monitors = sys::monitors();
    let mut display = vec![row(
        "Placement",
        if cfg.display_follow {
            "Follows the active window".to_string()
        } else if cfg.display_follow_mouse {
            "Follows the mouse".to_string()
        } else {
            match &cfg.display {
                Some(d) if monitors.iter().any(|m| &m.device == d) => format!("Fixed on {d}"),
                _ => "Main display".to_string(),
            }
        },
    )];
    for (i, m) in monitors.iter().enumerate() {
        let here = if m.device == app.notch.monitor.device { "  ·  notch is here" } else { "" };
        display.push(row(
            &format!("Display {}{}", i + 1, if m.primary { " (primary)" } else { "" }),
            format!("{} x {} at {:.0}%{here}", m.rect.right - m.rect.left, m.rect.bottom - m.rect.top, m.scale * 100.0),
        ));
    }
    display.push(row("Hide in full screen", if cfg.hide_fullscreen { "On" } else { "Off" }));
    display.push(row(
        "Full screen now",
        match sys::fullscreen_blocker(&app.notch.monitor.rect) {
            Some(what) => what,
            None => "Nothing fills the notch's screen".to_string(),
        },
    ));
    display.push(row("Notch hidden", yes_no(app.notch.hidden)));
    display.push(row("Do Not Disturb or busy", yes_no(quiet::active())));

    let music = vec![
        row(
            "Player",
            match &app.media {
                Some(m) => format!("{} ({})", media::source_name(&m.info.app_id), if m.info.playing { "playing" } else { "paused" }),
                None => "Nothing playing".to_string(),
            },
        ),
        row("Players open", app.media.as_ref().map_or(0, |m| m.info.sources).to_string()),
        row(
            "Lyrics",
            match (app.lyrics_source, &app.media) {
                (Some((source, synced)), _) => format!("{} · {}", source.name(), if synced { "synced" } else { "unsynced" }),
                (None, Some(_)) => "None found yet".to_string(),
                (None, None) => "-".to_string(),
            },
        ),
        row("Saved lyrics", lyrics_cache::count().to_string()),
        row("Song check", song_check(app)),
        row("Recently played", app.recent.items.len().to_string()),
        row("Songs learned for lookups ahead", app.history.len().to_string()),
    ];
    let lookups: Vec<(String, String)> = crate::lyrics_log::recent()
        .into_iter()
        .map(|l| {
            let when = crate::store::local(l.at).format("%H:%M:%S");
            let waited = if l.waited_ms > 0 { format!(", waited {} ms", l.waited_ms) } else { String::new() };
            row(&format!("{when} {}", l.kind), format!("{}: {} ({} requests, {} ms{waited})", l.title, l.result, l.requests, l.ms))
        })
        .collect();

    let calendar = vec![row(
        "Calendar",
        if cfg.calendar_url.is_empty() {
            "No link added".to_string()
        } else if !app.calendar_error.is_empty() {
            format!("Problem: {}", app.calendar_error)
        } else {
            format!("{} events", app.calendar.len())
        },
    )];

    Diag {
        groups: vec![("SuiNotch", app_group), ("Display and full screen", display), ("Music", music), ("Lyric lookups", lookups), ("Calendar", calendar)],
        log: read_tail(&log_path(), LOG_TAIL_BYTES, LOG_LINES),
    }
}

/// What the song check makes of the track playing: the lookup decision, its score and the signs.
fn song_check(app: &App) -> String {
    let Some(m) = &app.media else { return "-".to_string() };
    let gate = match crate::lyric_filter::gate(&app.cfg, &m.info) {
        crate::lyric_filter::Gate::Fetch => "Automatic".to_string(),
        crate::lyric_filter::Gate::Verify => "Checked with a music catalog first".to_string(),
        crate::lyric_filter::Gate::Skip(Some(why)) => format!("Skipped: {why}"),
        crate::lyric_filter::Gate::Skip(None) => "Manual for this player".to_string(),
    };
    let a = crate::lyric_filter::assessment(&m.info);
    let score = if a.score == i32::MIN { "decided".to_string() } else { format!("score {}", a.score) };
    format!("{gate} ({score}: {})", if a.signs.is_empty() { "no signs".to_string() } else { a.signs.join(", ") })
}

/// The last `lines` non-empty lines of a file, reading only its final `bytes`.
fn read_tail(path: &Path, bytes: u64, lines: usize) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(path) else { return Vec::new() };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(bytes);
    let mut buf = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut all: Vec<&str> = text.lines().collect();
    // Starting mid-file, the first line is usually cut off.
    if start > 0 && !all.is_empty() {
        all.remove(0);
    }
    let all: Vec<String> = all.into_iter().map(str::trim_end).filter(|l| !l.is_empty()).map(str::to_string).collect();
    all[all.len().saturating_sub(lines)..].to_vec()
}

/// Plain text of the snapshot, for pasting into a bug report.
pub(super) fn to_text(d: &Diag) -> String {
    let mut out = String::from("SuiNotch debug info\n");
    for (name, rows) in &d.groups {
        out.push_str(&format!("\n[{name}]\n"));
        for (k, v) in rows {
            out.push_str(&format!("{k}: {v}\n"));
        }
    }
    out.push_str(&format!("\n[Log, last {} lines]\n", d.log.len()));
    for line in &d.log {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_lists_groups_rows_and_log() {
        let d = Diag { groups: vec![("Music", vec![row("Player", "Spotify (playing)")])], log: vec!["12:00 hello".into()] };
        let t = to_text(&d);
        assert!(t.contains("[Music]\nPlayer: Spotify (playing)\n"));
        assert!(t.contains("[Log, last 1 lines]\n12:00 hello\n"));
    }

    #[test]
    fn tail_keeps_the_last_lines_and_drops_a_cut_first_line() {
        let path = std::env::temp_dir().join(format!("suinotch-diag-{}.log", std::process::id()));
        std::fs::write(&path, "one\ntwo\nthree\n\nfour\n").unwrap();
        assert_eq!(read_tail(&path, 1000, 2), vec!["three", "four"]);
        // Reading only the last 9 bytes starts inside "three", which is dropped as a partial line.
        assert_eq!(read_tail(&path, 9, 5), vec!["four"]);
        let _ = std::fs::remove_file(&path);
        assert!(read_tail(&path, 1000, 5).is_empty());
    }

    #[test]
    fn uptime_reads_naturally() {
        assert_eq!(uptime(30), "30 s");
        assert_eq!(uptime(600), "10 min");
        assert_eq!(uptime(7500), "2 h 5 min");
    }
}
