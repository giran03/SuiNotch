#![windows_subsystem = "windows"]

mod anim;
mod app;
mod changelog;
mod config;
mod dnd;
mod events;
mod gfx;
mod lrc;
mod lyrics;
mod mascot;
mod media;
mod notch;
mod pages;
mod pomodoro;
mod profile;
mod settings;
mod sound;
mod store;
mod sys;
mod ticker;
mod tray;
mod update;
mod views;

use std::rc::Rc;
use std::time::Instant;

use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};

/// Appends a line to %APPDATA%\SuiNotch\suinotch.log (there is no console).
pub fn log(msg: &str) {
    use std::io::Write;
    let dir = config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("suinotch.log")) {
        let _ = writeln!(f, "{} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

/// Writes panics to the log before the process ends; release builds abort, so nothing else would.
fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        log(&format!("panic in {}: {info}", env!("CARGO_PKG_VERSION")));
    }));
}

fn main() {
    log_panics();
    if !sys::single_instance() {
        // Already running: ask that copy to show its settings instead.
        sys::signal_running_instance();
        return;
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = OleInitialize(None);
    }
    if let Err(e) = run() {
        log(&format!("fatal: {e}"));
    }
}

fn run() -> windows::core::Result<()> {
    let first_run = !config::data_dir().join("config.json").exists();
    let mut cfg: config::Config = config::load_json::<config::Config>("config.json").sanitized();
    let gfx = Rc::new(gfx::Gfx::new()?);
    let events = events::init();
    let notch = notch::Notch::create(&cfg)?;
    // Fresh installs and updates open this version's release notes once.
    let show_notes = cfg.notes_seen != env!("CARGO_PKG_VERSION");
    if show_notes {
        cfg.notes_seen = env!("CARGO_PKG_VERSION").to_string();
    }
    if first_run || show_notes {
        config::save_json("config.json", &cfg);
    }
    sys::set_startup(cfg.start_with_windows);
    let durations = app::durations(&cfg);
    let today = chrono::Local::now().date_naive();
    let app = app::App {
        gfx,
        todos: config::load_json("todos.json"),
        water: config::load_json("water.json"),
        shelf: config::load_json("shelf.json"),
        audio: sys::Audio::new().map_err(|e| log(&format!("volume control unavailable: {e}"))).ok(),
        media: None,
        artwork: None,
        art_color: None,
        lyrics: Vec::new(),
        lyrics_key: 0,
        lyrics_requested: 0,
        pomo: pomodoro::Pomodoro::new(&durations),
        notch,
        settings: None,
        events,
        started: Instant::now(),
        network: None,
        language: String::new(),
        caps: false,
        last_battery: None,
        last_day: today,
        update: update::Status::Idle,
        cfg,
    };
    let hwnd = app.notch.hwnd;
    app::install(app);
    app::with(|app| {
        if app.shelf.prune_missing() {
            app.save_shelf();
        }
        tray::add(app);
        notch::start(app);
    });
    dnd::register(hwnd);
    media::start();
    sys::start_network_watch();
    sys::start_bluetooth_watch();
    sys::install_hooks();
    sys::listen_for_second_launch();
    if let Some(mode) = app::with(|app| app.cfg.update_mode) {
        update::start(mode);
    }
    if show_notes || first_run {
        let page = show_notes.then_some(settings::CHANGELOG_PAGE);
        app::defer(app::Deferred::OpenSettings(page));
        app::with(|_| {});
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    tray::remove(hwnd);
    Ok(())
}
