//! Software drawing for the GNOME ledge: the same small vocabulary as the Windows
//! `gfx::Canvas` (rounded fills, circles, text, images, the island outline), drawn
//! with tiny-skia into a premultiplied RGBA pixmap that GTK shows.
use crate::frame::Frame;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use std::sync::OnceLock;
use tiny_skia::{
    Color, FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

pub const BLACK: u32 = 0x000000;
pub const CARD: u32 = 0x141518;
pub const PILL: u32 = 0x171717;
pub const PILL_HOVER: u32 = 0x262626;
pub const INK: u32 = 0xf5f6f8;
pub const DIM: u32 = 0x9398a1;
pub const RED: u32 = 0xf4505e;
pub const BLUE: u32 = 0x3b9eff;
pub const GREEN: u32 = 0x34d399;

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

fn font() -> &'static FontRef<'static> {
    static F: OnceLock<FontRef<'static>> = OnceLock::new();
    F.get_or_init(|| FontRef::try_from_slice(include_bytes!("../../assets/InterTight.ttf")).expect("bundled font"))
}

/// ab_glyph scales by ascent-to-descent height; Windows sizes text by the em.
fn em(px: f32) -> PxScale {
    let f = font();
    let k = f.units_per_em().unwrap_or(1000.0) / f.height_unscaled();
    PxScale::from(px / k)
}

fn color(hex: u32, a: f32) -> Color {
    Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn paint(hex: u32, a: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(hex, a));
    p.anti_alias = true;
    p
}

/// Cubic approximation of a quarter circle.
const K: f32 = 0.552_284_8;

fn rounded(r: R, rad: f32) -> Option<tiny_skia::Path> {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (l, t, rt, b) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let mut p = PathBuilder::new();
    if rad <= 0.01 {
        return Rect::from_xywh(r.x, r.y, r.w, r.h).map(PathBuilder::from_rect);
    }
    let c = rad * K;
    p.move_to(l + rad, t);
    p.line_to(rt - rad, t);
    p.cubic_to(rt - rad + c, t, rt, t + rad - c, rt, t + rad);
    p.line_to(rt, b - rad);
    p.cubic_to(rt, b - rad + c, rt - rad + c, b, rt - rad, b);
    p.line_to(l + rad, b);
    p.cubic_to(l + rad - c, b, l, b - rad + c, l, b - rad);
    p.line_to(l, t + rad);
    p.cubic_to(l, t + rad - c, l + rad - c, t, l + rad, t);
    p.close();
    p.finish()
}

/// An image ready to draw: premultiplied RGBA.
pub struct Image(pub Pixmap);

impl Image {
    pub fn from_frame(f: &Frame) -> Option<Image> {
        let mut pm = Pixmap::new(f.w.max(1) as u32, f.h.max(1) as u32)?;
        for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(f.px.chunks_exact(4)) {
            // Frames are opaque BGRA.
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
        Some(Image(pm))
    }
    pub fn w(&self) -> f32 {
        self.0.width() as f32
    }
    pub fn h(&self) -> f32 {
        self.0.height() as f32
    }
}

pub struct Canvas {
    pub pm: Pixmap,
    scale: f32,
}

impl Canvas {
    /// `w` x `h` logical pixels at `scale` device pixels per logical pixel.
    pub fn new(w: f32, h: f32, scale: f32) -> Option<Canvas> {
        let pm = Pixmap::new((w * scale).ceil().max(1.0) as u32, (h * scale).ceil().max(1.0) as u32)?;
        Some(Canvas { pm, scale })
    }

    fn t(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }

    pub fn clear(&mut self) {
        self.pm.fill(Color::TRANSPARENT);
    }

    pub fn fill(&mut self, r: R, rad: f32, hex: u32, a: f32) {
        if let Some(p) = rounded(r, rad) {
            self.pm.fill_path(&p, &paint(hex, a), FillRule::Winding, self.t(), None);
        }
    }

    pub fn stroke(&mut self, r: R, rad: f32, width: f32, hex: u32, a: f32) {
        let r = R::new(r.x + width / 2.0, r.y + width / 2.0, r.w - width, r.h - width);
        if let Some(p) = rounded(r, rad) {
            let s = Stroke { width, ..Stroke::default() };
            self.pm.stroke_path(&p, &paint(hex, a), &s, self.t(), None);
        }
    }

    pub fn circle(&mut self, x: f32, y: f32, r: f32, hex: u32, a: f32) {
        if let Some(p) = PathBuilder::from_circle(x, y, r) {
            self.pm.fill_path(&p, &paint(hex, a), FillRule::Winding, self.t(), None);
        }
    }

    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, hex: u32, alpha: f32) {
        let mut p = PathBuilder::new();
        p.move_to(a.0, a.1);
        p.line_to(b.0, b.1);
        if let Some(p) = p.finish() {
            let s = Stroke { width, line_cap: tiny_skia::LineCap::Round, ..Stroke::default() };
            self.pm.stroke_path(&p, &paint(hex, alpha), &s, self.t(), None);
        }
    }

    /// The ledge outline: flat top flush with the edge it hangs from, concave
    /// "ears" where it meets that edge, rounded lower corners.
    pub fn island(&mut self, r: R, rad: f32, ear: f32) {
        let rad = rad.min(r.h * 0.6).min(r.w / 2.0);
        let ear = ear.min(r.h - rad).max(0.0);
        let (l, rt, t, b) = (r.x, r.x + r.w, r.y, r.y + r.h);
        let (ce, cr) = (ear * K, rad * K);
        let mut p = PathBuilder::new();
        p.move_to(l - ear, t);
        p.line_to(rt + ear, t);
        // Right ear: concave quarter from (rt+ear, t) down to (rt, t+ear).
        p.cubic_to(rt + ear - ce, t, rt, t + ear - ce, rt, t + ear);
        p.line_to(rt, (b - rad).max(t + ear));
        p.cubic_to(rt, b - rad + cr, rt - rad + cr, b, rt - rad, b);
        p.line_to(l + rad, b);
        p.cubic_to(l + rad - cr, b, l, b - rad + cr, l, (b - rad).max(t + ear));
        p.line_to(l, t + ear);
        p.cubic_to(l, t + ear - ce, l - ear + ce, t, l - ear, t);
        p.close();
        if let Some(p) = p.finish() {
            self.pm.fill_path(&p, &paint(BLACK, 1.0), FillRule::Winding, self.t(), None);
        }
    }

    pub fn measure(&self, s: &str, size: f32) -> f32 {
        let f = font().as_scaled(em(size));
        let mut w = 0.0;
        let mut prev = None;
        for ch in s.chars() {
            let g = f.glyph_id(ch);
            if let Some(p) = prev {
                w += f.kern(p, g);
            }
            w += f.h_advance(g);
            prev = Some(g);
        }
        w
    }

    pub fn text(&mut self, s: &str, r: R, size: f32, hex: u32, a: f32, al: Align) {
        let px = size * self.scale;
        let f = font().as_scaled(em(px));
        let width = self.measure(s, size) * self.scale;
        let (rx, ry, rw, rh) = (r.x * self.scale, r.y * self.scale, r.w * self.scale, r.h * self.scale);
        let mut x = match al {
            Align::Left => rx,
            Align::Center => rx + (rw - width) / 2.0,
            Align::Right => rx + rw - width,
        };
        // Centre the cap height (about 0.73 em in Inter) in the box.
        let baseline = ry + rh / 2.0 + px * 0.36;
        let c = color(hex, 1.0);
        let (cr, cg, cb) = ((c.red() * 255.0) as u32, (c.green() * 255.0) as u32, (c.blue() * 255.0) as u32);
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let mut prev = None;
        for ch in s.chars() {
            let id = f.glyph_id(ch);
            if let Some(p) = prev {
                x += f.kern(p, id);
            }
            let glyph = id.with_scale_and_position(em(px), ab_glyph::point(x, baseline));
            if let Some(o) = font().outline_glyph(glyph) {
                let bb = o.px_bounds();
                let data = self.pm.data_mut();
                o.draw(|gx, gy, cov| {
                    let (dx, dy) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                    if dx < 0 || dy < 0 || dx >= w || dy >= h {
                        return;
                    }
                    let k = (cov * a).clamp(0.0, 1.0);
                    if k <= 0.0 {
                        return;
                    }
                    let i = ((dy * w + dx) * 4) as usize;
                    let sa = (k * 255.0) as u32;
                    let inv = 255 - sa;
                    // Premultiplied "over".
                    data[i] = ((cr * sa + data[i] as u32 * inv) / 255) as u8;
                    data[i + 1] = ((cg * sa + data[i + 1] as u32 * inv) / 255) as u8;
                    data[i + 2] = ((cb * sa + data[i + 2] as u32 * inv) / 255) as u8;
                    data[i + 3] = (sa + data[i + 3] as u32 * inv / 255).min(255) as u8;
                });
            }
            x += f.h_advance(id);
            prev = Some(id);
        }
    }

    /// Draw `img` cover-fitted into `r` with rounded corners.
    pub fn image_round(&mut self, img: &Image, r: R, rad: f32, a: f32) {
        let s = (r.w / img.w()).max(r.h / img.h());
        let (dw, dh) = (img.w() * s, img.h() * s);
        let (ox, oy) = (r.x + (r.w - dw) / 2.0, r.y + (r.h - dh) / 2.0);
        let mut mask = match Mask::new(self.pm.width(), self.pm.height()) {
            Some(m) => m,
            None => return,
        };
        if let Some(p) = rounded(r, rad) {
            mask.fill_path(&p, FillRule::Winding, true, self.t());
        }
        let paint = PixmapPaint { opacity: a.clamp(0.0, 1.0), quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
        let t = self.t().pre_translate(ox, oy).pre_scale(s, s);
        self.pm.draw_pixmap(0, 0, img.0.as_ref(), &paint, t, Some(&mask));
    }

    /// Bytes for a cairo ARGB32 surface (premultiplied BGRA, little-endian).
    pub fn cairo_bytes(&self) -> Vec<u8> {
        let mut out = self.pm.data().to_vec();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        out
    }

    pub fn device_size(&self) -> (i32, i32) {
        (self.pm.width() as i32, self.pm.height() as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_is_black_inside_and_clear_outside() {
        let mut c = Canvas::new(240.0, 40.0, 1.0).unwrap();
        c.island(R::new(20.0, 0.0, 200.0, 30.0), 12.0, 10.0);
        let at = |x: u32, y: u32| c.pm.pixel(x, y).unwrap().alpha();
        assert_eq!(at(120, 15), 255);
        assert_eq!(at(120, 35), 0);
        assert_eq!(at(2, 20), 0);
    }

    #[test]
    fn text_draws_ink_inside_its_box() {
        let mut c = Canvas::new(120.0, 30.0, 1.0).unwrap();
        c.text("Next 1 of 3", R::new(0.0, 0.0, 120.0, 30.0), 13.0, INK, 1.0, Align::Center);
        let inked = c.pm.pixels().iter().filter(|p| p.alpha() > 128).count();
        assert!(inked > 40, "only {inked} pixels inked");
        assert!(c.measure("Next 1 of 3", 13.0) > 50.0);
    }
}
