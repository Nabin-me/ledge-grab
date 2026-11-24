#![windows_subsystem = "windows"]
//! Signet: screenshots that live on the notch.
mod anim;
mod capture;
mod clip;
mod gfx;
mod notch;
mod ocr;
mod overlay;
mod pdf;
mod redact;
mod scroll;
mod store;

use capture::{Frame, cursor, grab, monitors};
use gfx::Gfx;
use notch::{Notch, View};
use overlay::{Mode, Outcome, Overlay};
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, Ordering::Relaxed};
use std::time::{Duration, Instant};
use store::{Kind, Store};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PWSTR, w};

pub const WM_DRAG_OUT: u32 = WM_APP + 1;
pub const WM_RUN: u32 = WM_APP + 2;
/// Boxed Work in lparam.
const WM_WORK: u32 = WM_APP + 3;
const WM_KEY: u32 = WM_APP + 4;
pub const WM_PICK_FOLDER: u32 = WM_APP + 5;

pub const T_DWELL: usize = 1;
pub const T_LEAVE: usize = 2;
pub const T_FLASH: usize = 3;
const T_TICK: usize = 4;
const T_COUNT: usize = 5;
const T_INTERVAL: usize = 6;
const T_HOURLY: usize = 7;
const T_TRIM: usize = 8;

enum Work {
    Shot(Frame),
    Text(String),
    Failed,
}

pub struct App {
    pub g: Gfx,
    pub store: Store,
    inst: HINSTANCE,
    main: HWND,
    /// Items currently on the clipboard.
    pub clip: Vec<i64>,
    png: Option<PngSource>,
    /// Which hotkeys Windows gave us.
    pub hot: [bool; 3],
    pub interval: Option<RECT>,
    pub login: bool,
    pub outgoing: Vec<i64>,
    count: i64,
    overlay: Option<Overlay>,
    last_foreground: HWND,
}

enum PngSource {
    Frame(Frame),
    File(PathBuf),
}

impl PngSource {
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            PngSource::Frame(f) => Some(store::encode_png(f)),
            PngSource::File(p) => std::fs::read(p).ok(),
        }
    }
}

static APP: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());
static NOTCH: AtomicPtr<Notch> = AtomicPtr::new(std::ptr::null_mut());

// The whole app runs on one thread. Handlers that can pump messages (drag, folder
// picker) are posted, so they never run inside another handler's borrow.
fn app() -> Option<&'static mut App> {
    unsafe { APP.load(Relaxed).as_mut() }
}
fn notch() -> Option<&'static mut Notch> {
    unsafe { NOTCH.load(Relaxed).as_mut() }
}

pub fn testing() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("SIGNET_TEST").is_some())
}

/// Timings, only when a test asks for them.
pub fn note(s: &str) {
    if let Some(p) = std::env::var_os("SIGNET_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{s}");
        }
    }
}

impl App {
    /// Put items on the clipboard: a picture for one shot, files otherwise.
    pub fn copy(&mut self, ids: &[i64]) -> bool {
        let items: Vec<_> = ids.iter().filter_map(|i| self.store.get(*i)).collect();
        let files: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        let ok = match items.as_slice() {
            [one] if one.kind == Kind::Shot => {
                let path = one.path.clone();
                match std::fs::read(&path).ok().and_then(|b| store::decode_png(&b)) {
                    Some(f) => {
                        let ok = clip::set_image(self.main, &f);
                        self.png = Some(PngSource::File(path));
                        ok
                    }
                    None => false,
                }
            }
            [] => false,
            _ => clip::set_files(self.main, &files),
        };
        if ok {
            self.clip = ids.to_vec();
        }
        ok
    }

    pub fn merge(&mut self, ids: &[i64]) -> Option<i64> {
        let files: Vec<PathBuf> = ids.iter().filter_map(|i| self.store.get(*i)).map(|i| i.path.clone()).collect();
        let bytes = pdf::merge(&files)?;
        let thumb = self.store.thumb(*ids.first()?)?;
        let path = self.store.new_path("pdf");
        store::write_file(&path, &bytes).ok()?;
        let id = self.store.add(&path, Kind::Pdf, &thumb).ok()?;
        self.copy(&[id]);
        Some(id)
    }

    pub fn run(&self, m: Mode) {
        unsafe {
            let _ = PostMessageW(Some(self.main), WM_RUN, WPARAM(m.index()), LPARAM(0));
        }
    }

    pub fn stop_interval(&mut self) {
        self.interval = None;
        unsafe {
            let _ = KillTimer(Some(self.main), T_INTERVAL);
        }
    }

