//! Sound choices, local imports and an inline custom clip editor.
use super::*;
use crate::sound;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static IMPORTING: AtomicBool = AtomicBool::new(false);
#[derive(Default)]
pub(super) struct State {
    choosing: Option<usize>,
    editing: Option<usize>,
    renaming: Option<(usize, String)>,
    confirm_delete: Option<usize>,
    preview: Option<Action>,
    message: String,
}
impl State {
    pub(super) fn leave(&mut self) {
        self.renaming = None;
        self.choosing = None;
        self.confirm_delete = None;
        self.preview = None;
        sound::stop_preview();
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Action {
    Pick(usize),
    Assign(usize, usize),
    Preview(usize),
    Test(usize),
    Import,
    Edit(usize),
    Rename(usize),
    SaveName,
    Replace(usize),
    Delete(usize),
    Trim(usize, bool, i32),
    Normalize(usize),
    Defaults,
}
pub(crate) fn message(app: &mut App, message: &str) {
    if let Some(s) = &mut app.settings {
        s.sound.message = message.into();
        invalidate(s);
    }
    crate::notch::notice(app, message);
}
fn begin_import(app: &mut App, path: PathBuf, replacement: Option<String>) {
    if IMPORTING.swap(true, Ordering::SeqCst) {
        message(app, "A sound import is already in progress.");
        return;
    }
    message(app, "Importing sound...");
    sound::import(path, replacement);
}
pub(crate) fn drop_files(app: &mut App, paths: Vec<PathBuf>) {
    if !app.settings.as_ref().is_some_and(|s| s.page == SOUNDS_PAGE && s.query.is_empty()) {
        message(app, "Open Settings > Sounds to import audio.");
        return;
    }
    if paths.len() != 1 {
        message(app, "Drop one WAV, MP3 or OGG file at a time.");
        return;
    }
    begin_import(app, paths.into_iter().next().unwrap(), None);
}
pub(crate) fn choose_file(replacement: Option<String>) {
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH};
    if IMPORTING.load(Ordering::SeqCst) {
        return;
    }
    let Some(owner) = app::with(|app| app.settings.as_ref().map(|s| s.hwnd)).flatten() else { return };
    let chosen: std::result::Result<Option<PathBuf>, windows::core::Error> = (|| unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(w!("Import a sound"))?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: w!("Audio (WAV, MP3, OGG)"), pszSpec: w!("*.wav;*.mp3;*.ogg") }])?;
        if let Err(e) = dialog.Show(Some(owner)) {
            if e.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(e);
        }
        let display = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let value = display.to_string();
        CoTaskMemFree(Some(display.0.cast()));
        Ok(Some(PathBuf::from(value?)))
    })();
    match chosen {
        Ok(Some(path)) => {
            app::with(|app| begin_import(app, path, replacement));
        }
        Ok(None) => {}
        Err(e) => {
            crate::log(&format!("sound file picker: {e}"));
            app::with(|app| message(app, "Could not open the audio file picker."));
        }
    }
}
pub(crate) fn import_finished(app: &mut App, result: std::result::Result<sound::CustomSound, String>, replacement: Option<String>) {
    IMPORTING.store(false, Ordering::SeqCst);
    match result {
        Err(e) => message(app, &e),
        Ok(mut imported) => {
            let mut cfg = app.cfg.clone();
            let library = &mut cfg.sound_library;
            if let Some(i) = replacement.as_ref().and_then(|id| library.custom.iter().position(|s| &s.id == id)) {
                let old = library.custom[i].clone();
                imported.name = old.name;
                for assigned in &mut library.assignments {
                    if *assigned == old.id {
                        *assigned = imported.id.clone();
                    }
                }
                // Retain the old entry if removing its file fails; the successfully imported sound is still usable.
                if library.remove(i).is_err() {
                    library.custom.push(imported);
                } else {
                    library.custom.insert(i, imported);
                }
            } else {
                library.custom.push(imported);
            }
            app.set_config(cfg);
            message(app, "Sound saved on this PC. Choose it for any reminder.");
        }
    }
}

