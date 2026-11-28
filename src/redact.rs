//! Hiding pixels for good. Everything under a patch is replaced by a handful of
//! jittered averages; the original values are not recoverable from the result.
use crate::frame::{Frame, inside};
use crate::geom::{POINT, RECT};

fn rng(s: &mut u32) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s
}

fn seed() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    // Random per call, so the jitter can't be replayed and subtracted.
    let s = std::collections::hash_map::RandomState::new().build_hasher().finish() as u32;
    s | 1
}

/// Overwrite `r` (frame pixels). Returns what was there, for undo.
pub fn apply(f: &mut Frame, r: RECT, smooth: bool) -> Option<(RECT, Frame)> {
    let r = crate::frame::intersect(&r, &RECT { left: 0, top: 0, right: f.w, bottom: f.h })?;
    let before = f.crop(r);
    let (w, h) = (before.w, before.h);
    // At most three blocks over a line of text: too coarse to hold a letter.
    let cell = (h / 3).clamp(14, 36);
    let (cw, ch) = ((w + cell - 1) / cell, (h + cell - 1) / cell);
    let mut s = seed();
    let mut grid = Vec::with_capacity((cw * ch) as usize);
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut c, mut n) = ([0u64; 3], 0u64);
            for y in cy * cell..((cy + 1) * cell).min(h) {
                for x in cx * cell..((cx + 1) * cell).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    for k in 0..3 {
                        c[k] += before.px[i + k] as u64;
                    }
                    n += 1;
                }
            }
            let j = (rng(&mut s) % 37) as i32 - 18;
            grid.push(c.map(|v| ((v / n.max(1)) as i32 + j).clamp(0, 255) as f32));
        }
    }
    let at = |x: i32, y: i32| grid[(y.clamp(0, ch - 1) * cw + x.clamp(0, cw - 1)) as usize];
    let mut out = Frame::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = if smooth {
                let (fx, fy) = (x as f32 / cell as f32 - 0.5, y as f32 / cell as f32 - 0.5);
                let (ix, iy) = (fx.floor(), fy.floor());
                let ease = |t: f32| t * t * (3.0 - 2.0 * t);
                let (tx, ty) = (ease(fx - ix), ease(fy - iy));
                let (ix, iy) = (ix as i32, iy as i32);
                let (a, b, c, d) = (at(ix, iy), at(ix + 1, iy), at(ix, iy + 1), at(ix + 1, iy + 1));
                std::array::from_fn(|k| {
                    let top = a[k] + (b[k] - a[k]) * tx;
                    let bot = c[k] + (d[k] - c[k]) * tx;
                    top + (bot - top) * ty
                })
            } else {
                at(x / cell, y / cell)
            };
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                out.px[i + k] = v[k] as u8;
            }
            out.px[i + 3] = 255;
        }
    }
    f.paste(&out, r.left, r.top);
    Some((r, before))
}

/// Paint a brushed area with the background around it. `seed` is the color to match:
/// where the stroke began. Flat colors and even gradients come out seamless.
pub fn erase(f: &mut Frame, stroke: &[POINT], rad: f32) -> Option<(RECT, Frame)> {
    let first = *stroke.first()?;
    let pad = rad.ceil() as i32 + 4;
    let b = stroke.iter().fold(RECT { left: first.x, top: first.y, right: first.x, bottom: first.y }, |a, p| RECT {
        left: a.left.min(p.x),
        top: a.top.min(p.y),
        right: a.right.max(p.x),
        bottom: a.bottom.max(p.y),
    });
    let b = RECT { left: b.left - pad, top: b.top - pad, right: b.right + pad, bottom: b.bottom + pad };
    let dist = |x: f32, y: f32| {
        let seg = |a: POINT, b: POINT| {
            let (ax, ay, dx, dy) = (a.x as f32, a.y as f32, (b.x - a.x) as f32, (b.y - a.y) as f32);
            let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
            (x - ax - dx * t).hypot(y - ay - dy * t)
        };
        stroke.windows(2).map(|w| seg(w[0], w[1])).fold(seg(first, first), f32::min)
    };
    let seed = (inside(&RECT { left: 0, top: 0, right: f.w, bottom: f.h }, first)).then(|| at(f, first.x, first.y));
    fill(f, b, |x, y| dist(x as f32, y as f32) - rad, seed)
}

