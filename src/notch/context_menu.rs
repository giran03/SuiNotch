//! Quick choices for music controls. Run the modal menu outside the app borrow so
//! Windows can keep dispatching frames and media events while the menu is open.
use super::*;
use crate::lyric_style::LyricsMode;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODT_MENU, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED, ODS_CHECKED};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};

#[derive(Clone)]
struct Row {
    label: String,
    mode: Option<LyricsMode>,
    icon: Option<char>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FrameShape {
    hwnd: isize,
    size: (i32, i32),
    radius: i32,
    inset: i32,
}

thread_local! {
    static PRESENTATION: std::cell::RefCell<Option<(HMENU, Vec<Row>)>> = const { std::cell::RefCell::new(None) };
    static FRAME: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((12, 1)) };
    static FRAME_SHAPED: std::cell::Cell<Option<FrameShape>> = const { std::cell::Cell::new(None) };
}

fn frame_region(width: i32, height: i32, radius: i32, inset: i32) -> HRGN {
    let diameter = (radius * 2).min(width - 2 * inset).min(height - 2 * inset);
    unsafe { CreateRoundRectRgn(inset, inset, width - inset, height - inset, diameter, diameter) }
}

/// Clip the native nonclient frame away, using the same roundness multiplier as the notch.
fn shape_popup(hwnd: HWND, radius: i32, inset: i32) {
    unsafe {
        let mut class = [0u16; 32];
        let length = GetClassNameW(hwnd, &mut class).max(0) as usize;
        if String::from_utf16_lossy(&class[..length]) != "#32768" { return; }
        let mut bounds = RECT::default();
        if GetWindowRect(hwnd, &mut bounds).is_err() { return; }
        let (width, height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
        if width <= 2 * inset || height <= 2 * inset { return; }
        let signature = FrameShape { hwnd: hwnd.0 as isize, size: (width, height), radius, inset };
        if FRAME_SHAPED.with(|f| f.get() == Some(signature)) { return; }
        // DWM borders/corners are independent of the menu's own one-pixel frame.
        let none = DWMWA_COLOR_NONE;
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
        let region = frame_region(width, height, radius, inset);
        if !region.is_invalid() {
            if SetWindowRgn(hwnd, Some(region), true) == 0 {
                let _ = DeleteObject(region.into());
            } else {
                FRAME_SHAPED.with(|f| f.set(Some(signature)));
            }
        }
        // On success Windows owns the region and releases it with the popup window.
    }
}

pub(super) fn idle_popup(hwnd: HWND) {
    if PRESENTATION.with(|p| p.borrow().is_some()) {
        let (radius, inset) = FRAME.with(|f| f.get());
        shape_popup(hwnd, radius, inset);
    }
}

unsafe extern "system" fn popup_started(_: HWINEVENTHOOK, event: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    if event != EVENT_SYSTEM_MENUPOPUPSTART || hwnd.is_invalid() { return; }
    idle_popup(hwnd);
}

struct PopupHook(HWINEVENTHOOK);
impl Drop for PopupHook {
    fn drop(&mut self) {
        if !self.0.is_invalid() { unsafe { let _ = UnhookWinEvent(self.0); } }
    }
}

fn background(app: &App) -> C {
    if app.cfg.glass { C::hex(0x0D0E12) } else { C::BLACK }
}

/// Keep native menu navigation and dismissal, while drawing with the notch's own renderer.
fn theme(menu: HMENU, rows: Vec<Row>, app: &App) -> HBRUSH {
    let radius = if app.cfg.style() == NotchStyle::Flat { 0.0 } else { 13.0 * app.cfg.roundness() };
    FRAME.with(|f| f.set(((radius * app.notch.scale).round() as i32, app.notch.scale.ceil().max(1.0) as i32)));
    FRAME_SHAPED.with(|f| f.set(None));
    let color = background(app);
    let rgb = COLORREF((color.0 * 255.0) as u32 | ((color.1 * 255.0) as u32) << 8 | ((color.2 * 255.0) as u32) << 16);
    unsafe {
        let brush = CreateSolidBrush(rgb);
        let info = MENUINFO { cbSize: std::mem::size_of::<MENUINFO>() as u32, fMask: MIM_BACKGROUND | MIM_STYLE, hbrBack: brush, dwStyle: MNS_NOCHECK, ..Default::default() };
        let _ = SetMenuInfo(menu, &info);
        for index in 0..rows.len() {
            let info = MENUITEMINFOW { cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_FTYPE | MIIM_DATA, fType: MFT_OWNERDRAW, dwItemData: index + 1, ..Default::default() };
            let _ = SetMenuItemInfoW(menu, index as u32, true, &info);
        }
        PRESENTATION.with(|p| *p.borrow_mut() = Some((menu, rows)));
        brush
    }
}

pub(super) fn measure(app: &App, lparam: LPARAM) -> bool {
    if lparam.0 == 0 { return false; }
    let item = unsafe { &mut *(lparam.0 as *mut MEASUREITEMSTRUCT) };
    if item.CtlType != ODT_MENU { return false; }
    PRESENTATION.with(|p| {
        let p = p.borrow();
        let Some((_, rows)) = p.as_ref() else { return false };
        if item.itemData == 0 || item.itemData > rows.len() { return false; }
        let width = rows.iter().map(|row| app.gfx.text_width(&row.label, 12.0, 500) + 80.0).fold(180.0_f32, f32::max).min(440.0);
        item.itemWidth = (width * app.notch.scale).ceil() as u32;
        item.itemHeight = (34.0 * app.notch.scale).ceil() as u32;
        true
    })
}

fn paint_row(cv: &Canvas, app: &App, row: &Row, area: R, selected: bool, checked: bool, disabled: bool) {
    let accent = views::accent(app);
    cv.clear(background(app));
    if selected && !disabled {
        cv.fill_rrect(area.inset(4.0, 2.0), (6.0 * app.cfg.roundness()).min(12.0), accent.a(0.18));
    }
    let ink = if disabled { views::DIM } else { C::WHITE.a(0.92) };
    let icon_color = if checked && !disabled { accent } else { ink };
    if let Some(mode) = row.mode {
        views::lyrics_mode_glyph(cv, 22.0, area.cy(), 13.0, icon_color, mode);
    } else if let Some(icon) = row.icon {
        cv.glyph(icon, 22.0, area.cy(), 13.0, icon_color);
    } else {
        cv.glyph(gfx::icon::MUSIC, 22.0, area.cy(), 13.0, icon_color);
    }
    cv.text(&row.label, R::new(43.0, area.y, area.w - 78.0, area.h), 12.0, 500, ink, gfx::Align::Left);
    if checked {
        cv.glyph(gfx::icon::CHECK, area.right() - 18.0, area.cy(), 11.0, accent);
    }
}

pub(super) fn draw(app: &App, lparam: LPARAM) -> bool {
    if lparam.0 == 0 { return false; }
    let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
    if item.CtlType != ODT_MENU { return false; }
    PRESENTATION.with(|p| {
        let p = p.borrow();
        let Some((menu, rows)) = p.as_ref() else { return false };
        if item.hwndItem.0 != menu.0 { return false; }
        let Some(row) = item.itemData.checked_sub(1).and_then(|i| rows.get(i)) else { return false };
        let rendered = (|| -> Result<()> {
            let rt = gfx::dc_render_target(&app.gfx)?;
            unsafe {
                rt.BindDC(item.hDC, &item.rcItem)?;
                rt.SetDpi(96.0 * app.notch.scale, 96.0 * app.notch.scale);
                rt.BeginDraw();
            }
            let cv = Canvas::new(&app.gfx, rt.cast()?)?;
            let area = R::new(0.0, 0.0, (item.rcItem.right - item.rcItem.left) as f32 / app.notch.scale, (item.rcItem.bottom - item.rcItem.top) as f32 / app.notch.scale);
            paint_row(&cv, app, row, area, item.itemState.0 & ODS_SELECTED.0 != 0, item.itemState.0 & ODS_CHECKED.0 != 0, item.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0) != 0);
            unsafe { rt.EndDraw(None, None) }
        })();
        if let Err(error) = rendered { crate::log(&format!("music menu drawing failed: {error}")); }
        true
    })
}

