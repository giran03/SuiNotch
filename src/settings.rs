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

// ------------------------------------------------------------------ immediate-mode controls

struct Ui<'a, 'b> {
    cv: &'a mut Canvas<'b>,
    s: &'a mut SettingsWindow,
    y: f32,
    x: f32,
    w: f32,
    row_index: usize,
    moving: bool,
    dt: f32,
}

const ROW: f32 = 48.0;
const ROW_SUB: f32 = 56.0;
/// Line height of release notes on the "What's new" page.
const NOTE_LINE: f32 = 19.0;

impl Ui<'_, '_> {
    fn accent(cfg: &Config) -> C {
        C::hex(cfg.accent_rgb())
    }

    fn title(&mut self, t: &str) {
        self.cv.display(t, R::new(self.x, self.y, self.w, 34.0), 22.0, 700, C::WHITE, Align::Left);
        self.y += 46.0;
    }

    fn section(&mut self, label: &str) {
        self.cv.text(label, R::new(self.x + 4.0, self.y, self.w, 18.0), 12.0, 600, DIM, Align::Left);
        self.y += 22.0;
    }

    /// Draws a card of `rows` heights and returns their rects.
    fn card(&mut self, heights: &[f32]) -> Vec<R> {
        let total: f32 = heights.iter().sum();
        let r = R::new(self.x, self.y, self.w, total);
        self.cv.fill_rrect(r, 12.0, CARD);
        let mut rows = Vec::new();
        let mut y = self.y;
        for (i, h) in heights.iter().enumerate() {
            if i > 0 {
                self.cv.line(self.x + 16.0, y, self.x + self.w - 16.0, y, LINE, 1.0);
            }
            rows.push(R::new(self.x, y, self.w, *h));
            y += h;
        }
        self.y = y + 18.0;
        rows
    }

    fn label(&mut self, r: R, label: &str, sub: Option<&str>) {
        match sub {
            Some(sub) => {
                self.cv.text(label, R::new(r.x + 16.0, r.y + 9.0, r.w * 0.62, 20.0), 13.5, 550, C::WHITE, Align::Left);
                self.cv.text(sub, R::new(r.x + 16.0, r.y + 29.0, r.w * 0.62, 17.0), 11.5, 500, DIM, Align::Left);
            }
            None => self.cv.text(label, R::new(r.x + 16.0, r.y, r.w * 0.62, r.h), 13.5, 550, C::WHITE, Align::Left),
        }
    }

    fn toggle(&mut self, cfg: &Config, r: R, label: &str, sub: Option<&str>, f: BoolField) {
        self.label(r, label, sub);
        let on = *f(&mut cfg.clone());
        let id = f as usize;
        let knob = self.s.knobs.entry(id).or_insert_with(|| Spring::new(if on { 1.0 } else { 0.0 }, 0.28, 0.72));
        knob.set(if on { 1.0 } else { 0.0 });
        self.moving |= knob.step(self.dt);
        let t = knob.value;
        let sw = R::new(r.right() - 16.0 - 44.0, r.cy() - 12.0, 44.0, 24.0);
        let act = Act::Toggle(f);
        self.s.hits.push((r, act));
        let off = C::WHITE.a(if hovered(self.s, act) { 0.22 } else { 0.16 });
        self.cv.fill_rrect(sw, 12.0, off.mix(Self::accent(cfg), t.clamp(0.0, 1.0)));
        let kx = sw.x + 12.0 + t * 20.0;
        let stretch = if self.s.pressed == Some(act) { 4.0 } else { 0.0 };
        self.cv.fill_rrect(R::new(kx - 9.0 - stretch * (1.0 - t), sw.cy() - 9.0, 18.0 + stretch, 18.0), 9.0, C::WHITE);
    }

