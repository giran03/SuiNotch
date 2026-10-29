//! Settings window. Custom drawn with Direct2D: a sidebar of categories and grouped cards of
//! controls. Every change applies instantly and is saved.
#![allow(unpredictable_function_pointer_comparisons)]

use std::collections::HashMap;
use std::time::Instant;

use windows::core::{w, Interface, Result, BOOL};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::{ID2D1HwndRenderTarget, ID2D1RenderTarget};
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::Spring;
use crate::app::{self, App};
use crate::changelog;
use crate::config::{self, Config, ACCENTS};
use crate::gfx::{self, icon, Align, Canvas, C, R};
use crate::sys::{self, Monitor};
use crate::update::{self, Status};

mod controls;
mod pages;

use controls::*;

pub const ABOUT_PAGE: usize = 10;
/// "What's new": reached from About, so it has no sidebar entry of its own.
pub const CHANGELOG_PAGE: usize = 11;
pub const DEVELOPER: &str = "CafeTako";
pub const SUPPORT_EMAIL: &str = "support@cafetako.com";
const W: f32 = 800.0;
const H: f32 = 580.0;
const SIDEBAR: f32 = 228.0;
const TITLE_H: f32 = 44.0;
const WM_MOUSELEAVE: u32 = 0x02A3;
const T_ANIM: usize = 1;

const BG: C = C(0.075, 0.078, 0.09, 1.0);
const SIDE: C = C(0.105, 0.108, 0.125, 1.0);
const CARD: C = C(0.135, 0.14, 0.16, 1.0);
const LINE: C = C(1.0, 1.0, 1.0, 0.06);
const DIM: C = C(1.0, 1.0, 1.0, 0.52);

struct Category {
    name: &'static str,
    glyph: char,
    tile: u32,
}

const CATEGORIES: [Category; 11] = [
    Category { name: "General", glyph: icon::SETTINGS, tile: 0x8E8E93 },
    Category { name: "Display", glyph: icon::DISPLAY, tile: 0x5E5CE6 },
    Category { name: "Appearance", glyph: icon::COLOR, tile: 0x3A3A3C },
    Category { name: "Music and lyrics", glyph: icon::MUSIC, tile: 0xFF375F },
    Category { name: "To-do", glyph: icon::CHECK, tile: 0x30B85A },
    Category { name: "Calendar", glyph: icon::CALENDAR, tile: 0xFF453A },
    Category { name: "Pomodoro", glyph: icon::TIMER, tile: 0xFF7A1A },
    Category { name: "Water", glyph: '\u{0}', tile: 0x0A84FF },
    Category { name: "Shelf", glyph: icon::FOLDER, tile: 0xF5B400 },
    Category { name: "HUD and popups", glyph: icon::BELL, tile: 0x32ADE6 },
    Category { name: "About", glyph: icon::INFO, tile: 0x636366 },
];

type BoolField = fn(&mut Config) -> &mut bool;
type IndexField = fn(&mut Config) -> &mut usize;
type NumField = fn(&mut Config) -> &mut u32;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Act {
    Page(usize),
    /// An in-page button that switches pages (scrolls with the content, unlike the sidebar).
    Link(usize),
    Close,
    Minimize,
    Toggle(BoolField),
    Seg(IndexField, usize),
    Step(NumField, i32, u32, u32),
    Display(Option<usize>),
    Accent(usize),
    OpenData,
    ClearShelf,
    ResetWater,
    Website,
    Support,
    CheckUpdate,
    InstallUpdate,
}

pub struct SettingsWindow {
    hwnd: HWND,
    rt: Option<ID2D1HwndRenderTarget>,
    page: usize,
    scroll: Spring,
    content_h: f32,
    hits: Vec<(R, Act)>,
    hover: Option<Act>,
    pressed: Option<Act>,
    knobs: HashMap<usize, Spring>,
    segs: HashMap<usize, Spring>,
    side_sel: Spring,
    monitors: Vec<Monitor>,
    mouse: (f32, f32),
    last: Instant,
    scale: f32,
}

// ------------------------------------------------------------------ lifecycle

