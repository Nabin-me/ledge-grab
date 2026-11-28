//! The GNOME app (Wayland and X11). Built up feature by feature; see docs/linux-port.md.
mod app;
mod canvas;
mod capture;
mod clip;
mod ledge;
mod picker;

pub fn run() {
    app::run();
}
