//! Screen capture: one BGRA frame per monitor, in physical pixels.
//!
//! Monitors on the GPU we already draw with are duplicated there, so the frozen frame
//! never leaves it. Others (laptops often split monitors across GPUs) go through GDI.
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, Result};

pub use crate::frame::*;

pub fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(out.0 as *mut Vec<Monitor>);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(h, &mut mi).as_bool() {
                let (mut dx, mut dy) = (96, 96);
                let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(
                    h,
                    windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI,
                    &mut dx,
                    &mut dy,
                );
                list.push(Monitor { handle: h.0 as isize, rect: mi.rcMonitor, primary: mi.dwFlags & 1 != 0, dpi: dx });
            }
        }
        BOOL(1)
    }
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut v as *mut _ as isize));
    }
    v.sort_by_key(|m| !m.primary);
    v
}

pub fn desktop(mons: &[Monitor]) -> RECT {
    mons.iter().fold(RECT { left: i32::MAX, top: i32::MAX, right: i32::MIN, bottom: i32::MIN }, |a, m| RECT {
        left: a.left.min(m.rect.left),
        top: a.top.min(m.rect.top),
        right: a.right.max(m.rect.right),
        bottom: a.bottom.max(m.rect.bottom),
    })
}

/// Any rectangle of the virtual desktop.
pub fn grab(r: RECT) -> Result<Frame> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let res = (|| {
            let bmp = CreateDIBSection(Some(screen), &bi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(mem, bmp.into());
            let ok = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY);
            let _ = GdiFlush();
            let mut f = Frame { w, h, px: Vec::new() };
            if ok.is_ok() {
                f.px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
            }
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            ok?;
            f.px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            Ok(f)
        })();
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        res
    }
}

/// Stitch a desktop rectangle out of per-monitor frames.
pub fn compose(mons: &[Monitor], frames: &[Frame], r: RECT) -> Frame {
    let mut out = Frame::new(r.right - r.left, r.bottom - r.top);
    out.px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    for (m, f) in mons.iter().zip(frames) {
        if let Some(i) = intersect(&m.rect, &r) {
            let part = f.crop(RECT {
                left: i.left - m.rect.left,
                top: i.top - m.rect.top,
                right: i.right - m.rect.left,
                bottom: i.bottom - m.rect.top,
            });
            out.paste(&part, i.left - r.left, i.top - r.top);
        }
    }
    out
}

/// Visible frame of a window, without its invisible resize border.
pub fn window_rect(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe {
        DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, size_of::<RECT>() as u32).ok()?;
    }
    (r.right - r.left > 8 && r.bottom - r.top > 8).then_some(r)
}

#[derive(Clone, Copy)]
pub struct Win {
    pub rect: RECT,
    hwnd: isize,
}

/// The app most of a rectangle belongs to: whatever is frontmost at its middle.
pub fn app_at(wins: &[Win], r: RECT) -> String {
    let p = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
    wins.iter().find(|w| inside(&w.rect, p)).map(|w| app_name(HWND(w.hwnd as *mut _))).unwrap_or_default()
}

pub fn exe_path(h: HWND) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Threading::*;
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut b = [0u16; 600];
        let mut len = b.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(b.as_mut_ptr()), &mut len);
        let _ = windows::Win32::Foundation::CloseHandle(p);
        ok.ok()?;
        Some(String::from_utf16_lossy(&b[..len as usize]).into())
    }
}

