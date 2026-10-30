//! The notch: a per-pixel-alpha layered window at the top of the screen. Its outline is
//! driven by springs, so every change of state (hover, page, HUD, alert) morphs fluidly.
//! Frames are drawn only while something moves; otherwise one timer waits for the next change.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate, Timelike};
use windows::core::{w, Interface, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1RenderTarget};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::anim::Spring;
use crate::app::{self, App, Deferred};
use crate::config::{Config, NotchStyle};
use crate::events::{self, Event};
use crate::gfx::{self, Canvas, C, R};
use crate::mascot::Mascot;
use crate::pomodoro::Phase;
use crate::sound::{self, Chime};
use crate::store;
use crate::sys::{self, Monitor};
use crate::views;

mod input;
mod kinds;
mod reminders;
mod render;

pub use input::*;
pub use kinds::*;
pub use reminders::*;
pub use render::*;

pub const FLARE: f32 = 9.0;
pub const SHADOW: f32 = 22.0;
pub const PILL_SPACE: f32 = 40.0;
/// Largest notch outline (the Calendar / lyrics pages), used to size the drawing buffer once.
const MAX_NOTCH_W: f32 = 700.0;
const MAX_NOTCH_H: f32 = 280.0;
const MORPH_SPRINGS: usize = 6;

const WM_MOUSELEAVE: u32 = 0x02A3;
const T_FRAME: usize = 1;
const T_HOVER: usize = 2;
const T_LEAVE: usize = 3;
const T_SCHED: usize = 4;
const T_IDLE: usize = 5;
const T_DEMO: usize = 6;

const LEAVE_DELAY_MS: u32 = 380;
const ALERT_TIMEOUT: Duration = Duration::from_secs(25);
const IDLE_MASCOT_S: u32 = 20;
const LOW_BATTERY: u8 = 20;
const STARTUP_GRACE_S: i64 = 300;
pub struct Notch {
    pub hwnd: HWND,
    surface: Surface,
    rt: Option<ID2D1DCRenderTarget>,
    pub monitor: Monitor,
    pub scale: f32,
    pub w: Spring,
    pub h: Spring,
    pub radius: Spring,
    pub open: Spring,
    pub page: Page,
    pub page_pos: Spring,
    /// Space between the screen edge and the notch; only a floating notch has any.
    pub gap: Spring,
    pub pill_w: Spring,
    pub pill_alpha: Spring,
    pub pill_line: Option<usize>,
    pub line_fade: Spring,
    pub expanded: bool,
    pub hovering: bool,
    pub hud: Option<(Hud, Instant)>,
    pub alert: Option<(AlertKind, Instant)>,
    pub hits: Vec<(R, Hit)>,
    pub hover: Option<Hit>,
    pub pressed: Option<(Hit, f32, f32)>,
    pub mouse: (f32, f32),
    pub field: Option<TextField>,
    pub lyrics_view: bool,
    pub volume_open: Spring,
    pub vol_drag: bool,
    pub seek_drag: Option<f32>,
    pub shelf_sel: BTreeSet<usize>,
    pub shelf_selecting: bool,
    pub shelf_anchor: Option<usize>,
    pub shelf_scroll: Spring,
    pub clip_scroll: Spring,
    pub shelf_clear: Spring,
    pub drag_over: bool,
    pub own_drag: bool,
    pub todo_scroll: Spring,
    pub new_due: Option<i64>,
    pub new_color: usize,
    pub due_edit: Option<u64>,
    pub cal_month: (i32, u32),
    pub cal_sel: NaiveDate,
    pub mascot: Mascot,
    pub idle_mode: bool,
    pub bitmaps: HashMap<String, Option<ID2D1Bitmap>>,
    pub art: Option<(u64, ID2D1Bitmap)>,
    pub water_fill: Spring,
    pub eq_phase: f32,
    pub hidden: bool,
    pub notice: Option<(String, Instant)>,
    /// Monitor of the active window, while the notch follows it.
    pub follow: Option<String>,
    last_frame: Instant,
    pub win: (f32, f32),
}

pub fn monitor_scale(cfg: &Config, m: &Monitor) -> f32 {
    let width_dip = (m.rect.right - m.rect.left) as f32 / m.scale;
    m.scale * cfg.size_scale(width_dip)
}

