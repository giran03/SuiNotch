//! Contents of the expanded notch pages. Each function draws inside `area` (DIPs) and
//! registers its hit regions; clicks are dispatched in `views::click`.

use chrono::{Datelike, Local, NaiveDate, Timelike};
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::Graphics::Imaging::{IWICBitmapSource, WICBitmapUsePremultipliedAlpha};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT};

use crate::alerts::EVENT_COLOR;
use crate::app::App;
use crate::gfx::{icon, Align, Canvas, C, R};
use crate::ical::CalEvent;
use crate::mascot;
use crate::media;
use crate::notch::{EditTarget, Hit};
use crate::store::{self, ShelfItem, Todo, TODO_COLORS};
use crate::sys;
use crate::views::{self, accent, bar, fmt_ms, hit, hovered, icon_button, pill_button, readable_on, DIM, FAINT, SUBTLE};

pub const TODO_ROW: f32 = 31.0;
pub const TILE_W: f32 = 96.0;

mod clips;
pub mod focus;
mod stats;

pub use focus::timer;
pub use stats::stats;

// ------------------------------------------------------------------ shared

/// Draws the current track artwork; returns false when there is none.
pub fn draw_artwork(cv: &mut Canvas, app: &mut App, r: R, radius: f32) -> bool {
    let Some((key, source)) = app.artwork.as_ref() else { return false };
    let key = *key;
    if app.notch.art.as_ref().map(|(k, _)| *k) != Some(key) {
        app.notch.art = cv.create_bitmap(source).map(|b| (key, b));
    }
    match app.notch.art.as_ref() {
        Some((_, bmp)) => {
            let bmp = bmp.clone();
            cv.bitmap_cover(&bmp, r, radius, 1.0);
            true
        }
        None => false,
    }
}

pub fn water_icon(cv: &Canvas, cx: f32, cy: f32, size: f32, c: C) {
    mascot::drop_shape(cv, cx, cy + size * 0.5, size * 0.72, size, c);
}

fn empty_state(cv: &Canvas, area: R, title: &str, sub: &str) {
    cv.display(title, R::new(area.x, area.cy() - 22.0, area.w, 22.0), 15.0, 650, C::WHITE, Align::Center);
    cv.text(sub, R::new(area.x, area.cy() + 2.0, area.w, 18.0), 12.0, 500, DIM, Align::Center);
}

// ------------------------------------------------------------------ music

pub fn music(cv: &mut Canvas, app: &mut App, area: R) {
    let Some(m) = app.media.as_ref() else {
        empty_state(cv, area, "Nothing playing", "Play something in Spotify, Apple Music or your browser");
        return;
    };
    let info = m.info.clone();
    let pos = app.notch.seek_drag.map(|f| (f * info.duration_ms as f32) as u64).unwrap_or_else(|| m.position_ms());
    let lyrics_view = app.notch.lyrics_view;
    let accent = accent(app);
    let tint = app.art_color.unwrap_or(accent);

    let art = R::new(area.x, area.y + 2.0, 100.0, 100.0);
    if !draw_artwork(cv, app, art, 14.0) {
        cv.fill_rrect(art, 14.0, C::WHITE.a(0.08));
        cv.glyph(icon::MUSIC, art.cx(), art.cy(), 30.0, SUBTLE);
    }
    let info_w = if lyrics_view { area.w - 118.0 - 268.0 } else { area.w - 118.0 };
    let x0 = art.right() + 18.0;
    cv.display(&info.title, R::new(x0, area.y + 2.0, info_w, 22.0), 16.0, 700, C::WHITE, Align::Left);
    cv.text(&info.artist, R::new(x0, area.y + 25.0, info_w, 18.0), 12.5, 500, DIM, Align::Left);
    // With several players open, always say which one the controls drive.
    if (app.cfg.show_source || info.sources > 1) && !info.app_id.is_empty() {
        cv.text(&media::source_name(&info.app_id), R::new(x0, area.y + 43.0, info_w, 16.0), 11.0, 500, C::WHITE.a(0.35), Align::Left);
    }

    // Progress
    let bar_r = R::new(x0, area.y + 68.0, info_w, 4.0);
    hit(app, bar_r.inset(0.0, -6.0), Hit::Seek);
    let frac = if info.duration_ms > 0 { pos as f32 / info.duration_ms as f32 } else { 0.0 };
    let thick = if hovered(app, Hit::Seek) || app.notch.seek_drag.is_some() { R::new(bar_r.x, bar_r.y - 1.0, bar_r.w, 6.0) } else { bar_r };
    bar(cv, thick, frac, C::WHITE);
    if hovered(app, Hit::Seek) || app.notch.seek_drag.is_some() {
        cv.fill_circle(bar_r.x + bar_r.w * frac.clamp(0.0, 1.0), bar_r.cy(), 5.5, C::WHITE);
    }
    let remain = info.duration_ms.saturating_sub(pos);
    cv.text(&fmt_ms(pos), R::new(x0, bar_r.bottom() + 4.0, 60.0, 14.0), 10.5, 600, DIM, Align::Left);
    cv.text(&format!("-{}", fmt_ms(remain)), R::new(bar_r.right() - 60.0, bar_r.bottom() + 4.0, 60.0, 14.0), 10.5, 600, DIM, Align::Right);

    // Controls
    let cy = area.bottom() - 16.0;
    let cx = x0 + info_w / 2.0;
    icon_button(cv, app, cx - 44.0, cy, 14.0, icon::PREV, Hit::Prev, C::WHITE);
    let play_hit = Hit::PlayPause;
    hit(app, R::centered(cx, cy, 40.0, 40.0), play_hit);
    let pr = if views::pressed(app, play_hit) { 17.0 } else if hovered(app, play_hit) { 19.5 } else { 18.5 };
    cv.fill_circle(cx, cy, pr, C::WHITE);
    cv.glyph(if info.playing { icon::PAUSE } else { icon::PLAY }, cx + if info.playing { 0.0 } else { 1.0 }, cy, 14.0, C::BLACK);
    icon_button(cv, app, cx + 44.0, cy, 14.0, icon::NEXT, Hit::Next, C::WHITE);
    let lyr_color = if lyrics_view { tint.mix(C::WHITE, 0.3) } else { DIM };
    icon_button(cv, app, x0 + 10.0, cy, 12.0, icon::LYRICS, Hit::LyricsToggle, lyr_color);
    if info.sources > 1 {
        source_toggle(cv, app, x0 + 40.0, cy, info.sources);
    }
    volume_control(cv, app, x0 + info_w - 10.0, cy);

    if lyrics_view {
        lyrics_panel(cv, app, R::new(area.right() - 252.0, area.y - 6.0, 252.0, area.h + 8.0), tint);
    }
}

