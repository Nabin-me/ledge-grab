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

/// What a clipboard write actually did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Set {
    Ok,
    /// Couldn't open or empty the clipboard; whatever was there is still there.
    Busy,
    /// The clipboard was emptied, then the new data couldn't be set. It is now empty.
    Cleared,
}

impl Set {
    pub fn ok(self) -> bool {
        self == Set::Ok
    }
}

/// Keep the owner's record in step with what a write left on the clipboard, so a
/// delayed format can only ever be rendered from the image whose bitmap is there.
pub fn record<S>(r: Set, ids_now: &mut Vec<i64>, source_now: &mut Option<S>, ids: &[i64], source: Option<S>) {
    match r {
        Set::Ok => {
            *ids_now = ids.to_vec();
            *source_now = source;
        }
        // The previous image is still on the clipboard and still described.
        Set::Busy => {}
        Set::Cleared => {
            ids_now.clear();
            *source_now = None;
        }
    }
}

/// A packed top-down frame as a bottom-up 32-bit CF_DIBV5.
pub fn dibv5(f: &Frame) -> Vec<u8> {
    let head = size_of::<BITMAPV5HEADER>();
    let n = (f.w * 4) as usize;
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
    let mut out = vec![0u8; head + f.px.len()];
    unsafe { std::ptr::copy_nonoverlapping(&h as *const _ as *const u8, out.as_mut_ptr(), head) };
    for y in 0..f.h as usize {
        out[head + y * n..head + (y + 1) * n].copy_from_slice(f.row(f.h - 1 - y as i32));
    }
    out
}

unsafe fn put_bytes(format: u32, b: &[u8]) -> Result<()> {
    unsafe {
        let h = global(b.len(), |p| std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()))?;
        if let Err(e) = SetClipboardData(format, Some(h)) {
            let _ = GlobalFree(Some(HGLOBAL(h.0)));
            return Err(e);
        }
        Ok(())
    }
}

/// Offer a format to be rendered on request. Success returns NULL here, so the
/// last error is what tells success from failure.
unsafe fn promise(format: u32) -> bool {
    unsafe {
        SetLastError(WIN32_ERROR(0));
        let _ = SetClipboardData(format, None);
        GetLastError() == WIN32_ERROR(0)
    }
}

/// Mark a queued image as private to the paste, so clipboard history and
/// monitors that honour these formats don't read it before the target app does.
unsafe fn keep_out_of_history() {
    unsafe {
        let zero = 0u32.to_le_bytes();
        for name in [w!("ExcludeClipboardContentFromMonitorProcessing"), w!("CanIncludeInClipboardHistory"), w!("CanUploadToCloudClipboard")] {
            let _ = put_bytes(RegisterClipboardFormatW(name), &zero);
        }
    }
}

/// Put one image on the clipboard. Normally the bitmap goes on now and PNG is
/// promised; `delayed` promises both, so the first read by any app reaches the
/// owner as WM_RENDERFORMAT. The PNG promise is made only once the bitmap is in
/// place, so the two formats can never describe different images.
pub fn set_image(hwnd: HWND, f: &Frame, delayed: bool) -> Set {
    if f.w < 1 || f.h < 1 || !open(hwnd) {
        return Set::Busy;
    }
    unsafe {
        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            return Set::Busy;
        }
        let bitmap = if delayed { promise(CF_DIBV5.0 as u32) } else { put_bytes(CF_DIBV5.0 as u32, &dibv5(f)).is_ok() };
        let r = if bitmap {
            let _ = promise(png_format());
            if delayed {
                keep_out_of_history();
            }
            Set::Ok
        } else {
            Set::Cleared
        };
        let _ = CloseClipboard();
        r
    }
}

/// Answer WM_RENDERFORMAT. The clipboard is already open then.
pub fn render(format: u32, bytes: &[u8]) {
    unsafe {
        let _ = put_bytes(format, bytes);
    }
}

/// Answer WM_RENDERALLFORMATS as the app goes away: fill every open promise.
pub fn render_all(hwnd: HWND, mut bytes: impl FnMut(u32) -> Option<Vec<u8>>) {
    unsafe {
        if open(hwnd) {
            if GetClipboardOwner().is_ok_and(|o| o == hwnd) {
                for format in [CF_DIBV5.0 as u32, png_format()] {
                    if IsClipboardFormatAvailable(format).is_ok() {
                        if let Some(b) = bytes(format) {
                            render(format, &b);
                        }
                    }
                }
            }
            let _ = CloseClipboard();
        }
    }
}

