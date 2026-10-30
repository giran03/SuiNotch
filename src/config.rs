//! User settings. Every field has a default so older or partial files still load.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SIZE_STEPS: [f32; 5] = [0.0, 1.0, 1.15, 1.3, 1.5];
pub const SIZE_LABELS: [&str; 5] = ["Auto", "100%", "115%", "130%", "150%"];

pub const ACCENTS: [u32; 8] = [
    0x3BA7FF, // Sui blue
    0x2DD4BF, // lagoon
    0x34C759, // leaf
    0xFFB020, // amber
    0xFF6B5A, // coral
    0xF0629C, // blossom
    0xA78BFA, // lilac
    0xE8E8ED, // pearl
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    // General
    pub start_with_windows: bool,
    pub size: usize,
    pub minimal: bool,
    pub mascot: bool,
    pub sounds: bool,
    pub hide_fullscreen: bool,
    /// Hold reminders while Windows is in Do Not Disturb, presenting or running a full-screen game.
    pub respect_quiet: bool,
    /// Sui cheers for finished sessions and goals, and dozes through long breaks.
    pub sui_reactions: bool,
    /// Old Stats switch, read once to carry the choice into `hidden_pages`; never written.
    #[serde(skip_serializing)]
    pub stats_page: bool,
    /// Notch pages switched off in Settings, by `Page::key`. Their features keep working.
    pub hidden_pages: Vec<String>,
    // Display
    pub display: Option<String>,
    /// Move the notch to whichever monitor holds the active window.
    pub display_follow: bool,
    pub expand_on_hover: bool,
    pub hover_delay: usize,
    // Appearance
    pub accent: usize,
    /// Use the accent colour chosen in Windows instead of one of ours.
    pub accent_follow_system: bool,
    pub roundness: usize,
    /// Index into `NOTCH_STYLE_LABELS`: docked, floating or flat.
    pub notch_style: usize,
    /// Space in pixels between the screen edge and a floating notch.
    pub notch_gap: u32,
    pub animation: usize,
    pub shadow: bool,
    pub artwork_glow: bool,
    pub clock_format: usize,
    pub glass: bool,
    // Music and lyrics
    pub lyrics_auto: bool,
    pub lyrics_pill: bool,
    pub equalizer: bool,
    pub show_source: bool,
    /// AppUserModelID of the player last picked with the switch; picked again when it reopens.
    pub preferred_player: String,
    // To-do
    pub todo_snooze: usize,
    pub todo_show_done: bool,
    pub todo_clear_done_daily: bool,
    // Calendar
    /// Old Calendar switch, read once to carry the choice into `hidden_pages`; never written.
    #[serde(skip_serializing)]
    pub calendar_page: bool,
    pub week_starts_monday: bool,
    pub calendar_dots: bool,
    /// Secret iCal address of an Outlook, Google or other calendar; empty for none.
    pub calendar_url: String,
    pub calendar_reminders: bool,
    // Pomodoro
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    pub sessions_before_long: u32,
    pub auto_start_breaks: bool,
    pub auto_start_focus: bool,
    /// Focus tab mode: 0 Pomodoro, 1 timer, 2 stopwatch.
    pub focus_mode: usize,
    pub timer_min: u32,
    // Water
    pub water_enabled: bool,
    pub glass_ml: u32,
    pub goal_ml: u32,
    pub water_start_hour: u32,
    pub water_end_hour: u32,
    pub water_interval_min: u32,
    // Shelf
    pub shelf_open_on_drag: bool,
    pub shelf_remove_after_drag: bool,
    pub shelf_show_sizes: bool,
    pub clipboard_history: bool,
    pub clipboard_limit: u32,
    /// Index into clips::KEEP_MIN: how long SuiNotch may be off and keep its clipboard history.
    pub clipboard_keep: usize,
    // HUD and popups
    pub hud_volume: bool,
    pub hud_brightness: bool,
    pub hud_caps: bool,
    pub hud_language: bool,
    pub hud_battery: bool,
    pub hud_wifi: bool,
    pub hud_headphones: bool,
    pub hud_clipboard: bool,
    pub hud_duration: usize,
    pub hud_style: usize,
    // About
    pub update_mode: usize,
    /// Last version whose "What's new" page was shown; a new install or update shows it once.
    pub notes_seen: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            start_with_windows: true,
            size: 0,
            minimal: false,
            mascot: true,
            sounds: true,
            hide_fullscreen: true,
            respect_quiet: true,
            sui_reactions: true,
            stats_page: true,
            hidden_pages: Vec::new(),
            display: None,
            display_follow: false,
            expand_on_hover: true,
            hover_delay: 1,
            accent: 0,
            accent_follow_system: false,
            roundness: 1,
            notch_style: 0,
            notch_gap: DEFAULT_NOTCH_GAP,
            animation: 1,
            shadow: true,
            artwork_glow: true,
            clock_format: 0,
            glass: false,
            lyrics_auto: false,
            lyrics_pill: true,
            equalizer: true,
            show_source: true,
            preferred_player: String::new(),
            todo_snooze: 1,
            todo_show_done: true,
            todo_clear_done_daily: false,
            calendar_page: true,
            week_starts_monday: true,
            calendar_dots: true,
            calendar_url: String::new(),
            calendar_reminders: true,
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            sessions_before_long: 4,
            auto_start_breaks: false,
            auto_start_focus: false,
            focus_mode: 0,
            timer_min: 10,
            water_enabled: true,
            glass_ml: 250,
            goal_ml: 2000,
            water_start_hour: 9,
            water_end_hour: 21,
            water_interval_min: 60,
            shelf_open_on_drag: true,
            shelf_remove_after_drag: false,
            shelf_show_sizes: true,
            clipboard_history: true,
            clipboard_limit: crate::clips::DEFAULT_LIMIT,
            clipboard_keep: 1,
            hud_volume: true,
            hud_brightness: true,
            hud_caps: true,
            hud_language: true,
            hud_battery: true,
            hud_wifi: true,
            hud_headphones: true,
            hud_clipboard: true,
            hud_duration: 1,
            hud_style: 0,
            update_mode: crate::update::MODE_AUTO,
            notes_seen: String::new(),
        }
    }
}