pub(super) fn character(app: &mut App, code: u32) -> bool {
    let Some(s) = app.settings.as_mut() else { return false };
    let Some((_, text)) = s.sound.renaming.as_mut() else { return false };
    match code {
        8 => {
            text.pop();
        }
        c if c >= 32 && c != 127 => {
            if let Some(ch) = char::from_u32(c).filter(|_| text.chars().count() < 60) {
                text.push(ch);
            }
        }
        _ => {}
    }
    invalidate(s);
    true
}
pub(super) fn key(app: &mut App, vk: u16) -> bool {
    if app.settings.as_ref().is_none_or(|s| s.sound.renaming.is_none()) {
        return false;
    }
    if vk == VK_RETURN.0 {
        act(app, Action::SaveName);
    } else if vk == VK_ESCAPE.0 {
        if let Some(s) = &mut app.settings {
            s.sound.renaming = None;
            invalidate(s);
        }
    }
    true
}
pub(super) fn act(app: &mut App, action: Action) {
    let mut cfg = app.cfg.clone();
    let Some(s) = &mut app.settings else { return };
    if !matches!(action, Action::Delete(_)) {
        s.sound.confirm_delete = None;
    }
    match action {
        Action::Pick(i) => {
            s.sound.choosing = if s.sound.choosing == Some(i) { None } else { Some(i) };
            invalidate(s);
            return;
        }
        Action::Assign(event, choice) => {
            if let Some((id, _)) = cfg.sound_library.choices().get(choice) {
                cfg.sound_library.assignments[event] = id.clone();
            }
            s.sound.choosing = None;
        }
        Action::Preview(i) | Action::Test(i) => {
            let id = if matches!(action, Action::Test(_)) {
                cfg.sound_library.assignments[i].clone()
            } else if i < sound::PRESETS.len() {
                sound::PRESETS[i].id.into()
            } else {
                match cfg.sound_library.custom.get(i - sound::PRESETS.len()) {
                    Some(c) => c.id.clone(),
                    None => return,
                }
            };
            if !cfg.sounds || cfg.sound_library.volume == 0 {
                s.sound.message = "Sounds are muted. Enable reminder sounds and raise the volume to preview.".into();
            } else if sound::alert_playing() {
                s.sound.message = "Let the reminder finish or dismiss its alert before previewing.".into();
            } else if sound::previewing() && s.sound.preview == Some(action) {
                sound::stop_preview();
            } else {
                s.sound.preview = Some(action);
                sound::preview(&cfg.sound_library, &id);
            }
            invalidate(s);
            return;
        }
        Action::Import => {
            app::defer(app::Deferred::ImportSound(None));
            return;
        }
        Action::Replace(i) => {
            if let Some(c) = cfg.sound_library.custom.get(i) {
                app::defer(app::Deferred::ImportSound(Some(c.id.clone())));
            }
            return;
        }
        Action::Edit(i) => {
            s.sound.editing = if s.sound.editing == Some(i) { None } else { Some(i) };
            s.sound.renaming = None;
            invalidate(s);
            return;
        }
        Action::Rename(i) => {
            if let Some(c) = cfg.sound_library.custom.get(i) {
                s.sound.renaming = Some((i, c.name.clone()));
            }
            invalidate(s);
            return;
        }
        Action::SaveName => {
            if let Some((i, name)) = s.sound.renaming.take() {
                if let Some(c) = cfg.sound_library.custom.get_mut(i) {
                    c.name = name;
                    c.sanitize();
                }
            }
        }
        Action::Delete(i) => {
            if s.sound.confirm_delete != Some(i) {
                s.sound.confirm_delete = Some(i);
                invalidate(s);
                return;
            }
            sound::stop_preview();
            if let Err(e) = cfg.sound_library.remove(i) {
                s.sound.message = e;
                invalidate(s);
                return;
            }
            s.sound.confirm_delete = None;
            s.sound.editing = None;
            s.sound.renaming = None;
        }
        Action::Trim(i, start, delta) => {
            if let Some(c) = cfg.sound_library.custom.get_mut(i) {
                if start {
                    c.start_ms = (c.start_ms as i64 + delta as i64).clamp(0, c.duration_ms.saturating_sub(100) as i64) as u32;
                } else {
                    c.length_ms = (c.length_ms as i64 + delta as i64).clamp(100, 30_000) as u32;
                }
                c.sanitize();
            }
        }
        Action::Normalize(i) => {
            if let Some(c) = cfg.sound_library.custom.get_mut(i) {
                c.normalize = !c.normalize;
            }
        }
        Action::Defaults => {
            cfg.sound_library.reset_assignments();
            s.sound.message = "Original sound assignments restored. Your imports are kept.".into();
        }
    }
    app.set_config(cfg);
    refresh(app);
}

