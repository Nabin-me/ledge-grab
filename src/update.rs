//! Auto-update core, shared by every platform: the signed release manifest,
//! version order, and file checks. Fetching and installing live in the
//! platform layers.
//!
//! Each release carries `ledge-grab-update.txt` (written by
//! `packaging/update-manifest.py`):
//!
//! ```text
//! ledge-grab-update 1
//! version 0.1.3
//! file LedgeGrab-0.1.3-win-x64.exe 4194304 <sha256 hex>
//! file LedgeGrab-Setup-0.1.3-x64.exe 5242880 <sha256 hex>
//! sig <base64 ed25519 signature of every byte above this line>
//! ```
//!
//! Nothing is installed unless the signature checks out against the public key
//! compiled in here, the version is newer than the running one, and the file's
//! size and SHA-256 match the manifest.
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

pub const MANIFEST: &str = "ledge-grab-update.txt";
/// Release key (private half: the UPDATE_SIGNING_KEY secret in GitHub Actions).
const RELEASE_KEY: &str = "c5b14b13e0daed01fa3c75db94244913fc171290abffd3b6525674d95c15c9b4";
const RELEASES: &str = "https://github.com/Nabin-me/ledge-grab/releases";
/// Nothing we ship is anywhere near this; it bounds a bad download.
pub const MAX_FILE: u64 = 200 << 20;
pub const MAX_MANIFEST: u64 = 16 << 10;

/// Only builds made by the release pipeline update themselves; a local
/// `cargo build` never replaces itself with a published release.
pub fn enabled() -> bool {
    option_env!("LEDGE_GRAB_UPDATES") == Some("1")
        && std::env::var_os("LEDGE_GRAB_NO_AUTO_UPDATE").is_none()
}

/// The release key, or the one a test build was compiled with.
pub fn public_key() -> [u8; 32] {
    let hex = option_env!("LEDGE_GRAB_UPDATE_PUBKEY").unwrap_or(RELEASE_KEY);
    unhex(hex).and_then(|v| v.try_into().ok()).expect("valid update public key")
}

/// Where releases live. Tests point this at a local server; signatures are
/// checked the same way, so the override can't install anything unsigned.
pub fn feed() -> String {
    std::env::var("LEDGE_GRAB_UPDATE_FEED").ok().filter(|s| allowed_url(s)).unwrap_or_else(|| RELEASES.into())
}

/// HTTPS anywhere; plain HTTP only to this machine.
fn allowed_url(u: &str) -> bool {
    u.starts_with("https://") || u.starts_with("http://127.0.0.1:") || u.starts_with("http://localhost:")
}

pub fn manifest_url() -> String {
    format!("{}/latest/download/{MANIFEST}", feed())
}

pub fn file_url(v: Version, name: &str) -> String {
    format!("{}/download/v{v}/{name}", feed())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
        let v = Version(it.next()??, it.next()??, it.next()??);
        it.next().is_none().then_some(v)
    }

    /// The running build. Test builds may pose as an older version.
    pub fn current() -> Self {
        let s = option_env!("LEDGE_GRAB_VERSION_OVERRIDE").unwrap_or(env!("CARGO_PKG_VERSION"));
        Self::parse(s).expect("package version is x.y.z")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: Version,
    pub files: Vec<File>,
}

impl Manifest {
    /// Verify and parse a downloaded manifest.
    pub fn verify(text: &[u8], key: &[u8; 32]) -> Result<Self, &'static str> {
        let s = std::str::from_utf8(text).map_err(|_| "manifest is not text")?;
        let at = s.rfind("\nsig ").ok_or("manifest is not signed")? + 1;
        let (body, sig_line) = s.split_at(at);
        let sig = b64(sig_line.trim_start_matches("sig ").trim()).ok_or("bad signature encoding")?;
        let sig = Signature::from_slice(&sig).map_err(|_| "bad signature length")?;
        let key = VerifyingKey::from_bytes(key).map_err(|_| "bad public key")?;
        key.verify_strict(body.as_bytes(), &sig).map_err(|_| "signature doesn't match")?;
        let mut lines = body.lines();
        if lines.next() != Some("ledge-grab-update 1") {
            return Err("unknown manifest format");
        }
        let version = lines
            .next()
            .and_then(|l| l.strip_prefix("version "))
            .and_then(Version::parse)
            .ok_or("manifest has no version")?;
        let mut files = Vec::new();
        for l in lines {
            let mut p = l.split(' ');
            let (Some("file"), Some(name), Some(size), Some(hash), None) = (p.next(), p.next(), p.next(), p.next(), p.next())
            else {
                return Err("bad manifest line");
            };
            let sha256 = unhex(hash).and_then(|v| v.try_into().ok()).ok_or("bad checksum")?;
            let size = size.parse().map_err(|_| "bad size")?;
            // Names carry the version, so a signed old file can't pose as a new one.
            if !name.contains(&format!("-{version}-")) || name.contains(['/', '\\']) || size > MAX_FILE {
                return Err("file doesn't belong to this version");
            }
            files.push(File { name: name.into(), size, sha256 });
        }
        Ok(Manifest { version, files })
    }

    pub fn file(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|f| f.name == name)
    }

    /// The Windows file to install: the installer for installed copies, the
    /// single executable for portable ones.
    pub fn windows_file(&self, installed: bool) -> Option<&File> {
        let v = self.version;
        self.file(&if installed { format!("LedgeGrab-Setup-{v}-x64.exe") } else { format!("LedgeGrab-{v}-win-x64.exe") })
    }
}