const MODES: [(LyricsMode, &str); 5] = [
    (LyricsMode::Auto, "Auto"),
    (LyricsMode::Outside, "Outside"),
    (LyricsMode::Inside, "Inside"),
    (LyricsMode::Both, "Both"),
    (LyricsMode::Off, "Off"),
];

pub(super) fn open(app: &mut App, lparam: LPARAM) {
    if app.notch.menu_open { return; }
    let mut screen = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
    if screen.x == -1 && screen.y == -1 {
        unsafe { let _ = GetCursorPos(&mut screen); }
    }
    let mut client = screen;
    unsafe { let _ = ScreenToClient(app.notch.hwnd, &mut client); }
    let hit = app.notch.hit_at(client.x as f32 / app.notch.scale, client.y as f32 / app.notch.scale - app.notch.gap.value);
    if let Some(hit @ (Hit::LyricsToggle | Hit::Source | Hit::Volume | Hit::VolumeBar)) = hit {
        let menu_hit = if hit == Hit::VolumeBar { Hit::Volume } else { hit };
        app.notch.menu_open = true;
        app.notch.pressed = None;
        app::defer(Deferred::MusicMenu(menu_hit, screen));
    }
}

fn append(menu: HMENU, id: usize, label: &str, selected: bool) {
    // Ampersands in player names are text, not Windows menu mnemonics.
    let label = HSTRING::from(label.replace('&', "&&"));
    let flags = MF_STRING | if selected { MF_CHECKED } else { MF_UNCHECKED };
    unsafe { let _ = AppendMenuW(menu, flags, id, PCWSTR(label.as_ptr())); }
}

