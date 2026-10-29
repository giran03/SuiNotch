//! Drawing of the notch body, its closed states, HUDs, alerts, the lyrics pill and the
//! page bar, plus click dispatch. Page contents live in `pages.rs`.

use std::time::Instant;

use crate::app::{App, Deferred};
use crate::gfx::{icon, Align, Canvas, C, R};
use crate::media::{self, Command};
use crate::notch::{self, AlertKind, EditTarget, Hit, Hud, Notch, Page, TextField};
use crate::pages;
use crate::pomodoro::Phase;
use crate::store;

pub const DIM: C = C(1.0, 1.0, 1.0, 0.56);
pub const FAINT: C = C(1.0, 1.0, 1.0, 0.1);
pub const SUBTLE: C = C(1.0, 1.0, 1.0, 0.18);

pub fn accent(app: &App) -> C {
    C::hex(app.cfg.accent_rgb())
}

pub fn hit(app: &mut App, r: R, h: Hit) {
    app.notch.hits.push((r, h));
}

pub fn hovered(app: &App, h: Hit) -> bool {
    app.notch.hover == Some(h)
}

pub fn pressed(app: &App, h: Hit) -> bool {
    app.notch.pressed.is_some_and(|(p, _, _)| p == h) && hovered(app, h)
}

pub fn icon_button(cv: &Canvas, app: &mut App, cx: f32, cy: f32, size: f32, glyph: char, h: Hit, color: C) {
    let r = R::centered(cx, cy, size * 2.1, size * 2.1);
    hit(app, r, h);
    if hovered(app, h) {
        cv.fill_circle(cx, cy, size * 1.05, if pressed(app, h) { SUBTLE } else { FAINT });
    }
    let s = if pressed(app, h) { size * 0.92 } else { size };
    cv.glyph(glyph, cx, cy, s, color);
}

pub fn pill_button(cv: &Canvas, app: &mut App, r: R, label: &str, h: Hit, primary: bool) {
    hit(app, r, h);
    let base = if primary { accent(app) } else { C::WHITE.a(0.12) };
    let fill = if pressed(app, h) {
        base.mix(C::BLACK, 0.2)
    } else if hovered(app, h) {
        base.mix(C::WHITE, 0.14)
    } else {
        base
    };
    cv.fill_rrect(r, r.h / 2.0, fill);
    let fg = if primary { readable_on(base) } else { C::WHITE };
    cv.text(label, r, 12.5, 600, fg, Align::Center);
}

pub fn readable_on(bg: C) -> C {
    let lum = 0.299 * bg.0 + 0.587 * bg.1 + 0.114 * bg.2;
    if lum > 0.62 {
        C(0.04, 0.05, 0.07, 1.0)
    } else {
        C::WHITE
    }
}

pub fn bar(cv: &Canvas, r: R, frac: f32, fg: C) {
    cv.fill_rrect(r, r.h / 2.0, C::WHITE.a(0.16));
    let w = (r.w * frac.clamp(0.0, 1.0)).max(r.h);
    if frac > 0.0 {
        cv.fill_rrect(R::new(r.x, r.y, w, r.h), r.h / 2.0, fg);
    }
}

pub fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

// ------------------------------------------------------------------ frame