/// Empty the clipboard, so a paste can't repeat a stale image.
pub fn clear(hwnd: HWND) -> Set {
    if !open(hwnd) {
        return Set::Busy;
    }
    unsafe {
        let r = if EmptyClipboard().is_ok() { Set::Ok } else { Set::Busy };
        let _ = CloseClipboard();
        r
    }
}

pub fn set_files(hwnd: HWND, files: &[PathBuf]) -> Set {
    if files.is_empty() || !open(hwnd) {
        return Set::Busy;
    }
    let mut names: Vec<u16> = Vec::new();
    for f in files {
        use std::os::windows::ffi::OsStrExt;
        names.extend(f.as_os_str().encode_wide());
        names.push(0);
    }
    names.push(0);
    unsafe {
        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            return Set::Busy;
        }
        let head = size_of::<DROPFILES>();
        let mut b = vec![0u8; head + names.len() * 2];
        let d = DROPFILES { pFiles: head as u32, fWide: true.into(), ..Default::default() };
        std::ptr::copy_nonoverlapping(&d as *const _ as *const u8, b.as_mut_ptr(), head);
        std::ptr::copy_nonoverlapping(names.as_ptr() as *const u8, b.as_mut_ptr().add(head), names.len() * 2);
        let r = if put_bytes(CF_HDROP.0 as u32, &b).is_ok() { Set::Ok } else { Set::Cleared };
        let _ = CloseClipboard();
        r
    }
}