pub(super) fn lyrics_menu(current: LyricsMode) -> Result<HMENU> {
    let menu = unsafe { CreatePopupMenu()? };
    for (index, (mode, label)) in MODES.iter().enumerate() {
        append(menu, index + 1, label, *mode == current);
    }
    Ok(menu)
}

pub(crate) fn show(hit: Hit, point: POINT) {
    let Some((hwnd, mode)) = app::with(|app| (app.notch.hwnd, app.cfg.lyrics_mode())) else { return };
    let sources = if hit == Hit::Source { crate::media::source_choices() } else { Vec::new() };
    let vol_sources = if hit == Hit::Volume {
        app::with(|app| app.audio.as_ref().map(|a| a.source_choices())).flatten().unwrap_or_default()
    } else {
        Vec::new()
    };
    let menu = if hit == Hit::LyricsToggle {
        lyrics_menu(mode)
    } else if hit == Hit::Source {
        unsafe { CreatePopupMenu() }.inspect(|&menu| {
            for (index, choice) in sources.iter().enumerate() {
                append(menu, index + 1, &choice.label, choice.selected);
            }
            if sources.is_empty() {
                unsafe { let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("No audio sources available")); }
            }
        })
    } else {
        unsafe { CreatePopupMenu() }.inspect(|&menu| {
            for (index, choice) in vol_sources.iter().enumerate() {
                append(menu, index + 1, &choice.label, choice.selected);
            }
            if vol_sources.is_empty() {
                unsafe { let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("No audio outputs or apps available")); }
            }
        })
    };
    if let Ok(menu) = menu {
        let rows = if hit == Hit::LyricsToggle {
            MODES.iter().map(|(mode, label)| Row { label: label.to_string(), mode: Some(*mode), icon: None }).collect()
        } else if hit == Hit::Source {
            if sources.is_empty() {
                vec![Row { label: "No audio sources available".into(), mode: None, icon: Some(gfx::icon::MUSIC) }]
            } else {
                sources.iter().map(|choice| Row { label: choice.label.clone(), mode: None, icon: Some(gfx::icon::MUSIC) }).collect()
            }
        } else {
            if vol_sources.is_empty() {
                vec![Row { label: "No audio outputs or apps available".into(), mode: None, icon: Some(gfx::icon::VOLUME) }]
            } else {
                vol_sources.iter().map(|choice| Row { label: choice.label.clone(), mode: None, icon: Some(choice.icon) }).collect()
            }
        };
        let brush = app::with(|app| theme(menu, rows, app)).unwrap_or_default();
        // Scope the hook to this thread/process and only this popup's modal lifetime.
        let hook = PopupHook(unsafe { SetWinEventHook(EVENT_SYSTEM_MENUPOPUPSTART, EVENT_SYSTEM_MENUPOPUPSTART, None, Some(popup_started), GetCurrentProcessId(), GetCurrentThreadId(), WINEVENT_OUTOFCONTEXT) });
        let chosen = unsafe {
            let _ = SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, point.x, point.y, Some(0), hwnd, None).0 as usize;
            let _ = DestroyMenu(menu);
            PRESENTATION.with(|p| *p.borrow_mut() = None);
            let _ = DeleteObject(brush.into());
            chosen
        };
        drop(hook);
        if chosen > 0 {
            if hit == Hit::LyricsToggle {
                if let Some((mode, _)) = MODES.get(chosen - 1) {
                    app::with(|app| views::select_lyrics_mode(app, *mode));
                }
            } else if hit == Hit::Source {
                if let Some(choice) = sources.into_iter().nth(chosen - 1) {
                    crate::media::select_source(choice);
                }
            } else if hit == Hit::Volume {
                if let Some(choice) = vol_sources.into_iter().nth(chosen - 1) {
                    app::with(|app| {
                        if let Some(a) = app.audio.as_mut() {
                            a.select_target(choice.id);
                        }
                    });
                }
            }
        }
    }
    app::with(|app| {
        app.notch.menu_open = false;
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
            let _ = ScreenToClient(hwnd, &mut cursor);
        }
        let (x, y) = (cursor.x as f32 / app.notch.scale, cursor.y as f32 / app.notch.scale - app.notch.gap.value);
        on_mouse_move(app, x, y);
        if !inside_body(app, x, y) { on_mouse_leave(app); }
        request_frame(app);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_region_removes_all_border_edges_and_follows_roundness() {
        for scale in [1.0_f32, 1.5, 2.0] {
            let (width, height, inset) = ((200.0 * scale) as i32, (140.0 * scale) as i32, scale.ceil() as i32);
            for round in crate::config::ROUNDNESS {
                let region = frame_region(width, height, (13.0 * round * scale).round() as i32, inset);
                unsafe {
                    assert!(!region.is_invalid());
                    for point in [(0, height / 2), (width - 1, height / 2), (width / 2, 0), (width / 2, height - 1), (inset, inset)] {
                        assert!(!PtInRegion(region, point.0, point.1).as_bool(), "border/corner must be clipped");
                    }
                    assert!(PtInRegion(region, width / 2, height / 2).as_bool());
                    let _ = DeleteObject(region.into());
                }
            }
            let flat = frame_region(width, height, 0, inset);
            unsafe {
                assert!(PtInRegion(flat, inset, inset).as_bool(), "Flat keeps square corners");
                let _ = DeleteObject(flat.into());
            }
        }
    }

    #[test]
    #[ignore = "native graphics; requires isolated APPDATA ending in lyrics-toggle-test"]
    fn music_menu_renders_icons_and_tracks_live_accent_at_each_scale() {
        let mut app = tasks::integration::fixture();
        let output = crate::config::data_dir();
        std::fs::create_dir_all(&output).unwrap();
        for scale in [1.0, 1.5, 2.0] {
            app.notch.scale = scale;
            let menu = lyrics_menu(LyricsMode::Inside).unwrap();
            let rows = MODES.iter().map(|(mode, label)| Row { label: label.to_string(), mode: Some(*mode), icon: None }).collect();
            let brush = theme(menu, rows, &app);
            let mut size = MEASUREITEMSTRUCT { CtlType: ODT_MENU, itemData: 1, ..Default::default() };
            assert!(measure(&app, LPARAM(&mut size as *mut _ as isize)));
            assert!(size.itemWidth >= (180.0 * scale) as u32);
            assert_eq!(size.itemHeight, (34.0 * scale) as u32);
            let mut colors = Vec::new();
            for accent in [0, 4] {
                // Repaint the same open menu with a changed config: no theme snapshot is cached.
                app.cfg.accent_follow_system = false;
                app.cfg.accent = accent;
                render::dc_snapshot(&app.gfx, &output.join(format!("music-menu-{accent}-{scale}.png")), size.itemWidth as i32, size.itemHeight as i32 * 4, |dc| {
                    for index in 0..4 {
                        let item = DRAWITEMSTRUCT {
                            CtlType: ODT_MENU, itemID: index + 1, itemData: index as usize + 1,
                            hwndItem: HWND(menu.0), hDC: dc,
                            rcItem: RECT { left: 0, top: index as i32 * size.itemHeight as i32, right: size.itemWidth as i32, bottom: (index as i32 + 1) * size.itemHeight as i32 },
                            itemState: windows::Win32::UI::Controls::ODS_FLAGS(if index == 1 { ODS_SELECTED.0 | ODS_CHECKED.0 } else { 0 }), ..Default::default()
                        };
                        assert!(draw(&app, LPARAM(&item as *const _ as isize)));
                    }
                    colors.push(unsafe { GetPixel(dc, size.itemWidth as i32 - (40.0 * scale) as i32, (51.0 * scale) as i32) });
                }).unwrap();
            }
            assert_ne!(colors[0], colors[1], "highlight follows current accent while the menu remains open");
            unsafe { DestroyMenu(menu).unwrap(); let _ = DeleteObject(brush.into()); }
            PRESENTATION.with(|p| *p.borrow_mut() = None);
        }
        unsafe { DestroyWindow(app.notch.hwnd).unwrap(); }
    }

    #[test]
    fn lyrics_menu_exposes_every_mode_and_checks_only_the_current_choice() {
        for (current, _) in MODES {
            let menu = lyrics_menu(current).unwrap();
            unsafe {
                assert_eq!(GetMenuItemCount(Some(menu)), MODES.len() as i32);
                for (index, (mode, _)) in MODES.iter().enumerate() {
                    assert_eq!(GetMenuItemID(menu, index as i32), index as u32 + 1);
                    let state = GetMenuState(menu, index as u32, MF_BYPOSITION);
                    assert_eq!(state & MF_CHECKED.0 != 0, *mode == current);
                }
                DestroyMenu(menu).unwrap();
            }
        }
    }
}
