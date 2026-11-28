//! The GNOME app: one running instance (later `ledge-grab --region` calls are
//! forwarded to it over D-Bus by GApplication), the ledge window, capture, copy
//! and Sequential Copy.
use super::canvas::{Canvas, Image};
use super::clip::{self, ImageProvider};
use super::ledge::{Action, Ledge, QueueView, View};
use super::{capture, picker};
use crate::frame::Frame;
use crate::note;
use crate::paste_queue::{Phase, Placed, Queue, Then};
use crate::store::{self, Kind, Store};
use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const APP_ID: &str = "org.ledgegrab.LedgeGrab";
/// Concave corner where the ledge meets the edge it hangs from.
const EAR: f32 = 14.0;
/// Grace before the shelf closes after the pointer leaves.
const LEAVE_MS: u64 = 450;
/// GNOME's clipboard manager copies new clipboard content (up to a few MB) the
/// moment an app takes the clipboard, so it survives the app quitting. That read
/// isn't a paste; nobody can paste this soon after clicking.
const MANAGER_READ_MS: u128 = 400;

struct State {
    store: Store,
    ledge: Ledge,
    thumbs: HashMap<i64, Rc<Image>>,
    window: gtk::ApplicationWindow,
    area: gtk::DrawingArea,
    /// The provider currently owning the clipboard, if we own it.
    provider: Option<ImageProvider>,
    queue: Option<Queue>,
    /// When we last took the clipboard.
    owned_at: Option<Instant>,
    leave: Option<glib::SourceId>,
    revert: Option<glib::SourceId>,
}

type S = Rc<RefCell<State>>;

pub fn run() {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let state: Rc<RefCell<Option<S>>> = Default::default();
    app.connect_command_line(move |app, cmd| {
        let s = match state.borrow().clone() {
            Some(s) => s,
            None => match build(app) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Ledge Grab could not start: {e}");
                    return 1;
                }
            },
        };
        *state.borrow_mut() = Some(s.clone());
        for a in cmd.arguments().iter().skip(1) {
            match a.to_string_lossy().as_ref() {
                "--region" => start_capture(&s, true),
                "--screen" => start_capture(&s, false),
                "--quit" => app.quit(),
                other => eprintln!("unknown option {other}"),
            }
        }
        0
    });
    app.run();
}