    fn segmented(&mut self, cfg: &Config, r: R, labels: &[&str], f: IndexField) {
        let sel = *f(&mut cfg.clone());
        let id = f as usize;
        let spring = self.s.segs.entry(id).or_insert_with(|| Spring::new(sel as f32, 0.3, 0.82));
        spring.set(sel as f32);
        self.moving |= spring.step(self.dt);
        let pos = spring.value;
        let bar = R::new(r.x, r.y, r.w, 30.0);
        self.cv.fill_rrect(bar, 8.0, C::WHITE.a(0.06));
        let seg_w = bar.w / labels.len() as f32;
        self.cv.fill_rrect(R::new(bar.x + 2.0 + pos * seg_w, bar.y + 2.0, seg_w - 4.0, bar.h - 4.0), 6.5, C::WHITE.a(0.16));
        for (i, l) in labels.iter().enumerate() {
            let sr = R::new(bar.x + i as f32 * seg_w, bar.y, seg_w, bar.h);
            let act = Act::Seg(f, i);
            self.s.hits.push((sr, act));
            if hovered(self.s, act) && i != sel {
                self.cv.fill_rrect(sr.inset(2.0, 2.0), 6.5, C::WHITE.a(0.05));
            }
            self.cv.text(l, sr, 12.0, if i == sel { 650 } else { 500 }, if i == sel { C::WHITE } else { DIM }, Align::Center);
        }
    }

    fn seg_row(&mut self, cfg: &Config, r: R, label: &str, sub: Option<&str>, labels: &[&str], f: IndexField) {
        let lr = R::new(r.x, r.y, r.w, if sub.is_some() { 50.0 } else { 38.0 });
        self.label(lr.offset(0.0, if sub.is_some() { 0.0 } else { 4.0 }), label, sub);
        self.segmented(cfg, R::new(r.x + 16.0, r.bottom() - 42.0, r.w - 32.0, 30.0), labels, f);
    }

    #[allow(clippy::too_many_arguments)]
    fn stepper(&mut self, cfg: &Config, r: R, label: &str, sub: Option<&str>, f: NumField, step: i32, lo: u32, hi: u32, fmt: &dyn Fn(u32) -> String) {
        self.label(r, label, sub);
        let v = *f(&mut cfg.clone());
        let plus = R::centered(r.right() - 32.0, r.cy(), 28.0, 28.0);
        let value_w = 92.0;
        let value = R::new(plus.x - 4.0 - value_w, r.cy() - 14.0, value_w, 28.0);
        let minus = R::centered(value.x - 18.0, r.cy(), 28.0, 28.0);
        for (rect, d, g) in [(minus, -step, '\u{E738}'), (plus, step, icon::ADD)] {
            let act = Act::Step(f, d, lo, hi);
            self.s.hits.push((rect, act));
            let disabled = (d < 0 && v <= lo) || (d > 0 && v >= hi);
            self.cv.fill_circle(rect.cx(), rect.cy(), 14.0, C::WHITE.a(if hovered(self.s, act) && !disabled { 0.16 } else { 0.08 }));
            self.cv.glyph(g, rect.cx(), rect.cy(), 10.0, if disabled { C::WHITE.a(0.25) } else { C::WHITE });
        }
        self.cv.text(&fmt(v), value, 13.0, 650, C::WHITE, Align::Center);
    }

    fn button(&mut self, r: R, label: &str, sub: Option<&str>, text: &str, act: Act, danger: bool) {
        self.label(r, label, sub);
        let bw = self.cv.gfx.text_width(text, 12.5, 600) + 30.0;
        let b = R::new(r.right() - 16.0 - bw, r.cy() - 14.0, bw, 28.0);
        self.s.hits.push((b, act));
        let base = if danger { C::hex(0xFF453A).a(0.2) } else { C::WHITE.a(0.1) };
        self.cv.fill_rrect(b, 14.0, if hovered(self.s, act) { base.mix(C::WHITE, 0.08) } else { base });
        self.cv.text(text, b, 12.5, 600, if danger { C::hex(0xFF8A80) } else { C::WHITE }, Align::Center);
    }