/// Switch glyph with a count of open players; a click moves on to the next one.
fn source_toggle(cv: &Canvas, app: &mut App, cx: f32, cy: f32, sources: usize) {
    icon_button(cv, app, cx, cy, 12.0, icon::SWITCH, Hit::Source, DIM);
    let badge = R::centered(cx + 10.0, cy - 8.0, 13.0, 13.0);
    cv.fill_circle(badge.cx(), badge.cy(), 6.5, C::WHITE.a(0.16));
    cv.text(&sources.min(9).to_string(), badge, 9.0, 700, C::WHITE.a(0.8), Align::Center);
}

fn volume_control(cv: &mut Canvas, app: &mut App, x: f32, cy: f32) {
    let Some(audio) = app.audio.as_ref() else { return };
    let (level, muted) = audio.get();
    let open = app.notch.volume_open.value.clamp(0.0, 1.0);
    let w = 96.0 * open;
    if open > 0.02 {
        let r = R::new(x - 16.0 - w, cy - 2.5, w, 5.0);
        cv.push_opacity(open);
        bar(cv, r, if muted { 0.0 } else { level }, C::WHITE);
        cv.fill_circle(r.x + r.w * if muted { 0.0 } else { level }, r.cy(), 5.0, C::WHITE);
        cv.pop();
        hit(app, R::new(r.x - 6.0, cy - 12.0, r.w + 12.0, 24.0), Hit::VolumeBar);
    }
    let g = if muted || level <= 0.001 { icon::MUTE } else { icon::VOLUME };
    icon_button(cv, app, x, cy, 12.0, g, Hit::Volume, DIM);
}

fn lyrics_panel(cv: &mut Canvas, app: &mut App, r: R, tint: C) {
    cv.fill_rrect(r, 16.0, C::WHITE.a(0.05));
    let inner = r.inset(16.0, 10.0);
    let key = app.media.as_ref().map(|m| m.info.track_key()).unwrap_or(0);
    if app.lyrics_key != key {
        cv.text("Looking for lyrics...", inner, 12.5, 500, DIM, Align::Center);
        return;
    }
    if app.lyrics.is_empty() {
        cv.text("No lyrics for this song", R::new(inner.x, inner.cy() - 22.0, inner.w, 20.0), 12.5, 500, DIM, Align::Center);
        let w = app.gfx.text_width("Search again", 12.5, 600) + 28.0;
        pill_button(cv, app, R::centered(inner.cx(), inner.cy() + 14.0, w, 26.0), "Search again", Hit::LyricsRetry, false);
        return;
    }
    let cur = app.current_line();
    let center = cur.unwrap_or(0) as isize;
    let row = 27.0;
    let mid = inner.cy() - row / 2.0;
    cv.push_clip_rrect(r, 16.0, 1.0);
    for k in -3isize..=3 {
        let i = center + k;
        if i < 0 || i as usize >= app.lyrics.len() {
            continue;
        }
        let line = app.lyrics[i as usize].clone();
        let y = mid + k as f32 * row;
        let is_cur = cur == Some(i as usize);
        let fade = 1.0 - (k.abs() as f32 / 4.0);
        let lr = R::new(inner.x, y, inner.w, row);
        hit(app, lr, Hit::LyricLine(line.at_ms));
        if hovered(app, Hit::LyricLine(line.at_ms)) {
            cv.fill_rrect(lr.inset(-6.0, 1.0), 8.0, FAINT);
        }
        let text = if line.text.is_empty() { "\u{266A}" } else { &line.text };
        let (size, weight, color) = if is_cur { (14.0, 700, tint.mix(C::WHITE, 0.45)) } else { (12.5, 500, C::WHITE.a(0.42 * fade)) };
        cv.text(text, lr, size, weight, color, Align::Left);
    }
    cv.pop();
    let (mx, my) = app.notch.mouse;
    if r.contains(mx, my) {
        lyrics_tools(cv, app, r);
    }
}

