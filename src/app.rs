//! Global application state, owned by the UI thread.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use windows::Win32::Graphics::Imaging::IWICBitmapSource;

use crate::config::{self, Config};
use crate::events::{Event, MediaInfo};
use crate::gfx::{Gfx, C};
use crate::ical::CalEvent;
use crate::lrc::Line;
use crate::notch::Notch;
use crate::pomodoro::{Durations, Pomodoro};
use crate::settings::SettingsWindow;
use crate::stats::Stats;
use crate::store::{self, Shelf, Todos, Water};
use crate::sys::Audio;

pub struct Media {
    pub info: MediaInfo,
    /// When `info.position_ms` was sampled, to extrapolate playback position.
    pub at: Instant,
}

impl Media {
    pub fn position_ms(&self) -> u64 {
        let mut p = self.info.position_ms;
        if self.info.playing {
            p += self.at.elapsed().as_millis() as u64;
        }
        if self.info.duration_ms > 0 {
            p.min(self.info.duration_ms)
        } else {
            p
        }
    }
}

pub struct App {
    pub gfx: Rc<Gfx>,
    pub cfg: Config,
    pub todos: Todos,
    pub water: Water,
    pub shelf: Shelf,
    pub audio: Option<Audio>,
    pub media: Option<Media>,
    pub artwork: Option<(u64, IWICBitmapSource)>,
    pub art_color: Option<C>,
    pub lyrics: Vec<Line>,
    pub lyrics_key: u64,
    /// Track whose lyrics were last requested, so each song is looked up once.
    pub lyrics_requested: u64,
    /// "Search again" presses for the current song.
    pub lyrics_attempt: usize,
    pub shifts: crate::lyrics::Shifts,
    pub pomo: Pomodoro,
    pub timer: crate::timers::Countdown,
    pub stopwatch: crate::timers::Stopwatch,
    pub stats: Stats,
    pub clips: crate::clips::Clips,
    pub calendar: Vec<CalEvent>,
    /// Last calendar refresh problem, shown in Settings; empty when fine.
    pub calendar_error: String,
    pub calendar_synced: Option<i64>,
    /// Event occurrences already announced.
    pub calendar_alerted: HashSet<u64>,
    pub notch: Notch,
    pub settings: Option<SettingsWindow>,
    pub events: Receiver<Event>,
    pub started: Instant,
    pub network: Option<(bool, String)>,
    pub language: String,
    pub caps: bool,
    pub last_battery: Option<(u8, bool)>,
    pub last_day: chrono::NaiveDate,
    pub update: crate::update::Status,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DEFERRED: RefCell<Vec<Deferred>> = const { RefCell::new(Vec::new()) };
}

/// Work that must run while the app is not borrowed (modal loops, window creation).
pub enum Deferred {
    DragOut(Vec<usize>),
    DragClip(usize),
    OpenSettings(Option<usize>),
    Exit,
}

pub fn install(app: App) {
    APP.with(|a| *a.borrow_mut() = Some(app));
}

/// Runs `f` with the app, or returns None when it is already in use (re-entrant message).
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let r = APP.with(|a| {
        let mut guard = a.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    });
    run_deferred();
    r
}

pub fn defer(d: Deferred) {
    DEFERRED.with(|q| q.borrow_mut().push(d));
}

fn run_deferred() {
    let busy = APP.with(|a| a.try_borrow_mut().is_err());
    if busy {
        return;
    }
    let jobs: Vec<Deferred> = DEFERRED.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for job in jobs {
        match job {
            Deferred::DragOut(indices) => crate::dnd::drag_out(indices),
            Deferred::DragClip(i) => crate::dnd::drag_clip(i),
            Deferred::OpenSettings(page) => crate::settings::open(page),
            Deferred::Exit => unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) },
        }
    }
}

impl App {
    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn durations(&self) -> Durations {
        durations(&self.cfg)
    }

    pub fn save_config(&self) {
        config::save_json("config.json", &self.cfg);
    }
    pub fn save_todos(&self) {
        config::save_json("todos.json", &self.todos);
    }
    pub fn save_water(&self) {
        config::save_json("water.json", &self.water);
    }
    pub fn save_shelf(&self) {
        config::save_json("shelf.json", &self.shelf);
    }
    pub fn save_stats(&self) {
        config::save_json("stats.json", &self.stats);
    }
    pub fn save_shifts(&self) {
        config::save_json("lyrics.json", &self.shifts);
    }

    /// Any Focus tab clock is counting: Pomodoro, timer or stopwatch.
    pub fn focus_running(&self) -> bool {
        self.pomo.is_running() || self.timer.is_running() || self.stopwatch.is_running()
    }

    /// Ticks or unticks a task, counts it in today's stats, and lets Sui cheer a cleared day.
    pub fn toggle_task(&mut self, id: u64, now: i64) {
        let before = self.todos.get(id).map(|t| (t.done, t.done_at));
        self.todos.toggle(id, now);
        let today = store::day_of(now);
        match before {
            Some((false, _)) => self.stats.task_done(today, true),
            Some((true, Some(at))) if store::day_of(at) == today => self.stats.task_done(today, false),
            _ => {}
        }
        self.save_todos();
        self.save_stats();
        let (done, total) = self.todos.today_progress(now);
        if before.is_some_and(|(was_done, _)| !was_done) && total > 0 && done == total {
            crate::notch::cheer(self);
        }
    }

    /// Playback position to match lyric lines against: a little ahead, minus this song's shift.
    pub fn lyric_pos(&self) -> Option<u64> {
        let m = self.media.as_ref()?;
        let shift = self.shifts.get(m.info.track_key());
        Some((m.position_ms() as i64 + 250 - shift).max(0) as u64)
    }

    /// Applies a settings change everywhere and persists it.
    pub fn set_config(&mut self, new: Config) {
        let old = std::mem::replace(&mut self.cfg, new.sanitized());
        if old.start_with_windows != self.cfg.start_with_windows {
            crate::sys::set_startup(self.cfg.start_with_windows);
        }
        crate::update::set_mode(self.cfg.update_mode);
        self.pomo.durations_changed(&durations(&old), &durations(&self.cfg));
        self.save_config();
        crate::notch::config_changed(self, &old);
    }

    pub fn current_line(&self) -> Option<usize> {
        let m = self.media.as_ref()?;
        if self.lyrics_key != m.info.track_key() {
            return None;
        }
        crate::lrc::current(&self.lyrics, self.lyric_pos()?)
    }
}

pub fn durations(c: &Config) -> Durations {
    Durations {
        focus_ms: c.focus_min as u64 * 60_000,
        short_ms: c.short_break_min as u64 * 60_000,
        long_ms: c.long_break_min as u64 * 60_000,
        sessions_before_long: c.sessions_before_long,
    }
}
