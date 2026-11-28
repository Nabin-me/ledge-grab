//! The GNOME ledge: which view is showing, where things are, what a click means,
//! and how it looks. No GTK here, so layout and hit-testing are unit-tested; the
//! app turns `Action`s into work and hands drawing a `Canvas`.
//!
//! Sizes and colours follow the Windows ledge (`notch.rs`).
use super::canvas::*;
use crate::paste_queue::Stall;
use crate::store::{Kind, Store};
use std::collections::HashSet;

pub const TILE: (f32, f32) = (132.0, 84.0);
const GAP: f32 = 10.0;
const PAD: f32 = 16.0;
const COLS: usize = 4;
const ROWS: usize = 3;
const BAR: f32 = 52.0;
pub const MAX_DOTS: usize = 5;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    /// Collapsed: a thin strip with recency dots.
    Rest,
    Shelf,
    /// Sequential Copy is running.
    Queue,
    /// A capture just landed.
    Flash(i64),
    /// A short message (e.g. a failed copy), then back to Rest.
    Notice(&'static str, &'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Btn {
    Tile(i64),
    Copy,
    Sequential,
    Delete,
    Region,
    Screen,
    QueueNext,
    QueueStop,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    None,
    /// Selection or hover changed; redraw.
    Redraw,
    Copy(Vec<i64>),
    Sequential(Vec<i64>),
    Delete(Vec<i64>),
    Region,
    Screen,
    QueueNext,
    QueueStop,
}

/// What the queue view needs to show.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct QueueView {
    pub pos: usize,
    pub len: usize,
    pub current: Option<i64>,
    pub stall: Option<Stall>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Age {
    Fresh,
    Today,
    Older,
}

pub fn age(ms: i64) -> Age {
    if ms < 15 * 60_000 {
        Age::Fresh
    } else if ms < 24 * 3_600_000 {
        Age::Today
    } else {
        Age::Older
    }
}

/// Ages of the newest visible screenshots, newest first, and whether more exist.
pub fn shelf_dots(st: &Store, now_ms: i64) -> (Vec<Age>, bool) {
    let mut created: Vec<i64> =
        st.items.iter().filter(|i| i.kind == Kind::Shot && !i.hidden).map(|i| i.created).collect();
    created.sort_unstable_by(|a, b| b.cmp(a));
    let ages = created.iter().take(MAX_DOTS).map(|&t| age(now_ms - t)).collect();
    (ages, created.len() > MAX_DOTS)
}

pub struct Ledge {
    pub view: View,
    /// Selected groups, by their first id, in the order picked.
    pub picked: Vec<i64>,
    pub hover: Option<Btn>,
    pub queue: Option<QueueView>,
}

impl Default for Ledge {
    fn default() -> Self {
        Ledge { view: View::Rest, picked: Vec::new(), hover: None, queue: None }
    }
}

impl Ledge {
    /// Logical size of the island for the current view (width, height, corner radius).
    pub fn size(&self, st: &Store) -> (f32, f32, f32) {
        match self.view {
            View::Rest => (210.0, 9.0, 6.0),
            View::Shelf => {
                let rows = st.shelf().len().div_ceil(COLS).clamp(1, ROWS) as f32;
                let w = PAD * 2.0 + COLS as f32 * TILE.0 + (COLS - 1) as f32 * GAP;
                (w, PAD + rows * (TILE.1 + GAP) + BAR, 28.0)
            }
            View::Queue => (330.0, 48.0, 18.0),
            View::Flash(_) => (TILE.0 + PAD * 2.0, 116.0, 26.0),
            View::Notice(..) => (360.0, 64.0, 18.0),
        }
    }

    /// Hit areas for the current view, in island coordinates.
    pub fn spots(&self, st: &Store) -> Vec<(Btn, R)> {
        let (w, h, _) = self.size(st);
        let mut v = Vec::new();
        match self.view {
            View::Shelf => {
                for (i, g) in st.shelf().iter().take(COLS * ROWS).enumerate() {
                    let (c, r) = ((i % COLS) as f32, (i / COLS) as f32);
                    v.push((Btn::Tile(g[0]), R::new(PAD + c * (TILE.0 + GAP), PAD + r * (TILE.1 + GAP), TILE.0, TILE.1)));
                }
                let y = h - BAR + 10.0;
                let mut x = PAD;
                let mut pill = |b: Btn, label: &str, v: &mut Vec<(Btn, R)>| {
                    let pw = label_width(label);
                    v.push((b, R::new(x, y, pw, 32.0)));
                    x += pw + 8.0;
                };
                if self.picked.is_empty() {
                    pill(Btn::Region, "Region", &mut v);
                    pill(Btn::Screen, "Screen", &mut v);
                } else {
                    pill(Btn::Copy, "Copy", &mut v);
                    if self.picked_ids(st).len() > 1 && self.all_shots(st) {
                        pill(Btn::Sequential, "Sequential Copy", &mut v);
                    }
                    pill(Btn::Delete, "Delete", &mut v);
                }
            }
            View::Queue => {
                v.push((Btn::QueueStop, R::new(w - 40.0, 8.0, 32.0, 32.0)));
                v.push((Btn::QueueNext, R::new(w - 104.0, 8.0, 58.0, 32.0)));
            }
            _ => {}
        }
        v
    }

    pub fn hit(&self, st: &Store, x: f32, y: f32) -> Option<Btn> {
        self.spots(st).into_iter().find(|(_, r)| r.has(x, y)).map(|(b, _)| b)
    }

    /// Every id in the picked groups, in shelf display order.
    pub fn picked_ids(&self, st: &Store) -> Vec<i64> {
        let picked: HashSet<i64> = self.picked.iter().copied().collect();
        st.shelf().into_iter().filter(|g| picked.contains(&g[0])).flatten().collect()
    }

    fn all_shots(&self, st: &Store) -> bool {
        self.picked_ids(st).iter().all(|id| st.get(*id).is_some_and(|i| i.kind == Kind::Shot))
    }

    /// Pointer moved to (x, y) in island coordinates.
    pub fn motion(&mut self, st: &Store, x: f32, y: f32) -> Action {
        let h = self.hit(st, x, y);
        if h != self.hover {
            self.hover = h;
            return Action::Redraw;
        }
        Action::None
    }

    pub fn click(&mut self, st: &Store, x: f32, y: f32) -> Action {
        match self.hit(st, x, y) {
            Some(Btn::Tile(id)) => {
                if let Some(i) = self.picked.iter().position(|p| *p == id) {
                    self.picked.remove(i);
                } else {
                    self.picked.push(id);
                }
                Action::Redraw
            }
            Some(Btn::Copy) => Action::Copy(self.picked_ids(st)),
            Some(Btn::Sequential) => Action::Sequential(self.picked_ids(st)),
            Some(Btn::Delete) => {
                let ids = self.picked_ids(st);
                self.picked.clear();
                Action::Delete(ids)
            }
            Some(Btn::Region) => Action::Region,
            Some(Btn::Screen) => Action::Screen,
            Some(Btn::QueueNext) => Action::QueueNext,
            Some(Btn::QueueStop) => Action::QueueStop,
            None => Action::None,
        }
    }

    /// Draw the island into `c`, which is sized `self.size()` plus `ear` on each side.
    pub fn draw(&self, c: &mut Canvas, st: &Store, ear: f32, now_ms: i64, thumb: &mut dyn FnMut(i64) -> Option<std::rc::Rc<Image>>) {
        let (w, h, rad) = self.size(st);
        c.clear();
        c.island(R::new(ear, 0.0, w, h), rad, ear.min(h - 1.0).max(0.0));
        let spots = self.spots(st);
        let at = |b: Btn| spots.iter().find(|(s, _)| *s == b).map(|(_, r)| R::new(r.x + ear, r.y, r.w, r.h));
        let hover = self.hover;
        let pill = |c: &mut Canvas, b: Btn, text: &str| {
            if let Some(r) = at(b) {
                c.fill(r, r.h / 2.0, if hover == Some(b) { PILL_HOVER } else { PILL }, 1.0);
                c.text(text, r, 13.0, INK, 1.0, Align::Center);
            }
        };
        match self.view {
            View::Rest => {
                let (dots, more) = shelf_dots(st, now_ms);
                let mut x = ear + 15.0;
                for a in &dots {
                    let (hex, al) = match a {
                        Age::Fresh => (INK, 1.0),
                        Age::Today => (DIM, 0.95),
                        Age::Older => (DIM, 0.5),
                    };
                    c.circle(x, h / 2.0, 1.7, hex, al);
                    x += 6.0;
                }
                if more {
                    c.text("+", R::new(x - 3.0, -1.0, 10.0, h + 2.0), 8.0, DIM, 0.9, Align::Left);
                }
            }
            View::Shelf => {
                let groups = st.shelf();
                if groups.is_empty() {
                    c.text("No screenshots yet", R::new(ear, PAD, w, TILE.1), 14.0, DIM, 1.0, Align::Center);
                }
                for g in groups.iter().take(COLS * ROWS) {
                    let Some(r) = at(Btn::Tile(g[0])) else { continue };
                    c.fill(r, 12.0, CARD, 1.0);
                    if let Some(img) = thumb(g[0]) {
                        c.image_round(&img, r, 12.0, 1.0);
                    }
                    if let Some(n) = self.picked.iter().position(|p| *p == g[0]) {
                        c.stroke(r.grow(1.5), 13.5, 2.5, INK, 1.0);
                        c.circle(r.x + r.w - 14.0, r.y + 14.0, 9.0, INK, 1.0);
                        c.text(&(n + 1).to_string(), R::new(r.x + r.w - 23.0, r.y + 5.0, 18.0, 18.0), 11.0, BLACK, 1.0, Align::Center);
                    } else if hover == Some(Btn::Tile(g[0])) {
                        c.stroke(r, 12.0, 1.5, DIM, 0.8);
                    }
                }
                pill(c, Btn::Region, "Region");
                pill(c, Btn::Screen, "Screen");
                pill(c, Btn::Copy, "Copy");
                pill(c, Btn::Sequential, "Sequential Copy");
                pill(c, Btn::Delete, "Delete");
                let n = groups.len();
                let label = if n == 1 { "1 screenshot".to_string() } else { format!("{n} screenshots") };
                c.text(&label, R::new(ear, h - BAR + 10.0, w - PAD, 32.0), 12.0, DIM, 1.0, Align::Right);
            }
            View::Queue => {
                if let Some(q) = self.queue {
                    let tr = R::new(ear + 10.0, 8.0, 48.0, 32.0);
                    c.fill(tr, 7.0, CARD, 1.0);
                    if let Some(img) = q.current.and_then(|id| thumb(id)) {
                        c.image_round(&img, tr, 7.0, 1.0);
                    }
                    let (title, hint) = queue_text(&q);
                    c.text(&title, R::new(ear + 68.0, 7.0, 150.0, 18.0), 13.0, INK, 1.0, Align::Left);
                    c.text(hint, R::new(ear + 68.0, 24.0, 150.0, 16.0), 11.0, DIM, 1.0, Align::Left);
                    let next = match q.stall {
                        Some(Stall::Busy) => "Retry",
                        Some(Stall::Missing) => "Skip",
                        _ => "Next",
                    };
                    pill(c, Btn::QueueNext, next);
                    if let Some(r) = at(Btn::QueueStop) {
                        c.fill(r, r.h / 2.0, if hover == Some(Btn::QueueStop) { PILL_HOVER } else { PILL }, 1.0);
                        let (cx, cy, d) = (r.cx(), r.cy(), 5.0);
                        c.line((cx - d, cy - d), (cx + d, cy + d), 1.6, INK, 1.0);
                        c.line((cx - d, cy + d), (cx + d, cy - d), 1.6, INK, 1.0);
                    }
                }
            }
            View::Flash(id) => {
                let r = R::new(ear + PAD, PAD, TILE.0, TILE.1);
                c.fill(r, 12.0, CARD, 1.0);
                if let Some(img) = thumb(id) {
                    c.image_round(&img, r, 12.0, 1.0);
                }
            }
            View::Notice(title, hint) => {
                c.circle(ear + 22.0, h / 2.0, 4.0, RED, 1.0);
                c.text(title, R::new(ear + 36.0, 12.0, w - 48.0, 20.0), 13.0, INK, 1.0, Align::Left);
                c.text(hint, R::new(ear + 36.0, 32.0, w - 48.0, 18.0), 11.0, DIM, 1.0, Align::Left);
            }
        }
    }
}

/// Pill width for a label at 13 px.
fn label_width(label: &str) -> f32 {
    let probe = Canvas::new(1.0, 1.0, 1.0).expect("tiny canvas");
    (probe.measure(label, 13.0) + 30.0).ceil()
}

pub fn queue_text(q: &QueueView) -> (String, &'static str) {
    match q.stall {
        None => (format!("Next {} of {}", q.pos, q.len), "Ctrl+V pastes it"),
        Some(Stall::NotRead) => ("Paste not detected".into(), "Paste again, or Next"),
        Some(Stall::ReadElsewhere) => ("Pasted another way?".into(), "Next, or paste again"),
        Some(Stall::Busy) => ("Clipboard busy".into(), "Retry to swap"),
        Some(Stall::Missing) => (format!("Image {} of {} missing", q.pos, q.len), "Skip it, or stop"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_fade_with_age() {
        assert_eq!(age(0), Age::Fresh);
        assert_eq!(age(15 * 60_000), Age::Today);
        assert_eq!(age(24 * 3_600_000), Age::Older);
    }

    #[test]
    fn queue_hint_never_hides_a_stall() {
        let q = QueueView { pos: 2, len: 3, current: Some(1), stall: Some(Stall::NotRead) };
        assert_eq!(queue_text(&q).0, "Paste not detected");
        let q = QueueView { stall: None, ..q };
        assert_eq!(queue_text(&q).0, "Next 2 of 3");
    }
}