/// Shown while the pointer is over the lyrics: nudge timing for this song, or look again.
fn lyrics_tools(cv: &Canvas, app: &mut App, r: R) {
    let shift = app.media.as_ref().map(|m| app.shifts.get(m.info.track_key())).unwrap_or(0);
    let bar = R::new(r.x + 8.0, r.bottom() - 34.0, r.w - 16.0, 28.0);
    cv.fill_rrect(bar, 14.0, C(0.09, 0.095, 0.11, 0.96));
    icon_button(cv, app, bar.x + 16.0, bar.cy(), 8.0, '\u{E738}', Hit::LyricsEarlier, C::WHITE);
    let label = if shift == 0 { "In sync".to_string() } else { format!("{:+.1} s", shift as f32 / 1000.0) };
    cv.text(&label, R::new(bar.x + 28.0, bar.y, 58.0, bar.h), 11.0, 650, DIM, Align::Center);
    icon_button(cv, app, bar.x + 98.0, bar.cy(), 8.0, icon::ADD, Hit::LyricsLater, C::WHITE);
    let w = app.gfx.text_width("Search again", 12.5, 600) + 24.0;
    pill_button(cv, app, R::new(bar.right() - w - 3.0, bar.y + 3.0, w, bar.h - 6.0), "Search again", Hit::LyricsRetry, false);
}

// ------------------------------------------------------------------ water

pub fn water(cv: &mut Canvas, app: &mut App, area: R) {
    let now = store::now();
    let today = app.water.today(now);
    let goal = app.cfg.goal_ml.max(1);
    let frac = today as f32 / goal as f32;
    app.notch.water_fill.set(frac.min(1.0));
    let blue = C::hex(0x3BA7FF).mix(accent(app), 0.35);

    // Droplet gauge
    let (dx, base, dw, dh) = (area.x + 42.0, area.y + 112.0, 70.0, 104.0);
    mascot::drop_shape(cv, dx, base, dw, dh, C::WHITE.a(0.08));
    let fill = app.notch.water_fill.value.clamp(0.0, 1.05);
    if fill > 0.002 {
        let top = base - dh * fill.min(1.0);
        cv.push_clip_rrect(R::new(dx - dw, top, dw * 2.0, base - top + 2.0), 0.0, 1.0);
        mascot::drop_shape(cv, dx, base, dw, dh, blue);
        cv.pop();
        let wave = (app.started.elapsed().as_secs_f32() * 2.0).sin() * 1.5;
        cv.fill_ellipse(dx, top + wave, dw * 0.32, 2.0, C::WHITE.a(0.25));
    }

    let x0 = area.x + 98.0;
    let litres = format!("{:.2} / {:.1} L", today as f32 / 1000.0, goal as f32 / 1000.0);
    cv.display(&litres, R::new(x0, area.y + 2.0, 190.0, 30.0), 22.0, 700, C::WHITE, Align::Left);
    let status = if today >= goal { "Today · Goal reached".to_string() } else { format!("Today · {}%", (frac * 100.0).round() as u32) };
    cv.text(&status, R::new(x0, area.y + 34.0, 190.0, 18.0), 12.5, 600, if today >= goal { blue } else { DIM }, Align::Left);
    let next = if !app.cfg.water_enabled {
        "Reminders are off".to_string()
    } else {
        let c = &app.cfg;
        match app.water.next_reminder(now, c.water_start_hour, c.water_end_hour, c.water_interval_min, c.goal_ml) {
            Some(t) => format!("Next reminder {}", sys::format_time(&store::local(t), app.cfg.clock_format)),
            None => "No more reminders today".to_string(),
        }
    };
    cv.text(&next, R::new(x0, area.y + 54.0, 200.0, 16.0), 11.5, 500, C::WHITE.a(0.4), Align::Left);
    let label = format!("Drink {} ml", app.cfg.glass_ml);
    pill_button(cv, app, R::new(x0, area.bottom() - 34.0, 132.0, 32.0), &label, Hit::Drink, true);
    icon_button(cv, app, x0 + 158.0, area.bottom() - 18.0, 12.0, icon::UNDO, Hit::WaterUndo, DIM);

    // Hourly chart
    let chart = R::new(area.x + 320.0, area.y + 4.0, area.w - 320.0, area.h - 26.0);
    let hours = app.water.by_hour(now);
    let (h0, h1) = (app.cfg.water_start_hour.min(23) as usize, (app.cfg.water_end_hour.max(app.cfg.water_start_hour + 1)).min(24) as usize);
    let span = (h1 - h0).max(1);
    let peak = hours[h0..h1].iter().copied().max().unwrap_or(0).max(app.cfg.glass_ml * 2);
    let scale_top = ((peak + 99) / 100 * 100) as f32;
    cv.text(&format!("{} ml", scale_top as u32), R::new(chart.x, chart.y - 4.0, 60.0, 14.0), 10.0, 500, C::WHITE.a(0.35), Align::Left);
    let gx = chart.x + 4.0;
    let gw = chart.w - 4.0;
    let slot = gw / span as f32;
    let current = Local::now().hour() as usize;
    for (k, h) in (h0..h1).enumerate() {
        let v = hours[h] as f32 / scale_top;
        let bh = (chart.h - 18.0) * v;
        let x = gx + k as f32 * slot + slot * 0.18;
        let bw = (slot * 0.64).max(3.0);
        let floor = chart.bottom();
        cv.fill_rrect(R::new(x, floor - (chart.h - 18.0), bw, chart.h - 18.0), bw / 2.0, C::WHITE.a(0.05));
        if bh > 0.5 {
            cv.fill_rrect(R::new(x, floor - bh.max(bw), bw, bh.max(bw)), bw / 2.0, if h == current { blue } else { blue.a(0.55) });
        }
        if (h - h0) % 3 == 0 {
            cv.text(&format!("{h:02}"), R::new(x - 8.0, floor + 4.0, bw + 16.0, 14.0), 10.0, 500, C::WHITE.a(0.4), Align::Center);
        }
    }
}

