//! The Windows app: Win32 message loop, Direct2D ledge, hooks and capture.
#[path = "../capture.rs"]
pub mod capture;
#[path = "../clip.rs"]
pub mod clip;
#[path = "../float.rs"]
pub mod float;
#[path = "../gfx.rs"]
pub mod gfx;
#[path = "../keys.rs"]
pub mod keys;
#[path = "../notch.rs"]
pub mod notch;
#[path = "../overlay.rs"]
pub mod overlay;
#[path = "../pretty.rs"]
pub mod pretty;
#[path = "../rec.rs"]
pub mod rec;
#[path = "../scroll.rs"]
pub mod scroll;
#[path = "../scroll_pdf.rs"]
pub mod scroll_pdf;
#[path = "../updater.rs"]
pub mod updater;

use crate::{note, ocr, paste_queue, pdf, save_should_fail, scroll_pdf_preview, store, testing};
use capture::{Frame, cursor, grab, monitors};
use float::Float;
use gfx::Gfx;
use notch::{Notch, View};
use overlay::{Mode, Outcome, Overlay};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering::Relaxed};
use std::time::{Duration, Instant};
use store::{Kind, Store};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PWSTR, w};

pub const WM_DRAG_OUT: u32 = WM_APP + 1;
pub const WM_RUN: u32 = WM_APP + 2;
/// Boxed Work in lparam.
const WM_WORK: u32 = WM_APP + 3;
const WM_KEY: u32 = WM_APP + 4;
pub const WM_PICK_FOLDER: u32 = WM_APP + 5;
const WM_PASTED: u32 = WM_APP + 6;
const WM_NOTCH_KEY: u32 = WM_APP + 7;
/// A Sequential Copy step to run outside WM_RENDERFORMAT; wparam is a `Then`.
const WM_QUEUE_STEP: u32 = WM_APP + 8;
/// Replay one paste held back while the next image was placed.
const WM_REPLAY: u32 = WM_APP + 9;
/// Marks our own replayed keystrokes.
const REPLAY_TAG: usize = 0x4C47_5150; // "LGQP"

// Give motion a chance under a continuous stream of pointer/keyboard messages.
// A single handler can still exceed the budget; never interrupt one mid-dispatch.
const INPUT_SLICE: Duration = Duration::from_millis(4);

pub const T_DWELL: usize = 1;
pub const T_LEAVE: usize = 2;
pub const T_FLASH: usize = 3;
const T_TICK: usize = 4;
const T_COUNT: usize = 5;
const T_INTERVAL: usize = 6;
const T_HOURLY: usize = 7;
const T_TRIM: usize = 8;
const T_CLIP: usize = 9;
const T_PASTE: usize = 10;
pub const T_REC: usize = 11;
const T_QUEUE_RETRY: usize = 12;
/// Redraws the collapsed ledge so its dots fade as screenshots age.
const T_DOTS: usize = 13;
/// Waits for an idle moment to install a downloaded update.
const T_UPDATE: usize = 14;

enum Work {
    Shot(Frame, String),
    Text(String),
    /// A shot's file write finished.
    Saved(i64, bool),
    /// What a shot turned out to say.
    Name(i64, String),
    Clip(Option<rec::Clip>),
    Gif(Option<(PathBuf, i64)>),
    ScrollPdf(Result<(PathBuf, Frame, String, scroll_pdf::End), (&'static str, &'static str)>),
    Failed,
}

pub struct App {
    pub g: Gfx,
    pub store: Store,
    inst: HINSTANCE,
    main: HWND,
    /// Items currently on the clipboard.
    pub clip: Vec<i64>,
    /// Where promised formats of the image on the clipboard come from.
    /// Always describes the same image as `clip`; see `clip::record`.
    png: Option<ClipSource>,
    /// Which hotkeys Windows gave us.
    pub hot: [bool; 4],
    pub rec: Option<rec::Rec>,
    /// App the clip is of.
    rec_app: String,
    floats: Vec<Float>,
    /// Background jobs the notch shows as one moving line.
    pub jobs: usize,
    /// Last picture another app copied.
    seen: u64,
    quitting: bool,
    pub interval: Option<RECT>,
    scroll_pdf_stop: Option<Arc<AtomicBool>>,
    pub login: bool,
    pub outgoing: Vec<i64>,
    count: i64,
    overlay: Option<Overlay>,
    last_foreground: HWND,
    /// A verified update, installed at the next idle moment.
    update: Option<updater::Ready>,
}

enum ClipSource {
    Frame(Frame),
    File(PathBuf),
    /// A Sequential Copy image: every format is rendered on first read.
    Queued(Frame, PathBuf),
}

impl ClipSource {
    fn bytes(&self, format: u32) -> Option<Vec<u8>> {
        use windows::Win32::System::Ole::CF_DIBV5;
        if format == clip::png_format() {
            match self {
                ClipSource::Frame(f) => Some(store::encode_png(f)),
                ClipSource::File(p) | ClipSource::Queued(_, p) => std::fs::read(p).ok(),
            }
        } else if format == CF_DIBV5.0 as u32 {
            match self {
                ClipSource::Frame(f) | ClipSource::Queued(f, _) => Some(clip::dibv5(f)),
                ClipSource::File(p) => read_shot(p).map(|f| clip::dibv5(&f)),
            }
        } else {
            None
        }
    }
}

/// Sequential Copy state. The paste hook reads it before the target app sees a
/// key, so it lives outside App. Never held across anything that pumps messages.
static QUEUE: std::sync::Mutex<Option<paste_queue::Queue>> = std::sync::Mutex::new(None);

pub(crate) fn with_queue<R>(f: impl FnOnce(&mut paste_queue::Queue) -> R) -> Option<R> {
    QUEUE.lock().ok()?.as_mut().map(f)
}

fn window_pid(h: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// What the ledge shows while Sequential Copy runs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct QueueView {
    /// 1-based position of the image on the clipboard.
    pub pos: usize,
    pub len: usize,
    pub current: Option<i64>,
    pub stall: Option<paste_queue::Stall>,
}

pub fn queue_view() -> Option<QueueView> {
    with_queue(|q| {
        let (pos, len) = q.progress();
        let stall = match q.phase {
            paste_queue::Phase::Stalled(s) => Some(s),
            _ => None,
        };
        QueueView { pos, len, current: q.current(), stall }
    })
}

fn then_code(t: paste_queue::Then) -> usize {
    use paste_queue::Then::*;
    match t {
        Nothing => 0,
        Place => 1,
        Replay => 2,
        Finished => 3,
        Retry => 4,
    }
}

fn code_then(c: usize) -> paste_queue::Then {
    use paste_queue::Then::*;
    [Nothing, Place, Replay, Finished, Retry].get(c).copied().unwrap_or(Nothing)
}

static APP: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());
static NOTCH: AtomicPtr<Notch> = AtomicPtr::new(std::ptr::null_mut());

// The whole app runs on one thread. Handlers that can pump messages (drag, folder
// picker) are posted, so they never run inside another handler's borrow.
fn app() -> Option<&'static mut App> {
    unsafe { APP.load(Relaxed).as_mut() }
}
fn notch() -> Option<&'static mut Notch> {
    unsafe { NOTCH.load(Relaxed).as_mut() }
}


/// Opt-in timing for the Windows smoke test. Collect in memory and write at
/// idle, rather than adding a disk write to every timed frame.
struct FrameTrace {
    path: PathBuf,
    start: Instant,
    frames: Vec<(u128, u128, bool, bool)>,
}

