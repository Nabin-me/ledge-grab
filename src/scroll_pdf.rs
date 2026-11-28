//! User-initiated Windows capture of one foreground scroll pane. No upload or background capture.
use crate::capture::{Frame, grab, intersect, monitors};
use crate::scroll_pdf_core::{self, Stability, Step};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, GetForegroundWindow, GetWindowRect, IsWindow, SetCursorPos, WindowFromPoint,
};

const MIN_WIDTH: i32 = 120;
const MIN_HEIGHT: i32 = 200;
const MAX_FRAME_PIXELS: i64 = 4_500_000;
const MAX_OUTPUT_PIXELS: usize = 36_000_000;
const MAX_OUTPUT_ROWS: usize = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    NoMoreMovement,
    UserStopped,
    TargetChanged,
    NotForeground,
    PaneCovered,
    MouseMoved,
    UnstableFrame,
    NoSafeOverlap,
    CaptureFailed,
    Limit,
    NoMovementFromStart,
}

pub struct Captured {
    pub frame: Frame,
    pub end: End,
}

#[derive(Clone, Copy, Debug)]
pub enum PaneError { TooSmall, TooLarge, OverLedge, NotOnOneDisplay }

impl PaneError {
    pub fn details(self) -> (&'static str, &'static str) {
        match self {
            Self::TooSmall => ("Selection is too small", "Select a wider pane at least 120 × 200 pixels."),
            Self::TooLarge => ("Selection is too large", "Select just the message column, not the whole display."),
            Self::OverLedge => ("Selection covers the ledge", "Start the selection below the top-edge controls."),
            Self::NotOnOneDisplay => ("Selection crosses displays", "Keep the whole pane on one monitor."),
        }
    }
}

impl End {
    pub fn start_details(self) -> (&'static str, &'static str) {
        match self {
            Self::NotForeground => ("Target window not active", "Click the target window, then try Scroll to PDF again."),
            Self::PaneCovered | Self::TargetChanged => ("Pane not fully visible", "Keep the selected pane visible inside one foreground window."),
            Self::MouseMoved => ("Mouse moved before capture", "Leave the mouse still after releasing the selection."),
            Self::UnstableFrame => ("Content did not settle", "The view changed before the first scroll. Try the included test pane or let loading finish."),
            Self::UserStopped => ("Stopped before the first page", "Try again and allow the first view to settle."),
            _ => ("Could not capture the pane", "Use a regular window on the desktop and try again."),
        }
    }
}

pub fn valid_pane(r: RECT, controls: RECT) -> Result<(), PaneError> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w < MIN_WIDTH || h < MIN_HEIGHT { return Err(PaneError::TooSmall); }
    if i64::from(w) * i64::from(h) > MAX_FRAME_PIXELS { return Err(PaneError::TooLarge); }
    if intersect(&r, &controls).is_some() { return Err(PaneError::OverLedge); }
    if !monitors().iter().any(|m|
        r.left >= m.rect.left && r.right <= m.rect.right && r.top >= m.rect.top && r.bottom <= m.rect.bottom)
    { return Err(PaneError::NotOnOneDisplay); }
    Ok(())
}

fn mouse() -> Option<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some(point)
}

fn same_point(a: POINT, b: POINT) -> bool {
    (a.x - b.x).abs() <= 1 && (a.y - b.y).abs() <= 1
}

fn root(h: HWND) -> HWND {
    unsafe { GetAncestor(h, GA_ROOT) }
}

fn check_pane(target: HWND, r: RECT, win: Option<RECT>) -> Result<(), End> {
    unsafe {
        if !IsWindow(Some(target)).as_bool() || root(GetForegroundWindow()) != target {
            return Err(End::NotForeground);
        }
        // The pane rectangle is fixed at the start; a moved or resized window
        // would put other content under it.
        if let Some(win) = win {
            let mut now = RECT::default();
            if GetWindowRect(target, &mut now).is_err() || now != win {
                return Err(End::TargetChanged);
            }
        }
        // A 5 x 5 grid, edges included, so a small window over the middle of the
        // pane is caught here instead of surfacing later as a failed seam.
        let (w, h) = (r.right - r.left - 5, r.bottom - r.top - 5);
        for i in 0..5 {
            for j in 0..5 {
                let p = POINT { x: r.left + 2 + w * i / 4, y: r.top + 2 + h * j / 4 };
                if root(WindowFromPoint(p)) != target {
                    return Err(End::PaneCovered);
                }
            }
        }
        Ok(())
    }
}

fn window_rect(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok().map(|_| r)
}

fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Relaxed) || (unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0)
}