// ------------------------------------------------------------------ shelf

fn thumbnail(cv: &Canvas, app: &mut App, path: &std::path::Path) -> Option<ID2D1Bitmap> {
    let key = path.display().to_string();
    if let Some(b) = app.notch.bitmaps.get(&key) {
        return b.clone();
    }
    let bmp = (|| unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let hbmp = factory.GetImage(SIZE { cx: 96, cy: 96 }, SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK).ok()?;
        let wic = app.gfx.wic.CreateBitmapFromHBITMAP(hbmp, windows::Win32::Graphics::Gdi::HPALETTE::default(), WICBitmapUsePremultipliedAlpha);
        let _ = DeleteObject(hbmp.into());
        let source: IWICBitmapSource = wic.ok()?.cast().ok()?;
        let source = app.gfx.to_pbgra(&source)?;
        cv.create_bitmap(&source)
    })();
    app.notch.bitmaps.insert(key, bmp.clone());
    bmp
}

pub fn shelf(cv: &mut Canvas, app: &mut App, area: R) {
    if app.notch.shelf_clear.target == 0.0 && app.notch.shelf_clear.value < 0.04 {
        app.shelf.items.clear();
        app.notch.shelf_sel.clear();
        app.notch.bitmaps.clear();
        app.notch.shelf_clear.snap(1.0);
        app.notch.shelf_scroll.snap(0.0);
        app.save_shelf();
    }
    let count = app.shelf.items.len();
    let accent = accent(app);
    if app.cfg.clipboard_history && !app.notch.drag_over {
        clips::tabs(cv, app, area);
        if app.notch.shelf_tab == 1 {
            clips::clipboard(cv, app, area);
            return;
        }
    } else {
        let header = format!("{count} item{}", if count == 1 { "" } else { "s" });
        cv.text(&header, R::new(area.x, area.y - 2.0, 160.0, 20.0), 12.0, 500, DIM, Align::Left);
    }
    if count > 0 && !app.notch.drag_over {
        let mut x = area.right();
        let clear_w = 62.0;
        x -= clear_w;
        pill_button(cv, app, R::new(x, area.y - 4.0, clear_w, 24.0), "Clear", Hit::ShelfClear, false);
        let sel_label = if app.notch.shelf_selecting { "Done" } else { "Select" };
        let sw = 66.0;
        x -= sw + 6.0;
        pill_button(cv, app, R::new(x, area.y - 4.0, sw, 24.0), sel_label, Hit::ShelfSelect, app.notch.shelf_selecting);
        if !app.notch.shelf_sel.is_empty() {
            let label = format!("Remove {}", app.notch.shelf_sel.len());
            let rw = app.gfx.text_width(&label, 12.5, 600) + 26.0;
            x -= rw + 6.0;
            pill_button(cv, app, R::new(x, area.y - 4.0, rw, 24.0), &label, Hit::ShelfRemove, false);
        }
    }
    let list = R::new(area.x, area.y + 28.0, area.w, area.h - 28.0);
    if count == 0 || app.notch.drag_over {
        let zone = list.inset(0.0, 2.0);
        let hot = app.notch.drag_over;
        cv.fill_rrect(zone, 16.0, if hot { accent.a(0.16) } else { C::WHITE.a(0.04) });
        dashed_rrect(cv, zone, 16.0, if hot { accent } else { C::WHITE.a(0.22) });
        let (title, sub) = if hot { ("Release to add", "Files stay where they are") } else { ("Drop files here", "Drag out anytime  ·  click to open") };
        cv.glyph(icon::FOLDER, zone.cx(), zone.cy() - 22.0, 18.0, if hot { accent } else { DIM });
        cv.text(title, R::new(zone.x, zone.cy() - 6.0, zone.w, 20.0), 13.0, 650, C::WHITE, Align::Center);
        cv.text(sub, R::new(zone.x, zone.cy() + 13.0, zone.w, 16.0), 11.5, 500, DIM, Align::Center);
        if hot {
            return;
        }
        if count == 0 {
            return;
        }
    }
    let scroll = app.notch.shelf_scroll.value;
    let squeeze = app.notch.shelf_clear.value.clamp(0.0, 1.0);
    cv.push_clip_rrect(list.inset(-2.0, -2.0), 12.0, 1.0);
    let items = app.shelf.items.clone();
    for (i, item) in items.iter().enumerate() {
        let x = list.x + i as f32 * (TILE_W + 10.0) - scroll;
        if x > list.right() || x + TILE_W < list.x {
            continue;
        }
        let stagger = (squeeze * 1.6 - i as f32 * 0.04).clamp(0.0, 1.0);
        let tile = R::new(x, list.y + (1.0 - stagger) * 30.0, TILE_W, list.h);
        let h = Hit::ShelfItem(i);
        hit(app, tile, h);
        let selected = app.notch.shelf_sel.contains(&i);
        cv.push_opacity(stagger);
        let bg = if selected { accent.a(0.22) } else if hovered(app, h) { C::WHITE.a(0.1) } else { C::WHITE.a(0.05) };
        cv.fill_rrect(tile, 14.0, bg);
        if selected {
            cv.stroke_rrect(tile.inset(0.5, 0.5), 14.0, accent, 1.5);
        }
        match item {
            ShelfItem::File { path } => {
                let thumb = R::centered(tile.cx(), tile.y + 34.0, 46.0, 46.0);
                match thumbnail(cv, app, path) {
                    Some(b) => cv.bitmap_cover(&b, thumb, 8.0, 1.0),
                    None => cv.glyph(if path.is_dir() { icon::FOLDER } else { icon::DOCUMENT }, thumb.cx(), thumb.cy(), 24.0, DIM),
                }
                cv.text(&sys::file_name(path), R::new(tile.x + 8.0, tile.y + 62.0, tile.w - 16.0, 16.0), 11.5, 600, C::WHITE, Align::Center);
                let meta = if path.is_dir() {
                    "Folder".to_string()
                } else if app.cfg.shelf_show_sizes {
                    sys::file_size(path).map(sys::human_size).unwrap_or_else(|| "Missing".to_string())
                } else {
                    String::new()
                };
                cv.text(&meta, R::new(tile.x + 8.0, tile.y + 78.0, tile.w - 16.0, 14.0), 10.5, 500, DIM, Align::Center);
            }
            ShelfItem::Text { text } => {
                cv.glyph(icon::TEXT, tile.x + 18.0, tile.y + 18.0, 11.0, accent);
                cv.text("Text", R::new(tile.x + 30.0, tile.y + 10.0, 60.0, 16.0), 10.5, 650, DIM, Align::Left);
                let preview: String = text.chars().take(90).collect();
                cv.wrapped(&preview, R::new(tile.x + 10.0, tile.y + 32.0, tile.w - 20.0, tile.h - 40.0), 11.0, 500, C::WHITE.a(0.85), Align::Left);
            }
        }
        if app.notch.shelf_selecting || selected {
            let (bx, by) = (tile.right() - 14.0, tile.y + 14.0);
            cv.fill_circle(bx, by, 8.0, if selected { accent } else { C::BLACK.a(0.5) });
            cv.stroke_circle(bx, by, 8.0, C::WHITE.a(0.7), 1.2);
            if selected {
                cv.glyph(icon::CHECK, bx, by, 8.0, readable_on(accent));
            }
        }
        cv.pop();
    }
    cv.pop();
}

