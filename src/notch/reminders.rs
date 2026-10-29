//! Reminder scheduling and events arriving from background threads.

use super::*;

/// Arms one timer for the earliest of: water reminder, task reminder, focus timer end.
pub fn schedule(app: &mut App) {
    let now = store::now();
    let mut next: Option<i64> = app.todos.next_due(now);
    if app.cfg.water_enabled {
        let c = &app.cfg;
        if let Some(t) = app.water.next_reminder(now, c.water_start_hour, c.water_end_hour, c.water_interval_min, c.goal_ml) {
            next = Some(next.map_or(t, |n| n.min(t)));
        }
    }
    let mut ms: u64 = next.map(|t| ((t - now).max(0) as u64) * 1000).unwrap_or(3_600_000);
    if let Some(end) = app.pomo.ends_at() {
        ms = ms.min(end.saturating_sub(app.now_ms()));
    }
    // Re-check at least hourly so a new day and sleep/resume are picked up.
    let ms = ms.clamp(50, 3_600_000);
    unsafe {
        SetTimer(Some(app.notch.hwnd), T_SCHED, ms as u32, None);
    }
}

pub(super) fn on_schedule(app: &mut App) {
    let now = store::now();
    let today = store::day_of(now);
    if today != app.last_day {
        app.last_day = today;
        app.water.snoozed_until = None;
        if app.cfg.todo_clear_done_daily {
            app.todos.clear_done_before(now);
            app.save_todos();
        }
    }
    if let Some(phase) = app.pomo.poll(app.now_ms()) {
        let d = app.durations();
        let auto = if phase == Phase::Focus { app.cfg.auto_start_breaks } else { app.cfg.auto_start_focus };
        let now_ms = app.now_ms();
        app.pomo.advance(now_ms, &d, auto);
        show_alert(app, AlertKind::Pomodoro(phase));
    }
    for id in app.todos.due_alerts(now) {
        if let Some(t) = app.todos.get_mut(id) {
            t.alerted = true;
        }
        app.save_todos();
        if app.notch.alert.is_none() {
            show_alert(app, AlertKind::Todo(id));
        }
    }
    if app.cfg.water_enabled && app.notch.alert.is_none() {
        let c = &app.cfg;
        if let Some(t) = app.water.next_reminder(now, c.water_start_hour, c.water_end_hour, c.water_interval_min, c.goal_ml) {
            if t <= now {
                app.water.snoozed_until = Some(now + c.water_interval_min as i64 * 60);
                app.save_water();
                show_alert(app, AlertKind::Water);
            }
        }
    }
    schedule(app);
    request_frame(app);
}

// ------------------------------------------------------------------ events from background threads

pub fn drain_events(app: &mut App) {
    let mut redraw = false;
    while let Ok(ev) = app.events.try_recv() {
        crate::profile::count("event");
        redraw |= on_event(app, ev);
    }
    if redraw {
        request_frame(app);
    }
}

/// Applies one background event; returns true when the notch looks different afterwards.
pub(super) fn on_event(app: &mut App, ev: Event) -> bool {
    match ev {
        Event::Media(info) => {
            let Some(info) = info else {
                app.media = None;
                return true;
            };
            // One lookup per track, even if the player briefly reports nothing between songs.
            let key = info.track_key();
            if key != app.lyrics_requested {
                app.lyrics_requested = key;
                app.lyrics.clear();
                app.lyrics_key = 0;
                crate::lyrics::fetch(info.clone());
            }
            app.media = Some(app::Media { info, at: Instant::now() });
            true
        }
        Event::Artwork { key, art } => {
            app.art_color = art.as_ref().and_then(|a| a.color);
            app.artwork = art.and_then(|a| app.gfx.bitmap_from_pixels(&a)).map(|s| (key, s));
            app.notch.art = None;
            true
        }
        Event::Lyrics { key, lines } => {
            if app.media.as_ref().is_some_and(|m| m.info.track_key() == key) {
                app.lyrics = lines;
                app.lyrics_key = key;
            }
            true
        }
        Event::Volume { level, muted } => {
            if app.cfg.hud_volume {
                show_hud(app, Hud::Volume { level, muted });
            }
            false
        }
        Event::AudioDeviceChanged => {
            if let Some(a) = app.audio.as_mut() {
                a.rebind();
            }
            false
        }
        Event::Network { connected, name } => {
            let state = (connected, name.clone());
            let previous = app.network.replace(state.clone());
            if previous.is_some_and(|p| p != state) && app.cfg.hud_wifi {
                show_hud(app, Hud::Wifi { connected, name });
            }
            false
        }
        Event::Bluetooth { name, connected } => {
            if app.cfg.hud_headphones {
                show_hud(app, Hud::Headphones { name, connected });
            }
            false
        }
        Event::Update(status) => {
            use crate::update::{Status, MODE_AUTO, MODE_OFF};
            if let Status::Available(r) = &status {
                let known = matches!(&app.update, Status::Available(old) if old.version == r.version);
                if !known && app.cfg.update_mode != MODE_OFF {
                    let text = if app.cfg.update_mode == MODE_AUTO {
                        format!("SuiNotch {} installs when you're away", r.version)
                    } else {
                        format!("SuiNotch {} is available", r.version)
                    };
                    notice(app, &text);
                }
            }
            app.update = status;
            crate::settings::refresh(app);
            false
        }
    }
}

pub(super) fn on_power(app: &mut App, lparam: LPARAM) {
    use windows::Win32::System::Power::POWERBROADCAST_SETTING;
    let Some(setting) = (unsafe { (lparam.0 as *const POWERBROADCAST_SETTING).as_ref() }) else { return };
    let value = if setting.DataLength >= 4 { unsafe { *(setting.Data.as_ptr() as *const u32) } } else { 0 };
    if setting.PowerSetting == sys::GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS {
        if app.cfg.hud_brightness {
            show_hud(app, Hud::Brightness(value.min(100) as u8));
        }
    } else if setting.PowerSetting == sys::GUID_ACDC_SOURCE || setting.PowerSetting == sys::GUID_BATTERY_PERCENT {
        let (percent, charging, has) = sys::battery();
        if !has {
            return;
        }
        let prev = app.last_battery.replace((percent, charging));
        let Some((prev_pct, prev_charging)) = prev else { return };
        if prev_charging != charging && app.cfg.hud_battery {
            show_hud(app, Hud::Battery { percent, charging });
        } else if !charging && prev_pct > LOW_BATTERY && percent <= LOW_BATTERY && app.cfg.hud_battery {
            show_alert(app, AlertKind::LowBattery(percent));
        }
    }
}

