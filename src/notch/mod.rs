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
use crate::sound;
use crate::store;
use crate::sys::{self, Monitor};
use crate::views;

mod input;
pub(crate) mod context_menu;
mod kinds;
mod reminders;
mod render;
mod tasks;
mod stacking;
pub mod debug;
use stacking::ensure_topmost;
#[cfg(test)]
mod update_test;
#[cfg(test)]
mod lyrics_test;

pub use input::*;
pub use kinds::*;
pub use reminders::*;
pub use render::*;
pub use tasks::*;

pub const FLARE: f32 = 9.0;
pub const SHADOW: f32 = 22.0;
/// Closed notch with a lyric line inside it: the artwork and equalizer take this much beside the text.
const IN_NOTCH_CHROME: f32 = 88.0;
const MAX_IN_NOTCH_W: f32 = 560.0;
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
/// Checks which monitor the mouse is on, while "Follow the mouse" is chosen.
const T_CURSOR: usize = 8;
const T_ZORDER: usize = 9;
const CURSOR_POLL_MS: u32 = 500;

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
    /// Page to return to when the temporary Updates page disappears.
    update_return_page: Option<Page>,
    pub update_preview: Option<debug::UpdatePreview>,
    pub page_pos: Spring,
    /// Space between the screen edge and the notch; only a floating notch has any.
    pub gap: Spring,
    pub pill_w: Spring,
    pub pill_inline_w: f32,
    pub pill_alpha: Spring,
    pub pill_line: Option<usize>,
    pub line_fade: Spring,
    /// Height of the lyric pill under the notch; it grows when the song has a second line to show.
    pub pill_h: Spring,
    /// When the line on show appeared, for scrolling a long one.
    pub line_since: Instant,
    /// The line on show is wider than the most that is drawn, so it scrolls, shrinks or is cut.
    pub pill_overflow: bool,
    pub expanded: bool,
    pub hovering: bool,
    pub hud: Option<(Hud, Instant)>,
    pub alert: Option<(AlertKind, Instant)>,
    pub hits: Vec<(R, Hit)>,
    pub hover: Option<Hit>,
    /// The button a tooltip is waiting for or showing, and when the pointer arrived on it.
    tip_for: Option<Hit>,
    tip_since: Instant,
    /// A tooltip was showing when the pointer moved on, so the next one appears at once.
    tip_warm: bool,
    /// When a waiting tooltip is due, so a frame is drawn exactly then.
    tip_ready_at: Option<Instant>,
    pub tip_text: String,
    /// Shown under its button rather than above it, for buttons near the top edge.
    pub tip_below: bool,
    pub tip_alpha: Spring,
    /// Where the tooltip points: the button's centre, and the edge of the tooltip nearest to it.
    pub tip_x: Spring,
    pub tip_y: Spring,
    pub pressed: Option<(Hit, f32, f32)>,
    pub mouse: (f32, f32),
    pub field: Option<TextField>,
    /// The task editor hidden temporarily while entering a reminder time.
    pub time_return: Option<TextField>,
    pub time_select_all: bool,
    pub task_day: Option<NaiveDate>,
    pub task_drafts: Vec<TaskDraft>,
    pub task_error: Option<String>,
    pub lyrics_view: bool,
    /// The side panel shows recently played songs instead of lyrics.
    pub recent_view: bool,
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
    pub menu_open: bool,
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
                style: CS_DBLCLKS,
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
            update_return_page: None,
            update_preview: None,
            page_pos: Spring::new(0.0, 0.42, 0.88),
            gap: Spring::new(cfg.gap_px(), anim_r, 1.0),
            pill_w: Spring::new(0.0, 0.42, 0.8),
            pill_inline_w: 0.0,
            pill_alpha: Spring::new(0.0, 0.3, 1.0),
            pill_line: None,
            line_fade: Spring::new(1.0, 0.28, 1.0),
            pill_h: Spring::new(26.0, 0.3, 0.9),
            line_since: Instant::now(),
            pill_overflow: false,
            expanded: false,
            hovering: false,
            hud: None,
            alert: None,
            hits: Vec::new(),
            hover: None,
            tip_for: None,
            tip_since: Instant::now(),
            tip_warm: false,
            tip_ready_at: None,
            tip_text: String::new(),
            tip_below: false,
            tip_alpha: Spring::new(0.0, 0.25, 1.0),
            tip_x: Spring::new(0.0, 0.3, 0.9),
            tip_y: Spring::new(0.0, 0.3, 0.9),
            pressed: None,
            mouse: (-1.0, -1.0),
            field: None,
            time_return: None,
            time_select_all: false,
            task_day: None,
            task_drafts: Vec::new(),
            task_error: None,
            lyrics_view: cfg.lyrics_auto && cfg.lyrics_pill,
            recent_view: false,
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
            menu_open: false,
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
    fn springs(&mut self) -> [&mut Spring; 19] {
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
            &mut self.tip_alpha,
            &mut self.tip_x,
            &mut self.tip_y,
            &mut self.pill_h,
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
        let (w, h) = page.size((n.lyrics_view || n.recent_view) && page == Page::Music);
        (w, h, 30.0)
    } else if app.cfg.minimal && !has_activity(app) {
        (170.0, 5.0, 2.5)
    } else if app.media.is_some() {
        // Wide enough for the artwork, the song title and the equalizer; a lyric line shown inside
        // the notch widens it further.
        let in_notch = app.cfg.lyric_look().in_notch() && n.pill_alpha.target > 0.5;
        let lyric_w = if in_notch { n.pill_inline_w + IN_NOTCH_CHROME } else { 0.0 };
        (lyric_w.clamp(360.0, MAX_IN_NOTCH_W), 32.0, 13.0)
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
    stacking::raise_for_interaction(app);
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
    debug::close_updates(app);
    let n = &mut app.notch;
    if !n.expanded {
        return;
    }
    cancel_time_edit(app);
    if app.notch.field.as_ref().is_some_and(|f| matches!(f.target, EditTarget::Rename(_))) {
        commit_field(app);
        app.notch.field = None;
    }
    let n = &mut app.notch;
    n.expanded = false;
    n.due_edit = None;
    n.shelf_selecting = false;
    n.shelf_sel.clear();
    n.vol_drag = false;
    n.seek_drag = None;
    n.h.kick(60.0);
    request_frame(app);
}

