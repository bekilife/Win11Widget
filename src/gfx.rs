//! Drawing: Direct2D (software rasteriser, no GPU device) + DirectWrite, rendered into a
//! 32-bit DIB and pushed to a layered window with per-pixel alpha.
//! All coordinates are CSS-like logical pixels (DIPs); the DPI scale is applied by D2D.

use std::{cell::RefCell, collections::HashMap, mem::ManuallyDrop};

use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::{HWND, POINT, RECT, SIZE},
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA},
    },
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{image::Image, wide};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn hex(v: u32) -> Color {
        Color(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0)
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
    }
    pub const fn white(a: f32) -> Color {
        Color(1.0, 1.0, 1.0, a)
    }
    pub fn alpha(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Display,
    Text,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub fn translate(x: f32, y: f32) -> Matrix3x2 {
    Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: x, M32: y }
}
pub fn scale(s: f32) -> Matrix3x2 {
    Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: 0.0, M32: 0.0 }
}
/// `a` applied first, then `b`.
pub fn mul(a: Matrix3x2, b: Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}
pub fn identity() -> Matrix3x2 {
    scale(1.0)
}

pub struct Gfx {
    pub factory: ID2D1Factory,
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    typo: IDWriteTypography,
    display: Vec<u16>,
    text: Vec<u16>,
    /// ascent, descent, line gap — per em
    metrics: (f32, f32, f32),
    formats: RefCell<HashMap<(Family, u16, u32), IDWriteTextFormat>>,
    paths: RefCell<HashMap<usize, ID2D1PathGeometry>>,
    transform: RefCell<Vec<Matrix3x2>>,
}

fn new_rt(factory: &ID2D1Factory) -> windows::core::Result<ID2D1DCRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 0.0,
        dpiY: 0.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    unsafe { factory.CreateDCRenderTarget(&props) }
}

impl Gfx {
    pub fn new() -> windows::core::Result<Gfx> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let rt = new_rt(&factory)?;
            let brush = rt.CreateSolidColorBrush(&Color::white(1.0).d2d(), None)?;
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let typo = dw.CreateTypography()?;
            typo.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;

            let mut coll = None;
            dw.GetSystemFontCollection(&mut coll, false)?;
            let coll = coll.unwrap();
            let has = |name: &str| {
                let w = wide(name);
                let (mut idx, mut exists) = (0u32, windows::core::BOOL(0));
                coll.FindFamilyName(PCWSTR(w.as_ptr()), &mut idx, &mut exists).is_ok() && exists.as_bool()
            };
            let pick = |a: &str| if has(a) { wide(a) } else { wide("Segoe UI") };
            let display = pick("Segoe UI Variable Display");
            let text = pick("Segoe UI Variable Text");

            // Font metrics for CSS-like "line-height: normal" and baseline placement.
            let mut metrics = (0.92, 0.23, 0.0);
            let (mut idx, mut exists) = (0u32, windows::core::BOOL(0));
            if coll.FindFamilyName(PCWSTR(display.as_ptr()), &mut idx, &mut exists).is_ok() && exists.as_bool() {
                if let Ok(font) = coll.GetFontFamily(idx).and_then(|f| {
                    f.GetFirstMatchingFont(
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                    )
                }) {
                    let mut m = DWRITE_FONT_METRICS::default();
                    font.GetMetrics(&mut m);
                    let em = m.designUnitsPerEm.max(1) as f32;
                    metrics = (m.ascent as f32 / em, m.descent as f32 / em, m.lineGap as f32 / em);
                }
            }

