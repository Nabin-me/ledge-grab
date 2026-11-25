//! The notch: a click-through window that draws, and an invisible one that feels the mouse.
use crate::anim::{Anim, Curve};
use crate::capture::{Frame, Monitor, cursor, monitors};
use crate::gfx::*;
use crate::overlay::Mode;
use crate::pointer_timing::{CLOSE_MS, Dwell, OPEN_MS, PANEL_CLOSE_MS, Wake, in_hover_zone};
use crate::store::{Filter, Kind, Store};
use crate::{App, T_DWELL, T_FLASH, T_LEAVE, WM_DRAG_OUT, testing};
use std::collections::HashMap;
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};
use windows_numerics::Matrix3x2;

const WIN: (f32, f32) = (760.0, 470.0);
const EAR: f32 = 14.0;
pub const TILE: (f32, f32) = (132.0, 84.0);
const GAP: f32 = 10.0;
const PAD: f32 = 16.0;
const COLS: usize = 4;
const ROWS: usize = 3;
const BAR: f32 = 52.0;
const DESK_SHIFT: f32 = 480.0;

mod glyph {
    pub const ADD: u16 = 0xE710;
    pub const DOWN: u16 = 0xE70D;
    pub const UP: u16 = 0xE70E;
    pub const COPY: u16 = 0xE8C8;
    pub const PIN: u16 = 0xE718;
    pub const UNPIN: u16 = 0xE77A;
    pub const TRASH: u16 = 0xE74D;
    pub const NEXT: u16 = 0xE72A;
    pub const BACK: u16 = 0xE72B;
    pub const CHECK: u16 = 0xE73E;
    pub const PINNED: u16 = 0xE841;
    pub const GEAR: u16 = 0xE713;
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Rest,
    Peek,
    Shelf,
    Menu,
    Settings,
    /// A new shot, shown for a moment.
    Flash(i64),
    Count(i64),
    Done,
    /// Recording a clip.
    Rec,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Btn {
    Capture,
    Open,
    Close,
    Copy,
    Pin,
    Delete,
    More,
    Merge,
    Backdrop,
    Float,
    Gif,
    Queue,
    Stack,
    Unstack,
    Go(Mode),
    Stop,
    StopRec,
    Filter(usize),
    BackToApps,
    Settings,
    Quit,
    Key(usize),
    Folder,
    Step(usize, i64),
    History,
    Login,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Tile(i64),
    /// The ID of the newest item identifies the folder, not an actionable capture.
    AppFolder(i64),
    Check(i64),
    B(Btn),
}

struct Spot {
    hit: Hit,
    r: R,
}

struct Drag {
    ids: Vec<i64>,
    grab: (f32, f32),
    at: (f32, f32),
}

#[derive(Clone, Copy, PartialEq)]
enum Drop {
    Onto(i64),
    Before(Option<i64>),
}

pub struct Notch {
    pub win: HWND,
    pub sensor: HWND,
    surf: Surface,
    pub mon: Monitor,
    pub s: f32,
    pub view: View,
    prev: Option<View>,
    swap: Instant,
    w: Anim,
    h: Anim,
    rad: Anim,
    shift: f32,
    hover: Option<Hit>,
    press: Option<(Hit, f32, f32)>,
    drag: Option<Drag>,
    pub sel: Vec<i64>,
    more: bool,
    scroll: Anim,
    apps_scroll: f32,
    just_opened_app: bool,
    lift: HashMap<i64, Anim>,
    folder_peek: HashMap<i64, Anim>,
    thumbs: HashMap<i64, ID2D1Bitmap1>,
    copied: Option<(i64, Instant)>,
    captured: Option<(i64, Instant)>,
    failed: Option<Instant>,
    /// Hotkey row waiting for keys.
    pub recording: Option<usize>,
    pub modal: bool,
    /// When the clip being recorded began.
    pub rec: Option<Instant>,
    hidden: bool,
    dirty: bool,
    /// A background job is running.
    pub working: bool,
    tracking: bool,
    open_dwell: Dwell,
    close_grace: Dwell,
    /// Last mouse position, island-local.
    mouse: (f32, f32),
    born: Instant,
}

fn groups(st: &Store, v: View) -> Vec<Vec<i64>> {
    match v {
        View::Shelf => st.shelf(),
        _ => st.recent(),
    }
}

fn group_of(st: &Store, v: View, id: i64) -> Vec<i64> {
    groups(st, v).into_iter().find(|g| g.contains(&id)).unwrap_or(vec![id])
}

/// Cut to a width, with an ellipsis.
fn fitted(g: &Gfx, s: &str, size: f32, w: f32) -> String {
    if g.measure(s, size, 500) <= w {
        return s.into();
    }
    let mut t: String = s.into();
    while !t.is_empty() && g.measure(&format!("{t}…"), size, 500) > w {
        t.pop();
    }
    t.trim_end().to_string() + "…"
}

fn clock(s: i64) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

/// Shelf filters worth offering.
fn filters(st: &Store) -> Vec<(usize, &'static str)> {
    let mut v = vec![(0, "All")];
    if st.has(|i| matches!(i.kind, Kind::Clip | Kind::Gif)) {
        v.push((1, "Clips"));
    }
    if !st.items.is_empty() {
        v.push((2, "App folders"));
    }
    v
}

fn folder_name(app: &str) -> &str {
    let app = app.trim();
    if app.is_empty() { "Other (no source)" } else { app }
}

/// Deterministic muted accents so a folder stays recognizable without app icons.
fn folder_palette(name: &str) -> (u32, u32, u32) {
    const PALETTES: [(u32, u32, u32); 5] = [
        (0x253950, 0x36526e, 0x6887a6),
        (0x382f4b, 0x514362, 0x88769e),
        (0x2b3d39, 0x3a5850, 0x6d9b88),
        (0x413339, 0x5b4951, 0x977a89),
        (0x43382b, 0x5e4e3a, 0xa08a6b),
    ];
    let app = name.trim();
    let hash = app.bytes().fold(0usize, |v, b| v.wrapping_mul(31).wrapping_add(b.to_ascii_lowercase() as usize));
    PALETTES[hash % PALETTES.len()]
}

fn folder_sheet_pose(tile: R, wing: f32, peek: f32) -> (R, f32) {
    let peek = peek.clamp(0.0, 1.0);
    let x = tile.cx() - 13.0 + wing * (4.0 + 12.0 * peek);
    let y = tile.y + 22.0 - 12.0 * peek + 3.0 * wing.abs();
    let angle = wing * (2.5 + 11.5 * peek);
    (R::new(x, y, 26.0, 29.0), angle)
}

fn draw_folder_sheet(c: &Canvas, sheet: R, angle: f32, paper: u32) {
    let mut parent = Matrix3x2::identity();
    unsafe { c.ctx.GetTransform(&mut parent) };
    // In local DIPs: rotate around the sheet's center, then apply the island's DPI/offset transform.
    let (sin, cos) = angle.to_radians().sin_cos();
    let (cx, cy) = (sheet.cx(), sheet.cy());
    let (tx, ty) = (cx - cos * cx + sin * cy, cy - sin * cx - cos * cy);
    c.transform(&Matrix3x2 {
        M11: cos * parent.M11 + sin * parent.M21,
        M12: cos * parent.M12 + sin * parent.M22,
        M21: -sin * parent.M11 + cos * parent.M21,
        M22: -sin * parent.M12 + cos * parent.M22,
        M31: tx * parent.M11 + ty * parent.M21 + parent.M31,
        M32: tx * parent.M12 + ty * parent.M22 + parent.M32,
    });
    c.fill(sheet, 3.0, paper, 1.0);
    c.stroke(sheet.grow(-0.5), 2.5, 1.0, BLACK, 0.22);
    c.fill(R::new(sheet.x + 5.0, sheet.y + 5.0, 16.0, 7.0), 1.5, 0x344457, 0.34);
    c.fill(R::new(sheet.x + 5.0, sheet.y + 15.0, 12.0, 1.5), 0.75, 0x344457, 0.26);
    c.transform(&parent);
}

pub fn size(g: &Gfx, st: &Store, v: View) -> (f32, f32, f32) {
    let n = match v {
        View::Peek | View::Shelf => groups(st, v).len(),
        _ => 0,
    };
    match v {
        View::Rest => (210.0, 9.0, 6.0),
        View::Peek => {
            let k = n.min(3) as f32;
            let tiles = k * TILE.0 + (k - 1.0).max(0.0) * GAP;
            if n == 0 {
                (PAD * 2.0 + 36.0 * 2.0 + 8.0, 60.0, 22.0)
            } else {
                (PAD * 2.0 + tiles + 46.0, 116.0, 26.0)
            }
        }
        View::Shelf => {
            let rows = n.div_ceil(COLS).clamp(1, ROWS) as f32;
            let w = PAD * 2.0 + COLS as f32 * TILE.0 + (COLS - 1) as f32 * GAP;
            (w, PAD + rows * (TILE.1 + GAP) + BAR, 28.0)
        }
        View::Menu => (424.0, PAD * 2.0 + 7.0 * 38.0 + 4.0, 28.0),
        View::Settings => (400.0, PAD * 2.0 + 10.0 * 38.0 + 40.0, 28.0),
        View::Rec => (118.0, 36.0, 16.0),
        View::Flash(_) => (TILE.0 + PAD * 2.0, 116.0, 26.0),
        View::Count(_) | View::Done => {
            let _ = g;
            (84.0, 48.0, 20.0)
        }
    }
}

pub fn key_name(k: (u32, u32)) -> String {
    let mut s = String::new();
    for (m, n) in [(MOD_WIN, "Win"), (MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift")] {
        if k.0 & m.0 != 0 {
            s += n;
            s += " ";
        }
    }
    let vk = VIRTUAL_KEY(k.1 as u16);
    let name = match k.1 {
        0 => "None".into(),
        c @ (0x30..=0x39 | 0x41..=0x5A) => (c as u8 as char).to_string(),
        c @ 0x70..=0x87 => format!("F{}", c - 0x6F),
        _ => match vk {
            VK_SNAPSHOT => "PrtSc".into(),
            VK_SPACE => "Space".into(),
            VK_INSERT => "Ins".into(),
            VK_PAUSE => "Pause".into(),
            VK_HOME => "Home".into(),
            VK_END => "End".into(),
            VK_PRIOR => "PgUp".into(),
            VK_NEXT => "PgDn".into(),
            _ => {
                let mut b = [0u16; 32];
                unsafe {
                    let sc = MapVirtualKeyW(k.1, MAPVK_VK_TO_VSC);
                    let n = GetKeyNameTextW((sc << 16) as i32, &mut b);
                    String::from_utf16_lossy(&b[..n as usize])
                }
            }
        },
    };
    s + &name
}

impl Notch {
    pub fn new(g: &Gfx, inst: HINSTANCE) -> Result<Notch> {
        let mon = monitors()[0];
        unsafe {
            let class = w!("LedgeGrabNotch");
            RegisterClassW(&WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(crate::wndproc),
                hInstance: inst,
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            });
            let make = |ex, title| {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | ex,
                    class,
                    title,
                    WS_POPUP,
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    Some(inst),
                    None,
                )
            };
            let win = make(WS_EX_TRANSPARENT | WS_EX_LAYERED, w!("Ledge Grab"))?;
            let sensor = make(WINDOW_EX_STYLE(0), w!("Ledge Grab sensor"))?;
            if !testing() {
                let _ = SetWindowDisplayAffinity(win, WDA_EXCLUDEFROMCAPTURE);
            }
            let s = mon.dpi as f32 / 96.0;
            let surf = g.surface(win, (WIN.0 * s) as i32, (WIN.1 * s) as i32)?;
            Ok(Notch {
                win,
                sensor,
                surf,
                mon,
                s,
                view: View::Rest,
                prev: None,
                swap: Instant::now(),
                w: Anim::new(210.0),
                h: Anim::new(0.0),
                rad: Anim::new(6.0),
                shift: 0.0,
                hover: None,
                press: None,
                drag: None,
                sel: Vec::new(),
                more: false,
                scroll: Anim::new(0.0),
                apps_scroll: 0.0,
                just_opened_app: false,
                lift: HashMap::new(),
                folder_peek: HashMap::new(),
                thumbs: HashMap::new(),
                copied: None,
                captured: None,
                failed: None,
                recording: None,
                modal: false,
                rec: None,
                hidden: false,
                dirty: true,
                working: false,
                tracking: false,
                open_dwell: Dwell::default(),
                close_grace: Dwell::default(),
                mouse: (0.0, 0.0),
                born: Instant::now(),
            })
        }
    }

