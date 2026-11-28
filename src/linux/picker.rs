//! Region picker: the frozen screenshot fills the screen; drag a rectangle,
//! release to keep it, Esc or a click without a drag to cancel.
use crate::frame::Frame;
use crate::geom::RECT;
use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub fn pick(app: Option<&gtk::Application>, f: Frame, done: impl FnOnce(Option<Frame>) + 'static) {
    let win = gtk::Window::builder().title("Ledge Grab region").decorated(false).build();
    if let Some(a) = app {
        win.set_application(Some(a));
    }
    let area = gtk::DrawingArea::new();
    win.set_child(Some(&area));
    let frame = Rc::new(f);
    let texture = texture(&frame);
    let sel: Rc<Cell<Option<(f64, f64, f64, f64)>>> = Rc::new(Cell::new(None));
    let done: Rc<RefCell<Option<Box<dyn FnOnce(Option<Frame>)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));

    let (t2, s2) = (texture.clone(), sel.clone());
    area.set_draw_func(move |_, cr, w, h| {
        if let Some(t) = &t2 {
            let (tw, th) = (t.width() as f64, t.height() as f64);
            cr.save().ok();
            cr.scale(w as f64 / tw, h as f64 / th);
            let surf = cairo_from_texture(t);
            if let Some(surf) = surf {
                let _ = cr.set_source_surface(&surf, 0.0, 0.0);
                let _ = cr.paint();
            }
            cr.restore().ok();
        }
        // Dim everything outside the selection.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        if let Some((x0, y0, x1, y1)) = s2.get() {
            let (l, t, r, b) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
            cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            cr.rectangle(l, t, r - l, b - t);
            let _ = cr.fill();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
            cr.set_line_width(1.5);
            cr.rectangle(l + 0.75, t + 0.75, r - l - 1.5, b - t - 1.5);
            let _ = cr.stroke();
        } else {
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill();
        }
    });

    let finish = {
        let (win, done) = (win.clone(), done.clone());
        Rc::new(move |cut: Option<Frame>| {
            win.close();
            if let Some(d) = done.borrow_mut().take() {
                d(cut);
            }
        })
    };

    let drag = gtk::GestureDrag::new();
    let (s3, a3) = (sel.clone(), area.clone());
    drag.connect_drag_begin(move |_, x, y| {
        s3.set(Some((x, y, x, y)));
        a3.queue_draw();
    });
    let (s4, a4) = (sel.clone(), area.clone());
    drag.connect_drag_update(move |g, dx, dy| {
        if let Some((x, y)) = g.start_point() {
            s4.set(Some((x, y, x + dx, y + dy)));
            a4.queue_draw();
        }
    });
    let (s5, a5, fr, fin) = (sel.clone(), area.clone(), frame.clone(), finish.clone());
    drag.connect_drag_end(move |_, _, _| {
        let Some((x0, y0, x1, y1)) = s5.get() else { return fin(None) };
        let (w, h) = (a5.width().max(1) as f64, a5.height().max(1) as f64);
        let (kx, ky) = (fr.w as f64 / w, fr.h as f64 / h);
        let r = RECT {
            left: (x0.min(x1) * kx).round() as i32,
            top: (y0.min(y1) * ky).round() as i32,
            right: (x0.max(x1) * kx).round() as i32,
            bottom: (y0.max(y1) * ky).round() as i32,
        };
        if r.right - r.left < 4 || r.bottom - r.top < 4 {
            return fin(None);
        }
        fin(Some(fr.crop(r)));
    });
    area.add_controller(drag);

    let keys = gtk::EventControllerKey::new();
    let fin = finish.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            fin(None);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);
    win.set_cursor_from_name(Some("crosshair"));
    win.fullscreen();
    win.present();
    crate::note("region picker open");
}

fn texture(f: &Frame) -> Option<gdk::MemoryTexture> {
    let bytes = glib::Bytes::from(&f.px[..]);
    Some(gdk::MemoryTexture::new(f.w, f.h, gdk::MemoryFormat::B8g8r8a8, &bytes, (f.w * 4) as usize))
}

fn cairo_from_texture(t: &gdk::MemoryTexture) -> Option<gtk::cairo::ImageSurface> {
    let (w, h) = (t.width(), t.height());
    let mut surf = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).ok()?;
    let stride = surf.stride() as usize;
    {
        let mut data = surf.data().ok()?;
        t.download(&mut data, stride);
    }
    Some(surf)
}