impl Notch {
    pub fn create(cfg: &Config) -> Result<Notch> {
        let monitor = sys::pick_monitor(crate::profile::display_override().or(cfg.display.clone()).as_deref()).unwrap_or(Monitor {
            device: String::new(),
            rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
            top: 0,
            primary: true,
            scale: 1.0,
        });
        let hwnd = unsafe {
            let hinst = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: w!("SuiNotchWindow"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&class);
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("SuiNotchWindow"),
                w!("SuiNotch"),
                WS_POPUP,
                monitor.rect.left,
                monitor.rect.top,
                10,
                10,
                None,
                None,
                Some(hinst.into()),
                None,
            )?
        };
        let (anim_r, anim_z) = cfg.animation();
        let today = Local::now().date_naive();
        Ok(Notch {
            hwnd,
            surface: Surface::empty(),
            rt: None,
            scale: monitor_scale(cfg, &monitor),
            monitor,
            w: Spring::new(180.0, anim_r, anim_z),
            h: Spring::new(32.0, anim_r, anim_z),
            radius: Spring::new(13.0, anim_r, anim_z.max(0.9)),
            open: Spring::new(0.0, anim_r * 0.9, 1.0),
            page: Notch::pages(cfg).first().copied().unwrap_or(Page::Music),
            page_pos: Spring::new(0.0, 0.42, 0.88),
            gap: Spring::new(cfg.gap_px(), anim_r, 1.0),
            pill_w: Spring::new(0.0, 0.42, 0.8),
            pill_alpha: Spring::new(0.0, 0.3, 1.0),
            pill_line: None,
            line_fade: Spring::new(1.0, 0.28, 1.0),
            expanded: false,
            hovering: false,
            hud: None,
            alert: None,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            mouse: (-1.0, -1.0),
            field: None,
            lyrics_view: cfg.lyrics_auto,
            volume_open: Spring::new(0.0, 0.3, 1.0),
            vol_drag: false,
            seek_drag: None,
            shelf_sel: BTreeSet::new(),
            shelf_selecting: false,
            shelf_anchor: None,
            shelf_scroll: Spring::new(0.0, 0.35, 1.0),
            clip_scroll: Spring::new(0.0, 0.35, 1.0),
            shelf_clear: Spring::new(1.0, 0.35, 1.0),
            drag_over: false,
            own_drag: false,
            todo_scroll: Spring::new(0.0, 0.35, 1.0),
            new_due: None,
            new_color: 0,
            due_edit: None,
            cal_month: (today.year(), today.month()),
            cal_sel: today,
            mascot: Mascot::new(),
            idle_mode: false,
            bitmaps: HashMap::new(),
            art: None,
            water_fill: Spring::new(0.0, 0.8, 0.55),
            eq_phase: 0.0,
            hidden: false,
            notice: None,
            follow: None,
            last_frame: Instant::now(),
            win: (0.0, 0.0),
        })
    }

    /// The page on screen: the Shelf while something is dragged over the notch, else the chosen one.
    pub fn shown_page(&self) -> Page {
        if self.drag_over {
            Page::Shelf
        } else {
            self.page
        }
    }

    pub fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h)
    }

    /// The notch body in window DIPs.
    pub fn body(&self) -> R {
        let w = self.w.value.max(1.0);
        R::new(self.win.0 / 2.0 - w / 2.0, 0.0, w, self.h.value.max(1.0))
    }

    pub fn pages(cfg: &Config) -> Vec<Page> {
        Page::ALL.iter().copied().filter(|p| cfg.page_shown(p.key()) && (*p != Page::Clipboard || cfg.clipboard_history)).collect()
    }

    /// The first MORPH_SPRINGS change the notch outline or slide pages; the rest are small fades.
    fn springs(&mut self) -> [&mut Spring; 15] {
        [
            &mut self.w,
            &mut self.h,
            &mut self.radius,
            &mut self.open,
            &mut self.page_pos,
            &mut self.gap,
            &mut self.pill_w,
            &mut self.pill_alpha,
            &mut self.line_fade,
            &mut self.volume_open,
            &mut self.shelf_scroll,
            &mut self.clip_scroll,
            &mut self.shelf_clear,
            &mut self.todo_scroll,
            &mut self.water_fill,
        ]
    }
}

// ------------------------------------------------------------------ geometry