    pub fn start(&mut self) {
        self.h.go(9.0, 0.5, Curve::Emphasized);
        unsafe {
            let _ = ShowWindow(self.win, SW_SHOWNOACTIVATE);
            let _ = ShowWindow(self.sensor, SW_SHOWNOACTIVATE);
        }
        self.place();
    }

    fn center_px(&self) -> i32 {
        (self.mon.rect.left + self.mon.rect.right) / 2 + (self.shift * self.s) as i32
    }

    /// Screen rectangle of the island at its current target size.
    pub fn island_px(&self) -> RECT {
        let (w, h) = (self.w.to * self.s, self.h.to * self.s);
        let l = self.center_px() - (w / 2.0) as i32;
        RECT { left: l, top: self.mon.rect.top, right: l + w as i32, bottom: self.mon.rect.top + h.ceil() as i32 }
    }

    /// Where a new shot lands, in screen pixels.
    pub fn landing(&self) -> RECT {
        let (w, h) = (TILE.0 * self.s, TILE.1 * self.s);
        let l = self.center_px() - (w / 2.0) as i32;
        let t = self.mon.rect.top + (PAD * self.s) as i32;
        RECT { left: l, top: t, right: l + w as i32, bottom: t + h as i32 }
    }

    fn place(&mut self) {
        let r = self.island_px();
        let ww = (WIN.0 * self.s) as i32;
        unsafe {
            let _ = SetWindowPos(
                self.win,
                Some(HWND_TOPMOST),
                self.center_px() - ww / 2,
                self.mon.rect.top,
                ww,
                (WIN.1 * self.s) as i32,
                SWP_NOACTIVATE,
            );
            let _ = SetWindowPos(
                self.sensor,
                Some(HWND_TOPMOST),
                r.left,
                r.top,
                r.right - r.left,
                (r.bottom - r.top).max(4),
                SWP_NOACTIVATE,
            );
        }
        self.dirty = true;
    }

    /// Monitor layout, DPI, or Personal Desk changed.
    pub fn adapt(&mut self, g: &Gfx, desk: bool) {
        let Some(mon) = monitors().first().copied() else { return };
        let shift = if desk { DESK_SHIFT } else { 0.0 };
        let moved = mon.rect != self.mon.rect || mon.dpi != self.mon.dpi;
        if moved {
            self.mon = mon;
            self.s = mon.dpi as f32 / 96.0;
            let _ = g;
            let _ = self.surf.resize((WIN.0 * self.s) as i32, (WIN.1 * self.s) as i32);
        }
        if moved || shift != self.shift {
            self.cancel_open();
            self.cancel_close();
            self.shift = shift;
            self.place();
            self.sync_auto_close();
        }
    }

    pub fn hide(&mut self, on: bool) {
        if self.hidden != on {
            self.hidden = on;
            self.cancel_open();
            self.cancel_close();
            unsafe {
                for h in [self.win, self.sensor] {
                    let _ = ShowWindow(h, if on { SW_HIDE } else { SW_SHOWNOACTIVATE });
                }
            }
            self.place();
            self.sync_auto_close();
        }
    }