pub fn draw_notch(cv: &mut Canvas, app: &mut App) {
    let body = app.notch.body();
    let open = app.notch.open.value.clamp(0.0, 1.0);
    let radius = app.notch.radius.value;
    let raised = app.notch.alert.is_some() || app.notch.hud.is_some();
    if app.cfg.shadow && (open > 0.02 || raised) {
        let a = if raised { 0.7 } else { open };
        for i in 1..=7 {
            let g = i as f32 * 2.6;
            cv.fill_rrect(R::new(body.x - g * 0.5, -20.0, body.w + g, body.h + 20.0 + g * 0.9), radius + g, C::BLACK.a(a * 0.045));
        }
    }
    let Some(path) = notch::outline(cv, body, radius) else { return };
    let fill = if app.cfg.glass { C(0.05, 0.055, 0.07, 0.88) } else { C::BLACK };
    cv.fill_path(&path, fill);
    if app.cfg.glass {
        cv.stroke_path(&path, C::WHITE.a(0.09), 1.0);
    }
    let music_visible = open < 0.5 || app.notch.page == Page::Music;
    if app.cfg.artwork_glow && app.media.is_some() && !raised && music_visible {
        if let Some(col) = app.art_color {
            // A soft halo centred behind the cover, closed or open.
            let gx = crate::anim::lerp(body.x + 22.0, body.x + 74.0, open);
            let gy = crate::anim::lerp(body.h * 0.5, 92.0, open);
            let radius = crate::anim::lerp(40.0, 190.0, open);
            cv.fill_path_glow(&path, gx, gy, radius, col.a(0.16));
        }
    }
    // Content sits well inside the rounded corners, so a rectangle clip is enough and far cheaper
    // than clipping to the outline.
    cv.push_clip_rect(body);
    hit(app, body, Hit::Body);
    if app.notch.alert.is_some() {
        draw_alert(cv, app, body);
    } else if let Some((h, _)) = app.notch.hud.clone() {
        draw_hud(cv, app, body, &h);
    } else {
        let closed = (1.0 - open * 2.4).clamp(0.0, 1.0);
        if closed > 0.01 && body.h > 12.0 {
            cv.push_opacity(closed);
            let keep = app.notch.hits.len();
            draw_closed(cv, app, body);
            if open > 0.3 {
                app.notch.hits.truncate(keep);
            }
            cv.pop();
        }
        if open > 0.2 {
            cv.push_opacity(((open - 0.2) / 0.8).clamp(0.0, 1.0));
            draw_expanded(cv, app, body);
            cv.pop();
        }
    }
    if let Some((text, since)) = app.notch.notice.clone() {
        let a = (1.0 - (since.elapsed().as_secs_f32() - 1.1).max(0.0) / 0.4).clamp(0.0, 1.0);
        let w = app.gfx.text_width(&text, 12.0, 600) + 28.0;
        let r = R::centered(body.cx(), body.bottom() - 18.0, w, 24.0);
        cv.fill_rrect(r, 12.0, accent(app).a(a));
        cv.text(&text, r, 12.0, 600, readable_on(accent(app)).a(a), Align::Center);
    }
    cv.pop();
    draw_pill(cv, app, body);
}

// ------------------------------------------------------------------ closed states

fn draw_closed(cv: &mut Canvas, app: &mut App, body: R) {
    let cy = body.h / 2.0;
    let accent = accent(app);
    if notch::mascot_visible(app) {
        let body_color = C::hex(0xEAF6FF).mix(accent, 0.25);
        app.notch.mascot.draw(cv, body.cx(), cy - 1.0, 19.0, body_color);
        return;
    }
    if let Some(m) = app.media.as_ref() {
        let playing = m.info.playing;
        let art = R::new(body.x + 12.0, cy - 10.0, 20.0, 20.0);
        if !pages::draw_artwork(cv, app, art, 5.0) {
            cv.fill_rrect(art, 5.0, C::WHITE.a(0.12));
            cv.glyph(icon::MUSIC, art.cx(), art.cy(), 10.0, DIM);
        }
        let color = app.art_color.unwrap_or(accent);
        let ex = body.right() - 30.0;
        if playing && app.cfg.equalizer {
            let t = app.notch.eq_phase;
            for i in 0..4 {
                let f = [5.3f32, 7.1, 4.4, 6.2][i];
                let h = 4.0 + 10.0 * (0.5 + 0.5 * (t * f + i as f32 * 1.7).sin()).powf(1.4);
                let x = ex + i as f32 * 4.6;
                cv.fill_rrect(R::new(x, cy - h / 2.0, 2.6, h), 1.3, color);
            }
        } else {
            cv.glyph(if playing { icon::PLAY } else { icon::PAUSE }, ex + 7.0, cy, 10.0, DIM);
        }
        return;
    }
    if app.pomo.is_running() {
        let d = app.durations();
        let left = app.pomo.remaining(app.now_ms());
        let frac = 1.0 - left as f32 / app.pomo.length(&d).max(1) as f32;
        let (x, color) = (body.x + 22.0, phase_color(app, app.pomo.phase));
        cv.stroke_circle(x, cy, 7.5, C::WHITE.a(0.16), 2.4);
        cv.arc(x, cy, 7.5, 0.0, frac * std::f32::consts::TAU, color, 2.4);
        cv.text(&fmt_ms(left + 999), R::new(body.right() - 80.0, 0.0, 64.0, body.h), 13.0, 650, C::WHITE, Align::Right);
        return;
    }
    let now = chrono::Local::now();
    let time = crate::sys::format_time(&now, app.cfg.clock_format);
    cv.display(&time, R::new(body.x, 0.0, body.w, body.h), 13.5, 650, C::WHITE, Align::Center);
}

