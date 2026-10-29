//! Thin wrappers over Windows services: volume, network, Bluetooth, power, keyboard and
//! foreground hooks, startup registration, clipboard, monitors and shell helpers.

use std::path::{Path, PathBuf};

use windows::core::{implement, w, Result, BOOL, GUID, HSTRING, PCWSTR};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothMajorClass};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Foundation::TypedEventHandler;
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::Foundation::*;
use windows::Win32::Globalization::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Power::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::events::{self, post, Event, WM_APP_CAPS, WM_APP_FOREGROUND, WM_APP_LANG};

// ---------------------------------------------------------------- volume

/// Volume changes we make ourselves carry this context so they don't pop the HUD.
pub const OWN_VOLUME_CONTEXT: GUID = GUID::from_u128(0x5e1d_0c7a_9b3f_4c2e_a1d4_53554e4f5443);

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback;

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            if d.guidEventContext != OWN_VOLUME_CONTEXT {
                post(Event::Volume { level: d.fMasterVolume, muted: d.bMuted.as_bool() });
            }
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback;

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eMultimedia {
            post(Event::AudioDeviceChanged);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

pub struct Audio {
    enumerator: IMMDeviceEnumerator,
    endpoint: Option<IAudioEndpointVolume>,
    volume_cb: IAudioEndpointVolumeCallback,
    _device_cb: IMMNotificationClient,
}

impl Audio {
    pub fn new() -> Result<Audio> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device_cb: IMMNotificationClient = DeviceCallback.into();
            enumerator.RegisterEndpointNotificationCallback(&device_cb)?;
            let mut audio = Audio { enumerator, endpoint: None, volume_cb: VolumeCallback.into(), _device_cb: device_cb };
            audio.rebind();
            Ok(audio)
        }
    }

    pub fn rebind(&mut self) {
        unsafe {
            if let Some(old) = self.endpoint.take() {
                let _ = old.UnregisterControlChangeNotify(&self.volume_cb);
            }
            let endpoint = self
                .enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None));
            match endpoint {
                Ok(ep) => {
                    let _ = ep.RegisterControlChangeNotify(&self.volume_cb);
                    self.endpoint = Some(ep);
                }
                Err(e) => crate::log(&format!("no audio endpoint: {e}")),
            }
        }
    }

    pub fn get(&self) -> (f32, bool) {
        let Some(ep) = &self.endpoint else { return (0.0, true) };
        unsafe {
            let level = ep.GetMasterVolumeLevelScalar().unwrap_or(0.0);
            let muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            (level, muted)
        }
    }

    pub fn set(&self, level: f32) {
        if let Some(ep) = &self.endpoint {
            unsafe {
                let _ = ep.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), &OWN_VOLUME_CONTEXT);
                if level > 0.0 {
                    let _ = ep.SetMute(false, &OWN_VOLUME_CONTEXT);
                }
            }
        }
    }

    pub fn toggle_mute(&self) {
        if let Some(ep) = &self.endpoint {
            let (_, muted) = self.get();
            unsafe {
                let _ = ep.SetMute(!muted, &OWN_VOLUME_CONTEXT);
            }
        }
    }
}

// ---------------------------------------------------------------- network

pub fn network_state() -> (bool, String) {
    let Ok(profile) = NetworkInformation::GetInternetConnectionProfile() else { return (false, String::new()) };
    let connected =
        profile.GetNetworkConnectivityLevel().map(|l| l == NetworkConnectivityLevel::InternetAccess).unwrap_or(false);
    let name = if profile.IsWlanConnectionProfile().unwrap_or(false) {
        profile.WlanConnectionProfileDetails().and_then(|d| d.GetConnectedSsid()).map(|s| s.to_string()).unwrap_or_default()
    } else {
        profile.ProfileName().map(|s| s.to_string()).unwrap_or_default()
    };
    (connected, name)
}

pub fn start_network_watch() {
    let handler = NetworkStatusChangedEventHandler::new(|_| {
        let (connected, name) = network_state();
        post(Event::Network { connected, name });
        Ok(())
    });
    if let Err(e) = NetworkInformation::NetworkStatusChanged(&handler) {
        crate::log(&format!("network watch unavailable: {e}"));
    }
}