fn build(app: &gtk::Application) -> Result<S, String> {
    let store = Store::open().map_err(|e| format!("index: {e}"))?;
    let css = gtk::CssProvider::new();
    css.load_from_string("window.ledge, window.ledge > * { background: transparent; box-shadow: none; }");
    if let Some(d) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Ledge Grab")
        .decorated(false)
        .resizable(false)
        .build();
    window.add_css_class("ledge");
    let area = gtk::DrawingArea::new();
    window.set_child(Some(&area));
    let s: S = Rc::new(RefCell::new(State {
        store,
        ledge: Ledge::default(),
        thumbs: HashMap::new(),
        window: window.clone(),
        area: area.clone(),
        provider: None,
        queue: None,
        owned_at: None,
        leave: None,
        revert: None,
    }));

    let sd = Rc::downgrade(&s);
    area.set_draw_func(move |a, cr, _, _| {
        if let Some(s) = sd.upgrade() {
            draw(&s, a, cr);
        }
    });

    let motion = gtk::EventControllerMotion::new();
    let sd = Rc::downgrade(&s);
    motion.connect_enter(move |_, x, y| {
        let Some(s) = sd.upgrade() else { return };
        cancel_leave(&s);
        let open = matches!(s.borrow().ledge.view, View::Rest | View::Flash(_) | View::Notice(..));
        if open {
            set_view(&s, View::Shelf);
        }
        pointer(&s, x, y);
    });
    let sd = Rc::downgrade(&s);
    motion.connect_motion(move |_, x, y| {
        if let Some(s) = sd.upgrade() {
            pointer(&s, x, y);
        }
    });
    let sd = Rc::downgrade(&s);
    motion.connect_leave(move |_| {
        let Some(s) = sd.upgrade() else { return };
        cancel_leave(&s);
        let sd2 = Rc::downgrade(&s);
        let id = glib::timeout_add_local_once(Duration::from_millis(LEAVE_MS), move || {
            if let Some(s) = sd2.upgrade() {
                s.borrow_mut().leave = None;
                let v = s.borrow().ledge.view;
                if v == View::Shelf {
                    s.borrow_mut().ledge.hover = None;
                    set_view(&s, View::Rest);
                }
            }
        });
        s.borrow_mut().leave = Some(id);
    });
    area.add_controller(motion);

    let click = gtk::GestureClick::new();
    let sd = Rc::downgrade(&s);
    click.connect_released(move |_, _, x, y| {
        let Some(s) = sd.upgrade() else { return };
        let act = {
            let mut st = s.borrow_mut();
            let st = &mut *st;
            st.ledge.click(&st.store, x as f32 - EAR, y as f32)
        };
        act_on(&s, act);
    });
    area.add_controller(click);

    // Dots fade as screenshots age.
    let sd = Rc::downgrade(&s);
    glib::timeout_add_seconds_local(60, move || match sd.upgrade() {
        Some(s) => {
            s.borrow().area.queue_draw();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });

    resize(&s);
    window.present();
    note(&format!("ready: {} session", if capture::wayland() { "wayland" } else { "x11" }));
    Ok(s)
}

fn cancel_leave(s: &S) {
    if let Some(id) = s.borrow_mut().leave.take() {
        id.remove();
    }
}

fn pointer(s: &S, x: f64, y: f64) {
    let act = {
        let mut st = s.borrow_mut();
        let st = &mut *st;
        st.ledge.motion(&st.store, x as f32 - EAR, y as f32)
    };
    if act == Action::Redraw {
        s.borrow().area.queue_draw();
    }
}

fn set_view(s: &S, v: View) {
    {
        let mut st = s.borrow_mut();
        if st.ledge.view == v {
            return;
        }
        st.ledge.view = v;
        if let Some(id) = st.revert.take() {
            id.remove();
        }
    }
    note(&format!("view {v:?}"));
    resize(s);
}

/// Show `v` for `ms`, then go back to Rest (or the queue, if one is running).
fn flash(s: &S, v: View, ms: u64) {
    set_view(s, v);
    let sd = Rc::downgrade(s);
    let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
        if let Some(s) = sd.upgrade() {
            s.borrow_mut().revert = None;
            let back = if s.borrow().queue.is_some() { View::Queue } else { View::Rest };
            set_view(&s, back);
        }
    });
    s.borrow_mut().revert = Some(id);
}

fn resize(s: &S) {
    let st = s.borrow();
    let (w, h, _) = st.ledge.size(&st.store);
    let (cw, ch) = ((w + EAR * 2.0).ceil() as i32, h.ceil() as i32);
    st.area.set_content_width(cw);
    st.area.set_content_height(ch);
    st.window.set_default_size(cw, ch);
    st.area.queue_draw();
}

fn draw(s: &S, area: &gtk::DrawingArea, cr: &gtk::cairo::Context) {
    let scale = area.scale_factor().max(1) as f32;
    let (w, h) = (area.width() as f32, area.height() as f32);
    let Some(mut c) = Canvas::new(w, h, scale) else { return };
    {
        let mut st = s.borrow_mut();
        let st = &mut *st;
        let (store, thumbs) = (&st.store, &mut st.thumbs);
        let mut thumb = |id: i64| -> Option<Rc<Image>> {
            if let Some(i) = thumbs.get(&id) {
                return Some(i.clone());
            }
            let img = Rc::new(Image::from_frame(&store.thumb(id)?)?);
            thumbs.insert(id, img.clone());
            Some(img)
        };
        st.ledge.draw(&mut c, store, EAR, store::now_ms(), &mut thumb);
        dump_spots(st);
    }
    let (dw, dh) = c.device_size();
    let Ok(surf) = gtk::cairo::ImageSurface::create_for_data(c.cairo_bytes(), gtk::cairo::Format::ARgb32, dw, dh, dw * 4) else {
        return;
    };
    cr.set_operator(gtk::cairo::Operator::Source);
    cr.scale(1.0 / scale as f64, 1.0 / scale as f64);
    let _ = cr.set_source_surface(&surf, 0.0, 0.0);
    let _ = cr.paint();
}

