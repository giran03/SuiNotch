//! Frame pacing. While something animates, a background thread waits for each vertical blank
//! of the notch's monitor and posts one frame message, so frames land exactly once per refresh
//! (60, 120, 144 Hz...) instead of on the coarse 15.6 ms Windows timer tick. When nothing
//! moves, the thread sleeps and no frames are drawn at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};

use crate::events::{self, WM_APP_FRAME};

static RUNNING: Mutex<bool> = Mutex::new(false);
static WAKE: Condvar = Condvar::new();
/// A frame message is queued and not yet handled; never queue a second one.
static POSTED: AtomicBool = AtomicBool::new(false);
static DEVICE: Mutex<String> = Mutex::new(String::new());
static FAST: AtomicBool = AtomicBool::new(true);

const FALLBACK_HZ: u32 = 60;
/// Frame-rate caps: shape changes (opening, resizing, page slides) and everything else
/// (fades, the volume slider). On faster screens frames are drawn on every 2nd/3rd/4th refresh.
const MORPH_FPS: u32 = 120;
const DETAIL_FPS: u32 = 60;

pub fn start(device: &str) {
    set_monitor(device);
    std::thread::spawn(run_loop);
}

pub fn set_monitor(device: &str) {
    *DEVICE.lock().unwrap_or_else(|e| e.into_inner()) = device.to_string();
}

/// Asks for frames every refresh (`true`) or lets the thread sleep (`false`).
pub fn animate(on: bool) {
    let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if *running != on {
        *running = on;
        if on {
            WAKE.notify_one();
        }
    }
}

/// Full frame rate while the outline morphs; a lower cap for small animations.
pub fn set_fast(fast: bool) {
    FAST.store(fast, Ordering::Relaxed);
}

/// Called by the UI thread when it starts handling a frame message.
pub fn frame_taken() {
    POSTED.store(false, Ordering::Release);
}

fn wait_until_running() {
    let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    while !*running {
        running = WAKE.wait(running).unwrap_or_else(|e| e.into_inner());
    }
}

fn find_output(device: &str) -> Option<IDXGIOutput> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut first = None;
        for a in 0.. {
            let Ok(adapter) = factory.EnumAdapters1(a) else { break };
            for o in 0.. {
                let Ok(output) = adapter.EnumOutputs(o) else { break };
                let Ok(desc) = output.GetDesc() else { continue };
                let len = desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(desc.DeviceName.len());
                if String::from_utf16_lossy(&desc.DeviceName[..len]) == device {
                    return Some(output);
                }
                first.get_or_insert(output);
            }
        }
        first
    }
}

fn refresh_hz(device: &str) -> u32 {
    let name: Vec<u16> = device.encode_utf16().chain(Some(0)).collect();
    let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    let ok = unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool();
    if ok && mode.dmDisplayFrequency > 1 {
        mode.dmDisplayFrequency
    } else {
        FALLBACK_HZ
    }
}

fn run_loop() {
    let mut current = String::new();
    let mut output: Option<IDXGIOutput> = None;
    let mut hz = FALLBACK_HZ;
    loop {
        wait_until_running();
        let device = DEVICE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if device != current {
            output = find_output(&device);
            hz = refresh_hz(&device);
            current = device;
        }
        let cap = if FAST.load(Ordering::Relaxed) { MORPH_FPS } else { DETAIL_FPS };
        let every = hz.div_ceil(cap).max(1);
        let period = Duration::from_secs_f64(every as f64 / hz as f64);
        let synced = output.as_ref().is_some_and(|o| (0..every).all(|_| unsafe { o.WaitForVBlank() }.is_ok()));
        if !synced {
            // No vblank source (remote desktop, driver hiccup): fall back to the refresh period.
            output = None;
            std::thread::sleep(period);
        }
        if !POSTED.swap(true, Ordering::AcqRel) {
            events::wake(WM_APP_FRAME);
        }
    }
}
