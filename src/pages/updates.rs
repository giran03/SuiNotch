//! Temporary update page, shown only while an update is actionable or in progress.
use super::*;
use crate::update::{Status, MODE_AUTO};

pub fn updates(cv: &Canvas, app: &mut App, area: R) {
    let color = accent(app);
    let preview = app.notch.update_preview;
    let simulated = preview.map(crate::notch::debug::UpdatePreview::status);
    let status = simulated.as_ref().unwrap_or(&app.update);
    let (title, detail, action, busy) = match status {
        Status::Available(release) => (
            format!("SuiNotch {} is available", release.version),
            if app.cfg.update_mode == MODE_AUTO {
                "Download now, or let SuiNotch install while you're away.".into()
            } else {
                "Download the update now and choose when to restart.".into()
            },
            Some("Download update"),
            false,
        ),
        Status::Downloading(version) => {
            (format!("Downloading {version}"), "Getting the update and checking the download. You can keep using SuiNotch.".into(), None, true)
        }
        Status::Ready(download) => (
            format!("Version {} is ready", download.release.version),
            "Download verified. Install when you're ready for SuiNotch to restart.".into(),
            Some("Install and restart"),
            false,
        ),
        Status::Installing(version) => (format!("Installing {version}"), "SuiNotch will close briefly and reopen after the update.".into(), None, true),
        Status::InstallFailed { release, message } => (format!("Couldn't update to {}", release.version), message.clone(), Some("Retry download"), false),
        _ => return,
    };
    let title = if let Some(stage) = preview {
        use crate::notch::debug::UpdatePreview::*;
        match stage {
            Available => "Preview: Update available",
            Downloading => "Preview: Downloading update",
            Ready => "Preview: Ready to install",
            Installing => "Preview: Installing update",
            Failed => "Preview: Update failed",
        }.to_string()
    } else { title };
    let detail = if preview.is_some() { "Local test only. Next state walks through the update flow; leave this tab to close the preview.".into() } else { detail };
    let action = match preview { Some(crate::notch::debug::UpdatePreview::Failed) => Some("Finish preview"), Some(_) => Some("Next test state"), None => action };
    let cx = area.x + 28.0;
    let cy = area.y + 31.0;
    cv.fill_circle(cx, cy, 25.0, color.a(0.16));
    cv.glyph('\u{E895}', cx, cy, 22.0, color);
    let text_x = area.x + 68.0;
    cv.display(&title, R::new(text_x, area.y + 7.0, area.w - 68.0, 25.0), 16.0, 700, C::WHITE, Align::Left);
    cv.wrapped(&detail, R::new(text_x, area.y + 38.0, area.w - 76.0, 46.0), 12.0, 500, DIM, Align::Left);
    cv.text(&format!("Current version {}", env!("CARGO_PKG_VERSION")), R::new(area.x + 4.0, area.bottom() - 23.0, 185.0, 20.0), 11.5, 500, DIM, Align::Left);
    let button_y = area.bottom() - 34.0;
    if let Some(label) = action {
        let width = app.gfx.text_width(label, 12.5, 600) + 30.0;
        pill_button(cv, app, R::new(area.right() - width, button_y, width, 30.0), label, if preview.is_some() { Hit::DebugUpdateNext } else { Hit::UpdateAction }, true);
    }
    let notes = if preview.is_some() { "Close preview" } else { "Release notes" };
    let notes_width = app.gfx.text_width(notes, 12.5, 600) + 30.0;
    pill_button(cv, app, R::new(area.x + 4.0, area.y + 96.0, notes_width, 28.0), notes, if preview.is_some() { Hit::DebugUpdateClose } else { Hit::UpdateNotes }, false);
    if busy {
        // Indeterminate motion conveys activity without inventing a download percentage.
        let track = R::new(text_x, area.y + 90.0, area.w - 80.0, 3.0);
        cv.fill_rrect(track, 1.5, C::WHITE.a(0.1));
        let position = (app.started.elapsed().as_secs_f32() * 0.6).fract();
        let width = track.w * 0.24;
        cv.fill_rrect(R::new(track.x + position * (track.w - width), track.y, width, track.h), 1.5, color);
    }
}
