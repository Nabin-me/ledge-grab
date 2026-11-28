//! Platform-neutral pixels and rectangles shared by capture, the shelf and PDF code.
use crate::geom::{POINT, RECT};

/// Top-down BGRA pixels, tightly packed.
#[derive(Clone)]
pub struct Frame {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

impl Frame {
    pub fn new(w: i32, h: i32) -> Self {
        Frame { w, h, px: vec![0; (w.max(0) as usize) * (h.max(0) as usize) * 4] }
    }

    pub fn crop(&self, r: RECT) -> Frame {
        let (l, t) = (r.left.clamp(0, self.w), r.top.clamp(0, self.h));
        let (rr, b) = (r.right.clamp(l, self.w), r.bottom.clamp(t, self.h));
        let mut out = Frame::new(rr - l, b - t);
        let n = (out.w * 4) as usize;
        for y in 0..out.h {
            let s = (((t + y) * self.w + l) * 4) as usize;
            let d = (y * out.w * 4) as usize;
            out.px[d..d + n].copy_from_slice(&self.px[s..s + n]);
        }
        out
    }

    /// Paste `src` with its top-left at (x, y), clipped.
    pub fn paste(&mut self, src: &Frame, x: i32, y: i32) {
        let x0 = x.max(0);
        let x1 = (x + src.w).min(self.w);
        if x1 <= x0 {
            return;
        }
        let n = ((x1 - x0) * 4) as usize;
        for sy in 0..src.h {
            let dy = y + sy;
            if dy < 0 || dy >= self.h {
                continue;
            }
            let s = ((sy * src.w + (x0 - x)) * 4) as usize;
            let d = ((dy * self.w + x0) * 4) as usize;
            self.px[d..d + n].copy_from_slice(&src.px[s..s + n]);
        }
    }

    pub fn row(&self, y: i32) -> &[u8] {
        let n = (self.w * 4) as usize;
        &self.px[y as usize * n..(y as usize + 1) * n]
    }

    /// Area-averaged cover-crop to exactly w x h.
    pub fn thumb(&self, w: i32, h: i32) -> Frame {
        let mut out = Frame::new(w, h);
        if self.w < 1 || self.h < 1 {
            return out;
        }
        let scale = (w as f64 / self.w as f64).max(h as f64 / self.h as f64);
        let (sw, sh) = (w as f64 / scale, h as f64 / scale);
        let x0 = (self.w as f64 - sw) / 2.0;
        // Tall shots keep their top: that's the part you recognise.
        let y0 = if self.h > self.w { 0.0 } else { (self.h as f64 - sh) / 2.0 };
        for y in 0..h {
            let a = ((y0 + y as f64 / scale) as i32).clamp(0, self.h - 1);
            let b = ((y0 + (y + 1) as f64 / scale).ceil() as i32).clamp(a + 1, self.h);
            let sy = (((b - a) / 4).max(1)) as usize;
            for x in 0..w {
                let l = ((x0 + x as f64 / scale) as i32).clamp(0, self.w - 1);
                let r = ((x0 + (x + 1) as f64 / scale).ceil() as i32).clamp(l + 1, self.w);
                let sx = (((r - l) / 4).max(1)) as usize;
                let (mut c, mut n) = ([0u32; 3], 0u32);
                for yy in (a..b).step_by(sy) {
                    for xx in (l..r).step_by(sx) {
                        let i = ((yy * self.w + xx) * 4) as usize;
                        c[0] += self.px[i] as u32;
                        c[1] += self.px[i + 1] as u32;
                        c[2] += self.px[i + 2] as u32;
                        n += 1;
                    }
                }
                let d = ((y * w + x) * 4) as usize;
                for k in 0..3 {
                    out.px[d + k] = (c[k] / n) as u8;
                }
                out.px[d + 3] = 255;
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub handle: isize,
    pub rect: RECT,
    pub primary: bool,
    pub dpi: u32,
}

impl Monitor {
    pub fn w(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn h(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
    pub fn has(&self, p: POINT) -> bool {
        inside(&self.rect, p)
    }
}

pub fn inside(r: &RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

pub fn intersect(a: &RECT, b: &RECT) -> Option<RECT> {
    let r = RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}