/// Test sessions only: write the clickable spots (window coordinates) where the
/// harness can find them, so it can click buttons without knowing the layout.
fn dump_spots(st: &State) {
    let Some(p) = std::env::var_os("LEDGE_GRAB_SPOTS") else { return };
    let mut out = format!("view {:?}
", st.ledge.view);
    for (b, r) in st.ledge.spots(&st.store) {
        out += &format!("{b:?} {} {} {} {}
", r.x + EAR, r.y, r.w, r.h);
    }
    let _ = std::fs::write(p, out);
}

fn act_on(s: &S, act: Action) {
    match act {
        Action::None => {}
        Action::Redraw => s.borrow().area.queue_draw(),
        Action::Copy(ids) => {
            let ok = ids.first().and_then(|id| png_of(&s.borrow().store, *id)).map(|png| put(s, png)).unwrap_or(false);
            note(&format!("copy {ids:?}: {}", if ok { "ok" } else { "failed" }));
            if !ok {
                flash(s, View::Notice("Couldn't copy that screenshot", "The file may be missing. Nothing was copied."), 2600);
            }
        }
        Action::Sequential(ids) => start_queue(s, ids),
        Action::Delete(ids) => {
            let paths = s.borrow_mut().store.remove(&ids);
            for p in paths {
                let _ = std::fs::remove_file(p);
            }
            for id in &ids {
                s.borrow_mut().thumbs.remove(id);
            }
            note(&format!("deleted {ids:?}"));
            resize(s);
        }
        Action::Region => start_capture(s, true),
        Action::Screen => start_capture(s, false),
        Action::QueueNext => {
            let then = s.borrow_mut().queue.as_mut().map(|q| q.next());
            if let Some(t) = then {
                queue_step(s, t);
            }
        }
        Action::QueueStop => end_queue(s, "stopped"),
    }
}

fn png_of(st: &Store, id: i64) -> Option<Vec<u8>> {
    let item = st.get(id)?;
    if item.kind != Kind::Shot {
        return None;
    }
    std::fs::read(&item.path).ok()
}

/// Own the clipboard with a fresh provider holding `png`.
fn put(s: &S, png: Vec<u8>) -> bool {
    let p = ImageProvider::new(Some(png));
    watch(s, &p);
    let Some(d) = gdk::Display::default() else { return false };
    if !clip::own(&d.clipboard(), &p) {
        return false;
    }
    let old = {
        let mut st = s.borrow_mut();
        st.owned_at = Some(Instant::now());
        st.provider.replace(p)
    };
    drop(old);
    true
}

/// Route a provider's reads and its loss of the clipboard back to the app.
fn watch(s: &S, p: &ImageProvider) {
    let sd = Rc::downgrade(s);
    p.on_read(move || {
        let sd = sd.clone();
        glib::idle_add_local_once(move || {
            if let Some(s) = sd.upgrade() {
                queue_read(&s);
            }
        });
    });
    let sd = Rc::downgrade(s);
    let me = p.downgrade();
    p.on_lost(move || {
        let (sd, me) = (sd.clone(), me.clone());
        glib::idle_add_local_once(move || {
            let (Some(s), Some(me)) = (sd.upgrade(), me.upgrade()) else { return };
            // Only a loss of the provider that currently owns the clipboard counts;
            // replacing our own provider detaches the old one too.
            let ours = s.borrow().provider.as_ref() == Some(&me);
            if ours {
                s.borrow_mut().provider = None;
                note("clipboard taken by another app");
                if s.borrow().queue.is_some() {
                    end_queue(&s, "clipboard replaced elsewhere");
                }
            }
        });
    });
}

// ── Sequential Copy ─────────────────────────────────────────────────────────

fn start_queue(s: &S, ids: Vec<i64>) {
    if ids.len() < 2 {
        return;
    }
    let first = png_of(&s.borrow().store, ids[0]);
    let Some(png) = first else {
        flash(s, View::Notice("Couldn't start Sequential Copy", "The first image is missing."), 2600);
        return;
    };
    if !put(s, png) {
        flash(s, View::Notice("Couldn't start Sequential Copy", "The clipboard is busy. Try again."), 2600);
        return;
    }
    note(&format!("queue start {ids:?}"));
    s.borrow_mut().queue = Some(Queue::new(ids));
    sync_queue(s);
    s.borrow_mut().ledge.picked.clear();
    set_view(s, View::Queue);
}

/// An app read the clipboard image. On Linux every read is a paste, so it arms
/// and advances the queue in one go.
fn queue_read(s: &S) {
    let manager = s.borrow().owned_at.is_some_and(|t| t.elapsed().as_millis() < MANAGER_READ_MS);
    if manager {
        if s.borrow().queue.is_some() {
            note("queue read ignored: clipboard manager copy");
        }
        return;
    }
    let then = {
        let mut st = s.borrow_mut();
        let Some(q) = st.queue.as_mut() else { return };
        let _ = q.key(Instant::now());
        if !matches!(q.phase, Phase::Pressed(_)) {
            // Clipboard holds nothing usable (a missing item): nothing to advance.
            return;
        }
        q.read()
    };
    note(&format!("queue read: {then:?}"));
    queue_step(s, then);
}

fn queue_step(s: &S, then: Then) {
    match then {
        Then::Place | Then::Retry => {
            let (id, png) = {
                let st = s.borrow();
                let Some(id) = st.queue.as_ref().and_then(|q| q.current()) else { return };
                (id, png_of(&st.store, id))
            };
            let placed = match (png, s.borrow().provider.clone()) {
                (Some(png), Some(p)) => {
                    p.set_png(Some(png));
                    Placed::Ok
                }
                (None, Some(p)) => {
                    // Never let the previous image paste twice.
                    p.set_png(None);
                    Placed::Missing
                }
                (_, None) => Placed::Cleared,
            };
            note(&format!("queue placed {id}: {placed:?}"));
            let next = s.borrow_mut().queue.as_mut().map(|q| q.placed(placed));
            sync_queue(s);
            if let Some(Then::Retry) = next {
                queue_step(s, Then::Retry);
            }
        }
        Then::Finished => end_queue(s, "finished"),
        Then::Nothing | Then::Replay => sync_queue(s),
    }
}

fn sync_queue(s: &S) {
    let mut st = s.borrow_mut();
    st.ledge.queue = st.queue.as_ref().map(|q| {
        let (pos, len) = q.progress();
        QueueView {
            pos,
            len,
            current: q.current(),
            stall: match q.phase {
                Phase::Stalled(x) => Some(x),
                _ => None,
            },
        }
    });
    st.area.queue_draw();
}

fn end_queue(s: &S, why: &str) {
    if s.borrow_mut().queue.take().is_none() {
        return;
    }
    note(&format!("queue ended: {why}"));
    s.borrow_mut().ledge.queue = None;
    set_view(s, View::Rest);
}

// ── Capture ────────────────────────────────────────────────────────────────

fn start_capture(s: &S, region: bool) {
    // Keep the ledge out of its own screenshot.
    s.borrow().window.set_visible(false);
    let sd = Rc::downgrade(s);
    glib::timeout_add_local_once(Duration::from_millis(180), move || {
        let Some(s) = sd.upgrade() else { return };
        let sd = Rc::downgrade(&s);
        capture::screen(move |r| {
            let Some(s) = sd.upgrade() else { return };
            match r {
                Ok(f) if region => {
                    let sd = Rc::downgrade(&s);
                    let app = s.borrow().window.application();
                    picker::pick(app.as_ref(), f, move |cut| {
                        if let Some(s) = sd.upgrade() {
                            s.borrow().window.set_visible(true);
                            match cut {
                                Some(f) => keep(&s, f),
                                None => note("region canceled"),
                            }
                        }
                    });
                }
                Ok(f) => {
                    s.borrow().window.set_visible(true);
                    keep(&s, f);
                }
                Err(e) => {
                    s.borrow().window.set_visible(true);
                    note(&format!("capture failed: {e}"));
                    flash(&s, View::Notice("Screenshot failed", "GNOME didn't hand over the screen. Try again."), 3200);
                }
            }
        });
    });
}

/// File a capture, put it on the clipboard and show it on the ledge.
fn keep(s: &S, f: Frame) {
    let png = store::encode_png(&f);
    let thumb = f.thumb(store::THUMB.0, store::THUMB.1);
    let added = {
        let mut st = s.borrow_mut();
        st.store.new_path("png", "Screenshot").ok().and_then(|path| {
            store::write_file(&path, &png).ok()?;
            let id = st.store.add(&path, Kind::Shot, &thumb, "Screenshot", 0).ok()?;
            Some((id, path))
        })
    };
    let Some((id, path)) = added else {
        note("capture save failed");
        flash(s, View::Notice("Couldn't save the screenshot", "Check the Pictures folder and free space."), 3200);
        return;
    };
    // Wayland: we're not focused after a shortcut capture, so the shell sets it.
    let copied = if capture::wayland() && clip::shell_set(&path) {
        s.borrow_mut().provider = None;
        "shell"
    } else if put(s, png) {
        "app"
    } else {
        "no"
    };
    note(&format!("captured {}x{} as {id}; copied: {copied}", f.w, f.h));
    flash(s, View::Flash(id), 1600);
}
