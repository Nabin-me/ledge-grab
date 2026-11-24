//! A shot on a soft backdrop taken from its own colors.
use crate::capture::Frame;
use crate::gfx::*;
use windows::core::Result;

fn hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let f = |n: f32| {
        let k = (n + h / 30.0) % 12.0;
        l - s * l.min(1.0 - l) * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn hue(p: &[u8]) -> Option<(f32, f32)> {
    let [b, g, r] = [p[0], p[1], p[2]].map(|v| v as f32 / 255.0);
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let d = hi - lo;
    if d < 0.12 {
        return None;
    }
    let h = if hi == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if hi == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Some((h * 60.0, d))
}

/// The two ends of the backdrop.
fn colors(f: &Frame) -> ([f32; 3], [f32; 3]) {
    let mut bins = [0f32; 24];
    let (mut light, mut n) = (0f32, 0f32);
    let step = ((f.w * f.h) as usize / 40_000).max(1);
    for p in f.px.chunks_exact(4).step_by(step) {
        light += (p[0] as f32 * 0.07 + p[1] as f32 * 0.72 + p[2] as f32 * 0.21) / 255.0;
        n += 1.0;
        if let Some((h, d)) = hue(p) {
            bins[(h / 15.0) as usize % 24] += d;
        }
    }
    let light = light / n.max(1.0);
    let total: f32 = bins.iter().sum();
    let best = (0..24).max_by(|a, b| bins[*a].total_cmp(&bins[*b])).unwrap_or(0);
    // Mostly grey: dusk.
    let (h1, h2) = if total < n * 0.004 { (222.0, 24.0) } else { (best as f32 * 15.0 + 7.0, best as f32 * 15.0 + 47.0) };
    let l = if light > 0.6 { (0.46, 0.64) } else { (0.40, 0.58) };
    (hsl(h1 % 360.0, 0.52, l.0), hsl(h2 % 360.0, 0.58, l.1))
}

pub fn make(g: &Gfx, f: &Frame) -> Result<Frame> {
    let long = f.w.max(f.h);
    let pad = (long / 11).clamp(36, 220);
    let (w, h) = (f.w + pad * 2, f.h + pad * 2);
    let (a, b) = colors(f);
    let mut out = Frame::new(w, h);
    let mut s = 0x9e37_79b9u32;
    for y in 0..h {
        for x in 0..w {
            let t = (x as f32 / w as f32 * 0.55 + y as f32 / h as f32 * 0.45).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            // A little noise so the gradient never bands.
            let j = (s & 255) as f32 / 255.0 - 0.5;
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                out.px[i + 2 - k] = ((a[k] + (b[k] - a[k]) * t) * 255.0 + j + 0.5).clamp(0.0, 255.0) as u8;
            }
            out.px[i + 3] = 255;
        }
    }
    let p = pad as f32;
    let r = R::new(p, p, f.w as f32, f.h as f32);
    let rad = (f.w.min(f.h) as f32 * 0.025).clamp(6.0, 22.0);
    g.bake(&out, |c| {
        c.layer(p * 0.32, 0.5, |c| c.fill(R { y: r.y + p * 0.16, ..r }.grow(-p * 0.04), rad, BLACK, 1.0));
        c.layer(p * 0.06, 0.28, |c| c.fill(R { y: r.y + p * 0.03, ..r }, rad, BLACK, 1.0));
        if let Ok(b) = c.upload(f) {
            c.image_round(&b, r, rad, 1.0);
        }
        c.stroke(r.grow(-0.5), rad, 1.0, INK, 0.14);
    })
}
