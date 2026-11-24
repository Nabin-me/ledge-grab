//! Shots on disk, an index in SQLite, settings beside it.
use crate::capture::Frame;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const THUMB: (i32, i32) = (264, 168);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Shot,
    Pdf,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: i64,
    pub path: PathBuf,
    pub kind: Kind,
    pub created: i64,
    pub pinned: bool,
    pub ord: f64,
    pub stack: i64,
}

#[derive(Clone)]
pub struct Settings {
    pub folder: PathBuf,
    /// Hotkeys as (modifiers, virtual key); key 0 is unset.
    pub region: (u32, u32),
    pub screen: (u32, u32),
    pub window: (u32, u32),
    pub keep_days: i64,
    pub delay: i64,
    pub interval: i64,
    pub last: Option<[i32; 4]>,
}

pub struct Store {
    db: Connection,
    pub items: Vec<Item>,
    pub settings: Settings,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_default())
}

impl Store {
    pub fn open() -> rusqlite::Result<Store> {
        // SIGNET_DIR keeps test runs away from the real history.
        let test = std::env::var_os("SIGNET_DIR").map(PathBuf::from);
        let data = test.clone().unwrap_or_else(|| env_dir("LOCALAPPDATA").join("Signet"));
        let _ = std::fs::create_dir_all(&data);
        let db = Connection::open(data.join("index.db"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-256;
             CREATE TABLE IF NOT EXISTS items(
               id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, kind INTEGER NOT NULL,
               created INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, ord REAL NOT NULL,
               stack INTEGER NOT NULL DEFAULT 0, thumb BLOB);
             CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);",
        )?;
        let pictures = windows_pictures().unwrap_or_else(|| env_dir("USERPROFILE").join("Pictures"));
        let folder = test.map(|t| t.join("shots")).unwrap_or_else(|| pictures.join("Signet"));
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let mut s = Store {
            db,
            items: Vec::new(),
            settings: Settings {
                folder,
                region: ((MOD_CONTROL | MOD_SHIFT).0, 'S' as u32),
                screen: (0, VK_SNAPSHOT.0 as u32),
                window: (MOD_ALT.0, VK_SNAPSHOT.0 as u32),
                keep_days: 7,
                delay: 3,
                interval: 60,
                last: None,
            },
        };
        s.load_settings();
        s.reload()?;
        Ok(s)
    }

    fn kv(&self, k: &str) -> Option<String> {
        self.db.query_row("SELECT v FROM kv WHERE k=?", [k], |r| r.get(0)).optional().ok().flatten()
    }

