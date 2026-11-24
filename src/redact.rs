//! Hiding pixels for good. Everything under a patch is replaced by a handful of
//! jittered averages; the original values are not recoverable from the result.
use crate::capture::{Frame, inside};
use windows::Win32::Foundation::{POINT, RECT};

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
    let r = crate::capture::intersect(&r, &RECT { left: 0, top: 0, right: f.w, bottom: f.h })?;
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

pub fn word_at(words: &[crate::ocr::Word], p: POINT) -> Option<RECT> {
    words
        .iter()
        .map(|w| {
            let pad = (w.r[3] * 0.22).max(2.0);
            RECT {
                left: (w.r[0] - pad) as i32,
                top: (w.r[1] - pad) as i32,
                right: (w.r[0] + w.r[2] + pad) as i32,
                bottom: (w.r[1] + w.r[3] + pad) as i32,
            }
        })
        .find(|r| inside(r, p))
}

#[cfg(test)]
mod tests {
    use super::*;

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