/// A hosted app's own window, not the ApplicationFrameHost wrapper.
fn hosted_window(frame: HWND) -> Option<HWND> {
    unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
        const CORE_WINDOW: &str = "Windows.UI.Core.CoreWindow";
        let mut name = [0u16; 64];
        let len = unsafe { GetClassNameW(h, &mut name) }.max(0) as usize;
        if name[..len].iter().copied().eq(CORE_WINDOW.encode_utf16()) {
            unsafe { *(out.0 as *mut Option<HWND>) = Some(h) };
            BOOL(0)
        } else {
            BOOL(1)
        }
    }
    let mut found = None;
    unsafe {
        let _ = EnumChildWindows(Some(frame), Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// "Google Chrome", from the program's own description.
pub fn app_name(h: HWND) -> String {
    use windows::Win32::Storage::FileSystem::*;
    use windows::core::{HSTRING, w};
    let Some(mut path) = exe_path(h) else { return String::new() };
    let stem_of = |p: &std::path::Path| p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let mut stem = stem_of(&path);
    if stem.eq_ignore_ascii_case("ApplicationFrameHost") {
        // Window titles can include private document names and change while
        // using one app. Never use them as a source-app or folder name.
        let Some(hosted) = hosted_window(h).and_then(exe_path) else { return "Windows app".into() };
        path = hosted;
        stem = stem_of(&path);
        if stem.eq_ignore_ascii_case("ApplicationFrameHost") {
            return "Windows app".into();
        }
    }
    if stem.eq_ignore_ascii_case("explorer") {
        return "Explorer".into();
    }
    let described = unsafe {
        let file = HSTRING::from(path.as_os_str());
        let n = GetFileVersionInfoSizeW(&file, None);
        let mut buf = vec![0u8; n as usize];
        (|| {
            if n == 0 {
                return None;
            }
            GetFileVersionInfoW(&file, None, n, buf.as_mut_ptr() as *mut _).ok()?;
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            VerQueryValueW(buf.as_ptr() as *const _, w!("\\VarFileInfo\\Translation"), &mut p, &mut len).ok().ok()?;
            if len < 4 {
                return None;
            }
            let t = *(p as *const [u16; 2]);
            let key = HSTRING::from(format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", t[0], t[1]));
            VerQueryValueW(buf.as_ptr() as *const _, &key, &mut p, &mut len).ok().ok()?;
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize));
            let s = s.trim_end_matches('\0').trim().to_string();
            (!s.is_empty() && s.chars().count() <= 32).then_some(s)
        })()
    };
    described.unwrap_or(stem)
}

/// Top-level windows, front to back, skipping our own process.
pub fn windows() -> Vec<Win> {
    unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(out.0 as *mut Vec<Win>);
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                return BOOL(1);
            }
            let mut pid = 0;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == std::process::id() {
                return BOOL(1);
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if cloaked != 0 || ex & WS_EX_TRANSPARENT.0 != 0 {
                return BOOL(1);
            }
            if let Some(r) = window_rect(h) {
                list.push(Win { rect: r, hwnd: h.0 as isize });
            }
        }
        BOOL(1)
    }
    let mut v: Vec<Win> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// A frame still on the GPU, plus a copy on its way to the CPU.
pub struct GpuFrame {
    pub tex: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    ctx: ID3D11DeviceContext,
    w: i32,
    h: i32,
}

impl GpuFrame {
    /// Freeze a monitor driven by `dev`'s own adapter. None: use `grab` instead.
    pub fn take(dev: &ID3D11Device, m: &Monitor) -> Option<GpuFrame> {
        unsafe {
            let ad = dev.cast::<IDXGIDevice>().ok()?.GetAdapter().ok()?;
            let mut i = 0;
            let out = loop {
                let o = ad.EnumOutputs(i).ok()?;
                i += 1;
                let d = o.GetDesc().ok()?;
                if d.Monitor.0 as isize == m.handle {
                    if d.Rotation.0 > DXGI_MODE_ROTATION_IDENTITY.0 {
                        return None;
                    }
                    break o;
                }
            };
            let dup = out.cast::<IDXGIOutput1>().ok()?.DuplicateOutput(dev).ok()?;
            let t = std::time::Instant::now();
            // The first frame can be empty; the next one is the real desktop.
            while t.elapsed().as_millis() < 60 {
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut res = None;
                dup.AcquireNextFrame(50, &mut info, &mut res).ok()?;
                if info.LastPresentTime == 0 {
                    let _ = dup.ReleaseFrame();
                    continue;
                }
                let src: ID3D11Texture2D = res?.cast().ok()?;
                let mut d = D3D11_TEXTURE2D_DESC::default();
                src.GetDesc(&mut d);
                if (d.Width as i32, d.Height as i32) != (m.w(), m.h()) || d.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
                    return None;
                }
                let ctx = dev.GetImmediateContext().ok()?;
                d.MiscFlags = 0;
                d.CPUAccessFlags = 0;
                d.Usage = D3D11_USAGE_DEFAULT;
                d.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                let mut tex = None;
                dev.CreateTexture2D(&d, None, Some(&mut tex)).ok()?;
                d.BindFlags = 0;
                d.Usage = D3D11_USAGE_STAGING;
                d.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                let mut staging = None;
                dev.CreateTexture2D(&d, None, Some(&mut staging)).ok()?;
                let (tex, staging) = (tex?, staging?);
                ctx.CopyResource(&tex, &src);
                ctx.CopyResource(&staging, &tex);
                return Some(GpuFrame { tex, staging, ctx, w: m.w(), h: m.h() });
            }
            None
        }
    }

    pub fn read(&self) -> Result<Frame> {
        unsafe {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
            let n = (self.w * 4) as usize;
            let mut px = Vec::with_capacity(n * self.h as usize);
            for y in 0..self.h as usize {
                px.extend_from_slice(std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), n));
            }
            self.ctx.Unmap(&self.staging, 0);
            px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            Ok(Frame { w: self.w, h: self.h, px })
        }
    }
}