    /// Keep above other topmost windows.
    pub fn raise(&self) {
        unsafe {
            for h in [self.win, self.sensor] {
                let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    pub fn show(&mut self, g: &Gfx, st: &Store, v: View) {
        let v = if v == View::Rest && self.rec.is_some() { View::Rec } else { v };
        let (w, h, r) = size(g, st, v);
        let same = std::mem::discriminant(&v) == std::mem::discriminant(&self.view)
            && !matches!(v, View::Flash(_) | View::Count(_));
        if !same {
            self.cancel_open();
            self.cancel_close();
            self.prev = Some(self.view);
            self.swap = Instant::now();
            self.more = false;
            if matches!(v, View::Rest | View::Rec) {
                self.sel.clear();
                self.scroll.jump(0.0);
                self.recording = None;
            }
        }
        self.view = v;
        let grow = w * h >= self.w.to * self.h.to;
        if matches!(v, View::Flash(_)) {
            self.w.go(w, 0.50, Curve::Spring);
            self.h.go(h, 0.50, Curve::Spring);
            self.rad.go(r, 0.42, Curve::Emphasized);
        } else {
            let (d, c) = if grow { (0.52, Curve::Emphasized) } else { (0.40, Curve::Standard) };
            self.w.go(w, d, c);
            self.h.go(h, d, c);
            self.rad.go(r, d, c);
        }
        self.set_hover(None);
        self.clamp_scroll(st);
        crate::set_notch_hook(self.expanded());
        self.place();
        self.sync_auto_close();
    }

    pub fn refresh(&mut self, g: &Gfx, st: &Store) {
        self.sel.retain(|i| st.get(*i).is_some());
        self.thumbs.retain(|i, _| st.get(*i).is_some());
        self.lift.retain(|i, _| st.get(*i).is_some());
        self.folder_peek.retain(|i, _| st.get(*i).is_some());
        self.show(g, st, self.view);
    }

    pub fn flash(&mut self, g: &Gfx, st: &Store, id: i64) {
        self.captured = Some((id, Instant::now()));
        match self.view {
            View::Rest | View::Flash(_) | View::Count(_) | View::Done | View::Rec => {
                self.show(g, st, View::Flash(id));
                unsafe {
                    SetTimer(Some(self.sensor), T_FLASH, 1500, None);
                }
            }
            _ => self.refresh(g, st),
        }
    }

    pub fn done(&mut self, g: &Gfx, st: &Store) {
        self.show(g, st, View::Done);
        unsafe {
            SetTimer(Some(self.sensor), T_FLASH, 900, None);
        }
    }

    pub fn expanded(&self) -> bool {
        matches!(self.view, View::Peek | View::Shelf | View::Menu | View::Settings)
    }

    pub fn mark_copied(&mut self, id: i64) {
        self.copied = Some((id, Instant::now()));
        self.dirty = true;
    }

    /// Something didn't work: one red blink, no words.
    pub fn fail(&mut self) {
        self.failed = Some(Instant::now());
        self.dirty = true;
    }

    pub fn repaint(&mut self) {
        self.dirty = true;
    }

    /// Drop thumbnails nobody will look at soon.
    pub fn trim(&mut self, st: &Store) {
        let keep: Vec<i64> = st.recent().iter().take(3).map(|g| g[0]).collect();
        self.thumbs.retain(|i, _| keep.contains(i));
    }

    fn max_scroll(&self, st: &Store) -> f32 {
        let rows = st.shelf().len().div_ceil(COLS);
        rows.saturating_sub(ROWS) as f32 * (TILE.1 + GAP)
    }

    fn clamp_scroll(&mut self, st: &Store) {
        let m = self.max_scroll(st);
        if self.scroll.to > m {
            self.scroll.go(m, 0.3, Curve::Emphasized);
        }
    }

    // ── Layout ─────────────────────────────────────────────────────────────

    fn tile_rects(&self, st: &Store, v: View, now: Instant) -> Vec<(i64, R)> {
        let y0 = PAD;
        groups(st, v)
            .iter()
            .enumerate()
            .filter_map(|(n, g)| match v {
                View::Peek if n < 3 => Some((g[0], R::new(PAD + n as f32 * (TILE.0 + GAP), y0, TILE.0, TILE.1))),
                View::Shelf => Some((
                    g[0],
                    R::new(
                        PAD + (n % COLS) as f32 * (TILE.0 + GAP),
                        y0 + (n / COLS) as f32 * (TILE.1 + GAP) - self.scroll.get(now),
                        TILE.0,
                        TILE.1,
                    ),
                )),
                _ => None,
            })
            .collect()
    }

    fn shelf_clip(h: f32, w: f32) -> R {
        R::new(0.0, 8.0, w, h - BAR - 8.0)
    }

    fn actions(&self, g: &Gfx, st: &Store) -> Vec<(Btn, &'static str, f32)> {
        let items: Vec<_> = self.sel.iter().filter_map(|i| st.get(*i)).collect();
        let all = |k: Kind| !items.is_empty() && items.iter().all(|i| i.kind == k);
        let many = items.len() > 1;
        let mut v = Vec::new();
        if all(Kind::Shot) {
            v.push((Btn::Merge, if many { "Merge as PDF" } else { "Convert to PDF" }));
            v.push((Btn::Backdrop, "Background"));
            v.push((Btn::Float, "Float"));
        }
        if all(Kind::Clip) {
            v.push((Btn::Gif, "GIF"));
        }
        if many {
            v.push((Btn::Queue, "Queue"));
        }
        let stacks: Vec<i64> = items.iter().map(|i| i.stack).collect();
        let one = !stacks.is_empty() && stacks[0] != 0 && stacks.iter().all(|s| *s == stacks[0]);
        if many && !one {
            v.push((Btn::Stack, "Stack"));
        } else if stacks.iter().any(|s| *s != 0) {
            v.push((Btn::Unstack, "Unstack"));
        }
        v.into_iter().map(|(b, t)| (b, t, g.measure(t, 13.0, 500) + 24.0)).collect()
    }

    fn menu(app_interval: bool, rec: bool) -> Vec<(Btn, &'static str)> {
        let mut v: Vec<(Btn, &'static str)> = [
            (Mode::Region, "Region"),
            (Mode::Screen, "Screen"),
            (Mode::Window, "Window"),
            (Mode::All, "All screens"),
            (Mode::Last, "Last region"),
            (Mode::Scroll, "Scrolling"),
            (Mode::Delay, "Delay"),
        ]
        .map(|(m, t)| (Btn::Go(m), t))
        .to_vec();
        v.push(if app_interval { (Btn::Stop, "Stop") } else { (Btn::Go(Mode::Interval), "Interval") });
        v.push(if rec { (Btn::StopRec, "Stop clip") } else { (Btn::Go(Mode::Clip), "Clip") });
        v.push((Btn::Go(Mode::Float), "Float"));
        v.push((Btn::Go(Mode::Text), "Copy text"));
        v.push((Btn::Go(Mode::Color), "Pick color"));
        v.push((Btn::Settings, "Settings"));
        v.push((Btn::Quit, "Quit"));
        v
    }

    fn spots(&self, app: &App, view: View, now: Instant) -> Vec<Spot> {
        let (g, st) = (&app.g, &app.store);
        let (w, h, _) = size(g, st, view);
        let mut v: Vec<Spot> = Vec::new();
        let b = |v: &mut Vec<Spot>, btn, r| v.push(Spot { hit: Hit::B(btn), r });
        match view {
            View::Peek => {
                if st.items.is_empty() {
                    b(&mut v, Btn::Capture, R::new(PAD, 12.0, 36.0, 36.0));
                    b(&mut v, Btn::Settings, R::new(PAD + 44.0, 12.0, 36.0, 36.0));
                } else {
                    let x = w - PAD - 36.0;
                    b(&mut v, Btn::Capture, R::new(x, PAD, 36.0, 26.0));
                    b(&mut v, Btn::Open, R::new(x, PAD + 29.0, 36.0, 26.0));
                    b(&mut v, Btn::Settings, R::new(x, PAD + 58.0, 36.0, 26.0));
                }
                for (id, r) in self.tile_rects(st, view, now) {
                    v.push(Spot { hit: Hit::Tile(id), r });
                }
            }
            View::Shelf => {
                let y = h - 44.0;
                b(&mut v, Btn::Close, R::new(w - PAD - 36.0, y, 36.0, 32.0));
                if self.more && !self.sel.is_empty() {
                    b(&mut v, Btn::More, R::new(PAD, y, 36.0, 32.0));
                    let mut x = PAD + 44.0;
                    for (k, _, tw) in self.actions(g, st) {
                        b(&mut v, k, R::new(x, y, tw, 32.0));
                        x += tw + 5.0;
                    }
                } else {
                    b(&mut v, Btn::Capture, R::new(w - PAD - 78.0, y, 36.0, 32.0));
                    b(&mut v, Btn::Settings, R::new(w - PAD - 120.0, y, 36.0, 32.0));
                    let mut x = PAD;
                    if !self.sel.is_empty() {
                        for k in [Btn::Copy, Btn::Pin, Btn::Delete, Btn::More] {
                            b(&mut v, k, R::new(x, y, 36.0, 32.0));
                            x += 42.0;
                        }
                    } else if matches!(st.filter, Filter::App(_)) {
                        b(&mut v, Btn::BackToApps, R::new(x, y, 36.0, 32.0));
                    } else {
                        let f = filters(st);
                        if f.len() > 1 {
                            for (k, t) in f {
                                let tw = g.measure(t, 13.0, 500) + 24.0;
                                b(&mut v, Btn::Filter(k), R::new(x, y, tw, 32.0));
                                x += tw + 5.0;
                            }
                        }
                    }
                }
                let apps = st.filter == Filter::Apps;
                let clip = Self::shelf_clip(h, w);
                for (id, r) in self.tile_rects(st, view, now) {
                    if r.y + r.h > clip.y && r.y < clip.y + clip.h {
                        if apps {
                            v.push(Spot { hit: Hit::AppFolder(id), r });
                        } else {
                            v.push(Spot { hit: Hit::Check(id), r: R::new(r.x + r.w - 30.0, r.y, 30.0, 30.0) });
                            v.push(Spot { hit: Hit::Tile(id), r });
                        }
                    }
                }
            }
            View::Menu => {
                let cw = (w - PAD * 2.0 - 8.0) / 2.0;
                for (n, (k, _)) in Self::menu(app.interval.is_some(), app.rec.is_some()).into_iter().enumerate() {
                    let (col, row) = (n / 7, n % 7);
                    b(&mut v, k, R::new(PAD + col as f32 * (cw + 8.0), PAD + row as f32 * 38.0, cw, 36.0));
                }
            }
            View::Settings => {
                let right = w - PAD;
                let row = |n: usize| PAD + n as f32 * 38.0 + 4.0;
                for n in 0..4 {
                    b(&mut v, Btn::Key(n), R::new(right - 150.0, row(n), 150.0, 30.0));
                }
                b(&mut v, Btn::Folder, R::new(right - 150.0, row(4), 150.0, 30.0));
                for n in 0..3 {
                    b(&mut v, Btn::Step(n, -1), R::new(right - 150.0, row(5 + n), 36.0, 30.0));
                    b(&mut v, Btn::Step(n, 1), R::new(right - 36.0, row(5 + n), 36.0, 30.0));
                }
                b(&mut v, Btn::History, R::new(right - 48.0, row(8), 48.0, 30.0));
                b(&mut v, Btn::Login, R::new(right - 48.0, row(9), 48.0, 30.0));
                let fy = h - PAD - 32.0;
                b(&mut v, Btn::Close, R::new(PAD, fy, 36.0, 32.0));
                b(&mut v, Btn::Quit, R::new(right - 68.0, fy, 68.0, 32.0));
            }
            View::Rec => b(&mut v, Btn::StopRec, R::new(0.0, 0.0, w, h)),
            _ => {}
        }
        v
    }

    fn hit(&self, app: &App, x: f32, y: f32) -> Option<Hit> {
        self.spots(app, self.view, Instant::now()).into_iter().find(|s| s.r.has(x, y)).map(|s| s.hit)
    }

    fn drop_at(&self, st: &Store) -> Option<Drop> {
        let d = self.drag.as_ref()?;
        if self.view == View::Shelf && st.filter == Filter::Apps {
            return None;
        }
        let (x, y) = self.mouse;
        let rects = self.tile_rects(st, self.view, Instant::now());
        let groups = groups(st, self.view);
        for (n, (id, r)) in rects.iter().enumerate() {
            if r.grow(GAP / 2.0).has(x, y) {
                if d.ids.contains(id) {
                    return None;
                }
                let k = (x - r.x) / r.w;
                if (0.25..0.75).contains(&k) {
                    return Some(Drop::Onto(*id));
                }
                let at = if k < 0.5 { n } else { n + 1 };
                return Some(Drop::Before(groups.get(at).map(|g| g[0])));
            }
        }
        None
    }

    // ── Drawing ────────────────────────────────────────────────────────────

    pub fn busy(&self, now: Instant) -> bool {
        !self.hidden
            && (self.dirty
                || self.w.busy(now)
                || self.h.busy(now)
                || self.rad.busy(now)
                || self.scroll.busy(now)
                || self.prev.is_some()
                || self.copied.is_some()
                || self.captured.is_some()
                || self.failed.is_some()
                || self.working
                || (now - self.swap).as_secs_f32() < 0.5
                || self.lift.values().any(|a| a.busy(now))
                || self.folder_peek.values().any(|a| a.busy(now)))
    }

    pub fn draw(&mut self, app: &App) -> Result<()> {
        let now = Instant::now();
        self.dirty = false;
        self.working = app.jobs > 0;
        let t = (now - self.swap).as_secs_f32();
        let (out, inn) = (t / 0.2, (t - 0.07) / 0.4);
        if out >= 1.0 {
            self.prev = None;
        }
        if self.copied.is_some_and(|c| (now - c.1).as_secs_f32() > 0.9) {
            self.copied = None;
        }
        if self.captured.is_some_and(|c| (now - c.1).as_secs_f32() > 1.45) {
            self.captured = None;
        }
        let (w, h, r) = (self.w.get(now).max(0.0), self.h.get(now).max(0.0), self.rad.get(now).max(0.0));
        let fail = self.failed.map(|f| (now - f).as_secs_f32() / 0.9).filter(|t| *t < 1.0);
        if fail.is_none() {
            self.failed = None;
        }
        let cap_t = self.captured.map(|(_, at)| (now - at).as_secs_f32());
        // Load whatever this frame needs before drawing.
        let need: Vec<i64> = match self.view {
            View::Flash(id) => vec![id],
            View::Shelf if app.store.filter == Filter::Apps => Vec::new(),
            v => self
                .tile_rects(&app.store, v, now)
                .into_iter()
                .filter(|(_, r)| r.y < WIN.1 && r.y + r.h > 0.0)
                .map(|t| t.0)
                .collect(),
        };
        let (view, prev, s) = (self.view, self.prev, self.s);
        let mut fresh: Vec<(i64, Frame)> = Vec::new();
        for id in need {
            if !self.thumbs.contains_key(&id) {
                if let Some(f) = app.store.thumb(id) {
                    fresh.push((id, f));
                }
            }
        }
        let mut loaded = Vec::new();
        let this: *const Notch = self;
        let res = self.surf.frame(&app.g, true, |c| {
            for (id, f) in &fresh {
                if let Ok(b) = c.upload(f) {
                    loaded.push((*id, b));
                }
            }
            // SAFETY: drawing only reads the notch; the surface borrow is the only other use.
            let me = unsafe { &*this };
            let base = Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: 0.0, M32: 0.0 };
            c.transform(&base);
            let island = R::new((WIN.0 - w) / 2.0, 0.0, w, h);
            if h > 0.5 {
                c.island(island, r, EAR);
                if let Some(ct) = cap_t.filter(|&ct| ct < 0.72) {
                    let p = ct / 0.72;
                    let pulse = (1.0 - p).powi(2);
                    c.island_rim(island, r, EAR, 1.8, BLUE, 0.75 * pulse);
                    c.island_rim(island, r, EAR, 0.9, INK, 0.45 * pulse);
                }
            }
            c.clip(island, || {
                let shifted = |v: View| {
                    let (tw, _, _) = size(&app.g, &app.store, v);
                    Matrix3x2 { M31: (WIN.0 - tw) / 2.0 * s, ..base }
                };
                if let (Some(p), true) = (prev, out < 1.0) {
                    let k = Curve::Standard.at(out);
                    c.transform(&shifted(p));
                    c.layer(9.0 * k, 1.0 - k, |c| me.content(c, app, p, now, &loaded, false));
                }
                if inn > 0.0 {
                    let k = Curve::Emphasized.at(inn);
                    c.transform(&shifted(view));
                    c.layer(10.0 * (1.0 - k), k, |c| me.content(c, app, view, now, &loaded, true));
                }
                c.transform(&base);
                if let Some(ct) = cap_t.filter(|&ct| ct < 0.58) {
                    let p = ct / 0.58;
                    let k = Curve::Emphasized.at(p);
                    let fade = (1.0 - p).powi(2);
                    let bw = (20.0 + (island.w - 48.0).max(16.0) * k).min(island.w - 24.0);
                    c.fill(R::new(island.cx() - bw / 2.0, island.y + island.h - 4.5, bw, 2.5), 1.25, BLUE, 0.85 * fade);
                    c.fill(R::new(island.cx() - bw * 0.3, island.y + island.h - 4.5, bw * 0.6, 2.5), 1.25, INK, 0.65 * fade);
                }
                if me.working {
                    let t = me.born.elapsed().as_secs_f32() * 0.9 % 1.0;
                    let k = (t * std::f32::consts::PI).sin();
                    let x = island.x + 16.0 + (island.w - 32.0 - 34.0) * Curve::Standard.at(t);
                    c.fill(R::new(x, island.y + island.h - 4.0, 34.0, 2.0), 1.0, INK, 0.8 * k);
                }
                if let Some(t) = fail {
                    let k = (t * std::f32::consts::PI * 2.0).sin().abs();
                    c.fill(R::new(island.cx() - 24.0, island.y + island.h - 5.0, 48.0, 3.0), 1.5, RED, k);
                }
            });
        });
        self.thumbs.extend(loaded);
        res
    }

    fn content(&self, c: &Canvas, app: &App, v: View, now: Instant, extra: &[(i64, ID2D1Bitmap1)], live: bool) {
        let (w, h, _) = size(&app.g, &app.store, v);
        let hover = |b: Btn| live && self.hover == Some(Hit::B(b));
        let down = |b: Btn| live && matches!(self.press, Some((Hit::B(x), ..)) if x == b);
        let spots = self.spots(app, v, now);
        let at = |b: Btn| spots.iter().find(|s| s.hit == Hit::B(b)).map(|s| s.r);
        let round = |b: Btn, glyph: u16, hex: u32| {
            if let Some(r) = at(b) {
                let r2 = if down(b) { r.scale(0.94) } else { r };
                c.fill(r2, r2.h / 2.0, if hover(b) { PILL_HOVER } else { PILL }, 1.0);
                let sz = (r2.h * 0.48).clamp(12.5, 15.0);
                c.icon(glyph, r2, sz, hex, 1.0);
            }
        };
        let pill = |b: Btn, text: &str, hex: u32| {
            if let Some(r) = at(b) {
                let r2 = if down(b) { r.scale(0.97) } else { r };
                c.fill(r2, r2.h / 2.0, if hover(b) { PILL_HOVER } else { PILL }, 1.0);
                c.text(text, r2, 13.0, hex, 1.0, Align::Center);
            }
        };
        match v {
            View::Rest => {
                if app.interval.is_some() {
                    c.circle(w / 2.0, h / 2.0, 2.2, RED, 1.0);
                }
                // One dot for each thing left to paste.
                let left = app.queue.len().min(14);
                let x0 = w / 2.0 - (left as f32 - 1.0) * 4.5;
                for k in 0..left {
                    c.circle(x0 + k as f32 * 9.0, h / 2.0, 2.2, INK, if k == 0 { 1.0 } else { 0.5 });
                }
            }
            View::Rec => {
                let s = self.rec.map_or(0, |t| (now - t).as_secs() as i64);
                let r = R::new(16.0, h / 2.0 - 6.0, 12.0, 12.0);
                if hover(Btn::StopRec) {
                    c.fill(r, 2.5, RED, 1.0);
                } else {
                    let t = self.rec.map_or(0.0, |t| (now - t).as_secs_f32());
                    c.circle(r.cx(), r.cy(), 5.0, RED, 0.65 + 0.35 * (t * std::f32::consts::PI).cos().abs());
                }
                c.text_w(&clock(s), R::new(0.0, 0.0, w - 18.0, h), 14.0, 600, INK, 1.0, Align::Right);
            }
            View::Flash(id) => {
                let dy = match self.captured {
                    Some((cid, at)) if cid == id => {
                        let k = Curve::Spring.at(((now - at).as_secs_f32() / 0.46).min(1.0));
                        -8.0 * (1.0 - k)
                    }
                    _ => 0.0,
                };
                self.tile(c, app, id, R::new(PAD, PAD + dy, TILE.0, TILE.1), now, extra, false, 1.0);
            }
            View::Count(n) => {
                c.text_w(&n.to_string(), R::new(0.0, 0.0, w, h), 22.0, 600, INK, 1.0, Align::Center);
            }
            View::Done => c.icon(glyph::CHECK, R::new(0.0, 0.0, w, h), 18.0, INK, 1.0),
            View::Peek => {
                for (id, r) in self.tile_rects(&app.store, v, now) {
                    self.tile(c, app, id, r, now, extra, false, 1.0);
                }
                round(Btn::Capture, glyph::ADD, INK);
                round(Btn::Open, glyph::DOWN, INK);
                round(Btn::Settings, glyph::GEAR, INK);
                self.drag_layer(c, app, v, now, extra, live);
            }
            View::Shelf => {
                let clip = Self::shelf_clip(h, w);
                c.clip(clip, || {
                    let app_folders = app.store.filter == Filter::Apps;
                    let rects = self.tile_rects(&app.store, v, now);
                    if app_folders {
                        let folders = app.store.shelf();
                        for ((id, r), folder) in rects.into_iter().zip(folders) {
                            if r.y + r.h > clip.y && r.y < clip.y + clip.h {
                                self.app_folder(c, app, (id, folder.len()), r, now, live);
                            }
                        }
                    } else {
                        if rects.is_empty() && matches!(app.store.filter, Filter::App(_)) {
                            c.text("No captures in this folder", clip, 14.0, DIM, 1.0, Align::Center);
                        }
                        for (id, r) in rects {
                            if r.y + r.h > clip.y - 8.0 && r.y < clip.y + clip.h {
                                self.tile(c, app, id, r, now, extra, true, 1.0);
                            }
                        }
                    }
                });
                let m = self.max_scroll(&app.store);
                if m > 0.0 && live {
                    let track = clip.h - 24.0;
                    let th = track * clip.h / (clip.h + m);
                    let y = clip.y + 12.0 + (track - th) * (self.scroll.get(now) / m);
                    c.fill(R::new(w - 8.0, y, 3.0, th), 1.5, INK, 0.22);
                }
                round(Btn::Settings, glyph::GEAR, INK);
                round(Btn::Capture, glyph::ADD, INK);
                round(Btn::Close, glyph::UP, INK);
                if live {
                    let pinned = self.sel.iter().all(|i| app.store.get(*i).is_some_and(|i| i.pinned));
                    round(Btn::Copy, glyph::COPY, INK);
                    round(Btn::Pin, if pinned { glyph::UNPIN } else { glyph::PIN }, INK);
                    round(Btn::Delete, glyph::TRASH, if hover(Btn::Delete) { RED } else { INK });
                    round(Btn::More, if self.more { glyph::BACK } else { glyph::NEXT }, INK);
                    for (b, t, _) in self.actions(&app.g, &app.store) {
                        pill(b, t, INK);
                    }
                }
                if let (Filter::App(name), true) = (&app.store.filter, self.sel.is_empty()) {
                    round(Btn::BackToApps, glyph::BACK, INK);
                    if let Some(r) = at(Btn::BackToApps) {
                        let name = fitted(&app.g, folder_name(name), 13.0, 260.0);
                        c.text(&name, R::new(r.x + 46.0, r.y, 260.0, r.h), 13.0, INK, 1.0, Align::Left);
                    }
                } else {
                    let now_on = match app.store.filter {
                        Filter::All => 0,
                        Filter::Clips => 1,
                        _ => 2,
                    };
                    for (k, t) in filters(&app.store) {
                        if let Some(r) = at(Btn::Filter(k)) {
                            let b = Btn::Filter(k);
                            let r2 = if down(b) { r.scale(0.97) } else { r };
                            let bg = if k == now_on { INK } else if hover(b) { PILL_HOVER } else { PILL };
                            c.fill(r2, r2.h / 2.0, bg, 1.0);
                            c.text(t, r2, 13.0, if k == now_on { BLACK } else { INK }, 1.0, Align::Center);
                        }
                    }
                }
                self.drag_layer(c, app, v, now, extra, live);
            }
            View::Menu => {
                for (b, t) in Self::menu(app.interval.is_some(), app.rec.is_some()) {
                    if let Some(r) = at(b) {
                        if hover(b) {
                            c.fill(r, 12.0, TAB, 1.0);
                        }
                        let hex = if matches!(b, Btn::Settings | Btn::Quit) { DIM } else { INK };
                        c.text(t, R::new(r.x + 14.0, r.y, r.w, r.h), 14.0, if matches!(b, Btn::Stop | Btn::StopRec) { RED } else { hex }, 1.0, Align::Left);
                        let key = match b {
                            Btn::Go(Mode::Region) => app.store.settings.region,
                            Btn::Go(Mode::Screen) => app.store.settings.screen,
                            Btn::Go(Mode::Window) => app.store.settings.window,
                            Btn::Go(Mode::Clip) => app.store.settings.clip,
                            _ => continue,
                        };
                        if key.1 != 0 {
                            c.text(&key_name(key), R::new(r.x, r.y, r.w - 14.0, r.h), 12.0, DIM, 1.0, Align::Right);
                        }
                    }
                }
            }
            View::Settings => {
                let s = &app.store.settings;
                let rows = [
                    "Region",
                    "Screen",
                    "Window",
                    "Clip",
                    "Folder",
                    "Keep",
                    "Delay",
                    "Interval",
                    "Copied images",
                    "Launch at login",
                ];
                for (n, t) in rows.iter().enumerate() {
                    let y = PAD + n as f32 * 38.0;
                    c.text(t, R::new(PAD + 6.0, y, 200.0, 38.0), 14.0, INK, 1.0, Align::Left);
                }
                for (n, k) in s.keys().into_iter().enumerate() {
                    let name = if self.recording == Some(n) { "Press keys".into() } else { key_name(k) };
                    let hex = if self.recording == Some(n) {
                        BLUE
                    } else if app.hot[n] || k.1 == 0 {
                        INK
                    } else {
                        RED
                    };
                    pill(Btn::Key(n), &name, hex);
                }
                let folder = s.folder.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                let folder: String = if folder.chars().count() > 18 {
                    folder.chars().take(17).chain(['…']).collect()
                } else {
                    folder
                };
                pill(Btn::Folder, &folder, INK);
                let vals = [plural(s.keep_days, "day"), format!("{} s", s.delay), span(s.interval)];
                for (n, t) in vals.iter().enumerate() {
                    if let (Some(a), Some(b)) = (at(Btn::Step(n, -1)), at(Btn::Step(n, 1))) {
                        c.text(t, R::new(a.x + a.w, a.y, b.x - a.x - a.w, a.h), 13.0, INK, 1.0, Align::Center);
                    }
                    for d in [-1, 1] {
                        if let Some(r) = at(Btn::Step(n, d)) {
                            c.fill(r, 15.0, if hover(Btn::Step(n, d)) { PILL_HOVER } else { PILL }, 1.0);
                            c.icon(if d < 0 { 0xE738 } else { glyph::ADD }, r, 12.0, INK, 1.0);
                        }
                    }
                }
                for (b, on) in [(Btn::History, s.history), (Btn::Login, app.login)] {
                    let Some(r) = at(b) else { continue };
                    let t = R::new(r.x, r.y + 2.0, 46.0, 26.0);
                    c.fill(t, 13.0, if on { INK } else { PILL_HOVER }, 1.0);
                    c.circle(if on { t.x + 33.0 } else { t.x + 13.0 }, t.cy(), 9.5, if on { BLACK } else { DIM }, 1.0);
                }
                round(Btn::Close, glyph::BACK, INK);
                pill(Btn::Quit, "Quit", if hover(Btn::Quit) { RED } else { DIM });
            }
        }
    }

    fn drag_layer(&self, c: &Canvas, app: &App, v: View, now: Instant, extra: &[(i64, ID2D1Bitmap1)], live: bool) {
        let (Some(d), true) = (&self.drag, live) else { return };
        let rects = self.tile_rects(&app.store, v, now);
        match self.drop_at(&app.store) {
            Some(Drop::Onto(id)) => {
                if let Some((_, r)) = rects.iter().find(|r| r.0 == id) {
                    c.stroke(r.grow(3.0), 13.0, 2.0, BLUE, 1.0);
                }
            }
            Some(Drop::Before(b)) => {
                let r = match b {
                    Some(id) => rects.iter().find(|r| r.0 == id).map(|r| (r.1.x - GAP / 2.0, r.1)),
                    None => rects.last().map(|r| (r.1.x + r.1.w + GAP / 2.0, r.1)),
                };
                if let Some((x, r)) = r {
                    c.fill(R::new(x - 1.0, r.y + 4.0, 2.0, r.h - 8.0), 1.0, BLUE, 1.0);
                }
            }
            None => {}
        }
        let r = R::new(d.at.0 - d.grab.0, d.at.1 - d.grab.1, TILE.0, TILE.1).scale(0.92);
        if let Some(b) = self.bitmap(d.ids[0], extra) {
            c.image_round(b, r, 10.0, 0.92);
        }
    }

    fn app_folder(&self, c: &Canvas, app: &App, folder: (i64, usize), r: R, now: Instant, live: bool) {
        let (id, count) = folder;
        let Some(item) = app.store.get(id) else { return };
        let hit = Hit::AppFolder(id);
        let pressed = live && self.press.is_some_and(|(h, ..)| h == hit);
        let peek = if live { self.folder_peek.get(&id).map_or(0.0, |a| a.get(now)) } else { 0.0 };
        let r = if pressed { r.scale(0.97) } else { r };

        c.fill(r, 12.0, 0x16181d, 1.0);
        c.fill(r, 12.0, 0x252b34, 0.78 * peek);
        c.stroke(r.grow(-0.5), 11.5, 1.0, INK, 0.09 + 0.09 * peek);

        let (back, face, edge) = folder_palette(&item.app);
        let x = r.cx() - 34.0;
        let y = r.y;
        // The tab belongs to the back wall, with papers drawn between it and the front flap.
        c.fill(R::new(x + 1.0, y + 30.0, 67.0, 24.0), 6.0, BLACK, 0.36);
        c.fill(R::new(x + 1.0, y + 22.0, 66.0, 28.0), 5.5, back, 1.0);
        c.fill(R::new(x + 2.0, y + 17.0, 24.0, 13.0), 4.0, back, 1.0);
        c.line((x + 6.0, y + 18.0), (x + 21.0, y + 18.0), 1.0, edge, 0.55);
        let wings: &[f32] = match count {
            0 => &[],
            1 => &[0.0],
            2 => &[-0.65, 0.65],
            _ => &[-1.0, 1.0, 0.0],
        };
        for (layer, &wing) in wings.iter().enumerate() {
            let paper = if layer + 1 == wings.len() { 0xd9e1e9 } else { 0x9cabc0 };
            let (sheet, angle) = folder_sheet_pose(r, wing, peek);
            draw_folder_sheet(c, sheet, angle, paper);
        }

        let flap = R::new(x, y + 31.0 + 2.0 * peek, 68.0, 20.0 - 2.0 * peek);
        c.fill(flap, 5.5, face, 1.0);
        c.stroke(flap.grow(-0.5), 5.0, 1.0, edge, 0.68);
        c.line(
            (flap.x + 6.0, flap.y + 2.0),
            (flap.x + flap.w - 6.0, flap.y + 2.0),
            1.0, INK, 0.16 + 0.13 * peek,
        );

        let name = fitted(&app.g, folder_name(&item.app), 13.0, r.w - 20.0);
        c.text(&name, R::new(r.x + 10.0, y + 53.0, r.w - 20.0, 17.0), 13.0, INK, 1.0, Align::Center);
        let item_count = if count == 1 { "1 capture".to_string() } else { format!("{count} captures") };
        c.text(&item_count, R::new(r.x + 10.0, y + 69.0, r.w - 20.0, 12.0), 10.5, DIM, 1.0, Align::Center);
        if peek > 0.01 {
            c.icon(glyph::NEXT, R::new(r.x + r.w - 24.0, y + 13.0, 15.0, 22.0), 11.0, INK, 0.66 * peek);
        }
    }

    fn bitmap<'a>(&'a self, id: i64, extra: &'a [(i64, ID2D1Bitmap1)]) -> Option<&'a ID2D1Bitmap1> {
        self.thumbs.get(&id).or_else(|| extra.iter().find(|e| e.0 == id).map(|e| &e.1))
    }

    #[allow(clippy::too_many_arguments)]
    fn tile(
        &self,
        c: &Canvas,
        app: &App,
        id: i64,
        r: R,
        now: Instant,
        extra: &[(i64, ID2D1Bitmap1)],
        checks: bool,
        alpha: f32,
    ) {
        let Some(item) = app.store.get(id) else { return };
        let group = group_of(&app.store, self.view, id);
        let lifted = self.lift.get(&id).map(|a| a.get(now)).unwrap_or(0.0);
        let pressed = matches!(self.press, Some((Hit::Tile(p), ..)) if p == id);
        let dragged = self.drag.as_ref().is_some_and(|d| d.ids.contains(&id));
        let cap_age = self.captured.and_then(|(cid, at)| (cid == id).then(|| (now - at).as_secs_f32()));
        let pop = cap_age.map_or(1.0, |ct| 0.86 + 0.14 * Curve::Spring.at((ct / 0.44).min(1.0)));
        let r = r.scale((1.0 + 0.025 * lifted - if pressed { 0.035 } else { 0.0 }) * pop);
        let a = alpha * if dragged { 0.3 } else { 1.0 };
        if group.len() > 1 {
            c.fill(R::new(r.x + 14.0, r.y - 7.0, r.w - 28.0, 30.0), 8.0, 0x2a2c31, a);
            c.fill(R::new(r.x + 7.0, r.y - 3.5, r.w - 14.0, 30.0), 9.0, 0x3a3d44, a);
        }
        c.fill(r, 10.0, CARD, a);
        if let Some(b) = self.bitmap(id, extra) {
            c.image_round(b, r, 10.0, a);
        }
        c.stroke(r.grow(-0.5), 9.5, 1.0, INK, 0.10 * a);
        if item.kind == Kind::Pdf {
            c.fill(r, 10.0, BLACK, 0.55 * a);
            c.text_w("PDF", r, 17.0, 600, INK, a, Align::Center);
        }
        if matches!(item.kind, Kind::Clip | Kind::Gif) {
            let t = if item.kind == Kind::Gif { "GIF".into() } else { clock(item.secs) };
            let p = R::new(r.x + 6.0, r.y + 6.0, app.g.measure(&t, 11.5, 600) + 14.0, 19.0);
            c.fill(p, 9.5, BLACK, 0.78 * a);
            c.text_w(&t, p, 11.5, 600, INK, a, Align::Center);
        }
        if group.iter().any(|g| app.clip.contains(g)) {
            // Outside the tile, so it shows on white shots too.
            c.stroke(r.grow(3.0), 13.0, 2.0, INK, a);
        }
        if item.pinned {
            let p = R::new(r.x + 6.0, r.y + r.h - 26.0, 20.0, 20.0);
            c.circle(p.cx(), p.cy(), 10.0, BLACK, 0.72 * a);
            c.icon(glyph::PINNED, p, 10.0, INK, a);
        }
        if group.len() > 1 {
            let p = R::new(r.x + r.w - 28.0, r.y + r.h - 26.0, 22.0, 20.0);
            c.fill(p, 10.0, BLACK, 0.78 * a);
            c.text_w(&group.len().to_string(), p, 12.0, 600, INK, a, Align::Center);
        }
        if let Some((cid, at)) = self.copied {
            if cid == id {
                let t = (now - at).as_secs_f32();
                let k = Curve::Emphasized.at(t / 0.22) * (1.0 - Curve::Standard.at((t - 0.55) / 0.3));
                c.fill(r, 10.0, BLACK, 0.5 * k * a);
                c.icon(glyph::CHECK, r, 16.0 + 6.0 * k, INK, k * a);
            }
        }
        if let Some(ct) = cap_age {
            // Brief camera-shutter flash over the newly captured frame.
            if ct < 0.32 {
                let flash = (1.0 - ct / 0.32).powi(2);
                c.fill(r, 10.0, INK, 0.48 * flash * a);
            }
            // Emerald check pill springs in to confirm "captured & copied".
            if ct > 0.10 && ct < 1.38 {
                let enter = Curve::Spring.at(((ct - 0.10) / 0.30).min(1.0));
                let exit = if ct > 1.12 { 1.0 - Curve::Standard.at((ct - 1.12) / 0.26) } else { 1.0 };
                let k = (enter * exit).max(0.0);
                let rad = 10.5 * k;
                let (cx, cy) = (r.x + r.w - 15.0, r.y + r.h - 15.0);
                c.circle(cx, cy, rad + 1.5, BLACK, 0.88 * exit * a);
                c.ring(cx, cy, rad, 1.3, GREEN, 0.9 * exit * a);
                c.icon(glyph::CHECK, R::new(cx - rad, cy - rad, rad * 2.0, rad * 2.0), 10.0 * k.min(1.0), GREEN, exit * a);
            }
        }
        if checks {
            let on = group.iter().all(|g| self.sel.contains(g));
            let show = on || !self.sel.is_empty() || lifted > 0.01;
            if show {
                let k = if on || !self.sel.is_empty() { 1.0 } else { lifted };
                let (x, y) = (r.x + r.w - 16.0, r.y + 16.0);
                if on {
                    c.circle(x, y, 12.0, BLACK, 1.0);
                }
                c.circle(x, y, 10.0, if on { INK } else { BLACK }, if on { 1.0 } else { 0.6 * k });
                c.ring(x, y, 10.0, 1.2, INK, if on { 1.0 } else { 0.7 * k });
                if on {
                    let n = self.order(app, id);
                    c.text_w(&n.to_string(), R::new(x - 10.0, y - 10.0, 20.0, 20.0), 12.0, 600, BLACK, 1.0, Align::Center);
                }
            }
        }
    }

    /// 1-based place of a tile in the selection.
    fn order(&self, app: &App, id: i64) -> usize {
        let groups = groups(&app.store, self.view);
        let head = |i: i64| groups.iter().find(|g| g.contains(&i)).map(|g| g[0]).unwrap_or(i);
        let mut seen: Vec<i64> = Vec::new();
        for i in &self.sel {
            let h = head(*i);
            if !seen.contains(&h) {
                seen.push(h);
            }
        }
        seen.iter().position(|h| *h == id).map(|n| n + 1).unwrap_or(0)
    }

    // ── Mouse ──────────────────────────────────────────────────────────────

    fn local(&self, l: LPARAM) -> (f32, f32) {
        let x = (l.0 & 0xffff) as i16 as f32 / self.s;
        let y = ((l.0 >> 16) & 0xffff) as i16 as f32 / self.s;
        (x, y)
    }

    pub fn in_island(&self) -> bool {
        if testing() {
            let (x, y) = self.mouse;
            return x >= 0.0 && y >= 0.0 && x < self.w.to && y < self.h.to;
        }
        crate::capture::inside(&self.island_px(), cursor())
    }

    /// True when the notch should consume this keyboard shortcut instead of passing it behind.
    pub fn wants_key(&self, app: &App, mods: u32, vk: u32) -> bool {
        if !self.expanded() || mods & (MOD_ALT.0 | MOD_WIN.0) != 0 {
            return false;
        }
        let ctrl = mods & MOD_CONTROL.0 != 0;
        let shift = mods & MOD_SHIFT.0 != 0;
        let key = VIRTUAL_KEY(vk as u16);
        match (ctrl, shift, key) {
            (false, false, VK_ESCAPE) => true,
            (true, false, VK_A) => match self.view {
                View::Peek => !app.store.items.is_empty(),
                View::Shelf => app.store.filter != Filter::Apps && !app.store.shelf().is_empty(),
                _ => false,
            },
            (true, false, VK_C) => {
                !self.sel.is_empty() || matches!(self.hover, Some(Hit::Tile(_) | Hit::Check(_)))
            }
            (true, false, VK_D) => !self.sel.is_empty(),
            (true, false, VK_OEM_COMMA) => true,
            (false, false, VK_DELETE) => self.view == View::Shelf && !self.sel.is_empty(),
            (false, false, VK_BACK) => self.view == View::Shelf
                && (!self.sel.is_empty() || matches!(app.store.filter, Filter::App(_))),
            (false, false, VK_P) => self.view == View::Shelf && !self.sel.is_empty(),
            _ => false,
        }
    }

    /// Handle a keyboard shortcut while the notch is expanded.
    pub fn key(&mut self, app: &mut App, mods: u32, vk: u32) {
        if !self.wants_key(app, mods, vk) {
            return;
        }
        let ctrl = mods & MOD_CONTROL.0 != 0;
        let key = VIRTUAL_KEY(vk as u16);
        match (ctrl, key) {
            (true, VK_A) => {
                if self.view == View::Peek {
                    app.store.filter = Filter::All;
                    self.show(&app.g, &app.store, View::Shelf);
                }
                self.sel = groups(&app.store, View::Shelf).into_iter().flatten().collect();
                self.more = false;
                self.dirty = true;
            }
            (true, VK_C) => {
                if !self.sel.is_empty() {
                    self.button(app, Btn::Copy);
                } else if let Some(Hit::Tile(id) | Hit::Check(id)) = self.hover {
                    let ids = group_of(&app.store, self.view, id);
                    if app.copy(&ids) {
                        self.mark_copied(id);
                    }
                }
            }
            (true, VK_D) => {
                self.sel.clear();
                self.more = false;
                self.dirty = true;
            }
            (true, VK_OEM_COMMA) => {
                let next = if self.view == View::Settings { View::Peek } else { View::Settings };
                self.show(&app.g, &app.store, next);
            }
            (false, VK_BACK) if matches!(app.store.filter, Filter::App(_)) => {
                if self.sel.is_empty() {
                    self.back_to_apps(app);
                } else {
                    self.sel.clear();
                    self.more = false;
                    self.dirty = true;
                }
            }
            (false, VK_DELETE | VK_BACK) => self.button(app, Btn::Delete),
            (false, VK_P) => self.button(app, Btn::Pin),
            (false, VK_ESCAPE) => {
                if self.more {
                    self.more = false;
                } else if !self.sel.is_empty() {
                    self.sel.clear();
                } else if matches!(self.view, View::Settings | View::Menu) {
                    self.show(&app.g, &app.store, View::Peek);
                } else if self.view == View::Shelf && matches!(app.store.filter, Filter::App(_)) {
                    self.back_to_apps(app);
                } else if self.view == View::Shelf {
                    self.show(&app.g, &app.store, View::Peek);
                } else {
                    self.show(&app.g, &app.store, View::Rest);
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn set_hover(&mut self, h: Option<Hit>) {
        if self.hover == h {
            return;
        }
        let tile = |h: Option<Hit>| match h {
            Some(Hit::Tile(i) | Hit::Check(i)) => Some(i),
            _ => None,
        };
        let (a, b) = (tile(self.hover), tile(h));
        if a != b {
            if let Some(i) = a {
                self.lift.entry(i).or_insert(Anim::new(1.0)).go(0.0, 0.22, Curve::Standard);
            }
            if let Some(i) = b {
                self.lift.entry(i).or_insert(Anim::new(0.0)).go(1.0, 0.22, Curve::Emphasized);
            }
        }
        if let Some(Hit::AppFolder(id)) = self.hover {
            if let Some(peek) = self.folder_peek.get_mut(&id) {
                peek.go(0.0, 0.18, Curve::Standard);
            }
        }
        if let Some(Hit::AppFolder(id)) = h {
            self.folder_peek.entry(id).or_insert(Anim::new(0.0)).go(1.0, 0.25, Curve::Emphasized);
        }
        self.hover = h;
        self.dirty = true;
    }

    fn track_leave(&mut self) {
        if self.tracking || testing() {
            return;
        }
        let mut t = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.sensor,
            dwHoverTime: 0,
        };
        self.tracking = unsafe { TrackMouseEvent(&mut t) }.is_ok();
    }

    fn cancel_open(&mut self) {
        if self.open_dwell.cancel() {
            unsafe { let _ = KillTimer(Some(self.sensor), T_DWELL); }
        }
    }

    fn cancel_close(&mut self) {
        if self.close_grace.cancel() {
            unsafe { let _ = KillTimer(Some(self.sensor), T_LEAVE); }
        }
    }

    fn can_hover_open(&self) -> bool {
        !self.hidden && matches!(self.view, View::Rest | View::Flash(_))
    }

    fn pointer_in_open_zone(&self) -> bool {
        let (x, y) = if testing() {
            self.mouse
        } else {
            let p = cursor();
            let r = self.island_px();
            ((p.x - r.left) as f32 / self.s, (p.y - r.top) as f32 / self.s)
        };
        in_hover_zone(x, y, self.w.to)
    }

    fn pointer_button_held() -> bool {
        !testing() && [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0)
    }

    fn sync_hover_open(&mut self) {
        if !self.can_hover_open() || !self.pointer_in_open_zone() || Self::pointer_button_held() {
            self.cancel_open();
            return;
        }
        if self.open_dwell.begin(Instant::now(), OPEN_MS)
            && unsafe { SetTimer(Some(self.sensor), T_DWELL, OPEN_MS, None) } == 0
        {
            self.open_dwell.cancel();
        }
    }

    fn can_auto_close(&self) -> bool {
        !self.hidden
            && self.expanded()
            && !self.in_island()
            && self.press.is_none()
            && self.drag.is_none()
            && !self.modal
            && self.recording.is_none()
    }

    /// Resume or pause the leave grace after capture, a modal, or hotkey editing.
    pub fn sync_auto_close(&mut self) {
        if !self.can_auto_close() {
            self.cancel_close();
            return;
        }
        let delay = if matches!(self.view, View::Menu | View::Settings) || self.more { PANEL_CLOSE_MS } else { CLOSE_MS };
        if self.close_grace.begin(Instant::now(), delay) &&
            unsafe { SetTimer(Some(self.sensor), T_LEAVE, delay, None) } == 0
        {
            self.close_grace.cancel();
        }
    }

    pub fn mouse_move(&mut self, app: &mut App, l: LPARAM) {
        self.mouse = self.local(l);
        self.track_leave();
        self.sync_auto_close();
        let (x, y) = self.mouse;
        match self.view {
            View::Rest | View::Flash(_) | View::Done => {
                self.sync_hover_open();
                return;
            }
            View::Count(_) => return,
            _ => {}
        }
        if self.view == View::Rec {
            let h = self.hit(app, x, y);
            self.set_hover(h);
            return;
        }
        if let Some(d) = &mut self.drag {
            d.at = (x, y);
            self.dirty = true;
            if !self.in_island() {
                let ids = self.drag.take().unwrap().ids;
                self.press = None;
                app.outgoing = ids;
                unsafe {
                    let _ = ReleaseCapture();
                    let _ = PostMessageW(Some(self.sensor), WM_DRAG_OUT, WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        if let Some((Hit::Tile(id), px, py)) = self.press {
            if (x - px).hypot(y - py) > 6.0 {
                let mut ids = group_of(&app.store, self.view, id);
                if ids.iter().all(|i| self.sel.contains(i)) {
                    ids = self.sel.clone();
                    // Keep the grabbed tile on top.
                    ids.sort_by_key(|i| *i != id);
                }
                let r = self.tile_rects(&app.store, self.view, Instant::now()).into_iter().find(|t| t.0 == id).map(|t| t.1);
                let r = r.unwrap_or_default();
                self.drag = Some(Drag { ids, grab: (px - r.x, py - r.y), at: (x, y) });
                self.dirty = true;
                return;
            }
        }
        let h = self.hit(app, x, y);
        self.set_hover(h);
    }

    pub fn mouse_leave(&mut self) {
        self.tracking = false;
        self.cancel_open();
        if testing() {
            // The posted message driver intentionally leaves the OS pointer alone.
            self.mouse = (-1.0, -1.0);
        } else if self.in_island() {
            // A queued leave can predate re-entry or the sensor's resize.
            self.track_leave();
            self.sync_hover_open();
            self.sync_auto_close();
            return;
        }
        self.set_hover(None);
        self.sync_auto_close();
    }

    /// Report a real automatic close so the app can schedule thumbnail trimming.
    pub fn timer(&mut self, app: &App, id: usize) -> bool {
        unsafe { let _ = KillTimer(Some(self.sensor), id); }
        let transition = match id {
            T_DWELL => self.open_dwell.poll(Instant::now()),
            T_LEAVE => self.close_grace.poll(Instant::now()),
            T_FLASH if matches!(self.view, View::Flash(_) | View::Done) => {
                self.show(&app.g, &app.store, View::Rest);
                return false;
            }
            _ => return false,
        };
        match transition {
            Wake::Early(wait) => {
                if unsafe { SetTimer(Some(self.sensor), id, wait, None) } == 0 {
                    if id == T_DWELL {
                        self.open_dwell.cancel();
                    } else {
                        self.close_grace.cancel();
                    }
                }
            }
            Wake::Ready if id == T_DWELL && self.can_hover_open() && self.pointer_in_open_zone() && !Self::pointer_button_held() => {
                self.show(&app.g, &app.store, View::Peek);
            }
            Wake::Ready if id == T_LEAVE && self.can_auto_close() => {
                self.show(&app.g, &app.store, View::Rest);
                return true;
            }
            _ => {}
        }
        false
    }

    /// A lost Win32 capture must not leave a phantom press blocking auto-close.
    fn capture_lost(&mut self) {
        let press = self.press.take().is_some();
        let drag = self.drag.take().is_some();
        if press || drag {
            self.dirty = true;
            self.sync_auto_close();
        }
    }

    pub fn down(&mut self, app: &App, l: LPARAM) {
        self.just_opened_app = false;
        self.mouse = self.local(l);
        self.cancel_open();
        if !self.expanded() && self.view != View::Rec {
            if matches!(self.view, View::Rest | View::Flash(_)) {
                self.show(&app.g, &app.store, View::Peek);
            }
            return;
        }
        let (x, y) = self.mouse;
        if let Some(h) = self.hit(app, x, y) {
            self.press = Some((h, x, y));
            self.cancel_close();
            unsafe {
                SetCapture(self.sensor);
            }
        }
        self.dirty = true;
    }

    pub fn up(&mut self, app: &mut App, l: LPARAM) {
        self.mouse = self.local(l);
        let press = self.press.take();
        let drop = self.drop_at(&app.store);
        let drag = self.drag.take();
        unsafe { let _ = ReleaseCapture(); }
        self.dirty = true;
        self.sync_auto_close();
        if let Some(d) = drag {
            match drop {
                Some(Drop::Onto(id)) => {
                    let mut ids = group_of(&app.store, self.view, id);
                    let key = app.store.get(id).map(|i| i.stack).filter(|s| *s != 0).unwrap_or(id);
                    ids.extend(d.ids.iter().filter(|i| **i != id));
                    app.store.stack(&ids, key);
                }
                Some(Drop::Before(b)) => app.store.reorder(&d.ids, b),
                None => {}
            }
            self.refresh(&app.g, &app.store);
            return;
        }
        let (x, y) = self.mouse;
        let Some((h, ..)) = press else { return };
        if self.hit(app, x, y) != Some(h) {
            return;
        }
        match h {
            Hit::AppFolder(id) => {
                if let Some(a) = app.store.get(id).map(|i| i.app.trim().to_owned()) {
                    self.apps_scroll = self.scroll.to;
                    app.store.filter = Filter::App(a);
                    self.just_opened_app = true;
                    self.scroll.jump(0.0);
                    self.refresh(&app.g, &app.store);
                }
            }
            Hit::Tile(id) | Hit::Check(id) => {
                let ids = group_of(&app.store, self.view, id);
                let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
                let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                if shift && self.view == View::Shelf && !self.sel.is_empty() {
                    let gs = groups(&app.store, View::Shelf);
                    let anchor = *self.sel.last().unwrap();
                    if let (Some(a), Some(b)) = (
                        gs.iter().position(|g| g.contains(&anchor)),
                        gs.iter().position(|g| g.contains(&id)),
                    ) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        for g in &gs[lo..=hi] {
                            for &i in g {
                                if !self.sel.contains(&i) {
                                    self.sel.push(i);
                                }
                            }
                        }
                    }
                } else if matches!(h, Hit::Check(_)) || ctrl {
                    if ids.iter().all(|i| self.sel.contains(i)) {
                        self.sel.retain(|i| !ids.contains(i));
                    } else {
                        self.sel.retain(|i| !ids.contains(i));
                        self.sel.extend(ids);
                    }
                    if self.sel.is_empty() {
                        self.more = false;
                    }
                } else if app.copy(&ids) {
                    self.mark_copied(id);
                }
            }
            Hit::B(b) => self.button(app, b),
        }
    }

    pub fn double(&mut self, app: &App, l: LPARAM) {
        // Windows sends a double-click instead of the second down. The first
        // click opened the folder, so don't open the newly exposed shot too.
        if std::mem::take(&mut self.just_opened_app) {
            return;
        }
        let (x, y) = self.local(l);
        match self.hit(app, x, y) {
            Some(Hit::Tile(id)) => {
                if let Some(p) = app.file(id) {
                    crate::clip::open_file(&p);
                }
            }
            _ => self.down(app, l),
        }
    }

    pub fn right(&mut self, app: &App, l: LPARAM) {
        self.mouse = self.local(l);
        self.show(&app.g, &app.store, View::Menu);
    }

    pub fn wheel(&mut self, app: &mut App, delta: f32) {
        match self.view {
            View::Peek if delta < 0.0 && !app.store.items.is_empty() => {
                app.store.filter = Filter::All;
                self.show(&app.g, &app.store, View::Shelf);
            }
            View::Shelf => {
                let to = (self.scroll.to - delta / 120.0 * (TILE.1 + GAP)).clamp(0.0, self.max_scroll(&app.store));
                self.scroll.go(to, 0.35, Curve::Emphasized);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn back_to_apps(&mut self, app: &mut App) {
        app.store.filter = Filter::Apps;
        self.scroll.jump(self.apps_scroll.min(self.max_scroll(&app.store)));
        self.refresh(&app.g, &app.store);
    }

    fn button(&mut self, app: &mut App, b: Btn) {
        match b {
            Btn::Capture => self.show(&app.g, &app.store, View::Menu),
            Btn::Open => {
                app.store.filter = Filter::All;
                self.show(&app.g, &app.store, View::Shelf);
            }
            Btn::BackToApps => self.back_to_apps(app),
            Btn::Filter(n) => {
                app.store.filter = [Filter::All, Filter::Clips, Filter::Apps][n].clone();
                self.scroll.jump(0.0);
                self.refresh(&app.g, &app.store);
            }
            Btn::Close => self.show(&app.g, &app.store, View::Peek),
            Btn::Settings => self.show(&app.g, &app.store, View::Settings),
            Btn::More => self.more = !self.more,
            Btn::Copy => {
                if app.copy(&self.sel.clone()) {
                    self.mark_copied(self.sel[0]);
                }
            }
            Btn::Pin => {
                let on = !self.sel.iter().all(|i| app.store.get(*i).is_some_and(|i| i.pinned));
                app.store.pin(&self.sel, on);
                self.sel.clear();
                self.refresh(&app.g, &app.store);
            }
            Btn::Delete => {
                let files = app.store.remove(&std::mem::take(&mut self.sel));
                app.clip.retain(|i| app.store.get(*i).is_some());
                self.more = false;
                self.refresh(&app.g, &app.store);
                std::thread::spawn(move || crate::clip::recycle(&files));
            }
            Btn::Merge => {
                let ids = std::mem::take(&mut self.sel);
                self.more = false;
                if let Some(id) = app.merge(&ids) {
                    self.scroll.go(0.0, 0.35, Curve::Emphasized);
                    self.refresh(&app.g, &app.store);
                    self.mark_copied(id);
                }
            }
            Btn::Backdrop => {
                let ids = std::mem::take(&mut self.sel);
                self.more = false;
                match app.backdrop(&ids) {
                    Some(id) => {
                        self.scroll.go(0.0, 0.35, Curve::Emphasized);
                        self.refresh(&app.g, &app.store);
                        self.mark_copied(id);
                    }
                    None => self.fail(),
                }
            }
            Btn::Float => {
                let ids = std::mem::take(&mut self.sel);
                let mon = self.mon;
                self.show(&app.g, &app.store, View::Rest);
                if !app.float(&ids, &mon) {
                    self.fail();
                }
            }
            Btn::Gif => {
                let ids = std::mem::take(&mut self.sel);
                self.more = false;
                app.gif(&ids);
            }
            Btn::Queue => {
                let ids = std::mem::take(&mut self.sel);
                let ok = app.start_queue(&ids);
                self.show(&app.g, &app.store, View::Rest);
                if !ok {
                    self.fail();
                }
            }
            Btn::Stack => {
                let ids = std::mem::take(&mut self.sel);
                app.store.stack(&ids, ids[0]);
                self.more = false;
                self.refresh(&app.g, &app.store);
            }
            Btn::Unstack => {
                let ids = std::mem::take(&mut self.sel);
                app.store.stack(&ids, 0);
                self.more = false;
                self.refresh(&app.g, &app.store);
            }
            Btn::Go(m) => {
                self.show(&app.g, &app.store, View::Rest);
                app.run(m);
            }
            Btn::Stop => {
                app.stop_interval();
                self.show(&app.g, &app.store, View::Rest);
            }
            Btn::StopRec => {
                app.stop_rec(self);
                self.show(&app.g, &app.store, View::Rest);
            }
            Btn::Quit => app.quit(self),
            Btn::Key(n) => app.record_key(self, n),
            Btn::Folder => unsafe {
                let _ = PostMessageW(Some(self.sensor), crate::WM_PICK_FOLDER, WPARAM(0), LPARAM(0));
            },
            Btn::Step(n, d) => {
                let s = &mut app.store.settings;
                match n {
                    0 => s.keep_days = next(&[1, 3, 7, 14, 30, 90, 365], s.keep_days, d),
                    1 => s.delay = next(&[1, 2, 3, 5, 10], s.delay, d),
                    _ => s.interval = next(&[5, 10, 30, 60, 120, 300, 600, 1800, 3600], s.interval, d),
                }
                app.store.save_settings();
            }
            Btn::History => {
                app.store.settings.history = !app.store.settings.history;
                app.store.save_settings();
            }
            Btn::Login => {
                app.login = !app.login;
                crate::set_login(app.login);
            }
        }
        self.dirty = true;
    }
}

fn next(steps: &[i64], v: i64, d: i64) -> i64 {
    let at = steps.iter().position(|s| *s >= v).unwrap_or(steps.len() - 1) as i64;
    steps[(at + d).clamp(0, steps.len() as i64 - 1) as usize]
}

fn plural(n: i64, unit: &str) -> String {
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

fn span(s: i64) -> String {
    match s {
        s if s >= 3600 => format!("{} h", s / 3600),
        s if s >= 60 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

/// Route a window message. True when handled.
pub fn message(n: &mut Notch, app: &mut App, msg: u32, w: WPARAM, l: LPARAM) -> bool {
    match msg {
        WM_MOUSEMOVE => n.mouse_move(app, l),
        WM_MOUSELEAVE => n.mouse_leave(),
        WM_LBUTTONDOWN => n.down(app, l),
        WM_LBUTTONUP => n.up(app, l),
        WM_LBUTTONDBLCLK => n.double(app, l),
        WM_RBUTTONUP => n.right(app, l),
        WM_CAPTURECHANGED => n.capture_lost(),
        WM_MOUSEWHEEL => n.wheel(app, ((w.0 >> 16) & 0xffff) as i16 as f32),
        WM_KEYDOWN => {
            let mut mods = 0u32;
            unsafe {
                if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                    mods |= MOD_CONTROL.0;
                }
                if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    mods |= MOD_SHIFT.0;
                }
                if GetKeyState(VK_MENU.0 as i32) < 0 {
                    mods |= MOD_ALT.0;
                }
            }
            n.key(app, mods, w.0 as u32);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod folder_tests {
    use super::*;

    #[test]
    fn folder_sheet_fan_rises_and_stays_inside_the_tile() {
        let tile = R::new(16.0, 16.0, TILE.0, TILE.1);
        let (closed, closed_angle) = folder_sheet_pose(tile, -1.0, 0.0);
        let (left, left_angle) = folder_sheet_pose(tile, -1.0, 1.0);
        let (right, right_angle) = folder_sheet_pose(tile, 1.0, 1.0);
        assert!(left.x < closed.x && left.y < closed.y && left_angle < closed_angle);
        assert_eq!(left_angle, -right_angle);
        assert_eq!(left.cx() + right.cx(), tile.cx() * 2.0);
        // Even the rotated sheet's corners remain inside the non-text area.
        for (sheet, angle) in [(left, left_angle), (right, right_angle)] {
            let (sin, cos) = angle.to_radians().sin_cos();
            for (x, y) in [(-13.0, -14.5), (13.0, -14.5), (13.0, 14.5), (-13.0, 14.5)] {
                let point = (sheet.cx() + cos * x - sin * y, sheet.cy() + sin * x + cos * y);
                assert!(R::new(tile.x, tile.y, tile.w, 53.0).has(point.0, point.1));
            }
        }
    }

    #[test]
    fn app_folder_color_is_stable_for_case_and_padding() {
        assert_eq!(folder_palette("  Premiere Pro "), folder_palette("premiere pro"));
    }
}
