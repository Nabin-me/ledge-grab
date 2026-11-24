//! Short clips: a screen region to MP4, and an MP4 to GIF.
use crate::capture::{Frame, Monitor, intersect, monitors};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HMONITOR};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, Result, w};

const FPS: u32 = 30;
/// A forgotten recording ends by itself.
const LONGEST: Duration = Duration::from_secs(600);
const GIF_FPS: f64 = 15.0;
const GIF_W: i32 = 960;

pub struct Clip {
    pub path: PathBuf,
    pub thumb: Frame,
    pub secs: i64,
}

/// A recording in progress.
pub struct Rec {
    stop: Arc<AtomicBool>,
    edges: Vec<isize>,
}

impl Rec {
    pub fn start(inst: HINSTANCE, r: RECT, path: PathBuf, done: impl FnOnce(Option<Clip>) + Send + 'static) -> Option<Rec> {
        let mons = monitors();
        let m = *mons.iter().filter(|m| intersect(&m.rect, &r).is_some()).max_by_key(|m| {
            let i = intersect(&m.rect, &r).unwrap();
            (i.right - i.left) as i64 * (i.bottom - i.top) as i64
        })?;
        let r = fit(r, m.rect)?;
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        std::thread::spawn(move || {
            let res = unsafe { record(m, r, &path, &s) };
            if let Err(e) = &res {
                crate::note(&format!("clip failed {e}"));
                let _ = std::fs::remove_file(&path);
            }
            done(res.ok().map(|(thumb, secs)| Clip { path, thumb, secs }));
        });
        Some(Rec { stop, edges: edges(inst, r, (m.dpi as f32 / 96.0 * 2.0).round() as i32) })
    }

    pub fn stop(&self) {
        self.stop.store(true, Relaxed);
    }
}

impl Drop for Rec {
    fn drop(&mut self) {
        self.stop();
        for h in &self.edges {
            unsafe {
                let _ = DestroyWindow(HWND(*h as *mut _));
            }
        }
    }
}

/// Inside the monitor, even sides, big enough for any encoder.
fn fit(r: RECT, m: RECT) -> Option<RECT> {
    let r = intersect(&r, &m)?;
    let side = |lo: i32, hi: i32, mlo: i32, mhi: i32| {
        let n = ((hi - lo) & !1).max(96).min((mhi - mlo) & !1);
        let lo = lo.min(mhi - n).max(mlo);
        (lo, lo + n)
    };
    let (left, right) = side(r.left, r.right, m.left, m.right);
    let (top, bottom) = side(r.top, r.bottom, m.top, m.bottom);
    Some(RECT { left, top, right, bottom })
}

/// Four red lines just outside the region.
fn edges(inst: HINSTANCE, r: RECT, t: i32) -> Vec<isize> {
    unsafe {
        let class = w!("LedgeGrabEdge");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(crate::plain),
            hInstance: inst,
            lpszClassName: class,
            hbrBackground: CreateSolidBrush(COLORREF(0x005e50f4)),
            ..Default::default()
        });
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        [
            (r.left - t, r.top - t, w + t * 2, t),
            (r.left - t, r.bottom, w + t * 2, t),
            (r.left - t, r.top, t, h),
            (r.right, r.top, t, h),
        ]
        .into_iter()
        .filter_map(|(x, y, w, h)| {
            let h = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
                class,
                w!("Ledge Grab edge"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(inst),
                None,
            )
            .ok()?;
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            Some(h.0 as isize)
        })
        .collect()
    }
}

fn pair(a: u32, b: u32) -> u64 {
    (a as u64) << 32 | b as u64
}

unsafe fn video_type(sub: windows::core::GUID, w: u32, h: u32) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &sub)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pair(w, h))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pair(FPS, 1))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1))?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        Ok(t)
    }
}

struct Sink {
    w: IMFSinkWriter,
    stream: u32,
}

