//! Scrolling capture: wheel, grab, stitch, until nothing moves.
use crate::capture::{Frame, grab};
use std::time::Duration;
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

const MAX_H: i32 = 20_000;
const BINS: usize = 24;

/// Coarse brightness profile of each row.
fn profile(f: &Frame) -> Vec<[i32; BINS]> {
    (0..f.h)
        .map(|y| {
            let row = f.row(y);
            let mut s = [0i32; BINS];
            let mut n = [0i32; BINS];
            for (x, p) in row.chunks_exact(4).enumerate().step_by(2) {
                let b = x * BINS / f.w as usize;
                s[b] += p[0] as i32 + 2 * p[1] as i32 + p[2] as i32;
                n[b] += 1;
            }
            std::array::from_fn(|i| s[i] / n[i].max(1))
        })
        .collect()
}

fn diff(a: &[i32; BINS], b: &[i32; BINS]) -> i32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum()
}

const SAME: i32 = BINS as i32 * 4;

/// How far `cur` scrolled past `prev`: (header, footer, shift). None if it didn't move or can't be matched.
fn shift(prev: &Frame, cur: &Frame) -> Option<(i32, i32, i32)> {
    if prev.w != cur.w || prev.h != cur.h {
        return None;
    }
    let (a, b) = (profile(prev), profile(cur));
    let h = prev.h as usize;
    let same = |y: &usize| diff(&a[*y], &b[*y]) <= SAME;
    let head = (0..h).take_while(same).count();
    if head == h {
        return None;
    }
    let foot = (0..h).rev().take_while(same).count();
    // Plain background is identical everywhere, so only trust part of it as a fixed bar.
    let (head, foot) = (head.min(h / 3), foot.min(h / 3));
    let body = h - head - foot;
    let mut best: Option<(i64, usize)> = None;
    for d in 1..body.saturating_sub(24) {
        let rows = body - d;
        let step = (rows / 220).max(1);
        let (mut e, mut n) = (0i64, 0i64);
        for k in (0..rows).step_by(step) {
            e += diff(&a[head + d + k], &b[head + k]) as i64;
            n += 1;
        }
        let e = e * 16 / n;
        if best.is_none_or(|b| e < b.0) {
            best = Some((e, d));
        }
    }
    let (e, d) = best?;
    (e < SAME as i64 * 16 * 2).then_some((head as i32, foot as i32, d as i32))
}

pub struct Stitch {
    out: Frame,
    last: Frame,
    foot: i32,
}

impl Stitch {
    pub fn new(f: Frame) -> Stitch {
        Stitch { out: f.clone(), last: f, foot: 0 }
    }

    /// False when the page has stopped.
    pub fn push(&mut self, f: Frame) -> bool {
        let Some((_, foot, d)) = shift(&self.last, &f) else { return false };
        // Drop the old fixed footer, then append the new rows and the footer again.
        let n = (self.out.w * 4) as usize;
        let keep = (self.out.h - self.foot.max(foot)).max(0);
        // When a footer shows up late, rows above it were already kept.
        self.out.px.truncate(keep as usize * n);
        let from = f.h - self.foot.max(foot) - d;
        self.out.px.extend_from_slice(&f.px[from.max(0) as usize * n..]);
        self.out.h = (self.out.px.len() / n) as i32;
        self.foot = self.foot.max(foot);
        self.last = f;
        self.out.h < MAX_H
    }

    pub fn finish(self) -> Frame {
        self.out
    }
}

fn settle(r: RECT) -> Option<Frame> {
    // Smooth scrolling: wait until two grabs agree.
    let mut last = grab(r).ok()?;
    for _ in 0..14 {
        std::thread::sleep(Duration::from_millis(60));
        let f = grab(r).ok()?;
        if f.px == last.px {
            return Some(f);
        }
        last = f;
    }
    Some(last)
}

/// Blocking; call from a worker thread. Esc stops early and keeps what's there.
pub fn run(r: RECT) -> Option<Frame> {
    unsafe {
        let mut back = Default::default();
        let _ = GetCursorPos(&mut back);
        std::thread::sleep(Duration::from_millis(250));
        let _ = SetCursorPos((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        std::thread::sleep(Duration::from_millis(120));
        let mut st = Stitch::new(settle(r)?);
        for _ in 0..80 {
            let i = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT { mouseData: (-240i32) as u32, dwFlags: MOUSEEVENTF_WHEEL, ..Default::default() },
                },
            };
            SendInput(&[i], size_of::<INPUT>() as i32);
            std::thread::sleep(Duration::from_millis(90));
            if GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0 {
                break;
            }
            let Some(f) = settle(r) else { break };
            if !st.push(f) {
                break;
            }
        }
        let _ = SetCursorPos(back.x, back.y);
        Some(st.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(w: i32, h: i32) -> Frame {
        let mut f = Frame::new(w, h);
        let mut s = 12345u32;
        for y in 0..h {
            // text-like bands with gaps
            let band = (y / 18) % 3 != 2;
            for x in 0..w {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let v = if band && s % 3 == 0 { (s >> 8) as u8 } else { 250 };
                let i = ((y * w + x) * 4) as usize;
                f.px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        f
    }

    fn rows(f: &Frame, a: i32, b: i32) -> Frame {
        f.crop(RECT { left: 0, top: a, right: f.w, bottom: b })
    }

    #[test]
    fn stitches_exactly() {
        let p = page(300, 1500);
        let mut st = Stitch::new(rows(&p, 0, 400));
        for y in [170, 340, 510, 700, 900, 1100] {
            assert!(st.push(rows(&p, y, y + 400)), "at {y}");
        }
        assert!(!st.push(rows(&p, 1100, 1500)));
        let out = st.finish();
        assert_eq!(out.h, 1500);
        assert!(out.px == p.px);
    }

    #[test]
    fn keeps_fixed_bars_once() {
        let p = page(300, 1200);
        let bar = |v: u8| {
            let mut b = Frame::new(300, 40);
            b.px.chunks_exact_mut(4).enumerate().for_each(|(i, q)| q.copy_from_slice(&[v, (i % 251) as u8, v, 255]));
            b
        };
        let (top, bot) = (bar(30), bar(90));
        let shot = |y: i32| {
            let mut f = rows(&p, y, y + 400);
            f.paste(&top, 0, 0);
            f.paste(&bot, 0, 360);
            f
        };
        let mut st = Stitch::new(shot(0));
        for y in [150, 300, 450, 600, 800] {
            assert!(st.push(shot(y)));
        }
        let out = st.finish();
        assert_eq!(out.h, 1200);
        assert!(rows(&out, 40, 1160).px == rows(&p, 40, 1160).px);
        assert!(rows(&out, 1160, 1200).px == bot.px && rows(&out, 0, 40).px == top.px);
    }
}