// ---------------------------------------------------------------- bluetooth audio

pub fn start_bluetooth_watch() {
    std::thread::spawn(|| {
        if let Err(e) = bluetooth_watch() {
            crate::log(&format!("bluetooth watch unavailable: {e}"));
        }
    });
}

fn is_audio_device(id: &HSTRING) -> bool {
    BluetoothDevice::FromIdAsync(id)
        .and_then(|op| op.get())
        .and_then(|d| d.ClassOfDevice())
        .and_then(|c| c.MajorClass())
        .map(|m| m == BluetoothMajorClass::AudioVideo)
        .unwrap_or(false)
}

fn bluetooth_watch() -> Result<()> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    let selector = BluetoothDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected)?;
    let watcher = DeviceInformation::CreateWatcherAqsFilter(&selector)?;
    let known: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let (k, r) = (known.clone(), ready.clone());
    watcher.Added(&TypedEventHandler::new(move |_, info: windows::core::Ref<DeviceInformation>| {
        let Some(info) = info.as_ref() else { return Ok(()) };
        let id = info.Id()?;
        if !is_audio_device(&id) {
            return Ok(());
        }
        let name = info.Name()?.to_string();
        k.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), name.clone());
        if r.load(std::sync::atomic::Ordering::SeqCst) {
            post(Event::Bluetooth { name, connected: true });
        }
        Ok(())
    }))?;
    let k = known.clone();
    watcher.Removed(&TypedEventHandler::new(move |_, upd: windows::core::Ref<windows::Devices::Enumeration::DeviceInformationUpdate>| {
        let Some(upd) = upd.as_ref() else { return Ok(()) };
        if let Some(name) = k.lock().unwrap_or_else(|e| e.into_inner()).remove(&upd.Id()?.to_string()) {
            post(Event::Bluetooth { name, connected: false });
        }
        Ok(())
    }))?;
    watcher.Updated(&TypedEventHandler::new(|_, _| Ok(())))?;
    let r = ready.clone();
    watcher.EnumerationCompleted(&TypedEventHandler::new(move |_, _| {
        r.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }))?;
    watcher.Start()?;
    // The watcher lives as long as the app; park this thread to keep it referenced.
    loop {
        std::thread::park();
        let _ = &watcher;
    }
}

// ---------------------------------------------------------------- power

pub const GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS: GUID = GUID::from_u128(0x8ffee2c6_2d01_46be_adb9_398addc5b4ff);
pub const GUID_ACDC_SOURCE: GUID = GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);
pub const GUID_BATTERY_PERCENT: GUID = GUID::from_u128(0xa7ad8041_b45a_4cae_87a3_eecbb468a9e1);

pub fn register_power(hwnd: HWND) {
    for guid in [GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, GUID_ACDC_SOURCE, GUID_BATTERY_PERCENT] {
        unsafe {
            let _ = RegisterPowerSettingNotification(HANDLE(hwnd.0), &guid, DEVICE_NOTIFY_WINDOW_HANDLE);
        }
    }
}

/// (percent, charging, has battery)
pub fn battery() -> (u8, bool, bool) {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return (100, false, false);
    }
    let has = s.BatteryFlag != 128 && s.BatteryFlag != 255;
    (s.BatteryLifePercent.min(100), s.ACLineStatus == 1, has)
}

// ---------------------------------------------------------------- hooks

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && (wparam.0 as u32 == WM_KEYUP || wparam.0 as u32 == WM_SYSKEYUP) {
        // Only the virtual-key code is read, and only to recognise these keys.
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = VIRTUAL_KEY(kb.vkCode as u16);
        if vk == VK_CAPITAL {
            events::wake(WM_APP_CAPS);
        } else if matches!(vk, VK_LWIN | VK_RWIN | VK_SPACE | VK_LSHIFT | VK_RSHIFT | VK_LMENU | VK_RMENU | VK_LCONTROL | VK_RCONTROL)
        {
            events::wake(WM_APP_LANG);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn win_event_proc(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_object: i32, _: i32, _: u32, _: u32) {
    const OBJID_WINDOW: i32 = 0;
    if event == EVENT_SYSTEM_FOREGROUND || (id_object == OBJID_WINDOW && hwnd == GetForegroundWindow()) {
        events::wake(WM_APP_FOREGROUND);
    }
}

pub fn install_hooks() {
    unsafe {
        if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) {
            crate::log(&format!("keyboard hook unavailable: {e}"));
        }
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(win_event_proc), 0, 0, flags);
        SetWinEventHook(EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND, None, Some(win_event_proc), 0, 0, flags);
        SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(win_event_proc), 0, 0, flags);
    }
}