pub const NOTCH_STYLE_LABELS: [&str; 3] = ["Docked", "Floating", "Flat"];
pub const MIN_NOTCH_GAP: u32 = 4;
pub const MAX_NOTCH_GAP: u32 = 24;
pub const DEFAULT_NOTCH_GAP: u32 = 8;

/// How the notch meets the top edge of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotchStyle {
    /// Concave shoulders blend into the screen edge.
    Docked,
    /// A detached capsule with a gap above it.
    Floating,
    /// Flush with the edge, with square top corners and no shoulders.
    Flat,
}

pub const HOVER_DELAYS_MS: [u32; 3] = [0, 150, 400];
pub const HOVER_DELAY_LABELS: [&str; 3] = ["Instant", "Short", "Relaxed"];
pub const ROUNDNESS: [f32; 3] = [0.6, 1.0, 1.45];
pub const ROUNDNESS_LABELS: [&str; 3] = ["Soft", "Round", "Pebble"];
/// (response seconds, damping ratio) per animation style.
pub const ANIMATION: [(f32, f32); 3] = [(0.62, 0.86), (0.46, 0.74), (0.32, 0.8)];
pub const ANIMATION_LABELS: [&str; 3] = ["Relaxed", "Fluid", "Snappy"];
pub const CLOCK_LABELS: [&str; 3] = ["System", "12-hour", "24-hour"];
pub const SNOOZE_MIN: [u32; 4] = [5, 10, 15, 30];
pub const SNOOZE_LABELS: [&str; 4] = ["5 min", "10 min", "15 min", "30 min"];
pub const HUD_DURATION_MS: [u32; 3] = [1200, 2000, 3500];
pub const HUD_DURATION_LABELS: [&str; 3] = ["Short", "Normal", "Long"];
pub const HUD_STYLE_LABELS: [&str; 2] = ["Compact", "Wide"];

