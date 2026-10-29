//! Mouse, wheel and keyboard input, plus the inline text field.

use super::*;

pub(super) fn point(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
    (x / scale, y / scale)
}

pub(super) fn inside_body(app: &App, x: f32, y: f32) -> bool {
    let b = app.notch.body();
    R::new(b.x - FLARE, 0.0, b.w + 2.0 * FLARE, b.h + 2.0).contains(x, y)
        || (app.notch.pill_alpha.value > 0.5 && y > b.bottom() && y < b.bottom() + PILL_SPACE && (x - b.cx()).abs() < app.notch.pill_w.value / 2.0)
}

pub(super) fn on_mouse_move(app: &mut App, x: f32, y: f32) {
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

pub(super) fn on_mouse_leave(app: &mut App) {
    app.notch.hovering = false;
    app.notch.hover = None;
    app.notch.volume_open.set(0.0);
    unsafe {
        KillTimer(Some(app.notch.hwnd), T_HOVER).ok();
        SetTimer(Some(app.notch.hwnd), T_LEAVE, LEAVE_DELAY_MS, None);
    }
    request_frame(app);
}

pub(super) fn set_volume_from(app: &mut App, x: f32) {
    if let (Some(r), Some(audio)) = (views::volume_bar_rect(app), app.audio.as_ref()) {
        audio.set(((x - r.x) / r.w).clamp(0.0, 1.0));
    }
}

pub(super) fn on_button_down(app: &mut App, x: f32, y: f32) {
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

pub(super) fn on_button_up(app: &mut App, x: f32, y: f32) {
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

pub(super) fn on_wheel(app: &mut App, delta: i16, horizontal: bool) {
    if !app.notch.expanded {
        return;
    }
    let steps = delta as f32 / 120.0;
    let over_list = matches!(app.notch.hover, Some(Hit::ShelfItem(_) | Hit::TodoRow(_) | Hit::TodoTitle(_) | Hit::TodoDelete(_) | Hit::TodoCheck(_) | Hit::TodoDue(_)));
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

pub(super) fn on_key(app: &mut App, vk: VIRTUAL_KEY) {
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
            let (now, d) = (app.now_ms(), app.durations());
            app.pomo.toggle(now, &d);
            schedule(app);
        }
        _ => {}
    }
    request_frame(app);
}

pub(super) fn on_char(app: &mut App, ch: u16) {
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

pub(super) fn clipboard_text() -> Option<String> {
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