fn dashed_rrect(cv: &Canvas, r: R, radius: f32, c: C) {
    // Dashes along the straight edges only; rounded corners stay solid for a calm look.
    let dash = 7.0;
    let gap = 6.0;
    let mut x = r.x + radius;
    while x < r.right() - radius {
        let e = (x + dash).min(r.right() - radius);
        cv.line(x, r.y, e, r.y, c, 1.4);
        cv.line(x, r.bottom(), e, r.bottom(), c, 1.4);
        x += dash + gap;
    }
    let mut y = r.y + radius;
    while y < r.bottom() - radius {
        let e = (y + dash).min(r.bottom() - radius);
        cv.line(r.x, y, r.x, e, c, 1.4);
        cv.line(r.right(), y, r.right(), e, c, 1.4);
        y += dash + gap;
    }
    use std::f32::consts::FRAC_PI_2 as Q;
    for (cx, cy, a) in [
        (r.x + radius, r.y + radius, 3.0 * Q),
        (r.right() - radius, r.y + radius, 0.0),
        (r.right() - radius, r.bottom() - radius, Q),
        (r.x + radius, r.bottom() - radius, 2.0 * Q),
    ] {
        cv.arc(cx, cy, radius, a, a + Q, c, 1.4);
    }
}

// ------------------------------------------------------------------ to-do

pub fn visible_todos(app: &App) -> Vec<Todo> {
    let mut v: Vec<Todo> = app.todos.items.iter().filter(|t| app.cfg.todo_show_done || !t.done).cloned().collect();
    v.sort_by_key(|t| (t.done, t.due.is_none(), t.due.unwrap_or(i64::MAX)));
    v
}

fn due_label(app: &App, due: i64, now: i64) -> String {
    let dt = store::local(due);
    let time = sys::format_time(&dt, app.cfg.clock_format);
    let day = store::day_of(due);
    let today = store::day_of(now);
    if day == today {
        time
    } else if Some(day) == today.succ_opt() {
        format!("Tomorrow {time}")
    } else {
        format!("{} {time}", dt.format("%a %-d"))
    }
}