pub fn open(page: Option<usize>) {
    let existing = app::with(|app| app.settings.as_ref().map(|s| s.hwnd)).flatten();
    if let Some(hwnd) = existing {
        app::with(|app| {
            if let (Some(s), Some(p)) = (app.settings.as_mut(), page) {
                s.page = p;
                s.scroll.snap(0.0);
            }
        });
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        return;
    }
    let hwnd = match create_window() {
        Ok(h) => h,
        Err(e) => return crate::log(&format!("cannot open settings: {e}")),
    };
    let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    app::with(|app| {
        let page = page.unwrap_or(0);
        app.settings = Some(SettingsWindow {
            hwnd,
            rt: None,
            page,
            scroll: Spring::new(0.0, 0.32, 1.0),
            content_h: 0.0,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            knobs: HashMap::new(),
            segs: HashMap::new(),
            side_sel: Spring::new(page as f32, 0.3, 0.86),
            monitors: sys::monitors(),
            mouse: (-1.0, -1.0),
            last: Instant::now(),
            scale: scale.max(1.0),
        });
    });
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn create_window() -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("SuiNotchSettings"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let mut dpi = (96, 96);
        let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(mon, windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut dpi.0, &mut dpi.1);
        let s = dpi.0 as f32 / 96.0;
        let (w, h) = ((W * s) as i32, (H * s) as i32);
        let wa = info.rcWork;
        let x = (wa.left + wa.right - w) / 2;
        let y = (wa.top + wa.bottom - h) / 2;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SuiNotchSettings"),
            w!("SuiNotch Settings"),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinst.into()),
            None,
        )?;
        let dark: BOOL = true.into();
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as *const _, 4);
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        Ok(hwnd)
    }
}

pub fn refresh(app: &App) {
    if let Some(s) = &app.settings {
        unsafe {
            let _ = InvalidateRect(Some(s.hwnd), None, false);
        }
    }
}

fn close(app: &mut App) {
    if let Some(s) = app.settings.take() {
        unsafe {
            let _ = DestroyWindow(s.hwnd);
        }
    }
}

// ------------------------------------------------------------------ window procedure

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCALCSIZE && wparam.0 != 0 {
        return LRESULT(0);
    }
    let r = app::with(|app| handle(app, hwnd, msg, wparam, lparam)).flatten();
    r.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
}

fn handle(app: &mut App, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if app.settings.as_ref().map(|s| s.hwnd) != Some(hwnd) {
        return None;
    }
    let scale = app.settings.as_ref().map(|s| s.scale).unwrap_or(1.0);
    let pt = |lp: LPARAM| ((lp.0 & 0xffff) as i16 as f32 / scale, ((lp.0 >> 16) & 0xffff) as i16 as f32 / scale);
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut ps) };
            paint(app);
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            Some(LRESULT(0))
        }
        WM_ERASEBKGND => Some(LRESULT(1)),
        WM_NCHITTEST => {
            let mut p = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
            unsafe {
                let _ = ScreenToClient(hwnd, &mut p);
            }
            let (x, y) = (p.x as f32 / scale, p.y as f32 / scale);
            let s = app.settings.as_ref()?;
            let on_button = s.hits.iter().any(|(r, a)| matches!(a, Act::Close | Act::Minimize) && r.contains(x, y));
            Some(LRESULT(if y < TITLE_H && !on_button { HTCAPTION as isize } else { HTCLIENT as isize }))
        }
        WM_SIZE => {
            let s = app.settings.as_mut()?;
            if let Some(rt) = &s.rt {
                let (w, h) = ((lparam.0 & 0xffff) as u32, ((lparam.0 >> 16) & 0xffff) as u32);
                unsafe {
                    let _ = rt.Resize(&D2D_SIZE_U { width: w, height: h });
                }
            }
            None
        }
        WM_DPICHANGED => {
            let s = app.settings.as_mut()?;
            s.scale = ((wparam.0 & 0xffff) as f32 / 96.0).max(1.0);
            s.rt = None;
            let r = unsafe { &*(lparam.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            let (x, y) = pt(lparam);
            let s = app.settings.as_mut()?;
            s.mouse = (x, y);
            let hover = s.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a);
            if hover != s.hover {
                s.hover = hover;
                invalidate(s);
            }
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            Some(LRESULT(0))
        }
        WM_MOUSELEAVE => {
            let s = app.settings.as_mut()?;
            s.hover = None;
            invalidate(s);
            Some(LRESULT(0))
        }
        WM_LBUTTONDOWN => {
            let s = app.settings.as_mut()?;
            s.pressed = s.hover;
            invalidate(s);
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            let (x, y) = pt(lparam);
            let s = app.settings.as_mut()?;
            let hit = s.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a);
            let pressed = s.pressed.take();
            invalidate(s);
            if let (Some(a), Some(p)) = (hit, pressed) {
                if a == p {
                    act(app, a);
                }
            }
            Some(LRESULT(0))
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
            let s = app.settings.as_mut()?;
            let max = (s.content_h - (H - TITLE_H - 16.0)).max(0.0);
            let t = (s.scroll.target - delta * 60.0).clamp(0.0, max);
            s.scroll.set(t);
            invalidate(s);
            Some(LRESULT(0))
        }
        WM_KEYDOWN => {
            let vk = VIRTUAL_KEY(wparam.0 as u16);
            let s = app.settings.as_mut()?;
            match vk {
                VK_ESCAPE => close(app),
                VK_UP | VK_DOWN => {
                    let n = CATEGORIES.len() as i32;
                    let d = if vk == VK_UP { -1 } else { 1 };
                    s.page = ((s.page.min(ABOUT_PAGE) as i32 + d).rem_euclid(n)) as usize;
                    s.scroll.snap(0.0);
                    invalidate(s);
                }
                _ => {}
            }
            Some(LRESULT(0))
        }
        WM_TIMER if wparam.0 == T_ANIM => {
            paint(app);
            Some(LRESULT(0))
        }
        WM_CLOSE => {
            close(app);
            Some(LRESULT(0))
        }
        _ => None,
    }
}

