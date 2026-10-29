//! The Clipboard page: recent texts and images copied anywhere, newest first.
//! Click to copy again, drag to drop anywhere.

use crate::app::App;
use crate::clips::Clip;
use crate::gfx::{icon, Align, Canvas, C, R};
use crate::notch::Hit;
use crate::views::{accent, hit, hovered, pill_button, DIM};

use super::{thumbnail, TILE_W};

pub fn clipboard(cv: &mut Canvas, app: &mut App, area: R) {
    let accent = accent(app);
    let count = app.clips.items.len();
    let header = format!("{count} item{}  ·  click to copy, drag to drop", if count == 1 { "" } else { "s" });
    cv.text(&header, R::new(area.x, area.y - 2.0, area.w - 80.0, 20.0), 12.0, 500, DIM, Align::Left);
    if !app.clips.items.is_empty() {
        pill_button(cv, app, R::new(area.right() - 62.0, area.y - 4.0, 62.0, 24.0), "Clear", Hit::ClipClear, false);
    }
    let list = R::new(area.x, area.y + 28.0, area.w, area.h - 28.0);
    if app.clips.items.is_empty() {
        let zone = list.inset(0.0, 2.0);
        cv.fill_rrect(zone, 16.0, C::WHITE.a(0.04));
        cv.glyph(icon::CLIPBOARD, zone.cx(), zone.cy() - 22.0, 18.0, DIM);
        cv.text("Copy something and it shows up here", R::new(zone.x, zone.cy() - 6.0, zone.w, 20.0), 13.0, 650, C::WHITE, Align::Center);
        let sub = format!("The last {} texts and images  ·  kept until SuiNotch closes", app.cfg.clipboard_limit);
        cv.text(&sub, R::new(zone.x, zone.cy() + 13.0, zone.w, 16.0), 11.5, 500, DIM, Align::Center);
        return;
    }
    let scroll = app.notch.clip_scroll.value;
    cv.push_clip_rrect(list.inset(-2.0, -2.0), 12.0, 1.0);
    let items = app.clips.items.clone();
    for (i, clip) in items.iter().enumerate() {
        let x = list.x + i as f32 * (TILE_W + 10.0) - scroll;
        if x > list.right() || x + TILE_W < list.x {
            continue;
        }
        let tile = R::new(x, list.y, TILE_W, list.h);
        let h = Hit::ClipItem(i);
        hit(app, tile, h);
        cv.fill_rrect(tile, 14.0, if hovered(app, h) { C::WHITE.a(0.1) } else { C::WHITE.a(0.05) });
        match clip {
            Clip::Text(text) => {
                cv.glyph(icon::TEXT, tile.x + 18.0, tile.y + 18.0, 11.0, accent);
                cv.text("Text", R::new(tile.x + 30.0, tile.y + 10.0, 60.0, 16.0), 10.5, 650, DIM, Align::Left);
                let preview: String = text.chars().take(90).collect();
                cv.wrapped(&preview, R::new(tile.x + 10.0, tile.y + 32.0, tile.w - 20.0, tile.h - 40.0), 11.0, 500, C::WHITE.a(0.85), Align::Left);
            }
            Clip::Image { path, w, h } => {
                let thumb = R::new(tile.x + 8.0, tile.y + 8.0, tile.w - 16.0, tile.h - 34.0);
                match thumbnail(cv, app, path) {
                    Some(b) => cv.bitmap_cover(&b, thumb, 8.0, 1.0),
                    None => cv.glyph(icon::IMAGE, thumb.cx(), thumb.cy(), 22.0, DIM),
                }
                cv.text(&format!("{w} × {h}"), R::new(tile.x + 6.0, tile.bottom() - 22.0, tile.w - 12.0, 16.0), 10.5, 600, DIM, Align::Center);
            }
        }
        if i == 0 {
            // The newest item is what's on the clipboard now.
            cv.fill_circle(tile.right() - 12.0, tile.y + 12.0, 3.0, accent);
        }
    }
    cv.pop();
}