unsafe fn sink(path: &Path, w: u32, h: u32, hardware: bool) -> Result<Sink> {
    unsafe {
        let mut a = None;
        MFCreateAttributes(&mut a, 2)?;
        let a: IMFAttributes = a.unwrap();
        a.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hardware as u32)?;
        a.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        let sw = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &a)?;
        let out = video_type(MFVideoFormat_H264, w, h)?;
        let rate = (w as u64 * h as u64 * FPS as u64 / 12).clamp(2_000_000, 28_000_000) as u32;
        out.SetUINT32(&MF_MT_AVG_BITRATE, rate)?;
        out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
        let stream = sw.AddStream(&out)?;
        let input = video_type(MFVideoFormat_NV12, w, h)?;
        input.SetUINT32(&MF_MT_DEFAULT_STRIDE, w)?;
        sw.SetInputMediaType(stream, &input, None)?;
        sw.BeginWriting()?;
        Ok(Sink { w: sw, stream })
    }
}

/// BT.709, video range.
fn nv12(px: &[u8], pitch: usize, w: usize, h: usize, out: &mut [u8]) {
    let (luma, chroma) = out.split_at_mut(w * h);
    for y in 0..h {
        let src = &px[y * pitch..y * pitch + w * 4];
        for (d, p) in luma[y * w..(y + 1) * w].iter_mut().zip(src.chunks_exact(4)) {
            let (b, g, r) = (p[0] as i32, p[1] as i32, p[2] as i32);
            *d = (16 + ((47 * r + 157 * g + 16 * b + 128) >> 8)) as u8;
        }
    }
    for y in 0..h / 2 {
        let a = &px[y * 2 * pitch..y * 2 * pitch + w * 4];
        let c = &px[(y * 2 + 1) * pitch..(y * 2 + 1) * pitch + w * 4];
        for ((d, p), q) in chroma[y * w..(y + 1) * w].chunks_exact_mut(2).zip(a.chunks_exact(8)).zip(c.chunks_exact(8)) {
            let s = |k: usize| p[k] as i32 + p[k + 4] as i32 + q[k] as i32 + q[k + 4] as i32;
            let (b, g, r) = (s(0), s(1), s(2));
            d[0] = (128 + ((-26 * r - 86 * g + 112 * b + 512) >> 10)).clamp(0, 255) as u8;
            d[1] = (128 + ((112 * r - 102 * g - 10 * b + 512) >> 10)).clamp(0, 255) as u8;
        }
    }
}

