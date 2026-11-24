//! A picture held above everything: drag it, scroll to size, Ctrl scroll to fade.
use crate::capture::Frame;
use crate::gfx::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

const CLOSE: f32 = 24.0;
const MIN: f32 = 48.0;

pub struct Float {
    pub hwnd: HWND,
    surf: Surface,
    bmp: Option<ID2D1Bitmap1>,
    /// Native pixels.
    size: (i32, i32),
    zoom: f32,
    alpha: f32,
    s: f32,
    /// Where in the window the drag began.
    grab: Option<POINT>,
    hover: bool,
}

fn rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

impl Float {
    /// `zoom` is relative to native pixels.
    pub fn open(g: &Gfx, inst: HINSTANCE, f: &Frame, at: POINT, zoom: f32, dpi: u32) -> Result<Float> {
        unsafe {
            let class = w!("LedgeGrabFloat");
            RegisterClassW(&WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(crate::wndproc),
                hInstance: inst,
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            });
            let (w, h) = ((f.w as f32 * zoom) as i32, (f.h as f32 * zoom) as i32);
            let (w, h) = (w.max(1), h.max(1));
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
                class,
                w!("Ledge Grab float"),
                WS_POPUP,
                at.x,
                at.y,
                w,
                h,
                None,
                None,
                Some(inst),
                None,
            )?;
            let surf = g.surface(hwnd, w, h)?;
            let mut fl = Float {
                hwnd,
                surf,
                bmp: None,
                size: (f.w, f.h),
                zoom,
                alpha: 1.0,
                s: dpi as f32 / 96.0,
                grab: None,
                hover: false,
            };
            let mut up = None;
            fl.surf.frame(g, false, |c| up = c.upload(f).ok())?;
            fl.bmp = up;
            fl.draw(g)?;
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            Ok(fl)
        }
    }

    pub fn close(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    fn cross(&self, w: f32) -> R {
        let k = CLOSE * self.s;
        R::new(w - k - 6.0 * self.s, 6.0 * self.s, k, k)
    }

    fn draw(&self, g: &Gfx) -> Result<()> {
        let r = rect(self.hwnd);
        let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        self.surf.frame(g, false, |c| {
            let full = R::new(0.0, 0.0, w, h);
            let rad = (w.min(h) * 0.06).min(9.0 * self.s);
            if let Some(b) = &self.bmp {
                c.image_round(b, full, rad, self.alpha);
            }
            c.stroke(full.grow(-0.5), rad, 1.0, BLACK, 0.45);
            c.stroke(full.grow(-1.5), (rad - 1.0).max(0.0), 1.0, INK, 0.22);
            if self.hover {
                let x = self.cross(w);
                c.circle(x.cx(), x.cy(), x.w / 2.0, BLACK, 0.8);
                c.icon(0xE711, x, 10.0 * self.s, INK, 1.0);
            }
        })
    }

    /// Returns true when it should go away.
    pub fn message(&mut self, g: &Gfx, msg: u32, wp: WPARAM, l: LPARAM) -> bool {
        let p = POINT { x: (l.0 & 0xffff) as i16 as i32, y: ((l.0 >> 16) & 0xffff) as i16 as i32 };
        let r = rect(self.hwnd);
        match msg {
            WM_MOUSEMOVE => {
                if let Some(from) = self.grab {
                    unsafe {
                        let _ = SetWindowPos(
                            self.hwnd,
                            None,
                            r.left + p.x - from.x,
                            r.top + p.y - from.y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                } else if !self.hover {
                    self.hover = true;
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    let _ = self.draw(g);
                }
            }
            WM_MOUSELEAVE => {
                self.hover = false;
                let _ = self.draw(g);
            }
            WM_LBUTTONDOWN => {
                if self.cross((r.right - r.left) as f32).has(p.x as f32, p.y as f32) {
                    return true;
                }
                self.grab = Some(p);
                unsafe {
                    SetCapture(self.hwnd);
                    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
                    let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
                }
            }
            WM_LBUTTONUP => {
                self.grab = None;
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            WM_LBUTTONDBLCLK | WM_RBUTTONUP | WM_MBUTTONUP => return true,
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
                if wp.0 & 0x8 != 0 {
                    self.alpha = (self.alpha + d * 0.08).clamp(0.15, 1.0);
                    let _ = self.draw(g);
                } else {
                    self.resize(g, self.zoom * 1.1f32.powf(d), p, r);
                }
            }
            _ => {}
        }
        false
    }

    fn resize(&mut self, g: &Gfx, zoom: f32, p: POINT, r: RECT) {
        let (fw, fh) = (self.size.0 as f32, self.size.1 as f32);
        let zoom = zoom.clamp((MIN / fw.min(fh)).min(1.0), (6000.0 / fw.max(fh)).clamp(1.0, 8.0));
        let (w, h) = ((fw * zoom).round().max(1.0), (fh * zoom).round().max(1.0));
        let (ow, oh) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        // Keep the point under the cursor where it is.
        let x = p.x as f32 - (p.x - r.left) as f32 * w / ow;
        let y = p.y as f32 - (p.y - r.top) as f32 * h / oh;
        self.zoom = zoom;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                x.round() as i32,
                y.round() as i32,
                w as i32,
                h as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        if self.surf.resize(w as i32, h as i32).is_ok() {
            let _ = self.draw(g);
        }
    }
}