impl FrameTrace {
    fn start() -> Option<Self> {
        if !testing() {
            return None;
        }
        Some(Self {
            path: std::env::var_os("LEDGE_GRAB_FRAME_LOG")?.into(),
            start: Instant::now(),
            frames: Vec::with_capacity(4096),
        })
    }

    fn record(&mut self, sample: Instant, notch: bool, overlay: bool) {
        if self.frames.len() < self.frames.capacity() {
            self.frames.push(((sample - self.start).as_micros(), sample.elapsed().as_micros(), notch, overlay));
        }
    }

    fn flush(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        use std::io::Write;
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let mut out = std::io::BufWriter::new(f);
            for &(start, render, notch, overlay) in &self.frames {
                let _ = writeln!(out, "{start},{render},{},{}", u8::from(notch), u8::from(overlay));
            }
        }
        self.frames.clear();
    }
}

impl App {
    /// Put items on the clipboard: a picture for one shot, files otherwise.
    /// `Busy` means the clipboard is unchanged; `Cleared` means it is now empty.
    pub fn copy(&mut self, ids: &[i64]) -> clip::Set {
        self.end_queue();
        self.put(ids)
    }

    fn put(&mut self, ids: &[i64]) -> clip::Set {
        let items: Vec<_> = ids.iter().filter_map(|i| self.store.get(*i)).collect();
        let files: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        let (r, source) = match items.as_slice() {
            [one] if one.kind == Kind::Shot => {
                let path = one.path.clone();
                match read_shot(&path) {
                    Some(f) => (clip::set_image(self.main, &f, false), Some(ClipSource::File(path))),
                    // Nothing was touched.
                    None => (clip::Set::Busy, None),
                }
            }
            [] => (clip::Set::Busy, None),
            _ => (clip::set_files(self.main, &files), None),
        };
        clip::record(r, &mut self.clip, &mut self.png, ids, source);
        r
    }

    /// Clipboard holds text now; no item is on it.
    fn put_text(&mut self, s: &str) -> clip::Set {
        self.end_queue();
        let r = clip::set_text(self.main, s);
        clip::record(r, &mut self.clip, &mut self.png, &[], None);
        r
    }

    /// Sequential Copy: `ids` in shelf order; each paste hands over the next.
    pub fn start_queue(&mut self, ids: &[i64]) -> Result<(), paste_queue::Placed> {
        self.end_queue();
        let Some(&first) = ids.first() else { return Err(paste_queue::Placed::Missing) };
        match self.place_queued(first) {
            paste_queue::Placed::Ok => {}
            other => return Err(other),
        }
        if let Ok(mut q) = QUEUE.lock() {
            *q = Some(paste_queue::Queue::new(ids.to_vec()));
        }
        start_paste_hook();
        unsafe {
            SetTimer(Some(self.main), T_PASTE, 200, None);
        }
        Ok(())
    }

    /// Put one queued image on the clipboard with every format promised.
    fn place_queued(&mut self, id: i64) -> paste_queue::Placed {
        use paste_queue::Placed;
        let Some(item) = self.store.get(id) else { return Placed::Missing };
        if self.store.is_unsaved(id) {
            // Still being written: try again shortly rather than call it missing.
            return Placed::Busy;
        }
        let path = item.path.clone();
        let Some(f) = (item.kind == Kind::Shot).then(|| read_shot(&path)).flatten() else { return Placed::Missing };
        let r = clip::set_image(self.main, &f, true);
        clip::record(r, &mut self.clip, &mut self.png, &[id], Some(ClipSource::Queued(f, path)));
        match r {
            clip::Set::Ok => Placed::Ok,
            clip::Set::Busy => Placed::Busy,
            clip::Set::Cleared => Placed::Cleared,
        }
    }

    pub fn stop_queue(&mut self) {
        self.end_queue();
    }

    pub fn queue_active(&self) -> bool {
        QUEUE.lock().is_ok_and(|q| q.is_some())
    }

    fn end_queue(&mut self) {
        if let Ok(mut q) = QUEUE.lock() {
            *q = None;
        }
        stop_hook(&PASTE_HOOK);
        unsafe {
            let _ = KillTimer(Some(self.main), T_PASTE);
            let _ = KillTimer(Some(self.main), T_QUEUE_RETRY);
        }
    }

    /// Carry out what the queue asked for, until it needs to wait.
    pub fn queue_step(&mut self, n: &mut Notch, mut then: paste_queue::Then) {
        use paste_queue::{Placed, Then};
        loop {
            match then {
                Then::Nothing => break,
                Then::Place => {
                    let Some(id) = with_queue(|q| q.current()).flatten() else { break };
                    let placed = self.place_queued(id);
                    if placed == Placed::Missing {
                        // Never leave the previous, already pasted image to be pasted again.
                        if clip::clear(self.main).ok() {
                            clip::record(clip::Set::Cleared, &mut self.clip, &mut self.png, &[], None);
                        }
                    }
                    note(&format!("queue placed {placed:?}"));
                    then = with_queue(|q| q.placed(placed)).unwrap_or(Then::Nothing);
                }
                Then::Replay => {
                    unsafe {
                        let _ = PostMessageW(Some(self.main), WM_REPLAY, WPARAM(0), LPARAM(0));
                    }
                    break;
                }
                Then::Retry => {
                    unsafe {
                        SetTimer(Some(self.main), T_QUEUE_RETRY, 120, None);
                    }
                    break;
                }
                Then::Finished => {
                    note("queue finished");
                    self.end_queue();
                    n.done(&self.g, &self.store);
                    break;
                }
            }
        }
        n.repaint();
    }

    /// A file to hand to another app.
    pub fn file(&self, id: i64) -> Option<PathBuf> {
        Some(self.store.get(id)?.path.clone())
    }

    pub fn merge(&mut self, ids: &[i64]) -> Option<i64> {
        let items: Vec<_> = ids.iter().filter_map(|i| self.store.get(*i)).collect();
        let files: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        let first_app = items.first()?.export_app();
        let app = if items.iter().all(|i| i.export_app() == first_app) { first_app } else { "Mixed apps" }.to_owned();
        let bytes = pdf::merge(&files)?;
        let thumb = self.store.thumb(*ids.first()?)?;
        let path = self.store.new_path("pdf", &app).ok()?;
        store::write_file(&path, &bytes).ok()?;
        let id = self.store.add(&path, Kind::Pdf, &thumb, &app, 0).ok()?;
        // The PDF exists either way; the shelf shows whether it reached the clipboard.
        let _ = self.copy(&[id]);
        Some(id)
    }

    /// Set shots on a backdrop, as new shots.
    pub fn backdrop(&mut self, ids: &[i64]) -> Option<i64> {
        let mut last = None;
        for id in ids {
            let i = self.store.get(*id)?;
            let (path, app) = (i.path.clone(), i.export_app().to_owned());
            let f = pretty::make(&self.g, &read_shot(&path)?).ok()?;
            last = self.deliver(f, false, || app);
        }
        last
    }

    pub fn float(&mut self, ids: &[i64], mon: &capture::Monitor) -> bool {
        let mut any = false;
        for (n, id) in ids.iter().take(12).enumerate() {
            let Some(f) = self.store.get(*id).map(|i| i.path.clone()).and_then(|p| read_shot(&p)) else { continue };
            let fit = |k: f32| (mon.w() as f32 * k / f.w as f32).min(mon.h() as f32 * k / f.h as f32);
            let zoom = fit(0.5).min(1.0);
            let (w, h) = ((f.w as f32 * zoom) as i32, (f.h as f32 * zoom) as i32);
            let step = n as i32 * 36;
            let at = POINT {
                x: (mon.rect.left + mon.rect.right - w) / 2 + step,
                y: (mon.rect.top + mon.rect.bottom - h) / 2 + step,
            };
            any |= self.float_at(&f, at, zoom, mon.dpi);
        }
        any
    }

