//! The Focus page: Pomodoro, a plain timer and a stopwatch, behind a small mode switch.

use crate::app::App;
use crate::gfx::{icon, Align, Canvas, C, R};
use crate::notch::{self, Hit};
use crate::pomodoro::{self, Phase};
use crate::timers::{MAX_MIN, PRESETS_MIN};
use crate::views::{self, accent, fmt_ms, hit, hovered, icon_button, pill_button, DIM, FAINT};

const MODES: [&str; 3] = ["Pomodoro", "Timer", "Stopwatch"];
const STOPWATCH_COLOR: C = C(0.176, 0.831, 0.749, 1.0);
const LAP_ROWS: usize = 3;

pub fn timer(cv: &mut Canvas, app: &mut App, area: R) {
    let x0 = area.x + 152.0;
    mode_tabs(cv, app, x0, area.y);
    match app.cfg.focus_mode {
        1 => countdown(cv, app, area, x0),
        2 => stopwatch(cv, app, area, x0),
        _ => pomodoro_mode(cv, app, area, x0),
    }
}

/// The ring on the left: progress, big time and a label.
fn ring(cv: &Canvas, area: R, frac: f32, time: &str, label: &str, color: C) {
    let (cx, cy, radius) = (area.x + 64.0, area.y + 75.0, 58.0);
    cv.stroke_circle(cx, cy, radius, C::WHITE.a(0.1), 7.0);
    cv.arc(cx, cy, radius, 0.0, frac.max(0.0005) * std::f32::consts::TAU, color, 7.0);
    let size = if time.len() > 6 { 22.0 } else { 27.0 };
    cv.display(time, R::centered(cx, cy - 6.0, 112.0, 34.0), size, 700, C::WHITE, Align::Center);
    cv.text(label, R::centered(cx, cy + 20.0, 110.0, 16.0), 11.5, 600, color, Align::Center);
}

/// Pomodoro, Timer and Stopwatch tabs; a clock running out of view keeps a dot on its tab.
fn mode_tabs(cv: &Canvas, app: &mut App, x0: f32, y: f32) {
    let running = [app.pomo.is_running(), app.timer.is_running(), app.stopwatch.is_running()];
    let mut x = x0;
    for (i, label) in MODES.iter().enumerate() {
        let on = app.cfg.focus_mode == i;
        let dot = if running[i] && !on { 10.0 } else { 0.0 };
        let w = app.gfx.text_width(label, 11.5, 600) + 20.0 + dot;
        let r = R::new(x, y, w, 22.0);
        let h = Hit::FocusMode(i as u8);
        hit(app, r, h);
        let fill = if on { C::WHITE.a(0.16) } else if hovered(app, h) { FAINT } else { C::CLEAR };
        cv.fill_rrect(r, 11.0, fill);
        if dot > 0.0 {
            cv.fill_circle(r.x + 12.0, r.cy(), 3.0, accent(app));
        }
        cv.text(label, R::new(r.x + dot, r.y, r.w - dot, r.h), 11.5, 600, if on { C::WHITE } else { DIM }, Align::Center);
        x += w + 4.0;
    }
}

// ------------------------------------------------------------------ Pomodoro

