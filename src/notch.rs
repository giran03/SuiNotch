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

pub const FLARE: f32 = 9.0;
pub const SHADOW: f32 = 22.0;
pub const PILL_SPACE: f32 = 40.0;

const WM_MOUSELEAVE: u32 = 0x02A3;
const T_FRAME: usize = 1;
const T_HOVER: usize = 2;
const T_LEAVE: usize = 3;
const T_SCHED: usize = 4;
const T_IDLE: usize = 5;

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

struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
}

impl Surface {
    fn ensure(&mut self, w: i32, h: i32) {
        if w <= self.w && h <= self.h && !self.dc.is_invalid() {
            return;
        }
        let (w, h) = (w.max(self.w).max(1) + 64, h.max(self.h).max(1) + 32);
        unsafe {
            if !self.dc.is_invalid() {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bmp.into());
                let _ = DeleteDC(self.dc);
            }
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
            let old = SelectObject(dc, bmp.into());
            *self = Surface { dc, bmp, old, w, h };
        }
    }
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
        let monitor = sys::pick_monitor(cfg.display.as_deref()).unwrap_or(Monitor {
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
            surface: Surface { dc: HDC::default(), bmp: HBITMAP::default(), old: HGDIOBJ::default(), w: 0, h: 0 },
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

// ------------------------------------------------------------------ frame

pub fn request_frame(app: &mut App) {
    app.notch.last_frame = app.notch.last_frame.min(Instant::now());
    unsafe {
        SetTimer(Some(app.notch.hwnd), T_FRAME, 1, None);
    }
}

fn frame(app: &mut App) {
    let now = Instant::now();
    let dt = (now - app.notch.last_frame).as_secs_f32().min(0.05);
    app.notch.last_frame = now;

    expire_transients(app);
    let (tw, th, tr) = target_shape(app);
    let n = &mut app.notch;
    n.w.set(tw);
    n.h.set(th);
    n.radius.set(tr);
    n.open.set(if (n.expanded || n.drag_over) && n.alert.is_none() && n.hud.is_none() { 1.0 } else { 0.0 });
    let idx = Notch::pages(&app.cfg).iter().position(|p| *p == n.page).unwrap_or(0) as f32;
    n.page_pos.set(idx);
    update_pill(app);

    let mut moving = false;
    for s in app.notch.springs() {
        moving |= s.step(dt);
    }
    let idle_s = sys::idle_seconds();
    let mascot_visible = mascot_visible(app);
    if mascot_visible {
        moving |= app.notch.mascot.update(idle_s, dt);
    }
    let eq = eq_running(app);
    if eq {
        app.notch.eq_phase += dt;
    }

    if !app.notch.hidden {
        if let Err(e) = render(app) {
            crate::log(&format!("render failed: {e}"));
            app.notch.rt = None;
        }
    }

    let next = if moving || app.notch.pressed.is_some() {
        16
    } else {
        next_frame_ms(app, eq, mascot_visible)
    };
    unsafe {
        SetTimer(Some(app.notch.hwnd), T_FRAME, next as u32, None);
    }
}

fn eq_running(app: &App) -> bool {
    app.cfg.equalizer
        && !app.notch.expanded
        && app.notch.alert.is_none()
        && app.notch.hud.is_none()
        && app.media.as_ref().is_some_and(|m| m.info.playing)
}

pub fn mascot_visible(app: &App) -> bool {
    app.cfg.mascot
        && !app.notch.expanded
        && app.notch.alert.is_none()
        && app.notch.hud.is_none()
        && app.media.is_none()
        && !app.pomo.is_running()
        && !app.cfg.minimal
        && app.notch.idle_mode
}

/// How long nothing needs drawing, from whatever changes next on screen.
fn next_frame_ms(app: &App, eq: bool, mascot: bool) -> u64 {
    let n = &app.notch;
    let mut next: u64 = 60_000;
    if eq {
        next = next.min(42);
    }
    if mascot {
        next = next.min(if n.mascot.mood == crate::mascot::Mood::Awake { n.mascot.next_wake_ms() } else { 50 });
    }
    if n.field.is_some() {
        next = next.min(500);
    }
    if let Some(m) = &app.media {
        if m.info.playing {
            if n.expanded && n.page == Page::Music {
                next = next.min(if n.lyrics_view { 120 } else { 500 });
            }
            if let Some(i) = app.current_line() {
                if let Some(line) = app.lyrics.get(i + 1) {
                    let pos = m.position_ms() + 250;
                    next = next.min(line.at_ms.saturating_sub(pos).max(16));
                }
            } else if let Some(first) = app.lyrics.first() {
                next = next.min(first.at_ms.saturating_sub(m.position_ms() + 250).max(16));
            }
        }
    }
    if app.pomo.is_running() {
        let ms_left = app.pomo.remaining(app.now_ms());
        next = next.min(if n.expanded { 200 } else { (ms_left % 1000).max(16) });
    }
    if let Some((_, since)) = &n.hud {
        let until = *since + Duration::from_millis(app.cfg.hud_ms() as u64);
        next = next.min(until.saturating_duration_since(Instant::now()).as_millis() as u64 + 1);
    }
    if let Some((_, since)) = &n.notice {
        next = next.min((*since + Duration::from_millis(1500)).saturating_duration_since(Instant::now()).as_millis() as u64 + 1);
    }
    let now = Local::now();
    next.min(((60 - now.second()) as u64) * 1000).max(16)
}

fn expire_transients(app: &mut App) {
    let n = &mut app.notch;
    if let Some((_, since)) = &n.hud {
        if since.elapsed() > Duration::from_millis(app.cfg.hud_ms() as u64) {
            n.hud = None;
        }
    }
    if let Some((_, since)) = &n.alert {
        if since.elapsed() > ALERT_TIMEOUT && !n.hovering {
            n.alert = None;
        }
    }
    if let Some((_, since)) = &n.notice {
        if since.elapsed() > Duration::from_millis(1500) {
            n.notice = None;
        }
    }
}

fn update_pill(app: &mut App) {
    let line = app.current_line();
    let visible_text = line
        .and_then(|i| app.lyrics.get(i))
        .map(|l| l.text.clone())
        .filter(|t| !t.is_empty())
        .filter(|_| app.cfg.lyrics_pill && app.media.as_ref().is_some_and(|m| m.info.playing));
    let n = &mut app.notch;
    let show = visible_text.is_some() && !n.expanded && n.alert.is_none() && !n.drag_over;
    if line != n.pill_line {
        n.pill_line = line;
        n.line_fade.snap(0.0);
        n.line_fade.set(1.0);
    }
    n.pill_alpha.set(if show { 1.0 } else { 0.0 });
    if let Some(t) = visible_text {
        let tw = app.gfx.text_width(&t, 12.5, 600).min(560.0);
        app.notch.pill_w.set(tw + 34.0);
    }
}

fn render(app: &mut App) -> Result<()> {
    let n = &mut app.notch;
    let pill = n.pill_alpha.value > 0.01;
    let max_w = n.w.value.max(n.pill_w.value) + 2.0 * (FLARE + SHADOW);
    let win_w = max_w.ceil();
    let win_h = (n.h.value + SHADOW + if pill { PILL_SPACE } else { 0.0 }).ceil().max(8.0);
    n.win = (win_w, win_h);
    let scale = n.scale;
    let (pw, ph) = ((win_w * scale).ceil() as i32, (win_h * scale).ceil() as i32);
    n.surface.ensure(pw, ph);
    if n.rt.is_none() {
        n.rt = Some(gfx::dc_render_target(&app.gfx)?);
    }
    let rt = n.rt.clone().expect("render target");
    let dc = n.surface.dc;
    unsafe {
        rt.BindDC(dc, &RECT { left: 0, top: 0, right: pw, bottom: ph })?;
        rt.SetDpi(96.0 * scale, 96.0 * scale);
        rt.BeginDraw();
    }
    let gfx = app.gfx.clone();
    let target: ID2D1RenderTarget = rt.cast()?;
    let mut cv = Canvas::new(&gfx, target)?;
    cv.clear(C::CLEAR);
    app.notch.hits.clear();
    views::draw_notch(&mut cv, app);
    let end = unsafe { rt.EndDraw(None, None) };
    if let Err(e) = end {
        app.notch.rt = None;
        return Err(e);
    }
    present(app, pw, ph);
    Ok(())
}

fn present(app: &App, pw: i32, ph: i32) {
    let n = &app.notch;
    let m = &n.monitor.rect;
    let x = (m.left + m.right) / 2 - pw / 2;
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    unsafe {
        let _ = UpdateLayeredWindow(
            n.hwnd,
            None,
            Some(&POINT { x, y: n.monitor.top }),
            Some(&SIZE { cx: pw, cy: ph }),
            Some(n.surface.dc),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        if !IsWindowVisible(n.hwnd).as_bool() {
            let _ = ShowWindow(n.hwnd, SW_SHOWNOACTIVATE);
        }
    }
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
    if let Some(m) = sys::pick_monitor(app.cfg.display.as_deref()) {
        app.notch.scale = monitor_scale(&app.cfg, &m);
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

// ------------------------------------------------------------------ reminders

/// Arms one timer for the earliest of: water reminder, task reminder, focus timer end.
pub fn schedule(app: &mut App) {
    let now = store::now();
    let mut next: Option<i64> = app.todos.next_due(now);
    if app.cfg.water_enabled {
        let c = &app.cfg;
        if let Some(t) = app.water.next_reminder(now, c.water_start_hour, c.water_end_hour, c.water_interval_min, c.goal_ml) {
            next = Some(next.map_or(t, |n| n.min(t)));
        }
    }
    let mut ms: u64 = next.map(|t| ((t - now).max(0) as u64) * 1000).unwrap_or(3_600_000);
    if let Some(end) = app.pomo.ends_at() {
        ms = ms.min(end.saturating_sub(app.now_ms()));
    }
    // Re-check at least hourly so a new day and sleep/resume are picked up.
    let ms = ms.clamp(50, 3_600_000);
    unsafe {
        SetTimer(Some(app.notch.hwnd), T_SCHED, ms as u32, None);
    }
}

fn on_schedule(app: &mut App) {
    let now = store::now();
    let today = store::day_of(now);
    if today != app.last_day {
        app.last_day = today;
        app.water.snoozed_until = None;
        if app.cfg.todo_clear_done_daily {
            app.todos.clear_done_before(now);
            app.save_todos();
        }
    }
    if let Some(phase) = app.pomo.poll(app.now_ms()) {
        let d = app.durations();
        let auto = if phase == Phase::Focus { app.cfg.auto_start_breaks } else { app.cfg.auto_start_focus };
        let now_ms = app.now_ms();
        app.pomo.advance(now_ms, &d, auto);
        show_alert(app, AlertKind::Pomodoro(phase));
    }
    for id in app.todos.due_alerts(now) {
        if let Some(t) = app.todos.get_mut(id) {
            t.alerted = true;
        }
        app.save_todos();
        if app.notch.alert.is_none() {
            show_alert(app, AlertKind::Todo(id));
        }
    }
    if app.cfg.water_enabled && app.notch.alert.is_none() {
        let c = &app.cfg;
        if let Some(t) = app.water.next_reminder(now, c.water_start_hour, c.water_end_hour, c.water_interval_min, c.goal_ml) {
            if t <= now {
                app.water.snoozed_until = Some(now + c.water_interval_min as i64 * 60);
                app.save_water();
                show_alert(app, AlertKind::Water);
            }
        }
    }
    schedule(app);
    request_frame(app);
}

// ------------------------------------------------------------------ events from background threads

pub fn drain_events(app: &mut App) {
    while let Ok(ev) = app.events.try_recv() {
        on_event(app, ev);
    }
    request_frame(app);
}

fn on_event(app: &mut App, ev: Event) {
    match ev {
        Event::Media(info) => {
            let Some(info) = info else {
                app.media = None;
                return;
            };
            let key = info.track_key();
            let changed = app.media.as_ref().map(|m| m.info.track_key()) != Some(key);
            if changed {
                app.lyrics.clear();
                app.lyrics_key = 0;
                crate::lyrics::fetch(info.clone());
            }
            app.media = Some(app::Media { info, at: Instant::now() });
        }
        Event::Artwork { key, bytes } => {
            let decoded = bytes.and_then(|b| app.gfx.decode_image(&b)).and_then(|s| app.gfx.fit(&s, 256));
            app.art_color = decoded.as_ref().and_then(|s| app.gfx.average_color(s));
            app.artwork = decoded.map(|s| (key, s));
            app.notch.art = None;
        }
        Event::Lyrics { key, lines } => {
            if app.media.as_ref().is_some_and(|m| m.info.track_key() == key) {
                app.lyrics = lines;
                app.lyrics_key = key;
            }
        }
        Event::Volume { level, muted } => {
            if app.cfg.hud_volume {
                show_hud(app, Hud::Volume { level, muted });
            }
        }
        Event::AudioDeviceChanged => {
            if let Some(a) = app.audio.as_mut() {
                a.rebind();
            }
        }
        Event::Network { connected, name } => {
            let state = (connected, name.clone());
            let previous = app.network.replace(state.clone());
            if previous.is_some_and(|p| p != state) && app.cfg.hud_wifi {
                show_hud(app, Hud::Wifi { connected, name });
            }
        }
        Event::Bluetooth { name, connected } => {
            if app.cfg.hud_headphones {
                show_hud(app, Hud::Headphones { name, connected });
            }
        }
    }
}

fn on_power(app: &mut App, lparam: LPARAM) {
    use windows::Win32::System::Power::POWERBROADCAST_SETTING;
    let Some(setting) = (unsafe { (lparam.0 as *const POWERBROADCAST_SETTING).as_ref() }) else { return };
    let value = if setting.DataLength >= 4 { unsafe { *(setting.Data.as_ptr() as *const u32) } } else { 0 };
    if setting.PowerSetting == sys::GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS {
        if app.cfg.hud_brightness {
            show_hud(app, Hud::Brightness(value.min(100) as u8));
        }
    } else if setting.PowerSetting == sys::GUID_ACDC_SOURCE || setting.PowerSetting == sys::GUID_BATTERY_PERCENT {
        let (percent, charging, has) = sys::battery();
        if !has {
            return;
        }
        let prev = app.last_battery.replace((percent, charging));
        let Some((prev_pct, prev_charging)) = prev else { return };
        if prev_charging != charging && app.cfg.hud_battery {
            show_hud(app, Hud::Battery { percent, charging });
        } else if !charging && prev_pct > LOW_BATTERY && percent <= LOW_BATTERY && app.cfg.hud_battery {
            show_alert(app, AlertKind::LowBattery(percent));
        }
    }
}

// ------------------------------------------------------------------ input

fn point(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
    (x / scale, y / scale)
}

fn inside_body(app: &App, x: f32, y: f32) -> bool {
    let b = app.notch.body();
    R::new(b.x - FLARE, 0.0, b.w + 2.0 * FLARE, b.h + 2.0).contains(x, y)
        || (app.notch.pill_alpha.value > 0.5 && y > b.bottom() && y < b.bottom() + PILL_SPACE && (x - b.cx()).abs() < app.notch.pill_w.value / 2.0)
}

fn on_mouse_move(app: &mut App, x: f32, y: f32) {
    app.notch.mouse = (x, y);
    let inside = inside_body(app, x, y);
    if inside && !app.notch.hovering {
        app.notch.hovering = true;
        unsafe {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: app.notch.hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            KillTimer(Some(app.notch.hwnd), T_LEAVE).ok();
            if app.cfg.expand_on_hover && !app.notch.expanded && app.notch.alert.is_none() {
                SetTimer(Some(app.notch.hwnd), T_HOVER, app.cfg.hover_delay_ms().max(1), None);
            }
        }
        if mascot_visible(app) {
            app.notch.mascot.poke();
        }
    } else if !inside && app.notch.hovering && app.notch.pressed.is_none() {
        on_mouse_leave(app);
    }
    let hover = app.notch.hit_at(x, y);
    if hover != app.notch.hover {
        app.notch.hover = hover;
        request_frame(app);
    }
    if matches!(hover, Some(Hit::Volume | Hit::VolumeBar)) || app.notch.vol_drag {
        app.notch.volume_open.set(1.0);
    } else {
        app.notch.volume_open.set(0.0);
    }
    if app.notch.vol_drag {
        set_volume_from(app, x);
    }
    if app.notch.seek_drag.is_some() {
        if let Some(r) = views::seek_bar_rect(app) {
            app.notch.seek_drag = Some(((x - r.x) / r.w).clamp(0.0, 1.0));
        }
    }
    if let Some((Hit::ShelfItem(i), px, py)) = app.notch.pressed {
        if (x - px).abs() + (y - py).abs() > 6.0 {
            app.notch.pressed = None;
            let mut indices: Vec<usize> = if app.notch.shelf_sel.contains(&i) { app.notch.shelf_sel.iter().copied().collect() } else { vec![i] };
            indices.retain(|&k| k < app.shelf.items.len());
            unsafe {
                let _ = ReleaseCapture();
            }
            app::defer(Deferred::DragOut(indices));
        }
    }
    request_frame(app);
}

fn on_mouse_leave(app: &mut App) {
    app.notch.hovering = false;
    app.notch.hover = None;
    app.notch.volume_open.set(0.0);
    unsafe {
        KillTimer(Some(app.notch.hwnd), T_HOVER).ok();
        SetTimer(Some(app.notch.hwnd), T_LEAVE, LEAVE_DELAY_MS, None);
    }
    request_frame(app);
}

fn set_volume_from(app: &mut App, x: f32) {
    if let (Some(r), Some(audio)) = (views::volume_bar_rect(app), app.audio.as_ref()) {
        audio.set(((x - r.x) / r.w).clamp(0.0, 1.0));
    }
}

fn on_button_down(app: &mut App, x: f32, y: f32) {
    let hit = app.notch.hit_at(x, y);
    app.notch.pressed = hit.map(|h| (h, x, y));
    unsafe {
        SetCapture(app.notch.hwnd);
    }
    match hit {
        Some(Hit::VolumeBar) => {
            app.notch.vol_drag = true;
            set_volume_from(app, x);
        }
        Some(Hit::Seek) => {
            if let Some(r) = views::seek_bar_rect(app) {
                app.notch.seek_drag = Some(((x - r.x) / r.w).clamp(0.0, 1.0));
            }
        }
        _ => {}
    }
    request_frame(app);
}

fn on_button_up(app: &mut App, x: f32, y: f32) {
    unsafe {
        let _ = ReleaseCapture();
    }
    let pressed = app.notch.pressed.take();
    app.notch.vol_drag = false;
    if let Some(frac) = app.notch.seek_drag.take() {
        if let Some(m) = &app.media {
            let ms = (frac * m.info.duration_ms as f32) as u64;
            crate::media::command(crate::media::Command::Seek(ms));
            if let Some(m) = app.media.as_mut() {
                m.info.position_ms = ms;
                m.at = Instant::now();
            }
        }
        request_frame(app);
        return;
    }
    let hit = app.notch.hit_at(x, y);
    if let (Some((p, _, _)), Some(h)) = (pressed, hit) {
        if p == h {
            let mods = unsafe { (GetKeyState(VK_CONTROL.0 as i32) < 0, GetKeyState(VK_SHIFT.0 as i32) < 0) };
            views::click(app, h, mods);
        }
    }
    if !app.notch.expanded && hit.is_some_and(|h| h == Hit::Body) && app.notch.alert.is_none() {
        expand(app);
    }
    if app.notch.field.is_some() || app.notch.page == Page::Todo {
        unsafe {
            let _ = SetForegroundWindow(app.notch.hwnd);
        }
    }
    request_frame(app);
}

fn on_wheel(app: &mut App, delta: i16, horizontal: bool) {
    if !app.notch.expanded {
        return;
    }
    let steps = delta as f32 / 120.0;
    let over_list = matches!(app.notch.hover, Some(Hit::ShelfItem(_) | Hit::TodoTitle(_) | Hit::TodoCheck(_) | Hit::TodoDue(_)));
    if !horizontal && over_list && app.notch.page == Page::Todo && views::todo_overflow(app) > 0.0 {
        let max = views::todo_overflow(app);
        let t = (app.notch.todo_scroll.target - steps * 36.0).clamp(0.0, max);
        app.notch.todo_scroll.set(t);
    } else if over_list && app.notch.page == Page::Shelf && views::shelf_overflow(app) > 0.0 {
        let max = views::shelf_overflow(app);
        let t = (app.notch.shelf_scroll.target - steps * 90.0).clamp(0.0, max);
        app.notch.shelf_scroll.set(t);
    } else if matches!(app.notch.hover, Some(Hit::Volume | Hit::VolumeBar)) {
        if let Some(a) = app.audio.as_ref() {
            let (v, _) = a.get();
            a.set(v + steps * 0.04);
        }
    } else {
        step_page(app, if steps < 0.0 { 1 } else { -1 });
    }
    request_frame(app);
}

fn on_key(app: &mut App, vk: VIRTUAL_KEY) {
    if app.notch.field.is_some() {
        let f = app.notch.field.as_mut().expect("field");
        let len = f.text.chars().count();
        match vk {
            VK_LEFT => f.caret = f.caret.saturating_sub(1),
            VK_RIGHT => f.caret = (f.caret + 1).min(len),
            VK_HOME => f.caret = 0,
            VK_END => f.caret = len,
            VK_DELETE if f.caret < len => {
                let mut chars: Vec<char> = f.text.chars().collect();
                chars.remove(f.caret);
                f.text = chars.into_iter().collect();
            }
            VK_RETURN => {
                commit_field(app);
                if matches!(app.notch.field.as_ref().map(|f| &f.target), Some(EditTarget::Rename(_))) {
                    app.notch.field = None;
                }
            }
            VK_ESCAPE => app.notch.field = None,
            _ => {}
        }
        request_frame(app);
        return;
    }
    match vk {
        VK_LEFT => step_page(app, -1),
        VK_RIGHT => step_page(app, 1),
        VK_ESCAPE => collapse(app),
        VK_SPACE if app.notch.page == Page::Music => crate::media::command(crate::media::Command::PlayPause),
        VK_SPACE if app.notch.page == Page::Timer => {
            let now = app.now_ms();
            app.pomo.toggle(now);
            schedule(app);
        }
        _ => {}
    }
    request_frame(app);
}

fn on_char(app: &mut App, ch: u16) {
    let Some(f) = app.notch.field.as_mut() else { return };
    let Some(c) = char::from_u32(ch as u32) else { return };
    let mut chars: Vec<char> = f.text.chars().collect();
    match c {
        '\u{8}' => {
            if f.caret > 0 {
                chars.remove(f.caret - 1);
                f.caret -= 1;
            }
        }
        '\r' | '\n' | '\u{1b}' | '\t' => return,
        '\u{16}' => {
            // Ctrl+V
            if let Some(text) = clipboard_text() {
                let insert: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
                for (k, c) in insert.iter().enumerate() {
                    chars.insert(f.caret + k, *c);
                }
                f.caret += insert.len();
            }
        }
        c if c.is_control() => return,
        c => {
            if chars.len() < 200 {
                chars.insert(f.caret, c);
                f.caret += 1;
            }
        }
    }
    f.text = chars.into_iter().collect();
    request_frame(app);
}

fn clipboard_text() -> Option<String> {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        OpenClipboard(None).ok()?;
        let result = (|| {
            let h = GetClipboardData(13).ok()?;
            let ptr = GlobalLock(HGLOBAL(h.0)) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(HGLOBAL(h.0));
            Some(s)
        })();
        let _ = CloseClipboard();
        result
    }
}

pub fn commit_field(app: &mut App) {
    let Some(f) = app.notch.field.clone() else { return };
    match f.target {
        EditTarget::NewTask => {
            let due = app.notch.new_due;
            if app.todos.add(&f.text, due, app.notch.new_color).is_some() {
                app.notch.field = Some(TextField { target: EditTarget::NewTask, text: String::new(), caret: 0 });
                app.notch.new_due = None;
                app.save_todos();
                schedule(app);
                let max = views::todo_overflow(app);
                app.notch.todo_scroll.set(max);
            }
        }
        EditTarget::Rename(id) => {
            app.todos.rename(id, &f.text);
            app.save_todos();
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