    fn float_at(&mut self, f: &Frame, at: POINT, zoom: f32, dpi: u32) -> bool {
        match Float::open(&self.g, self.inst, f, at, zoom, dpi) {
            Ok(f) => {
                self.floats.push(f);
                true
            }
            Err(e) => {
                note(&format!("float failed {e}"));
                false
            }
        }
    }

    pub fn gif(&mut self, ids: &[i64]) {
        for id in ids {
            let Some(i) = self.store.get(*id).filter(|i| i.kind == Kind::Clip) else { continue };
            let Ok(dst) = self.store.new_path("gif", i.export_app()) else {
                note("GIF destination unavailable");
                continue;
            };
            let (src, from, main) = (i.path.clone(), *id, self.main.0 as isize);
            self.jobs += 1;
            std::thread::spawn(move || {
                let ok = rec::gif(&src, &dst).is_some();
                post(main, Work::Gif(ok.then_some((dst, from))));
            });
        }
    }

    pub fn stop_rec(&mut self, n: &mut Notch) {
        if self.rec.take().is_some() {
            self.jobs += 1;
            n.rec = None;
            unsafe {
                let _ = KillTimer(Some(self.main), T_REC);
            }
            n.show(&self.g, &self.store, View::Rest);
        }
    }

    pub fn quit(&mut self, n: &mut Notch) {
        if self.scroll_pdf_busy() {
            self.quitting = true;
            self.stop_scroll_pdf(n);
        } else if self.rec.is_some() {
            // Finish the clip first.
            self.quitting = true;
            self.stop_rec(n);
        } else {
            unsafe {
                let _ = DestroyWindow(self.main);
            }
        }
    }

    pub fn run(&self, m: Mode) {
        unsafe {
            let _ = PostMessageW(Some(self.main), WM_RUN, WPARAM(m.index()), LPARAM(0));
        }
    }

    pub fn stop_interval(&mut self) {
        self.interval = None;
        unsafe {
            let _ = KillTimer(Some(self.main), T_INTERVAL);
        }
    }

    pub fn record_key(&mut self, n: &mut Notch, row: usize) {
        n.recording = Some(row);
        n.sync_auto_close();
        self.unregister();
        start_key_hook();
    }

    fn unregister(&mut self) {
        for i in 0..4 {
            if std::mem::take(&mut self.hot[i]) {
                unsafe {
                    let _ = UnregisterHotKey(Some(self.main), i as i32 + 1);
                }
            }
        }
    }

    /// Claim hotkeys we don't hold yet. Returns true if any changed.
    fn register(&mut self) -> bool {
        if testing() || scroll_pdf_preview() || !HOOK.load(Relaxed).is_null() {
            return false;
        }
        let s = &self.store.settings;
        let mut changed = false;
        for (i, k) in s.keys().into_iter().enumerate() {
            if !self.hot[i] && k.1 != 0 {
                let m = HOT_KEY_MODIFIERS(k.0) | MOD_NOREPEAT;
                self.hot[i] = unsafe { RegisterHotKey(Some(self.main), i as i32 + 1, m, k.1) }.is_ok();
                changed |= self.hot[i];
            }
        }
        changed
    }

    /// A finished capture: clipboard first, then notch, then disk.
    fn deliver(&mut self, f: Frame, quiet: bool, app: impl FnOnce() -> String) -> Option<i64> {
        if f.w < 1 || f.h < 1 {
            return None;
        }
        let t = Instant::now();
        if !quiet {
            self.end_queue();
        }
        let r = if quiet { clip::Set::Busy } else { clip::set_image(self.main, &f, false) };
        note(&format!("clipboard_ms {:.1}", t.elapsed().as_secs_f64() * 1e3));
        // Record now, before indexing can fail, so the PNG promise always follows
        // the bitmap that is actually on the clipboard.
        let source = r.ok().then(|| ClipSource::Frame(f.clone()));
        clip::record(r, &mut self.clip, &mut self.png, &[], source);
        let id = self.keep(f, &app(), !quiet)?;
        if r.ok() {
            self.clip = vec![id];
        }
        note(&format!("indexed_ms {:.1}", t.elapsed().as_secs_f64() * 1e3));
        Some(id)
    }

    /// Index a picture and save it behind the scenes, named after what it says.
    fn keep(&mut self, f: Frame, app: &str, named: bool) -> Option<i64> {
        let t = Instant::now();
        let thumb = f.thumb(store::THUMB.0, store::THUMB.1);
        let Ok(path) = self.store.new_path("png", app) else {
            post(self.main.0 as isize, Work::Failed);
            return None;
        };
        let id = self.store.add_unsaved(&path, &thumb, app).ok()?;
        let main = self.main.0 as isize;
        std::thread::spawn(move || {
            let ok = !save_should_fail() && store::write_file(&path, &store::encode_png(&f)).is_ok();
            note(&format!("saved_ms {:.1} {ok}", t.elapsed().as_secs_f64() * 1e3));
            post(main, Work::Saved(id, ok));
            if !ok {
                return;
            }
            if named {
                // The top says what it is; long captures needn't be read whole.
                let top = f.crop(RECT { left: 0, top: 0, right: f.w.min(4000), bottom: f.h.min(1600) });
                drop(f);
                if let Some(t) = ocr::words(&top).ok().and_then(|w| store::title(&w)) {
                    post(main, Work::Name(id, t));
                }
            }
        });
        unsafe {
            SetTimer(Some(self.main), T_TRIM, 4000, None);
        }
        Some(id)
    }

    fn name(&mut self, id: i64, title: &str) {
        // Files already on the clipboard keep their names.
        if self.clip.len() > 1 && self.clip.contains(&id) {
            return;
        }
        if let Some(p) = self.store.rename(id, title) {
            if self.clip == [id] {
                match &mut self.png {
                    Some(ClipSource::File(old) | ClipSource::Queued(_, old)) => *old = p,
                    _ => {}
                }
            }
        }
    }

    /// Another app copied an image.
    fn take_clipboard(&mut self, n: &mut Notch) {
        let app = capture::app_name(unsafe { GetForegroundWindow() });
        let Some(f) = clip::read_image(self.main) else { return };
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        (f.w, f.h, &f.px).hash(&mut h);
        if std::mem::replace(&mut self.seen, h.finish()) == self.seen {
            return;
        }
        if let Some(id) = self.keep(f, &app, true) {
            self.clip = vec![id];
        }
        n.refresh(&self.g, &self.store);
    }

    fn grab_now(&mut self, n: &mut Notch, r: RECT) {
        if let Ok(f) = grab(r) {
            self.remember(r);
            if let Some(id) = self.deliver(f, false, || capture::app_at(&capture::windows(), r)) {
                n.flash(&self.g, &self.store, id);
            }
        } else {
            n.fail();
        }
    }

    fn record(&mut self, n: &mut Notch, r: RECT) {
        let main = self.main.0 as isize;
        let Ok(path) = self.store.new_path("mp4", &self.rec_app) else {
            self.rec_app.clear();
            n.fail();
            return;
        };
        match rec::Rec::start(self.inst, r, path, move |c| post(main, Work::Clip(c))) {
            Some(r) => {
                self.rec = Some(r);
                n.rec = Some(Instant::now());
                n.show(&self.g, &self.store, View::Rest);
                unsafe {
                    SetTimer(Some(self.main), T_REC, 250, None);
                }
            }
            None => {
                self.rec_app.clear();
                n.fail();
            }
        }
    }

