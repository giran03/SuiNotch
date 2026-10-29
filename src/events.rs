//! Background threads (WinRT callbacks, HTTP) hand results to the UI thread through this channel.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::lrc::Line;

pub const WM_APP_EVENT: u32 = WM_APP + 1;
pub const WM_APP_TRAY: u32 = WM_APP + 2;
pub const WM_APP_CAPS: u32 = WM_APP + 3;
pub const WM_APP_LANG: u32 = WM_APP + 4;
pub const WM_APP_FOREGROUND: u32 = WM_APP + 5;
/// Sent by a second launch of the exe: the running copy opens Settings.
pub const WM_APP_SHOW_SETTINGS: u32 = WM_APP + 6;
/// One per vertical blank while the notch animates (see ticker.rs).
pub const WM_APP_FRAME: u32 = WM_APP + 7;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app_id: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
}

impl MediaInfo {
    pub fn track_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.title.hash(&mut h);
        self.artist.hash(&mut h);
        h.finish()
    }
}

#[derive(Debug)]
pub enum Event {
    Media(Option<MediaInfo>),
    Artwork { key: u64, art: Option<crate::gfx::ArtPixels> },
    Lyrics { key: u64, lines: Vec<Line> },
    Volume { level: f32, muted: bool },
    AudioDeviceChanged,
    Network { connected: bool, name: String },
    Bluetooth { name: String, connected: bool },
}

static SENDER: OnceLock<Mutex<Sender<Event>>> = OnceLock::new();
static TARGET: AtomicIsize = AtomicIsize::new(0);

pub fn init() -> Receiver<Event> {
    let (tx, rx) = channel();
    let _ = SENDER.set(Mutex::new(tx));
    rx
}

pub fn set_target(hwnd: HWND) {
    TARGET.store(hwnd.0 as isize, Ordering::SeqCst);
}

pub fn target() -> HWND {
    HWND(TARGET.load(Ordering::SeqCst) as *mut _)
}

pub fn post(ev: Event) {
    if let Some(tx) = SENDER.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(ev);
        }
    }
    wake(WM_APP_EVENT);
}

pub fn wake(msg: u32) {
    let hwnd = target();
    if !hwnd.0.is_null() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
        }
    }
}