pub fn caps_lock_on() -> bool {
    unsafe { GetKeyState(VK_CAPITAL.0 as i32) & 1 == 1 }
}

/// Short language tag ("ENG", "FRA", ...) of the keyboard layout in the foreground window.
pub fn keyboard_language() -> String {
    unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let hkl = GetKeyboardLayout(thread);
        let lcid = (hkl.0 as usize & 0xffff) as u32;
        let mut name = [0u16; 85];
        if LCIDToLocaleName(lcid, Some(&mut name), 0) == 0 {
            return String::new();
        }
        let mut abbr = [0u16; 16];
        let n = GetLocaleInfoEx(PCWSTR(name.as_ptr()), LOCALE_SABBREVLANGNAME, Some(&mut abbr));
        if n <= 1 {
            return String::new();
        }
        String::from_utf16_lossy(&abbr[..n as usize - 1]).to_uppercase()
    }
}

pub fn language_display_name() -> String {
    unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let lcid = (GetKeyboardLayout(thread).0 as usize & 0xffff) as u32;
        let mut name = [0u16; 85];
        if LCIDToLocaleName(lcid, Some(&mut name), 0) == 0 {
            return String::new();
        }
        let mut full = [0u16; 128];
        let n = GetLocaleInfoEx(PCWSTR(name.as_ptr()), LOCALE_SLOCALIZEDDISPLAYNAME, Some(&mut full));
        if n <= 1 {
            return String::new();
        }
        String::from_utf16_lossy(&full[..n as usize - 1])
    }
}

/// True when the foreground app covers the whole monitor (games, videos, slideshows).
pub fn foreground_is_fullscreen(monitor: &RECT) -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == GetDesktopWindow() || fg == GetShellWindow() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(fg, &mut class);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        let borderless = style & WS_CAPTION.0 != WS_CAPTION.0;
        borderless && r.left <= monitor.left && r.top <= monitor.top && r.right >= monitor.right && r.bottom >= monitor.bottom
    }
}

pub fn idle_seconds() -> u32 {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut info).as_bool() {
            return windows::Win32::System::SystemInformation::GetTickCount().wrapping_sub(info.dwTime) / 1000;
        }
    }
    0
}

// ---------------------------------------------------------------- startup

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("SuiNotch");

pub fn set_startup(enabled: bool) {
    unsafe {
        if enabled {
            let Ok(exe) = std::env::current_exe() else { return };
            let value: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
            let err = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(value.as_ptr() as *const _),
                (value.len() * 2) as u32,
            );
            if err != ERROR_SUCCESS {
                crate::log(&format!("cannot enable start with Windows: {err:?}"));
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
    }
}

// ---------------------------------------------------------------- clipboard

pub fn copy_text(hwnd: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = (|| -> Result<()> {
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            let ptr = GlobalLock(mem) as *mut u16;
            if ptr.is_null() {
                return Err(windows::core::Error::from_win32());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(13 /* CF_UNICODETEXT */, Some(HANDLE(mem.0)))?;
            Ok(())
        })()
        .is_ok();
        let _ = CloseClipboard();
        ok
    }
}

// ---------------------------------------------------------------- monitors

#[derive(Clone, Debug)]
pub struct Monitor {
    pub device: String,
    pub rect: RECT,
    /// Top edge the notch hangs from: below a taskbar docked at the top, else the screen edge.
    pub top: i32,
    pub primary: bool,
    pub scale: f32,
}

unsafe extern "system" fn monitor_enum(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<Monitor>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
        let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        list.push(Monitor {
            device: String::from_utf16_lossy(&info.szDevice[..len]),
            rect: info.monitorInfo.rcMonitor,
            top: info.monitorInfo.rcWork.top.max(info.monitorInfo.rcMonitor.top),
            primary: info.monitorInfo.dwFlags & 1 == 1,
            scale: dx as f32 / 96.0,
        });
    }
    true.into()
}

