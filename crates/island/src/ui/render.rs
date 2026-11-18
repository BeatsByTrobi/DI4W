//! Drawing with Direct2D into a per-pixel-alpha layered window.
//!
//! The C# island was a transparent WPF window: Windows sends clicks on fully transparent pixels to whatever is
//! below, so only the pill (and the bubble, the dots) catch the mouse. The same holds here: frames are drawn on the
//! GPU with Direct2D (and DirectWrite, and the Gaussian blur / shadow effects) into a GDI-compatible bitmap, which
//! is handed to `UpdateLayeredWindow`. A frame is only drawn when something changed or is animating.

use super::colors::Color;
use super::el::{Brush, El, Kind, Paint, Shape};
use super::geom::{Path, Point, Rect, Seg};
use super::image;
use std::collections::HashMap;
use windows::Win32::Foundation::{HMODULE, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device};
use windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_RANGE;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

/// Hover and press state of buttons, for drawing their feedback.
#[derive(Default)]
pub struct PaintState {
    pub hovered_button: Option<u64>,
    /// Current press scale per button key (0.88 while held, springing back to 1).
    pub press: HashMap<u64, f64>,
}

struct Device {
    _d3d: ID3D11Device,
    dc: ID2D1DeviceContext,
    target: Option<(ID2D1Bitmap1, u32, u32)>,
    images: HashMap<u64, ID2D1Bitmap1>,
}

/// Draws frames for one layered window.
pub struct Renderer {
    factory: ID2D1Factory1,
    device: Option<Device>,
}

fn color_f(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: c.a as f32 / 255.0 }
}

fn v2(p: Point) -> Vector2 {
    Vector2 { X: p.x as f32, Y: p.y as f32 }
}

fn rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x as f32, top: r.y as f32, right: (r.x + r.w) as f32, bottom: (r.y + r.h) as f32 }
}

fn identity() -> Matrix3x2 {
    Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
}

/// A transform that scales by `s` around `(cx, cy)`.
pub fn scale_around(sx: f64, sy: f64, cx: f64, cy: f64) -> Matrix3x2 {
    Matrix3x2 {
        M11: sx as f32,
        M12: 0.0,
        M21: 0.0,
        M22: sy as f32,
        M31: (cx - sx * cx) as f32,
        M32: (cy - sy * cy) as f32,
    }
}

pub fn translation(dx: f64, dy: f64) -> Matrix3x2 {
    Matrix3x2 { M31: dx as f32, M32: dy as f32, ..identity() }
}