/// Does the file on disk match the manifest entry?
pub fn file_matches(path: &Path, f: &File) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    if file.metadata().map(|m| m.len()) .ok() != Some(f.size) {
        return false;
    }
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    h.finalize().as_slice() == f.sha256
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn b64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(&bytes[1..4]),
            3 => out.extend_from_slice(&bytes[1..3]),
            2 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn b64enc(b: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    fn signed(body: &str) -> Vec<u8> {
        let sig = key().sign(body.as_bytes());
        format!("{body}sig {}\n", b64enc(&sig.to_bytes())).into_bytes()
    }

    const BODY: &str = "ledge-grab-update 1\nversion 0.2.0\n\
        file LedgeGrab-0.2.0-win-x64.exe 3 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
        file LedgeGrab-Setup-0.2.0-x64.exe 3 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n";

    #[test]
    fn a_signed_manifest_parses() {
        let m = Manifest::verify(&signed(BODY), key().verifying_key().as_bytes()).unwrap();
        assert_eq!(m.version, Version(0, 2, 0));
        assert_eq!(m.windows_file(true).unwrap().name, "LedgeGrab-Setup-0.2.0-x64.exe");
        assert_eq!(m.windows_file(false).unwrap().size, 3);
    }

    #[test]
    fn tampering_or_another_key_is_refused() {
        let k = *key().verifying_key().as_bytes();
        let mut t = signed(BODY);
        let i = t.iter().position(|&c| c == b'3').unwrap();
        t[i] = b'4';
        assert_eq!(Manifest::verify(&t, &k), Err("signature doesn't match"));
        let other = *SigningKey::from_bytes(&[9u8; 32]).verifying_key().as_bytes();
        assert_eq!(Manifest::verify(&signed(BODY), &other), Err("signature doesn't match"));
        assert_eq!(Manifest::verify(BODY.as_bytes(), &k), Err("manifest is not signed"));
    }

    #[test]
    fn files_must_belong_to_the_signed_version() {
        let body = "ledge-grab-update 1\nversion 0.2.0\nfile LedgeGrab-0.1.0-win-x64.exe 3 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n";
        let k = *key().verifying_key().as_bytes();
        assert_eq!(Manifest::verify(&signed(body), &k), Err("file doesn't belong to this version"));
        let body = "ledge-grab-update 1\nversion 0.2.0\nfile ../LedgeGrab-0.2.0-win-x64.exe 3 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n";
        assert!(Manifest::verify(&signed(body), &k).is_err());
    }

    #[test]
    fn versions_order_numerically() {
        assert!(Version::parse("v0.10.0").unwrap() > Version::parse("0.9.9").unwrap());
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::current(), Version::parse(env!("CARGO_PKG_VERSION")).unwrap());
    }

    #[test]
    fn checksums_and_sizes_are_checked() {
        let dir = std::env::temp_dir().join(format!("lg-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f");
        std::fs::write(&p, b"abc").unwrap();
        let m = Manifest::verify(&signed(BODY), key().verifying_key().as_bytes()).unwrap();
        assert!(file_matches(&p, &m.files[0]));
        std::fs::write(&p, b"abd").unwrap();
        assert!(!file_matches(&p, &m.files[0]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn release_key_and_urls() {
        assert_eq!(public_key().len(), 32);
        assert!(VerifyingKey::from_bytes(&unhex(RELEASE_KEY).unwrap().try_into().unwrap()).is_ok());
        assert!(allowed_url("http://127.0.0.1:8000/x") && !allowed_url("http://example.com/x"));
        assert_eq!(file_url(Version(1, 2, 3), "a.exe"), format!("{}/download/v1.2.3/a.exe", feed()));
        assert_eq!(b64(&b64enc(b"hello!?")).unwrap(), b"hello!?");
    }
}
