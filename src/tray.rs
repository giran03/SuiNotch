//! System tray icon and its menu. The icon is drawn at runtime from the SuiNotch mark.

use windows::core::{w, Interface, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1RenderTarget;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{self, App, Deferred};
use crate::events::WM_APP_TRAY;
use crate::gfx::{self, Canvas, Gfx, C, R};
use crate::mascot;

const ID_SETTINGS: u16 = 10;
const ID_STARTUP: u16 = 11;
const ID_MASCOT: u16 = 12;
const ID_MINIMAL: u16 = 13;
const ID_ABOUT: u16 = 14;
const ID_EXIT: u16 = 15;
const ID_HIDE_FS: u16 = 16;

/// Draws the mark (a pill holding a droplet with two eyes) into a 32-bit icon.
pub fn make_icon(gfx: &Gfx, px: i32, accent: u32) -> Result<HICON> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: px,
                biHeight: -px,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, color.into());
        let rt = gfx::dc_render_target(gfx)?;
        rt.BindDC(dc, &RECT { left: 0, top: 0, right: px, bottom: px })?;
        rt.BeginDraw();
        {
            let target: ID2D1RenderTarget = rt.cast()?;
            let cv = Canvas::new(gfx, target)?;
            cv.clear(C::CLEAR);
            draw_mark(&cv, px as f32, C::hex(accent), true);
        }
        rt.EndDraw(None, None)?;
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
        let mask = CreateBitmap(px, px, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// Below this size the mark switches to its small-size cut (bigger droplet, no eyes).
const SMALL_CUT_BELOW: f32 = 48.0;
const INK: u32 = 0x0A0B0D;

/// The SuiNotch mark on its 256-unit grid (see brand/build_brand.py): a pill in the accent
/// colour holding Sui, a white droplet with 60° sides.
pub fn draw_mark(cv: &Canvas, size: f32, accent: C, small: bool) {
    let u = size / 256.0;
    let (pill, (tip, cy, r)) = if small {
        (R::new(8.0, 64.0, 240.0, 128.0), (78.0, 150.0, 36.0))
    } else {
        (R::new(20.0, 72.0, 216.0, 112.0), (87.0, 145.0, 29.0))
    };
    let pill = R::new(pill.x * u, pill.y * u, pill.w * u, pill.h * u);
    cv.fill_rrect(pill, pill.h / 2.0, accent);
    let drop_h = (cy + r - tip) * u;
    mascot::drop_shape(cv, 128.0 * u, (cy + r) * u, 2.0 * r * u, drop_h, C::WHITE);
    if !small {
        for dx in [-10.5, 10.5] {
            cv.fill_circle((128.0 + dx) * u, (cy + 1.0) * u, 5.6 * u, C::hex(INK));
        }
    }
}

pub fn draw_mark_at(cv: &Canvas, x: f32, y: f32, size: f32) {
    cv.translate(x, y);
    draw_mark(cv, size, C::hex(crate::config::ACCENTS[0]), size < SMALL_CUT_BELOW);
    cv.reset_transform();
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

pub fn add(app: &App) {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) * 2;
    let icon = make_icon(&app.gfx, size, app.cfg.accent_rgb()).unwrap_or_default();
    let mut d = data(app.notch.hwnd);
    d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    d.uCallbackMessage = WM_APP_TRAY;
    d.hIcon = icon;
    let tip: Vec<u16> = "SuiNotch".encode_utf16().collect();
    d.szTip[..tip.len()].copy_from_slice(&tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
    }
}

pub fn refresh_icon(app: &App) {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) * 2;
    if let Ok(icon) = make_icon(&app.gfx, size, app.cfg.accent_rgb()) {
        let mut d = data(app.notch.hwnd);
        d.uFlags = NIF_ICON;
        d.hIcon = icon;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
    }
}

pub fn on_message(app: &mut App, lparam: LPARAM) {
    let event = (lparam.0 & 0xffff) as u32;
    match event {
        e if e == WM_LBUTTONUP || e == NIN_SELECT || e == NIN_SELECT + 1 => app::defer(Deferred::OpenSettings(None)),
        WM_CONTEXTMENU | WM_RBUTTONUP => show_menu(app),
        _ => {}
    }
}

fn show_menu(app: &App) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, w!("Settings..."));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING | check(app.cfg.start_with_windows), ID_STARTUP as usize, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_STRING | check(app.cfg.mascot), ID_MASCOT as usize, w!("Sui mascot"));
        let _ = AppendMenuW(menu, MF_STRING | check(app.cfg.minimal), ID_MINIMAL as usize, w!("Minimal notch"));
        let _ = AppendMenuW(menu, MF_STRING | check(app.cfg.hide_fullscreen), ID_HIDE_FS as usize, w!("Hide in full screen"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_ABOUT as usize, w!("About SuiNotch"));
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT as usize, w!("Exit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(app.notch.hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), app.notch.hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

pub fn on_command(app: &mut App, id: u16) {
    let mut c = app.cfg.clone();
    match id {
        ID_SETTINGS => return app::defer(Deferred::OpenSettings(None)),
        ID_ABOUT => return app::defer(Deferred::OpenSettings(Some(crate::settings::ABOUT_PAGE))),
        ID_EXIT => return app::defer(Deferred::Exit),
        ID_STARTUP => c.start_with_windows = !c.start_with_windows,
        ID_MASCOT => c.mascot = !c.mascot,
        ID_MINIMAL => c.minimal = !c.minimal,
        ID_HIDE_FS => c.hide_fullscreen = !c.hide_fullscreen,
        _ => return,
    }
    app.set_config(c);
    crate::settings::refresh(app);
}