fn has_activity(app: &App) -> bool {
    app.media.as_ref().is_some_and(|m| m.info.playing) || app.focus_running()
}

/// Target outline (w, h, bottom radius) for the current state.
fn target_shape(app: &App) -> (f32, f32, f32) {
    let n = &app.notch;
    let round = app.cfg.roundness();
    let (w, h, r) = if n.alert.is_some() {
        (410.0, 108.0, 28.0)
    } else if n.hud.is_some() {
        if app.cfg.hud_style == 1 {
            (400.0, 46.0, 19.0)
        } else {
            (310.0, 36.0, 15.0)
        }
    } else if n.expanded || n.drag_over {
        let page = n.shown_page();
        let (w, h) = page.size(n.lyrics_view && page == Page::Music);
        (w, h, 30.0)
    } else if app.cfg.minimal && !has_activity(app) {
        (170.0, 5.0, 2.5)
    } else if app.media.is_some() {
        // Wide enough for the artwork, the song title and the equalizer.
        (360.0, 32.0, 13.0)
    } else if app.focus_running() {
        (250.0, 32.0, 13.0)
    } else {
        (196.0, 32.0, 13.0)
    };
    (w, h, (r * round).min(h / 2.0))
}

/// Notch outline. Docked has concave shoulders blending into the top screen edge; Floating
/// rounds the top corners as well; Flat keeps them square.
pub fn outline(cv: &Canvas, body: R, radius: f32, style: NotchStyle) -> Option<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry> {
    if style != NotchStyle::Docked {
        return plain_outline(cv, body, radius, style == NotchStyle::Floating);
    }
    let f = FLARE.min(body.h * 0.8).max(0.5);
    let r = radius.min(body.h - f * 0.5).max(0.5);
    let (x0, x1, y1) = (body.x, body.right(), body.bottom());
    let mut p = cv.path()?;
    p.move_to(x0 - f, 0.0)
        .quad(x0, 0.0, x0, f)
        .line_to(x0, y1 - r)
        .arc_to(x0 + r, y1, r, false)
        .line_to(x1 - r, y1)
        .arc_to(x1, y1 - r, r, false)
        .line_to(x1, f)
        .quad(x1, 0.0, x1 + f, 0.0);
    Some(p.finish(true))
}

/// A rounded body without shoulders; `round_top` rounds the top corners to match the bottom ones.
fn plain_outline(cv: &Canvas, body: R, radius: f32, round_top: bool) -> Option<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry> {
    let r = radius.min(body.h / 2.0).max(0.5);
    let top = if round_top { r } else { 0.0 };
    let (x0, x1, y0, y1) = (body.x, body.right(), body.y, body.bottom());
    let mut p = cv.path()?;
    p.move_to(x0, y0 + top)
        .line_to(x0, y1 - r)
        .arc_to(x0 + r, y1, r, false)
        .line_to(x1 - r, y1)
        .arc_to(x1, y1 - r, r, false)
        .line_to(x1, y0 + top);
    if round_top {
        p.arc_to(x1 - top, y0, top, false).line_to(x0 + top, y0).arc_to(x0, y0 + top, top, false);
    }
    Some(p.finish(true))
}

// ------------------------------------------------------------------ state changes

pub fn expand(app: &mut App) {
    if app.notch.expanded {
        return;
    }
    app.notch.expanded = true;
    if app.notch.hud.is_some() {
        app.notch.hud = None;
    }
    app.notch.w.kick(-120.0);
    request_frame(app);
}

pub fn collapse(app: &mut App) {
    let n = &mut app.notch;
    if !n.expanded {
        return;
    }
    if n.field.is_some() {
        commit_field(app);
    }
    let n = &mut app.notch;
    n.expanded = false;
    n.field = None;
    n.due_edit = None;
    n.shelf_selecting = false;
    n.shelf_sel.clear();
    n.vol_drag = false;
    n.seek_drag = None;
    n.h.kick(60.0);
    request_frame(app);
}

pub fn set_page(app: &mut App, page: Page) {
    if app.notch.page != page {
        if app.notch.field.is_some() {
            commit_field(app);
            app.notch.field = None;
        }
        app.notch.page = page;
        app.notch.due_edit = None;
    }
    request_frame(app);
}