            Ok(Gfx {
                factory,
                dw,
                rt,
                brush,
                round,
                typo,
                display,
                text,
                metrics,
                formats: RefCell::default(),
                paths: RefCell::default(),
                transform: RefCell::new(vec![identity()]),
            })
        }
    }

    /// CSS `line-height: normal` for a font size — Chromium rounds ascent, descent and
    /// line gap to whole pixels separately.
    pub fn normal_lh(&self, size: f32) -> f32 {
        (self.metrics.0 * size).round() + (self.metrics.1 * size).round() + (self.metrics.2 * size).round()
    }

    /// Distance from the top of a line box of height `lh` to the baseline (CSS half-leading model).
    pub fn baseline_in(&self, size: f32, lh: f32) -> f32 {
        let (a, d) = ((self.metrics.0 * size).round(), (self.metrics.1 * size).round());
        ((lh - (a + d)) / 2.0).floor() + a
    }
    /// Baseline offset for `line-height: normal`.
    pub fn baseline(&self, size: f32) -> f32 {
        self.baseline_in(size, self.normal_lh(size))
    }

    fn format(&self, fam: Family, weight: u16, size: f32) -> IDWriteTextFormat {
        let key = (fam, weight, size.to_bits());
        if let Some(f) = self.formats.borrow().get(&key) {
            return f.clone();
        }
        let name = match fam {
            Family::Display => &self.display,
            Family::Text => &self.text,
        };
        let f = unsafe {
            self.dw
                .CreateTextFormat(
                    PCWSTR(name.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    PCWSTR(wide("ru-ru").as_ptr()),
                )
                .unwrap()
        };
        unsafe {
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        }
        self.formats.borrow_mut().insert(key, f.clone());
        f
    }

    pub fn text<'a>(&'a self, s: &'a str, size: f32) -> Text<'a> {
        Text {
            g: self,
            s,
            size,
            weight: 400,
            color: Color::white(1.0),
            lh: None,
            ls: 0.0,
            align: Align::Left,
            width: None,
            ellipsis: false,
            family: Family::Display,
            wrap: false,
        }
    }

    // ---------- frame ----------

    /// Render into `surf` (resized to `w`×`h` device pixels) at `scale`.
    pub fn draw(&self, surf: &mut Surface, w: i32, h: i32, scale: f32, f: impl FnOnce(&Gfx)) {
        surf.ensure(w, h);
        unsafe {
            let rc = RECT { left: 0, top: 0, right: w, bottom: h };
            if self.rt.BindDC(surf.dc, &rc).is_err() {
                return;
            }
            self.rt.SetDpi(96.0 * scale, 96.0 * scale);
            self.rt.BeginDraw();
            self.rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.rt.Clear(Some(&Color(0.0, 0.0, 0.0, 0.0).d2d()));
            *self.transform.borrow_mut() = vec![identity()];
            self.rt.SetTransform(&identity());
            f(self);
            // A software target never loses its device, so there is nothing to recreate.
            let _ = self.rt.EndDraw(None, None);
        }
    }

    pub fn push_transform(&self, m: Matrix3x2) {
        let mut t = self.transform.borrow_mut();
        let cur = mul(m, *t.last().unwrap());
        t.push(cur);
        unsafe { self.rt.SetTransform(&cur) };
    }
    pub fn pop_transform(&self) {
        let mut t = self.transform.borrow_mut();
        if t.len() > 1 {
            t.pop();
        }
        unsafe { self.rt.SetTransform(t.last().unwrap()) };
    }

    fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    // ---------- shapes ----------

    pub fn fill_rect(&self, r: Rect, c: Color) {
        unsafe { self.rt.FillRectangle(&r.d2d(), self.brush(c)) }
    }
    pub fn fill_rrect(&self, r: Rect, rad: f32, c: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.brush(c)) }
    }
    pub fn stroke_rrect(&self, r: Rect, rad: f32, width: f32, c: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.brush(c), width, None) }
    }
    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r, radiusY: r };
        unsafe { self.rt.FillEllipse(&e, self.brush(c)) }
    }
    pub fn stroke_circle(&self, cx: f32, cy: f32, r: f32, width: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r, radiusY: r };
        unsafe { self.rt.DrawEllipse(&e, self.brush(c), width, None) }
    }
    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: Color, round: bool) {
        unsafe {
            if round {
                self.rt.DrawLine(v2(x1, y1), v2(x2, y2), self.brush(c), width, &self.round)
            } else {
                self.rt.DrawLine(v2(x1, y1), v2(x2, y2), self.brush(c), width, None)
            }
        }
    }

    /// Progress ring: background circle + arc clockwise from 12 o'clock, round caps.
    pub fn ring(&self, cx: f32, cy: f32, r: f32, width: f32, pct: f32, fg: Color, bg: Color) {
        self.stroke_circle(cx, cy, r, width, bg);
        let p = pct.clamp(0.0, 100.0) / 100.0;
        if p <= 0.0 {
            return;
        }
        unsafe {
            if p >= 0.9999 {
                self.stroke_circle(cx, cy, r, width, fg);
                return;
            }
            let Ok(geo) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            let a = -std::f32::consts::FRAC_PI_2 + p * std::f32::consts::TAU;
            sink.BeginFigure(v2(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: v2(cx + r * a.cos(), cy + r * a.sin()),
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if p > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.rt.DrawGeometry(&geo, self.brush(fg), width, &self.round);
        }
    }

    /// Vertical linear gradient inside a rounded rectangle.
    pub fn fill_rrect_vgrad(&self, r: Rect, rad: f32, top: Color, bottom: Color) {
        self.fill_rrect_grad(r, rad, v2(r.x, r.y), v2(r.x, r.y + r.h), top, bottom);
    }
    pub fn fill_rrect_grad(&self, r: Rect, rad: f32, p0: Vector2, p1: Vector2, c0: Color, c1: Color) {
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: c0.d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: c1.d2d() },
            ];
            let Ok(coll) = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else {
                return;
            };
            let Ok(b) = self.rt.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: p0, endPoint: p1 },
                None,
                &coll,
            ) else {
                return;
            };
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.rt.FillRoundedRectangle(&rr, &b);
        }
    }

    fn poly(&self, pts: &[(f32, f32)], close_to: Option<f32>) -> Option<ID2D1PathGeometry> {
        let (first, rest) = pts.split_first()?;
        unsafe {
            let geo = self.factory.CreatePathGeometry().ok()?;
            let sink = geo.Open().ok()?;
            let filled = close_to.is_some();
            sink.BeginFigure(
                v2(first.0, first.1),
                if filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW },
            );
            for p in rest {
                sink.AddLine(v2(p.0, p.1));
            }
            if let Some(base) = close_to {
                let last = pts[pts.len() - 1];
                sink.AddLine(v2(last.0, base));
                sink.AddLine(v2(first.0, base));
            }
            sink.EndFigure(if filled { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            sink.Close().ok()?;
            Some(geo)
        }
    }

    /// Polyline with round joins and caps.
    pub fn stroke_poly(&self, pts: &[(f32, f32)], width: f32, c: Color) {
        if let Some(geo) = self.poly(pts, None) {
            unsafe { self.rt.DrawGeometry(&geo, self.brush(c), width, &self.round) }
        }
    }

    /// Area under a polyline down to `base`, filled with a vertical gradient.
    pub fn fill_under(&self, pts: &[(f32, f32)], top: f32, base: f32, c0: Color, c1: Color) {
        let Some(geo) = self.poly(pts, Some(base)) else { return };
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: c0.d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: c1.d2d() },
            ];
            let Ok(coll) = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else {
                return;
            };
            let Ok(b) = self.rt.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v2(0.0, top), endPoint: v2(0.0, base) },
                None,
                &coll,
            ) else {
                return;
            };
            self.rt.FillGeometry(&geo, &b, None);
        }
    }

    /// CSS `box-shadow: inset 0 1px 0 color` on a rounded rectangle: the sliver between the
    /// shape and the same shape moved down by 1 px.
    pub fn inset_top_highlight(&self, r: Rect, rad: f32, c: Color) {
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            let Ok(a) = self.factory.CreateRoundedRectangleGeometry(&rr) else { return };
            let Ok(path) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            let shift = translate(0.0, 1.0);
            if a.CombineWithGeometry(&a, D2D1_COMBINE_MODE_EXCLUDE, Some(&shift), 0.05, &sink).is_ok() {
                let _ = sink.Close();
                self.rt.FillGeometry(&path, self.brush(c), None);
            }
        }
    }

    /// Soft drop shadow approximation for a rounded rectangle (`box-shadow: 0 dy blur color`).
    pub fn shadow(&self, r: Rect, rad: f32, dy: f32, blur: f32, c: Color) {
        let steps = 7;
        for i in 0..steps {
            let t = (i as f32 + 0.5) / steps as f32;
            let grow = blur * (t - 0.5);
            let a = c.3 / steps as f32 * 1.6 * (1.0 - t * 0.6);
            self.fill_rrect(
                Rect::new(r.x - grow, r.y + dy - grow, r.w + 2.0 * grow, r.h + 2.0 * grow),
                (rad + grow).max(0.0),
                c.alpha(a / c.3.max(0.001)),
            );
        }
    }

    // ---------- clipping ----------

    pub fn push_clip_rrect(&self, r: Rect, rad: f32) {
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            let geo: Option<ID2D1Geometry> =
                self.factory.CreateRoundedRectangleGeometry(&rr).ok().and_then(|g| g.cast().ok());
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: ManuallyDrop::new(geo),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: identity(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.rt.PushLayer(&params, None);
            let _ = ManuallyDrop::into_inner(params.geometricMask);
        }
    }
    pub fn push_clip_rect(&self, r: Rect) {
        unsafe { self.rt.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) }
    }
    pub fn pop_clip_rect(&self) {
        unsafe { self.rt.PopAxisAlignedClip() }
    }
    pub fn pop_layer(&self) {
        unsafe { self.rt.PopLayer() }
    }

    // ---------- bitmaps ----------

    pub fn bitmap(&self, img: &Image) -> Option<ID2D1Bitmap> {
        unsafe {
            self.rt
                .CreateBitmap(
                    D2D_SIZE_U { width: img.w, height: img.h },
                    Some(img.px.as_ptr() as _),
                    img.w * 4,
                    &D2D1_BITMAP_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                    },
                )
                .ok()
        }
    }

    pub fn draw_bitmap(&self, b: &ID2D1Bitmap, dst: Rect) {
        unsafe { self.rt.DrawBitmap(b, Some(&dst.d2d()), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) }
    }

    /// Fill a rounded rect with a bitmap mapped through `m` (bitmap pixels -> DIPs).
    pub fn fill_rrect_bitmap(&self, r: Rect, rad: f32, b: &ID2D1Bitmap, m: Matrix3x2) {
        unsafe {
            let Ok(brush) = self.rt.CreateBitmapBrush(
                b,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                }),
                None,
            ) else {
                return;
            };
            brush.SetTransform(&m);
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.rt.FillRoundedRectangle(&rr, &brush);
        }
    }

    // ---------- SVG paths ----------

    fn path(&self, d: &'static str) -> Option<ID2D1PathGeometry> {
        let key = d.as_ptr() as usize;
        if let Some(p) = self.paths.borrow().get(&key) {
            return Some(p.clone());
        }
        let p = svg_path(&self.factory, d)?;
        self.paths.borrow_mut().insert(key, p.clone());
        Some(p)
    }
    pub fn fill_path(&self, d: &'static str, c: Color) {
        if let Some(p) = self.path(d) {
            unsafe { self.rt.FillGeometry(&p, self.brush(c), None) }
        }
    }
    pub fn stroke_path(&self, d: &'static str, width: f32, c: Color) {
        if let Some(p) = self.path(d) {
            unsafe { self.rt.DrawGeometry(&p, self.brush(c), width, &self.round) }
        }
    }
}

