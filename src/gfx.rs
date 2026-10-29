//! Direct2D + DirectWrite drawing helpers shared by the notch and the settings window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use windows::core::{w, Interface, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows_numerics::{Matrix3x2, Vector2};

pub const ICON_FONT_FLUENT: PCWSTR = w!("Segoe Fluent Icons");
pub const ICON_FONT_MDL2: PCWSTR = w!("Segoe MDL2 Assets");

// Icon glyphs shared by Segoe Fluent Icons and Segoe MDL2 Assets.
pub mod icon {
    pub const PLAY: char = '\u{E768}';
    pub const PAUSE: char = '\u{E769}';
    pub const PREV: char = '\u{E892}';
    pub const NEXT: char = '\u{E893}';
    pub const VOLUME: char = '\u{E767}';
    pub const MUTE: char = '\u{E74F}';
    pub const BRIGHTNESS: char = '\u{E706}';
    pub const SETTINGS: char = '\u{E713}';
    pub const ADD: char = '\u{E710}';
    pub const DELETE: char = '\u{E74D}';
    pub const CHECK: char = '\u{E73E}';
    pub const CALENDAR: char = '\u{E787}';
    pub const TIMER: char = '\u{E916}';
    pub const MUSIC: char = '\u{EC4F}';
    pub const LYRICS: char = '\u{E8D2}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const DOCUMENT: char = '\u{E8A5}';
    pub const WIFI: char = '\u{E701}';
    pub const WIFI_OFF: char = '\u{EB5E}';
    pub const HEADPHONE: char = '\u{E7F6}';
    pub const BLUETOOTH: char = '\u{E702}';
    pub const KEYBOARD: char = '\u{E765}';
    pub const COPY: char = '\u{E8C8}';
    pub const CLOSE: char = '\u{E711}';
    pub const RESET: char = '\u{E72C}';
    pub const SKIP: char = '\u{E893}';
    pub const UP: char = '\u{E70E}';
    pub const LEFT: char = '\u{E76B}';
    pub const RIGHT: char = '\u{E76C}';
    pub const INFO: char = '\u{E946}';
    pub const COLOR: char = '\u{E790}';
    pub const DISPLAY: char = '\u{E7F4}';
    pub const BELL: char = '\u{EA8F}';
    pub const TODO: char = '\u{E9D5}';
    pub const SHELF: char = '\u{E7B8}';
    pub const SELECT: char = '\u{E8B3}';
    pub const TEXT: char = '\u{E8D2}';
    pub const UNDO: char = '\u{E7A7}';
    pub const SNOOZE: char = '\u{E823}';
    pub const POWER: char = '\u{E7E8}';
    pub const SPARKLE: char = '\u{E734}';
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        R { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn inset(&self, dx: f32, dy: f32) -> R {
        R::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }
    pub fn offset(&self, dx: f32, dy: f32) -> R {
        R::new(self.x + dx, self.y + dy, self.w, self.h)
    }
    pub fn centered(cx: f32, cy: f32, w: f32, h: f32) -> R {
        R::new(cx - w / 2.0, cy - h / 2.0, w, h)
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct C(pub f32, pub f32, pub f32, pub f32);

impl C {
    pub fn hex(rgb: u32) -> C {
        C(((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0, 1.0)
    }
    pub fn a(self, alpha: f32) -> C {
        C(self.0, self.1, self.2, self.3 * alpha.clamp(0.0, 1.0))
    }
    pub fn mix(self, other: C, t: f32) -> C {
        let t = t.clamp(0.0, 1.0);
        C(
            self.0 + (other.0 - self.0) * t,
            self.1 + (other.1 - self.1) * t,
            self.2 + (other.2 - self.2) * t,
            self.3 + (other.3 - self.3) * t,
        )
    }
    pub const WHITE: C = C(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: C = C(0.0, 0.0, 0.0, 1.0);
    pub const CLEAR: C = C(0.0, 0.0, 0.0, 0.0);
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    Text,
    Display,
    Icon,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FormatKey {
    font: Font,
    size_x10: u32,
    weight: i32,
    align: Align,
    wrap: bool,
}

pub struct Gfx {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    pub wic: IWICImagingFactory,
    icon_font: PCWSTR,
    text_font: PCWSTR,
    display_font: PCWSTR,
    formats: RefCell<HashMap<FormatKey, IDWriteTextFormat>>,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let has = |name: PCWSTR| -> bool {
                let mut coll = None;
                if dw.GetSystemFontCollection(&mut coll, false).is_err() {
                    return false;
                }
                let Some(coll) = coll else { return false };
                let mut index = 0;
                let mut exists = BOOL(0);
                coll.FindFamilyName(name, &mut index, &mut exists).is_ok() && exists.as_bool()
            };
            let icon_font = if has(ICON_FONT_FLUENT) { ICON_FONT_FLUENT } else { ICON_FONT_MDL2 };
            let text_font = if has(w!("Segoe UI Variable Text")) { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
            let display_font =
                if has(w!("Segoe UI Variable Display")) { w!("Segoe UI Variable Display") } else { w!("Segoe UI") };
            Ok(Gfx { d2d, dw, wic, icon_font, text_font, display_font, formats: RefCell::new(HashMap::new()) })
        }
    }

    fn format(&self, font: Font, size: f32, weight: i32, align: Align, wrap: bool) -> Option<IDWriteTextFormat> {
        let key = FormatKey { font, size_x10: (size * 10.0) as u32, weight, align, wrap };
        if let Some(f) = self.formats.borrow().get(&key) {
            return Some(f.clone());
        }
        let family = match font {
            Font::Text => self.text_font,
            Font::Display => self.display_font,
            Font::Icon => self.icon_font,
        };
        unsafe {
            let f = self
                .dw
                .CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT(weight),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )
                .ok()?;
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = f.SetParagraphAlignment(if wrap { DWRITE_PARAGRAPH_ALIGNMENT_NEAR } else { DWRITE_PARAGRAPH_ALIGNMENT_CENTER });
            let _ = f.SetWordWrapping(if wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
            if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = f.SetTrimming(&trim, &sign);
            }
            self.formats.borrow_mut().insert(key, f.clone());
            Some(f)
        }
    }

    pub fn text_width(&self, s: &str, size: f32, weight: i32) -> f32 {
        self.measure(s, Font::Text, size, weight).0
    }

    pub fn measure(&self, s: &str, font: Font, size: f32, weight: i32) -> (f32, f32) {
        let Some(f) = self.format(font, size, weight, Align::Left, false) else { return (0.0, 0.0) };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&wide, &f, 10_000.0, 1000.0) else { return (0.0, 0.0) };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_err() {
                return (0.0, 0.0);
            }
            (m.widthIncludingTrailingWhitespace, m.height)
        }
    }

    /// Decodes an encoded image (PNG/JPEG/...) into a premultiplied BGRA WIC bitmap.
    pub fn decode_image(&self, bytes: &[u8]) -> Option<IWICBitmapSource> {
        unsafe {
            let stream = self.wic.CreateStream().ok()?;
            stream.InitializeFromMemory(bytes).ok()?;
            let decoder = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad).ok()?;
            let frame = decoder.GetFrame(0).ok()?;
            self.to_pbgra(&frame.cast().ok()?)
        }
    }

    pub fn to_pbgra(&self, source: &IWICBitmapSource) -> Option<IWICBitmapSource> {
        unsafe {
            let conv = self.wic.CreateFormatConverter().ok()?;
            conv.Initialize(source, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)
                .ok()?;
            // Materialize so the bitmap no longer depends on the decoder's stream.
            let bmp = self.wic.CreateBitmapFromSource(&conv, WICBitmapCacheOnLoad).ok()?;
            bmp.cast().ok()
        }
    }

    /// Scales a WIC bitmap so its longest side is at most `max_px`.
    pub fn fit(&self, source: &IWICBitmapSource, max_px: u32) -> Option<IWICBitmapSource> {
        unsafe {
            let (mut w, mut h) = (0, 0);
            source.GetSize(&mut w, &mut h).ok()?;
            if w <= max_px && h <= max_px {
                return Some(source.clone());
            }
            let s = max_px as f32 / w.max(h) as f32;
            let scaler = self.wic.CreateBitmapScaler().ok()?;
            scaler
                .Initialize(source, ((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1), WICBitmapInterpolationModeFant)
                .ok()?;
            let bmp = self.wic.CreateBitmapFromSource(&scaler, WICBitmapCacheOnLoad).ok()?;
            bmp.cast().ok()
        }
    }

    /// Average colour of a bitmap, nudged brighter so it reads as a glow on black.
    pub fn average_color(&self, source: &IWICBitmapSource) -> Option<C> {
        let small = self.fit(source, 16)?;
        unsafe {
            let (mut w, mut h) = (0, 0);
            small.GetSize(&mut w, &mut h).ok()?;
            let stride = w * 4;
            let mut buf = vec![0u8; (stride * h) as usize];
            small.CopyPixels(std::ptr::null(), stride, &mut buf).ok()?;
            let (mut r, mut g, mut b, mut n) = (0f32, 0f32, 0f32, 0f32);
            for px in buf.chunks_exact(4) {
                let a = px[3] as f32 / 255.0;
                if a < 0.1 {
                    continue;
                }
                b += px[0] as f32 / a;
                g += px[1] as f32 / a;
                r += px[2] as f32 / a;
                n += 1.0;
            }
            if n == 0.0 {
                return None;
            }
            let c = C(r / n / 255.0, g / n / 255.0, b / n / 255.0, 1.0);
            let max = c.0.max(c.1).max(c.2).max(0.05);
            let boost = (0.85 / max).min(2.5);
            Some(C((c.0 * boost).min(1.0), (c.1 * boost).min(1.0), (c.2 * boost).min(1.0), 1.0))
        }
    }
}

/// A frame being drawn. Coordinates are in DIPs; the render target applies the DPI.
pub struct Canvas<'a> {
    pub gfx: &'a Gfx,
    pub rt: ID2D1RenderTarget,
    brush: ID2D1SolidColorBrush,
    opacity_stack: Vec<f32>,
}

impl<'a> Canvas<'a> {
    pub fn new(gfx: &'a Gfx, rt: ID2D1RenderTarget) -> Result<Self> {
        let brush = unsafe { rt.CreateSolidColorBrush(&C::WHITE.d2d(), None)? };
        Ok(Canvas { gfx, rt, brush, opacity_stack: vec![1.0] })
    }

    fn paint(&self, c: C) -> &ID2D1SolidColorBrush {
        let alpha = self.opacity_stack.last().copied().unwrap_or(1.0);
        unsafe { self.brush.SetColor(&c.a(alpha).d2d()) };
        &self.brush
    }

    pub fn clear(&self, c: C) {
        unsafe { self.rt.Clear(Some(&c.d2d())) }
    }

    pub fn fill_rect(&self, r: R, c: C) {
        unsafe { self.rt.FillRectangle(&r.d2d(), self.paint(c)) }
    }

    pub fn fill_rrect(&self, r: R, radius: f32, c: C) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.paint(c)) }
    }

    pub fn stroke_rrect(&self, r: R, radius: f32, c: C, width: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.paint(c), width, None) }
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, radius: f32, c: C) {
        self.fill_ellipse(cx, cy, radius, radius, c);
    }

    pub fn fill_ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32, c: C) {
        let e = D2D1_ELLIPSE { point: Vector2::new(cx, cy), radiusX: rx, radiusY: ry };
        unsafe { self.rt.FillEllipse(&e, self.paint(c)) }
    }

    pub fn stroke_circle(&self, cx: f32, cy: f32, radius: f32, c: C, width: f32) {
        let e = D2D1_ELLIPSE { point: Vector2::new(cx, cy), radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawEllipse(&e, self.paint(c), width, None) }
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: C, width: f32) {
        unsafe { self.rt.DrawLine(Vector2::new(x0, y0), Vector2::new(x1, y1), self.paint(c), width, &self.round_cap()) }
    }

    fn round_cap(&self) -> ID2D1StrokeStyle {
        thread_local! {
            static CAP: RefCell<Option<ID2D1StrokeStyle>> = const { RefCell::new(None) };
        }
        CAP.with(|c| {
            if let Some(s) = c.borrow().as_ref() {
                return s.clone();
            }
            let props = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            let s = unsafe { self.gfx.d2d.CreateStrokeStyle(&props, None).expect("stroke style") };
            *c.borrow_mut() = Some(s.clone());
            s
        })
    }

    /// Arc around (cx, cy) from `start` to `end` radians (0 = 12 o'clock, clockwise).
    pub fn arc(&self, cx: f32, cy: f32, radius: f32, start: f32, end: f32, c: C, width: f32) {
        let sweep = end - start;
        if sweep.abs() < 0.001 {
            return;
        }
        if sweep.abs() >= std::f32::consts::TAU - 0.001 {
            return self.stroke_circle(cx, cy, radius, c, width);
        }
        let pt = |a: f32| Vector2::new(cx + radius * a.sin(), cy - radius * a.cos());
        unsafe {
            let Ok(path) = self.gfx.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            sink.BeginFigure(pt(start), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: pt(end),
                size: D2D_SIZE_F { width: radius, height: radius },
                rotationAngle: 0.0,
                sweepDirection: if sweep > 0.0 { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                arcSize: if sweep.abs() > std::f32::consts::PI { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.rt.DrawGeometry(&path, self.paint(c), width, &self.round_cap());
        }
    }

    pub fn fill_path(&self, path: &ID2D1PathGeometry, c: C) {
        unsafe { self.rt.FillGeometry(path, self.paint(c), None) }
    }

    pub fn stroke_path(&self, path: &ID2D1PathGeometry, c: C, width: f32) {
        unsafe { self.rt.DrawGeometry(path, self.paint(c), width, &self.round_cap()) }
    }

    pub fn text(&self, s: &str, r: R, size: f32, weight: i32, c: C, align: Align) {
        self.text_in(s, r, Font::Text, size, weight, c, align, false);
    }

    pub fn display(&self, s: &str, r: R, size: f32, weight: i32, c: C, align: Align) {
        self.text_in(s, r, Font::Display, size, weight, c, align, false);
    }

    pub fn wrapped(&self, s: &str, r: R, size: f32, weight: i32, c: C, align: Align) {
        self.text_in(s, r, Font::Text, size, weight, c, align, true);
    }

    pub fn glyph(&self, g: char, cx: f32, cy: f32, size: f32, c: C) {
        let mut buf = [0u8; 4];
        let s: &str = g.encode_utf8(&mut buf);
        self.text_in(s, R::centered(cx, cy, size * 2.0, size * 2.0), Font::Icon, size, 400, c, Align::Center, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn text_in(&self, s: &str, r: R, font: Font, size: f32, weight: i32, c: C, align: Align, wrap: bool) {
        if s.is_empty() || r.w <= 0.0 {
            return;
        }
        let Some(f) = self.gfx.format(font, size, weight, align, wrap) else { return };
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            self.rt.DrawText(
                &wide,
                &f,
                &r.d2d(),
                self.paint(c),
                D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        }
    }

    pub fn bitmap(&self, bmp: &ID2D1Bitmap, r: R, opacity: f32) {
        let alpha = self.opacity_stack.last().copied().unwrap_or(1.0);
        unsafe { self.rt.DrawBitmap(bmp, Some(&r.d2d()), opacity * alpha, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) }
    }

    /// Draws a bitmap cropped to fill `r` (like CSS object-fit: cover) with rounded corners.
    pub fn bitmap_cover(&mut self, bmp: &ID2D1Bitmap, r: R, radius: f32, opacity: f32) {
        let size = unsafe { bmp.GetSize() };
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let s = (r.w / size.width).max(r.h / size.height);
        let (w, h) = (size.width * s, size.height * s);
        let dest = R::new(r.cx() - w / 2.0, r.cy() - h / 2.0, w, h);
        self.push_clip_rrect(r, radius, 1.0);
        self.bitmap(bmp, dest, opacity);
        self.pop();
    }

    pub fn create_bitmap(&self, source: &IWICBitmapSource) -> Option<ID2D1Bitmap> {
        unsafe { self.rt.CreateBitmapFromWicBitmap(source, None).ok() }
    }

    pub fn push_clip_rrect(&mut self, r: R, radius: f32, opacity: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        let geo: Option<ID2D1Geometry> =
            unsafe { self.gfx.d2d.CreateRoundedRectangleGeometry(&rr).ok().and_then(|g| g.cast().ok()) };
        self.push_layer(geo, opacity);
    }

    pub fn push_clip_path(&mut self, path: &ID2D1PathGeometry, opacity: f32) {
        self.push_layer(path.cast().ok(), opacity);
    }

    pub fn push_opacity(&mut self, opacity: f32) {
        let parent = self.opacity_stack.last().copied().unwrap_or(1.0);
        self.opacity_stack.push(parent * opacity.clamp(0.0, 1.0));
        // Pair with pop(); no D2D layer needed for plain opacity.
        self.layer_kinds_push(false);
    }

    fn push_layer(&mut self, geo: Option<ID2D1Geometry>, opacity: f32) {
        let params = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
            geometricMask: ManuallyDrop::new(geo),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: 1.0,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        unsafe {
            self.rt.PushLayer(&params, None);
        }
        let mut params = params;
        unsafe { ManuallyDrop::drop(&mut params.geometricMask) };
        let parent = self.opacity_stack.last().copied().unwrap_or(1.0);
        self.opacity_stack.push(parent * opacity.clamp(0.0, 1.0));
        self.layer_kinds_push(true);
    }

    fn layer_kinds_push(&mut self, is_layer: bool) {
        LAYER_KINDS.with(|k| k.borrow_mut().push(is_layer));
    }

    pub fn pop(&mut self) {
        self.opacity_stack.pop();
        if self.opacity_stack.is_empty() {
            self.opacity_stack.push(1.0);
        }
        let was_layer = LAYER_KINDS.with(|k| k.borrow_mut().pop()).unwrap_or(false);
        if was_layer {
            unsafe { self.rt.PopLayer() };
        }
    }

    pub fn translate(&self, dx: f32, dy: f32) {
        unsafe { self.rt.SetTransform(&Matrix3x2::translation(dx, dy)) }
    }

    pub fn transform(&self, m: Matrix3x2) {
        unsafe { self.rt.SetTransform(&m) }
    }

    pub fn reset_transform(&self) {
        unsafe { self.rt.SetTransform(&Matrix3x2::identity()) }
    }

    pub fn path(&self) -> Option<PathBuilder> {
        unsafe {
            let geo = self.gfx.d2d.CreatePathGeometry().ok()?;
            let sink = geo.Open().ok()?;
            Some(PathBuilder { geo, sink, open: false })
        }
    }
}

thread_local! {
    static LAYER_KINDS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

pub struct PathBuilder {
    geo: ID2D1PathGeometry,
    sink: ID2D1GeometrySink,
    open: bool,
}

impl PathBuilder {
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        unsafe {
            if self.open {
                self.sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            self.sink.BeginFigure(Vector2::new(x, y), D2D1_FIGURE_BEGIN_FILLED);
        }
        self.open = true;
        self
    }
    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        unsafe { self.sink.AddLine(Vector2::new(x, y)) };
        self
    }
    pub fn cubic(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) -> &mut Self {
        unsafe {
            self.sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: Vector2::new(x1, y1),
                point2: Vector2::new(x2, y2),
                point3: Vector2::new(x, y),
            })
        };
        self
    }
    pub fn quad(&mut self, x1: f32, y1: f32, x: f32, y: f32) -> &mut Self {
        unsafe {
            self.sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: Vector2::new(x1, y1), point2: Vector2::new(x, y) })
        };
        self
    }
    pub fn arc_to(&mut self, x: f32, y: f32, radius: f32, clockwise: bool) -> &mut Self {
        unsafe {
            self.sink.AddArc(&D2D1_ARC_SEGMENT {
                point: Vector2::new(x, y),
                size: D2D_SIZE_F { width: radius, height: radius },
                rotationAngle: 0.0,
                sweepDirection: if clockwise { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                arcSize: D2D1_ARC_SIZE_SMALL,
            })
        };
        self
    }
    pub fn finish(mut self, closed: bool) -> ID2D1PathGeometry {
        unsafe {
            if self.open {
                self.sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
                self.open = false;
            }
            let _ = self.sink.Close();
        }
        self.geo.clone()
    }
}

pub fn dc_render_target(gfx: &Gfx) -> Result<ID2D1DCRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    unsafe { gfx.d2d.CreateDCRenderTarget(&props) }
}

pub fn hwnd_render_target(gfx: &Gfx, hwnd: HWND, w: u32, h: u32, dpi: f32) -> Result<ID2D1HwndRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: dpi,
        dpiY: dpi,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: D2D_SIZE_U { width: w.max(1), height: h.max(1) },
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    unsafe { gfx.d2d.CreateHwndRenderTarget(&props, &hprops) }
}