fn settle(
    r: RECT, win: Option<RECT>, stop: &AtomicBool, target: HWND, parked: POINT, wheel: usize,
) -> Result<Frame, End> {
    let start = Instant::now();
    let (mut samples, mut broad, mut local) = (0, 0, 0);
    let mut anchor: Option<Frame> = None;
    let mut consecutive = 0;
    while start.elapsed() < Duration::from_millis(2550) {
        if stopped(stop) {
            return Err(End::UserStopped);
        }
        check_pane(target, r, win)?;
        if !mouse().is_some_and(|p| same_point(p, parked)) {
            return Err(End::MouseMoved);
        }
        let frame = grab(r).map_err(|_| End::CaptureFailed)?;
        // Reject a focus/pointer change that happened during the capture too.
        check_pane(target, r, win)?;
        if !mouse().is_some_and(|p| same_point(p, parked)) {
            return Err(End::MouseMoved);
        }
        samples += 1;
        if let Some(old) = &anchor {
            match scroll_pdf_core::stability(&old.px, &frame.px, frame.w as usize) {
                Stability::BroadChange => broad += 1,
                kind => {
                    if kind == Stability::LocalChange { local += 1; }
                    consecutive += 1;
                    if consecutive >= 2 {
                        if broad > 0 || local > 0 {
                            crate::note(&format!("Scroll PDF stability: wheel={wheel} samples={samples} broad={broad} local={local} elapsed_ms={}", start.elapsed().as_millis()));
                        }
                        // Take the latest sample, not the anchor from before the wait.
                        return Ok(frame);
                    }
                    std::thread::sleep(Duration::from_millis(75));
                    continue;
                }
            }
        }
        // Compare each sample against a fixed anchor so gradual drift can't
        // pass solely because adjacent frames differ by a little.
        anchor = Some(frame);
        consecutive = 0;
        std::thread::sleep(Duration::from_millis(75));
    }
    crate::note(&format!("Scroll PDF settle timeout: wheel={wheel} samples={samples} broad={broad} local={local} elapsed_ms={}", start.elapsed().as_millis()));
    Err(End::UnstableFrame)
}

fn wheel_once() -> bool {
    let i = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { mouseData: (-120i32) as u32, dwFlags: MOUSEEVENTF_WHEEL, ..Default::default() },
        },
    };
    unsafe { SendInput(&[i], size_of::<INPUT>() as i32) == 1 }
}

/// cursor restoration must also run if initial capture fails.
struct Pointer { before: POINT, parked: POINT }

impl Drop for Pointer {
    fn drop(&mut self) {
        if mouse().is_some_and(|p| same_point(p, self.parked)) {
            unsafe { let _ = SetCursorPos(self.before.x, self.before.y); }
        }
    }
}

pub fn run(r: RECT, hwnd: isize, stop: &AtomicBool) -> Result<Captured, End> {
    let target = root(HWND(hwnd as *mut _));
    if target.0.is_null() {
        return Err(End::TargetChanged);
    }
    // Hover near the edge of the selected content instead of directly over
    // the conversation text, where chat apps often open a hover toolbar.
    let parked = POINT { x: r.left + ((r.right - r.left) / 32).max(6), y: r.top + (r.bottom - r.top) / 2 };
    // Overlay destruction and Windows foreground restoration are asynchronous.
    // Give the selected window a short chance to return, without sending input
    // or redirecting the mouse until the whole pane passes the check.
    std::thread::sleep(Duration::from_millis(270));
    let mut target_check = check_pane(target, r, None);
    for _ in 0..12 {
        if stopped(stop) { return Err(End::UserStopped); }
        if target_check.is_ok() { break; }
        std::thread::sleep(Duration::from_millis(55));
        target_check = check_pane(target, r, None);
    }
    target_check?;
    let win = window_rect(target);
    if stopped(stop) { return Err(End::UserStopped); }
    if !mouse().is_some_and(|p| p.x >= r.left - 20 && p.x <= r.right + 20 && p.y >= r.top - 20 && p.y <= r.bottom + 20) {
        return Err(End::MouseMoved);
    }
    let before = mouse().ok_or(End::CaptureFailed)?;
    if root(unsafe { WindowFromPoint(parked) }) != target { return Err(End::PaneCovered); }
    unsafe { SetCursorPos(parked.x, parked.y).map_err(|_| End::CaptureFailed)?; }
    let _pointer = Pointer { before, parked };
    let mut last = settle(r, win, stop, target, parked, 0)?;
    let mut stitched = last.clone();
    let width = last.w as usize;
    let height = last.h as usize;
    let stride = width * 4;
    let deadline = Instant::now() + Duration::from_secs(60);
    let (mut steady, mut moved) = (0, false);
    let mut end = End::Limit;
    let mut steps = 0;
    for wheel in 1..=180 {
        steps = wheel;
        if stopped(stop) {
            end = End::UserStopped;
            break;
        }
        if Instant::now() >= deadline { break; }
        if let Err(reason) = check_pane(target, r, win) { end = reason; break; }
        if !mouse().is_some_and(|p| same_point(p, parked)) {
            end = End::MouseMoved;
            break;
        }
        // Ctrl-wheel would zoom instead of scrolling and Shift-wheel can go sideways.
        if [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0) {
            end = End::UserStopped;
            break;
        }
        if !wheel_once() {
            end = End::CaptureFailed;
            break;
        }
        std::thread::sleep(Duration::from_millis(105));
        let current = match settle(r, win, stop, target, parked, wheel) {
            Ok(f) => f,
            Err(reason) => { end = reason; break; }
        };
        match scroll_pdf_core::compare(&last.px, &current.px, width, height) {
            Step::Still => {
                steady += 1;
                if steady == 3 {
                    end = if moved { End::NoMoreMovement } else { End::NoMovementFromStart };
                    break;
                }
            }
            Step::Forward(n) => {
                let rows = stitched.h as usize + n;
                if rows > MAX_OUTPUT_ROWS || rows * width > MAX_OUTPUT_PIXELS { break; }
                stitched.px.extend_from_slice(&current.px[(height - n) * stride..]);
                stitched.h = rows as i32;
                last = current;
                steady = 0;
                moved = true;
            }
            Step::Uncertain => {
                end = End::NoSafeOverlap;
                break;
            }
        }
    }
    // Where and why it ended; no pixels or text.
    crate::note(&format!("Scroll PDF ended: wheel={steps} reason={end:?} rows={}", stitched.h));
    Ok(Captured { frame: stitched, end })
}