    fn page(&mut self, app: &App) {
        let cfg = app.cfg.clone();
        let scroll = self.s.scroll.value;
        let clip = R::new(SIDEBAR + 1.0, TITLE_H - 8.0, W - SIDEBAR - 1.0, H - TITLE_H + 8.0);
        self.cv.push_clip_rrect(clip, 0.0, 1.0);
        self.y = TITLE_H - 10.0 - scroll;
        let start = self.y;
        self.row_index = 0;
        match self.s.page {
            0 => self.general(&cfg),
            1 => self.display(&cfg),
            2 => self.appearance(&cfg),
            3 => self.music(&cfg),
            4 => self.todo(&cfg),
            5 => self.calendar(&cfg),
            6 => self.pomodoro(&cfg),
            7 => self.water(&cfg),
            8 => self.shelf(&cfg, app.shelf.items.len()),
            9 => self.hud(&cfg),
            CHANGELOG_PAGE => self.changelog(),
            _ => self.about(&cfg, &app.update),
        }
        self.s.content_h = self.y - start;
        self.cv.pop();
        // Keep hits from scrolled-out content from stealing clicks in the title area.
        self.s.hits.retain(|(r, a)| matches!(a, Act::Page(_)) || r.bottom() > TITLE_H - 8.0);
    }

    fn general(&mut self, c: &Config) {
        self.title("General");
        self.section("Startup");
        let r = self.card(&[ROW]);
        self.toggle(c, r[0], "Start with Windows", None, |c| &mut c.start_with_windows);
        self.section("Notch");
        let r = self.card(&[86.0, ROW_SUB, ROW_SUB]);
        self.seg_row(c, r[0], "Size", Some("Applies right away. Auto is larger on wide monitors"), &config::SIZE_LABELS, |c| &mut c.size);
        self.toggle(c, r[1], "Minimal notch", Some("A thin line when nothing is playing"), |c| &mut c.minimal);
        self.toggle(c, r[2], "Sui mascot", Some("A little droplet that lives in the notch"), |c| &mut c.mascot);
        self.section("Sounds");
        let r = self.card(&[ROW_SUB]);
        self.toggle(c, r[0], "Reminder sounds", Some("Chimes for Pomodoro, To-do and Water"), |c| &mut c.sounds);
        self.section("Full screen");
        let r = self.card(&[ROW_SUB]);
        self.toggle(c, r[0], "Hide in full screen", Some("Games, videos and slides that fill the screen"), |c| &mut c.hide_fullscreen);
    }