    fn load_settings(&mut self) {
        let pair = |s: Option<String>, d: (u32, u32)| {
            s.and_then(|v| v.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or(d)
        };
        let num = |s: Option<String>, d: i64| s.and_then(|v| v.parse().ok()).unwrap_or(d);
        let d = self.settings.clone();
        self.settings = Settings {
            folder: self.kv("folder").map(PathBuf::from).unwrap_or(d.folder),
            region: pair(self.kv("region"), d.region),
            screen: pair(self.kv("screen"), d.screen),
            window: pair(self.kv("window"), d.window),
            keep_days: num(self.kv("keep_days"), d.keep_days),
            delay: num(self.kv("delay"), d.delay),
            interval: num(self.kv("interval"), d.interval),
            last: self.kv("last").and_then(|v| {
                let n: Vec<i32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                n.try_into().ok()
            }),
        };
    }

    pub fn save_settings(&self) {
        let s = &self.settings;
        let put = |k: &str, v: String| {
            let _ = self.db.execute("INSERT OR REPLACE INTO kv(k,v) VALUES(?,?)", params![k, v]);
        };
        put("folder", s.folder.to_string_lossy().into_owned());
        for (k, v) in [("region", s.region), ("screen", s.screen), ("window", s.window)] {
            put(k, format!("{},{}", v.0, v.1));
        }
        put("keep_days", s.keep_days.to_string());
        put("delay", s.delay.to_string());
        put("interval", s.interval.to_string());
        if let Some(l) = s.last {
            put("last", l.map(|x| x.to_string()).join(","));
        }
    }

    /// Re-read the index, dropping what expired or was deleted on disk.
    pub fn reload(&mut self) -> rusqlite::Result<()> {
        let mut q = self.db.prepare("SELECT id,path,kind,created,pinned,ord,stack FROM items ORDER BY pinned DESC, ord DESC")?;
        let all: Vec<Item> = q
            .query_map([], |r| {
                Ok(Item {
                    id: r.get(0)?,
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    kind: if r.get::<_, i64>(2)? == 1 { Kind::Pdf } else { Kind::Shot },
                    created: r.get(3)?,
                    pinned: r.get(4)?,
                    ord: r.get(5)?,
                    stack: r.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(q);
        let now = now_ms();
        let oldest = now - self.settings.keep_days * 86_400_000;
        let mut keep = Vec::new();
        for i in all {
            let expired = !i.pinned && i.created < oldest;
            // A shot's file is written in the background, so give it a moment.
            let missing = now - i.created > 10_000 && !i.path.exists();
            if expired || missing {
                let _ = self.db.execute("DELETE FROM items WHERE id=?", [i.id]);
            } else {
                keep.push(i);
            }
        }
        self.items = keep;
        Ok(())
    }

    pub fn get(&self, id: i64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Tiles in display order; each is one item or one stack.
    pub fn groups(&self) -> Vec<Vec<i64>> {
        let mut out: Vec<(i64, Vec<i64>)> = Vec::new();
        for i in &self.items {
            if i.stack != 0 {
                if let Some(g) = out.iter_mut().find(|g| g.0 == i.stack) {
                    g.1.push(i.id);
                    continue;
                }
            }
            out.push((i.stack, vec![i.id]));
        }
        out.into_iter().map(|g| g.1).collect()
    }

    /// A new file name inside this month's folder.
    pub fn new_path(&self, ext: &str) -> PathBuf {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let dir = self.settings.folder.join(format!("{:04}-{:02}", t.wYear, t.wMonth));
        let _ = std::fs::create_dir_all(&dir);
        let stem = format!(
            "{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        );
        let mut p = dir.join(format!("{stem}.{ext}"));
        let mut n = 2;
        while p.exists() || self.items.iter().any(|i| i.path == p) {
            p = dir.join(format!("{stem} {n}.{ext}"));
            n += 1;
        }
        p
    }

    pub fn add(&mut self, path: &Path, kind: Kind, thumb: &Frame) -> rusqlite::Result<i64> {
        let t = now_ms();
        let top = self.items.iter().filter(|i| !i.pinned).map(|i| i.ord).fold(0.0, f64::max);
        let ord = (t as f64).max(top + 1.0);
        self.db.execute(
            "INSERT INTO items(path,kind,created,ord,thumb) VALUES(?,?,?,?,?)",
            params![path.to_string_lossy(), (kind == Kind::Pdf) as i64, t, ord, encode_png(thumb)],
        )?;
        let id = self.db.last_insert_rowid();
        self.reload()?;
        Ok(id)
    }

    pub fn thumb(&self, id: i64) -> Option<Frame> {
        let b: Vec<u8> = self.db.query_row("SELECT thumb FROM items WHERE id=?", [id], |r| r.get(0)).ok()?;
        decode_png(&b)
    }

    pub fn pin(&mut self, ids: &[i64], on: bool) {
        for id in ids {
            let _ = self.db.execute("UPDATE items SET pinned=? WHERE id=?", params![on, id]);
        }
        let _ = self.reload();
    }

    pub fn stack(&mut self, ids: &[i64], stack: i64) {
        let head = ids.iter().filter_map(|i| self.get(*i)).next().map(|i| (i.ord, i.pinned));
        for (n, id) in ids.iter().enumerate() {
            let _ = self.db.execute("UPDATE items SET stack=? WHERE id=?", params![stack, id]);
            if let (Some((ord, pinned)), true) = (head, stack != 0) {
                let _ = self.db.execute(
                    "UPDATE items SET ord=?, pinned=? WHERE id=?",
                    params![ord - n as f64 * 1e-3, pinned, id],
                );
            }
        }
        let _ = self.reload();
    }

    /// Move a tile so it sits before `before` (None puts it last).
    pub fn reorder(&mut self, ids: &[i64], before: Option<i64>) {
        let groups: Vec<Vec<i64>> =
            self.groups().into_iter().filter(|g| !g.iter().any(|i| ids.contains(i))).collect();
        let at = before.and_then(|b| groups.iter().position(|g| g.contains(&b))).unwrap_or(groups.len());
        let ord_of = |g: Option<&Vec<i64>>| g.and_then(|g| self.get(g[0])).map(|i| (i.ord, i.pinned));
        let next = ord_of(groups.get(at));
        let prev = if at > 0 { ord_of(groups.get(at - 1)) } else { None };
        let pinned = match (prev, next) {
            (_, Some(n)) => n.1 && prev.is_none_or(|p| p.1),
            (Some(p), None) => p.1,
            _ => false,
        };
        let same = |o: Option<(f64, bool)>| o.filter(|o| o.1 == pinned).map(|o| o.0);
        let ord = match (same(prev), same(next)) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(a), None) => a - 1000.0,
            (None, Some(b)) => b + 1000.0,
            _ => now_ms() as f64,
        };
        for (n, id) in ids.iter().enumerate() {
            let _ = self.db.execute(
                "UPDATE items SET ord=?, pinned=? WHERE id=?",
                params![ord - n as f64 * 1e-3, pinned, id],
            );
        }
        let _ = self.reload();
    }

    pub fn remove(&mut self, ids: &[i64]) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for id in ids {
            if let Some(i) = self.get(*id) {
                paths.push(i.path.clone());
            }
            let _ = self.db.execute("DELETE FROM items WHERE id=?", [id]);
        }
        let _ = self.reload();
        paths
    }
}

fn windows_pictures() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::*;
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// Opaque RGB PNG. The PDF writer reuses the compressed stream as is.
pub fn encode_png(f: &Frame) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((f.w * f.h * 3) as usize);
    for p in f.px.chunks_exact(4) {
        rgb.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, f.w as u32, f.h as u32);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        if let Ok(mut w) = e.write_header() {
            let _ = w.write_image_data(&rgb);
        }
    }
    out
}

pub fn decode_png(b: &[u8]) -> Option<Frame> {
    let mut d = png::Decoder::new(std::io::Cursor::new(b));
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let i = r.next_frame(&mut buf).ok()?;
    let mut f = Frame::new(i.width as i32, i.height as i32);
    let n = i.color_type.samples();
    for (d, s) in f.px.chunks_exact_mut(4).zip(buf.chunks_exact(n)) {
        match n {
            3 => d.copy_from_slice(&[s[2], s[1], s[0], 255]),
            4 => d.copy_from_slice(&[s[2], s[1], s[0], 255]),
            _ => d.copy_from_slice(&[s[0], s[0], s[0], 255]),
        }
    }
    Some(f)
}

/// Write through a temporary name so a half-written shot never exists.
pub fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