pub fn todo(cv: &mut Canvas, app: &mut App, area: R) {
    let now = store::now();
    let (done, total) = app.todos.today_progress(now);
    let accent = accent(app);
    cv.text(&format!("{done}/{total} done today"), R::new(area.x, area.y - 2.0, 180.0, 20.0), 12.0, 500, DIM, Align::Left);
    let eye = if app.cfg.todo_show_done { "Hide done" } else { "Show done" };
    pill_button(cv, app, R::new(area.right() - 200.0, area.y - 4.0, 90.0, 24.0), eye, Hit::ShowDone, false);
    pill_button(cv, app, R::new(area.right() - 102.0, area.y - 4.0, 102.0, 24.0), "+ Add task", Hit::TodoAdd, false);

    let editing_due = app.notch.due_edit.is_some() || matches!(app.notch.field.as_ref().map(|f| &f.target), Some(EditTarget::NewTask));
    let bottom_h = if editing_due { 72.0 } else { 36.0 };
    let list = R::new(area.x - 6.0, area.y + 26.0, area.w + 12.0, area.h - 26.0 - bottom_h);
    let todos = visible_todos(app);
    if todos.is_empty() {
        let msg = if app.todos.items.is_empty() { "Nothing to do yet. Add your first task." } else { "All done. Nice work." };
        cv.text(msg, list, 12.5, 500, DIM, Align::Center);
    }
    let max_scroll = views::todo_overflow(app);
    if app.notch.todo_scroll.target > max_scroll {
        app.notch.todo_scroll.set(max_scroll);
    }
    let scroll = app.notch.todo_scroll.value;
    cv.push_clip_rrect(list, 10.0, 1.0);
    for (k, t) in todos.iter().enumerate() {
        let y = list.y + k as f32 * TODO_ROW - scroll;
        if y + TODO_ROW < list.y || y > list.bottom() {
            continue;
        }
        let row = R::new(list.x, y, list.w, TODO_ROW - 3.0);
        let color = C::hex(TODO_COLORS[t.color % TODO_COLORS.len()]);
        // Registered first so the controls on top win; the gaps between them still count as the row.
        let row_hit = R::new(row.x, row.y.max(list.y), row.w, row.bottom().min(list.bottom()) - row.y.max(list.y));
        hit(app, row_hit, Hit::TodoRow(t.id));
        let row_hover = app.notch.hover.is_some_and(|h| {
            matches!(h, Hit::TodoRow(i) | Hit::TodoTitle(i) | Hit::TodoCheck(i) | Hit::TodoDue(i) | Hit::TodoDelete(i) | Hit::TodoColor(i) if i == t.id)
        });
        // A task being edited (due picker or rename) keeps its controls even when the pointer leaves it.
        let renaming = app.notch.field.as_ref().is_some_and(|f| f.target == EditTarget::Rename(t.id));
        let selected = app.notch.due_edit == Some(t.id) || renaming;
        let show_controls = row_hover || selected;
        if show_controls {
            cv.fill_rrect(row, 9.0, C::WHITE.a(0.06));
        }
        // Colour tag
        let tag = R::new(row.x + 6.0, row.y + 6.0, 4.0, row.h - 12.0);
        hit(app, tag.inset(-4.0, 0.0), Hit::TodoColor(t.id));
        cv.fill_rrect(tag, 2.0, color);
        // Checkbox
        let (cx, cy) = (row.x + 26.0, row.cy());
        hit(app, R::centered(cx, cy, 24.0, 24.0), Hit::TodoCheck(t.id));
        if t.done {
            cv.fill_circle(cx, cy, 8.0, color);
            cv.glyph(icon::CHECK, cx, cy, 9.0, readable_on(color));
        } else {
            cv.stroke_circle(cx, cy, 7.5, if hovered(app, Hit::TodoCheck(t.id)) { color } else { color.a(0.7) }, 1.6);
        }
        // Due chip
        let overdue = !t.done && t.due.is_some_and(|d| d <= now);
        let due_text = t.due.map(|d| due_label(app, d, now)).unwrap_or_else(|| if show_controls { "+ Time".into() } else { String::new() });
        let dw = if due_text.is_empty() { 0.0 } else { app.gfx.text_width(&due_text, 11.0, 600) + 18.0 };
        let del_x = row.right() - 16.0;
        let chip = R::new(del_x - 18.0 - dw, row.cy() - 10.0, dw, 20.0);
        if dw > 0.0 {
            hit(app, chip, Hit::TodoDue(t.id));
            let bg = if overdue { C::hex(0xFF6B5A).a(0.22) } else if hovered(app, Hit::TodoDue(t.id)) { SUBTLE } else { FAINT };
            cv.fill_rrect(chip, 10.0, bg);
            cv.text(&due_text, chip, 11.0, 600, if overdue { C::hex(0xFF8A7A) } else { DIM }, Align::Center);
        }
        if show_controls {
            icon_button(cv, app, del_x, row.cy(), 9.0, icon::DELETE, Hit::TodoDelete(t.id), DIM);
        }
        // Title or inline editor
        let title_r = R::new(row.x + 42.0, row.y, (chip.x - 8.0) - (row.x + 42.0), row.h);
        let renaming = app.notch.field.as_ref().filter(|f| f.target == EditTarget::Rename(t.id)).cloned();
        match renaming {
            Some(f) => views::draw_field(cv, app, title_r.inset(0.0, 2.0), &f, "Task name"),
            None => {
                hit(app, title_r, Hit::TodoTitle(t.id));
                let col = if t.done { C::WHITE.a(0.38) } else { C::WHITE };
                cv.text(&t.title, title_r, 13.0, 550, col, Align::Left);
                if t.done {
                    let tw = app.gfx.text_width(&t.title, 13.0, 550).min(title_r.w);
                    cv.line(title_r.x, title_r.cy() + 0.5, title_r.x + tw, title_r.cy() + 0.5, C::WHITE.a(0.38), 1.2);
                }
            }
        }
    }
    cv.pop();

    // Bottom: add field and due picker
    let mut y = area.bottom() - 30.0;
    if editing_due {
        due_picker(cv, app, R::new(area.x, area.bottom() - 66.0, area.w, 28.0), now);
    }
    let field = app.notch.field.clone().filter(|f| f.target == EditTarget::NewTask);
    let fr = R::new(area.x + 30.0, y, area.w - 30.0, 30.0);
    let color = C::hex(TODO_COLORS[app.notch.new_color % TODO_COLORS.len()]);
    icon_button(cv, app, area.x + 11.0, y + 15.0, 7.0, '\u{25CF}', Hit::NewColor, color);
    cv.fill_circle(area.x + 11.0, y + 15.0, 6.0, color);
    match field {
        Some(f) => {
            let hint = match app.notch.new_due {
                Some(d) => format!("New task  ·  reminder {}", due_label(app, d, now)),
                None => "New task, press Enter to add".to_string(),
            };
            views::draw_field(cv, app, fr, &f, &hint);
        }
        None => {
            hit(app, fr, Hit::TodoField);
            cv.fill_rrect(fr, 9.0, if hovered(app, Hit::TodoField) { C::WHITE.a(0.09) } else { C::WHITE.a(0.05) });
            cv.text("Add a task...", fr.inset(12.0, 0.0), 12.5, 500, C::WHITE.a(0.35), Align::Left);
        }
    }
    y += 0.0;
    let _ = (y, accent);
}

