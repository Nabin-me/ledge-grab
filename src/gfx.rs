//! Direct2D on DirectComposition: per-pixel alpha windows drawn on the GPU.
use crate::capture::Frame;
use std::collections::HashMap;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

static INTER: &[u8] = include_bytes!("../assets/InterTight.ttf");

pub const BLACK: u32 = 0x000000;
pub const CARD: u32 = 0x141518;
pub const TAB: u32 = 0x1d1f23;
pub const PILL: u32 = 0x171717;
pub const PILL_HOVER: u32 = 0x262626;
pub const INK: u32 = 0xf5f6f8;
pub const DIM: u32 = 0x9398a1;
pub const RED: u32 = 0xf4505e;
pub const BLUE: u32 = 0x3b9eff;
pub const GREEN: u32 = 0x34d399;

pub fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 255) as f32 / 255.0,
        g: ((hex >> 8) & 255) as f32 / 255.0,
        b: (hex & 255) as f32 / 255.0,
        a,
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }
    pub fn has(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn grow(&self, d: f32) -> R {
        R::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    pub fn scale(&self, k: f32) -> R {
        let (w, h) = (self.w * k, self.h * k);
        R::new(self.cx() - w / 2.0, self.cy() - h / 2.0, w, h)
    }
    pub fn lerp(&self, o: &R, k: f32) -> R {
        let f = |a: f32, b: f32| a + (b - a) * k;
        R::new(f(self.x, o.x), f(self.y, o.y), f(self.w, o.w), f(self.h, o.h))
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
}

pub struct Gfx {
    pub d3d: ID3D11Device,
    factory: ID2D1Factory1,
    d2d: ID2D1Device,
    dw: IDWriteFactory5,
    fonts: IDWriteFontCollection1,
    dcomp: IDCompositionDevice,
    formats: std::cell::RefCell<HashMap<(u32, bool, i32), IDWriteTextFormat>>,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let mut dev = None;
            for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
                let r = D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut dev),
                    None,
                    None,
                );
                if r.is_ok() {
                    break;
                }
            }
            let d3d = dev.ok_or_else(windows::core::Error::from_thread)?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d = factory.CreateDevice(&dxgi)?;
            let dw: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let loader = dw.CreateInMemoryFontFileLoader()?;
            dw.RegisterFontFileLoader(&loader)?;
            let file = loader.CreateInMemoryFontFileReference(&dw, INTER.as_ptr() as *const _, INTER.len() as u32, None)?;
            let set = dw.CreateFontSetBuilder()?;
            set.AddFontFile(&file)?;
            let fonts = dw.CreateFontCollectionFromFontSet(&set.CreateFontSet()?)?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            Ok(Gfx { d3d, factory, d2d, dw, fonts, dcomp, formats: Default::default() })
        }
    }

    pub fn surface(&self, hwnd: HWND, w: i32, h: i32) -> Result<Surface> {
        unsafe {
            let fac: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let swap = fac.CreateSwapChainForComposition(
                &self.d3d,
                &DXGI_SWAP_CHAIN_DESC1 {
                    Width: w as u32,
                    Height: h as u32,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                    ..Default::default()
                },
                None,
            )?;
            let ctx = self.d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let target = Some(target(&ctx, &swap)?);
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let comp = self.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = self.dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            comp.SetRoot(&visual)?;
            self.dcomp.Commit()?;
            Ok(Surface { ctx, swap, target, _comp: comp, _visual: visual })
        }
    }

    /// Ask the driver to give back memory while idle.
    pub fn trim(&self) {
        unsafe {
            self.d2d.ClearResources(0);
            if let Ok(d) = self.d3d.cast::<IDXGIDevice3>() {
                d.Trim();
            }
        }
    }

    pub fn format(&self, size: f32, icon: bool, weight: i32) -> IDWriteTextFormat {
        let key = ((size * 4.0) as u32, icon, weight);
        if let Some(f) = self.formats.borrow().get(&key) {
            return f.clone();
        }
        unsafe {
            let col: IDWriteFontCollection = self.fonts.cast().unwrap();
            let f = self
                .dw
                .CreateTextFormat(
                    if icon { w!("Segoe Fluent Icons") } else { w!("Inter Tight") },
                    if icon { None } else { Some(&col) },
                    DWRITE_FONT_WEIGHT(weight),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
                .unwrap();
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            self.formats.borrow_mut().insert(key, f.clone());
            f
        }
    }

    pub fn measure(&self, text: &str, size: f32, weight: i32) -> f32 {
        unsafe {
            let s: Vec<u16> = text.encode_utf16().collect();
            let l = self.dw.CreateTextLayout(&s, &self.format(size, false, weight), 4000.0, 400.0).unwrap();
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }

    pub fn path(&self, build: impl FnOnce(&ID2D1GeometrySink)) -> Result<ID2D1PathGeometry1> {
        unsafe {
            let g = self.factory.CreatePathGeometry()?;
            let s = g.Open()?;
            build(&s);
            s.Close()?;
            Ok(g)
        }
    }

    /// Draw on top of a frame off-screen and read it back.
    pub fn bake(&self, f: &Frame, draw: impl FnOnce(&Canvas)) -> Result<Frame> {
        unsafe {
            let ctx = self.d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let size = D2D_SIZE_U { width: f.w as u32, height: f.h as u32 };
            let props = |o| D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: pixel_format(),
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: o,
                ..Default::default()
            };
            let target = ctx.CreateBitmap(size, None, 0, &props(D2D1_BITMAP_OPTIONS_TARGET))?;
            let read =
                ctx.CreateBitmap(size, None, 0, &props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW))?;
            ctx.SetTarget(&target);
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            ctx.BeginDraw();
            let c = Canvas { g: self, ctx: &ctx };
            let bmp = c.upload(f)?;
            c.image(&bmp, R::new(0.0, 0.0, f.w as f32, f.h as f32), 1.0);
            draw(&c);
            ctx.EndDraw(None, None)?;
            read.CopyFromBitmap(None, &target, None)?;
            let m = read.Map(D2D1_MAP_OPTIONS_READ)?;
            let mut out = Frame::new(f.w, f.h);
            let n = (f.w * 4) as usize;
            for y in 0..f.h as usize {
                std::ptr::copy_nonoverlapping(m.bits.add(y * m.pitch as usize), out.px.as_mut_ptr().add(y * n), n);
            }
            read.Unmap()?;
            out.px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            Ok(out)
        }
    }
}