// ---------- text ----------

pub struct Text<'a> {
    g: &'a Gfx,
    s: &'a str,
    size: f32,
    weight: u16,
    color: Color,
    lh: Option<f32>,
    ls: f32,
    align: Align,
    width: Option<f32>,
    ellipsis: bool,
    family: Family,
    wrap: bool,
}

impl Text<'_> {
    pub fn weight(mut self, w: u16) -> Self {
        self.weight = w;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    /// Line height in px (default: CSS `normal`).
    pub fn lh(mut self, lh: f32) -> Self {
        self.lh = Some(lh);
        self
    }
    /// Letter spacing in px.
    pub fn ls(mut self, ls: f32) -> Self {
        self.ls = ls;
        self
    }
    /// Box width for alignment / ellipsis.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w.max(0.0));
        self
    }
    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
    pub fn ellipsis(mut self) -> Self {
        self.ellipsis = true;
        self
    }
    pub fn family(mut self, f: Family) -> Self {
        self.family = f;
        self
    }
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }

    pub fn line_height(&self) -> f32 {
        self.lh.unwrap_or_else(|| self.g.normal_lh(self.size))
    }

    fn layout(&self) -> Option<IDWriteTextLayout> {
        let g = self.g;
        let w16: Vec<u16> = self.s.encode_utf16().collect();
        let fmt = g.format(self.family, self.weight, self.size);
        let lh = self.line_height();
        let baseline = g.baseline_in(self.size, lh);
        unsafe {
            let l = g.dw.CreateTextLayout(&w16, &fmt, self.width.unwrap_or(100_000.0), 10_000.0).ok()?;
            let all = DWRITE_TEXT_RANGE { startPosition: 0, length: w16.len() as u32 };
            let _ = l.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, lh, baseline);
            let _ = l.SetTypography(&g.typo, all);
            if self.wrap {
                let _ = l.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            }
            if self.ls != 0.0 {
                if let Ok(l1) = l.cast::<IDWriteTextLayout1>() {
                    let _ = l1.SetCharacterSpacing(0.0, self.ls, 0.0, all);
                }
            }
            let align = match self.align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            };
            if self.width.is_some() {
                let _ = l.SetTextAlignment(align);
            }
            if self.ellipsis {
                if let Ok(sign) = g.dw.CreateEllipsisTrimmingSign(&fmt) {
                    let t = DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    };
                    let _ = l.SetTrimming(&t, &sign);
                }
            }
            Some(l)
        }
    }

    /// Natural width of the text.
    pub fn measure(&self) -> f32 {
        let Some(l) = self.layout() else { return 0.0 };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = l.GetMetrics(&mut m);
        }
        m.widthIncludingTrailingWhitespace
    }

    /// Height of wrapped text (with `.wrap()` and `.width()`).
    pub fn measure_height(&self) -> f32 {
        let Some(l) = self.layout() else { return 0.0 };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = l.GetMetrics(&mut m);
        }
        m.height
    }

    /// Draw with the top of the line box at (x, y). Returns the natural text width.
    pub fn draw(&self, x: f32, y: f32) -> f32 {
        let Some(l) = self.layout() else { return 0.0 };
        unsafe {
            self.g.rt.DrawTextLayout(v2(x, y), &l, self.g.brush(self.color), D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }
}

// ---------- SVG path data -> D2D geometry (M L H V C S Q A Z, absolute and relative) ----------

fn svg_path(factory: &ID2D1Factory, d: &str) -> Option<ID2D1PathGeometry> {
    let b = d.as_bytes();
    let mut i = 0;
    let skip = |i: &mut usize| {
        while *i < b.len() && (b[*i] == b' ' || b[*i] == b',' || b[*i] == b'\n') {
            *i += 1;
        }
    };
    let num = |i: &mut usize| -> Option<f32> {
        skip(i);
        let start = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        let mut dot = false;
        while *i < b.len() && (b[*i].is_ascii_digit() || (b[*i] == b'.' && !dot)) {
            dot |= b[*i] == b'.';
            *i += 1;
        }
        if *i < b.len() && (b[*i] == b'e' || b[*i] == b'E') {
            *i += 1;
            if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
                *i += 1;
            }
            while *i < b.len() && b[*i].is_ascii_digit() {
                *i += 1;
            }
        }
        std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
    };
    let flag = |i: &mut usize| -> Option<bool> {
        skip(i);
        let c = *b.get(*i)?;
        *i += 1;
        Some(c == b'1')
    };
    unsafe {
        let geo = factory.CreatePathGeometry().ok()?;
        let sink = geo.Open().ok()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let (mut cx, mut cy) = (0f32, 0f32);
        let (mut sx, mut sy) = (0f32, 0f32);
        let mut open = false;
        let mut cmd = b'M';
        let mut last_ctrl: Option<(f32, f32)> = None;
        loop {
            skip(&mut i);
            if i >= b.len() {
                break;
            }
            if b[i].is_ascii_alphabetic() {
                cmd = b[i];
                i += 1;
            }
            let rel = cmd.is_ascii_lowercase();
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            match cmd.to_ascii_uppercase() {
                b'M' => {
                    let (x, y) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(v2(x, y), D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                    (cx, cy, sx, sy) = (x, y, x, y);
                    cmd = if rel { b'l' } else { b'L' }; // implicit lineto after moveto
                    last_ctrl = None;
                }
                b'L' => {
                    let (x, y) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    sink.AddLine(v2(x, y));
                    (cx, cy) = (x, y);
                    last_ctrl = None;
                }
                b'H' => {
                    let x = num(&mut i)? + ox;
                    sink.AddLine(v2(x, cy));
                    cx = x;
                    last_ctrl = None;
                }
                b'V' => {
                    let y = num(&mut i)? + oy;
                    sink.AddLine(v2(cx, y));
                    cy = y;
                    last_ctrl = None;
                }
                b'C' | b'S' => {
                    let (x1, y1) = if cmd.eq_ignore_ascii_case(&b'C') {
                        (num(&mut i)? + ox, num(&mut i)? + oy)
                    } else {
                        last_ctrl.map_or((cx, cy), |(lx, ly)| (2.0 * cx - lx, 2.0 * cy - ly))
                    };
                    let (x2, y2) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    let (x, y) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v2(x1, y1), point2: v2(x2, y2), point3: v2(x, y) });
                    (cx, cy) = (x, y);
                    last_ctrl = Some((x2, y2));
                }
                b'Q' => {
                    let (qx, qy) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    let (x, y) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: v2(cx + 2.0 / 3.0 * (qx - cx), cy + 2.0 / 3.0 * (qy - cy)),
                        point2: v2(x + 2.0 / 3.0 * (qx - x), y + 2.0 / 3.0 * (qy - y)),
                        point3: v2(x, y),
                    });
                    (cx, cy) = (x, y);
                    last_ctrl = None;
                }
                b'A' => {
                    let (rx, ry, rot) = (num(&mut i)?, num(&mut i)?, num(&mut i)?);
                    let (large, sweep) = (flag(&mut i)?, flag(&mut i)?);
                    let (x, y) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: v2(x, y),
                        size: D2D_SIZE_F { width: rx, height: ry },
                        rotationAngle: rot,
                        sweepDirection: if sweep {
                            D2D1_SWEEP_DIRECTION_CLOCKWISE
                        } else {
                            D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                        },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                    (cx, cy) = (x, y);
                    last_ctrl = None;
                }
                b'Z' => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    (cx, cy) = (sx, sy);
                    last_ctrl = None;
                    // a following number without a command would be invalid; next must be a command
                    skip(&mut i);
                    if i < b.len() && !b[i].is_ascii_alphabetic() {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close().ok()?;
        Some(geo)
    }
}

