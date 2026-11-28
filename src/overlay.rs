//! The frozen screen: pick a region, hide things, draw on it.
use crate::anim::{Anim, Curve};
use crate::capture::*;
use crate::gfx::*;
use crate::ocr::Word;
use crate::{App, redact};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_U;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Region,
    Window,
    Screen,
    All,
    Last,
    Scroll,
    ScrollPdf,
    Delay,
    Interval,
    Text,
    Color,
    Clip,
    Float,
}

impl Mode {
    pub const ALL: [Mode; 13] = [
        Mode::Region,
        Mode::Window,
        Mode::Screen,
        Mode::All,
        Mode::Last,
        Mode::Scroll,
        Mode::Delay,
        Mode::Interval,
        Mode::Text,
        Mode::Color,
        Mode::Clip,
        Mode::Float,
        Mode::ScrollPdf,
    ];
    pub fn index(self) -> usize {
        Mode::ALL.iter().position(|m| *m == self).unwrap()
    }
    fn draws(self) -> bool {
        matches!(self, Mode::Region | Mode::Window | Mode::Float)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tool {
    Region,
    Blur,
    Pixelate,
    Erase,
    Box,
    Arrow,
    Pen,
    Mark,
    Text,
    Step,
}

impl Tool {
    /// Tools that take pixels away, and can aim at a word.
    fn hides(self) -> bool {
        matches!(self, Tool::Blur | Tool::Pixelate | Tool::Erase)
    }
}

const TOOLS: [(Tool, u16); 10] = [
    (Tool::Region, b'R' as u16),
    (Tool::Blur, b'B' as u16),
    (Tool::Pixelate, b'P' as u16),
    (Tool::Erase, b'E' as u16),
    (Tool::Box, b'X' as u16),
    (Tool::Arrow, b'A' as u16),
    (Tool::Pen, b'D' as u16),
    (Tool::Mark, b'H' as u16),
    (Tool::Text, b'T' as u16),
    (Tool::Step, b'N' as u16),
];
const COLORS: [u32; 6] = [0xf4505e, 0xf5a524, 0x34d399, 0x3b9eff, 0xf5f6f8, 0x0b0b0c];

#[derive(Clone)]
enum Shape {
    Box,
    Arrow,
    Pen(Vec<POINT>),
    Mark(Vec<POINT>),
    Text(String),
    Step(usize),
    /// Brushing something away; pixels change when it ends.
    Wipe(Vec<POINT>),
    /// Pixels already overwritten; what they were, per monitor, for undo only.
    Patch(Vec<(usize, RECT, Frame)>),
}

#[derive(Clone)]
struct Anno {
    shape: Shape,
    a: POINT,
    b: POINT,
    color: u32,
    width: f32,
    s: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Bar {
    Tool(Tool),
    Secrets,
    Color(usize),
    Undo,
    Cancel,
}

struct Pane {
    hwnd: HWND,
    surf: Surface,
    bmp: Option<ID2D1Bitmap1>,
    s: f32,
    dirty: bool,
    words: Arc<Mutex<Option<Vec<Word>>>>,
    /// Emails, keys and the like, in pane pixels.
    secrets: Arc<Mutex<Vec<RECT>>>,
    /// Frame rows changed since the last upload.
    stale: Option<RECT>,
}

struct Fly {
    from: RECT,
    to: RECT,
    frame: Frame,
    bmp: Vec<Option<ID2D1Bitmap1>>,
    at: Anim,
}

pub struct Overlay {
    pub mode: Mode,
    mons: Vec<Monitor>,
    frames: Vec<Frame>,
    panes: Vec<Pane>,
    wins: Vec<Win>,
    prev: HWND,
    home: usize,
    cur: POINT,
    start: Option<POINT>,
    tool: Tool,
    color: usize,
    width: f32,
    annos: Vec<Anno>,
    live: Option<Anno>,
    typing: bool,
    bar_hover: Option<Bar>,
    bar_y: Anim,
    dim: Anim,
    was_animating: bool,
    fly: Option<Fly>,
    ocr_started: bool,
    pub opened_ms: f64,
}

pub enum Outcome {
    None,
    Cancel,
    Shot(RECT),
    Color(u32),
}

fn rect(a: POINT, b: POINT) -> RECT {
    RECT { left: a.x.min(b.x), top: a.y.min(b.y), right: a.x.max(b.x), bottom: a.y.max(b.y) }
}

fn wh(r: &RECT) -> (i32, i32) {
    (r.right - r.left, r.bottom - r.top)
}

fn covers(a: &RECT, b: &RECT) -> bool {
    let p = POINT { x: (b.left + b.right) / 2, y: (b.top + b.bottom) / 2 };
    inside(a, p)
}

impl Overlay {
    pub fn open(g: &Gfx, inst: HINSTANCE, mode: Mode) -> Result<Overlay> {
        let t0 = Instant::now();
        let mons = monitors();
        let cur = cursor();
        let home = mons.iter().position(|m| m.has(cur)).unwrap_or(0);
        // Freeze first. What our GPU can't reach is grabbed off this thread while the windows get made.
        let mut gpu: Vec<Option<GpuFrame>> = mons.iter().map(|m| GpuFrame::take(&g.d3d, m)).collect();
        let frozen = t0.elapsed();
        let jobs: Vec<_> = mons
            .iter()
            .zip(&gpu)
            .map(|(m, f)| {
                let r = m.rect;
                f.is_none().then(|| {
                    std::thread::spawn(move || grab(r).unwrap_or_else(|_| Frame::new(r.right - r.left, r.bottom - r.top)))
                })
            })
            .collect();
        let wins = windows();
        let prev = unsafe { GetForegroundWindow() };
        let class = w!("LedgeGrabOverlay");
        unsafe {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(crate::wndproc),
                hInstance: inst,
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_CROSS)?,
                ..Default::default()
            });
        }
        let mut panes = Vec::new();
        for m in &mons {
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                    class,
                    w!("Ledge Grab overlay"),
                    WS_POPUP,
                    m.rect.left,
                    m.rect.top,
                    m.w(),
                    m.h(),
                    None,
                    None,
                    Some(inst),
                    None,
                )?
            };
            panes.push(Pane {
                hwnd,
                surf: g.surface(hwnd, m.w(), m.h())?,
                bmp: None,
                s: m.dpi as f32 / 96.0,
                dirty: true,
                words: Default::default(),
                secrets: Default::default(),
                stale: None,
            });
        }
        let frames = mons.iter().map(|_| Frame::new(0, 0)).collect();
        let mut o = Overlay {
            mode,
            mons,
            frames,
            panes,
            wins,
            prev,
            home,
            cur,
            start: None,
            tool: Tool::Region,
            color: 0,
            width: 4.0,
            annos: Vec::new(),
            live: None,
            typing: false,
            bar_hover: None,
            bar_y: Anim::new(-70.0),
            dim: Anim::new(0.0),
            was_animating: false,
            fly: None,
            ocr_started: false,
            opened_ms: 0.0,
        };
        for (p, f) in o.panes.iter_mut().zip(&gpu) {
            if let Some(f) = f {
                let mut b = None;
                p.surf.frame(g, false, |c| b = c.wrap(&f.tex).ok())?;
                p.bmp = b;
            }
        }
        // The monitor under the cursor never waits for a slower one.
        let mut jobs: Vec<_> = jobs.into_iter().enumerate().filter_map(|(n, j)| Some((n, j?))).collect();
        if let Some(k) = jobs.iter().position(|j| j.0 == home) {
            let (n, j) = jobs.remove(k);
            o.frames[n] = j.join().unwrap();
        }
        let ready = t0.elapsed();
        o.draw(g, false)?;
        o.show();
        o.opened_ms = t0.elapsed().as_secs_f64() * 1e3;
        unsafe {
            let h = o.panes[o.home].hwnd;
            let _ = SetForegroundWindow(h);
            let _ = SetFocus(Some(h));
        }
        if !jobs.is_empty() {
            for (n, j) in jobs {
                o.frames[n] = j.join().unwrap();
            }
            o.draw(g, false)?;
            o.show();
        }
        // On screen now; the pixels themselves are only needed from the first click on.
        for (n, f) in gpu.iter_mut().enumerate() {
            if let Some(f) = f.take() {
                o.frames[n] = f.read()?;
            }
        }
        crate::note(&format!(
            "  frozen {:.1} ready {:.1} shown {:.1} read {:.1}",
            frozen.as_secs_f64() * 1e3,
            ready.as_secs_f64() * 1e3,
            o.opened_ms,
            t0.elapsed().as_secs_f64() * 1e3
        ));
        o.dim.go(1.0, 0.16, Curve::Standard);
        if mode.draws() {
            o.bar_y.go(0.0, 0.42, Curve::Emphasized);
        }
        Ok(o)
    }

    fn show(&self) {
        for p in self.panes.iter().filter(|p| p.bmp.is_some()) {
            unsafe {
                let _ = SetWindowPos(p.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE);
            }
        }
    }

    pub fn owns(&self, h: HWND) -> Option<usize> {
        self.panes.iter().position(|p| p.hwnd == h)
    }

    pub fn flying(&self) -> bool {
        self.fly.is_some()
    }

    pub fn close(&mut self) {
        unsafe {
            for p in &self.panes {
                let _ = DestroyWindow(p.hwnd);
            }
        }
        self.panes.clear();
    }

    pub fn previous_window(&self) -> isize {
        self.prev.0 as isize
    }

    pub fn refocus(&self) {
        unsafe {
            if IsWindow(Some(self.prev)).as_bool() {
                let _ = SetForegroundWindow(self.prev);
            }
        }
    }

    fn touch(&mut self) {
        self.panes.iter_mut().for_each(|p| p.dirty = true);
    }

    // ── Result ─────────────────────────────────────────────────────────────

    pub fn app(&self, r: RECT) -> String {
        app_at(&self.wins, r)
    }

    /// Final pixels of a desktop rectangle, drawings included.
    pub fn take(&mut self, g: &Gfx, r: RECT) -> Frame {
        self.commit_live();
        let f = compose(&self.mons, &self.frames, r);
        let drawn = self.annos.iter().any(|a| !matches!(a.shape, Shape::Patch(_)));
        if !drawn {
            return f;
        }
        let annos = &self.annos;
        g.bake(&f, |c| {
            for a in annos {
                paint(c, a, r.left as f32, r.top as f32, 1.0);
            }
        })
        .unwrap_or(f)
    }

    /// Drop the frozen screen and send the shot to the notch.
    pub fn launch(&mut self, frame: Frame, from: RECT, to: RECT) {
        self.frames.clear();
        unsafe {
            for p in &mut self.panes {
                p.bmp = None;
                let ex = GetWindowLongW(p.hwnd, GWL_EXSTYLE);
                SetWindowLongW(p.hwnd, GWL_EXSTYLE, ex | (WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE).0 as i32);
            }
        }
        self.refocus();
        let mut at = Anim::new(0.0);
        at.go(1.0, 0.46, Curve::Emphasized);
        // Full size is never needed: it only shrinks.
        self.fly = Some(Fly { from, to, frame, bmp: self.panes.iter().map(|_| None).collect(), at });
        self.touch();
    }

    /// True once the flight is over.
    pub fn landed(&self, now: Instant) -> bool {
        self.fly.as_ref().is_some_and(|f| f.at.progress(now) >= 0.82)
    }

    // ── Drawing ────────────────────────────────────────────────────────────

    pub fn busy(&self, now: Instant) -> bool {
        self.fly.is_some() || self.was_animating || self.dim.busy(now) || self.bar_y.busy(now)
            || self.panes.iter().any(|p| p.dirty)
    }

    pub fn draw(&mut self, g: &Gfx, vsync: bool) -> Result<()> {
        self.draw_at(g, Instant::now(), vsync)
    }

    pub fn draw_at(&mut self, g: &Gfx, now: Instant, vsync: bool) -> Result<()> {
        let moving = self.dim.busy(now) || self.bar_y.busy(now);
        let anim = moving || self.was_animating;
        self.was_animating = moving;
        if let Some(mut fly) = self.fly.take() {
            let t = fly.at.progress(now);
            let k = fly.at.get(now);
            for (n, p) in self.panes.iter().enumerate() {
                let m = self.mons[n].rect;
                let off = |r: &RECT| {
                    R::new(
                        (r.left - m.left) as f32,
                        (r.top - m.top) as f32,
                        (r.right - r.left) as f32,
                        (r.bottom - r.top) as f32,
                    )
                };
                let r = off(&fly.from).lerp(&off(&fly.to), k);
                let a = 1.0 - ((t - 0.66) / 0.2).clamp(0.0, 1.0);
                let mut made = None;
                let have = fly.bmp[n].clone();
                let frame = &fly.frame;
                p.surf.frame(g, vsync && n == self.home, |c| {
                    let b = have.or_else(|| {
                        made = c.upload(frame).ok();
                        made.clone()
                    });
                    if let Some(b) = b {
                        c.image_round(&b, r, 12.0 * p.s * k, a);
                    }
                })?;
                if made.is_some() {
                    fly.bmp[n] = made;
                }
            }
            self.fly = Some(fly);
            return Ok(());
        }
        for n in 0..self.panes.len() {
            let waiting = self.panes[n].bmp.is_none() && self.frames[n].w == 0;
            if waiting || !self.panes[n].dirty && !anim {
                continue;
            }
            self.panes[n].dirty = false;
            let stale = self.panes[n].stale.take();
            let mut bmp = self.panes[n].bmp.take();
            let res = {
                let me = &*self;
                let p = &me.panes[n];
                p.surf.frame(g, vsync && n == me.home, |c| {
                    let f = &me.frames[n];
                    if let (Some(b), Some(r)) = (&bmp, stale) {
                        let d = D2D_RECT_U {
                            left: r.left as u32,
                            top: r.top as u32,
                            right: r.right as u32,
                            bottom: r.bottom as u32,
                        };
                        let at = ((r.top * f.w + r.left) * 4) as usize;
                        unsafe {
                            let _ = b.CopyFromMemory(Some(&d), f.px[at..].as_ptr() as *const _, (f.w * 4) as u32);
                        }
                    }
                    if bmp.is_none() {
                        bmp = c.upload(f).ok();
                    }
                    if let Some(b) = &bmp {
                        me.pane(c, n, b, now);
                    }
                })
            };
            self.panes[n].bmp = bmp;
            res?;
        }
        Ok(())
    }

    fn selection(&self) -> Option<RECT> {
        if self.mode == Mode::Color || self.tool != Tool::Region {
            return None;
        }
        match self.start {
            Some(s) if self.mode != Mode::Window => Some(rect(s, self.cur)),
            _ => self.snap(),
        }
    }

    /// Window (or whole monitor) under the cursor.
    fn snap(&self) -> Option<RECT> {
        let desk = desktop(&self.mons);
        self.wins
            .iter()
            .find(|w| inside(&w.rect, self.cur))
            .and_then(|w| intersect(&w.rect, &desk))
            .or_else(|| self.mons.iter().find(|m| m.has(self.cur)).map(|m| m.rect))
    }

    fn pane(&self, c: &Canvas, n: usize, bmp: &ID2D1Bitmap1, now: Instant) {
        let m = &self.mons[n];
        let s = self.panes[n].s;
        let (mx, my) = (m.rect.left as f32, m.rect.top as f32);
        let full = R::new(0.0, 0.0, m.w() as f32, m.h() as f32);
        c.image_part(bmp, full, full, 1.0);
        for a in self.annos.iter().chain(&self.live) {
            paint(c, a, mx, my, 1.0);
        }
        if let Some(a) = &self.live {
            if let Shape::Text(t) = &a.shape {
                let size = a.width * 5.5 * a.s;
                let x = a.a.x as f32 - mx + c.g.measure(t, size, 600);
                c.fill(R::new(x + 2.0, a.a.y as f32 - my, 2.0 * s, size * 1.25), 1.0, COLORS[self.color], 1.0);
            }
            if matches!(a.shape, Shape::Patch(_)) {
                let r = rect(a.a, a.b);
                let (w, h) = wh(&r);
                let r = R::new(r.left as f32 - mx, r.top as f32 - my, w as f32, h as f32);
                c.fill(r, 0.0, INK, 0.18);
                c.stroke(r, 0.0, 1.5, INK, 0.9);
            }
        }
        if self.tool.hides() && self.live.is_none() {
            for r in self.panes[n].secrets.lock().unwrap().iter() {
                let r = R::new(r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32);
                c.fill(r, 4.0 * s, RED, 0.14);
                c.stroke(r, 4.0 * s, 1.5 * s, RED, 1.0);
            }
        }
        // Words these tools can snap to
        if self.tool.hides() && self.live.is_none() && m.has(self.cur) && self.bar_hover.is_none() {
            if let Some(r) = self.word(n) {
                let r = R::new(r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32);
                c.fill(r, 4.0 * s, BLUE, 0.28);
                c.stroke(r, 4.0 * s, 1.5 * s, BLUE, 1.0);
            }
        }
        let k = self.dim.get(now);
        if self.tool == Tool::Region && self.mode != Mode::Color {
            let d = 0.42 * k;
            match self.selection().and_then(|r| intersect(&r, &m.rect)) {
                Some(r) => {
                    let (l, t) = ((r.left - m.rect.left) as f32, (r.top - m.rect.top) as f32);
                    let (rt, b) = ((r.right - m.rect.left) as f32, (r.bottom - m.rect.top) as f32);
                    for q in [
                        R::new(0.0, 0.0, full.w, t),
                        R::new(0.0, b, full.w, full.h - b),
                        R::new(0.0, t, l, b - t),
                        R::new(rt, t, full.w - rt, b - t),
                    ] {
                        if q.w > 0.0 && q.h > 0.0 {
                            c.fill(q, 0.0, BLACK, d);
                        }
                    }
                    c.stroke(R::new(l, t, rt - l, b - t).grow(-0.5), 0.0, if self.start.is_some() { 1.0 } else { 2.0 }, INK, 0.95 * k);
                }
                None => c.fill(full, 0.0, BLACK, d),
            }
        }
        if m.has(self.cur) && self.tool == Tool::Region && self.bar_hover.is_none() {
            self.loupe(c, n, bmp, k);
        }
        if n == self.home && self.mode.draws() {
            self.bar(c, n, now);
        }
    }

    fn loupe(&self, c: &Canvas, n: usize, bmp: &ID2D1Bitmap1, k: f32) {
        let (m, s) = (&self.mons[n], self.panes[n].s);
        let (x, y) = (self.cur.x - m.rect.left, self.cur.y - m.rect.top);
        let cells = 15;
        let cell = (8.0 * s).round();
        let side = cells as f32 * cell;
        let label = 26.0 * s;
        let gap = 22.0 * s;
        let mut px = x as f32 + gap;
        let mut py = y as f32 + gap;
        if px + side > m.w() as f32 - 8.0 {
            px = x as f32 - gap - side;
        }
        if py + side + label > m.h() as f32 - 8.0 {
            py = y as f32 - gap - side - label;
        }
        let half = cells / 2;
        let frame = R::new(px, py, side, side + label);
        c.fill(frame.grow(1.5 * s), 8.0 * s, BLACK, k);
        c.stroke(frame.grow(2.0 * s), 8.5 * s, 1.0 * s, INK, 0.35 * k);
        let src = R::new((x - half) as f32, (y - half) as f32, cells as f32, cells as f32);
        c.clip(R::new(px, py, side, side), || {
            c.fill(R::new(px, py, side, side), 0.0, CARD, k);
            // Clamp to the bitmap so edges show black, not stretched pixels.
            let l = src.x.max(0.0);
            let t = src.y.max(0.0);
            let r = (src.x + src.w).min(m.w() as f32);
            let b = (src.y + src.h).min(m.h() as f32);
            let dst = R::new(px + (l - src.x) * cell, py + (t - src.y) * cell, (r - l) * cell, (b - t) * cell);
            c.image_part(bmp, dst, R::new(l, t, r - l, b - t), k);
        });
        let mid = R::new(px + half as f32 * cell, py + half as f32 * cell, cell, cell);
        c.stroke(mid, 0.0, 1.0 * s, BLACK, k);
        c.stroke(mid.grow(1.0 * s), 0.0, 1.0 * s, INK, k);
        let text = if self.mode == Mode::Color {
            let h = self.pixel().unwrap_or(0);
            c.fill(R::new(px + 8.0 * s, py + side + 7.0 * s, 12.0 * s, 12.0 * s), 3.0 * s, h, k);
            format!("#{h:06X}")
        } else {
            let (w, h) = self.selection().map(|r| wh(&r)).unwrap_or((0, 0));
            format!("{w} × {h}")
        };
        c.text(&text, R::new(px, py + side, side, label), 13.0 * s, INK, k, Align::Center);
    }

    fn pixel(&self) -> Option<u32> {
        let n = self.mons.iter().position(|m| m.has(self.cur))?;
        let (m, f) = (&self.mons[n], self.frames.get(n)?);
        let i = (((self.cur.y - m.rect.top) * f.w + self.cur.x - m.rect.left) * 4) as usize;
        let p = f.px.get(i..i + 3)?;
        Some((p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32)
    }

    // ── Toolbar ────────────────────────────────────────────────────────────

    /// Buttons in logical px, relative to the bar; and the bar's size.
    fn bar_items(&self, g: &Gfx) -> (Vec<(Bar, R)>, f32, f32) {
        let (pad, hgt, gap) = (12.0, 36.0, 4.0);
        let mut x = pad;
        let y = 8.0;
        let mut v = Vec::new();
        let mut put = |b, w: f32, x: &mut f32| {
            v.push((b, R::new(*x, y, w, hgt)));
            *x += w + gap;
        };
        for (t, _) in TOOLS {
            let w = match t {
                Tool::Blur => g.measure("Blur", 13.0, 500) + 24.0,
                Tool::Pixelate => g.measure("Pixelate", 13.0, 500) + 24.0,
                _ => 36.0,
            };
            put(Bar::Tool(t), w, &mut x);
        }
        let found = self.found();
        if self.tool.hides() && found > 0 {
            x += 8.0;
            put(Bar::Secrets, g.measure(&format!("Hide {found}"), 13.0, 600) + 26.0, &mut x);
        }
        if self.tool != Tool::Region && !self.tool.hides() {
            x += 8.0;
            for n in 0..COLORS.len() {
                put(Bar::Color(n), 24.0, &mut x);
            }
        }
        x += 8.0;
        if !self.annos.is_empty() {
            put(Bar::Undo, 36.0, &mut x);
        }
        put(Bar::Cancel, 36.0, &mut x);
        (v, x - gap + pad, hgt + 16.0)
    }

    fn bar_origin(&self, g: &Gfx) -> (f32, f32) {
        let m = &self.mons[self.home];
        let s = self.panes[self.home].s;
        let (_, w, _) = self.bar_items(g);
        ((m.w() as f32 / s - w) / 2.0, 0.0)
    }

    fn bar_hit(&self, g: &Gfx, p: POINT) -> Option<Option<Bar>> {
        if !self.mode.draws() || !self.mons[self.home].has(p) || self.start.is_some() || self.dragging() {
            return None;
        }
        let m = &self.mons[self.home];
        let s = self.panes[self.home].s;
        let (ox, _) = self.bar_origin(g);
        let (x, y) = ((p.x - m.rect.left) as f32 / s - ox, (p.y - m.rect.top) as f32 / s);
        let (v, w, h) = self.bar_items(g);
        if !R::new(0.0, 0.0, w, h).has(x, y) {
            return None;
        }
        Some(v.into_iter().find(|b| b.1.has(x, y)).map(|b| b.0))
    }

    fn dragging(&self) -> bool {
        self.live.is_some() && !self.typing
    }

    fn bar(&self, c: &Canvas, n: usize, now: Instant) {
        // Out of the way while something is being dragged.
        if self.start.is_some() || self.dragging() {
            return;
        }
        use windows_numerics::Matrix3x2;
        let s = self.panes[n].s;
        let (ox, _) = self.bar_origin(c.g);
        let (v, w, h) = self.bar_items(c.g);
        let m = Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: ox * s, M32: self.bar_y.get(now) * s };
        c.transform(&m);
        c.island(R::new(0.0, 0.0, w, h), 24.0, 14.0);
        for (b, r) in v {
            let on = b == Bar::Tool(self.tool);
            let hov = self.bar_hover == Some(b);
            let ink = if on { BLACK } else { INK };
            if b == Bar::Secrets {
                c.fill(r, 18.0, RED, if hov { 1.0 } else { 0.88 });
                c.text_w(&format!("Hide {}", self.found()), r, 13.0, 600, BLACK, 1.0, Align::Center);
            } else if !matches!(b, Bar::Color(_)) {
                if on {
                    c.fill(r, 18.0, INK, 1.0);
                } else if hov {
                    c.fill(r, 18.0, PILL_HOVER, 1.0);
                }
            }
            let (cx, cy) = (r.cx(), r.cy());
            match b {
                Bar::Tool(Tool::Region) => c.icon(0xE7A8, r, 16.0, ink, 1.0),
                Bar::Tool(Tool::Blur) => c.text("Blur", r, 13.0, ink, 1.0, Align::Center),
                Bar::Tool(Tool::Pixelate) => c.text("Pixelate", r, 13.0, ink, 1.0, Align::Center),
                Bar::Tool(Tool::Erase) => c.icon(0xE75C, r, 16.0, ink, 1.0),
                Bar::Secrets => {}
                Bar::Tool(Tool::Box) => c.stroke(R::new(cx - 8.0, cy - 6.0, 16.0, 12.0), 2.0, 1.6, ink, 1.0),
                Bar::Tool(Tool::Arrow) => arrow(c, (cx - 7.0, cy + 7.0), (cx + 7.0, cy - 7.0), 1.6, ink, 1.0),
                Bar::Tool(Tool::Pen) => c.icon(0xE70F, r, 15.0, ink, 1.0),
                Bar::Tool(Tool::Mark) => c.icon(0xE7E6, r, 15.0, ink, 1.0),
                Bar::Tool(Tool::Text) => c.text_w("T", r, 17.0, 600, ink, 1.0, Align::Center),
                Bar::Tool(Tool::Step) => {
                    c.ring(cx, cy, 9.0, 1.6, ink, 1.0);
                    c.text_w("1", r, 12.0, 600, ink, 1.0, Align::Center);
                }
                Bar::Color(i) => {
                    c.circle(cx, cy, if hov { 8.5 } else { 7.5 }, COLORS[i], 1.0);
                    if i == 5 {
                        c.ring(cx, cy, 7.5, 1.0, DIM, 1.0);
                    }
                    if i == self.color {
                        c.ring(cx, cy, 11.0, 1.6, INK, 1.0);
                    }
                }
                Bar::Undo => c.icon(0xE7A7, r, 15.0, INK, 1.0),
                Bar::Cancel => c.icon(0xE711, r, 14.0, INK, 1.0),
            }
        }
        c.transform(&Matrix3x2::identity());
    }

    // ── Input ──────────────────────────────────────────────────────────────

    fn pick(&mut self, t: Tool) {
        self.commit_live();
        // Asking twice means: everything you found.
        if t == self.tool && t.hides() && self.found() > 0 {
            self.hide_secrets();
        }
        self.tool = t;
        if t.hides() {
            self.start_ocr();
        }
        self.touch();
    }

    fn found(&self) -> usize {
        self.panes.iter().map(|p| p.secrets.lock().unwrap().len()).sum()
    }

    fn hide_secrets(&mut self) {
        let mut all = Vec::new();
        for (n, p) in self.panes.iter().enumerate() {
            all.extend(p.secrets.lock().unwrap().drain(..).map(|r| (n, r)));
        }
        self.hide(all);
    }

    fn start_ocr(&mut self) {
        if std::mem::replace(&mut self.ocr_started, true) {
            return;
        }
        for (p, f) in self.panes.iter().zip(&self.frames) {
            let (out, secrets, f) = (p.words.clone(), p.secrets.clone(), f.clone());
            let hwnd = p.hwnd.0 as isize;
            std::thread::spawn(move || {
                let w = crate::ocr::words(&f).unwrap_or_default();
                *secrets.lock().unwrap() = crate::secrets::find(&w)
                    .into_iter()
                    .map(|r| redact::padded(crate::secrets::snug(&f, &w, r)))
                    .collect();
                *out.lock().unwrap() = Some(w);
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_NULL, WPARAM(0), LPARAM(0));
                }
            });
        }
    }

    /// Word under the cursor, in pane pixels.
    fn word(&self, n: usize) -> Option<RECT> {
        let m = self.mons[n].rect;
        let p = POINT { x: self.cur.x - m.left, y: self.cur.y - m.top };
        // A found secret is one target, however many words it spans.
        if let Some(r) = self.panes[n].secrets.lock().unwrap().iter().find(|r| inside(r, p)) {
            return Some(*r);
        }
        redact::word_at(self.panes[n].words.lock().unwrap().as_ref()?, p)
    }

    /// The word under the cursor. False if there is none.
    fn hide_word(&mut self) -> bool {
        let Some(n) = self.mons.iter().position(|m| m.has(self.cur)) else { return false };
        let Some(r) = self.word(n) else { return false };
        self.hide(vec![(n, r)]);
        true
    }

    /// A desktop rectangle, on whichever monitors it touches.
    fn hide_pixels(&mut self, r: RECT) {
        let parts = self
            .mons
            .iter()
            .enumerate()
            .map(|(n, m)| {
                let m = m.rect;
                (n, RECT { left: r.left - m.left, top: r.top - m.top, right: r.right - m.left, bottom: r.bottom - m.top })
            })
            .collect();
        self.hide(parts);
    }

    /// Pane rectangles, as one undo step.
    fn hide(&mut self, parts: Vec<(usize, RECT)>) {
        let mut patches = Vec::new();
        for (n, r) in parts {
            let f = &mut self.frames[n];
            let did = match self.tool {
                Tool::Erase => redact::erase_rect(f, r),
                t => redact::apply(f, r, t == Tool::Blur),
            };
            if let Some((rr, before)) = did {
                // Nothing left to find there.
                self.panes[n].secrets.lock().unwrap().retain(|s| !covers(&rr, s));
                self.mark_stale(n, rr);
                patches.push((n, rr, before));
            }
        }
        self.push_patch(patches);
    }

    fn wipe(&mut self, pts: &[POINT], rad: f32) {
        let mut patches = Vec::new();
        for n in 0..self.mons.len() {
            let m = self.mons[n].rect;
            let local: Vec<POINT> = pts.iter().map(|p| POINT { x: p.x - m.left, y: p.y - m.top }).collect();
            if let Some((r, before)) = redact::erase(&mut self.frames[n], &local, rad) {
                self.mark_stale(n, r);
                patches.push((n, r, before));
            }
        }
        self.push_patch(patches);
    }

    fn push_patch(&mut self, patches: Vec<(usize, RECT, Frame)>) {
        if !patches.is_empty() {
            let p = POINT::default();
            self.annos.push(Anno { shape: Shape::Patch(patches), a: p, b: p, color: 0, width: 0.0, s: 1.0 });
        }
        self.touch();
    }

    fn mark_stale(&mut self, n: usize, r: RECT) {
        let p = &mut self.panes[n];
        p.stale = Some(match p.stale {
            Some(o) => RECT {
                left: o.left.min(r.left),
                top: o.top.min(r.top),
                right: o.right.max(r.right),
                bottom: o.bottom.max(r.bottom),
            },
            None => r,
        });
        p.dirty = true;
    }

    fn undo(&mut self) {
        if self.live.take().is_some() {
            self.typing = false;
        } else if let Some(a) = self.annos.pop() {
            if let Shape::Patch(list) = a.shape {
                for (n, r, before) in list.into_iter().rev() {
                    self.frames[n].paste(&before, r.left, r.top);
                    self.mark_stale(n, r);
                }
            }
        }
        self.touch();
    }

    fn commit_live(&mut self) {
        self.typing = false;
        let Some(a) = self.live.take() else { return };
        let keep = match &a.shape {
            Shape::Text(t) => !t.trim().is_empty(),
            Shape::Pen(p) | Shape::Mark(p) => p.len() > 1,
            Shape::Box | Shape::Arrow => (a.a.x - a.b.x).abs() + (a.a.y - a.b.y).abs() > 4,
            Shape::Step(_) => true,
            Shape::Patch(_) | Shape::Wipe(_) => false,
        };
        if keep {
            self.annos.push(a);
        }
        self.touch();
    }

    fn point(&self, n: usize, l: LPARAM) -> POINT {
        POINT {
            x: (l.0 & 0xffff) as i16 as i32 + self.mons[n].rect.left,
            y: ((l.0 >> 16) & 0xffff) as i16 as i32 + self.mons[n].rect.top,
        }
    }

    pub fn message(&mut self, app: &App, n: usize, msg: u32, wp: WPARAM, l: LPARAM) -> Outcome {
        if self.fly.is_some() {
            return Outcome::None;
        }
        match msg {
            WM_NULL => self.touch(),
            WM_MOUSEMOVE => {
                let old = self.cur;
                self.cur = self.point(n, l);
                if self.cur != old {
                    self.moved(app);
                }
            }
            WM_LBUTTONDOWN => {
                self.cur = self.point(n, l);
                return self.down(app, n);
            }
            WM_LBUTTONUP => {
                self.cur = self.point(n, l);
                unsafe {
                    let _ = ReleaseCapture();
                }
                return self.up();
            }
            WM_RBUTTONUP => {
                if self.start.take().is_some() || self.live.is_some() {
                    self.live = None;
                    self.typing = false;
                    self.touch();
                } else if !self.annos.is_empty() {
                    self.undo();
                } else {
                    return Outcome::Cancel;
                }
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
                self.width = (self.width + d).clamp(1.0, 24.0);
                if let Some(a) = &mut self.live {
                    a.width = self.width;
                }
                self.touch();
            }
            WM_CHAR if self.typing => {
                if let (Some(Anno { shape: Shape::Text(t), .. }), Some(ch)) = (&mut self.live, char::from_u32(wp.0 as u32)) {
                    match ch {
                        '\u{8}' => {
                            t.pop();
                        }
                        c if c >= ' ' => t.push(c),
                        _ => {}
                    }
                    self.touch();
                }
            }
            WM_KEYDOWN => return self.key(wp.0 as u16),
            _ => {}
        }
        Outcome::None
    }

    fn moved(&mut self, app: &App) {
        let hov = self.bar_hit(&app.g, self.cur);
        let inner = hov.flatten();
        let on_bar = hov.is_some();
        if inner != self.bar_hover {
            self.bar_hover = inner;
            self.panes[self.home].dirty = true;
        }
        unsafe {
            let id = if on_bar {
                IDC_ARROW
            } else if self.tool == Tool::Text {
                IDC_IBEAM
            } else {
                IDC_CROSS
            };
            if let Ok(c) = LoadCursorW(None, id) {
                SetCursor(Some(c));
            }
        }
        let cur = self.cur;
        if !self.typing {
            if let Some(a) = &mut self.live {
                a.b = cur;
                if let Shape::Pen(p) | Shape::Mark(p) | Shape::Wipe(p) = &mut a.shape {
                    p.push(cur);
                }
            }
        }
        self.touch();
    }

    fn down(&mut self, app: &App, n: usize) -> Outcome {
        if let Some(hit) = self.bar_hit(&app.g, self.cur) {
            match hit {
                Some(Bar::Tool(t)) => self.pick(t),
                Some(Bar::Color(i)) => {
                    self.color = i;
                    if let Some(a) = &mut self.live {
                        a.color = COLORS[i];
                    }
                }
                Some(Bar::Undo) => self.undo(),
                Some(Bar::Secrets) => self.hide_secrets(),
                Some(Bar::Cancel) => return Outcome::Cancel,
                None => {}
            }
            self.touch();
            return Outcome::None;
        }
        if self.mode == Mode::Color {
            return self.pixel().map(Outcome::Color).unwrap_or(Outcome::None);
        }
        let hwnd = self.panes[n].hwnd;
        unsafe {
            SetCapture(hwnd);
        }
        let p = self.cur;
        let color = COLORS[self.color];
        let mk = |shape| Anno { shape, a: p, b: p, color, width: self.width, s: self.panes[n].s };
        match self.tool {
            Tool::Region => self.start = Some(p),
            Tool::Blur | Tool::Pixelate => self.live = Some(mk(Shape::Patch(Vec::new()))),
            Tool::Erase => {
                let hex = self.pixel().unwrap_or(0);
                let mut a = mk(Shape::Wipe(vec![p]));
                a.color = hex;
                self.live = Some(a);
            }
            Tool::Box => self.live = Some(mk(Shape::Box)),
            Tool::Arrow => self.live = Some(mk(Shape::Arrow)),
            Tool::Pen => self.live = Some(mk(Shape::Pen(vec![p]))),
            Tool::Mark => self.live = Some(mk(Shape::Mark(vec![p]))),
            Tool::Text => {
                let was = self.typing;
                let a = mk(Shape::Text(String::new()));
                self.commit_live();
                if !was {
                    self.live = Some(a);
                    self.typing = true;
                    unsafe {
                        let _ = SetFocus(Some(hwnd));
                    }
                }
            }
            Tool::Step => {
                let k = self.annos.iter().filter(|a| matches!(a.shape, Shape::Step(_))).count() + 1;
                let a = mk(Shape::Step(k));
                self.annos.push(a);
            }
        }
        self.touch();
        Outcome::None
    }

    fn up(&mut self) -> Outcome {
        self.touch();
        if self.tool == Tool::Region {
            let Some(s) = self.start.take() else { return Outcome::None };
            let r = rect(s, self.cur);
            let (w, h) = wh(&r);
            if self.mode == Mode::Window || (w < 5 && h < 5) {
                return self.snap().map(Outcome::Shot).unwrap_or(Outcome::None);
            }
            return if w > 1 && h > 1 { Outcome::Shot(r) } else { Outcome::None };
        }
        if self.typing {
            return Outcome::None;
        }
        match self.live.as_ref().map(|a| (&a.shape, rect(a.a, a.b))) {
            Some((Shape::Patch(_), r)) => {
                self.live = None;
                let (w, h) = wh(&r);
                if w > 5 || h > 5 {
                    self.hide_pixels(r);
                } else {
                    self.hide_word();
                }
                return Outcome::None;
            }
            Some((Shape::Wipe(p), _)) => {
                let p = p.clone();
                let a = self.live.take().unwrap();
                let moved = p.iter().any(|q| (q.x - p[0].x).abs() + (q.y - p[0].y).abs() > 5);
                if moved || !self.hide_word() {
                    self.wipe(&p, a.width * 2.5 * a.s);
                }
                return Outcome::None;
            }
            _ => {}
        }
        self.commit_live();
        Outcome::None
    }

    fn key(&mut self, k: u16) -> Outcome {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        if k == VK_ESCAPE.0 {
            if self.start.take().is_some() || self.live.take().is_some() {
                self.typing = false;
                self.touch();
                return Outcome::None;
            }
            return Outcome::Cancel;
        }
        if self.typing {
            if k == VK_RETURN.0 {
                self.commit_live();
            }
            return Outcome::None;
        }
        if ctrl && k == b'Z' as u16 {
            self.undo();
            return Outcome::None;
        }
        if k == VK_RETURN.0 || k == VK_SPACE.0 {
            if self.mode == Mode::Color {
                return self.pixel().map(Outcome::Color).unwrap_or(Outcome::None);
            }
            return self.mons.iter().find(|m| m.has(self.cur)).map(|m| Outcome::Shot(m.rect)).unwrap_or(Outcome::None);
        }
        // Nudge the cursor a pixel at a time.
        let step = if unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 { 10 } else { 1 };
        let (dx, dy) = match VIRTUAL_KEY(k) {
            VK_LEFT => (-step, 0),
            VK_RIGHT => (step, 0),
            VK_UP => (0, -step),
            VK_DOWN => (0, step),
            _ => (0, 0),
        };
        if (dx, dy) != (0, 0) {
            let p = cursor();
            unsafe {
                let _ = SetCursorPos(p.x + dx, p.y + dy);
            }
            return Outcome::None;
        }
        if self.mode.draws() && !ctrl {
            if let Some((t, _)) = TOOLS.iter().find(|t| t.1 == k) {
                self.pick(*t);
            }
        }
        Outcome::None
    }
}