/// `a` then `b` (Direct2D's row-vector order).
fn mul(a: &Matrix3x2, b: &Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        let factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        Ok(Renderer { factory, device: None })
    }

    fn create_device(&self) -> Result<Device> {
        unsafe {
            let make = |kind: D3D_DRIVER_TYPE| -> Result<ID3D11Device> {
                let mut device = None;
                D3D11CreateDevice(None, kind, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
                device.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
            };
            let d3d = make(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| make(D3D_DRIVER_TYPE_WARP))?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let device = self.factory.CreateDevice(&dxgi)?;
            let dc = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            // No ClearType on a transparent window (it needs an opaque background); grayscale like WPF there.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Ok(Device { _d3d: d3d, dc, target: None, images: HashMap::new() })
        }
    }

    /// True when the point is inside the filled outline.
    pub fn contains(&self, path: &Path, p: Point) -> bool {
        let Ok(geometry) = geometry(&self.factory, path) else { return false };
        unsafe { geometry.FillContainsPoint(v2(p), None, 0.25).map(|b| b.as_bool()).unwrap_or(false) }
    }

    /// Draws a frame of `w` × `h` pixels with `draw` (in DIPs, scaled by `scale` to pixels) and shows it on the
    /// layered window, its top-left at `pos` (screen pixels).
    pub fn present(&mut self, hwnd: HWND, pos: POINT, w: u32, h: u32, scale: f64, state: &PaintState, draw: &mut dyn FnMut(&mut Painter)) -> Result<()> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        if self.device.is_none() {
            self.device = Some(self.create_device()?);
        }
        let result = self.draw_frame(hwnd, pos, w, h, scale, state, draw);
        if let Err(e) = &result {
            // Lost the GPU (driver update, sleep): start over with a fresh device next frame.
            if e.code().0 as u32 == 0x8899_000C || e.code().0 as u32 == 0x887A_0005 {
                self.device = None;
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_frame(&mut self, hwnd: HWND, pos: POINT, w: u32, h: u32, scale: f64, state: &PaintState, draw: &mut dyn FnMut(&mut Painter)) -> Result<()> {
        let device = self.device.as_mut().unwrap();
        unsafe {
            if device.target.as_ref().is_none_or(|t| t.1 != w || t.2 != h) {
                device.dc.SetTarget(None);
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_GDI_COMPATIBLE,
                    colorContext: std::mem::ManuallyDrop::new(None),
                };
                let bitmap = device.dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &props)?;
                device.target = Some((bitmap, w, h));
            }
            let target = device.target.as_ref().unwrap().0.clone();
            device.dc.SetTarget(&target);
            device.dc.BeginDraw();
            device.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            // Pictures no longer registered are dropped from the GPU.
            device.images.retain(|id, _| image::pixels(*id).is_some());

            let base = scale_around(scale, scale, 0.0, 0.0);
            device.dc.SetTransform(&base);
            {
                let mut painter = Painter { dc: &device.dc, factory: &self.factory, images: &mut device.images, state, transform: base, pixel_scale: scale };
                draw(&mut painter);
            }
            device.dc.SetTransform(&identity());

            let interop: ID2D1GdiInteropRenderTarget = device.dc.cast()?;
            let shown = match interop.GetDC(D2D1_DC_INITIALIZE_MODE_COPY) {
                Ok(hdc) => {
                    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                    let size = SIZE { cx: w as i32, cy: h as i32 };
                    let src = POINT { x: 0, y: 0 };
                    let r = UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&size), Some(hdc), Some(&src), Default::default(), Some(&blend), ULW_ALPHA);
                    let _ = interop.ReleaseDC(None);
                    r
                }
                Err(e) => Err(e),
            };
            device.dc.EndDraw(None, None)?;
            shown
        }
    }
}

/// Builds a Direct2D geometry for the outline.
fn geometry(factory: &ID2D1Factory1, path: &Path) -> Result<ID2D1PathGeometry1> {
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        let mut open = false;
        let mut last = Point::default();
        for seg in &path.segs {
            match *seg {
                Seg::Move(p) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.BeginFigure(v2(p), D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                    last = p;
                }
                Seg::Close => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
                _ => {
                    if !open {
                        sink.BeginFigure(v2(last), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    match *seg {
                        Seg::Line(p) => {
                            sink.AddLine(v2(p));
                            last = p;
                        }
                        Seg::Quad(a, b) => {
                            sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: v2(a), point2: v2(b) });
                            last = b;
                        }
                        Seg::Cubic(a, b, c) => {
                            sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v2(a), point2: v2(b), point3: v2(c) });
                            last = c;
                        }
                        Seg::Arc(p, r, large, clockwise) => {
                            sink.AddArc(&D2D1_ARC_SEGMENT {
                                point: v2(p),
                                size: D2D_SIZE_F { width: r as f32, height: r as f32 },
                                rotationAngle: 0.0,
                                sweepDirection: if clockwise { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                                arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                            });
                            last = p;
                        }
                        _ => {}
                    }
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
        sink.Close()?;
        Ok(geometry)
    }
}

/// A rounded rectangle with its own radius per corner (top-left, top-right, bottom-right, bottom-left), each
/// clamped to fit, as an outline.
pub fn rounded_rect(r: Rect, radii: [f64; 4]) -> Path {
    let limit = (r.w.min(r.h) / 2.0).max(0.0);
    let [tl, tr, br, bl] = radii.map(|v| v.clamp(0.0, limit));
    let mut p = Path::default();
    p.segs.push(Seg::Move(Point::new(r.x + tl, r.y)));
    p.segs.push(Seg::Line(Point::new(r.right() - tr, r.y)));
    if tr > 0.0 {
        p.segs.push(Seg::Arc(Point::new(r.right(), r.y + tr), tr, false, true));
    }
    p.segs.push(Seg::Line(Point::new(r.right(), r.bottom() - br)));
    if br > 0.0 {
        p.segs.push(Seg::Arc(Point::new(r.right() - br, r.bottom()), br, false, true));
    }
    p.segs.push(Seg::Line(Point::new(r.x + bl, r.bottom())));
    if bl > 0.0 {
        p.segs.push(Seg::Arc(Point::new(r.x, r.bottom() - bl), bl, false, true));
    }
    p.segs.push(Seg::Line(Point::new(r.x, r.y + tl)));
    if tl > 0.0 {
        p.segs.push(Seg::Arc(Point::new(r.x + tl, r.y), tl, false, true));
    }
    p.segs.push(Seg::Close);
    p
}

/// Draws one frame; all coordinates are DIPs.
pub struct Painter<'a> {
    dc: &'a ID2D1DeviceContext,
    factory: &'a ID2D1Factory1,
    images: &'a mut HashMap<u64, ID2D1Bitmap1>,
    pub state: &'a PaintState,
    transform: Matrix3x2,
    /// Pixels per DIP (display scale × the island's size setting).
    pub pixel_scale: f64,
}