// ---------- layered window surface ----------

pub struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new() -> Surface {
        Surface {
            dc: unsafe { CreateCompatibleDC(None) },
            bmp: HBITMAP::default(),
            old: HGDIOBJ::default(),
            bits: std::ptr::null_mut(),
            w: 0,
            h: 0,
        }
    }

    fn ensure(&mut self, w: i32, h: i32) {
        if w == self.w && h == self.h && !self.bmp.is_invalid() {
            return;
        }
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bmp.into());
            }
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w.max(1),
                    biHeight: -h.max(1),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            self.bmp = CreateDIBSection(Some(self.dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
            self.old = SelectObject(self.dc, self.bmp.into());
            self.bits = bits as *mut u8;
        }
        self.w = w;
        self.h = h;
    }

    /// Show the rendered pixels in a layered window, optionally moving it.
    pub fn present(&self, hwnd: HWND, pos: Option<(i32, i32)>) {
        unsafe {
            let size = SIZE { cx: self.w, cy: self.h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dst = pos.map(|(x, y)| POINT { x, y });
            let _ = UpdateLayeredWindow(
                hwnd,
                None,
                dst.as_ref().map(|p| p as *const POINT),
                Some(&size),
                Some(self.dc),
                Some(&src),
                windows::Win32::Foundation::COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    /// Debug builds: write the pixels as a 32-bit BMP (premultiplied BGRA).
    #[cfg(debug_assertions)]
    pub fn dump(&self, path: &std::path::Path) {
        if self.bits.is_null() {
            return;
        }
        let n = (self.w * self.h * 4) as usize;
        let px = unsafe { std::slice::from_raw_parts(self.bits, n) };
        let mut f = Vec::with_capacity(n + 54);
        let le32 = |v: u32| v.to_le_bytes();
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&le32(54 + n as u32));
        f.extend_from_slice(&[0; 4]);
        f.extend_from_slice(&le32(54));
        f.extend_from_slice(&le32(40));
        f.extend_from_slice(&le32(self.w as u32));
        f.extend_from_slice(&(-self.h).to_le_bytes());
        f.extend_from_slice(&[1, 0, 32, 0]);
        f.extend_from_slice(&[0; 24]);
        f.extend_from_slice(px);
        let _ = std::fs::write(path, f);
    }

    /// Copy to a normal window's DC.
    pub fn blit(&self, dc: HDC) {
        unsafe {
            let _ = BitBlt(dc, 0, 0, self.w, self.h, Some(self.dc), 0, 0, SRCCOPY);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bmp.into());
            }
            let _ = DeleteDC(self.dc);
        }
    }
}