unsafe fn record(m: Monitor, r: RECT, path: &Path, stop: &AtomicBool) -> Result<(Frame, i64)> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let mut dev = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            None,
        )?;
        let dev: ID3D11Device = dev.unwrap();
        let ctx = dev.GetImmediateContext()?;
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = interop.CreateForMonitor(HMONITOR(m.handle as *mut _))?;
        let rt: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dev.cast::<IDXGIDevice>()?)?.cast()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &rt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            item.Size()?,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(true);
        let _ = session.SetIsBorderRequired(false);
        let (w, h) = ((r.right - r.left) as usize, (r.bottom - r.top) as usize);
        let mut staging = None;
        dev.CreateTexture2D(
            &D3D11_TEXTURE2D_DESC {
                Width: w as u32,
                Height: h as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            },
            None,
            Some(&mut staging),
        )?;
        let staging = staging.unwrap();
        let (x, y) = ((r.left - m.rect.left) as u32, (r.top - m.rect.top) as u32);
        let area = D3D11_BOX { left: x, top: y, front: 0, right: x + w as u32, bottom: y + h as u32, back: 1 };
        // Let the picker leave the screen first.
        std::thread::sleep(Duration::from_millis(120));
        session.StartCapture()?;

        let mut yuv = vec![0u8; w * h * 3 / 2];
        let mut first: Option<Frame> = None;
        let pull = |yuv: &mut [u8], first: &mut Option<Frame>| -> Result<bool> {
            let mut newest = None;
            while let Ok(f) = pool.TryGetNextFrame() {
                if let Some(old) = newest.replace(f) {
                    let _ = old.Close();
                }
            }
            let Some(f) = newest else { return Ok(false) };
            let tex: ID3D11Texture2D = f.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
            ctx.CopySubresourceRegion(&staging, 0, 0, 0, 0, &tex, 0, Some(&area));
            let _ = f.Close();
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
            let pitch = map.RowPitch as usize;
            let px = std::slice::from_raw_parts(map.pData as *const u8, pitch * (h - 1) + w * 4);
            nv12(px, pitch, w, h, yuv);
            if first.is_none() {
                let mut f = Frame::new(w as i32, h as i32);
                for (y, row) in f.px.chunks_exact_mut(w * 4).enumerate() {
                    row.copy_from_slice(&px[y * pitch..y * pitch + w * 4]);
                }
                f.px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
                *first = Some(f);
            }
            ctx.Unmap(&staging, 0);
            Ok(true)
        };
        let wait = Instant::now();
        while !pull(&mut yuv, &mut first)? {
            if wait.elapsed() > Duration::from_secs(2) {
                return Err(E_FAIL.into());
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        // Only the thumbnail is kept.
        let thumb = first.as_ref().map(|f| f.thumb(crate::store::THUMB.0, crate::store::THUMB.1));
        if let Some(f) = &mut first {
            f.px = Vec::new();
        }
        // Encoders on the graphics card refuse very small pictures.
        let hardware = w >= 256 && h >= 256;
        let sink = match sink(path, w as u32, h as u32, hardware) {
            Err(_) if hardware => {
                let _ = std::fs::remove_file(path);
                sink(path, w as u32, h as u32, false)?
            }
            s => s?,
        };
        let begin = Instant::now();
        let tick = 10_000_000i64 / FPS as i64;
        let mut n = 0i64;
        while !stop.load(Relaxed) && begin.elapsed() < LONGEST || n == 0 {
            pull(&mut yuv, &mut first)?;
            // Steady frames: late ones repeat so time stays true.
            let due = (begin.elapsed().as_secs_f64() * FPS as f64) as i64 + 1;
            while n < due {
                let buf = MFCreateMemoryBuffer(yuv.len() as u32)?;
                let mut p = std::ptr::null_mut();
                buf.Lock(&mut p, None, None)?;
                std::ptr::copy_nonoverlapping(yuv.as_ptr(), p, yuv.len());
                buf.Unlock()?;
                buf.SetCurrentLength(yuv.len() as u32)?;
                let s = MFCreateSample()?;
                s.AddBuffer(&buf)?;
                s.SetSampleTime(n * tick)?;
                s.SetSampleDuration(tick)?;
                sink.w.WriteSample(sink.stream, &s)?;
                n += 1;
            }
            let next = begin + Duration::from_secs_f64(n as f64 / FPS as f64);
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        let _ = session.Close();
        let _ = pool.Close();
        sink.w.Finalize()?;
        let secs = (n as f64 / FPS as f64).round().max(1.0) as i64;
        crate::note(&format!("clip {w}x{h} {n} frames {:.2}s", begin.elapsed().as_secs_f64()));
        Ok((thumb.unwrap(), secs))
    }
}

// ── GIF ────────────────────────────────────────────────────────────────────

/// Walk a video's frames: (seconds, picture). Returns its length.
unsafe fn frames(path: &Path, mut each: impl FnMut(f64, Frame) -> Result<()>) -> Result<f64> {
    unsafe {
        const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let mut a = None;
        MFCreateAttributes(&mut a, 1)?;
        let a: IMFAttributes = a.unwrap();
        a.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let rd = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &a)?;
        let size = rd.GetNativeMediaType(VIDEO, 0)?.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((size >> 32) as i32, size as u32 as i32);
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        rd.SetCurrentMediaType(VIDEO, None, &t)?;
        let cur = rd.GetCurrentMediaType(VIDEO)?;
        let full = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (fw, fh) = ((full >> 32) as i32, full as u32 as i32);
        let flipped = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).is_ok_and(|s| (s as i32) < 0);
        let mut end = 0.0;
        loop {
            let (mut flags, mut ts, mut s) = (0u32, 0i64, None);
            rd.ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut s))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(end);
            }
            let Some(s) = s else { continue };
            let at = ts as f64 / 1e7;
            end = at + s.GetSampleDuration().unwrap_or(0) as f64 / 1e7;
            let buf = s.ConvertToContiguousBuffer()?;
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            buf.Lock(&mut p, None, Some(&mut len))?;
            let px = std::slice::from_raw_parts(p, len as usize);
            let mut f = Frame::new(w.min(fw), h.min(fh));
            // Decoders pad rows as they please, so count what came.
            let pitch = len as usize / fh as usize;
            let n = (f.w * 4) as usize;
            for y in 0..f.h as usize {
                let sy = if flipped { fh as usize - 1 - y } else { y };
                if let Some(row) = px.get(sy * pitch..sy * pitch + n) {
                    f.px[y * n..(y + 1) * n].copy_from_slice(row);
                }
            }
            buf.Unlock()?;
            each(at, f)?;
        }
    }
}

