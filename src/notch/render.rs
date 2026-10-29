//! Drawing: the off-screen surface, one frame per vertical blank, and presenting it.

use super::*;

pub(super) struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut std::ffi::c_void,
    w: i32,
    h: i32,
}

impl Surface {
    /// No bitmap yet; the first frame allocates one.
    pub(super) fn empty() -> Surface {
        Surface { dc: HDC::default(), bmp: HBITMAP::default(), old: HGDIOBJ::default(), bits: std::ptr::null_mut(), w: 0, h: 0 }
    }

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
            *self = Surface { dc, bmp, old, bits, w, h };
        }
    }

    /// Writes the top-left `w` x `h` pixels as a PNG (profiling aid; premultiplied colours).
    pub(super) fn save_png(&self, gfx: &gfx::Gfx, path: &std::path::Path, w: i32, h: i32) -> Result<()> {
        use windows::Win32::Graphics::Imaging::*;
        let (w, h) = (w.min(self.w) as u32, h.min(self.h) as u32);
        let stride = self.w as u32 * 4;
        let pixels = unsafe { std::slice::from_raw_parts(self.bits as *const u8, (stride * h) as usize) };
        unsafe {
            let stream = gfx.wic.CreateStream()?;
            stream.InitializeFromFilename(&windows::core::HSTRING::from(path.as_os_str()), 0x4000_0000 /* GENERIC_WRITE */)?;
            let encoder = gfx.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
            let frame = frame.ok_or_else(windows::core::Error::empty)?;
            frame.Initialize(None)?;
            frame.SetSize(w, h)?;
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WritePixels(h, stride, pixels)?;
            frame.Commit()?;
            encoder.Commit()?;
        }
        Ok(())
    }
}

/// Draws on the next vertical blank. Many requests within one refresh become one frame.
pub fn request_frame(app: &mut App) {
    app.notch.last_frame = app.notch.last_frame.min(Instant::now());
    crate::ticker::animate(true);
}

pub(super) fn frame(app: &mut App) {
    crate::profile::count("frame");
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
    let mut morphing = false;
    for (i, s) in app.notch.springs().into_iter().enumerate() {
        let m = s.step(dt);
        moving |= m;
        morphing |= m && i < MORPH_SPRINGS;
    }
    let idle_s = sys::idle_seconds();
    let mascot_visible = mascot_visible(app) || sui_reacting(app);
    if mascot_visible {
        moving |= app.notch.mascot.update(idle_s, dt);
    }
    let eq = eq_running(app);
    if eq {
        app.notch.eq_phase += dt;
    }

    let mut cost = (0.0, 0.0);
    if !app.notch.hidden {
        match render(app) {
            Ok(c) => cost = c,
            Err(e) => {
                crate::log(&format!("render failed: {e}"));
                app.notch.rt = None;
            }
        }
    }
    if moving {
        crate::profile::record(dt * 1000.0, cost.0, cost.1);
    } else {
        crate::profile::end_burst();
    }

    // While moving, the vblank ticker drives frames; otherwise one timer waits for the next change.
    // Shape changes get the full frame rate; small fades and drags are smooth enough at 60 fps.
    let animating = moving || app.notch.pressed.is_some();
    crate::ticker::animate(animating);
    crate::ticker::set_fast(morphing);
    unsafe {
        if animating {
            let _ = KillTimer(Some(app.notch.hwnd), T_FRAME);
        } else {
            SetTimer(Some(app.notch.hwnd), T_FRAME, next_frame_ms(app, eq, mascot_visible) as u32, None);
        }
    }
}

pub(super) fn eq_running(app: &App) -> bool {
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
        && !app.focus_running()
        && !app.cfg.minimal
        && app.notch.idle_mode
}

/// Sui is cheering: shown in the closed notch, or in the alert that called for it.
pub fn sui_reacting(app: &App) -> bool {
    app.cfg.mascot && app.cfg.sui_reactions && app.notch.mascot.mood == crate::mascot::Mood::Cheering && !app.notch.expanded && app.notch.hud.is_none()
}

/// A long break is running and Sui naps beside it in the closed notch.
pub fn sui_napping(app: &App) -> bool {
    app.cfg.mascot && app.cfg.sui_reactions && app.pomo.is_running() && app.pomo.phase == Phase::LongBreak
}

/// How long nothing needs drawing, from whatever changes next on screen.
pub(super) fn next_frame_ms(app: &App, eq: bool, mascot: bool) -> u64 {
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
                    let pos = app.lyric_pos().unwrap_or(0);
                    next = next.min(line.at_ms.saturating_sub(pos).max(16));
                }
            } else if let Some(first) = app.lyrics.first() {
                next = next.min(first.at_ms.saturating_sub(app.lyric_pos().unwrap_or(0)).max(16));
            }
        }
    }
    if app.pomo.is_running() {
        let ms_left = app.pomo.remaining(app.now_ms());
        next = next.min(if n.expanded { 200 } else { (ms_left % 1000).max(16) });
        if !n.expanded && sui_napping(app) {
            next = next.min(80);
        }
    }
    if app.timer.is_running() {
        let ms_left = app.timer.remaining(app.now_ms());
        next = next.min(if n.expanded { 200 } else { (ms_left % 1000).max(16) });
    }
    if app.stopwatch.is_running() {
        let ms = app.stopwatch.elapsed(app.now_ms());
        next = next.min(if n.expanded && n.page == Page::Timer { 100 - ms % 100 } else { 1000 - ms % 1000 }.max(16));
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

pub(super) fn expire_transients(app: &mut App) {
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

pub(super) fn update_pill(app: &mut App) {
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

/// Draws and presents one frame; returns (draw ms, present ms).
pub(super) fn render(app: &mut App) -> Result<(f32, f32)> {
    let t0 = Instant::now();
    let n = &mut app.notch;
    let pill = n.pill_alpha.value > 0.01;
    let max_w = n.w.value.max(n.pill_w.value) + 2.0 * (FLARE + SHADOW);
    let win_w = max_w.ceil();
    let win_h = (n.h.value + SHADOW + if pill { PILL_SPACE } else { 0.0 }).ceil().max(8.0);
    n.win = (win_w, win_h);
    let scale = n.scale;
    let (pw, ph) = ((win_w * scale).ceil() as i32, (win_h * scale).ceil() as i32);
    // Sized once for the largest state so growing never reallocates mid-animation.
    let cap_w = ((MAX_NOTCH_W + 2.0 * (FLARE + SHADOW)) * scale).ceil() as i32;
    let cap_h = ((MAX_NOTCH_H + SHADOW + PILL_SPACE) * scale).ceil() as i32;
    n.surface.ensure(pw.max(cap_w), ph.max(cap_h));
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
    let t1 = Instant::now();
    present(app, pw, ph);
    let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0;
    Ok((ms(t1 - t0), ms(t1.elapsed())))
}

pub(super) fn present(app: &App, pw: i32, ph: i32) {
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

