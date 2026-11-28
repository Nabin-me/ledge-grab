//! Whole-screen capture on GNOME. X11 reads the root window directly. Wayland
//! asks the Ledge Grab Shell extension (no prompt), or failing that the
//! Screenshot portal (GNOME asks the user once), reads the PNG it saved, and
//! deletes that file again.
use crate::frame::Frame;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Capture every monitor as one frame; `done` runs on the main loop.
pub fn screen(done: impl FnOnce(Result<Frame, String>) + 'static) {
    if wayland() {
        shell(done);
    } else {
        done(x11_root());
    }
}

fn x11_root() -> Result<Frame, String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};
    let (conn, n) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    let root = &conn.setup().roots[n];
    let (w, h) = (root.width_in_pixels, root.height_in_pixels);
    let img = conn
        .get_image(ImageFormat::Z_PIXMAP, root.root, 0, 0, w, h, !0)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("X11 GetImage: {e}"))?;
    if img.depth < 24 || img.data.len() < w as usize * h as usize * 4 {
        return Err(format!("unsupported X11 depth {}", img.depth));
    }
    let mut f = Frame { w: w as i32, h: h as i32, px: img.data };
    f.px.truncate(w as usize * h as usize * 4);
    for px in f.px.chunks_exact_mut(4) {
        px[3] = 255; // BGRX -> opaque BGRA
    }
    Ok(f)
}

/// Screenshot through the Ledge Grab Shell extension; the portal if that fails.
fn shell(done: impl FnOnce(Result<Frame, String>) + 'static) {
    let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(c) => c,
        Err(e) => return done(Err(format!("session bus: {e}"))),
    };
    let dir = glib::user_runtime_dir();
    let path = dir.join(format!("ledge-grab-{}.png", std::process::id()));
    let p = path.to_string_lossy().to_string();
    conn.call(
        Some("org.gnome.Shell"),
        "/org/ledgegrab/Shell",
        "org.ledgegrab.Shell",
        "Screenshot",
        Some(&(p,).to_variant()),
        Some(glib::VariantTy::new("(b)").expect("type")),
        gio::DBusCallFlags::NONE,
        10_000,
        gio::Cancellable::NONE,
        move |r| match r.ok().and_then(|v| v.child_value(0).get::<bool>()) {
            Some(true) => done(load_path_and_remove(&path)),
            _ => {
                let _ = std::fs::remove_file(&path);
                crate::note("shell screenshot unavailable; using the portal");
                portal(done)
            }
        },
    );
}

fn portal(done: impl FnOnce(Result<Frame, String>) + 'static) {
    let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(c) => c,
        Err(e) => return done(Err(format!("session bus: {e}"))),
    };
    let token = format!("ledgegrab{}", std::process::id() as u64 * 1000 + (glib::monotonic_time() as u64 % 1000));
    let sender = conn.unique_name().map(|s| s.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let request = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let done = std::rc::Rc::new(std::cell::RefCell::new(Some(Box::new(done) as Box<dyn FnOnce(Result<Frame, String>)>)));
    let sub: std::rc::Rc<std::cell::RefCell<Option<gio::SignalSubscriptionId>>> = Default::default();
    let (d2, s2, c2) = (done.clone(), sub.clone(), conn.clone());
    // Subscribe before calling, so a fast reply can't be missed.
    let id = conn.signal_subscribe(
        Some("org.freedesktop.portal.Desktop"),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&request),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            if let Some(id) = s2.borrow_mut().take() {
                c2.signal_unsubscribe(id);
            }
            let Some(done) = d2.borrow_mut().take() else { return };
            let code = params.child_value(0).get::<u32>().unwrap_or(2);
            if code != 0 {
                return done(Err(if code == 1 { "Screenshot was declined".into() } else { "Screenshot portal failed".into() }));
            }
            let results = glib::VariantDict::new(Some(&params.child_value(1)));
            let uri: Option<String> = results.lookup("uri").ok().flatten();
            done(uri.ok_or_else(|| "portal gave no file".to_string()).and_then(|u| load_and_remove(&u)));
        },
    );
    *sub.borrow_mut() = Some(id);
    let opts = glib::VariantDict::new(None);
    opts.insert("handle_token", &token);
    opts.insert("interactive", &false);
    let args = glib::Variant::tuple_from_iter([String::new().to_variant(), opts.end()]);
    let (d3, s3, c3) = (done.clone(), sub.clone(), conn.clone());
    conn.call(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
        "Screenshot",
        Some(&args),
        Some(glib::VariantTy::new("(o)").expect("type")),
        gio::DBusCallFlags::NONE,
        30_000,
        gio::Cancellable::NONE,
        move |r| {
            if let Err(e) = r {
                if let Some(id) = s3.borrow_mut().take() {
                    c3.signal_unsubscribe(id);
                }
                if let Some(done) = d3.borrow_mut().take() {
                    done(Err(format!("Screenshot portal: {e}")));
                }
            }
        },
    );
}

fn load_and_remove(uri: &str) -> Result<Frame, String> {
    let path = gio::File::for_uri(uri).path().ok_or("portal file is not local")?;
    load_path_and_remove(&path)
}

fn load_path_and_remove(path: &std::path::Path) -> Result<Frame, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read screenshot: {e}"))?;
    // The portal saved a copy in the user's Pictures; this capture is ours to file.
    let _ = std::fs::remove_file(path);
    crate::store::decode_png(&bytes).ok_or_else(|| "screenshot is not a PNG".into())
}