    fn remember(&mut self, r: RECT) {
        self.store.settings.last = Some([r.left, r.top, r.right, r.bottom]);
        self.store.save_settings();
    }

    /// Nothing the user would lose or notice if the app restarted now.
    fn idle_for_update(&self, n: &Notch) -> bool {
        self.rec.is_none()
            && !self.scroll_pdf_busy()
            && self.overlay.is_none()
            && self.interval.is_none()
            && self.jobs == 0
            && !n.expanded()
            && QUEUE.lock().map(|q| q.is_none()).unwrap_or(false)
    }

    pub fn scroll_pdf_busy(&self) -> bool {
        self.scroll_pdf_stop.is_some()
    }

    pub fn scroll_pdf_stopping(&self) -> bool {
        self.scroll_pdf_stop.as_ref().is_some_and(|stop| stop.load(Relaxed))
    }

    pub fn stop_scroll_pdf(&mut self, n: &mut Notch) {
        if let Some(stop) = &self.scroll_pdf_stop {
            stop.store(true, Relaxed);
            n.repaint();
        }
    }

    fn start_scroll_pdf(&mut self, n: &mut Notch, region: RECT, target: isize, app: String) {
        if self.scroll_pdf_busy() || self.rec.is_some() || self.interval.is_some() {
            return n.pdf_error(&self.g, &self.store, "Another capture is running", "Stop it before starting Scroll to PDF.");
        }
        n.show(&self.g, &self.store, View::PdfScroll);
        if let Err(error) = scroll_pdf::valid_pane(region, n.island_px()) {
            let (title, hint) = error.details();
            return n.pdf_error(&self.g, &self.store, title, hint);
        }
        let path = match self.store.new_path("pdf", &app) {
            Ok(path) => path,
            Err(e) => {
                note(&format!("Scroll PDF output directory failed: {:?}", e.kind()));
                return n.pdf_error(&self.g, &self.store, "Cannot create the PDF folder", "Check the save folder and its permissions.");
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.scroll_pdf_stop = Some(stop.clone());
        start_esc_hook();
        let main = self.main.0 as isize;
        note(&format!("Scroll PDF pane: {} × {} physical pixels", region.right - region.left, region.bottom - region.top));
        std::thread::spawn(move || {
            let result = (|| {
                let captured = scroll_pdf::run(region, target, &stop)
                    .inspect_err(|e| note(&format!("Scroll PDF did not start: {e:?}")))
                    .map_err(scroll_pdf::End::start_details)?;
                stop.store(true, Relaxed); // no more scrolling; final encode/write
                let end = captured.end;
                let (pdf, pages) = pdf::scroll_pages(&captured.frame)
                    .ok_or(("Could not encode the PDF", "Try a smaller selection and check the launch log."))?;
                let thumb = captured.frame.thumb(store::THUMB.0, store::THUMB.1);
                let output = if end == scroll_pdf::End::NoMoreMovement { path } else {
                    let stem = path.file_stem().and_then(|n| n.to_str())
                        .ok_or(("Cannot name the partial PDF", "Choose a different save folder."))?;
                    path.with_file_name(format!("{stem} - partial.pdf"))
                };
                if output.exists() {
                    return Err(("PDF filename already exists", "The old file was kept. Start a new capture."));
                }
                store::write_file(&output, &pdf).map_err(|e| {
                    note(&format!("Scroll PDF write failed: {:?}", e.kind()));
                    ("Could not write the PDF", "Check free space and save-folder permissions.")
                })?;
                note(&format!("Scroll PDF saved {pages} pages: {end:?}"));
                Ok((output, thumb, app, end))
            })();
            post(main, Work::ScrollPdf(result));
        });
    }

    fn start(&mut self, n: &mut Notch, m: Mode) {
        if self.scroll_pdf_busy() { return; }
        if m == Mode::ScrollPdf && (self.rec.is_some() || self.interval.is_some() || self.count > 0) {
            return n.pdf_error(&self.g, &self.store, "Another capture is running", "Stop it before starting Scroll to PDF.");
        }
        if self.overlay.as_ref().is_some_and(|o| !o.flying()) {
            return;
        }
        if m == Mode::Clip && self.rec.is_some() {
            return self.stop_rec(n);
        }
        if let Some(mut o) = self.overlay.take() {
            o.close();
        }
        match m {
            Mode::Screen => {
                let p = cursor();
                let mons = monitors();
                let r = mons.iter().find(|m| m.has(p)).unwrap_or(&mons[0]).rect;
                self.grab_now(n, r);
            }
            Mode::All => self.grab_now(n, capture::desktop(&monitors())),
            Mode::Last if self.store.settings.last.is_some() => {
                let [left, top, right, bottom] = self.store.settings.last.unwrap();
                self.grab_now(n, RECT { left, top, right, bottom });
            }
            Mode::Delay => {
                self.count = self.store.settings.delay;
                n.show(&self.g, &self.store, View::Count(self.count));
                unsafe {
                    SetTimer(Some(self.main), T_COUNT, 1000, None);
                }
            }
            m => {
                let m = if m == Mode::Last { Mode::Region } else { m };
                match Overlay::open(&self.g, self.inst, m) {
                    Ok(o) => {
                        note(&format!("overlay_ms {:.1}", o.opened_ms));
                        self.overlay = Some(o);
                    }
                    Err(e) => {
                        note(&format!("overlay failed {e}"));
                        if m == Mode::ScrollPdf {
                            n.pdf_error(&self.g, &self.store, "Could not start the picker", "Use a regular foreground window; see the launch log.");
                        } else {
                            n.fail();
                        }
                    }
                }
            }
        }
    }

    /// The window hotkey: whatever is in front.
    fn front_window(&mut self, n: &mut Notch) {
        if self.scroll_pdf_busy() { return; }
        let h = unsafe { GetForegroundWindow() };
        if let Some(r) = capture::window_rect(h).and_then(|r| capture::intersect(&r, &capture::desktop(&monitors()))) {
            self.grab_now(n, r);
        }
    }

    fn finish(&mut self, n: &mut Notch, out: Outcome) {
        if matches!(out, Outcome::None) {
            return;
        }
        let Some(mut o) = self.overlay.take() else { return };
        match out {
            Outcome::None => {}
            Outcome::Cancel => {
                o.refocus();
                o.close();
            }
            Outcome::Color(c) => {
                o.refocus();
                o.close();
                let r = self.put_text(&format!("#{c:06X}"));
                n.copied_or_not(&self.g, &self.store, r);
            }
            Outcome::Shot(r) => match o.mode {
                Mode::ScrollPdf => {
                    let app = o.app(r);
                    let target = o.previous_window();
                    o.refocus();
                    o.close();
                    self.start_scroll_pdf(n, r, target, app);
                }
                Mode::Scroll => {
                    let app = o.app(r);
                    o.refocus();
                    o.close();
                    let main = self.main.0 as isize;
                    std::thread::spawn(move || {
                        post(main, scroll::run(r).map(|f| Work::Shot(f, app)).unwrap_or(Work::Failed));
                    });
                }
                Mode::Interval => {
                    o.refocus();
                    o.close();
                    self.interval = Some(r);
                    n.repaint();
                    unsafe {
                        SetTimer(Some(self.main), T_INTERVAL, self.store.settings.interval as u32 * 1000, None);
                    }
                    self.tick_interval(n);
                }
                Mode::Text => {
                    let f = o.take(&self.g, r);
                    o.refocus();
                    o.close();
                    let main = self.main.0 as isize;
                    std::thread::spawn(move || {
                        let t = ocr::words(&f).map(|w| ocr::text(&w)).unwrap_or_default();
                        post(main, if t.is_empty() { Work::Failed } else { Work::Text(t) });
                    });
                }
                Mode::Clip => {
                    self.rec_app = o.app(r);
                    o.refocus();
                    o.close();
                    self.record(n, r);
                }
                Mode::Float => {
                    let f = o.take(&self.g, r);
                    let app = o.app(r);
                    o.refocus();
                    o.close();
                    let p = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
                    let dpi = monitors().iter().find(|m| m.has(p)).map_or(96, |m| m.dpi);
                    self.remember(r);
                    let ok = self.float_at(&f, POINT { x: r.left, y: r.top }, 1.0, dpi);
                    match self.deliver(f, false, || app) {
                        Some(id) if ok => n.flash(&self.g, &self.store, id),
                        _ => n.fail(),
                    }
                }
                _ => {
                    let t = Instant::now();
                    let f = o.take(&self.g, r);
                    self.remember(r);
                    let id = self.deliver(f.clone(), false, || o.app(r));
                    note(&format!("release_ms {:.1}", t.elapsed().as_secs_f64() * 1e3));
                    if let Some(id) = id {
                        n.flash(&self.g, &self.store, id);
                        o.launch(f, r, n.landing());
                        self.overlay = Some(o);
                    } else {
                        o.close();
                        n.fail();
                    }
                }
            },
        }
        n.raise();
    }

    fn tick_interval(&mut self, n: &mut Notch) {
        let Some(r) = self.interval else { return };
        if let Ok(f) = grab(r) {
            if self.deliver(f, true, || capture::app_at(&capture::windows(), r)).is_some() {
                n.refresh(&self.g, &self.store);
            }
        }
    }
}

pub fn read_shot(p: &std::path::Path) -> Option<Frame> {
    std::fs::read(p).ok().and_then(|b| store::decode_png(&b))
}

fn post(main: isize, w: Work) {
    let p = Box::into_raw(Box::new(w));
    unsafe {
        if PostMessageW(Some(HWND(main as *mut _)), WM_WORK, WPARAM(0), LPARAM(p as isize)).is_err() {
            drop(Box::from_raw(p));
        }
    }
}

// ── Recording a hotkey ─────────────────────────────────────────────────────

type Hook = AtomicPtr<std::ffi::c_void>;
static HOOK: Hook = AtomicPtr::new(std::ptr::null_mut());
static PASTE_HOOK: Hook = AtomicPtr::new(std::ptr::null_mut());
static NOTCH_HOOK: Hook = AtomicPtr::new(std::ptr::null_mut());
static ESC_HOOK: Hook = AtomicPtr::new(std::ptr::null_mut());

/// Esc stops Scroll to PDF. Polling the key from the capture thread can miss a
/// quick tap between samples; this hook sees every press. The key still reaches
/// the foreground app.
fn start_esc_hook() {
    unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if code >= 0 && (w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN) {
                let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
                if k.vkCode == VK_ESCAPE.0 as u32 {
                    if let Some(stop) = app().and_then(|a| a.scroll_pdf_stop.as_ref()) {
                        stop.store(true, Relaxed);
                    }
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }
    unsafe {
        stop_hook(&ESC_HOOK);
        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) {
            ESC_HOOK.store(h.0, Relaxed);
        }
    }
}

fn start_key_hook() {
    stop_hook(&NOTCH_HOOK);
    unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if code >= 0 && (w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN) {
                let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
                let vk = VIRTUAL_KEY(k.vkCode as u16);
                if !keys::is_modifier(vk) {
                    // Either Windows key records as Win.
                    let m = keys::held_now();
                    if let Some(a) = app() {
                        let _ = PostMessageW(Some(a.main), WM_KEY, WPARAM(m as usize), LPARAM(k.vkCode as isize));
                    }
                    return LRESULT(1);
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }
    unsafe {
        stop_hook(&HOOK);
        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) {
            HOOK.store(h.0, Relaxed);
        }
    }
}

/// Listens for paste while Sequential Copy runs. A paste that would land on an
/// image that hasn't been swapped yet is held back and replayed after the swap;
/// auto-repeat of a held paste chord is dropped so it can't paste duplicates.
/// Every other key passes untouched.
fn start_paste_hook() {
    unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        use std::sync::atomic::AtomicBool;
        static HELD: AtomicBool = AtomicBool::new(false);
        unsafe {
            if code >= 0 {
                let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
                let vk = VIRTUAL_KEY(k.vkCode as u16);
                if vk == VK_V || vk == VK_INSERT {
                    let msg = w.0 as u32;
                    let press = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                    let ours = k.dwExtraInfo == REPLAY_TAG;
                    let repeat = HELD.swap(press, Relaxed) && press && !ours;
                    let m = keys::held_now();
                    let chord = if vk == VK_V {
                        m & MOD_CONTROL.0 != 0 && m & (MOD_ALT.0 | MOD_WIN.0) == 0
                    } else {
                        m & MOD_SHIFT.0 != 0 && m & (MOD_CONTROL.0 | MOD_ALT.0 | MOD_WIN.0) == 0
                    };
                    // Our replay sends its own Ctrl when the user already let go.
                    if press && (chord || ours) {
                        if repeat {
                            return LRESULT(1);
                        }
                        // try_lock: never wait inside a hook; an unknown state passes the key.
                        let decision = QUEUE.try_lock().ok().and_then(|mut q| q.as_mut().map(|q| q.key(Instant::now())));
                        if let Some(a) = app() {
                            let _ = PostMessageW(Some(a.main), WM_PASTED, WPARAM(0), LPARAM(0));
                        }
                        if decision == Some(paste_queue::Key::Defer) {
                            note("queue paste held");
                            return LRESULT(1);
                        }
                        note("queue paste passed");
                    }
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }
    unsafe {
        stop_hook(&PASTE_HOOK);
        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) {
            PASTE_HOOK.store(h.0, Relaxed);
        }
    }
}

/// Send one Ctrl+V on behalf of a paste that was held back.
fn replay_paste() {
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                dwExtraInfo: REPLAY_TAG,
                ..Default::default()
            },
        },
    };
    let ctrl = keys::held_now() & MOD_CONTROL.0 != 0;
    let mut inputs = Vec::new();
    if !ctrl {
        inputs.push(key(VK_CONTROL, false));
    }
    inputs.extend([key(VK_V, false), key(VK_V, true)]);
    if !ctrl {
        inputs.push(key(VK_CONTROL, true));
    }
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    note(&format!("queue replay sent {sent}/{}", inputs.len()));
}