fn due_picker(cv: &mut Canvas, app: &mut App, r: R, now: i64) {
    let mut x = r.x;
    for (k, label) in views::DUE_CHIPS.iter().enumerate() {
        let w = app.gfx.text_width(label, 11.5, 600) + 22.0;
        pill_chip(cv, app, R::new(x, r.y, w, r.h), label, Hit::DueChip(k as u8));
        x += w + 6.0;
    }
    let current = match app.notch.due_edit {
        Some(id) => app.todos.get(id).and_then(|t| t.due),
        None => app.notch.new_due,
    };
    let label = current.map(|d| due_label(app, d, now)).unwrap_or_else(|| "No time".to_string());
    let right = r.right();
    icon_button(cv, app, right - 12.0, r.cy(), 9.0, icon::CLOSE, Hit::DueClear, DIM);
    icon_button(cv, app, right - 40.0, r.cy(), 9.0, icon::ADD, Hit::DueStep(1), C::WHITE);
    let lw = app.gfx.text_width(&label, 12.0, 650) + 16.0;
    cv.text(&label, R::new(right - 56.0 - lw, r.y, lw, r.h), 12.0, 650, C::WHITE, Align::Center);
    icon_button(cv, app, right - 72.0 - lw, r.cy(), 9.0, '\u{E738}', Hit::DueStep(-1), C::WHITE);
}

fn pill_chip(cv: &Canvas, app: &mut App, r: R, label: &str, h: Hit) {
    hit(app, r, h);
    cv.fill_rrect(r, r.h / 2.0, if hovered(app, h) { SUBTLE } else { FAINT });
    cv.text(label, r, 11.5, 600, C::WHITE, Align::Center);
}

// ------------------------------------------------------------------ calendar

const AGENDA_ROWS: usize = 6;

/// Calendar events that touch a local day.
fn events_on(app: &App, day: NaiveDate) -> impl Iterator<Item = &CalEvent> {
    let from = store::at_local(day, 0, 0);
    let to = store::at_local(day.succ_opt().unwrap_or(day), 0, 0);
    app.calendar.iter().filter(move |e| e.start < to && (e.end > from || e.start >= from))
}

const WEEKDAYS_MON: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const WEEKDAYS_SUN: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

