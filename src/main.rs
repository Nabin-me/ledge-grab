#![cfg_attr(windows, windows_subsystem = "windows")]
//! Ledge Grab: screenshots that live on the ledge.
//!
//! Platform-neutral modules live here; `win` holds the Windows app and `linux`
//! the GNOME one. The platform module is re-exported at the crate root, so
//! `crate::capture`, `crate::App` and friends resolve to the current platform.
mod anim;
mod frame;
mod geom;
mod ocr;
mod paste_queue;
mod pdf;
mod pointer_timing;
mod redact;
mod scroll_pdf_core;
mod secrets;
mod store;
// Installed only by the Windows app so far; the Linux build has no releases yet.
#[cfg_attr(not(windows), allow(dead_code))]
mod update;
mod vk;

#[cfg(windows)]
mod win;
#[cfg(windows)]
#[allow(unused_imports)]
pub use win::*;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
#[allow(unused_imports)]
pub use linux::*;

/// Private launch mode: no registered shortcuts or copied-image monitoring.
pub fn scroll_pdf_preview() -> bool {
    static PREVIEW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PREVIEW.get_or_init(|| option_env!("LEDGE_GRAB_SCROLL_PDF_PREVIEW_BUILD").is_some()
        || std::env::var_os("LEDGE_GRAB_SCROLL_PDF_PREVIEW").is_some())
}

pub fn testing() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("LEDGE_GRAB_TEST").is_some())
}

/// Test-only fault injection: make background shot writes fail.
fn save_should_fail() -> bool {
    testing() && std::env::var_os("LEDGE_GRAB_FAIL_SAVES").is_some()
}

/// Timings, only when a test asks for them.
pub fn note(s: &str) {
    if let Some(p) = std::env::var_os("LEDGE_GRAB_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{s}");
        }
    }
}

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(not(windows))]
    linux::run();
}