fn stop_hook(h: &Hook) {
    let h = h.swap(std::ptr::null_mut(), Relaxed);
    if !h.is_null() {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(h));
        }
    }
}

/// Active only while the notch is expanded; intercepts notch shortcuts only when the cursor is inside it.
pub fn set_notch_hook(on: bool) {
    if testing() {
        return;
    }
    if !on {
        stop_hook(&NOTCH_HOOK);
        return;
    }
    if !NOTCH_HOOK.load(Relaxed).is_null() {
        return;
    }
    unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if code >= 0 && (w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN) {
                if let (Some(a), Some(n)) = (app(), notch()) {
                    if n.expanded() && n.in_island() && !n.modal && n.recording.is_none() && a.overlay.is_none() {
                        let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
                        let down = |v: VIRTUAL_KEY| GetAsyncKeyState(v.0 as i32) < 0;
                        let mods = keys::held(down);
                        if n.wants_key(a, mods, k.vkCode) {
                            // In a low-level hook key state is the state before this
                            // event. A held H should not flip save-only repeatedly.
                            if k.vkCode != VK_H.0 as u32 || !down(VK_H) {
                                let _ = PostMessageW(Some(a.main), WM_NOTCH_KEY, WPARAM(mods as usize), LPARAM(k.vkCode as isize));
                            }
                            return LRESULT(1);
                        }
                    }
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }
    unsafe {
        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) {
            NOTCH_HOOK.store(h.0, Relaxed);
        }
    }
}