fn invalidate(s: &SettingsWindow) {
    unsafe {
        let _ = InvalidateRect(Some(s.hwnd), None, false);
    }
}

fn act(app: &mut App, a: Act) {
    let mut c = app.cfg.clone();
    match a {
        Act::Page(p) | Act::Link(p) => {
            if let Some(s) = app.settings.as_mut() {
                s.page = p;
                s.scroll.snap(0.0);
                invalidate(s);
            }
            return;
        }
        Act::Close => return close(app),
        Act::Minimize => {
            if let Some(s) = &app.settings {
                unsafe {
                    let _ = ShowWindow(s.hwnd, SW_MINIMIZE);
                }
            }
            return;
        }
        Act::Toggle(f) => {
            let v = f(&mut c);
            *v = !*v;
        }
        Act::Seg(f, i) => *f(&mut c) = i,
        Act::Step(f, d, lo, hi) => {
            let v = f(&mut c);
            *v = (*v as i64 + d as i64).clamp(lo as i64, hi as i64) as u32;
        }
        Act::Display(i) => {
            c.display = i.and_then(|i| app.settings.as_ref().and_then(|s| s.monitors.get(i)).map(|m| m.device.clone()));
        }
        Act::Accent(i) => c.accent = i,
        Act::OpenData => {
            let dir = config::data_dir();
            let _ = std::fs::create_dir_all(&dir);
            sys::open(&dir);
            return;
        }
        Act::ClearShelf => {
            app.shelf.items.clear();
            app.notch.bitmaps.clear();
            app.save_shelf();
            crate::notch::notice(app, "Shelf cleared");
            return;
        }
        Act::ResetWater => {
            let now = crate::store::now();
            while app.water.undo_today(now) {}
            app.save_water();
            crate::notch::schedule(app);
            crate::notch::request_frame(app);
            return;
        }
        Act::Website => {
            sys::open_url("https://lrclib.net");
            return;
        }
        Act::Support => {
            sys::open_url(&format!("mailto:{SUPPORT_EMAIL}?subject=SuiNotch%20{}", env!("CARGO_PKG_VERSION")));
            return;
        }
        Act::CheckUpdate => {
            update::check_now();
            return;
        }
        Act::InstallUpdate => {
            if let Status::Available(release) = &app.update {
                update::install(release.clone());
            }
            return;
        }
    }
    let accent_changed = c.accent != app.cfg.accent;
    app.set_config(c);
    if accent_changed {
        crate::tray::refresh_icon(app);
    }
    if let Some(s) = &app.settings {
        invalidate(s);
    }
}

// ------------------------------------------------------------------ painting

fn paint(app: &mut App) {
    let Some(mut s) = app.settings.take() else { return };
    if let Err(e) = paint_window(app, &mut s) {
        crate::log(&format!("settings paint failed: {e}"));
        s.rt = None;
    }
    app.settings = Some(s);
}