fn pomodoro_mode(cv: &mut Canvas, app: &mut App, area: R, x0: f32) {
    let d = app.durations();
    let now = app.now_ms();
    // A phase picked while a timer is under way is previewed at full length; the timer keeps going.
    let preview = app.pomo.preview();
    let shown = preview.unwrap_or(app.pomo.phase);
    let total = pomodoro::phase_length(shown, &d).max(1);
    let left = if preview.is_some() { total } else { app.pomo.remaining(now) };
    let color = views::phase_color(app, shown);
    ring(cv, area, 1.0 - left as f32 / total as f32, &fmt_ms(left + 999), shown.label(), color);

    let w = area.right() - x0;
    let labels = ["Focus", "Short break", "Long break"];
    let phases = [Phase::Focus, Phase::ShortBreak, Phase::LongBreak];
    let mut x = x0;
    // The timer under way (running, or paused partway) keeps a dot on its chip, plus its time when out of view.
    let under_way = !app.pomo.is_fresh(&d);
    for (i, label) in labels.iter().enumerate() {
        let on_screen = shown == phases[i];
        let timing = under_way && app.pomo.phase == phases[i];
        let text = if timing && !on_screen { format!("{label}  {}", fmt_ms(app.pomo.remaining(now) + 999)) } else { label.to_string() };
        let dot = if timing { 12.0 } else { 0.0 };
        let cw = app.gfx.text_width(&text, 12.0, 600) + 26.0 + dot;
        let r = R::new(x, area.y + 32.0, cw, 28.0);
        let h = Hit::PomoPhase(i as u8);
        hit(app, r, h);
        let phase_color = views::phase_color(app, phases[i]);
        let fill = if on_screen { phase_color.a(0.24) } else if hovered(app, h) { FAINT } else { C::WHITE.a(0.05) };
        cv.fill_rrect(r, 14.0, fill);
        if timing {
            cv.fill_circle(r.x + 15.0, r.cy(), 3.5, phase_color);
        }
        let text_r = R::new(r.x + dot, r.y, r.w - dot, r.h);
        cv.text(&text, text_r, 12.0, 600, if on_screen || timing { C::WHITE } else { DIM }, Align::Center);
        x += cw + 8.0;
    }

    let sessions = app.cfg.sessions_before_long;
    let done = app.pomo.completed_focus % sessions;
    let label = format!("Session {} of {}  ·  {} done today", done + 1, sessions, app.pomo.completed_focus);
    cv.text(&label, R::new(x0, area.y + 68.0, w, 18.0), 12.0, 500, DIM, Align::Left);
    for i in 0..sessions {
        let filled = i < done;
        let dx = x0 + 5.0 + i as f32 * 16.0;
        cv.fill_circle(dx, area.y + 96.0, 4.5, if filled { color } else { C::WHITE.a(0.15) });
    }

    let by = area.bottom() - 32.0;
    if preview.is_some() {
        // Start replaces the timer under way with this phase; Back returns to it untouched.
        pill_button(cv, app, R::new(x0, by, 112.0, 32.0), "Start", Hit::PomoToggle, true);
        pill_button(cv, app, R::new(x0 + 120.0, by, 84.0, 32.0), "Back", Hit::PomoReset, false);
        return;
    }
    let running = app.pomo.is_running();
    let main = if running { "Pause" } else if app.pomo.is_fresh(&d) { "Start" } else { "Resume" };
    pill_button(cv, app, R::new(x0, by, 112.0, 32.0), main, Hit::PomoToggle, true);
    pill_button(cv, app, R::new(x0 + 120.0, by, 84.0, 32.0), "Reset", Hit::PomoReset, false);
    pill_button(cv, app, R::new(x0 + 212.0, by, 84.0, 32.0), "Skip", Hit::PomoSkip, false);
}

// ------------------------------------------------------------------ timer

fn countdown(cv: &mut Canvas, app: &mut App, area: R, x0: f32) {
    let now = app.now_ms();
    let t = app.timer.clone();
    let left = t.remaining(now);
    let accent = accent(app);
    ring(cv, area, 1.0 - left as f32 / t.length_ms.max(1) as f32, &fmt_ms(left + 999), "Timer", accent);

    // Presets pick a length while the timer is untouched; a running timer is never cut short.
    let mut x = x0;
    for (i, m) in PRESETS_MIN.iter().enumerate() {
        let label = format!("{m} min");
        let cw = app.gfx.text_width(&label, 12.0, 600) + 22.0;
        let r = R::new(x, area.y + 32.0, cw, 28.0);
        let h = Hit::TimerPreset(i as u8);
        let on = t.minutes() == *m;
        if t.is_fresh() {
            hit(app, r, h);
        }
        let fill = if on { accent.a(0.24) } else if hovered(app, h) { FAINT } else { C::WHITE.a(0.05) };
        cv.fill_rrect(r, 14.0, fill);
        cv.text(&label, r, 12.0, 600, if on { C::WHITE } else if t.is_fresh() { DIM } else { C::WHITE.a(0.25) }, Align::Center);
        x += cw + 6.0;
    }

    let cy = area.y + 82.0;
    if t.is_fresh() {
        icon_button(cv, app, x0 + 10.0, cy, 9.0, '\u{E738}', Hit::TimerStep(-1), C::WHITE);
        cv.text(&format!("{} min", t.minutes()), R::new(x0 + 24.0, cy - 10.0, 64.0, 20.0), 13.0, 650, C::WHITE, Align::Center);
        icon_button(cv, app, x0 + 102.0, cy, 9.0, icon::ADD, Hit::TimerStep(1), C::WHITE);
    } else {
        let ends = chrono::Local::now() + chrono::Duration::milliseconds(left as i64);
        let line = if t.is_running() { format!("Ends at {}", crate::sys::format_time(&ends, app.cfg.clock_format)) } else { "Paused".to_string() };
        cv.text(&line, R::new(x0, cy - 10.0, 220.0, 20.0), 12.0, 500, DIM, Align::Left);
    }

    let by = area.bottom() - 32.0;
    let main = if t.is_running() { "Pause" } else if t.is_fresh() || left == 0 { "Start" } else { "Resume" };
    pill_button(cv, app, R::new(x0, by, 112.0, 32.0), main, Hit::TimerToggle, true);
    if !t.is_fresh() {
        pill_button(cv, app, R::new(x0 + 120.0, by, 84.0, 32.0), "Reset", Hit::TimerReset, false);
    }
}