pub fn set_page(app: &mut App, page: Page) {
    if page == Page::Updates && !update_tab_visible(app) { return; }
    if page != Page::Updates && app.notch.update_preview.take().is_some() {
        app.notch.hits.clear();
        app.notch.hover = None;
        app.notch.pressed = None;
    }
    if app.notch.page != page {
        app.notch.update_return_page = if page == Page::Updates { Some(app.notch.page) } else { None };
        cancel_time_edit(app);
        if app.notch.field.as_ref().is_some_and(|f| matches!(f.target, EditTarget::Rename(_))) {
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
    // A timer remains actionable until acknowledged; a low-battery callback must not replace it.
    if matches!(app.notch.alert, Some((AlertKind::Timer, _))) && kind != AlertKind::Timer { return; }
    app.notch.hud = None;
    app.notch.alert = Some((kind, Instant::now()));
    app.notch.h.kick(140.0);
    sound::alert(&app.cfg, match kind {
        AlertKind::Pomodoro(Phase::Focus) => 0,
        AlertKind::Pomodoro(_) => 1,
        AlertKind::Timer => 2,
        AlertKind::Todo(_) => 3,
        AlertKind::Event(_) => 4,
        AlertKind::Water => 5,
        AlertKind::LowBattery(_) => 6,
    });
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
    if !app.cfg.lyrics_pill { app.notch.lyrics_view = false; }
    // A lookup ahead planned under the old choices must not run once any of them is turned off.
    let c = &app.cfg;
    let turned_off = |was: bool, now: bool| was && !now;
    if turned_off(old.lyrics_pill, c.lyrics_pill) || turned_off(old.lyrics_prefetch, c.lyrics_prefetch) || turned_off(old.lyrics_cache, c.lyrics_cache)
        || turned_off(old.lyrics_fallback, c.lyrics_fallback) || turned_off(old.lyrics_learn, c.lyrics_learn) || old.lyrics_sources != c.lyrics_sources
    {
        crate::lyrics_prefetch::cancel();
    }
    crate::lyric_filter::reconcile(app);
    if old.sounds != app.cfg.sounds || old.sound_library != app.cfg.sound_library || old.respect_quiet != app.cfg.respect_quiet { sound::stop(); }
    let (r, z) = app.cfg.animation();
    for s in [&mut app.notch.w, &mut app.notch.h] {
        s.tune(r, z);
    }
    app.notch.radius.tune(r, z.max(0.9));
    app.notch.gap.tune(r, 1.0);
    app.notch.open.tune(r * 0.9, 1.0);
    if old.display != app.cfg.display
        || old.size != app.cfg.size
        || old.display_follow != app.cfg.display_follow
        || old.display_follow_mouse != app.cfg.display_follow_mouse
    {
        relocate(app);
    }
    if old.display_follow_mouse != app.cfg.display_follow_mouse {
        sync_cursor_timer(app);
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
    // Romaji, a translation or word timing was just asked for: songs from LRCLIB lack them, so look the
    // one on screen up again, keeping its lines on show meanwhile.
    let extras_now = !old.wants_lyric_extras() && app.cfg.wants_lyric_extras() && app.lyrics_translation.is_empty() && app.lyrics_romaji.is_empty();
    let words_now = !old.wants_word_timing() && app.cfg.wants_word_timing() && app.lyrics_words.is_empty();
    if (extras_now || words_now) && !app.lyrics.is_empty() {
        crate::lyric_filter::request(app, app.lyric_request.manual, true);
    }
    check_fullscreen(app);
    schedule(app);
    request_frame(app);
}

/// Pages in the page bar: the ones switched on, plus the one on screen if it was opened anyway
/// (a file drop shows the Shelf, "+ Add" on the Calendar opens To-do). Never empty.
pub fn update_tab_visible(app: &App) -> bool {
    app.notch.update_preview.is_some() || app.update.tab_visible()
}

pub fn visible_pages(app: &App) -> Vec<Page> {
    let shown = Notch::pages(&app.cfg);
    let current = app.notch.shown_page();
    let mut pages: Vec<Page> = Page::ALL.iter().copied().filter(|p| shown.contains(p) || *p == current).collect();
    if update_tab_visible(app) { pages.push(Page::Updates); }
    if pages.is_empty() {
        vec![Page::Music]
    } else {
        pages
    }
}

/// The page on screen was switched off: move to the nearest page still showing.
fn leave_hidden_page(app: &mut App) {
    if app.notch.page == Page::Updates && update_tab_visible(app) { return; }
    let shown = Notch::pages(&app.cfg);
    if shown.is_empty() || shown.contains(&app.notch.page) {
        return;
    }
    let at = Page::ALL.iter().position(|p| *p == app.notch.page).unwrap_or(0);
    let next = Page::ALL[at..].iter().chain(Page::ALL[..at].iter().rev()).find(|p| shown.contains(p));
    app.notch.page = next.copied().unwrap_or(shown[0]);
}

/// Apply updater events without stealing focus; retire the temporary tab when work is done.
pub fn update_status(app: &mut App, status: crate::update::Status) {
    if status.tab_visible() { app.notch.update_preview = None; }
    app.update.receive(status);
    if app.notch.page == Page::Updates && !update_tab_visible(app) {
        let previous = app.notch.update_return_page.take();
        let shown = Notch::pages(&app.cfg);
        let destination = previous.filter(|p| *p != Page::Updates && shown.contains(p)).or_else(|| shown.first().copied()).unwrap_or(Page::Music);
        set_page(app, destination);
        let index = visible_pages(app).iter().position(|p| *p == destination).unwrap_or(0);
        app.notch.page_pos.snap(index as f32);
    }
    // Tab hit regions and page size must refresh even while the notch is already open.
    app.notch.hover = None;
    app.notch.pressed = None;
    request_frame(app);
}

/// With "Follow the mouse", moves the notch to the monitor the pointer has rested on. It waits for two
/// checks in a row so crossing a screen on the way elsewhere doesn't make the notch jump, and it
/// stays put while open or being dragged over.
fn follow_mouse(app: &mut App) {
    thread_local! {
        static CANDIDATE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    }
    let n = &app.notch;
    if !app.cfg.display_follow_mouse || n.expanded || n.drag_over || n.own_drag {
        CANDIDATE.with(|c| *c.borrow_mut() = None);
        return;
    }
    let under = sys::cursor_monitor();
    let settled = CANDIDATE.with(|c| {
        let mut c = c.borrow_mut();
        let same = *c == under;
        *c = under.clone();
        same
    });
    if let (true, Some(device)) = (settled, under) {
        if device != app.notch.monitor.device {
            app.notch.follow = Some(device);
            relocate(app);
        }
    }
}

/// Runs the mouse-following check only while that option is on.
fn sync_cursor_timer(app: &App) {
    unsafe {
        if app.cfg.display_follow_mouse {
            SetTimer(Some(app.notch.hwnd), T_CURSOR, CURSOR_POLL_MS, None);
        } else {
            let _ = KillTimer(Some(app.notch.hwnd), T_CURSOR);
        }
    }
}

pub fn relocate(app: &mut App) {
    let follow = if app.cfg.display_follow || app.cfg.display_follow_mouse { app.notch.follow.clone() } else { None };
    if let Some(m) = sys::pick_monitor(crate::profile::display_override().or(follow).or(app.cfg.display.clone()).as_deref()) {
        app.notch.scale = monitor_scale(&app.cfg, &m);
        crate::ticker::set_monitor(&m.device);
        app.notch.monitor = m;
    }
    // The new screen may or may not have a full-screen app on it.
    check_fullscreen(app);
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
            ensure_topmost(app);
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
                    let editing = app.notch.page == Page::Todo && app.notch.field.as_ref().is_some_and(|f| !f.text.is_empty());
                    let busy = editing || app.notch.own_drag || app.notch.drag_over || app.notch.menu_open;
                    if !app.notch.hovering && !busy {
                        collapse(app);
                    }
                }
                T_SCHED => on_schedule(app),
                T_DEMO => demo_step(app),
                T_IDLE => on_idle_check(app),
                T_CURSOR => follow_mouse(app),
                T_ZORDER => {
                    unsafe { let _ = KillTimer(Some(app.notch.hwnd), T_ZORDER); }
                    ensure_topmost(app);
                },
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
        WM_LBUTTONDBLCLK => {
            let (x, y) = point(lparam, scale, app.notch.gap.value);
            on_double_click(app, x, y);
            Some(LRESULT(0))
        }
        WM_CONTEXTMENU => {
            context_menu::open(app, lparam);
            Some(LRESULT(0))
        }
        WM_MEASUREITEM if context_menu::measure(app, lparam) => Some(LRESULT(1)),
        WM_DRAWITEM if context_menu::draw(app, lparam) => Some(LRESULT(1)),
        WM_ENTERIDLE if wparam.0 == MSGF_MENU as usize => {
            context_menu::idle_popup(HWND(lparam.0 as *mut _));
            Some(LRESULT(0))
        }
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
                check_fullscreen(app);
                ensure_topmost(app);
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
            ensure_topmost(app);
            // Some apps restack their windows just after activation; catch that delayed change too.
            unsafe { SetTimer(Some(app.notch.hwnd), T_ZORDER, 250, None); }
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
    // A game can go full screen without any focus change, so look again now and then.
    if app.cfg.hide_fullscreen {
        check_fullscreen(app);
    }
    ensure_topmost(app);
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
    sync_cursor_timer(app);
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
        sound::stop();
        expand(app);
    }
}