pub fn set_text(hwnd: HWND, s: &str) -> Set {
    if !open(hwnd) {
        return Set::Busy;
    }
    let t: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    unsafe {
        if EmptyClipboard().is_err() {
            let _ = CloseClipboard();
            return Set::Busy;
        }
        let r = if put_bytes(CF_UNICODETEXT.0 as u32, &t).is_ok() { Set::Ok } else { Set::Cleared };
        let _ = CloseClipboard();
        r
    }
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

/// An image another app just copied, unless it asked not to be remembered.
pub fn read_image(hwnd: HWND) -> Option<Frame> {
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
            if has(CF_UNICODETEXT.0 as u32) || has(CF_HDROP.0 as u32) {
                return None;
            }
            let png = bytes(png_format()).and_then(|b| crate::store::decode_png(&b));
            png.or_else(|| dib(&bytes(CF_DIB.0 as u32)?)).filter(|f| f.w > 3 && f.h > 3)
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

    #[test]
    fn owner_record_follows_what_the_write_left() {
        let (mut ids, mut src) = (Vec::new(), None);
        record(Set::Ok, &mut ids, &mut src, &[1], Some("A"));
        assert_eq!((ids.as_slice(), src), (&[1][..], Some("A")));
        // B failed before touching the clipboard: A is still there and still described.
        record(Set::Busy, &mut ids, &mut src, &[2], Some("B"));
        assert_eq!((ids.as_slice(), src), (&[1][..], Some("A")));
        // C emptied the clipboard and failed: nothing may be rendered any more.
        record(Set::Cleared, &mut ids, &mut src, &[3], Some("C"));
        assert_eq!((ids.as_slice(), src), (&[][..], None));
    }

    #[test]
    fn dibv5_round_trips() {
        let mut f = Frame::new(3, 2);
        for (n, p) in f.px.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[n as u8, 10 + n as u8, 20 + n as u8, 255]);
        }
        assert_eq!(dib(&dibv5(&f)).unwrap().px, f.px);
    }

    /// Live clipboard: run alone with `cargo test -- --ignored --test-threads=1`
    /// on a Windows desktop session.
    mod live {
        use super::super::*;
        use std::cell::RefCell;
        use windows::Win32::UI::WindowsAndMessaging::*;

        thread_local! {
            static SOURCE: RefCell<Option<Frame>> = const { RefCell::new(None) };
            static READS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        }

        extern "system" fn owner(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            if msg == WM_RENDERFORMAT {
                READS.with(|r| r.borrow_mut().push(w.0 as u32));
                SOURCE.with(|s| {
                    if let Some(f) = &*s.borrow() {
                        let b = if w.0 as u32 == png_format() { crate::store::encode_png(f) } else { dibv5(f) };
                        render(w.0 as u32, &b);
                    }
                });
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(h, msg, w, l) }
        }

        fn window() -> HWND {
            unsafe {
                let class = w!("LedgeGrabClipTest");
                RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(owner), lpszClassName: class, ..Default::default() });
                CreateWindowExW(Default::default(), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None)
                    .unwrap()
            }
        }

        fn solid(bgr: [u8; 3]) -> Frame {
            let mut f = Frame::new(8, 8);
            for p in f.px.chunks_exact_mut(4) {
                p.copy_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
            }
            f
        }

        /// What a pasting app gets from each image format.
        fn read_back(h: HWND) -> (Option<[u8; 3]>, Option<[u8; 3]>) {
            assert!(open(h));
            let px = |f: Frame| [f.px[0], f.px[1], f.px[2]];
            let out = unsafe {
                (bytes(CF_DIBV5.0 as u32).and_then(|b| dib(&b)).map(px), bytes(png_format()).and_then(|b| crate::store::decode_png(&b)).map(px))
            };
            unsafe { let _ = CloseClipboard(); }
            out
        }

        const RED: [u8; 3] = [0, 0, 255];
        const GREEN: [u8; 3] = [0, 255, 0];
        const BLUE: [u8; 3] = [255, 0, 0];

        #[test]
        #[ignore = "uses the real clipboard"]
        fn a_failed_copy_never_mixes_two_images() {
            let h = window();
            let (mut ids, mut src): (Vec<i64>, Option<Frame>) = (Vec::new(), None);
            let sync = |src: &Option<Frame>| SOURCE.with(|s| *s.borrow_mut() = src.clone());

            let r = set_image(h, &solid(RED), false);
            assert_eq!(r, Set::Ok);
            record(r, &mut ids, &mut src, &[1], Some(solid(RED)));
            sync(&src);

            // Another app holds the clipboard open while image B is copied.
            let (tx, rx) = std::sync::mpsc::channel();
            let holder = std::thread::spawn(move || unsafe {
                OpenClipboard(None).unwrap();
                tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(400));
                let _ = CloseClipboard();
            });
            rx.recv().unwrap();
            let r = set_image(h, &solid(GREEN), false);
            assert_eq!(r, Set::Busy, "a held clipboard must be reported, not treated as copied");
            record(r, &mut ids, &mut src, &[2], Some(solid(GREEN)));
            sync(&src);
            holder.join().unwrap();

            assert_eq!(ids, [1]);
            assert_eq!(read_back(h), (Some(RED), Some(RED)), "bitmap and delayed PNG must be the same image");

            // And a successful replacement moves both formats together.
            let r = set_image(h, &solid(BLUE), false);
            record(r, &mut ids, &mut src, &[3], Some(solid(BLUE)));
            sync(&src);
            assert_eq!(read_back(h), (Some(BLUE), Some(BLUE)));
            unsafe { let _ = DestroyWindow(h); }
        }

        #[test]
        #[ignore = "uses the real clipboard"]
        fn a_queued_image_reports_its_first_read() {
            let h = window();
            SOURCE.with(|s| *s.borrow_mut() = Some(solid(GREEN)));
            READS.with(|r| r.borrow_mut().clear());
            assert_eq!(set_image(h, &solid(GREEN), true), Set::Ok);
            // Nothing is rendered until an app asks.
            assert!(READS.with(|r| r.borrow().is_empty()));
            assert_eq!(read_back(h), (Some(GREEN), Some(GREEN)));
            let reads = READS.with(|r| r.borrow().clone());
            assert_eq!(reads, [CF_DIBV5.0 as u32, png_format()]);
            // Clipboard history is asked to leave it alone.
            let keep = unsafe { RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")) };
            assert!(unsafe { IsClipboardFormatAvailable(keep) }.is_ok());
            assert_eq!(clear(h), Set::Ok);
            assert_eq!(read_back(h), (None, None));
            unsafe { let _ = DestroyWindow(h); }
        }
    }
}