fn step_page(app: &mut App, delta: i32) {
    let pages = visible_pages(app);
    let i = pages.iter().position(|p| *p == app.notch.page).unwrap_or(0) as i32;
    let j = (i + delta).clamp(0, pages.len() as i32 - 1) as usize;
    if j as i32 == i {
        app.notch.page_pos.kick(delta as f32 * -1.5);
    }
    set_page(app, pages[j]);
}

pub fn show_hud(app: &mut App, hud: Hud) {
    if app.notch.expanded || app.notch.alert.is_some() || app.notch.hidden {
        return;
    }
    let same_kind = app.notch.hud.as_ref().is_some_and(|(h, _)| std::mem::discriminant(h) == std::mem::discriminant(&hud));
    if !same_kind {
        app.notch.w.kick(80.0);
    }
    app.notch.hud = Some((hud, Instant::now()));
    request_frame(app);
}

pub fn show_alert(app: &mut App, kind: AlertKind) {
    app.notch.hud = None;
    app.notch.alert = Some((kind, Instant::now()));
    app.notch.h.kick(140.0);
    if app.cfg.sounds && !(app.cfg.respect_quiet && crate::quiet::active()) {
        sound::play(match kind {
            AlertKind::Water => Chime::Drop,
            AlertKind::Pomodoro(Phase::Focus) => Chime::Bell,
            AlertKind::Pomodoro(_) => Chime::Soft,
            _ => Chime::Bell,
        });
    }
    request_frame(app);
}

/// Sui celebrates a finished session or goal, when reactions are on.
pub fn cheer(app: &mut App) {
    if app.cfg.mascot && app.cfg.sui_reactions {
        app.notch.mascot.cheer();
        request_frame(app);
    }
}

pub fn notice(app: &mut App, text: &str) {
    app.notch.notice = Some((text.to_string(), Instant::now()));
    request_frame(app);
}

pub fn config_changed(app: &mut App, old: &Config) {
    let (r, z) = app.cfg.animation();
    for s in [&mut app.notch.w, &mut app.notch.h] {
        s.tune(r, z);
    }
    app.notch.radius.tune(r, z.max(0.9));
    app.notch.gap.tune(r, 1.0);
    app.notch.open.tune(r * 0.9, 1.0);
    if old.display != app.cfg.display || old.size != app.cfg.size || old.display_follow != app.cfg.display_follow {
        relocate(app);
    }
    if old.hidden_pages != app.cfg.hidden_pages || old.clipboard_history != app.cfg.clipboard_history {
        leave_hidden_page(app);
    }
    if old.clipboard_limit != app.cfg.clipboard_limit {
        app.clips.set_limit(app.cfg.clipboard_limit);
    }
    if old.clipboard_keep_min() != app.cfg.clipboard_keep_min() {
        app.clips.set_keep(app.cfg.clipboard_keep_min());
    }
    if old.clipboard_history && !app.cfg.clipboard_history {
        // Turning history off also forgets what was kept.
        app.clips.clear();
    }
    check_fullscreen(app);
    schedule(app);
    request_frame(app);
}

/// Pages in the page bar: the ones switched on, plus the one on screen if it was opened anyway
/// (a file drop shows the Shelf, "+ Add" on the Calendar opens To-do). Never empty.
pub fn visible_pages(app: &App) -> Vec<Page> {
    let shown = Notch::pages(&app.cfg);
    let current = app.notch.shown_page();
    let pages: Vec<Page> = Page::ALL.iter().copied().filter(|p| shown.contains(p) || *p == current).collect();
    if pages.is_empty() {
        vec![Page::Music]
    } else {
        pages
    }
}

/// The page on screen was switched off: move to the nearest page still showing.
fn leave_hidden_page(app: &mut App) {
    let shown = Notch::pages(&app.cfg);
    if shown.is_empty() || shown.contains(&app.notch.page) {
        return;
    }
    let at = Page::ALL.iter().position(|p| *p == app.notch.page).unwrap_or(0);
    let next = Page::ALL[at..].iter().chain(Page::ALL[..at].iter().rev()).find(|p| shown.contains(p));
    app.notch.page = next.copied().unwrap_or(shown[0]);
}

