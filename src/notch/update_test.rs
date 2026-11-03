//! Isolated native rendering and tab lifecycle test; no networking or installer execution.
use super::*;
use crate::update::{Download, Release, Status};

#[test]
#[ignore = "native graphics; requires isolated APPDATA ending in update-tab-test"]
fn update_tab_appears_preserves_focus_and_returns_to_previous_page_when_done() {
    assert!(crate::config::data_dir().parent().unwrap().ends_with("update-tab-test"));
    let mut app = tasks::integration::fixture();
    let release = Release {
        version: "99.0.0".into(),
        setup_url: "https://example.invalid/setup".into(),
        setup_name: "SuiNotch-99.0.0-Setup.exe".into(),
        sums_url: "https://example.invalid/sums".into(),
    };
    let download = Download { release: release.clone(), path: std::path::PathBuf::from("not-an-installer.exe"), sha256: "0".repeat(64) };
    let output = crate::config::data_dir();
    std::fs::create_dir_all(&output).unwrap();
    app.notch.page = Page::Calendar;
    assert!(!visible_pages(&app).contains(&Page::Updates));
    assert!(!Page::ALL.contains(&Page::Updates));
    update_status(&mut app, Status::Available(release.clone()));
    assert_eq!(app.notch.page, Page::Calendar); // discovering an update never steals focus
    assert!(visible_pages(&app).contains(&Page::Updates));
    set_page(&mut app, Page::Updates);
    assert_eq!(app.notch.update_return_page, Some(Page::Calendar));
    let states = [
        ("available", Status::Available(release.clone()), true),
        ("downloading", Status::Downloading(release.version.clone()), false),
        ("ready", Status::Ready(download), true),
        ("installing", Status::Installing(release.version.clone()), false),
        ("failed", Status::InstallFailed { release: release.clone(), message: "The download was interrupted. Please try again.".into() }, true),
    ];
    for (name, status, actionable) in states {
        update_status(&mut app, status);
        assert!(visible_pages(&app).contains(&Page::Updates));
        for scale in [1.0, 1.5, 2.0] {
            render::task_snapshot(&mut app, &output.join(format!("update-{name}-{scale}.png")), scale).unwrap();
            assert_eq!(app.notch.hits.iter().any(|(_, h)| *h == Hit::UpdateAction), actionable);
            assert!(app.notch.hits.iter().any(|(_, h)| *h == Hit::UpdateNotes));
        }
    }
    // A failed update remains visible and retryable. Success closes it and restores Calendar.
    update_status(&mut app, Status::Installed(release.version.clone()));
    assert_eq!(app.notch.page, Page::Calendar);
    assert!(!visible_pages(&app).contains(&Page::Updates));
    assert!(app.notch.page_pos.value < visible_pages(&app).len() as f32);
    update_status(&mut app, Status::Available(Release { version: "100.0.0".into(), ..release.clone() }));
    set_page(&mut app, Page::Updates);
    set_page(&mut app, Page::Water);
    update_status(&mut app, Status::UpToDate);
    assert_eq!(app.notch.page, Page::Water); // closing a background tab must not move the user
    update_status(&mut app, Status::Available(release));
    set_page(&mut app, Page::Updates);
    app.cfg.hidden_pages.push(Page::Water.key().into());
    update_status(&mut app, Status::UpToDate);
    assert_eq!(app.notch.page, Page::Music); // return page was hidden in the meantime
    assert!(!visible_pages(&app).contains(&Page::Updates));
    set_page(&mut app, Page::Updates); // stale click cannot select a disappeared tab
    assert_eq!(app.notch.page, Page::Music);
    debug_update_preview_lifecycle(&mut app, &output);
    unsafe {
        DestroyWindow(app.notch.hwnd).unwrap();
    }
}

fn debug_update_preview_lifecycle(app: &mut App, output: &std::path::Path) {
    use super::debug::{self, UpdatePreview};
    app.update = Status::UpToDate;
    app.cfg.hidden_pages.retain(|page| page != Page::Water.key());
    set_page(app, Page::Calendar);
    let original_cfg = app.cfg.clone();
    debug::open_updates(app);
    assert!(app.notch.expanded);
    assert_eq!(app.notch.page, Page::Updates);
    assert!(matches!(app.update, Status::UpToDate));
    for (name, stage) in [("available", UpdatePreview::Available), ("downloading", UpdatePreview::Downloading), ("ready", UpdatePreview::Ready), ("installing", UpdatePreview::Installing), ("failed", UpdatePreview::Failed)] {
        assert_eq!(app.notch.update_preview, Some(stage));
        for scale in [1.0, 1.5, 2.0] {
            render::task_snapshot(app, &output.join(format!("debug-update-{name}-{scale}.png")), scale).unwrap();
            assert!(app.notch.hits.iter().any(|(_, h)| *h == Hit::DebugUpdateNext));
            assert!(app.notch.hits.iter().any(|(_, h)| *h == Hit::DebugUpdateClose));
            assert!(!app.notch.hits.iter().any(|(_, h)| matches!(h, Hit::UpdateAction | Hit::UpdateNotes)));
        }
        views::click(app, Hit::UpdateAction, (false, false)); // even stale real actions are blocked during preview
        assert!(matches!(app.update, Status::UpToDate));
        views::click(app, Hit::DebugUpdateNext, (false, false));
    }
    assert!(app.notch.update_preview.is_none());
    assert_eq!(app.notch.page, Page::Calendar);
    assert!(!visible_pages(app).contains(&Page::Updates));
    debug::open_updates(app);
    set_page(app, Page::Water);
    assert!(!update_tab_visible(app));
    assert!(app.notch.update_preview.is_none());
    debug::open_updates(app);
    collapse(app);
    assert_eq!(app.notch.page, Page::Water);
    assert!(!update_tab_visible(app));
    debug::open_updates(app);
    views::click(app, Hit::DebugUpdateClose, (false, false));
    assert!(!update_tab_visible(app));
    let release = Release { version: "99.0.0".into(), setup_url: String::new(), setup_name: String::new(), sums_url: String::new() };
    debug::open_updates(app);
    update_status(app, Status::Available(release));
    assert!(app.notch.update_preview.is_none()); // actual update takes over immediately
    views::click(app, Hit::DebugUpdateNext, (false, false));
    views::click(app, Hit::DebugUpdateClose, (false, false));
    assert!(matches!(app.update, Status::Available(_)));
    assert_eq!(app.notch.page, Page::Updates);
    debug::open_updates(app);
    debug::close_updates(app);
    assert!(update_tab_visible(app)); // real update remains after a preview closes
    assert!(matches!(app.update, Status::Available(_)));
    assert_eq!(app.cfg, original_cfg); // no preference or update-mode changes
}