pub fn calendar(cv: &mut Canvas, app: &mut App, area: R) {
    let accent = accent(app);
    let (year, month) = app.notch.cal_month;
    let grid_w = 300.0;
    cv.display(&sys::month_name(year, month), R::new(area.x, area.y - 4.0, 200.0, 22.0), 15.0, 700, C::WHITE, Align::Left);
    icon_button(cv, app, area.x + grid_w - 52.0, area.y + 7.0, 10.0, icon::LEFT, Hit::CalPrev, C::WHITE);
    icon_button(cv, app, area.x + grid_w - 12.0, area.y + 7.0, 10.0, icon::RIGHT, Hit::CalNext, C::WHITE);
    icon_button(cv, app, area.x + grid_w - 32.0, area.y + 7.0, 4.0, '\u{25CF}', Hit::CalToday, accent);

    let monday = app.cfg.week_starts_monday;
    let names = if monday { WEEKDAYS_MON } else { WEEKDAYS_SUN };
    let cw = grid_w / 7.0;
    let top = area.y + 26.0;
    for (i, n) in names.iter().enumerate() {
        cv.text(n, R::new(area.x + i as f32 * cw, top, cw, 16.0), 10.5, 600, C::WHITE.a(0.4), Align::Center);
    }
    let cells = store::month_grid(year, month, monday);
    let rows = cells.len() / 7;
    let ch = ((area.bottom() - top - 18.0) / rows as f32).min(28.0);
    let today = crate::notch::today();
    for (k, cell) in cells.iter().enumerate() {
        let Some(d) = cell else { continue };
        let (col, row) = (k % 7, k / 7);
        let r = R::new(area.x + col as f32 * cw, top + 18.0 + row as f32 * ch, cw, ch);
        let h = Hit::CalDay(*d);
        hit(app, r, h);
        let (cx, cy) = (r.cx(), r.cy() - 1.0);
        let selected = *d == app.notch.cal_sel;
        if selected {
            cv.fill_circle(cx, cy, 11.5, accent);
        } else if hovered(app, h) {
            cv.fill_circle(cx, cy, 11.5, FAINT);
        }
        if *d == today && !selected {
            cv.stroke_circle(cx, cy, 11.5, accent, 1.4);
        }
        let weekend = d.weekday().number_from_monday() >= 6;
        let fg = if selected { readable_on(accent) } else if weekend { DIM } else { C::WHITE };
        cv.text(&d.day().to_string(), R::centered(cx, cy, 24.0, 18.0), 11.5, 600, fg, Align::Center);
        if app.cfg.calendar_dots {
            let on_dot = readable_on(accent);
            let dots = [(app.todos.has_tasks_on(*d), if selected { on_dot } else { accent }), (events_on(app, *d).next().is_some(), if selected { on_dot } else { EVENT_COLOR })];
            let n = dots.iter().filter(|(on, _)| *on).count() as f32;
            let mut dx = cx - (n - 1.0) * 2.5;
            for (on, color) in dots {
                if on {
                    cv.fill_circle(dx, cy + 9.5, 1.8, color);
                    dx += 5.0;
                }
            }
        }
    }

    // Day agenda
    let px = area.x + grid_w + 24.0;
    let panel = R::new(px, area.y - 6.0, area.right() - px, area.h + 8.0);
    cv.fill_rrect(panel, 16.0, C::WHITE.a(0.05));
    let sel = app.notch.cal_sel;
    let title = if sel == today { "Today".to_string() } else { sel.format("%A %-d %B").to_string() };
    cv.display(&title, R::new(panel.x + 16.0, panel.y + 10.0, panel.w - 70.0, 20.0), 13.5, 700, C::WHITE, Align::Left);
    pill_button(cv, app, R::new(panel.right() - 66.0, panel.y + 9.0, 54.0, 22.0), "+ Add", Hit::CalAdd, false);
    let events: Vec<CalEvent> = events_on(app, sel).cloned().collect();
    let tasks: Vec<Todo> = app.todos.on_day(sel).into_iter().cloned().collect();
    if tasks.is_empty() && events.is_empty() {
        cv.text("Nothing on this day", R::new(panel.x, panel.y + 40.0, panel.w, panel.h - 50.0), 12.0, 500, DIM, Align::Center);
        return;
    }
    let mut y = panel.y + 40.0;
    let day_start = store::at_local(sel, 0, 0);
    for e in events.iter().take(AGENDA_ROWS) {
        let r = R::new(panel.x + 10.0, y, panel.w - 20.0, 26.0);
        cv.fill_rrect(R::new(r.x + 10.0, r.y + 6.0, 4.0, r.h - 12.0), 2.0, EVENT_COLOR);
        let time = if e.all_day || e.start < day_start { "All day".to_string() } else { sys::format_time(&store::local(e.start), app.cfg.clock_format) };
        cv.text(&time, R::new(r.x + 26.0, r.y, 64.0, r.h), 11.5, 600, DIM, Align::Left);
        cv.text(&e.title, R::new(r.x + 90.0, r.y, r.w - 90.0, r.h), 12.5, 550, C::WHITE, Align::Left);
        y += 27.0;
    }
    for t in tasks.iter().take(AGENDA_ROWS.saturating_sub(events.len())) {
        let r = R::new(panel.x + 10.0, y, panel.w - 20.0, 26.0);
        let color = C::hex(TODO_COLORS[t.color % TODO_COLORS.len()]);
        let h = Hit::TodoCheck(t.id);
        hit(app, R::centered(r.x + 12.0, r.cy(), 22.0, 22.0), h);
        if t.done {
            cv.fill_circle(r.x + 12.0, r.cy(), 6.5, color);
        } else {
            cv.stroke_circle(r.x + 12.0, r.cy(), 6.0, color, 1.5);
        }
        let time = t.due.map(|d| sys::format_time(&store::local(d), app.cfg.clock_format)).unwrap_or_default();
        cv.text(&time, R::new(r.x + 26.0, r.y, 64.0, r.h), 11.5, 600, DIM, Align::Left);
        cv.text(&t.title, R::new(r.x + 90.0, r.y, r.w - 90.0, r.h), 12.5, 550, if t.done { C::WHITE.a(0.4) } else { C::WHITE }, Align::Left);
        y += 27.0;
    }
}