pub fn monitors() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_enum), LPARAM(&mut list as *mut _ as isize));
    }
    list.sort_by_key(|m| (!m.primary, m.rect.left));
    list
}

pub fn pick_monitor(device: Option<&str>) -> Option<Monitor> {
    let all = monitors();
    device
        .and_then(|d| all.iter().find(|m| m.device == d).cloned())
        .or_else(|| all.iter().find(|m| m.primary).cloned())
        .or_else(|| all.first().cloned())
}

// ---------------------------------------------------------------- shell

pub fn open(path: &Path) {
    let p = HSTRING::from(path.as_os_str());
    unsafe {
        ShellExecuteW(None, w!("open"), &p, None, None, SW_SHOWNORMAL);
    }
}

pub fn open_url(url: &str) {
    let p = HSTRING::from(url);
    unsafe {
        ShellExecuteW(None, w!("open"), &p, None, None, SW_SHOWNORMAL);
    }
}

pub fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len())
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Files and text carried by a drag or the clipboard.
pub fn paths_from_hdrop(hdrop: HDROP) -> Vec<PathBuf> {
    unsafe {
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        (0..count)
            .filter_map(|i| {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
            })
            .collect()
    }
}

// ---------------------------------------------------------------- time formatting

fn systemtime(dt: &chrono::DateTime<chrono::Local>) -> SYSTEMTIME {
    use chrono::{Datelike, Timelike};
    SYSTEMTIME {
        wYear: dt.year() as u16,
        wMonth: dt.month() as u16,
        wDayOfWeek: dt.weekday().num_days_from_sunday() as u16,
        wDay: dt.day() as u16,
        wHour: dt.hour() as u16,
        wMinute: dt.minute() as u16,
        wSecond: dt.second() as u16,
        wMilliseconds: 0,
    }
}

/// Time of day in the user's Windows format, or forced to 12/24 hours.
pub fn format_time(dt: &chrono::DateTime<chrono::Local>, clock: usize) -> String {
    match clock {
        1 => return dt.format("%-I:%M %p").to_string(),
        2 => return dt.format("%H:%M").to_string(),
        _ => {}
    }
    let st = systemtime(dt);
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&st), PCWSTR::null(), Some(&mut buf)) };
    if n > 1 {
        String::from_utf16_lossy(&buf[..n as usize - 1])
    } else {
        dt.format("%H:%M").to_string()
    }
}

pub fn month_name(year: i32, month: u32) -> String {
    let st = SYSTEMTIME { wYear: year as u16, wMonth: month as u16, wDay: 1, ..Default::default() };
    let mut buf = [0u16; 64];
    let n = unsafe {
        GetDateFormatEx(PCWSTR::null(), ENUM_DATE_FORMATS_FLAGS(0), Some(&st), w!("MMMM yyyy"), Some(&mut buf), PCWSTR::null())
    };
    if n > 1 {
        String::from_utf16_lossy(&buf[..n as usize - 1])
    } else {
        format!("{month}/{year}")
    }
}

const SHOW_SETTINGS_EVENT: PCWSTR = w!(r"Local\SuiNotch.ShowSettings");

pub fn single_instance() -> bool {
    unsafe {
        let _ = windows::Win32::System::Threading::CreateMutexW(None, true, w!(r"Local\SuiNotch.SingleInstance"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Second launch: grant the running copy the foreground and ask it to show Settings.
pub fn signal_running_instance() {
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, SHOW_SETTINGS_EVENT) {
            let _ = SetEvent(ev);
            let _ = CloseHandle(ev);
        }
    }
}

/// Running copy: waits for later launches and opens Settings when one arrives.
pub fn listen_for_second_launch() {
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    let Ok(ev) = (unsafe { CreateEventW(None, false, false, SHOW_SETTINGS_EVENT) }) else { return };
    let raw = ev.0 as isize;
    std::thread::spawn(move || loop {
        let ev = HANDLE(raw as *mut _);
        if unsafe { WaitForSingleObject(ev, INFINITE) } != WAIT_OBJECT_0 {
            return;
        }
        events::wake(events::WM_APP_SHOW_SETTINGS);
    });
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn readable_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2_516_582), "2.4 MB");
    }
}