pub fn relocate(app: &mut App) {
    let follow = if app.cfg.display_follow { app.notch.follow.clone() } else { None };
    if let Some(m) = sys::pick_monitor(crate::profile::display_override().or(follow).or(app.cfg.display.clone()).as_deref()) {
        app.notch.scale = monitor_scale(&app.cfg, &m);
        crate::ticker::set_monitor(&m.device);
        app.notch.monitor = m;
    }
    request_frame(app);
}

/// With "Follow the active window", moves the notch to the monitor the user is working on.
fn follow_foreground(app: &mut App) {
    if !app.cfg.display_follow {
        return;
    }
    if let Some(device) = sys::foreground_monitor() {
        if device != app.notch.monitor.device {
            app.notch.follow = Some(device);
            relocate(app);
        }
    }
}

pub fn check_fullscreen(app: &mut App) {
    let hide = app.cfg.hide_fullscreen && sys::foreground_is_fullscreen(&app.notch.monitor.rect);
    if hide != app.notch.hidden {
        app.notch.hidden = hide;
        unsafe {
            let _ = ShowWindow(app.notch.hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
        }
        if !hide {
            request_frame(app);
        }
    }
}

// ------------------------------------------------------------------ window procedure

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = app::with(|app| handle(app, msg, wparam, lparam));
    match handled {
        Some(Some(r)) => r,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn handle(app: &mut App, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let scale = app.notch.scale;
    match msg {
        WM_TIMER => {
            match wparam.0 {
                T_FRAME => frame(app),
                T_HOVER => {
                    unsafe { KillTimer(Some(app.notch.hwnd), T_HOVER).ok() };
                    if app.notch.hovering {
                        expand(app);
                    }
                }
                T_LEAVE => {
                    unsafe { KillTimer(Some(app.notch.hwnd), T_LEAVE).ok() };
                    let busy = app.notch.field.as_ref().is_some_and(|f| !f.text.is_empty()) || app.notch.own_drag || app.notch.drag_over;
                    if !app.notch.hovering && !busy {
                        collapse(app);
                    }
                }
                T_SCHED => on_schedule(app),
                T_DEMO => demo_step(app),
                T_IDLE => on_idle_check(app),
                id => timer_hook(app, id),
            }
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam, scale, app.notch.gap.value);
            on_mouse_move(app, x, y);
            Some(LRESULT(0))
        }
        WM_MOUSELEAVE => {
            if app.notch.pressed.is_none() {
                on_mouse_leave(app);
            }
            Some(LRESULT(0))
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam, scale, app.notch.gap.value);
            on_button_down(app, x, y);
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam, scale, app.notch.gap.value);
            on_button_up(app, x, y);
            Some(LRESULT(0))
        }
        WM_LBUTTONDBLCLK => Some(LRESULT(0)),
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
            on_wheel(app, if msg == WM_MOUSEHWHEEL { -delta } else { delta }, msg == WM_MOUSEHWHEEL || shift);
            Some(LRESULT(0))
        }
        WM_KEYDOWN => {
            on_key(app, VIRTUAL_KEY(wparam.0 as u16));
            Some(LRESULT(0))
        }
        WM_CHAR => {
            on_char(app, wparam.0 as u16);
            Some(LRESULT(0))
        }
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
                on_power(app, lparam);
            } else if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                schedule(app);
            }
            Some(LRESULT(1))
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
            relocate(app);
            accent_changed(app);
            None
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            accent_changed(app);
            None
        }
        events::WM_APP_FRAME => {
            crate::ticker::frame_taken();
            frame(app);
            Some(LRESULT(0))
        }
        events::WM_APP_EVENT => {
            drain_events(app);
            Some(LRESULT(0))
        }
        events::WM_APP_CAPS => {
            let on = sys::caps_lock_on();
            if app.caps != on {
                app.caps = on;
                if app.cfg.hud_caps {
                    show_hud(app, Hud::Caps(on));
                }
            }
            Some(LRESULT(0))
        }
        events::WM_APP_LANG => {
            unsafe { SetTimer(Some(app.notch.hwnd), 77, 120, None) };
            Some(LRESULT(0))
        }
        events::WM_APP_FOREGROUND => {
            follow_foreground(app);
            check_fullscreen(app);
            Some(LRESULT(0))
        }
        events::WM_APP_SHOW_SETTINGS => {
            app::defer(Deferred::OpenSettings(None));
            Some(LRESULT(0))
        }
        events::WM_APP_TRAY => {
            crate::tray::on_message(app, lparam);
            Some(LRESULT(0))
        }
        WM_CLIPBOARDUPDATE => {
            if app.cfg.clipboard_history && crate::clips::capture(&mut app.clips, app.notch.hwnd) {
                if app.cfg.hud_clipboard {
                    if let Some((image, preview)) = app.clips.items.first().map(crate::clips::summary) {
                        show_hud(app, Hud::Clipboard { image, preview });
                    }
                }
                request_frame(app);
            }
            Some(LRESULT(0))
        }
        WM_COMMAND => {
            crate::tray::on_command(app, (wparam.0 & 0xffff) as u16);
            Some(LRESULT(0))
        }
        _ => None,
    }
}