// ── Launch at login ────────────────────────────────────────────────────────

const RUN: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

fn login_on() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, w!("Ledge Grab"), RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set_login(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let v: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                w!("Ledge Grab"),
                REG_SZ.0,
                Some(v.as_ptr() as *const _),
                v.len() as u32 * 2,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, w!("Ledge Grab"));
        }
    }
}

// ── What else is on screen ─────────────────────────────────────────────────

fn fullscreen(mon: &capture::Monitor) -> bool {
    unsafe {
        let h = GetForegroundWindow();
        if h.is_invalid() || IsZoomed(h).as_bool() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(h, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "XamlExplorerHostIslandWindow") {
            return false;
        }
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        let m = mon.rect;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// Is Personal Desk's own notch on screen?
fn desk_open() -> bool {
    unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
        unsafe {
            let mut class = [0u16; 16];
            let n = GetClassNameW(h, &mut class) as usize;
            if String::from_utf16_lossy(&class[..n]) != "TkTopLevel" || !IsWindowVisible(h).as_bool() {
                return BOOL(1);
            }
            let mut pid = 0;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut b = [0u16; 300];
                let mut len = b.len() as u32;
                let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(b.as_mut_ptr()), &mut len);
                let _ = CloseHandle(p);
                if ok.is_ok() && String::from_utf16_lossy(&b[..len as usize]).ends_with("Personal Desk.exe") {
                    *(out.0 as *mut bool) = true;
                    return BOOL(0);
                }
            }
        }
        BOOL(1)
    }
    let mut found = false;
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

fn pick_folder(owner: HWND) -> Option<PathBuf> {
    use windows::Win32::System::Com::*;
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        d.SetOptions(d.GetOptions().ok()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        d.Show(Some(owner)).ok()?;
        let p = d.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

// ── Messages ───────────────────────────────────────────────────────────────

pub extern "system" fn plain(h: HWND, msg: u32, wp: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wp, l) }
}