    fn display(&mut self, c: &Config) {
        self.title("Display");
        self.section("Show the notch on");
        let monitors = self.s.monitors.clone();
        let mut heights = vec![ROW];
        heights.extend(std::iter::repeat_n(ROW, monitors.len()));
        let rows = self.card(&heights);
        let accent = Self::accent(c);
        let auto = c.display.is_none() || !monitors.iter().any(|m| Some(&m.device) == c.display.as_ref());
        self.radio(rows[0], "Main display", Some("Follows the primary monitor"), auto, Act::Display(None), accent);
        for (i, m) in monitors.iter().enumerate() {
            let size = format!("{} x {}", m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
            let name = format!("Display {}{}", i + 1, if m.primary { " (primary)" } else { "" });
            let selected = !auto && c.display.as_deref() == Some(m.device.as_str());
            self.radio_with_meta(rows[i + 1], &name, &size, selected, Act::Display(Some(i)), accent);
        }
        self.section("Behaviour");
        let r = self.card(&[ROW_SUB, 86.0]);
        self.toggle(c, r[0], "Open on hover", Some("Otherwise click the notch to open it"), |c| &mut c.expand_on_hover);
        self.seg_row(c, r[1], "Hover delay", Some("How long to rest on the notch before it opens"), &config::HOVER_DELAY_LABELS, |c| &mut c.hover_delay);
    }

    fn radio(&mut self, r: R, label: &str, sub: Option<&str>, on: bool, act: Act, accent: C) {
        self.s.hits.push((r, act));
        if hovered(self.s, act) {
            self.cv.fill_rrect(r.inset(4.0, 3.0), 9.0, C::WHITE.a(0.03));
        }
        self.label(r, label, sub);
        let (cx, cy) = (r.right() - 30.0, r.cy());
        if on {
            self.cv.fill_circle(cx, cy, 10.0, accent);
            self.cv.glyph(icon::CHECK, cx, cy, 10.0, crate::views::readable_on(accent));
        } else {
            self.cv.stroke_circle(cx, cy, 9.0, C::WHITE.a(0.3), 1.5);
        }
    }

    fn radio_with_meta(&mut self, r: R, label: &str, meta: &str, on: bool, act: Act, accent: C) {
        self.radio(r, label, None, on, act, accent);
        self.cv.text(meta, R::new(r.right() - 220.0, r.y, 170.0, r.h), 12.0, 500, DIM, Align::Right);
    }

    fn appearance(&mut self, c: &Config) {
        self.title("Appearance");
        self.section("Accent colour");
        let r = self.card(&[64.0]);
        let row = r[0];
        for (i, rgb) in ACCENTS.iter().enumerate() {
            let cx = row.x + 32.0 + i as f32 * 46.0;
            let act = Act::Accent(i);
            self.s.hits.push((R::centered(cx, row.cy(), 40.0, 40.0), act));
            let col = C::hex(*rgb);
            let grow = if hovered(self.s, act) { 1.5 } else { 0.0 };
            self.cv.fill_circle(cx, row.cy(), 13.0 + grow, col);
            if c.accent == i {
                self.cv.stroke_circle(cx, row.cy(), 18.0, col, 2.0);
            }
        }
        self.section("Shape");
        let r = self.card(&[86.0, ROW_SUB, ROW_SUB]);
        self.seg_row(c, r[0], "Corner roundness", Some("How soft the bottom corners of the notch are"), &config::ROUNDNESS_LABELS, |c| &mut c.roundness);
        self.toggle(c, r[1], "Frosted glass", Some("A slightly translucent notch with a fine edge"), |c| &mut c.glass);
        self.toggle(c, r[2], "Shadow", Some("A soft shadow under the open notch"), |c| &mut c.shadow);
        self.section("Motion");
        let r = self.card(&[86.0]);
        self.seg_row(c, r[0], "Animation style", Some("Spring feel for opening, closing and switching pages"), &config::ANIMATION_LABELS, |c| &mut c.animation);
        self.section("Clock");
        let r = self.card(&[86.0]);
        self.seg_row(c, r[0], "Time format", Some("System follows your Windows region settings"), &config::CLOCK_LABELS, |c| &mut c.clock_format);
    }

    fn music(&mut self, c: &Config) {
        self.title("Music and lyrics");
        self.section("Lyrics");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Open lyrics automatically", Some("Show the lyrics panel whenever the Music page opens"), |c| &mut c.lyrics_auto);
        self.toggle(c, r[1], "Lyrics under the notch", Some("Each line lights up in a pill below the closed notch"), |c| &mut c.lyrics_pill);
        self.section("Now playing");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Equalizer animation", Some("Moving bars in the closed notch while music plays"), |c| &mut c.equalizer);
        self.toggle(c, r[1], "Artwork glow", Some("Tint the notch with the colours of the album cover"), |c| &mut c.artwork_glow);
        self.toggle(c, r[2], "Show player name", Some("Spotify, Apple Music, Chrome and others"), |c| &mut c.show_source);
        self.section("Privacy");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Lyrics come from LRCLIB", Some("Only the song title, artist, album and length are sent"), "lrclib.net", Act::Website, false);
    }

    fn todo(&mut self, c: &Config) {
        self.title("To-do");
        self.section("Reminders");
        let r = self.card(&[86.0]);
        self.seg_row(c, r[0], "Snooze for", Some("When a \"Time's up!\" reminder is snoozed"), &config::SNOOZE_LABELS, |c| &mut c.todo_snooze);
        self.section("List");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Show completed tasks", Some("Keep finished tasks visible, crossed out"), |c| &mut c.todo_show_done);
        self.toggle(c, r[1], "Clear completed tasks daily", Some("Finished tasks disappear the next morning"), |c| &mut c.todo_clear_done_daily);
    }

    fn calendar(&mut self, c: &Config) {
        self.title("Calendar");
        self.section("Calendar page");
        let r = self.card(&[ROW_SUB, 86.0, ROW_SUB]);
        self.toggle(c, r[0], "Show the Calendar page", Some("A month view of your tasks in the notch"), |c| &mut c.calendar_page);
        let mut idx = if c.week_starts_monday { 0usize } else { 1 };
        let _ = &mut idx;
        self.label(R::new(r[1].x, r[1].y, r[1].w, 38.0).offset(0.0, 4.0), "Week starts on", None);
        let bar = R::new(r[1].x + 16.0, r[1].bottom() - 42.0, r[1].w - 32.0, 30.0);
        self.week_seg(c, bar);
        self.toggle(c, r[2], "Task dots", Some("Mark days that have tasks"), |c| &mut c.calendar_dots);
    }

    fn week_seg(&mut self, c: &Config, bar: R) {
        let monday = c.week_starts_monday;
        self.cv.fill_rrect(bar, 8.0, C::WHITE.a(0.06));
        let half = bar.w / 2.0;
        let sel = if monday { 0 } else { 1 };
        self.cv.fill_rrect(R::new(bar.x + 2.0 + sel as f32 * half, bar.y + 2.0, half - 4.0, bar.h - 4.0), 6.5, C::WHITE.a(0.16));
        for (i, l) in ["Monday", "Sunday"].iter().enumerate() {
            let sr = R::new(bar.x + i as f32 * half, bar.y, half, bar.h);
            if i != sel {
                self.s.hits.push((sr, Act::Toggle(|c| &mut c.week_starts_monday)));
            }
            self.cv.text(l, sr, 12.0, if i == sel { 650 } else { 500 }, if i == sel { C::WHITE } else { DIM }, Align::Center);
        }
    }

    fn pomodoro(&mut self, c: &Config) {
        self.title("Pomodoro");
        self.section("Durations");
        let mins = |v: u32| format!("{v} min");
        let r = self.card(&[ROW, ROW, ROW, ROW]);
        self.stepper(c, r[0], "Focus", None, |c| &mut c.focus_min, 5, 5, 120, &mins);
        self.stepper(c, r[1], "Short break", None, |c| &mut c.short_break_min, 1, 1, 30, &mins);
        self.stepper(c, r[2], "Long break", None, |c| &mut c.long_break_min, 5, 5, 60, &mins);
        self.stepper(c, r[3], "Sessions before a long break", None, |c| &mut c.sessions_before_long, 1, 2, 8, &|v| v.to_string());
        self.section("Flow");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Start breaks automatically", Some("Begin the break as soon as focus ends"), |c| &mut c.auto_start_breaks);
        self.toggle(c, r[1], "Start focus automatically", Some("Begin the next session when a break ends"), |c| &mut c.auto_start_focus);
    }

    fn water(&mut self, c: &Config) {
        self.title("Water");
        self.section("Reminders");
        let hour = |v: u32| format!("{v:02}:00");
        let r = self.card(&[ROW_SUB, ROW, ROW, ROW]);
        self.toggle(c, r[0], "Water reminders", Some("A gentle nudge during your hours until the goal is met"), |c| &mut c.water_enabled);
        self.stepper(c, r[1], "Remind every", None, |c| &mut c.water_interval_min, 15, 15, 240, &|v| format!("{v} min"));
        self.stepper(c, r[2], "From", None, |c| &mut c.water_start_hour, 1, 0, 22, &hour);
        self.stepper(c, r[3], "Until", None, |c| &mut c.water_end_hour, 1, 1, 24, &hour);
        self.section("Amounts");
        let r = self.card(&[ROW_SUB, ROW_SUB]);
        self.stepper(c, r[0], "Glass size", Some("Added each time you press Drink"), |c| &mut c.glass_ml, 50, 50, 2000, &|v| format!("{v} ml"));
        self.stepper(c, r[1], "Daily goal", Some("Pick any goal that suits you"), |c| &mut c.goal_ml, 100, 500, 6000, &|v| format!("{:.1} L", v as f32 / 1000.0));
        self.section("Today");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Reset today's log", Some("Removes every drink logged today"), "Reset", Act::ResetWater, true);
    }

    fn shelf(&mut self, c: &Config, count: usize) {
        self.title("Shelf");
        self.section("Behaviour");
        let r = self.card(&[ROW_SUB, ROW_SUB, ROW_SUB]);
        self.toggle(c, r[0], "Open the shelf when dragging", Some("The notch opens as files move over it"), |c| &mut c.shelf_open_on_drag);
        self.toggle(c, r[1], "Remove after dragging out", Some("Otherwise items stay until you remove them"), |c| &mut c.shelf_remove_after_drag);
        self.toggle(c, r[2], "Show file sizes", None, |c| &mut c.shelf_show_sizes);
        self.section("Items");
        let r = self.card(&[ROW_SUB]);
        let sub = format!("{count} item{} on the shelf. Files themselves are not touched", if count == 1 { "" } else { "s" });
        self.button(r[0], "Clear the shelf", Some(&sub), "Clear all", Act::ClearShelf, true);
    }

    fn hud(&mut self, c: &Config) {
        self.title("HUD and popups");
        self.section("Show an overlay for");
        let r = self.card(&[ROW, ROW, ROW, ROW, ROW, ROW, ROW]);
        self.toggle(c, r[0], "Volume", None, |c| &mut c.hud_volume);
        self.toggle(c, r[1], "Brightness", None, |c| &mut c.hud_brightness);
        self.toggle(c, r[2], "Caps Lock", None, |c| &mut c.hud_caps);
        self.toggle(c, r[3], "Keyboard language", None, |c| &mut c.hud_language);
        self.toggle(c, r[4], "Battery and charger", None, |c| &mut c.hud_battery);
        self.toggle(c, r[5], "Wi-Fi", None, |c| &mut c.hud_wifi);
        self.toggle(c, r[6], "Headphones", None, |c| &mut c.hud_headphones);
        self.section("Style");
        let r = self.card(&[86.0, 86.0]);
        self.seg_row(c, r[0], "Layout", Some("Compact fits the notch, Wide adds a label"), &config::HUD_STYLE_LABELS, |c| &mut c.hud_style);
        self.seg_row(c, r[1], "Stays for", Some("How long an overlay is shown"), &config::HUD_DURATION_LABELS, |c| &mut c.hud_duration);
    }

    fn changelog(&mut self) {
        self.title("What's new");
        let version = env!("CARGO_PKG_VERSION");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], &format!("SuiNotch {version}"), Some("Release notes for the version you're running"), "Back to About", Act::Link(ABOUT_PAGE), false);
        let blocks = changelog::parse(changelog::TEXT);
        let mut bullets: Vec<&str> = Vec::new();
        for block in &blocks {
            if let changelog::Block::Bullet(b) = block {
                bullets.push(b);
                continue;
            }
            self.bullet_card(&bullets);
            bullets.clear();
            match block {
                changelog::Block::Heading(h) => self.section(h),
                changelog::Block::Text(t) => {
                    let gfx = self.cv.gfx;
                    for line in changelog::wrap(t, self.w - 8.0, |s| gfx.text_width(s, 13.0, 500)) {
                        self.cv.text(&line, R::new(self.x + 4.0, self.y, self.w - 8.0, NOTE_LINE), 13.0, 500, DIM, Align::Left);
                        self.y += NOTE_LINE;
                    }
                    self.y += 14.0;
                }
                changelog::Block::Bullet(_) => {}
            }
        }
        self.bullet_card(&bullets);
    }

    /// One card per run of bullets, one row per bullet, wrapped to the card width.
    fn bullet_card(&mut self, bullets: &[&str]) {
        if bullets.is_empty() {
            return;
        }
        const PAD: f32 = 16.0;
        const INDENT: f32 = 16.0;
        let text_w = self.w - PAD * 2.0 - INDENT;
        let gfx = self.cv.gfx;
        let wrapped: Vec<Vec<String>> = bullets.iter().map(|b| changelog::wrap(b, text_w, |s| gfx.text_width(s, 13.0, 500))).collect();
        let heights: Vec<f32> = wrapped.iter().map(|l| l.len() as f32 * NOTE_LINE + 20.0).collect();
        let rows = self.card(&heights);
        for (row, lines) in rows.iter().zip(&wrapped) {
            let top = row.y + 10.0;
            self.cv.fill_circle(row.x + PAD + 3.0, top + NOTE_LINE / 2.0, 2.5, DIM);
            for (i, line) in lines.iter().enumerate() {
                let lr = R::new(row.x + PAD + INDENT, top + i as f32 * NOTE_LINE, text_w, NOTE_LINE);
                self.cv.text(line, lr, 13.0, 500, C::WHITE, Align::Left);
            }
        }
    }

    fn about(&mut self, cfg: &Config, update: &Status) {
        self.title("About");
        let r = self.card(&[150.0]);
        let row = r[0];
        crate::tray::draw_mark_at(self.cv, row.x + 24.0, row.y + 30.0, 88.0);
        self.cv.display("SuiNotch", R::new(row.x + 130.0, row.y + 34.0, 300.0, 34.0), 26.0, 700, C::WHITE, Align::Left);
        self.cv.text(&format!("Version {}  ·  by {DEVELOPER}", env!("CARGO_PKG_VERSION")), R::new(row.x + 130.0, row.y + 70.0, 300.0, 18.0), 12.5, 500, DIM, Align::Left);
        self.cv.text("A living notch for Windows. Every feature included.", R::new(row.x + 130.0, row.y + 92.0, row.w - 150.0, 18.0), 12.5, 500, DIM, Align::Left);
        self.section("Developer");
        let r = self.card(&[ROW, ROW_SUB]);
        self.label(r[0], "Made by", None);
        self.cv.text(DEVELOPER, R::new(r[0].right() - 216.0, r[0].y, 200.0, r[0].h), 13.0, 600, C::WHITE, Align::Right);
        self.button(r[1], "Support", Some(SUPPORT_EMAIL), "Email us", Act::Support, false);
        self.section("Updates");
        let r = self.card(&[ROW_SUB, ROW_SUB, 86.0]);
        let current = env!("CARGO_PKG_VERSION");
        self.button(r[1], "What's new", Some(&format!("Changes in version {current}")), "View changelog", Act::Link(CHANGELOG_PAGE), false);
        let (label, sub, button) = match update {
            Status::Idle => ("Updates".to_string(), format!("Version {current}"), Some(("Check now", Act::CheckUpdate))),
            Status::Checking => ("Checking for updates…".to_string(), format!("Version {current}"), None),
            Status::UpToDate => ("You're up to date".to_string(), format!("Version {current} is the latest"), Some(("Check again", Act::CheckUpdate))),
            Status::Available(rel) => (format!("Version {} is available", rel.version), format!("You have {current}"), Some(("Install and restart", Act::InstallUpdate))),
            Status::Downloading(v) => (format!("Installing {v}…"), "SuiNotch restarts by itself in a moment".to_string(), None),
            Status::Failed(msg) => ("Couldn't update".to_string(), msg.clone(), Some(("Try again", Act::CheckUpdate))),
        };
        match button {
            Some((text, act)) => self.button(r[0], &label, Some(&sub), text, act, false),
            None => self.label(r[0], &label, Some(&sub)),
        }
        self.seg_row(cfg, r[2], "Automatic updates", Some("Automatic installs only while you're away from the PC"), &update::MODE_LABELS, |c| &mut c.update_mode);
        self.section("Your data");
        let r = self.card(&[ROW_SUB]);
        self.button(r[0], "Stored only on this PC", Some("Tasks, water log, shelf and settings. No accounts, no tracking"), "Open folder", Act::OpenData, false);
    }
}
