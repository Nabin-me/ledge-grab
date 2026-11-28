//! Windows auto-update. A background thread polls the release feed, downloads
//! and verifies the new version, then tells the main window; the main window
//! installs it once Ledge Grab is idle and relaunches.
//!
//! Installed copies (per-user Inno Setup install) run the new installer
//! silently after the app exits. Portable and standalone copies swap their own executable: Windows
//! lets a running exe be renamed, so the old one moves aside to `.old` and is
//! deleted by the next launch.
use crate::note;
use crate::update::{self, Manifest, Version};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, PCWSTR, w};

/// The main window receives this with a `Box<Ready>` in LPARAM.
pub const WM_UPDATE_READY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 20;
const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(10 * 60);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Inno Setup's uninstall key for our AppId.
const UNINSTALL: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{8F3B9C1A-4D2E-4E6B-9A73-2C1F0E5D8B41}_is1");

/// A verified update waiting on disk.
pub struct Ready {
    pub version: Version,
    pub file: PathBuf,
    pub layout: Layout,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Layout {
    /// Installed for this user: run the installer silently.
    UserInstall,
    /// Installed for all users: needs an admin, so no silent updates.
    MachineInstall,
    /// Portable ZIP or the single exe: swap the exe in place.
    Portable,
}

fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn dir() -> PathBuf {
    crate::store::app_data().join("updates")
}

/// Start polling the feed. Does nothing in builds that don't update themselves.
pub fn spawn(main: HWND) {
    if !update::enabled() {
        return;
    }
    let main = main.0 as isize;
    let every = std::env::var("LEDGE_GRAB_UPDATE_EVERY_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs);
    let _ = std::thread::Builder::new().name("updater".into()).spawn(move || {
        std::thread::sleep(every.map_or(FIRST_CHECK, |e| e.min(FIRST_CHECK)));
        remove_old_exe();
        loop {
            match check() {
                Ok(Some(r)) => {
                    note(&format!("update {} ready ({:?})", r.version, r.layout));
                    let p = Box::into_raw(Box::new(r));
                    unsafe {
                        if PostMessageW(Some(HWND(main as *mut _)), WM_UPDATE_READY, WPARAM(0), LPARAM(p as isize)).is_err() {
                            drop(Box::from_raw(p));
                        }
                    }
                    // One update at a time; the next launch polls again.
                    return;
                }
                Ok(None) => {}
                Err(e) => note(&format!("update check: {e}")),
            }
            std::thread::sleep(every.unwrap_or(EVERY));
        }
    });
}

/// One poll: is there a newer signed release? If so, have its file on disk.
fn check() -> Result<Option<Ready>, String> {
    let text = get(&update::manifest_url(), update::MAX_MANIFEST)?;
    let m = Manifest::verify(&text, &update::public_key())?;
    let current = Version::current();
    if m.version <= current {
        return Ok(None);
    }
    if failed_before(m.version) {
        return Ok(None);
    }
    let layout = layout();
    if layout == Layout::MachineInstall {
        return Err(format!("{} is out, but this copy is installed for all users; update it with the installer", m.version));
    }
    let f = m.windows_file(layout == Layout::UserInstall).ok_or("release has no Windows file")?;
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    // Keep only what this update needs.
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n != f.name && !n.starts_with("failed-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let path = d.join(&f.name);
    if !update::file_matches(&path, f) {
        let part = d.join(format!("{}.part", f.name));
        download(&update::file_url(m.version, &f.name), &part, f.size)?;
        if !update::file_matches(&part, f) {
            let _ = std::fs::remove_file(&part);
            return Err("download doesn't match the signed checksum".into());
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    }
    note(&format!("update {current} -> {} downloaded and verified", m.version));
    Ok(Some(Ready { version: m.version, file: path, layout }))
}

/// Install now and relaunch. The caller quits the app right after this
/// returns true; on false nothing changed.
pub fn apply(r: &Ready) -> bool {
    let Some(exe) = exe() else { return false };
    if !r.file.exists() {
        return false;
    }
    let updated = format!("--updated={}", r.version);
    match r.layout {
        Layout::UserInstall => {
            // A hidden PowerShell waits for this process to exit (the installer
            // refuses to run while the app holds its mutex), runs the installer,
            // then relaunches whatever is installed: a failed install brings the
            // old version back.
            let q = |p: &Path| p.display().to_string().replace('\'', "''");
            let script = format!(
                "Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue; \
                 Start-Process -Wait -FilePath '{setup}' -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER','/SP-'; \
                 Start-Process -FilePath '{exe}' -ArgumentList '--restarted','{updated}'",
                pid = std::process::id(),
                setup = q(&r.file),
                exe = q(&exe),
            );
            let spawned = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-EncodedCommand"])
                .arg(utf16_b64(&script))
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            note(&format!("update {}: installer started: {}", r.version, spawned.is_ok()));
            spawned.is_ok()
        }
        Layout::Portable => {
            let old = old_exe(&exe);
            let _ = std::fs::remove_file(&old);
            if std::fs::rename(&exe, &old).is_err() {
                note("update: couldn't move the running exe aside");
                return false;
            }
            if let Err(e) = std::fs::copy(&r.file, &exe) {
                note(&format!("update: couldn't write the new exe: {e}"));
                let _ = std::fs::rename(&old, &exe);
                return false;
            }
            let spawned = std::process::Command::new(&exe).args(["--restarted", &updated]).spawn();
            if spawned.is_err() {
                let _ = std::fs::remove_file(&exe);
                let _ = std::fs::rename(&old, &exe);
                return false;
            }
            note(&format!("update {}: exe swapped, relaunching", r.version));
            true
        }
        Layout::MachineInstall => false,
    }
}

/// After an update relaunch: did we come back as the promised version? If not,
/// remember that, so the same release isn't retried for a day.
pub fn after_relaunch() -> Option<Result<Version, Version>> {
    let want = std::env::args().find_map(|a| a.strip_prefix("--updated=").and_then(Version::parse))?;
    if Version::current() >= want {
        Some(Ok(Version::current()))
    } else {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(dir().join(format!("failed-{want}")), b"");
        note(&format!("update to {want} didn't take; still {}", Version::current()));
        Some(Err(want))
    }
}

/// PowerShell's -EncodedCommand: base64 of UTF-16LE, so no quoting surprises.
fn utf16_b64(s: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn failed_before(v: Version) -> bool {
    let p = dir().join(format!("failed-{v}"));
    let Ok(t) = std::fs::metadata(&p).and_then(|m| m.modified()) else { return false };
    if t.elapsed().unwrap_or_default() > Duration::from_secs(24 * 3600) {
        let _ = std::fs::remove_file(p);
        return false;
    }
    true
}

fn old_exe(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

fn remove_old_exe() {
    if let Some(exe) = exe() {
        let _ = std::fs::remove_file(old_exe(&exe));
    }
}

fn reg_string(root: HKEY, key: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(root, key, value, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)).ok().ok()?;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn same_dir(a: &str, b: &Path) -> bool {
    let a = a.trim_end_matches(['\\', '/']).to_lowercase();
    let b = b.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    !a.is_empty() && a == b
}

/// How this copy was installed: decided by whether the installer's record
/// points at the folder this exe runs from.
pub fn layout() -> Layout {
    let Some(here) = exe().and_then(|e| e.parent().map(Path::to_path_buf)) else { return Layout::Portable };
    let at = |root| reg_string(root, UNINSTALL, w!("InstallLocation")).is_some_and(|l| same_dir(&l, &here));
    if at(HKEY_CURRENT_USER) {
        Layout::UserInstall
    } else if at(HKEY_LOCAL_MACHINE) {
        Layout::MachineInstall
    } else {
        Layout::Portable
    }
}

// ── HTTP (WinHTTP: the system's proxy settings and TLS) ─────────────────────

struct H(*mut core::ffi::c_void);

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn get(url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    fetch(url, max, |b| {
        out.extend_from_slice(b);
        Ok(())
    })?;
    Ok(out)
}

fn download(url: &str, to: &Path, size: u64) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let r = fetch(url, size, |b| f.write_all(b).map_err(|e| e.to_string()));
    if r.is_err() {
        drop(f);
        let _ = std::fs::remove_file(to);
    }
    r
}

/// GET `url`, following redirects, feeding the body to `sink`; fails past `max` bytes.
fn fetch(url: &str, max: u64, mut sink: impl FnMut(&[u8]) -> Result<(), String>) -> Result<(), String> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("unsupported URL".into());
    };
    let (hostport, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "bad port")?),
        None => (hostport, if secure { 443 } else { 80 }),
    };
    let path = if path.is_empty() { "/" } else { path };
    let agent = HSTRING::from(format!("LedgeGrab/{}", Version::current()));
    unsafe {
        let session = H(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        let _ = WinHttpSetTimeouts(session.0, 10_000, 15_000, 30_000, 30_000);
        let conn = H(WinHttpConnect(session.0, &HSTRING::from(host), port, 0));
        if conn.0.is_null() {
            return Err("couldn't reach the update server".into());
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let req = H(WinHttpOpenRequest(conn.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if req.0.is_null() {
            return Err("couldn't open the request".into());
        }
        WinHttpSendRequest(req.0, None, None, 0, 0, 0).map_err(|e| format!("request failed: {e}"))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|e| format!("no response: {e}"))?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("no status: {e}"))?;
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }
        let mut total = 0u64;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read).map_err(|e| format!("read failed: {e}"))?;
            if read == 0 {
                return Ok(());
            }
            total += read as u64;
            if total > max {
                return Err("response is larger than expected".into());
            }
            sink(&buf[..read as usize])?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_folder_compare_ignores_case_and_slash() {
        assert!(same_dir("C:\\Users\\a\\AppData\\Local\\Programs\\Ledge Grab\\", Path::new("c:\\users\\A\\AppData\\Local\\Programs\\Ledge Grab")));
        assert!(!same_dir("", Path::new("C:\\x")));
        assert!(!same_dir("C:\\x\\y", Path::new("C:\\x")));
    }

    #[test]
    fn encoded_command_is_utf16le_base64() {
        assert_eq!(utf16_b64("ab"), "YQBiAA==");
    }

    #[test]
    fn old_exe_sits_next_to_the_exe() {
        assert_eq!(old_exe(Path::new("C:\\a\\ledge-grab.exe")), PathBuf::from("C:\\a\\ledge-grab.exe.old"));
    }
}