fn pixel_format() -> D2D1_PIXEL_FORMAT {
    D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }
}

pub struct Surface {
    pub ctx: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    target: Option<ID2D1Bitmap1>,
    _comp: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

unsafe fn target(ctx: &ID2D1DeviceContext, swap: &IDXGISwapChain1) -> Result<ID2D1Bitmap1> {
    unsafe {
        let buf: IDXGISurface = swap.GetBuffer(0)?;
        let t = ctx.CreateBitmapFromDxgiSurface(
            &buf,
            Some(&D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: pixel_format(),
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            }),
        )?;
        ctx.SetTarget(&t);
        Ok(t)
    }
}

impl Surface {
    pub fn resize(&mut self, w: i32, h: i32) -> Result<()> {
        unsafe {
            self.ctx.SetTarget(None);
            self.target = None;
            self.swap.ResizeBuffers(0, w as u32, h as u32, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
            self.target = Some(target(&self.ctx, &self.swap)?);
        }
        Ok(())
    }

    /// Draw one frame. Err means the GPU device is gone.
    pub fn frame(&self, g: &Gfx, vsync: bool, draw: impl FnOnce(&Canvas)) -> Result<()> {
        unsafe {
            match &self.target {
                Some(t) => self.ctx.SetTarget(t),
                None => return Err(windows::Win32::Foundation::E_FAIL.into()),
            }
            self.ctx.BeginDraw();
            self.ctx.SetTransform(&Matrix3x2::identity());
            self.ctx.Clear(Some(&rgba(0, 0.0)));
            draw(&Canvas { g, ctx: &self.ctx });
            self.ctx.SetTransform(&Matrix3x2::identity());
            self.ctx.EndDraw(None, None)?;
            self.swap.Present(vsync as u32, DXGI_PRESENT(0)).ok()
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Canvas<'a> {
    pub g: &'a Gfx,
    pub ctx: &'a ID2D1DeviceContext,
}

impl Canvas<'_> {
    pub fn brush(&self, hex: u32, a: f32) -> ID2D1SolidColorBrush {
        unsafe { self.ctx.CreateSolidColorBrush(&rgba(hex, a), None).unwrap() }
    }

    pub fn transform(&self, m: &Matrix3x2) {
        unsafe { self.ctx.SetTransform(m) }
    }

    pub fn fill(&self, r: R, rad: f32, hex: u32, a: f32) {
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.ctx.FillRoundedRectangle(&rr, &self.brush(hex, a));
        }
    }

    pub fn stroke(&self, r: R, rad: f32, width: f32, hex: u32, a: f32) {
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.ctx.DrawRoundedRectangle(&rr, &self.brush(hex, a), width, None);
        }
    }