pub fn phase_color(app: &App, phase: Phase) -> C {
    match phase {
        Phase::Focus => C::hex(0xFF6B5A),
        Phase::ShortBreak => accent(app),
        Phase::LongBreak => C::hex(0x34C759),
    }
}

// ------------------------------------------------------------------ HUD

fn draw_hud(cv: &mut Canvas, app: &mut App, body: R, hud: &Hud) {
    let wide = app.cfg.hud_style == 1;
    let cy = body.h / 2.0;
    let ix = body.x + if wide { 26.0 } else { 22.0 };
    let accent = accent(app);
    let label_r = R::new(ix + 18.0, 0.0, body.w * 0.48, body.h);
    let right_r = R::new(body.right() - 128.0, 0.0, 108.0, body.h);
    let bar_r = R::new(body.right() - 124.0, cy - 2.5, 100.0, 5.0);
    let size = if wide { 13.0 } else { 12.5 };
    match hud {
        Hud::Volume { level, muted } => {
            let g = if *muted || *level <= 0.001 { icon::MUTE } else { icon::VOLUME };
            cv.glyph(g, ix, cy, 14.0, C::WHITE);
            if wide {
                cv.text("Volume", label_r, size, 600, C::WHITE, Align::Left);
            }
            let r = if wide { bar_r } else { R::new(ix + 22.0, cy - 2.5, body.w - 100.0, 5.0) };
            bar(cv, r, if *muted { 0.0 } else { *level }, if *muted { DIM } else { C::WHITE });
            let pct = if *muted { "Muted".to_string() } else { format!("{}", (level * 100.0).round() as u32) };
            cv.text(&pct, R::new(body.right() - 58.0, 0.0, 42.0, body.h), 12.0, 600, DIM, Align::Right);
        }
        Hud::Brightness(v) => {
            cv.glyph(icon::BRIGHTNESS, ix, cy, 14.0, C::WHITE);
            if wide {
                cv.text("Brightness", label_r, size, 600, C::WHITE, Align::Left);
            }
            let r = if wide { bar_r } else { R::new(ix + 22.0, cy - 2.5, body.w - 100.0, 5.0) };
            bar(cv, r, *v as f32 / 100.0, C::hex(0xFFD54A));
            cv.text(&format!("{v}"), R::new(body.right() - 58.0, 0.0, 42.0, body.h), 12.0, 600, DIM, Align::Right);
        }
        Hud::Caps(on) => {
            cv.glyph(icon::UP, ix, cy, 13.0, if *on { accent } else { DIM });
            cv.text(if *on { "Caps Lock on" } else { "Caps Lock off" }, label_r, size, 600, C::WHITE, Align::Left);
            cv.fill_circle(body.right() - 26.0, cy, 4.0, if *on { accent } else { SUBTLE });
        }
        Hud::Language { short, name } => {
            cv.glyph(icon::KEYBOARD, ix, cy, 14.0, C::WHITE);
            cv.text(if name.is_empty() { "Keyboard language" } else { name }, R::new(ix + 18.0, 0.0, body.w - 110.0, body.h), size, 600, C::WHITE, Align::Left);
            let w = app.gfx.text_width(short, 11.5, 700) + 14.0;
            let b = R::new(body.right() - 18.0 - w, cy - 10.0, w, 20.0);
            cv.fill_rrect(b, 6.0, accent);
            cv.text(short, b, 11.5, 700, readable_on(accent), Align::Center);
        }
        Hud::Battery { percent, charging } => {
            battery_icon(cv, ix, cy, *percent, *charging, accent);
            let label = if *charging { "Charging" } else { "On battery" };
            cv.text(label, R::new(ix + 22.0, 0.0, body.w * 0.5, body.h), size, 600, C::WHITE, Align::Left);
            let col = if *charging { C::hex(0x34C759) } else if *percent <= 20 { C::hex(0xFF6B5A) } else { DIM };
            cv.text(&format!("{percent}%"), right_r, 12.5, 650, col, Align::Right);
        }
        Hud::Wifi { connected, name } => {
            cv.glyph(if *connected { icon::WIFI } else { icon::WIFI_OFF }, ix, cy, 14.0, if *connected { C::WHITE } else { C::hex(0xFF6B5A) });
            cv.text(if *connected { "Connected" } else { "No internet" }, label_r, size, 600, C::WHITE, Align::Left);
            cv.text(name, right_r, 12.0, 500, DIM, Align::Right);
        }
        Hud::Headphones { name, connected } => {
            cv.glyph(icon::HEADPHONE, ix, cy, 14.0, if *connected { accent } else { DIM });
            cv.text(name, R::new(ix + 20.0, 0.0, body.w - 130.0, body.h), size, 600, C::WHITE, Align::Left);
            cv.text(if *connected { "Connected" } else { "Disconnected" }, right_r, 12.0, 500, DIM, Align::Right);
        }
    }
}