/// Windows' accent colour may have changed; repaint everything that follows it.
fn accent_changed(app: &mut App) {
    if sys::refresh_system_accent() && app.cfg.accent_follow_system {
        crate::tray::refresh_icon(app);
        crate::settings::refresh(app);
        request_frame(app);
    }
}

/// Separate timer id 77 checks the keyboard language shortly after a switching key.
pub fn on_lang_timer(app: &mut App) {
    unsafe { KillTimer(Some(app.notch.hwnd), 77).ok() };
    let lang = sys::keyboard_language();
    if !lang.is_empty() && lang != app.language {
        let first = app.language.is_empty();
        app.language = lang.clone();
        if !first && app.cfg.hud_language {
            show_hud(app, Hud::Language { short: lang, name: sys::language_display_name() });
        }
    }
}

fn on_idle_check(app: &mut App) {
    app.clips.heartbeat(store::now());
    let idle = sys::idle_seconds();
    let was = app.notch.idle_mode;
    app.notch.idle_mode = idle >= IDLE_MASCOT_S || (was && app.notch.mascot.mood != crate::mascot::Mood::Awake);
    if was != app.notch.idle_mode || mascot_visible(app) {
        request_frame(app);
    }
    if app.notch.idle_mode && app.notch.mascot.mood == crate::mascot::Mood::Awake && idle < 2 && was {
        app.notch.idle_mode = false;
    }
}

pub fn start(app: &mut App) {
    events::set_target(app.notch.hwnd);
    crate::ticker::start(&app.notch.monitor.device);
    sys::register_power(app.notch.hwnd);
    crate::clips::start(app.notch.hwnd);
    app.caps = sys::caps_lock_on();
    app.language = sys::keyboard_language();
    app.network = Some(sys::network_state());
    let (p, c, has) = sys::battery();
    if has {
        app.last_battery = Some((p, c));
    }
    unsafe {
        SetTimer(Some(app.notch.hwnd), T_IDLE, 2000, None);
    }
    // No reminder the instant the app starts; the first one waits a few minutes.
    let grace = store::now() + STARTUP_GRACE_S;
    app.water.snoozed_until = Some(app.water.snoozed_until.map_or(grace, |s| s.max(grace)));
    if crate::profile::demo() {
        unsafe { SetTimer(Some(app.notch.hwnd), T_DEMO, crate::profile::DEMO_INTERVAL_MS, None) };
    }
    check_fullscreen(app);
    schedule(app);
    request_frame(app);
}

// Timer 77 is routed through the generic WM_TIMER arm; hook it here.
pub fn timer_hook(app: &mut App, id: usize) {
    if id == 77 {
        on_lang_timer(app);
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Profiling demo: open and close the notch on a fixed rhythm, then quit.
fn demo_step(app: &mut App) {
    thread_local! {
        static TOGGLES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    let n = TOGGLES.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    if n > crate::profile::DEMO_TOGGLES {
        app::defer(Deferred::Exit);
        return;
    }
    if app.notch.expanded {
        if let Some(dir) = std::env::var_os("SUINOTCH_PROFILE_DUMP") {
            let (pw, ph) = ((app.notch.win.0 * app.notch.scale).ceil() as i32, (app.notch.win.1 * app.notch.scale).ceil() as i32);
            let path = std::path::Path::new(&dir).join(format!("notch-{n:02}.png"));
            if let Err(e) = app.notch.surface.save_png(&app.gfx, &path, pw, ph) {
                crate::log(&format!("dump failed: {e}"));
            }
        }
        collapse(app);
    } else {
        app.notch.alert = None;
        expand(app);
    }
}