    pub fn circle(&self, x: f32, y: f32, r: f32, hex: u32, a: f32) {
        unsafe {
            let e = D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: r, radiusY: r };
            self.ctx.FillEllipse(&e, &self.brush(hex, a));
        }
    }

    pub fn ring(&self, x: f32, y: f32, r: f32, width: f32, hex: u32, a: f32) {
        unsafe {
            let e = D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: r, radiusY: r };
            self.ctx.DrawEllipse(&e, &self.brush(hex, a), width, None);
        }
    }

    pub fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, hex: u32, alpha: f32) {
        unsafe {
            self.ctx.DrawLine(
                Vector2 { X: a.0, Y: a.1 },
                Vector2 { X: b.0, Y: b.1 },
                &self.brush(hex, alpha),
                width,
                None,
            );
        }
    }

    pub fn poly(&self, pts: &[(f32, f32)], width: f32, hex: u32, a: f32, closed: bool) {
        if pts.len() < 2 {
            return;
        }
        let v = |p: (f32, f32)| Vector2 { X: p.0, Y: p.1 };
        unsafe {
            let Ok(g) = self.g.path(|s| {
                s.BeginFigure(v(pts[0]), if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW });
                for p in &pts[1..] {
                    s.AddLine(v(*p));
                }
                s.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
            }) else {
                return;
            };
            let b = self.brush(hex, a);
            if closed {
                self.ctx.FillGeometry(&g, &b, None);
            } else {
                let st = self
                    .g
                    .factory
                    .CreateStrokeStyle(
                        &D2D1_STROKE_STYLE_PROPERTIES1 {
                            startCap: D2D1_CAP_STYLE_ROUND,
                            endCap: D2D1_CAP_STYLE_ROUND,
                            lineJoin: D2D1_LINE_JOIN_ROUND,
                            miterLimit: 10.0,
                            ..Default::default()
                        },
                        None,
                    )
                    .ok();
                self.ctx.DrawGeometry(&g, &b, width, st.as_ref().map(|s| s.cast::<ID2D1StrokeStyle>().unwrap()).as_ref());
            }
        }
    }

    pub fn text_w(&self, s: &str, r: R, size: f32, weight: i32, hex: u32, a: f32, al: Align) {
        unsafe {
            let f = self.g.format(size, false, weight);
            let _ = f.SetTextAlignment(match al {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let t: Vec<u16> = s.encode_utf16().collect();
            self.ctx.DrawText(
                &t,
                &f,
                &r.d2d(),
                &self.brush(hex, a),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn text(&self, s: &str, r: R, size: f32, hex: u32, a: f32, al: Align) {
        self.text_w(s, r, size, 500, hex, a, al)
    }

    pub fn icon(&self, glyph: u16, r: R, size: f32, hex: u32, a: f32) {
        unsafe {
            let f = self.g.format(size, true, 400);
            let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            self.ctx.DrawText(
                &[glyph],
                &f,
                &r.d2d(),
                &self.brush(hex, a),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn upload(&self, f: &Frame) -> Result<ID2D1Bitmap1> {
        unsafe {
            self.ctx.CreateBitmap(
                D2D_SIZE_U { width: f.w as u32, height: f.h as u32 },
                Some(f.px.as_ptr() as *const _),
                (f.w * 4) as u32,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: pixel_format(),
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                },
            )
        }
    }

    /// A GPU texture as a bitmap, no copy.
    pub fn wrap(&self, t: &ID3D11Texture2D) -> Result<ID2D1Bitmap1> {
        unsafe {
            self.ctx.CreateBitmapFromDxgiSurface(
                &t.cast::<IDXGISurface>()?,
                Some(&D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                }),
            )
        }
    }

    pub fn image(&self, b: &ID2D1Bitmap1, r: R, a: f32) {
        unsafe {
            self.ctx.DrawBitmap(b, Some(&r.d2d()), a, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
        }
    }

    /// Part of a bitmap, pixel for pixel.
    pub fn image_part(&self, b: &ID2D1Bitmap1, dst: R, src: R, a: f32) {
        unsafe {
            self.ctx.DrawBitmap(
                b,
                Some(&dst.d2d()),
                a,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                Some(&src.d2d()),
                None,
            );
        }
    }

    /// Bitmap filling a rounded rectangle.
    pub fn image_round(&self, b: &ID2D1Bitmap1, r: R, rad: f32, a: f32) {
        unsafe {
            let s = b.GetSize();
            let Ok(br) = self.ctx.CreateBitmapBrush(
                b,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
                }),
                None,
            ) else {
                return;
            };
            br.SetOpacity(a);
            // cover
            let k = (r.w / s.width).max(r.h / s.height);
            let (ox, oy) = (r.x + (r.w - s.width * k) / 2.0, r.y + (r.h - s.height * k) / 2.0);
            br.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: ox, M32: oy });
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
            self.ctx.FillRoundedRectangle(&rr, &br);
        }
    }

    pub fn clip(&self, r: R, draw: impl FnOnce()) {
        unsafe {
            self.ctx.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            draw();
            self.ctx.PopAxisAlignedClip();
        }
    }

    /// Draw content through a blur and an opacity. Cheap when both are at rest.
    pub fn layer(&self, blur: f32, alpha: f32, draw: impl FnOnce(&Canvas)) {
        if alpha <= 0.004 {
            return;
        }
        if blur < 0.15 && alpha > 0.996 {
            return draw(self);
        }
        unsafe {
            let mut old = Matrix3x2::identity();
            self.ctx.GetTransform(&mut old);
            let target = self.ctx.GetTarget();
            let Ok(list) = self.ctx.CreateCommandList() else { return draw(self) };
            self.ctx.SetTarget(&list);
            draw(self);
            let _ = list.Close();
            if let Ok(t) = target {
                self.ctx.SetTarget(&t);
            }
            self.ctx.SetTransform(&Matrix3x2::identity());
            let (Ok(fx), Ok(op)) =
                (self.ctx.CreateEffect(&CLSID_D2D1GaussianBlur), self.ctx.CreateEffect(&CLSID_D2D1Opacity))
            else {
                self.ctx.SetTransform(&old);
                return;
            };
            fx.SetInput(0, &list, true);
            let _ = fx.SetValue(
                D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                D2D1_PROPERTY_TYPE_FLOAT,
                &blur.max(0.01).to_ne_bytes(),
            );
            if let Ok(o) = fx.GetOutput() {
                op.SetInput(0, &o, true);
            }
            let _ = op.SetValue(D2D1_OPACITY_PROP_OPACITY.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &alpha.to_ne_bytes());
            if let Ok(o) = op.GetOutput() {
                self.ctx.DrawImage(&o, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            }
            self.ctx.SetTransform(&old);
        }
    }

    /// The notch silhouette: flush with the top edge, concave ears, round bottom.
    pub fn island(&self, r: R, rad: f32, ear: f32) {
        let v = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let rad = rad.min(r.h * 0.6).min(r.w / 2.0);
        let ear = ear.min(r.h - rad);
        let (l, rt, b) = (r.x, r.x + r.w, r.y + r.h);
        let arc = |x, y, rd: f32, sweep| D2D1_ARC_SEGMENT {
            point: v(x, y),
            size: D2D_SIZE_F { width: rd, height: rd },
            rotationAngle: 0.0,
            sweepDirection: sweep,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        unsafe {
            if let Ok(g) = self.g.path(|s| {
                s.BeginFigure(v(l - ear, r.y), D2D1_FIGURE_BEGIN_FILLED);
                s.AddLine(v(rt + ear, r.y));
                s.AddArc(&arc(rt, r.y + ear, ear, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
                s.AddLine(v(rt, (b - rad).max(r.y + ear)));
                s.AddArc(&arc(rt - rad, b, rad, D2D1_SWEEP_DIRECTION_CLOCKWISE));
                s.AddLine(v(l + rad, b));
                s.AddArc(&arc(l, (b - rad).max(r.y + ear), rad, D2D1_SWEEP_DIRECTION_CLOCKWISE));
                s.AddLine(v(l, r.y + ear));
                s.AddArc(&arc(l - ear, r.y, ear, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
                s.EndFigure(D2D1_FIGURE_END_CLOSED);
            }) {
                self.ctx.FillGeometry(&g, &self.brush(BLACK, 1.0), None);
            }
        }
    }

    /// Subtle luminous contour along the lower rim of the notch (no line along the top monitor edge).
    pub fn island_rim(&self, r: R, rad: f32, ear: f32, width: f32, hex: u32, alpha: f32) {
        if alpha <= 0.004 || r.h <= 1.0 {
            return;
        }
        let v = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let inset = width * 0.5;
        let rad = (rad.min(r.h * 0.6).min(r.w / 2.0) - inset).max(0.5);
        let ear = ear.min(r.h - rad).max(0.0);
        let (l, rt, b) = (r.x + inset, r.x + r.w - inset, r.y + r.h - inset);
        let arc = |x, y, rd: f32, sweep| D2D1_ARC_SEGMENT {
            point: v(x, y),
            size: D2D_SIZE_F { width: rd, height: rd },
            rotationAngle: 0.0,
            sweepDirection: sweep,
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        unsafe {
            if let Ok(g) = self.g.path(|s| {
                s.BeginFigure(v(rt, r.y + ear), D2D1_FIGURE_BEGIN_HOLLOW);
                s.AddLine(v(rt, (b - rad).max(r.y + ear)));
                s.AddArc(&arc(rt - rad, b, rad, D2D1_SWEEP_DIRECTION_CLOCKWISE));
                s.AddLine(v(l + rad, b));
                s.AddArc(&arc(l, (b - rad).max(r.y + ear), rad, D2D1_SWEEP_DIRECTION_CLOCKWISE));
                s.AddLine(v(l, r.y + ear));
                s.EndFigure(D2D1_FIGURE_END_OPEN);
            }) {
                self.ctx.DrawGeometry(&g, &self.brush(hex, alpha), width, None);
            }
        }
    }
}