    pub fn record_key(&mut self, n: &mut Notch, row: usize) {
        n.recording = Some(row);
        self.unregister();
        start_key_hook();
    }

    fn unregister(&mut self) {
        for i in 0..3 {
            if std::mem::take(&mut self.hot[i]) {
                unsafe {
                    let _ = UnregisterHotKey(Some(self.main), i as i32 + 1);
                }
            }
        }
    }

    /// Claim hotkeys we don't hold yet. Returns true if any changed.
    fn register(&mut self) -> bool {
        if testing() || !HOOK.load(Relaxed).is_null() {
            return false;
        }
        let s = &self.store.settings;
        let mut changed = false;
        for (i, k) in [s.region, s.screen, s.window].into_iter().enumerate() {
            if !self.hot[i] && k.1 != 0 {
                let m = HOT_KEY_MODIFIERS(k.0) | MOD_NOREPEAT;
                self.hot[i] = unsafe { RegisterHotKey(Some(self.main), i as i32 + 1, m, k.1) }.is_ok();
                changed |= self.hot[i];
            }
        }
        changed
    }

    /// A finished capture: clipboard first, then notch, then disk.
    fn deliver(&mut self, f: Frame, quiet: bool) -> Option<i64> {
        if f.w < 1 || f.h < 1 {
            return None;
        }
        let t = Instant::now();
        let on_clip = !quiet && clip::set_image(self.main, &f);
        note(&format!("clipboard_ms {:.1}", t.elapsed().as_secs_f64() * 1e3));
        let thumb = f.thumb(store::THUMB.0, store::THUMB.1);
        let path = self.store.new_path("png");
        let id = self.store.add(&path, Kind::Shot, &thumb).ok()?;
        if on_clip {
            self.clip = vec![id];
            self.png = Some(PngSource::Frame(f.clone()));
        }
        note(&format!("indexed_ms {:.1}", t.elapsed().as_secs_f64() * 1e3));
        let main = self.main.0 as isize;
        std::thread::spawn(move || {
            let ok = store::write_file(&path, &store::encode_png(&f)).is_ok();
            note(&format!("saved_ms {:.1} {ok}", t.elapsed().as_secs_f64() * 1e3));
            if !ok {
                post(main, Work::Failed);
            }
        });
        unsafe {
            SetTimer(Some(self.main), T_TRIM, 4000, None);
        }
        Some(id)
    }

    fn grab_now(&mut self, n: &mut Notch, r: RECT) {
        if let Ok(f) = grab(r) {
            self.remember(r);
            if let Some(id) = self.deliver(f, false) {
                n.flash(&self.g, &self.store, id);
            }
        } else {
            n.fail();
        }
    }

    fn remember(&mut self, r: RECT) {
        self.store.settings.last = Some([r.left, r.top, r.right, r.bottom]);
        self.store.save_settings();
    }

    fn start(&mut self, n: &mut Notch, m: Mode) {
        if self.overlay.as_ref().is_some_and(|o| !o.flying()) {
            return;
        }
        if let Some(mut o) = self.overlay.take() {
            o.close();
        }
        match m {
            Mode::Screen => {
                let p = cursor();
                let mons = monitors();
                let r = mons.iter().find(|m| m.has(p)).unwrap_or(&mons[0]).rect;
                self.grab_now(n, r);
            }
            Mode::All => self.grab_now(n, capture::desktop(&monitors())),
            Mode::Last if self.store.settings.last.is_some() => {
                let [left, top, right, bottom] = self.store.settings.last.unwrap();
                self.grab_now(n, RECT { left, top, right, bottom });
            }
            Mode::Delay => {
                self.count = self.store.settings.delay;
                n.show(&self.g, &self.store, View::Count(self.count));
                unsafe {
                    SetTimer(Some(self.main), T_COUNT, 1000, None);
                }
            }
            m => {
                let m = if m == Mode::Last { Mode::Region } else { m };
                match Overlay::open(&self.g, self.inst, m) {
                    Ok(o) => {
                        note(&format!("overlay_ms {:.1}", o.opened_ms));
                        self.overlay = Some(o);
                    }
                    Err(e) => {
                        note(&format!("overlay failed {e}"));
                        n.fail();
                    }
                }
            }
        }
    }

    /// The window hotkey: whatever is in front.
    fn front_window(&mut self, n: &mut Notch) {
        let h = unsafe { GetForegroundWindow() };
        if let Some(r) = capture::window_rect(h).and_then(|r| capture::intersect(&r, &capture::desktop(&monitors()))) {
            self.grab_now(n, r);
        }
    }