fn paint_window(app: &mut App, s: &mut SettingsWindow) -> Result<()> {
    let now = Instant::now();
    let dt = (now - s.last).as_secs_f32().min(0.05);
    s.last = now;
    if s.rt.is_none() {
        let mut rc = RECT::default();
        unsafe { GetClientRect(s.hwnd, &mut rc)? };
        s.rt = Some(gfx::hwnd_render_target(&app.gfx, s.hwnd, rc.right as u32, rc.bottom as u32, 96.0 * s.scale)?);
    }
    let rt = s.rt.clone().expect("render target");
    let gfx = app.gfx.clone();
    s.side_sel.set(s.page.min(ABOUT_PAGE) as f32);
    let mut moving = s.scroll.step(dt) | s.side_sel.step(dt);
    unsafe { rt.BeginDraw() };
    {
        let target: ID2D1RenderTarget = rt.cast()?;
        let mut cv = Canvas::new(&gfx, target)?;
        s.hits.clear();
        cv.clear(BG);
        draw_sidebar(&cv, s);
        let mut ui = Ui { cv: &mut cv, s, y: 0.0, x: SIDEBAR + 28.0, w: W - SIDEBAR - 56.0, row_index: 0, moving: false, dt };
        ui.page(app);
        moving |= ui.moving;
        draw_title_buttons(&cv, s);
    }
    let end = unsafe { rt.EndDraw(None, None) };
    if end.is_err() {
        s.rt = None;
    }
    unsafe {
        if moving {
            SetTimer(Some(s.hwnd), T_ANIM, 16, None);
        } else {
            let _ = KillTimer(Some(s.hwnd), T_ANIM);
        }
    }
    Ok(())
}

fn hovered(s: &SettingsWindow, a: Act) -> bool {
    s.hover == Some(a)
}

fn draw_sidebar(cv: &Canvas, s: &mut SettingsWindow) {
    cv.fill_rect(R::new(0.0, 0.0, SIDEBAR, H), SIDE);
    cv.line(SIDEBAR, 0.0, SIDEBAR, H, LINE, 1.0);
    // Brand
    crate::tray::draw_mark_at(cv, 22.0, 14.0, 28.0);
    cv.display("SuiNotch", R::new(56.0, 12.0, 140.0, 30.0), 15.0, 700, C::WHITE, Align::Left);
    let item_h = 34.0;
    let top = 60.0;
    let y_of = |i: usize| if i == ABOUT_PAGE { H - 18.0 - item_h } else { top + i as f32 * item_h };
    // Sliding selection highlight between two neighbours of the spring position.
    let p = s.side_sel.value;
    let (a, b) = (p.floor() as usize, (p.ceil() as usize).min(ABOUT_PAGE));
    let t = p - p.floor();
    let sel_y = y_of(a) + (y_of(b) - y_of(a)) * t;
    cv.fill_rrect(R::new(12.0, sel_y, SIDEBAR - 24.0, item_h - 2.0), 9.0, C::WHITE.a(0.09));
    for (i, cat) in CATEGORIES.iter().enumerate() {
        let r = R::new(12.0, y_of(i), SIDEBAR - 24.0, item_h - 2.0);
        s.hits.push((r, Act::Page(i)));
        let current = s.page.min(ABOUT_PAGE) == i;
        if hovered(s, Act::Page(i)) && !current {
            cv.fill_rrect(r, 9.0, C::WHITE.a(0.04));
        }
        let tile = R::new(r.x + 8.0, r.cy() - 11.0, 22.0, 22.0);
        cv.fill_rrect(tile, 6.0, C::hex(cat.tile));
        if cat.glyph == '\u{0}' {
            crate::pages::water_icon(cv, tile.cx(), tile.cy(), 12.0, C::WHITE);
        } else {
            cv.glyph(cat.glyph, tile.cx(), tile.cy(), 11.5, C::WHITE);
        }
        cv.text(cat.name, R::new(tile.right() + 12.0, r.y, r.w - 50.0, r.h), 13.0, if current { 600 } else { 500 }, C::WHITE, Align::Left);
    }
}

fn draw_title_buttons(cv: &Canvas, s: &mut SettingsWindow) {
    let close = R::new(W - 46.0, 0.0, 46.0, 34.0);
    let min = R::new(W - 92.0, 0.0, 46.0, 34.0);
    s.hits.push((close, Act::Close));
    s.hits.push((min, Act::Minimize));
    if hovered(s, Act::Close) {
        cv.fill_rect(close, C::hex(0xC42B1C));
    }
    if hovered(s, Act::Minimize) {
        cv.fill_rect(min, C::WHITE.a(0.07));
    }
    cv.glyph(icon::CLOSE, close.cx(), close.cy(), 10.0, C::WHITE);
    cv.glyph('\u{E921}', min.cx(), min.cy(), 10.0, C::WHITE);
}