impl Ui<'_, '_> {
    pub(super) fn sounds(&mut self, c: &Config) {
        self.title("Sounds");
        let status = if !c.sounds || c.sound_library.volume == 0 {
            "Sounds are muted"
        } else if c.respect_quiet && crate::quiet::active() {
            "Quiet mode is active; reminders are silent"
        } else {
            "Presets and imported audio stay on this PC"
        };
        self.section(status);
        let rows = self.card(&[ROW_SUB, ROW, ROW_SUB]);
        self.toggle(c, rows[0], "Reminder sounds", Some("Audio for reminders and completed timers"), |c| &mut c.sounds);
        self.stepper(c, rows[1], "Sound volume", None, |c| &mut c.sound_library.volume, 5, 0, 100, &|n| format!("{n}%"));
        self.toggle(c, rows[2], "Respect Do Not Disturb", Some("Keep reminders quiet while presenting or gaming"), |c| &mut c.respect_quiet);
        if !self.s.sound.message.is_empty() {
            self.cv.wrapped(&self.s.sound.message, R::new(self.x + 4.0, self.y, self.w - 8.0, 44.0), 11.5, 500, DIM, Align::Left);
            self.y += 52.0;
        }
        // Keep the active button updated without inserting a row that shifts the page.
        self.moving |= sound::previewing();
        self.section("Sound for each reminder");
        let choices = c.sound_library.choices();
        for (i, label) in sound::EVENT_LABELS.iter().enumerate() {
            let r = self.card(&[ROW_SUB]);
            self.label_fit(r[0], label, Some(&c.sound_library.name(&c.sound_library.assignments[i])), 156.0);
            self.sound_button(R::new(r[0].right() - 146.0, r[0].cy() - 14.0, 72.0, 28.0), "Choose", Action::Pick(i));
            let playing = sound::previewing() && self.s.sound.preview == Some(Action::Test(i));
            self.sound_button(R::new(r[0].right() - 66.0, r[0].cy() - 14.0, 50.0, 28.0), if playing { "Stop" } else { "Play" }, Action::Test(i));
            if self.s.sound.choosing == Some(i) {
                let r = self.card(&vec![ROW; choices.len()]);
                for (j, (id, name)) in choices.iter().enumerate() {
                    let selected = *id == c.sound_library.assignments[i];
                    self.button(r[j], name, None, if selected { "Selected" } else { "Use" }, Act::Sound(Action::Assign(i, j)), false);
                }
            }
        }
        self.section("Timer ringing");
        let r = self.card(&[ROW_SUB, ROW, ROW, ROW_SUB]);
        self.toggle(c, r[0], "Repeat timer sound", Some("Ring until dismissed, or the time limit is reached"), |c| &mut c.sound_library.timer_repeat);
        self.dimmed(!c.sound_library.timer_repeat, |ui| {
            ui.stepper(c, r[1], "Pause between rings", None, |c| &mut c.sound_library.repeat_gap_s, 1, 1, 30, &|n| format!("{n} sec"));
            ui.stepper(c, r[2], "Stop ringing after", None, |c| &mut c.sound_library.ring_limit_s, 5, 5, 300, &|n| format!("{n} sec"));
            ui.toggle(c, r[3], "Gradual volume", Some("Start softly and reach your volume over ten seconds"), |c| &mut c.sound_library.gradual);
        });
        self.section("Preset library");
        let r = self.card(&[ROW_SUB; 8]);
        for (i, p) in sound::PRESETS.iter().enumerate() {
            let sub = format!("{} · {:.2} sec", p.description, p.seconds);
            let playing = sound::previewing() && self.s.sound.preview == Some(Action::Preview(i));
            self.button(r[i], p.name, Some(&sub), if playing { "Stop" } else { "Play" }, Act::Sound(Action::Preview(i)), false);
        }
        self.section("Your sounds");
        let r = self.card(&[ROW_SUB]);
        self.button(
            r[0],
            "Import sound",
            Some("WAV, MP3 or OGG · up to 20 MB"),
            if IMPORTING.load(Ordering::SeqCst) { "Importing..." } else { "Browse..." },
            Act::Sound(Action::Import),
            false,
        );
        self.cv.wrapped(
            "Or drop an audio file on this page. Sources up to five minutes; saved clips up to 30 seconds.",
            R::new(self.x + 4.0, self.y - 8.0, self.w - 8.0, 36.0),
            11.5,
            500,
            DIM,
            Align::Left,
        );
        self.y += 38.0;
        for (i, custom) in c.sound_library.custom.iter().enumerate() {
            let sub = if sound::available(&custom.id) {
                format!("{:.1} sec clip · {:.1} sec source", custom.length_ms as f32 / 1000.0, custom.duration_ms as f32 / 1000.0)
            } else {
                "File unavailable; reminders use their default sound".into()
            };
            let r = self.card(&[ROW_SUB]);
            self.label_fit(r[0], &custom.name, Some(&sub), 150.0);
            let playing = sound::previewing() && self.s.sound.preview == Some(Action::Preview(8 + i));
            self.sound_button(R::new(r[0].right() - 136.0, r[0].cy() - 14.0, 56.0, 28.0), if playing { "Stop" } else { "Play" }, Action::Preview(8 + i));
            self.sound_button(R::new(r[0].right() - 72.0, r[0].cy() - 14.0, 56.0, 28.0), "Edit", Action::Edit(i));
            if self.s.sound.editing == Some(i) {
                let r = self.card(&[ROW_SUB, ROW, ROW, ROW_SUB, ROW_SUB, ROW_SUB]);
                if let Some((_, name)) = self.s.sound.renaming.as_ref().filter(|(j, _)| *j == i) {
                    let name = format!("{name}|");
                    self.button(r[0], &name, Some("Type a name, then press Enter. Esc cancels."), "Save", Act::Sound(Action::SaveName), false);
                } else {
                    self.button(r[0], "Name", Some(&custom.name), "Rename", Act::Sound(Action::Rename(i)), false);
                }
                self.trim_row(r[1], "Clip starts at", custom.start_ms, i, true);
                self.trim_row(r[2], "Clip length", custom.length_ms, i, false);
                self.button(
                    r[3],
                    "Even out loudness",
                    Some("Peak adjustment with a gentle gain limit"),
                    if custom.normalize { "On" } else { "Off" },
                    Act::Sound(Action::Normalize(i)),
                    false,
                );
                self.button(r[4], "Replace audio", Some("Keep this name and its reminder assignments"), "Browse...", Act::Sound(Action::Replace(i)), false);
                let assigned: Vec<_> =
                    c.sound_library.assignments.iter().enumerate().filter(|(_, id)| **id == custom.id).map(|(i, _)| sound::EVENT_LABELS[i]).collect();
                let detail =
                    if assigned.is_empty() { "Remove this sound from your PC".into() } else { format!("Returns to defaults: {}", assigned.join(", ")) };
                self.button(
                    r[5],
                    "Delete sound",
                    Some(&detail),
                    if self.s.sound.confirm_delete == Some(i) { "Confirm delete" } else { "Delete" },
                    Act::Sound(Action::Delete(i)),
                    true,
                );
                if self.s.sound.confirm_delete == Some(i) && !assigned.is_empty() {
                    self.cv.wrapped(&detail, R::new(self.x + 4.0, self.y - 8.0, self.w - 8.0, 64.0), 11.5, 500, DIM, Align::Left);
                    self.y += 68.0;
                }
            }
        }
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Restore sound defaults", Some("Reset reminder choices; keep imported audio"), "Restore", Act::Sound(Action::Defaults), false);
    }
    fn sound_button(&mut self, r: R, text: &str, action: Action) {
        let act = Act::Sound(action);
        self.s.hits.push((r, act));
        self.cv.fill_rrect(r, 14.0, C::WHITE.a(if hovered(self.s, act) { 0.16 } else { 0.1 }));
        self.cv.text(text, r, 12.0, 600, C::WHITE, Align::Center);
    }
    fn trim_row(&mut self, r: R, label: &str, ms: u32, index: usize, start: bool) {
        self.label_fit(r, label, None, 196.0);
        self.cv.text(&format!("{:.1} sec", ms as f32 / 1000.0), R::new(r.right() - 138.0, r.y, 64.0, r.h), 12.5, 600, C::WHITE, Align::Center);
        self.sound_button(R::new(r.right() - 188.0, r.cy() - 14.0, 44.0, 28.0), "−1s", Action::Trim(index, start, -1000));
        self.sound_button(R::new(r.right() - 68.0, r.cy() - 14.0, 52.0, 28.0), "+1s", Action::Trim(index, start, 1000));
    }
}

#[cfg(test)]
#[path = "sounds_test.rs"]
mod integration;