    fn finish(&mut self, n: &mut Notch, out: Outcome) {
        if matches!(out, Outcome::None) {
            return;
        }
        let Some(mut o) = self.overlay.take() else { return };
        match out {
            Outcome::None => {}
            Outcome::Cancel => {
                o.refocus();
                o.close();
            }
            Outcome::Color(c) => {
                o.refocus();
                o.close();
                clip::set_text(self.main, &format!("#{c:06X}"));
                n.done(&self.g, &self.store);
            }
            Outcome::Shot(r) => match o.mode {
                Mode::Scroll => {
                    o.refocus();
                    o.close();
                    let main = self.main.0 as isize;
                    std::thread::spawn(move || {
                        post(main, scroll::run(r).map(Work::Shot).unwrap_or(Work::Failed));
                    });
                }
                Mode::Interval => {
                    o.refocus();
                    o.close();
                    self.interval = Some(r);
                    n.repaint();
                    unsafe {
                        SetTimer(Some(self.main), T_INTERVAL, self.store.settings.interval as u32 * 1000, None);
                    }
                    self.tick_interval(n);
                }
                Mode::Text => {
                    let f = o.take(&self.g, r);
                    o.refocus();
                    o.close();
                    let main = self.main.0 as isize;
                    std::thread::spawn(move || {
                        let t = ocr::words(&f).map(|w| ocr::text(&w)).unwrap_or_default();
                        post(main, if t.is_empty() { Work::Failed } else { Work::Text(t) });
                    });
                }
                _ => {
                    let t = Instant::now();
                    let f = o.take(&self.g, r);
                    self.remember(r);
                    let id = self.deliver(f.clone(), false);
                    note(&format!("release_ms {:.1}", t.elapsed().as_secs_f64() * 1e3));
                    if let Some(id) = id {
                        n.flash(&self.g, &self.store, id);
                        o.launch(f, r, n.landing());
                        self.overlay = Some(o);
                    } else {
                        o.close();
                        n.fail();
                    }
                }
            },
        }
        n.raise();
    }

    fn tick_interval(&mut self, n: &mut Notch) {
        if let Some(f) = self.interval.and_then(|r| grab(r).ok()) {
            if self.deliver(f, true).is_some() {
                n.refresh(&self.g, &self.store);
            }
        }
    }
}

fn post(main: isize, w: Work) {
    let p = Box::into_raw(Box::new(w));
    unsafe {
        if PostMessageW(Some(HWND(main as *mut _)), WM_WORK, WPARAM(0), LPARAM(p as isize)).is_err() {
            drop(Box::from_raw(p));
        }
    }
}

// ── Recording a hotkey ─────────────────────────────────────────────────────

static HOOK: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

fn start_key_hook() {
    unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if code >= 0 && (w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN) {
                let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
                let vk = VIRTUAL_KEY(k.vkCode as u16);
                let modifier = matches!(
                    vk,
                    VK_LSHIFT | VK_RSHIFT | VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN
                );
                if !modifier {
                    let down = |v: VIRTUAL_KEY| GetAsyncKeyState(v.0 as i32) < 0;
                    let mut m = 0;
                    for (v, bit) in
                        [(VK_CONTROL, MOD_CONTROL), (VK_SHIFT, MOD_SHIFT), (VK_MENU, MOD_ALT), (VK_LWIN, MOD_WIN)]
                    {
                        if down(v) {
                            m |= bit.0;
                        }
                    }
                    if let Some(a) = app() {
                        let _ = PostMessageW(Some(a.main), WM_KEY, WPARAM(m as usize), LPARAM(k.vkCode as isize));
                    }
                    return LRESULT(1);
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }
    unsafe {
        stop_key_hook();
        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) {
            HOOK.store(h.0, Relaxed);
        }
    }
}

fn stop_key_hook() {
    let h = HOOK.swap(std::ptr::null_mut(), Relaxed);
    if !h.is_null() {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(h));
        }
    }
}

// ── Launch at login ────────────────────────────────────────────────────────

const RUN: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

fn login_on() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, w!("Signet"), RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set_login(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let v: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                w!("Signet"),
                REG_SZ.0,
                Some(v.as_ptr() as *const _),
                v.len() as u32 * 2,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, w!("Signet"));
        }
    }
}

// ── What else is on screen ─────────────────────────────────────────────────