pub fn battery_icon(cv: &Canvas, cx: f32, cy: f32, percent: u8, charging: bool, accent: C) {
    let r = R::centered(cx - 1.0, cy, 20.0, 10.0);
    cv.stroke_rrect(r, 3.0, C::WHITE.a(0.8), 1.2);
    cv.fill_rrect(R::new(r.right() + 0.8, cy - 2.0, 1.8, 4.0), 0.9, C::WHITE.a(0.8));
    let col = if charging { C::hex(0x34C759) } else if percent <= 20 { C::hex(0xFF6B5A) } else { accent.mix(C::WHITE, 0.6) };
    let inner = r.inset(2.0, 2.0);
    cv.fill_rrect(R::new(inner.x, inner.y, (inner.w * percent as f32 / 100.0).max(1.5), inner.h), 1.5, col);
}

// ------------------------------------------------------------------ alerts

fn draw_alert(cv: &mut Canvas, app: &mut App, body: R) {
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
        AlertKind::LowBattery(p) => (icon::POWER, C::hex(0xFF6B5A), "Battery low".into(), format!("{p}% left. Plug in your charger soon."), "OK".into(), String::new()),
    };
    let ic = (body.x + 40.0, 50.0);
    cv.fill_circle(ic.0, ic.1, 21.0, color.a(0.2));
    if kind == AlertKind::Water {
        crate::pages::water_icon(cv, ic.0, ic.1, 20.0, color);
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

// ------------------------------------------------------------------ lyrics pill

fn draw_pill(cv: &mut Canvas, app: &mut App, body: R) {
    let a = app.notch.pill_alpha.value.clamp(0.0, 1.0);
    if a < 0.01 {
        return;
    }
    let Some(i) = app.notch.pill_line else { return };
    let Some(line) = app.lyrics.get(i).map(|l| l.text.clone()) else { return };
    let w = app.notch.pill_w.value.max(40.0);
    let r = R::centered(body.cx(), body.bottom() + 8.0 + 13.0 - (1.0 - a) * 8.0, w, 26.0);
    cv.push_opacity(a);
    cv.fill_rrect(r, 13.0, if app.cfg.glass { C(0.05, 0.055, 0.07, 0.9) } else { C::BLACK });
    let f = app.notch.line_fade.value.clamp(0.0, 1.0);
    let color = app.art_color.map(|c| c.mix(C::WHITE, 0.55)).unwrap_or(C::WHITE);
    cv.text(&line, r.inset(14.0, 0.0).offset(0.0, (1.0 - f) * 5.0), 12.5, 600, color.a(f), Align::Center);
    cv.pop();
}

// ------------------------------------------------------------------ expanded

fn draw_expanded(cv: &mut Canvas, app: &mut App, body: R) {
    let pages = Notch::pages(&app.cfg);
    let page = if app.notch.drag_over && !app.notch.expanded { Page::Shelf } else { app.notch.page };
    draw_page_bar(cv, app, body, &pages, page);
    let pos = app.notch.page_pos.value;
    for (i, p) in pages.iter().enumerate() {
        let d = i as f32 - pos;
        if d.abs() >= 1.0 && *p != page {
            continue;
        }
        let d = if *p == page && app.notch.drag_over && !app.notch.expanded { 0.0 } else { d };
        let alpha = (1.0 - d.abs() * 1.4).clamp(0.0, 1.0);
        if alpha <= 0.01 {
            continue;
        }
        let area = R::new(body.x + 24.0 + d * 70.0, 40.0, body.w - 48.0, body.h - 40.0 - 18.0);
        let keep = app.notch.hits.len();
        cv.push_opacity(alpha);
        match p {
            Page::Music => pages::music(cv, app, area),
            Page::Timer => pages::timer(cv, app, area),
            Page::Water => pages::water(cv, app, area),
            Page::Shelf => pages::shelf(cv, app, area),
            Page::Todo => pages::todo(cv, app, area),
            Page::Calendar => pages::calendar(cv, app, area),
        }
        cv.pop();
        if d.abs() > 0.45 {
            app.notch.hits.truncate(keep);
        }
    }
}

fn draw_page_bar(cv: &mut Canvas, app: &mut App, body: R, pages: &[Page], current: Page) {
    let cy = 19.0;
    let (mx, my) = app.notch.mouse;
    let bar_hover = app.notch.hovering && my < 34.0 && (mx - body.cx()).abs() < 140.0;
    let accent = accent(app);
    let n = pages.len() as f32;
    let spacing = if bar_hover { 30.0 } else { 13.0 };
    let x0 = body.cx() - spacing * (n - 1.0) / 2.0;
    for (i, p) in pages.iter().enumerate() {
        let x = x0 + i as f32 * spacing;
        let active = *p == current;
        hit(app, R::centered(x, cy, spacing.max(22.0), 26.0), Hit::PageDot(*p));
        if bar_hover {
            if active {
                cv.fill_circle(x, cy, 12.0, accent.a(0.22));
            } else if hovered(app, Hit::PageDot(*p)) {
                cv.fill_circle(x, cy, 12.0, FAINT);
            }
            let col = if active { accent } else { DIM };
            if *p == Page::Water {
                crate::pages::water_icon(cv, x, cy, 13.0, col);
            } else {
                cv.glyph(p.icon(), x, cy, 12.0, col);
            }
        } else if active {
            cv.fill_rrect(R::centered(x, cy, 14.0, 5.0), 2.5, accent);
        } else {
            cv.fill_circle(x, cy, 2.5, C::WHITE.a(0.35));
        }
    }
    cv.text(current.label(), R::new(body.x + 26.0, cy - 9.0, 120.0, 18.0), 12.0, 650, DIM, Align::Left);
    icon_button(cv, app, body.right() - 30.0, cy, 11.0, icon::SETTINGS, Hit::Settings, DIM);
}

pub fn draw_field(cv: &Canvas, app: &App, r: R, field: &TextField, placeholder: &str) {
    cv.fill_rrect(r, 9.0, C::WHITE.a(0.1));
    cv.stroke_rrect(r, 9.0, accent(app).a(0.7), 1.2);
    let inner = r.inset(12.0, 0.0);
    if field.text.is_empty() {
        cv.text(placeholder, inner, 12.5, 500, C::WHITE.a(0.35), Align::Left);
    }
    let before: String = field.text.chars().take(field.caret).collect();
    let caret_x = app.gfx.text_width(&before, 12.5, 500);
    let shift = (caret_x - inner.w + 6.0).max(0.0);
    cv.text(&field.text, R::new(inner.x - shift, inner.y, inner.w + shift, inner.h), 12.5, 500, C::WHITE, Align::Left);
    let blink = (app.started.elapsed().as_millis() / 530) % 2 == 0;
    if blink {
        let x = inner.x + caret_x - shift;
        cv.line(x, r.cy() - 7.0, x, r.cy() + 7.0, accent(app), 1.4);
    }
}

// ------------------------------------------------------------------ geometry queries for input

pub fn seek_bar_rect(app: &App) -> Option<R> {
    app.notch.hits.iter().find(|(_, h)| *h == Hit::Seek).map(|(r, _)| r.inset(0.0, 6.0))
}

pub fn volume_bar_rect(app: &App) -> Option<R> {
    app.notch.hits.iter().find(|(_, h)| *h == Hit::VolumeBar).map(|(r, _)| r.inset(6.0, 0.0))
}

pub fn todo_overflow(app: &App) -> f32 {
    let rows = pages::visible_todos(app).len() as f32;
    let (_, h) = Page::Todo.size(false);
    let list_h = h - 40.0 - 18.0 - 30.0 - 40.0;
    (rows * pages::TODO_ROW - list_h).max(0.0)
}

pub fn shelf_overflow(app: &App) -> f32 {
    let (w, _) = Page::Shelf.size(false);
    let total = app.shelf.items.len() as f32 * (pages::TILE_W + 10.0);
    (total - (w - 48.0)).max(0.0)
}

// ------------------------------------------------------------------ clicks

pub fn click(app: &mut App, h: Hit, (ctrl, shift): (bool, bool)) {
    let now = store::now();
    match h {
        Hit::Body | Hit::TodoRow(_) => {}
        Hit::PageDot(p) => notch::set_page(app, p),
        Hit::Settings => {
            notch::collapse(app);
            crate::app::defer(Deferred::OpenSettings(None));
        }
        Hit::Prev => media::command(Command::Previous),
        Hit::PlayPause => {
            if let Some(m) = app.media.as_mut() {
                let pos = m.position_ms();
                m.info.position_ms = pos;
                m.at = Instant::now();
                m.info.playing = !m.info.playing;
            }
            media::command(Command::PlayPause);
        }
        Hit::Next => media::command(Command::Next),
        Hit::Source => media::command(Command::NextSource),
        Hit::Seek | Hit::VolumeBar => {}
        Hit::LyricsToggle => app.notch.lyrics_view = !app.notch.lyrics_view,
        Hit::Volume => {
            if let Some(a) = app.audio.as_ref() {
                a.toggle_mute();
            }
        }
        Hit::LyricLine(ms) => {
            media::command(Command::Seek(ms));
            if let Some(m) = app.media.as_mut() {
                m.info.position_ms = ms;
                m.at = Instant::now();
            }
        }
        Hit::PomoToggle => {
            let (n, d) = (app.now_ms(), app.durations());
            app.pomo.toggle(n, &d);
            notch::schedule(app);
        }
        Hit::PomoReset => {
            // On a previewed phase this button reads "Back" and leaves the timer alone.
            if app.pomo.preview().is_some() {
                app.pomo.back();
            } else {
                let d = app.durations();
                app.pomo.reset(&d);
            }
            notch::schedule(app);
        }
        Hit::PomoSkip => {
            let (n, d) = (app.now_ms(), app.durations());
            let running = app.pomo.is_running();
            app.pomo.advance(n, &d, running);
            notch::schedule(app);
        }
        Hit::PomoPhase(i) => {
            let d = app.durations();
            let phase = [Phase::Focus, Phase::ShortBreak, Phase::LongBreak][i as usize % 3];
            app.pomo.select(phase, &d);
            notch::schedule(app);
        }
        Hit::Drink => drink(app, now),
        Hit::WaterUndo => {
            if app.water.undo_today(now) {
                app.save_water();
                notch::schedule(app);
            }
        }
        Hit::ShelfItem(i) => shelf_click(app, i, ctrl, shift),
        Hit::ShelfSelect => {
            app.notch.shelf_selecting = !app.notch.shelf_selecting;
            app.notch.shelf_sel.clear();
        }
        Hit::ShelfRemove => {
            let sel: Vec<usize> = app.notch.shelf_sel.iter().copied().collect();
            app.shelf.remove_indices(&sel);
            app.notch.shelf_sel.clear();
            app.notch.shelf_selecting = false;
            app.notch.bitmaps.clear();
            app.save_shelf();
        }
        Hit::ShelfClear => {
            if !app.shelf.items.is_empty() {
                app.notch.shelf_clear.set(0.0);
            }
        }
        Hit::TodoCheck(id) => {
            app.todos.toggle(id, now);
            app.save_todos();
            notch::schedule(app);
        }
        Hit::TodoTitle(id) => {
            if let Some(t) = app.todos.get(id) {
                let text = t.title.clone();
                let caret = text.chars().count();
                if app.notch.field.is_some() {
                    notch::commit_field(app);
                }
                app.notch.field = Some(TextField { target: EditTarget::Rename(id), text, caret });
            }
        }
        Hit::TodoColor(id) => {
            app.todos.cycle_color(id);
            app.save_todos();
        }
        Hit::TodoDue(id) => app.notch.due_edit = if app.notch.due_edit == Some(id) { None } else { Some(id) },
        Hit::TodoDelete(id) => {
            app.todos.remove(id);
            if app.notch.due_edit == Some(id) {
                app.notch.due_edit = None;
            }
            app.save_todos();
            notch::schedule(app);
        }
        Hit::TodoAdd | Hit::TodoField => {
            if !matches!(app.notch.field.as_ref().map(|f| &f.target), Some(EditTarget::NewTask)) {
                app.notch.field = Some(TextField { target: EditTarget::NewTask, text: String::new(), caret: 0 });
                app.notch.due_edit = None;
            } else if h == Hit::TodoAdd {
                notch::commit_field(app);
            }
        }
        Hit::NewColor => app.notch.new_color = (app.notch.new_color + 1) % store::TODO_COLORS.len(),
        Hit::DueChip(k) => apply_due(app, quick_due(k, now)),
        Hit::DueStep(dir) => {
            let current = current_due(app).unwrap_or_else(|| round_up(now, 15));
            apply_due(app, Some(current + dir as i64 * 15 * 60));
        }
        Hit::DueClear => apply_due(app, None),
        Hit::ShowDone => {
            let mut c = app.cfg.clone();
            c.todo_show_done = !c.todo_show_done;
            app.set_config(c);
        }
        Hit::CalPrev | Hit::CalNext => {
            let (y, m) = app.notch.cal_month;
            app.notch.cal_month = if h == Hit::CalPrev {
                if m == 1 { (y - 1, 12) } else { (y, m - 1) }
            } else if m == 12 {
                (y + 1, 1)
            } else {
                (y, m + 1)
            };
        }
        Hit::CalToday => {
            use chrono::Datelike;
            let t = notch::today();
            app.notch.cal_month = (t.year(), t.month());
            app.notch.cal_sel = t;
        }
        Hit::CalDay(d) => app.notch.cal_sel = d,
        Hit::CalAdd => {
            let day = app.notch.cal_sel;
            let due = if day == notch::today() { round_up(now + 3600, 15) } else { store::at_local(day, 9, 0) };
            app.notch.new_due = Some(due);
            app.notch.field = Some(TextField { target: EditTarget::NewTask, text: String::new(), caret: 0 });
            notch::set_page(app, Page::Todo);
        }
        Hit::AlertPrimary => alert_action(app, true),
        Hit::AlertSecondary => alert_action(app, false),
        Hit::AlertClose => app.notch.alert = None,
    }
    notch::request_frame(app);
}

fn drink(app: &mut App, now: i64) {
    let before = app.water.today(now);
    app.water.drink(now, app.cfg.glass_ml);
    app.save_water();
    app.notch.water_fill.kick(-1.2);
    if before < app.cfg.goal_ml && app.water.today(now) >= app.cfg.goal_ml {
        notch::notice(app, "Daily goal reached");
    }
    notch::schedule(app);
}

fn shelf_click(app: &mut App, i: usize, ctrl: bool, shift: bool) {
    let n = &mut app.notch;
    if shift {
        let a = n.shelf_anchor.unwrap_or(i);
        let (lo, hi) = (a.min(i), a.max(i));
        n.shelf_sel = (lo..=hi).collect();
        return;
    }
    if ctrl || n.shelf_selecting {
        if !n.shelf_sel.remove(&i) {
            n.shelf_sel.insert(i);
        }
        n.shelf_anchor = Some(i);
        return;
    }
    n.shelf_anchor = Some(i);
    match app.shelf.items.get(i).cloned() {
        Some(store::ShelfItem::File { path }) => crate::sys::open(&path),
        Some(store::ShelfItem::Text { text }) => {
            if crate::sys::copy_text(app.notch.hwnd, &text) {
                notch::notice(app, "Copied");
            }
        }
        None => {}
    }
}

fn current_due(app: &App) -> Option<i64> {
    match app.notch.due_edit {
        Some(id) => app.todos.get(id).and_then(|t| t.due),
        None => app.notch.new_due,
    }
}

fn apply_due(app: &mut App, due: Option<i64>) {
    match app.notch.due_edit {
        Some(id) => {
            app.todos.set_due(id, due);
            app.save_todos();
        }
        None => app.notch.new_due = due,
    }
    notch::schedule(app);
}

pub fn round_up(ts: i64, minutes: i64) -> i64 {
    let step = minutes * 60;
    (ts + step - 1) / step * step
}

pub const DUE_CHIPS: [&str; 4] = ["In 15 min", "In 1 hour", "This evening", "Tomorrow 9:00"];

fn quick_due(k: u8, now: i64) -> Option<i64> {
    let today = store::day_of(now);
    Some(match k {
        0 => round_up(now + 15 * 60, 5),
        1 => round_up(now + 3600, 5),
        2 => {
            let evening = store::at_local(today, 19, 0);
            if evening > now { evening } else { round_up(now + 3600, 15) }
        }
        _ => store::at_local(today.succ_opt().unwrap_or(today), 9, 0),
    })
}

fn alert_action(app: &mut App, primary: bool) {
    let Some((kind, _)) = app.notch.alert.take() else { return };
    let now = store::now();
    match kind {
        AlertKind::Todo(id) => {
            if primary {
                if app.todos.get(id).is_some_and(|t| !t.done) {
                    app.todos.toggle(id, now);
                }
            } else {
                let m = app.cfg.snooze_min();
                app.todos.snooze(id, now, m);
            }
            app.save_todos();
        }
        AlertKind::Water => {
            if primary {
                drink(app, now);
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
        AlertKind::LowBattery(_) => {}
    }
    notch::schedule(app);
}