// ------------------------------------------------------------------ stopwatch

fn fmt_tenths(ms: u64) -> String {
    format!("{}.{}", fmt_ms(ms), (ms % 1000) / 100)
}

fn stopwatch(cv: &mut Canvas, app: &mut App, area: R, x0: f32) {
    let now = app.now_ms();
    let s = app.stopwatch.clone();
    let elapsed = s.elapsed(now);
    ring(cv, area, (elapsed % 60_000) as f32 / 60_000.0, &fmt_tenths(elapsed), "Stopwatch", STOPWATCH_COLOR);

    if s.laps.is_empty() {
        cv.text("Lap marks each split while it runs", R::new(x0, area.y + 36.0, 260.0, 18.0), 12.0, 500, DIM, Align::Left);
    }
    for (row, i) in (0..s.laps.len()).rev().take(LAP_ROWS).enumerate() {
        let y = area.y + 32.0 + row as f32 * 22.0;
        let fade = 1.0 - row as f32 * 0.22;
        cv.text(&format!("Lap {}", i + 1), R::new(x0, y, 80.0, 20.0), 12.0, 600, DIM.a(0.56 * fade), Align::Left);
        cv.text(&fmt_tenths(s.lap_length(i)), R::new(x0 + 70.0, y, 90.0, 20.0), 12.5, 650, C::WHITE.a(fade), Align::Right);
        cv.text(&fmt_tenths(s.laps[i]), R::new(x0 + 170.0, y, 90.0, 20.0), 11.5, 500, DIM.a(0.56 * fade), Align::Right);
    }

    let by = area.bottom() - 32.0;
    let main = if s.is_running() { "Pause" } else if elapsed == 0 { "Start" } else { "Resume" };
    pill_button(cv, app, R::new(x0, by, 112.0, 32.0), main, Hit::WatchToggle, true);
    if s.is_running() {
        pill_button(cv, app, R::new(x0 + 120.0, by, 84.0, 32.0), "Lap", Hit::WatchLap, false);
    } else if elapsed > 0 {
        pill_button(cv, app, R::new(x0 + 120.0, by, 84.0, 32.0), "Reset", Hit::WatchReset, false);
    }
}

// ------------------------------------------------------------------ clicks

/// Handles the Focus page's mode, timer and stopwatch controls.
pub fn click(app: &mut App, h: Hit) {
    let now = app.now_ms();
    match h {
        Hit::FocusMode(m) => {
            let mut c = app.cfg.clone();
            c.focus_mode = m as usize;
            app.set_config(c);
        }
        Hit::TimerPreset(i) => set_timer_minutes(app, PRESETS_MIN[i as usize % PRESETS_MIN.len()]),
        Hit::TimerStep(dir) => {
            let m = app.timer.minutes();
            // One-minute steps up to ten minutes, five-minute steps above.
            let next = if dir > 0 { m + if m < 10 { 1 } else { 5 } } else { m.saturating_sub(if m <= 10 { 1 } else { 5 }) };
            set_timer_minutes(app, next.clamp(1, MAX_MIN));
        }
        Hit::TimerToggle => app.timer.toggle(now),
        Hit::TimerReset => app.timer.reset(),
        Hit::WatchToggle => app.stopwatch.toggle(now),
        Hit::WatchLap => app.stopwatch.lap(now),
        Hit::WatchReset => app.stopwatch.reset(),
        _ => {}
    }
    notch::schedule(app);
}

fn set_timer_minutes(app: &mut App, minutes: u32) {
    if app.timer.set_minutes(minutes) {
        let mut c = app.cfg.clone();
        c.timer_min = minutes;
        app.set_config(c);
    }
}