pub extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, l: LPARAM) -> LRESULT {
    let fallback = || unsafe { DefWindowProcW(h, msg, wp, l) };
    let (Some(a), Some(n)) = (app(), notch()) else { return fallback() };
    if let Some(k) = a.overlay.as_ref().and_then(|o| o.owns(h)) {
        if msg == WM_SETCURSOR {
            return LRESULT(1);
        }
        let ours = matches!(
            msg,
            WM_NULL | WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MOUSEWHEEL | WM_CHAR | WM_KEYDOWN
        );
        if !ours {
            return fallback();
        }
        // Lend the overlay out so it can read the app.
        let mut o = a.overlay.take().unwrap();
        let out = o.message(a, k, msg, wp, l);
        a.overlay = Some(o);
        a.finish(n, out);
        return LRESULT(0);
    }
    if let Some(k) = a.floats.iter().position(|f| f.hwnd == h) {
        if msg == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        if a.floats[k].message(&a.g, msg, wp, l) {
            a.floats.remove(k).close();
            return LRESULT(0);
        }
        return fallback();
    }
    if h != a.main {
        return fallback();
    }
    match msg {
        WM_HOTKEY => match wp.0 {
            1 => a.start(n, Mode::Region),
            2 => a.start(n, Mode::Screen),
            4 => a.start(n, Mode::Clip),
            _ => a.front_window(n),
        },
        WM_RUN => {
            if let Some(m) = Mode::ALL.get(wp.0) {
                a.start(n, *m);
            }
        }
        updater::WM_UPDATE_READY => {
            let r = unsafe { *Box::from_raw(l.0 as *mut updater::Ready) };
            a.update = Some(r);
            let every = if std::env::var_os("LEDGE_GRAB_UPDATE_EVERY_SECS").is_some() { 2000 } else { 15_000 };
            unsafe { SetTimer(Some(a.main), T_UPDATE, every, None) };
        }
        WM_WORK => {
            let w = unsafe { *Box::from_raw(l.0 as *mut Work) };
            match w {
                Work::Shot(f, app) => {
                    if let Some(id) = a.deliver(f, false, || app) {
                        n.flash(&a.g, &a.store, id);
                    }
                }
                Work::Saved(id, true) => a.store.saved(id),
                Work::Saved(id, false) => {
                    note("shot save failed");
                    let on_clipboard = a.clip.contains(&id);
                    // The image itself may still be on the clipboard; only the shelf entry goes.
                    a.store.save_failed(id);
                    a.clip.retain(|i| *i != id);
                    n.refresh(&a.g, &a.store);
                    let hint = if on_clipboard {
                        "It is still on the clipboard. Check free space and the save folder."
                    } else {
                        "Check free space and the save folder."
                    };
                    n.notice(&a.g, &a.store, "Couldn't save the screenshot", hint);
                }
                Work::Name(id, t) => {
                    // Never under a drag in progress.
                    if !n.modal {
                        a.name(id, &t);
                    }
                }
                Work::Clip(c) => {
                    // Ended by itself: the recorder hit its limit or failed.
                    if a.rec.is_some() {
                        a.stop_rec(n);
                    }
                    a.jobs -= 1;
                    let app = std::mem::take(&mut a.rec_app);
                    let id = c.and_then(|c| a.store.add(&c.path, Kind::Clip, &c.thumb, &app, c.secs).ok());
                    match id {
                        Some(id) => {
                            let _ = a.copy(&[id]);
                            n.flash(&a.g, &a.store, id);
                        }
                        None => n.fail(),
                    }
                    if a.quitting {
                        a.quit(n);
                    }
                }
                Work::Gif(g) => {
                    a.jobs -= 1;
                    let id = g.and_then(|(p, from)| {
                        let thumb = a.store.thumb(from)?;
                        let i = a.store.get(from)?;
                        let (app, secs) = (i.export_app().to_owned(), i.secs);
                        a.store.add(&p, Kind::Gif, &thumb, &app, secs).ok()
                    });
                    match id {
                        Some(id) => {
                            let copied = a.copy(&[id]).ok();
                            n.flash(&a.g, &a.store, id);
                            if copied {
                                n.mark_copied(id);
                            }
                        }
                        None => n.fail(),
                    }
                }
                Work::ScrollPdf(result) => {
                    a.scroll_pdf_stop = None;
                    stop_hook(&ESC_HOOK);
                    match result {
                        Ok((path, thumb, app, end)) => {
                            if let Ok(id) = a.store.add(&path, Kind::Pdf, &thumb, &app, 0) {
                                n.pdf_saved(&a.g, &a.store, id, end != scroll_pdf::End::NoMoreMovement);
                            } else {
                                // The file is already safely written; don't delete it on an index error.
                                n.pdf_error(&a.g, &a.store, "PDF saved, shelf index failed", "Look in the preview save folder for your PDF.");
                            }
                        }
                        Err((title, hint)) => n.pdf_error(&a.g, &a.store, title, hint),
                    }
                    if a.quitting { a.quit(n); }
                }
                Work::Text(t) => {
                    let r = a.put_text(&t);
                    n.copied_or_not(&a.g, &a.store, r);
                }
                Work::Failed => n.fail(),
            }
        }
        WM_KEY => {
            if let Some(row) = n.recording.take() {
                stop_hook(&HOOK);
                let vk = l.0 as u32;
                if vk != VK_ESCAPE.0 as u32 {
                    let k = if vk == VK_BACK.0 as u32 { (0, 0) } else { (wp.0 as u32, vk) };
                    let s = &mut a.store.settings;
                    *[&mut s.region, &mut s.screen, &mut s.window, &mut s.clip][row] = k;
                    a.store.save_settings();
                }
                a.register();
                set_notch_hook(n.expanded());
                n.sync_auto_close();
                n.repaint();
            }
        }
        WM_NOTCH_KEY => n.key(a, wp.0 as u32, l.0 as u32),
        // A paste key was seen; the swap waits for the target's read.
        WM_PASTED => n.repaint(),
        WM_QUEUE_STEP => a.queue_step(n, code_then(wp.0)),
        WM_REPLAY => {
            if a.queue_active() {
                replay_paste();
            }
        }
        WM_DRAG_OUT => {
            let ids = std::mem::take(&mut a.outgoing);
            let files: Vec<PathBuf> = ids.iter().filter_map(|i| a.file(*i)).collect();
            n.modal = true;
            n.show(&a.g, &a.store, View::Rest);
            let main = a.main;
            clip::drag(main, &files);
            if let Some(n) = notch() {
                n.modal = false;
            }
        }
        WM_PICK_FOLDER => {
            n.modal = true;
            n.sync_auto_close();
            let f = pick_folder(a.main);
            if let (Some(a), Some(n)) = (app(), notch()) {
                n.modal = false;
                if let Some(f) = f {
                    a.store.settings.folder = f;
                    a.store.save_settings();
                }
                n.repaint();
                n.sync_auto_close();
            }
        }
        WM_TIMER => match wp.0 {
            T_TICK => {
                let desk = desk_open();
                n.adapt(&a.g, desk);
                let hide = a.overlay.is_none() && !a.scroll_pdf_busy()
                    && !matches!(n.view, View::PdfError(..) | View::PdfSaved(..))
                    && fullscreen(&n.mon) && !testing();
                n.hide(hide);
                if a.register() {
                    n.repaint();
                }
                let f = unsafe { GetForegroundWindow() };
                if f != a.last_foreground {
                    a.last_foreground = f;
                    if !hide && a.overlay.is_none() {
                        n.raise();
                    }
                }
            }
            T_COUNT => {
                a.count -= 1;
                if a.count > 0 {
                    n.show(&a.g, &a.store, View::Count(a.count));
                } else {
                    unsafe {
                        let _ = KillTimer(Some(h), T_COUNT);
                    }
                    n.show(&a.g, &a.store, View::Rest);
                    a.start(n, Mode::Region);
                }
            }
            T_INTERVAL => a.tick_interval(n),
            T_REC => n.repaint(),
            T_CLIP => {
                unsafe {
                    let _ = KillTimer(Some(h), T_CLIP);
                }
                a.take_clipboard(n);
            }
            T_PASTE => {
                // Notice a paste nobody read; the queue then waits for Next.
                match with_queue(|q| (q.phase, { q.tick(Instant::now()); q.phase })) {
                    None => unsafe {
                        let _ = KillTimer(Some(h), T_PASTE);
                    },
                    Some((was, now)) if was != now => note(&format!("queue stalled {:?}", match now {
                        paste_queue::Phase::Stalled(s) => format!("{s:?}"),
                        p => format!("{p:?}"),
                    })),
                    Some(_) => {}
                }
            }
            T_QUEUE_RETRY => {
                unsafe {
                    let _ = KillTimer(Some(h), T_QUEUE_RETRY);
                }
                a.queue_step(n, paste_queue::Then::Place);
            }
            T_DOTS => {
                if n.view == View::Rest {
                    n.repaint();
                }
            }
            T_UPDATE => {
                if a.update.is_some() && a.idle_for_update(n) {
                    unsafe {
                        let _ = KillTimer(Some(h), T_UPDATE);
                    }
                    let r = a.update.take().expect("checked");
                    if updater::apply(&r) {
                        a.quit(n);
                    }
                }
            }
            T_HOURLY => {
                let _ = a.store.reload();
                n.refresh(&a.g, &a.store);
            }
            T_TRIM => {
                if !n.expanded() && a.overlay.is_none() {
                    unsafe {
                        let _ = KillTimer(Some(h), T_TRIM);
                    }
                    // The clipboard keeps its own copy; the file serves PNG from here.
                    if let (Some(ClipSource::Frame(_)), [id]) = (&a.png, a.clip.as_slice()) {
                        if let Some(i) = a.store.get(*id).filter(|i| i.path.exists()) {
                            a.png = Some(ClipSource::File(i.path.clone()));
                        }
                    }
                    n.trim(&a.store);
                    a.g.trim();
                }
            }
            id => {
                if n.timer(a, id) {
                    unsafe {
                        SetTimer(Some(h), T_TRIM, 4000, None);
                    }
                }
            }
        },
        WM_CLIPBOARDUPDATE => {
            let ours = unsafe { GetClipboardOwner() }.is_ok_and(|o| o == a.main);
            if !ours {
                a.end_queue();
                a.clip.clear();
                a.png = None;
                n.repaint();
                if a.store.settings.history {
                    // Apps put their formats on one by one.
                    unsafe {
                        SetTimer(Some(h), T_CLIP, 80, None);
                    }
                }
            }
        }
        WM_RENDERFORMAT => {
            let format = wp.0 as u32;
            let reader_win = unsafe { GetOpenClipboardWindow() }.ok();
            // Which app asked: a process name only, for diagnosing early readers.
            let reader = reader_win.map(capture::app_name).unwrap_or_default();
            // Clipboard monitors (Mouse Without Borders, sync tools) read every new
            // image at once, and can read again while a paste is pending. Rendering
            // for them would cache the data so the real paste never reaches us, or
            // count their read as the paste and skip an image. While Sequential Copy
            // runs, a read from a background process gets nothing and the promise
            // stays open for the app that was pasted into.
            let background = reader_win.is_some_and(|w| {
                let pid = window_pid(w);
                pid != 0 && pid != unsafe { GetCurrentProcessId() } && pid != window_pid(unsafe { GetForegroundWindow() })
            });
            if background && with_queue(|_| ()).is_some() {
                note(&format!("queue declined background read format {format} by '{reader}'"));
                return LRESULT(0);
            }
            if let Some(b) = a.png.as_ref().and_then(|p| p.bytes(format)) {
                clip::render(format, &b);
            }
            // The first read of a queued image: a paste has really happened. The
            // reader holds the clipboard open now, so swap after this returns.
            if let Some(then) = with_queue(|q| q.read()) {
                note(&format!("queue read format {format} by '{reader}': {then:?}"));
                let swapping = with_queue(|q| q.phase == paste_queue::Phase::Swapping).unwrap_or(false);
                if then == paste_queue::Then::Place || (then == paste_queue::Then::Nothing && swapping) {
                    // Browsers read a paste in several trips (formats, then data).
                    // Swapping straight away hands the later trips the next image
                    // or nothing, so wait until the reads go quiet. Each further read
                    // restarts the wait.
                    unsafe {
                        SetTimer(Some(h), T_QUEUE_RETRY, paste_queue::SETTLE.as_millis() as u32, None);
                    }
                } else if then != paste_queue::Then::Nothing {
                    unsafe {
                        let _ = PostMessageW(Some(h), WM_QUEUE_STEP, WPARAM(then_code(then)), LPARAM(0));
                    }
                }
                n.repaint();
            }
        }
        WM_RENDERALLFORMATS => {
            if let Some(src) = &a.png {
                clip::render_all(h, |format| src.bytes(format));
            }
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            let desk = desk_open();
            n.adapt(&a.g, desk);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            stop_hook(&HOOK);
            stop_hook(&PASTE_HOOK);
            stop_hook(&NOTCH_HOOK);
            stop_hook(&ESC_HOOK);
            a.unregister();
            unsafe { PostQuitMessage(0) };
        }
        _ => {
            if !notch::message(n, a, msg, wp, l) {
                return fallback();
            }
        }
    }
    // Whatever started, advanced or ended Sequential Copy, the ledge follows.
    n.sync_queue(&a.g, &a.store);
    LRESULT(0)
}