impl Painter<'_> {
    fn brush(&self, color: Color) -> Option<ID2D1SolidColorBrush> {
        unsafe { self.dc.CreateSolidColorBrush(&color_f(color), None).ok() }
    }

    fn any_brush(&mut self, brush: Brush, rect: Rect) -> Option<ID2D1Brush> {
        unsafe {
            match brush {
                Brush::Solid(c) => self.brush(c).and_then(|b| b.cast().ok()),
                Brush::Vertical(top, bottom) => {
                    let stops = [
                        D2D1_GRADIENT_STOP { position: 0.0, color: color_f(top) },
                        D2D1_GRADIENT_STOP { position: 1.0, color: color_f(bottom) },
                    ];
                    let rt: &ID2D1RenderTarget = self.dc;
                    let collection = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
                    let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: v2(Point::new(rect.x, rect.y)),
                        endPoint: v2(Point::new(rect.x, rect.bottom())),
                    };
                    self.dc.CreateLinearGradientBrush(&props, None, &collection).ok().and_then(|b| b.cast().ok())
                }
                Brush::Image(id) => {
                    let bitmap = self.bitmap(id)?;
                    let size = bitmap.GetPixelSize();
                    let (iw, ih) = (size.width as f64, size.height as f64);
                    // Stretch.UniformToFill: cover the box, centered.
                    let s = (rect.w / iw).max(rect.h / ih);
                    let (dx, dy) = (rect.x + (rect.w - iw * s) / 2.0, rect.y + (rect.h - ih * s) / 2.0);
                    let props = D2D1_BITMAP_BRUSH_PROPERTIES1 {
                        extendModeX: D2D1_EXTEND_MODE_CLAMP,
                        extendModeY: D2D1_EXTEND_MODE_CLAMP,
                        interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    };
                    let brush = self.dc.CreateBitmapBrush(&bitmap, Some(&props), None).ok()?;
                    let m = mul(&scale_around(s, s, 0.0, 0.0), &translation(dx, dy));
                    brush.SetTransform(&m);
                    brush.cast().ok()
                }
            }
        }
    }

    fn bitmap(&mut self, id: u64) -> Option<ID2D1Bitmap1> {
        if let Some(b) = self.images.get(&id) {
            return Some(b.clone());
        }
        let pixels = image::pixels(id)?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let bitmap = unsafe {
            self.dc
                .CreateBitmap(D2D_SIZE_U { width: pixels.w, height: pixels.h }, Some(pixels.pixels.as_ptr().cast()), pixels.w * 4, &props)
                .ok()?
        };
        self.images.insert(id, bitmap.clone());
        Some(bitmap)
    }

    pub fn push_transform(&mut self, local: Matrix3x2) -> Matrix3x2 {
        let saved = self.transform;
        self.transform = mul(&local, &self.transform);
        unsafe { self.dc.SetTransform(&self.transform) };
        saved
    }

    pub fn restore_transform(&mut self, saved: Matrix3x2) {
        self.transform = saved;
        unsafe { self.dc.SetTransform(&self.transform) };
    }

    fn layer(&self, opacity: f64, mask: Option<ID2D1Geometry>) {
        let params = D2D1_LAYER_PARAMETERS1 {
            contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
            geometricMask: std::mem::ManuallyDrop::new(mask),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: identity(),
            opacity: opacity as f32,
            opacityBrush: std::mem::ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        unsafe { self.dc.PushLayer(&params, None) };
        let _ = std::mem::ManuallyDrop::into_inner(params.geometricMask);
    }

    /// Everything until [`Painter::pop`] is drawn at this opacity as a group.
    pub fn push_opacity(&mut self, opacity: f64) {
        self.layer(opacity, None);
    }

    /// Everything until [`Painter::pop`] is clipped to the outline (anti-aliased).
    pub fn push_clip(&mut self, outline: &Path) {
        let mask = geometry(self.factory, outline).ok().and_then(|g| g.cast::<ID2D1Geometry>().ok());
        self.layer(1.0, mask);
    }

    pub fn pop(&mut self) {
        unsafe { self.dc.PopLayer() };
    }

    pub fn fill(&mut self, path: &Path, brush: Brush) {
        let Ok(g) = geometry(self.factory, path) else { return };
        let bounds = path.bounds();
        if let Some(b) = self.any_brush(brush, bounds) {
            unsafe { self.dc.FillGeometry(&g, &b, None) };
        }
    }

    pub fn stroke(&mut self, path: &Path, color: Color, thickness: f64, round_caps: bool) {
        let Ok(g) = geometry(self.factory, path) else { return };
        let Some(b) = self.brush(color) else { return };
        let cap = if round_caps { D2D1_CAP_STYLE_ROUND } else { D2D1_CAP_STYLE_FLAT };
        let props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: cap,
            endCap: cap,
            dashCap: cap,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        unsafe {
            let factory: &ID2D1Factory = self.factory;
            let style = factory.CreateStrokeStyle(&props, None).ok();
            self.dc.DrawGeometry(&g, &b, thickness as f32, style.as_ref());
        }
    }

    pub fn fill_rect(&mut self, rect: Rect, radii: [f64; 4], brush: Brush) {
        if radii.iter().all(|r| (*r - radii[0]).abs() < 1e-9) {
            let r = radii[0].clamp(0.0, rect.w.min(rect.h) / 2.0) as f32;
            if let Some(b) = self.any_brush(brush, rect) {
                let rr = D2D1_ROUNDED_RECT { rect: rect_f(rect), radiusX: r, radiusY: r };
                unsafe { self.dc.FillRoundedRectangle(&rr, &b) };
            }
        } else {
            let path = rounded_rect(rect, radii);
            self.fill(&path, brush);
        }
    }

    pub fn ellipse(&mut self, rect: Rect, fill: Option<Color>, stroke: Option<(Color, f64)>) {
        let e = |inset: f64| D2D1_ELLIPSE {
            point: v2(rect.center()),
            radiusX: ((rect.w / 2.0) - inset).max(0.0) as f32,
            radiusY: ((rect.h / 2.0) - inset).max(0.0) as f32,
        };
        unsafe {
            if let Some(c) = fill
                && let Some(b) = self.brush(c) {
                    self.dc.FillEllipse(&e(0.0), &b);
                }
            if let Some((c, t)) = stroke
                && let Some(b) = self.brush(c) {
                    self.dc.DrawEllipse(&e(t / 2.0), &b, t as f32, None);
                }
        }
    }

    /// Draws the text's runs in their colors at `origin`, faded by `opacity`.
    pub fn text(&mut self, layout: &super::text::TextLayout, runs: &[super::el::Run], origin: Point) {
        let Some(l) = &layout.layout else { return };
        let Some(first) = runs.first() else { return };
        let Some(default) = self.brush(first.color) else { return };
        unsafe {
            let mut start = 0u32;
            for run in runs {
                let len = run.text.encode_utf16().count() as u32;
                if let Some(b) = self.brush(run.color) {
                    let _ = l.SetDrawingEffect(&b, DWRITE_TEXT_RANGE { startPosition: start, length: len });
                }
                start += len;
            }
            // Snap the origin to whole pixels, so text stays crisp (WPF's layout rounding).
            let s = self.pixel_scale.max(0.01);
            let snapped = Point::new((origin.x * s).round() / s, (origin.y * s).round() / s);
            self.dc.DrawTextLayout(v2(snapped), l, &default, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }

    /// Records drawing into a command list instead of the frame (for blurring it as a whole). Call it while no
    /// clip or layer is pushed.
    pub fn record(&mut self, draw: &mut dyn FnMut(&mut Painter)) -> Option<ID2D1CommandList> {
        unsafe {
            let previous = self.dc.GetTarget().ok()?;
            let list = self.dc.CreateCommandList().ok()?;
            self.dc.SetTarget(&list);
            draw(self);
            self.dc.SetTarget(&previous);
            list.Close().ok()?;
            Some(list)
        }
    }

    /// Draws recorded content through a Gaussian blur of `sigma` DIPs (0 draws it sharp).
    pub fn draw_blurred(&mut self, list: &ID2D1CommandList, sigma: f64) {
        unsafe {
            let saved = self.transform;
            self.dc.SetTransform(&identity());
            if sigma > 0.05
                && let Ok(effect) = self.dc.CreateEffect(&CLSID_D2D1GaussianBlur) {
                    effect.SetInput(0, list, true);
                    let sd = (sigma * self.pixel_scale) as f32;
                    let _ = effect.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sd.to_le_bytes());
                    let _ = effect.SetValue(
                        D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32,
                        D2D1_PROPERTY_TYPE_ENUM,
                        &(D2D1_GAUSSIANBLUR_OPTIMIZATION_SPEED.0 as u32).to_le_bytes(),
                    );
                    if let Ok(output) = effect.GetOutput() {
                        self.dc.DrawImage(&output, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                    }
                    self.dc.SetTransform(&saved);
                    return;
                }
            self.dc.DrawImage(list, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            self.dc.SetTransform(&saved);
        }
    }

    /// A soft drop shadow of the outline (WPF's `DropShadowEffect`): blurred by `sigma`, moved by the offset.
    pub fn shadow(&mut self, outline: &Path, color: Color, sigma: f64, dx: f64, dy: f64) {
        let Some(list) = self.record(&mut |p| p.fill(outline, Brush::Solid(Color::BLACK))) else { return };
        unsafe {
            let Ok(effect) = self.dc.CreateEffect(&CLSID_D2D1Shadow) else { return };
            effect.SetInput(0, &list, true);
            let sd = (sigma * self.pixel_scale) as f32;
            let _ = effect.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sd.to_le_bytes());
            let c = color_f(color);
            let mut bytes = Vec::with_capacity(16);
            for v in [c.r, c.g, c.b, c.a] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = effect.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &bytes);
            let saved = self.transform;
            self.dc.SetTransform(&identity());
            if let Ok(output) = effect.GetOutput() {
                let offset = Vector2 { X: (dx * self.pixel_scale) as f32, Y: (dy * self.pixel_scale) as f32 };
                self.dc.DrawImage(&output, Some(&offset), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            }
            self.dc.SetTransform(&saved);
        }
    }

    // ───────────────────────────── element trees ─────────────────────────────

    /// Paints a laid-out element tree.
    pub fn paint(&mut self, el: &El) {
        if el.collapsed || el.opacity <= 0.0 {
            return;
        }
        let r = el.rect;
        let mut opacity = el.opacity;
        let mut scale = el.scale;
        if let Some(button) = &el.button {
            if self.state.hovered_button == Some(button.key) {
                opacity *= 0.8;
            }
            scale *= self.state.press.get(&button.key).copied().unwrap_or(1.0);
        }
        let saved = if (scale - 1.0).abs() > 1e-6 {
            let c = r.center();
            Some(self.push_transform(scale_around(scale, scale, c.x, c.y)))
        } else {
            None
        };
        let grouped = opacity < 0.999;
        if grouped {
            self.push_opacity(opacity);
        }

        // A negative radius means a capsule: half the shorter side (C#'s Ui.Button sets it once it has its size).
        let radii = if el.radius[0] < 0.0 { [r.w.min(r.h) / 2.0; 4] } else { el.radius };
        if let Some(bg) = el.background
            && bg != Brush::Solid(Color::TRANSPARENT) {
                self.fill_rect(r, radii, bg);
            }
        if let Some((color, t)) = el.border {
            let inner = Rect::new(r.x + t / 2.0, r.y + t / 2.0, (r.w - t).max(0.0), (r.h - t).max(0.0));
            let radii = radii.map(|v| (v - t / 2.0).max(0.0));
            let path = rounded_rect(inner, radii);
            self.stroke(&path, color, t, false);
        }

        match &el.kind {
            Kind::Text(t) => {
                if let Some(layout) = t.layout.borrow().as_ref() {
                    self.text(layout, &t.runs, Point::new(r.x, r.y));
                }
            }
            Kind::Shape(s) => self.shape(s, r),
            Kind::Ellipse { stroke, thickness } => self.ellipse(r, None, Some((*stroke, *thickness))),
            Kind::Ring { progress, thickness, color } => {
                let path = ring(r, *progress, *thickness);
                self.stroke(&path, *color, *thickness, true);
            }
            Kind::Custom(draw) => draw(self, r),
            Kind::Scroll { .. } => unsafe {
                self.dc.PushAxisAlignedClip(&rect_f(r), D2D1_ANTIALIAS_MODE_ALIASED);
            },
            _ => {}
        }
        for c in el.children() {
            self.paint(c);
        }
        if let Kind::Scroll { .. } = el.kind {
            unsafe { self.dc.PopAxisAlignedClip() };
        }

        if grouped {
            self.pop();
        }
        if let Some(saved) = saved {
            self.restore_transform(saved);
        }
    }

    fn shape(&mut self, s: &Shape, r: Rect) {
        let path = if s.stretch {
            // Stretch.Uniform: the outline's bounds fill the box less half the pen on each side, centered.
            let b = s.path.bounds();
            let t = if s.stroke.is_some() { s.thickness } else { 0.0 };
            let room = Rect::new(r.x + t / 2.0, r.y + t / 2.0, (r.w - t).max(0.0), (r.h - t).max(0.0));
            let k = if b.w <= 0.0 && b.h <= 0.0 {
                1.0
            } else if b.w <= 0.0 {
                room.h / b.h
            } else if b.h <= 0.0 {
                room.w / b.w
            } else {
                (room.w / b.w).min(room.h / b.h)
            };
            let dx = room.x + (room.w - b.w * k) / 2.0 - b.x * k;
            let dy = room.y + (room.h - b.h * k) / 2.0 - b.y * k;
            s.path.transformed(k, dx, dy)
        } else {
            s.path.transformed(1.0, r.x, r.y)
        };
        if let Some(fill) = s.fill {
            self.fill(&path, Brush::Solid(fill));
        }
        if let Some(stroke) = s.stroke {
            self.stroke(&path, stroke, s.thickness, s.round_caps);
        }
    }
}

impl Paint for Painter<'_> {
    fn fill_rounded(&mut self, rect: Rect, radius: f64, brush: Brush) {
        self.fill_rect(rect, [radius; 4], brush);
    }
}

/// The C#'s `Icons.Ring`: an arc from 12 o'clock, clockwise, for `progress` (0..1) of the circle.
pub fn ring(r: Rect, progress: f64, stroke: f64) -> Path {
    let size = r.w.min(r.h);
    let radius = (size - stroke) / 2.0;
    let p = progress.clamp(0.0001, 0.9999);
    let angle = p * 2.0 * std::f64::consts::PI;
    let (cx, cy) = (r.x + size / 2.0, r.y + size / 2.0);
    let start = Point::new(cx, r.y + stroke / 2.0);
    let end = Point::new(cx + radius * angle.sin(), cy - radius * angle.cos());
    Path { segs: vec![Seg::Move(start), Seg::Arc(end, radius, angle > std::f64::consts::PI, true)] }
}