/// Area-averaged resize.
fn shrink(f: &Frame, w: i32) -> Frame {
    if f.w <= w {
        return f.clone();
    }
    let h = (f.h as i64 * w as i64 / f.w as i64).max(1) as i32;
    let mut o = Frame::new(w, h);
    let k = f.w as f32 / w as f32;
    for y in 0..h {
        let (a, b) = ((y as f32 * k) as i32, (((y + 1) as f32 * k).ceil() as i32).min(f.h));
        for x in 0..w {
            let (l, r) = ((x as f32 * k) as i32, (((x + 1) as f32 * k).ceil() as i32).min(f.w));
            let (mut c, mut n) = ([0u32; 3], 0);
            for yy in a..b {
                let s = (yy * f.w * 4) as usize;
                for p in f.px[s + l as usize * 4..s + r as usize * 4].chunks_exact(4) {
                    c[0] += p[0] as u32;
                    c[1] += p[1] as u32;
                    c[2] += p[2] as u32;
                    n += 1;
                }
            }
            let i = ((y * w + x) * 4) as usize;
            for j in 0..3 {
                o.px[i + j] = (c[j] / n) as u8;
            }
            o.px[i + 3] = 255;
        }
    }
    o
}

/// The changed part of `f` as RGBA, unchanged pixels clear. Updates `shown`.
fn patch(shown: &mut Frame, f: &Frame) -> Option<(RECT, Vec<u8>)> {
    // Video compression shimmers; only real changes count.
    let diff = |a: &[u8], b: &[u8]| (0..3).any(|k| a[k].abs_diff(b[k]) > 7);
    let mut r = RECT { left: f.w, top: f.h, right: 0, bottom: 0 };
    for (n, (a, b)) in shown.px.chunks_exact(4).zip(f.px.chunks_exact(4)).enumerate() {
        if diff(a, b) {
            let (x, y) = (n as i32 % f.w, n as i32 / f.w);
            r = RECT { left: r.left.min(x), top: r.top.min(y), right: r.right.max(x + 1), bottom: r.bottom.max(y + 1) };
        }
    }
    if r.right <= r.left {
        return None;
    }
    let mut out = Vec::with_capacity(((r.right - r.left) * (r.bottom - r.top) * 4) as usize);
    for y in r.top..r.bottom {
        for x in r.left..r.right {
            let i = ((y * f.w + x) * 4) as usize;
            if diff(&shown.px[i..], &f.px[i..]) {
                out.extend_from_slice(&[f.px[i + 2], f.px[i + 1], f.px[i], 255]);
                shown.px[i..i + 3].copy_from_slice(&f.px[i..i + 3]);
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Some((r, out))
}

type Gif = gif::Encoder<std::io::BufWriter<std::fs::File>>;

/// Blocking; call from a worker thread.
pub fn gif(src: &Path, dst: &Path) -> Option<()> {
    let t = Instant::now();
    let tmp = dst.with_extension("part");
    let ok = write_gif(src, &tmp).is_some();
    crate::note(&format!("gif {:.1}s {ok}", t.elapsed().as_secs_f64()));
    if ok && std::fs::rename(&tmp, dst).is_ok() {
        return Some(());
    }
    let _ = std::fs::remove_file(&tmp);
    None
}

fn write_gif(src: &Path, dst: &Path) -> Option<()> {
    let mut out = Some(std::io::BufWriter::new(std::fs::File::create(dst).ok()?));
    let mut enc: Option<Gif> = None;
    let mut shown: Option<Frame> = None;
    // One frame behind: its length is known when the next one changes.
    let mut held: Option<gif::Frame<'static>> = None;
    let (mut last, mut spent) = (-1.0f64, 0i64);
    let mut flush = |enc: &mut Option<Gif>, held: Option<gif::Frame<'static>>, until: f64| {
        let Some(mut f) = held else { return true };
        let end = (until * 100.0).round() as i64;
        f.delay = (end - spent).clamp(2, 65_000) as u16;
        spent += f.delay as i64;
        enc.as_mut().is_some_and(|e| e.write_frame(&f).is_ok())
    };
    let fail = || windows::core::Error::from(E_FAIL);
    let end = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).ok()?;
        frames(src, |at, f| {
            if at - last < 1.0 / GIF_FPS - 0.004 {
                return Ok(());
            }
            last = at;
            let f = shrink(&f, GIF_W);
            let (r, mut px) = match &mut shown {
                None => {
                    let px = f.px.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
                    let file = out.take().ok_or_else(fail)?;
                    let mut e = gif::Encoder::new(file, f.w as u16, f.h as u16, &[]).map_err(|_| fail())?;
                    e.set_repeat(gif::Repeat::Infinite).map_err(|_| fail())?;
                    enc = Some(e);
                    let r = RECT { left: 0, top: 0, right: f.w, bottom: f.h };
                    shown = Some(f);
                    (r, px)
                }
                Some(s) => match patch(s, &f) {
                    Some(p) => p,
                    None => return Ok(()),
                },
            };
            if !flush(&mut enc, held.take(), at) {
                return Err(fail());
            }
            let mut g = gif::Frame::from_rgba_speed((r.right - r.left) as u16, (r.bottom - r.top) as u16, &mut px, 10);
            (g.left, g.top, g.dispose) = (r.left as u16, r.top as u16, gif::DisposalMethod::Keep);
            held = Some(g);
            Ok(())
        })
        .ok()?
    };
    (flush(&mut enc, held.take(), end) && enc.is_some()).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_survive() {
        for (b, g, r) in [(255, 255, 255), (0, 0, 0), (40, 80, 200), (200, 120, 30)] {
            let px: Vec<u8> = [b, g, r, 255].repeat(4);
            let mut o = [0u8; 6];
            nv12(&px, 8, 2, 2, &mut o);
            let (y, u, v) = ((o[0] as f32 - 16.0) * 1.164, o[4] as f32 - 128.0, o[5] as f32 - 128.0);
            let back = [y + 2.112 * u, y - 0.213 * u - 0.533 * v, y + 1.793 * v];
            for (a, e) in back.iter().zip([b, g, r]) {
                assert!((a - e as f32).abs() < 6.0, "{back:?} vs {:?}", (b, g, r));
            }
        }
    }

    #[test]
    fn fits_regions() {
        let m = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
        let f = fit(RECT { left: -31, top: 5, right: 300, bottom: 38 }, m).unwrap();
        assert_eq!(((f.right - f.left) % 2, (f.bottom - f.top) % 2), (0, 0));
        assert!(f.right - f.left >= 96 && f.left >= -1920 && f.right <= 0);
        assert!(intersect(&f, &m) == Some(f));
    }

    #[test]
    fn patches_only_changes() {
        let mut a = Frame::new(10, 10);
        let mut b = a.clone();
        assert!(patch(&mut a, &b).is_none());
        b.px[(5 * 10 + 3) * 4] = 200;
        let (r, px) = patch(&mut a, &b).unwrap();
        assert_eq!((r, px), (RECT { left: 3, top: 5, right: 4, bottom: 6 }, vec![0, 0, 200, 255]));
        assert!(patch(&mut a, &b).is_none());
    }
}