fn fullscreen(mon: &capture::Monitor) -> bool {
    unsafe {
        let h = GetForegroundWindow();
        if h.is_invalid() || IsZoomed(h).as_bool() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(h, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "XamlExplorerHostIslandWindow") {
            return false;
        }
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        let m = mon.rect;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// Is Personal Desk's own notch on screen?
fn desk_open() -> bool {
    unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
        unsafe {
            let mut class = [0u16; 16];
            let n = GetClassNameW(h, &mut class) as usize;
            if String::from_utf16_lossy(&class[..n]) != "TkTopLevel" || !IsWindowVisible(h).as_bool() {
                return BOOL(1);
            }
            let mut pid = 0;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut b = [0u16; 300];
                let mut len = b.len() as u32;
                let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(b.as_mut_ptr()), &mut len);
                let _ = CloseHandle(p);
                if ok.is_ok() && String::from_utf16_lossy(&b[..len as usize]).ends_with("Personal Desk.exe") {
                    *(out.0 as *mut bool) = true;
                    return BOOL(0);
                }
            }
        }
        BOOL(1)
    }
    let mut found = false;
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

fn pick_folder(owner: HWND) -> Option<PathBuf> {
    use windows::Win32::System::Com::*;
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        d.SetOptions(d.GetOptions().ok()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        d.Show(Some(owner)).ok()?;
        let p = d.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

// ── Messages ───────────────────────────────────────────────────────────────

pub extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, l: LPARAM) -> LRESULT {
    let fallback = || unsafe { DefWindowProcW(h, msg, wp, l) };
    let (Some(a), Some(n)) = (app(), notch()) else { return fallback() };
    if let Some(k) = a.overlay.as_ref().and_then(|o| o.owns(h)) {
        if msg == WM_SETCURSOR {
            return LRESULT(1);
        }
        let ours = matches!(
            msg,
            WM_NULL | WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MOUSEWHEEL | WM_CHAR | WM_KEYDOWN
        );
        if !ours {
            return fallback();
        }
        // Lend the overlay out so it can read the app.
        let mut o = a.overlay.take().unwrap();
        let out = o.message(a, k, msg, wp, l);
        a.overlay = Some(o);
        a.finish(n, out);
        return LRESULT(0);
    }
    if h != a.main {
        return fallback();
    }
    match msg {
        WM_HOTKEY => match wp.0 {
            1 => a.start(n, Mode::Region),
            2 => a.start(n, Mode::Screen),
            _ => a.front_window(n),
        },
        WM_RUN => {
            if let Some(m) = Mode::ALL.get(wp.0) {
                a.start(n, *m);
            }
        }
        WM_WORK => {
            let w = unsafe { *Box::from_raw(l.0 as *mut Work) };
            match w {
                Work::Shot(f) => {
                    if let Some(id) = a.deliver(f, false) {
                        n.flash(&a.g, &a.store, id);
                    }
                }
                Work::Text(t) => {
                    clip::set_text(a.main, &t);
                    n.done(&a.g, &a.store);
                }
                Work::Failed => n.fail(),
            }
        }
        WM_KEY => {
            if let Some(row) = n.recording.take() {
                stop_key_hook();
                let vk = l.0 as u32;
                if vk != VK_ESCAPE.0 as u32 {
                    let k = if vk == VK_BACK.0 as u32 { (0, 0) } else { (wp.0 as u32, vk) };
                    let s = &mut a.store.settings;
                    *[&mut s.region, &mut s.screen, &mut s.window][row] = k;
                    a.store.save_settings();
                }
                a.register();
                n.repaint();
            }
        }
        WM_DRAG_OUT => {
            let ids = std::mem::take(&mut a.outgoing);
            let files: Vec<PathBuf> =
                ids.iter().filter_map(|i| a.store.get(*i)).map(|i| i.path.clone()).collect();
            n.modal = true;
            n.show(&a.g, &a.store, View::Rest);
            let main = a.main;
            clip::drag(main, &files);
            if let Some(n) = notch() {
                n.modal = false;
            }
        }
        WM_PICK_FOLDER => {
            n.modal = true;
            let f = pick_folder(a.main);
            if let (Some(a), Some(n)) = (app(), notch()) {
                n.modal = false;
                if let Some(f) = f {
                    a.store.settings.folder = f;
                    a.store.save_settings();
                }
                n.repaint();
                unsafe {
                    SetTimer(Some(a.main), T_LEAVE, 1500, None);
                }
            }
        }
        WM_TIMER => match wp.0 {
            T_TICK => {
                let desk = desk_open();
                n.adapt(&a.g, desk);
                let hide = a.overlay.is_none() && fullscreen(&n.mon) && !testing();
                n.hide(hide);
                if a.register() {
                    n.repaint();
                }
                let f = unsafe { GetForegroundWindow() };
                if f != a.last_foreground {
                    a.last_foreground = f;
                    if !hide && a.overlay.is_none() {
                        n.raise();
                    }
                }
            }
            T_COUNT => {
                a.count -= 1;
                if a.count > 0 {
                    n.show(&a.g, &a.store, View::Count(a.count));
                } else {
                    unsafe {
                        let _ = KillTimer(Some(h), T_COUNT);
                    }
                    n.show(&a.g, &a.store, View::Rest);
                    a.start(n, Mode::Region);
                }
            }
            T_INTERVAL => a.tick_interval(n),
            T_HOURLY => {
                let _ = a.store.reload();
                n.refresh(&a.g, &a.store);
            }
            T_TRIM => {
                if !n.expanded() && a.overlay.is_none() {
                    unsafe {
                        let _ = KillTimer(Some(h), T_TRIM);
                    }
                    // The clipboard keeps its own copy; the file serves PNG from here.
                    if let (Some(PngSource::Frame(_)), [id]) = (&a.png, a.clip.as_slice()) {
                        if let Some(i) = a.store.get(*id).filter(|i| i.path.exists()) {
                            a.png = Some(PngSource::File(i.path.clone()));
                        }
                    }
                    n.trim(&a.store);
                    a.g.trim();
                }
            }
            id => {
                n.timer(a, id);
                if id == T_LEAVE {
                    unsafe {
                        SetTimer(Some(h), T_TRIM, 4000, None);
                    }
                }
            }
        },
        WM_CLIPBOARDUPDATE => {
            let ours = unsafe { GetClipboardOwner() }.is_ok_and(|o| o == a.main);
            if !ours && !a.clip.is_empty() {
                a.clip.clear();
                a.png = None;
                n.repaint();
            }
        }
        WM_RENDERFORMAT => {
            if wp.0 as u32 == clip::png_format() {
                if let Some(b) = a.png.as_ref().and_then(|p| p.bytes()) {
                    clip::render_png(&b);
                }
            }
        }
        WM_RENDERALLFORMATS => {
            if let Some(b) = a.png.as_ref().and_then(|p| p.bytes()) {
                clip::render_png_final(h, &b);
            }
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            let desk = desk_open();
            n.adapt(&a.g, desk);
        }
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            stop_key_hook();
            a.unregister();
            unsafe { PostQuitMessage(0) };
        }
        _ => {
            if !notch::message(n, a, msg, wp, l) {
                return fallback();
            }
        }
    }
    LRESULT(0)
}

