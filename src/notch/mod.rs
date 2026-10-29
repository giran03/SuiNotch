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
use crate::config::Config;
use crate::events::{self, Event};
use crate::gfx::{self, Canvas, C, R};
use crate::mascot::Mascot;
use crate::pomodoro::Phase;
use crate::sound::{self, Chime};
use crate::store;
use crate::sys::{self, Monitor};
use crate::views;

mod input;
mod reminders;
mod render;

pub use input::*;
pub use reminders::*;
pub use render::*;

pub const FLARE: f32 = 9.0;
pub const SHADOW: f32 = 22.0;
pub const PILL_SPACE: f32 = 40.0;
/// Largest notch outline (the Calendar / lyrics pages), used to size the drawing buffer once.
const MAX_NOTCH_W: f32 = 700.0;
const MAX_NOTCH_H: f32 = 280.0;
const MORPH_SPRINGS: usize = 5;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Page {
    Music,
    Timer,
    Water,
    Shelf,
    Todo,
    Calendar,
}

impl Page {
    pub const ALL: [Page; 6] = [Page::Music, Page::Timer, Page::Water, Page::Shelf, Page::Todo, Page::Calendar];
    pub fn icon(self) -> char {
        use gfx::icon;
        match self {
            Page::Music => icon::MUSIC,
            Page::Timer => icon::TIMER,
            Page::Water => '\u{E81E}',
            Page::Shelf => icon::FOLDER,
            Page::Todo => icon::CHECK,
            Page::Calendar => icon::CALENDAR,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Page::Music => "Music",
            Page::Timer => "Focus",
            Page::Water => "Water",
            Page::Shelf => "Shelf",
            Page::Todo => "To-do",
            Page::Calendar => "Calendar",
        }
    }
    /// Expanded size in DIPs.
    pub fn size(self, lyrics_view: bool) -> (f32, f32) {
        match self {
            Page::Music if lyrics_view => (680.0, 214.0),
            Page::Music => (580.0, 190.0),
            Page::Timer => (560.0, 188.0),
            Page::Water => (600.0, 204.0),
            Page::Shelf => (620.0, 200.0),
            Page::Todo => (600.0, 250.0),
            Page::Calendar => (640.0, 268.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hud {
    Volume { level: f32, muted: bool },
    Brightness(u8),
    Caps(bool),
    Language { short: String, name: String },
    Battery { percent: u8, charging: bool },
    Wifi { connected: bool, name: String },
    Headphones { name: String, connected: bool },
    Note(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertKind {
    Todo(u64),
    Water,
    Pomodoro(Phase),
    LowBattery(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    Body,
    PageDot(Page),
    Settings,
    Prev,
    PlayPause,
    Next,
    Seek,
    LyricsToggle,
    Volume,
    VolumeBar,
    LyricLine(u64),
    PomoToggle,
    PomoReset,
    PomoSkip,
    PomoPhase(u8),
    Drink,
    WaterUndo,
    ShelfItem(usize),
    ShelfSelect,
    ShelfClear,
    ShelfRemove,
    TodoCheck(u64),
    /// The whole task row, under its controls, so moving between them keeps them visible.
    TodoRow(u64),
    TodoTitle(u64),
    TodoColor(u64),
    TodoDue(u64),
    TodoDelete(u64),
    TodoAdd,
    TodoField,
    NewColor,
    DueChip(u8),
    DueStep(i8),
    DueClear,
    ShowDone,
    CalPrev,
    CalNext,
    CalToday,
    CalDay(NaiveDate),
    CalAdd,
    AlertPrimary,
    AlertSecondary,
    AlertClose,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditTarget {
    NewTask,
    Rename(u64),
}

#[derive(Clone, Debug)]
pub struct TextField {
    pub target: EditTarget,
    pub text: String,
    pub caret: usize,
}

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
            page: Page::Music,
            page_pos: Spring::new(0.0, 0.42, 0.88),
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
            last_frame: Instant::now(),
            win: (0.0, 0.0),
        })
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
        Page::ALL.iter().copied().filter(|p| *p != Page::Calendar || cfg.calendar_page).collect()
    }

    /// The first MORPH_SPRINGS change the notch outline or slide pages; the rest are small fades.
    fn springs(&mut self) -> [&mut Spring; 13] {
        [
            &mut self.w,
            &mut self.h,
            &mut self.radius,
            &mut self.open,
            &mut self.page_pos,
            &mut self.pill_w,
            &mut self.pill_alpha,
            &mut self.line_fade,
            &mut self.volume_open,
            &mut self.shelf_scroll,
            &mut self.shelf_clear,
            &mut self.todo_scroll,
            &mut self.water_fill,
        ]
    }
}

// ------------------------------------------------------------------ geometry

fn has_activity(app: &App) -> bool {
    app.media.as_ref().is_some_and(|m| m.info.playing) || app.pomo.is_running()
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
        let page = if n.drag_over { Page::Shelf } else { n.page };
        let (w, h) = page.size(n.lyrics_view && page == Page::Music);
        (w, h, 30.0)
    } else if app.cfg.minimal && !has_activity(app) {
        (170.0, 5.0, 2.5)
    } else if app.media.is_some() {
        (300.0, 32.0, 13.0)
    } else if app.pomo.is_running() {
        (250.0, 32.0, 13.0)
    } else {
        (196.0, 32.0, 13.0)
    };
    (w, h, (r * round).min(h / 2.0))
}

/// Notch outline with concave shoulders blending into the top screen edge.
pub fn outline(cv: &Canvas, body: R, radius: f32) -> Option<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry> {
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
    let pages = Notch::pages(&app.cfg);
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
    if app.cfg.sounds {
        sound::play(match kind {
            AlertKind::Water => Chime::Drop,
            AlertKind::Pomodoro(Phase::Focus) => Chime::Bell,
            AlertKind::Pomodoro(_) => Chime::Soft,
            _ => Chime::Bell,
        });
    }
    request_frame(app);
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
    app.notch.open.tune(r * 0.9, 1.0);
    if old.display != app.cfg.display || old.size != app.cfg.size {
        relocate(app);
    }
    if !app.cfg.calendar_page && app.notch.page == Page::Calendar {
        app.notch.page = Page::Todo;
    }
    check_fullscreen(app);
    schedule(app);
    request_frame(app);
}

pub fn relocate(app: &mut App) {
    if let Some(m) = sys::pick_monitor(crate::profile::display_override().or(app.cfg.display.clone()).as_deref()) {
        app.notch.scale = monitor_scale(&app.cfg, &m);
        crate::ticker::set_monitor(&m.device);
        app.notch.monitor = m;
    }
    request_frame(app);
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
            let (x, y) = point(lparam, scale);
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
            let (x, y) = point(lparam, scale);
            on_button_down(app, x, y);
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam, scale);
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
        WM_COMMAND => {
            crate::tray::on_command(app, (wparam.0 & 0xffff) as u16);
            Some(LRESULT(0))
        }
        _ => None,
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

pub fn hour_now() -> u32 {
    Local::now().hour()
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
