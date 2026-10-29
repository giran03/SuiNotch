//! Alerts in the notch: reminders, finished timers, events and low battery, with their buttons.

use crate::app::App;
use crate::gfx::{icon, Align, Canvas, C, R};
use crate::notch::{self, AlertKind, Hit};
use crate::pomodoro::Phase;
use crate::store;
use crate::views::{self, accent, icon_button, phase_color, pill_button, DIM};

/// Colour of calendar events, here and on the Calendar page.
pub const EVENT_COLOR: C = C(0.655, 0.545, 0.98, 1.0);

pub fn draw(cv: &mut Canvas, app: &mut App, body: R) {
    let Some((kind, since)) = app.notch.alert else { return };
    let accent = accent(app);
    let appear = (since.elapsed().as_secs_f32() / 0.25).clamp(0.0, 1.0);
    cv.push_opacity(appear);
    let (glyph, color, title, sub, primary, secondary): (char, C, String, String, String, String) = match kind {
        AlertKind::Todo(id) => {
            let task = app.todos.get(id).map(|t| t.title.clone()).unwrap_or_default();
            let color = app.todos.get(id).map(|t| C::hex(store::TODO_COLORS[t.color % store::TODO_COLORS.len()])).unwrap_or(accent);
            (icon::BELL, color, "Time's up!".into(), task, "Done".into(), format!("Snooze {} min", app.cfg.snooze_min()))
        }
        AlertKind::Water => {
            let now = store::now();
            let sub = format!("{:.2} of {:.1} L so far today", app.water.today(now) as f32 / 1000.0, app.cfg.goal_ml as f32 / 1000.0);
            ('\u{E81E}', C::hex(0x3BA7FF), "Time for some water".into(), sub, format!("Drink {} ml", app.cfg.glass_ml), "Later".into())
        }
        AlertKind::Pomodoro(Phase::Focus) => {
            let next = app.pomo.phase;
            let mins = match next {
                Phase::LongBreak => app.cfg.long_break_min,
                _ => app.cfg.short_break_min,
            };
            let primary = if app.pomo.is_running() { "Got it".to_string() } else { "Start break".to_string() };
            (icon::TIMER, phase_color(app, Phase::Focus), "Focus session done".into(), format!("Take a {mins} minute break"), primary, "Skip break".into())
        }
        AlertKind::Pomodoro(_) => {
            let primary = if app.pomo.is_running() { "Got it".to_string() } else { "Start focus".to_string() };
            (icon::TIMER, accent, "Break's over".into(), format!("Ready for {} minutes of focus?", app.cfg.focus_min), primary, "Later".into())
        }
        AlertKind::Timer => {
            let mins = app.timer.minutes();
            let sub = format!("Your {mins} minute timer is up");
            (icon::TIMER, accent, "Timer done".into(), sub, "OK".into(), "Restart".into())
        }
        AlertKind::Event(id) => {
            let event = app.calendar.iter().find(|e| e.id == id);
            let title = event.map(|e| e.title.clone()).unwrap_or_else(|| "Event".into());
            let (heading, sub) = match event {
                Some(e) => {
                    let mins = ((e.start - store::now()) as f32 / 60.0).round().max(0.0) as i64;
                    let at = crate::sys::format_time(&store::local(e.start), app.cfg.clock_format);
                    let heading = if mins <= 0 { "Starting now".to_string() } else { format!("In {mins} minute{}", if mins == 1 { "" } else { "s" }) };
                    (heading, format!("{title}  ·  {at}"))
                }
                None => ("Coming up".to_string(), title),
            };
            (icon::CALENDAR, EVENT_COLOR, heading, sub, "Got it".into(), String::new())
        }
        AlertKind::LowBattery(p) => (icon::POWER, C::hex(0xFF6B5A), "Battery low".into(), format!("{p}% left. Plug in your charger soon."), "OK".into(), String::new()),
    };
    let ic = (body.x + 40.0, 50.0);
    cv.fill_circle(ic.0, ic.1, 21.0, color.a(0.2));
    if kind == AlertKind::Water {
        crate::pages::water_icon(cv, ic.0, ic.1, 20.0, color);
    } else if kind == AlertKind::Pomodoro(Phase::Focus) && app.cfg.mascot && app.cfg.sui_reactions {
        // Sui celebrates the finished session in place of the timer icon.
        let body_color = C::hex(0xEAF6FF).mix(accent, 0.25);
        app.notch.mascot.draw(cv, ic.0, ic.1 - 1.0, 22.0, body_color);
    } else {
        cv.glyph(glyph, ic.0, ic.1, 17.0, color);
    }
    let tx = body.x + 74.0;
    cv.display(&title, R::new(tx, 18.0, body.w - 120.0, 22.0), 15.5, 700, C::WHITE, Align::Left);
    cv.text(&sub, R::new(tx, 40.0, body.w - 100.0, 18.0), 12.5, 500, DIM, Align::Left);
    let by = body.bottom() - 40.0;
    let pw = app.gfx.text_width(&primary, 12.5, 600) + 30.0;
    let pr = R::new(body.right() - 22.0 - pw, by, pw, 28.0);
    pill_button(cv, app, pr, &primary, Hit::AlertPrimary, true);
    if !secondary.is_empty() {
        let sw = app.gfx.text_width(&secondary, 12.5, 600) + 30.0;
        pill_button(cv, app, R::new(pr.x - 8.0 - sw, by, sw, 28.0), &secondary, Hit::AlertSecondary, false);
    }
    icon_button(cv, app, body.right() - 24.0, 22.0, 9.0, icon::CLOSE, Hit::AlertClose, DIM);
    cv.pop();
}

pub fn action(app: &mut App, primary: bool) {
    let Some((kind, _)) = app.notch.alert.take() else { return };
    let now = store::now();
    match kind {
        AlertKind::Todo(id) => {
            if primary {
                if app.todos.get(id).is_some_and(|t| !t.done) {
                    app.toggle_task(id, now);
                }
            } else {
                let m = app.cfg.snooze_min();
                app.todos.snooze(id, now, m);
                app.save_todos();
            }
        }
        AlertKind::Water => {
            if primary {
                views::drink(app, now);
            } else {
                app.water.snoozed_until = Some(now + 15 * 60);
                app.save_water();
            }
        }
        AlertKind::Pomodoro(_) => {
            let n = app.now_ms();
            if primary && !app.pomo.is_running() {
                app.pomo.start(n);
            } else if !primary {
                if app.pomo.phase != Phase::Focus {
                    let d = app.durations();
                    app.pomo.switch(Phase::Focus, &d);
                }
            }
        }
        AlertKind::Timer => {
            if !primary {
                let n = app.now_ms();
                app.timer.reset();
                app.timer.toggle(n);
            }
        }
        AlertKind::Event(_) | AlertKind::LowBattery(_) => {}
    }
    notch::schedule(app);
}