/// Erase a rectangle (a word).
pub fn erase_rect(f: &mut Frame, r: RECT) -> Option<(RECT, Frame)> {
    let b = RECT { left: r.left - 4, top: r.top - 4, right: r.right + 4, bottom: r.bottom + 4 };
    let d = |x: i32, y: i32| {
        (r.left - x).max(x - r.right + 1).max(r.top - y).max(y - r.bottom + 1) as f32
    };
    fill(f, b, d, None)
}

fn at(f: &Frame, x: i32, y: i32) -> [f32; 3] {
    let i = ((y * f.w + x) * 4) as usize;
    [f.px[i] as f32, f.px[i + 1] as f32, f.px[i + 2] as f32]
}

/// Solve a 3x3 least squares plane: color = c0 + c1 x + c2 y.
fn plane(v: &[(f32, f32, f32)]) -> Option<[f32; 3]> {
    let mut m = [[0f64; 4]; 3];
    for &(x, y, z) in v {
        let p = [1.0, x as f64, y as f64];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] += p[r] * p[c];
            }
            m[r][3] += p[r] * z as f64;
        }
    }
    // Keeps thin rings (all samples on one line) solvable.
    for (r, row) in m.iter_mut().enumerate().skip(1) {
        row[r] += 1e-3;
    }
    for i in 0..3 {
        let p = (i..3).max_by(|a, b| m[*a][i].abs().total_cmp(&m[*b][i].abs()))?;
        m.swap(i, p);
        if m[i][i].abs() < 1e-9 {
            return None;
        }
        for r in 0..3 {
            if r != i {
                let k = m[r][i] / m[i][i];
                for c in i..4 {
                    m[r][c] -= k * m[i][c];
                }
            }
        }
    }
    Some(std::array::from_fn(|i| (m[i][3] / m[i][i]) as f32))
}

/// `depth` is below zero inside the area, and the distance to it outside.
fn fill(f: &mut Frame, b: RECT, depth: impl Fn(i32, i32) -> f32, seed: Option<[f32; 3]>) -> Option<(RECT, Frame)> {
    let b = crate::frame::intersect(&b, &RECT { left: 0, top: 0, right: f.w, bottom: f.h })?;
    let mut ring: Vec<(f32, f32, [f32; 3])> = Vec::new();
    let mut hole = Vec::new();
    for y in b.top..b.bottom {
        for x in b.left..b.right {
            let d = depth(x, y);
            if d <= 0.0 {
                hole.push((x, y));
            } else if d > 1.0 && d <= 3.5 {
                ring.push(((x - b.left) as f32, (y - b.top) as f32, at(f, x, y)));
            }
        }
    }
    if hole.is_empty() {
        return None;
    }
    let seed = seed.or_else(|| {
        // Most common color nearby.
        let mut n: std::collections::HashMap<[u8; 3], (u32, [f32; 3])> = Default::default();
        for (_, _, c) in &ring {
            n.entry(c.map(|v| v as u8 >> 3)).or_insert((0, *c)).0 += 1;
        }
        n.into_values().max_by_key(|v| v.0).map(|v| v.1)
    })?;
    // Grow a gradient out of the seed color, ignoring whatever else the ring touches.
    let mut model = seed.map(|v| [v, 0.0, 0.0]);
    for tol in [26.0, 18.0, 14.0, 12.0, 12.0, 12.0] {
        let near: Vec<_> = ring
            .iter()
            .filter(|(x, y, c)| (0..3).all(|k| (model[k][0] + model[k][1] * x + model[k][2] * y - c[k]).abs() < tol))
            .collect();
        if near.len() < 12 {
            break;
        }
        for (k, m) in model.iter_mut().enumerate() {
            let v: Vec<_> = near.iter().map(|(x, y, c)| (*x, *y, c[k])).collect();
            if let Some(p) = plane(&v) {
                *m = p;
            }
        }
    }
    let before = f.crop(b);
    for (x, y) in hole {
        let i = ((y * f.w + x) * 4) as usize;
        let (dx, dy) = ((x - b.left) as f32, (y - b.top) as f32);
        for (k, m) in model.iter().enumerate() {
            f.px[i + k] = (m[0] + m[1] * dx + m[2] * dy).round().clamp(0.0, 255.0) as u8;
        }
    }
    Some((b, before))
}

