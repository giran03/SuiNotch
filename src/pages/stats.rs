//! The Stats page: this week's focus time, water goal days and finished tasks, with streaks.

use crate::app::App;
use crate::gfx::{icon, Align, Canvas, C, R};
use crate::stats::streak;
use crate::views::{accent, DIM};

const WATER: C = C(0.231, 0.655, 1.0, 1.0);
const FOCUS: C = C(1.0, 0.42, 0.353, 1.0);
const TASKS: C = C(0.204, 0.78, 0.349, 1.0);
const ROW_H: f32 = 54.0;
const LEFT_W: f32 = 236.0;

fn fmt_minutes(m: u32) -> String {
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn days(n: u32) -> String {
    format!("{n} day{}", if n == 1 { "" } else { "s" })
}

pub fn stats(cv: &mut Canvas, app: &mut App, area: R) {
    let today = crate::notch::today();
    let week = app.stats.week(today);
    let goal = app.cfg.goal_ml;
    let water_hit = |d| app.water.total_on(d) >= goal;

    let focus_week: u32 = week.iter().map(|(_, d)| d.focus_min).sum();
    let focus_streak = streak(today, |d| app.stats.day(d).sessions > 0);
    let water_days = week.iter().filter(|(d, _)| water_hit(*d)).count();
    let water_streak = streak(today, water_hit);
    let tasks_week: u32 = week.iter().map(|(_, d)| d.tasks_done).sum();
    let today_stats = week[6].1;

    let rows = [
        (icon::TIMER, FOCUS, fmt_minutes(focus_week), format!("focused this week  ·  {} streak", days(focus_streak))),
        ('\u{0}', WATER, format!("{water_days} of 7 days"), format!("water goal met  ·  {} streak", days(water_streak))),
        (icon::CHECK, TASKS, format!("{tasks_week} tasks"), format!("done this week  ·  {} today", today_stats.tasks_done)),
    ];
    for (i, (glyph, color, value, sub)) in rows.iter().enumerate() {
        let y = area.y + i as f32 * ROW_H;
        let (cx, cy) = (area.x + 17.0, y + 22.0);
        cv.fill_circle(cx, cy, 16.0, color.a(0.2));
        if *glyph == '\u{0}' {
            crate::pages::water_icon(cv, cx, cy, 15.0, *color);
        } else {
            cv.glyph(*glyph, cx, cy, 13.0, *color);
        }
        cv.display(value, R::new(area.x + 44.0, y + 4.0, LEFT_W - 44.0, 22.0), 15.5, 700, C::WHITE, Align::Left);
        cv.text(sub, R::new(area.x + 44.0, y + 25.0, LEFT_W - 40.0, 16.0), 11.5, 500, DIM, Align::Left);
    }

    // Focus minutes per day, with a drop under each day the water goal was met.
    let chart = R::new(area.x + LEFT_W + 24.0, area.y, area.right() - (area.x + LEFT_W + 24.0), area.h);
    cv.fill_rrect(chart.inset(-8.0, -6.0), 16.0, C::WHITE.a(0.04));
    cv.text("This week", R::new(chart.x + 6.0, chart.y, 120.0, 18.0), 12.0, 650, C::WHITE, Align::Left);
    cv.text("focus minutes", R::new(chart.right() - 126.0, chart.y, 120.0, 18.0), 11.0, 500, DIM, Align::Right);
    let peak = week.iter().map(|(_, d)| d.focus_min).max().unwrap_or(0).max(25) as f32;
    let (top, floor) = (chart.y + 34.0, chart.bottom() - 40.0);
    let slot = chart.w / 7.0;
    let accent = accent(app);
    for (k, (d, day)) in week.iter().enumerate() {
        let cx = chart.x + slot * (k as f32 + 0.5);
        let bw = (slot * 0.46).min(22.0);
        let full = floor - top;
        cv.fill_rrect(R::new(cx - bw / 2.0, top, bw, full), bw / 2.0, C::WHITE.a(0.05));
        if day.focus_min > 0 {
            let bh = (full * day.focus_min as f32 / peak).max(bw);
            let color = if *d == today { FOCUS } else { FOCUS.a(0.6) };
            cv.fill_rrect(R::new(cx - bw / 2.0, floor - bh, bw, bh), bw / 2.0, color);
            cv.text(&day.focus_min.to_string(), R::centered(cx, floor - bh - 9.0, slot, 14.0), 10.0, 600, DIM, Align::Center);
        }
        let label = d.format("%a").to_string();
        let fg = if *d == today { accent } else { DIM };
        cv.text(&label[..label.len().min(2)], R::centered(cx, floor + 12.0, slot, 14.0), 10.5, 650, fg, Align::Center);
        let hit = water_hit(*d);
        crate::pages::water_icon(cv, cx, floor + 28.0, 9.0, if hit { WATER } else { C::WHITE.a(0.14) });
    }
}