/// GPU reset: start clean. History is on disk, so nothing is lost.
fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--restarted").spawn();
    }
    std::process::exit(0)
}

fn main() {
    unsafe {
        let name = if testing() { w!("Local\\SignetTest") } else { w!("Local\\Signet") };
        let restarted = std::env::args().any(|a| a == "--restarted");
        let mut lock = CreateMutexW(None, true, name);
        let mut tries = 0;
        while GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(h) = lock {
                let _ = CloseHandle(h);
            }
            tries += 1;
            if !restarted || tries > 40 {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            lock = CreateMutexW(None, true, name);
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = OleInitialize(None);
        let inst: HINSTANCE = GetModuleHandleW(None).unwrap().into();
        let g = Gfx::new().expect("graphics");
        let store = Store::open().expect("index");
        let n = Box::leak(Box::new(Notch::new(&g, inst).expect("notch")));
        let a = Box::leak(Box::new(App {
            g,
            store,
            inst,
            main: n.sensor,
            clip: Vec::new(),
            png: None,
            hot: [false; 3],
            interval: None,
            login: login_on(),
            outgoing: Vec::new(),
            count: 0,
            overlay: None,
            last_foreground: HWND::default(),
        }));
        let (ap, np) = (a as *mut App, n as *mut Notch);
        APP.store(ap, Relaxed);
        NOTCH.store(np, Relaxed);
        {
            let (a, n) = (&mut *ap, &mut *np);
            a.register();
            let _ = AddClipboardFormatListener(a.main);
            SetTimer(Some(a.main), T_TICK, 1500, None);
            SetTimer(Some(a.main), T_HOURLY, 3_600_000, None);
            SetTimer(Some(a.main), T_TRIM, 4000, None);
            n.adapt(&a.g, desk_open());
            n.start();
        }
        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let (a, n) = (&mut *ap, &mut *np);
            let now = Instant::now();
            let mut idle = true;
            if let Some(mut o) = a.overlay.take() {
                if o.landed(now) {
                    o.close();
                } else {
                    if o.busy(now) {
                        idle = false;
                        if o.draw(&a.g, true).is_err() {
                            restart();
                        }
                    }
                    a.overlay = Some(o);
                }
            }
            if n.busy(now) {
                idle = false;
                if n.draw(a).is_err() {
                    restart();
                }
            }
            if idle {
                let _ = WaitMessage();
            }
        }
    }
}