fn arrow(c: &Canvas, a: (f32, f32), b: (f32, f32), w: f32, hex: u32, al: f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(0.01);
    let (ux, uy) = (dx / len, dy / len);
    let head = (w * 4.2 + 2.0).min(len * 0.6);
    let side = head * 0.52;
    let base = (b.0 - ux * head, b.1 - uy * head);
    c.poly(&[a, (b.0 - ux * head * 0.8, b.1 - uy * head * 0.8)], w, hex, al, false);
    c.poly(&[b, (base.0 - uy * side, base.1 + ux * side), (base.0 + uy * side, base.1 - ux * side)], 0.0, hex, al, true);
}

fn paint(c: &Canvas, a: &Anno, ox: f32, oy: f32, al: f32) {
    let p = |q: POINT| (q.x as f32 - ox, q.y as f32 - oy);
    let w = a.width * a.s;
    match &a.shape {
        Shape::Patch(_) => {}
        Shape::Wipe(v) => {
            let pts: Vec<_> = v.iter().map(|q| p(*q)).collect();
            let r = w * 2.5;
            c.circle(pts[0].0, pts[0].1, r, a.color, al);
            c.poly(&pts, r * 2.0, a.color, al, false);
        }
        Shape::Box => {
            let r = rect(a.a, a.b);
            let (x, y) = p(POINT { x: r.left, y: r.top });
            let (rw, rh) = wh(&r);
            c.stroke(R::new(x, y, rw as f32, rh as f32), 3.0 * a.s, w * 0.8, a.color, al);
        }
        Shape::Arrow => arrow(c, p(a.a), p(a.b), w, a.color, al),
        Shape::Pen(v) => c.poly(&v.iter().map(|q| p(*q)).collect::<Vec<_>>(), w, a.color, al, false),
        Shape::Mark(v) => c.poly(&v.iter().map(|q| p(*q)).collect::<Vec<_>>(), w * 4.5, a.color, 0.38 * al, false),
        Shape::Text(t) => {
            let size = a.width * 5.5 * a.s;
            let (x, y) = p(a.a);
            c.text_w(t, R::new(x, y, 8000.0, size * 1.25), size, 600, a.color, al, Align::Left);
        }
        Shape::Step(n) => {
            let (x, y) = p(a.a);
            let r = (a.width * 2.2 + 5.0) * a.s;
            c.circle(x, y, r, a.color, al);
            let ink = if a.color == COLORS[5] || a.color == COLORS[0] || a.color == COLORS[3] { INK } else { BLACK };
            c.text_w(&n.to_string(), R::new(x - r, y - r, r * 2.0, r * 2.0), r * 1.15, 600, ink, al, Align::Center);
        }
    }
}
