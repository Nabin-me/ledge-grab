//! Clipboard, recycle bin and drag-out.
use crate::capture::Frame;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER, LCS_GM_IMAGES};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_DIBV5, CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::*;
use windows::core::{PCWSTR, Result, w};

pub fn png_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

unsafe fn global(len: usize, write: impl FnOnce(*mut u8)) -> Result<HANDLE> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, len)?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        write(p);
        let _ = GlobalUnlock(h);
        Ok(HANDLE(h.0))
    }
}

fn open(hwnd: HWND) -> bool {
    for _ in 0..25 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    false
}

/// The bitmap goes on now; PNG is promised and rendered only when an app asks.
pub fn set_image(hwnd: HWND, f: &Frame) -> bool {
    if f.w < 1 || f.h < 1 || !open(hwnd) {
        return false;
    }
    unsafe {
        let _ = EmptyClipboard();
        let head = size_of::<BITMAPV5HEADER>();
        let n = (f.w * 4) as usize;
        let ok = global(head + f.px.len(), |p| {
            let h = BITMAPV5HEADER {
                bV5Size: head as u32,
                bV5Width: f.w,
                // bottom-up: some apps ignore the top-down flag
                bV5Height: f.h,
                bV5Planes: 1,
                bV5BitCount: 32,
                bV5Compression: BI_BITFIELDS,
                bV5SizeImage: f.px.len() as u32,
                bV5RedMask: 0x00ff0000,
                bV5GreenMask: 0x0000ff00,
                bV5BlueMask: 0x000000ff,
                bV5AlphaMask: 0xff000000,
                bV5CSType: 0x73524742, // sRGB
                bV5Intent: LCS_GM_IMAGES as u32,
                ..Default::default()
            };
            std::ptr::copy_nonoverlapping(&h as *const _ as *const u8, p, head);
            for y in 0..f.h {
                std::ptr::copy_nonoverlapping(f.row(f.h - 1 - y).as_ptr(), p.add(head + y as usize * n), n);
            }
        })
        .and_then(|h| SetClipboardData(CF_DIBV5.0 as u32, Some(h)))
        .is_ok();
        let _ = SetClipboardData(png_format(), None);
        let _ = CloseClipboard();
        ok
    }
}

/// Answer WM_RENDERFORMAT. The clipboard is already open then.
pub fn render_png(png: &[u8]) {
    unsafe {
        if let Ok(h) = global(png.len(), |p| std::ptr::copy_nonoverlapping(png.as_ptr(), p, png.len())) {
            let _ = SetClipboardData(png_format(), Some(h));
        }
    }
}

/// Answer WM_RENDERALLFORMATS as the app goes away.
pub fn render_png_final(hwnd: HWND, png: &[u8]) {
    unsafe {
        if open(hwnd) {
            if GetClipboardOwner().is_ok_and(|o| o == hwnd) {
                render_png(png);
            }
            let _ = CloseClipboard();
        }
    }
}

pub fn set_files(hwnd: HWND, files: &[PathBuf]) -> bool {
    if files.is_empty() || !open(hwnd) {
        return false;
    }
    let mut names: Vec<u16> = Vec::new();
    for f in files {
        use std::os::windows::ffi::OsStrExt;
        names.extend(f.as_os_str().encode_wide());
        names.push(0);
    }
    names.push(0);
    unsafe {
        let _ = EmptyClipboard();
        let head = size_of::<DROPFILES>();
        let ok = global(head + names.len() * 2, |p| {
            let d = DROPFILES { pFiles: head as u32, fWide: true.into(), ..Default::default() };
            std::ptr::copy_nonoverlapping(&d as *const _ as *const u8, p, head);
            std::ptr::copy_nonoverlapping(names.as_ptr() as *const u8, p.add(head), names.len() * 2);
        })
        .and_then(|h| SetClipboardData(CF_HDROP.0 as u32, Some(h)))
        .is_ok();
        let _ = CloseClipboard();
        ok
    }
}

pub fn set_text(hwnd: HWND, s: &str) -> bool {
    if !open(hwnd) {
        return false;
    }
    let t: Vec<u16> = s.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = EmptyClipboard();
        let ok = global(t.len() * 2, |p| std::ptr::copy_nonoverlapping(t.as_ptr() as *const u8, p, t.len() * 2))
            .and_then(|h| SetClipboardData(CF_UNICODETEXT.0 as u32, Some(h)))
            .is_ok();
        let _ = CloseClipboard();
        ok
    }
}

fn wide(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain([0]).collect()
}

pub fn recycle(files: &[PathBuf]) {
    let mut from: Vec<u16> = Vec::new();
    for f in files.iter().filter(|f| f.exists()) {
        from.extend(wide(f));
    }
    if from.is_empty() {
        return;
    }
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
        ..Default::default()
    };
    unsafe {
        SHFileOperationW(&mut op);
    }
}

pub fn open_file(p: &Path) {
    let f = wide(p);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(f.as_ptr()),
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

/// Blocks until the drop ends.
pub fn drag(hwnd: HWND, files: &[PathBuf]) {
    unsafe {
        let ids: Vec<*const Common::ITEMIDLIST> = files
            .iter()
            .map(|f| {
                let p = wide(f);
                ILCreateFromPathW(PCWSTR(p.as_ptr())) as *const _
            })
            .filter(|p: &*const Common::ITEMIDLIST| !p.is_null())
            .collect();
        if ids.is_empty() {
            return;
        }
        let _: Result<DROPEFFECT> = (|| {
            let arr = SHCreateShellItemArrayFromIDLists(&ids)?;
            let data: windows::Win32::System::Com::IDataObject = arr.BindToHandler(None, &BHID_DataObject)?;
            SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY)
        })();
        for p in ids {
            ILFree(Some(p));
        }
    }
}