impl Config {
    pub fn hover_delay_ms(&self) -> u32 {
        HOVER_DELAYS_MS[self.hover_delay.min(2)]
    }
    pub fn accent_rgb(&self) -> u32 {
        let own = ACCENTS[self.accent.min(ACCENTS.len() - 1)];
        if self.accent_follow_system {
            crate::sys::system_accent().unwrap_or(own)
        } else {
            own
        }
    }
    pub fn roundness(&self) -> f32 {
        ROUNDNESS[self.roundness.min(2)]
    }
    pub fn style(&self) -> NotchStyle {
        match self.notch_style {
            1 => NotchStyle::Floating,
            2 => NotchStyle::Flat,
            _ => NotchStyle::Docked,
        }
    }
    /// Pixels between the screen edge and the notch; zero unless it floats.
    pub fn gap_px(&self) -> f32 {
        if self.style() == NotchStyle::Floating {
            self.notch_gap.clamp(MIN_NOTCH_GAP, MAX_NOTCH_GAP) as f32
        } else {
            0.0
        }
    }
    pub fn animation(&self) -> (f32, f32) {
        ANIMATION[self.animation.min(2)]
    }
    pub fn snooze_min(&self) -> u32 {
        SNOOZE_MIN[self.todo_snooze.min(3)]
    }
    /// Minutes SuiNotch may be off and still restore its clipboard history (0 = never).
    pub fn clipboard_keep_min(&self) -> u32 {
        if self.clipboard_history {
            crate::clips::KEEP_MIN[self.clipboard_keep.min(crate::clips::KEEP_MIN.len() - 1)]
        } else {
            0
        }
    }
    pub fn hud_ms(&self) -> u32 {
        HUD_DURATION_MS[self.hud_duration.min(2)]
    }

    /// Scale for the chosen size step; "Auto" grows on wide monitors.
    pub fn size_scale(&self, monitor_width_dip: f32) -> f32 {
        match self.size.min(4) {
            0 if monitor_width_dip >= 3000.0 => 1.3,
            0 if monitor_width_dip >= 2200.0 => 1.15,
            0 => 1.0,
            i => SIZE_STEPS[i],
        }
    }

    /// Clamps values that came from disk into the ranges the UI offers.
    pub fn sanitized(mut self) -> Self {
        self.focus_min = self.focus_min.clamp(5, 120);
        self.short_break_min = self.short_break_min.clamp(1, 30);
        self.long_break_min = self.long_break_min.clamp(5, 60);
        self.sessions_before_long = self.sessions_before_long.clamp(2, 8);
        self.focus_mode = self.focus_mode.min(2);
        self.notch_style = self.notch_style.min(NOTCH_STYLE_LABELS.len() - 1);
        self.notch_gap = self.notch_gap.clamp(MIN_NOTCH_GAP, MAX_NOTCH_GAP);
        self.timer_min = self.timer_min.clamp(1, crate::timers::MAX_MIN);
        self.clipboard_limit = self.clipboard_limit.clamp(crate::clips::MIN_LIMIT, crate::clips::MAX_LIMIT);
        self.clipboard_keep = self.clipboard_keep.min(crate::clips::KEEP_MIN.len() - 1);
        self.glass_ml = self.glass_ml.clamp(50, 2000);
        self.goal_ml = self.goal_ml.clamp(500, 6000);
        self.water_start_hour = self.water_start_hour.min(23);
        self.water_end_hour = self.water_end_hour.clamp(self.water_start_hour + 1, 24);
        self.water_interval_min = self.water_interval_min.clamp(15, 240);
        self.update_mode = self.update_mode.min(crate::update::MODE_LABELS.len() - 1);
        for (shown, key) in [(self.calendar_page, "calendar"), (self.stats_page, "stats")] {
            if !shown && !self.hidden_pages.iter().any(|k| k == key) {
                self.hidden_pages.push(key.to_string());
            }
        }
        self.calendar_page = true;
        self.stats_page = true;
        self.hidden_pages.sort();
        self.hidden_pages.dedup();
        self
    }

