//! Local feature previews. Simulated updater state never enters the real updater.
use super::*;
use crate::update::{Download, Release, Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePreview {
    Available,
    Downloading,
    Ready,
    Installing,
    Failed,
}
impl UpdatePreview {
    pub fn busy(self) -> bool {
        matches!(self, Self::Downloading | Self::Installing)
    }
    pub fn status(self) -> Status {
        let release = Release {
            version: "preview".into(),
            setup_url: String::new(),
            setup_name: String::new(),
            sums_url: String::new(),
        };
        match self {
            Self::Available => Status::Available(release),
            Self::Downloading => Status::Downloading(release.version),
            Self::Ready => Status::Ready(Download {
                release,
                path: Default::default(),
                sha256: String::new(),
            }),
            Self::Installing => Status::Installing(release.version),
            Self::Failed => Status::InstallFailed {
                release,
                message: "Preview: sample download failure. No files were downloaded.".into(),
            },
        }
    }
}

pub fn open_updates(app: &mut App) {
    app.notch.update_preview = Some(UpdatePreview::Available);
    set_page(app, Page::Updates);
    expand(app);
    reset_hits(app);
}

pub fn next_update(app: &mut App) {
    use UpdatePreview::*;
    app.notch.update_preview = match app.notch.update_preview {
        Some(Available) => Some(Downloading),
        Some(Downloading) => Some(Ready),
        Some(Ready) => Some(Installing),
        Some(Installing) => Some(Failed),
        Some(Failed) => {
            close_updates(app);
            return;
        }
        None => return,
    };
    reset_hits(app);
}

pub fn close_updates(app: &mut App) {
    if app.notch.update_preview.take().is_none() {
        return;
    }
    if app.notch.page == Page::Updates {
        let shown = Notch::pages(&app.cfg);
        let destination = app
            .notch
            .update_return_page
            .take()
            .filter(|p| *p != Page::Updates && shown.contains(p))
            .or_else(|| shown.first().copied())
            .unwrap_or(Page::Music);
        set_page(app, destination);
        let index = visible_pages(app)
            .iter()
            .position(|p| *p == destination)
            .unwrap_or(0);
        app.notch.page_pos.snap(index as f32);
    }
    reset_hits(app);
}

fn reset_hits(app: &mut App) {
    app.notch.hits.clear();
    app.notch.hover = None;
    app.notch.pressed = None;
    request_frame(app);
}

pub fn volume_hud(app: &mut App) {
    collapse(app);
    show_hud(
        app,
        Hud::Volume {
            level: 0.5,
            muted: false,
        },
    );
}

pub fn notification(app: &mut App) {
    notice(app, "Test notification: SuiNotch is ready");
}
