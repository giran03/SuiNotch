//! Frame pacing. While something animates, a background thread waits for each vertical blank
//! of the notch's monitor and posts one frame message, so frames land exactly once per refresh
//! (60, 120, 144 Hz...) instead of on the coarse 15.6 ms Windows timer tick. When nothing
//! moves, the thread sleeps and no frames are drawn at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE,
    TIMER_ALL_ACCESS,
};

use crate::events::{self, WM_APP_FRAME};

static RUNNING: Mutex<bool> = Mutex::new(false);
static WAKE: Condvar = Condvar::new();
/// A frame message is queued and not yet handled; never queue a second one.
static POSTED: AtomicBool = AtomicBool::new(false);
static DEVICE: Mutex<String> = Mutex::new(String::new());
static FAST: AtomicBool = AtomicBool::new(true);

const FALLBACK_HZ: u32 = 60;
/// Target frame rates: shape changes (opening, resizing, page slides) and everything else
/// (fades, the volume slider). Frames are drawn every Nth refresh, N chosen to land nearest
/// the target: 60 Hz -> 60/60, 144 Hz -> 144/72, 165 Hz -> 165/55, 240 Hz -> 120/60.
const MORPH_FPS: f64 = 144.0;
const DETAIL_FPS: f64 = 60.0;

/// How many refreshes to wait per frame so the frame rate lands nearest `target`.
pub fn refresh_divisor(hz: u32, target: f64) -> u32 {
    ((hz as f64 / target).round() as u32).max(1)
}

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

/// Sleeps with sub-millisecond precision (std::thread::sleep rounds up to the 15.6 ms tick).
struct PreciseTimer(Option<HANDLE>);

impl PreciseTimer {
    fn new() -> Self {
        let h = unsafe { CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) };
        PreciseTimer(h.ok())
    }

    fn sleep_until(&self, deadline: Instant) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match self.0 {
            Some(h) => unsafe {
                // Negative due time = relative, in 100 ns units.
                let due = -((left.as_nanos() / 100) as i64).max(1);
                if SetWaitableTimer(h, &due, 0, None, None, false).is_ok() {
                    WaitForSingleObject(h, INFINITE);
                } else {
                    std::thread::sleep(left);
                }
            },
            // Windows before 10 1803 has no high-resolution timers.
            None => std::thread::sleep(left),
        }
    }
}

impl Drop for PreciseTimer {
    fn drop(&mut self) {
        if let Some(h) = self.0 {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}

fn run_loop() {
    let timer = PreciseTimer::new();
    let mut current = String::new();
    let mut output: Option<IDXGIOutput> = None;
    let mut hz = FALLBACK_HZ;
    let mut next_tick = Instant::now();
    loop {
        wait_until_running();
        let device = DEVICE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if device != current {
            output = if crate::profile::force_timer() { None } else { find_output(&device) };
            hz = refresh_hz(&device);
            if crate::profile::enabled() {
                crate::log(&format!("ticker: {device} at {hz} Hz, vblank sync {}", if output.is_some() { "on" } else { "off" }));
            }
            current = device;
        }
        let target = if FAST.load(Ordering::Relaxed) { MORPH_FPS } else { DETAIL_FPS };
        let every = refresh_divisor(hz, target);
        let synced = output.as_ref().is_some_and(|o| (0..every).all(|_| unsafe { o.WaitForVBlank() }.is_ok()));
        if !synced {
            // No vblank source (Remote Desktop, some VMs, a driver hiccup): keep a fixed schedule
            // at the refresh period so frames stay evenly spaced.
            output = None;
            let period = Duration::from_secs_f64(every as f64 / hz as f64);
            let now = Instant::now();
            next_tick = if next_tick + period < now { now + period } else { next_tick + period };
            timer.sleep_until(next_tick);
        }
        if !POSTED.swap(true, Ordering::AcqRel) {
            events::wake(WM_APP_FRAME);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_refresh_rates_get_sensible_frame_rates() {
        let fps = |hz: u32, target: f64| hz as f64 / refresh_divisor(hz, target) as f64;
        for (hz, morph, detail) in [(60, 60.0, 60.0), (75, 75.0, 75.0), (120, 120.0, 60.0), (144, 144.0, 72.0), (165, 165.0, 55.0), (240, 120.0, 60.0), (360, 120.0, 60.0)] {
            assert_eq!(fps(hz, MORPH_FPS), morph, "{hz} Hz morph");
            assert_eq!(fps(hz, DETAIL_FPS), detail, "{hz} Hz detail");
        }
    }

    #[test]
    fn divisor_is_never_zero() {
        assert_eq!(refresh_divisor(30, MORPH_FPS), 1);
        assert_eq!(refresh_divisor(1, DETAIL_FPS), 1);
    }
}