/// GPU reset: start clean. History is on disk, so nothing is lost.
fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--restarted").spawn();
    }
    std::process::exit(0)
}

/// A GUI binary otherwise gives no indication when a launch stops before the ledge exists.
fn preview_launch_error(stage: &str, detail: &str) -> ! {
    let message = format!("{stage}\n\n{detail}\n\nIf you used Start Preview.cmd, you can share preview-launch.log.");
    note(&format!("Launch stopped at {stage}: {detail}"));
    let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = MessageBoxW(None, windows::core::PCWSTR(wide.as_ptr()), w!("Ledge Grab Scroll to PDF Preview"),
            MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND);
    }
    std::process::exit(2);
}

fn startup_result<T, E: std::fmt::Display>(value: Result<T, E>, stage: &str) -> T {
    match value {
        Ok(value) => value,
        Err(err) if scroll_pdf_preview() => preview_launch_error(stage, &err.to_string()),
        Err(err) => panic!("{stage}: {err}"),
    }
}

fn composition_failed(err: windows::core::Error) -> ! {
    if scroll_pdf_preview() {
        preview_launch_error("DirectComposition could not draw the preview", &err.to_string());
    }
    restart()
}

pub fn run() {
    unsafe {
        let preview = scroll_pdf_preview();
        if preview {
            note("Preview process started");
        }
        let name = if testing() { w!("Local\\LedgeGrabTest") } else { w!("Local\\LedgeGrab") };
        let restarted = std::env::args().any(|a| a == "--restarted");
        // An update relaunch may wait while the old copy hands its clipboard over.
        let wait_tries = if std::env::args().any(|a| a.starts_with("--updated=")) { 300 } else { 40 };
        let mut lock = CreateMutexW(None, true, name);
        let mut tries = 0;
        while GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(h) = lock {
                let _ = CloseHandle(h);
            }
            tries += 1;
            if !restarted || tries > wait_tries {
                if preview {
                    preview_launch_error("Ledge Grab is already running",
                        "Quit the regular Ledge Grab or an earlier preview from its ledge menu, then launch the preview again. No files were changed.");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            lock = CreateMutexW(None, true, name);
        }
        if preview {
            if let Err(err) = &lock { preview_launch_error("Could not claim the Ledge Grab instance lock", &err.to_string()); }
            if monitors().is_empty() { preview_launch_error("No desktop monitor found", "Open the preview from a normal Windows desktop session."); }
            note("Instance lock acquired");
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = OleInitialize(None);
        let inst: HINSTANCE = startup_result(GetModuleHandleW(None), "Could not identify the application").into();
        let g = startup_result(Gfx::new(), "Could not initialize Windows graphics");
        let store = startup_result(Store::open(), "Could not open the preview data folder or index");
        let n = Box::leak(Box::new(startup_result(Notch::new(&g, inst), "Could not create the ledge UI")));
        let a = Box::leak(Box::new(App {
            g,
            store,
            inst,
            main: n.sensor,
            clip: Vec::new(),
            png: None,
            hot: [false; 4],
            rec: None,
            rec_app: String::new(),
            floats: Vec::new(),
            jobs: 0,
            seen: 0,
            quitting: false,
            interval: None,
            scroll_pdf_stop: None,
            login: !scroll_pdf_preview() && login_on(),
            outgoing: Vec::new(),
            count: 0,
            overlay: None,
            last_foreground: HWND::default(),
            update: None,
        }));
        let (ap, np) = (a as *mut App, n as *mut Notch);
        APP.store(ap, Relaxed);
        NOTCH.store(np, Relaxed);
        {
            let (a, n) = (&mut *ap, &mut *np);
            a.register();
            if !scroll_pdf_preview() { let _ = AddClipboardFormatListener(a.main); }
            SetTimer(Some(a.main), T_TICK, 1500, None);
            SetTimer(Some(a.main), T_HOURLY, 3_600_000, None);
            SetTimer(Some(a.main), T_DOTS, 60_000, None);
            SetTimer(Some(a.main), T_TRIM, 4000, None);
            n.adapt(&a.g, desk_open());
            n.start();
            if !preview && !testing() {
                note(&format!("Ledge Grab {} started", crate::update::Version::current()));
                match updater::after_relaunch() {
                    Some(Ok(v)) => {
                        let hint: &'static str = Box::leak(format!("You're now on version {v}.").into_boxed_str());
                        n.notice(&a.g, &a.store, "Ledge Grab updated", hint);
                    }
                    Some(Err(_)) => n.notice(&a.g, &a.store, "Update didn't install", "Ledge Grab will try again tomorrow."),
                    None => {}
                }
                updater::spawn(a.main);
            }
            if preview {
                // Draw before telling the user it's ready; a successful launch
                // never relies on noticing the normal 9-pixel collapsed strip.
                if let Err(err) = n.draw_at(a, Instant::now()) { composition_failed(err); }
                note("Ready: ledge initialized");
                let _ = MessageBoxW(None,
                    w!("Preview is ready.\n\nHover or click the ledge at the top center of your primary screen, then choose + > Scroll to PDF.\n\nThis preview does not add a tray icon."),
                    w!("Ledge Grab Scroll to PDF Preview"), MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND);
            }
        }
        let mut msg = MSG::default();
        let mut trace = FrameTrace::start();
        loop {
            let input_start = Instant::now();
            let mut drained = true;
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    if let Some(trace) = &mut trace {
                        trace.flush();
                    }
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if input_start.elapsed() >= INPUT_SLICE {
                    drained = false;
                    break;
                }
            }
            let (a, n) = (&mut *ap, &mut *np);
            let now = Instant::now();
            let draw_notch = n.busy(now);
            let mut draw_overlay = false;
            if let Some(mut o) = a.overlay.take() {
                if o.landed(now) {
                    o.close();
                } else {
                    draw_overlay = o.busy(now);
                    if draw_overlay {
                        // Submit both surfaces from the same animation sample, with
                        // at most one vsynced Present (the last surface submitted).
                        if let Err(err) = o.draw_at(&a.g, now, !draw_notch) {
                            composition_failed(err);
                        }
                    }
                    a.overlay = Some(o);
                }
            }
            if draw_notch {
                if let Err(err) = n.draw_at(a, now) {
                    composition_failed(err);
                }
            }
            if let Some(trace) = &mut trace {
                if draw_notch || draw_overlay {
                    trace.record(now, draw_notch, draw_overlay);
                }
            }
            if drained && !draw_notch && !draw_overlay {
                if let Some(trace) = &mut trace {
                    trace.flush();
                }
                let _ = WaitMessage();
            }
        }
    }
}
