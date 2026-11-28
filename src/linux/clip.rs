//! Clipboard images on GNOME. One `ImageProvider` owns the clipboard and serves
//! `image/png` on demand. Every request it answers is a paste by some app, which is
//! exactly what Sequential Copy needs: after each answer the app may swap in the
//! next image *inside the same provider*, so the clipboard never has to be taken
//! again. That matters on Wayland, where an unfocused app can't take it.
//!
//! When another app takes the clipboard, GTK detaches the provider; `on_lost` fires.
use gtk4::gdk;
use gtk4::gdk::subclass::prelude::*;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;

pub const PNG: &str = "image/png";

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ImageProvider {
        /// PNG bytes served for the next request; None serves an error.
        pub bytes: RefCell<Option<std::rc::Rc<Vec<u8>>>>,
        pub on_read: RefCell<Option<Box<dyn Fn()>>>,
        pub on_lost: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImageProvider {
        const NAME: &'static str = "LedgeGrabImageProvider";
        type Type = super::ImageProvider;
        type ParentType = gdk::ContentProvider;
    }

    impl ObjectImpl for ImageProvider {}

    impl ContentProviderImpl for ImageProvider {
        fn formats(&self) -> gdk::ContentFormats {
            gdk::ContentFormats::new(&[PNG])
        }

        fn detach_clipboard(&self, clipboard: &gdk::Clipboard) {
            self.parent_detach_clipboard(clipboard);
            if let Some(f) = self.on_lost.borrow().as_ref() {
                f();
            }
        }

        fn write_mime_type_future(
            &self,
            mime: &str,
            stream: &gio::OutputStream,
            io_priority: glib::Priority,
        ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'static>> {
            let bytes = (mime == PNG).then(|| self.bytes.borrow().clone()).flatten();
            let stream = stream.clone();
            let obj = self.obj().clone();
            Box::pin(async move {
                let Some(b) = bytes else {
                    return Err(glib::Error::new(gio::IOErrorEnum::NotFound, "no image on the clipboard"));
                };
                let data: Vec<u8> = (*b).clone();
                stream.write_all_future(data, io_priority).await.map_err(|(_, e)| e)?;
                stream.close_future(io_priority).await?;
                // The image went out: tell the app (Sequential Copy advances here).
                if let Some(f) = obj.imp().on_read.borrow().as_ref() {
                    f();
                }
                Ok(())
            })
        }
    }
}

glib::wrapper! {
    pub struct ImageProvider(ObjectSubclass<imp::ImageProvider>) @extends gdk::ContentProvider;
}

impl ImageProvider {
    pub fn new(png: Option<Vec<u8>>) -> Self {
        let p: Self = glib::Object::new();
        p.set_png(png);
        p
    }

    /// What the next paste receives. Doesn't touch clipboard ownership.
    pub fn set_png(&self, png: Option<Vec<u8>>) {
        *self.imp().bytes.borrow_mut() = png.map(std::rc::Rc::new);
    }

    pub fn has_png(&self) -> bool {
        self.imp().bytes.borrow().is_some()
    }

    pub fn on_read(&self, f: impl Fn() + 'static) {
        *self.imp().on_read.borrow_mut() = Some(Box::new(f));
    }

    pub fn on_lost(&self, f: impl Fn() + 'static) {
        *self.imp().on_lost.borrow_mut() = Some(Box::new(f));
    }
}

/// Take the clipboard with `p`. Returns false if GTK refused.
pub fn own(clipboard: &gdk::Clipboard, p: &ImageProvider) -> bool {
    clipboard.set_content(Some(p.upcast_ref::<gdk::ContentProvider>())).is_ok()
}

/// Ask the Ledge Grab Shell extension to put a saved PNG on the clipboard. On
/// Wayland only the focused app may take the clipboard, and after a shortcut
/// capture that's someone else; the shell itself may always set it.
pub fn shell_set(path: &std::path::Path) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return false };
    let Some(p) = path.to_str() else { return false };
    let r = conn.call_sync(
        Some("org.gnome.Shell"),
        "/org/ledgegrab/Shell",
        "org.ledgegrab.Shell",
        "SetClipboardImage",
        Some(&(p,).to_variant()),
        Some(glib::VariantTy::new("(b)").expect("type")),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    );
    matches!(r.ok().and_then(|v| v.child_value(0).get::<bool>()), Some(true))
}