    /// Whether the notch page with this key is switched on.
    pub fn page_shown(&self, key: &str) -> bool {
        !self.hidden_pages.iter().any(|k| k == key)
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("SuiNotch")
}

pub fn load_json<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            crate::log(&format!("{name} is unreadable ({e}); keeping a backup and starting fresh"));
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
            T::default()
        }),
        Err(_) => T::default(),
    }
}

/// Writes atomically through a temp file so a crash never leaves half a file behind.
pub fn save_json<T: Serialize>(name: &str, value: &T) {
    let dir = data_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::log(&format!("cannot create {}: {e}", dir.display()));
        return;
    }
    let text = match serde_json::to_string_pretty(value) {
        Ok(t) => t,
        Err(e) => return crate::log(&format!("cannot encode {name}: {e}")),
    };
    let tmp = dir.join(format!("{name}.tmp"));
    let target = dir.join(name);
    let mut result = std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &target));
    if result.is_err() {
        // Some file-system filters refuse renames; fall back to a direct write.
        let _ = std::fs::remove_file(&tmp);
        result = std::fs::write(&target, &text);
    }
    if let Err(e) = result {
        crate::log(&format!("cannot save {name}: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_fills_defaults() {
        let c: Config = serde_json::from_str(r#"{"mascot": false, "focus_min": 50}"#).unwrap();
        assert!(!c.mascot);
        assert_eq!(c.focus_min, 50);
        assert_eq!(c.short_break_min, 5);
        assert!(c.hud_volume);
    }

    #[test]
    fn old_page_switches_carry_over_and_are_not_written_back() {
        let c: Config = serde_json::from_str::<Config>(r#"{"calendar_page": false, "hidden_pages": ["water"]}"#).unwrap().sanitized();
        assert!(!c.page_shown("calendar"));
        assert!(!c.page_shown("water"));
        assert!(c.page_shown("stats"));
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("calendar_page"));
        let again: Config = serde_json::from_str::<Config>(&json).unwrap().sanitized();
        assert_eq!(again.hidden_pages, vec!["calendar".to_string(), "water".to_string()]);
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        let c = Config { focus_min: 1, goal_ml: 99_999, water_start_hour: 22, water_end_hour: 3, ..Config::default() }
            .sanitized();
        assert_eq!(c.focus_min, 5);
        assert_eq!(c.goal_ml, 6000);
        assert_eq!(c.water_end_hour, 23);
    }

    #[test]
    fn old_settings_keep_the_docked_notch() {
        let c: Config = serde_json::from_str(r#"{"roundness": 2}"#).unwrap();
        assert_eq!(c.style(), NotchStyle::Docked);
        assert_eq!(c.gap_px(), 0.0);
    }

    #[test]
    fn only_the_floating_style_leaves_a_gap() {
        let floating = Config { notch_style: 1, notch_gap: 12, ..Config::default() };
        assert_eq!(floating.style(), NotchStyle::Floating);
        assert_eq!(floating.gap_px(), 12.0);
        let flat = Config { notch_style: 2, notch_gap: 12, ..Config::default() };
        assert_eq!(flat.style(), NotchStyle::Flat);
        assert_eq!(flat.gap_px(), 0.0);
    }

    #[test]
    fn sanitize_clamps_notch_style_and_gap() {
        let c = Config { notch_style: 9, notch_gap: 500, ..Config::default() }.sanitized();
        assert_eq!(c.notch_style, 2);
        assert_eq!(c.notch_gap, MAX_NOTCH_GAP);
        let c = Config { notch_gap: 0, ..Config::default() }.sanitized();
        assert_eq!(c.notch_gap, MIN_NOTCH_GAP);
    }

    #[test]
    fn auto_size_grows_on_wide_monitors() {
        let c = Config::default();
        assert_eq!(c.size_scale(1920.0), 1.0);
        assert_eq!(c.size_scale(2560.0), 1.15);
        assert_eq!(c.size_scale(3840.0), 1.3);
        let fixed = Config { size: 4, ..Config::default() };
        assert_eq!(fixed.size_scale(3840.0), 1.5);
    }
}