/// A text box with a little air around it.
pub fn padded(r: [f32; 4]) -> RECT {
    let pad = (r[3] * 0.22).max(2.0);
    RECT {
        left: (r[0] - pad) as i32,
        top: (r[1] - pad) as i32,
        right: (r[0] + r[2] + pad) as i32,
        bottom: (r[1] + r[3] + pad) as i32,
    }
}

pub fn word_at(words: &[crate::ocr::Word], p: POINT) -> Option<RECT> {
    words.iter().map(|w| padded(w.r)).find(|r| inside(r, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(bg: impl Fn(i32, i32) -> [u8; 3]) -> (Frame, Frame) {
        let mut clean = Frame::new(240, 80);
        for y in 0..80 {
            for x in 0..240 {
                let c = bg(x, y);
                let i = ((y * 240 + x) * 4) as usize;
                clean.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let mut inked = clean.clone();
        for y in 34..46 {
            for x in (60..180).filter(|x| x / 4 % 2 == 0) {
                let i = ((y * 240 + x) * 4) as usize;
                inked.px[i..i + 3].copy_from_slice(&[10, 10, 200]);
            }
        }
        (clean, inked)
    }

    fn off(a: &Frame, b: &Frame) -> u8 {
        a.px.iter().zip(&b.px).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
    }

    #[test]
    fn erases_seamlessly() {
        let flat = |_, _| [240, 236, 230];
        let grad = |x: i32, y: i32| [(40 + x / 2) as u8, (90 + y) as u8, (200 - x / 3) as u8];
        let line = [POINT { x: 45, y: 40 }, POINT { x: 195, y: 40 }];
        for k in 0..2 {
            let (clean, mut f) = if k == 0 { scene(flat) } else { scene(grad) };
            let inked = f.clone();
            let (r, before) = erase(&mut f, &line, 12.0).unwrap();
            assert!(off(&f, &clean) <= 1, "brush {k}: {}", off(&f, &clean));
            f.paste(&before, r.left, r.top);
            assert_eq!(f.px, inked.px);
            erase_rect(&mut f, RECT { left: 56, top: 30, right: 184, bottom: 50 }).unwrap();
            assert!(off(&f, &clean) <= 1, "word {k}: {}", off(&f, &clean));
        }
    }

    #[test]
    fn destroys_and_undoes() {
        let mut f = Frame::new(200, 60);
        f.px.chunks_exact_mut(4).enumerate().for_each(|(i, p)| {
            let v = if (i / 3) % 2 == 0 { 0 } else { 255 };
            p.copy_from_slice(&[v, v, v, 255]);
        });
        let orig = f.clone();
        for smooth in [true, false] {
            let r = RECT { left: 20, top: 10, right: 180, bottom: 40 };
            let (rr, before) = apply(&mut f, r, smooth).unwrap();
            // No pixel contrast left inside: neighbours are averages.
            let inner = f.crop(rr);
            let extremes = inner.px.chunks_exact(4).filter(|p| p[0] < 40 || p[0] > 215).count();
            assert_eq!(extremes, 0);
            // Nothing outside moved.
            assert_eq!(f.row(5), orig.row(5));
            f.paste(&before, rr.left, rr.top);
            assert_eq!(f.px, orig.px);
        }
    }
}
