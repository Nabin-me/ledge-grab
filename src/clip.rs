//! Clipboard, recycle bin and drag-out.
use crate::capture::Frame;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER, LCS_GM_IMAGES};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY};
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

pub enum Taken {
    Text(String),
    Image(Frame),
}

unsafe fn bytes(format: u32) -> Option<Vec<u8>> {
    unsafe {
        IsClipboardFormatAvailable(format).ok()?;
        let h = HGLOBAL(GetClipboardData(format).ok()?.0);
        let n = GlobalSize(h);
        // A 100 MP picture is past anything worth keeping.
        if n == 0 || n > 400_000_000 {
            return None;
        }
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, n).to_vec();
        let _ = GlobalUnlock(h);
        Some(v)
    }
}

/// What another app just copied, unless it asked not to be remembered.
pub fn read(hwnd: HWND) -> Option<Taken> {
    unsafe {
        let named = |n| RegisterClipboardFormatW(n);
        let has = |f| IsClipboardFormatAvailable(f).is_ok();
        if has(named(w!("ExcludeClipboardContentFromMonitorProcessing"))) || has(named(w!("Clipboard Viewer Ignore"))) {
            return None;
        }
        if !open(hwnd) {
            return None;
        }
        let out = (|| {
            let keep = named(w!("CanIncludeInClipboardHistory"));
            if bytes(keep).is_some_and(|b| b.iter().take(4).all(|b| *b == 0)) {
                return None;
            }
            if let Some(b) = bytes(CF_UNICODETEXT.0 as u32) {
                let t: Vec<u16> =
                    b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
                let t = String::from_utf16_lossy(&t);
                return (!t.trim().is_empty() && t.len() <= crate::store::TEXT_MAX).then_some(Taken::Text(t));
            }
            if has(CF_HDROP.0 as u32) {
                return None;
            }
            let png = bytes(png_format()).and_then(|b| crate::store::decode_png(&b));
            png.or_else(|| dib(&bytes(CF_DIB.0 as u32)?)).filter(|f| f.w > 3 && f.h > 3).map(Taken::Image)
        })();
        let _ = CloseClipboard();
        out
    }
}

/// An uncompressed 24 or 32 bit bitmap.
fn dib(b: &[u8]) -> Option<Frame> {
    let u = |o: usize| b.get(o..o + 4).map(|v| u32::from_le_bytes(v.try_into().unwrap()));
    let (head, w, h) = (u(0)? as usize, u(4)? as i32, u(8)? as i32);
    let bits = b.get(14..16).map(|v| u16::from_le_bytes([v[0], v[1]]))? as usize;
    let (how, colors) = (u(16)?, u(32)? as usize);
    let masks = if head == 40 && how == BI_BITFIELDS.0 { 12 } else { 0 };
    if !matches!(bits, 24 | 32) || (how != 0 && how != BI_BITFIELDS.0) || w < 1 || h == 0 || w > 32768 || h.unsigned_abs() > 32768 {
        return None;
    }
    let pitch = (w as usize * bits).div_ceil(32) * 4;
    let px = b.get(head + masks + colors * 4..)?;
    let n = h.unsigned_abs() as usize;
    if px.len() < pitch * n {
        return None;
    }
    let mut f = Frame::new(w, n as i32);
    for (y, row) in f.px.chunks_exact_mut(w as usize * 4).enumerate() {
        let s = if h > 0 { n - 1 - y } else { y } * pitch;
        for (d, p) in row.chunks_exact_mut(4).zip(px[s..].chunks_exact(bits / 8)) {
            d.copy_from_slice(&[p[0], p[1], p[2], 255]);
        }
    }
    Some(f)
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

/// Copied text as a file, for dragging and opening.
pub fn text_file(id: i64, text: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("Signet");
    std::fs::create_dir_all(&dir).ok()?;
    let p = dir.join(format!("Text {id}.txt"));
    std::fs::write(&p, text).ok()?;
    Some(p)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bitmaps() {
        // 1x2, 24 bit, bottom-up: red above blue.
        let mut b = vec![0u8; 40];
        b[0] = 40;
        b[4] = 1;
        b[8] = 2;
        b[12] = 1;
        b[14] = 24;
        b.extend([255, 0, 0, 0, 0, 0, 255, 0]);
        let f = dib(&b).unwrap();
        assert_eq!((f.w, f.h), (1, 2));
        assert_eq!(f.px, [0, 0, 255, 255, 255, 0, 0, 255]);
        assert!(dib(&b[..44]).is_none());
    }
}
