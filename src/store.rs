//! Shots on disk, an index in SQLite, settings beside it.
use crate::capture::Frame;
use rusqlite::{Connection, OptionalExtension, params};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const THUMB: (i32, i32) = (264, 168);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Shot = 0,
    Pdf = 1,
    Clip = 2,
    Gif = 4,
}

impl Settings {
    pub fn keys(&self) -> [(u32, u32); 4] {
        [self.region, self.screen, self.window, self.clip]
    }
}

impl Kind {
    fn of(n: i64) -> Kind {
        match n {
            1 => Kind::Pdf,
            2 => Kind::Clip,
            4 => Kind::Gif,
            _ => Kind::Shot,
        }
    }
}

/// What the shelf shows.
#[derive(Clone, PartialEq, Debug)]
pub enum Filter {
    All,
    Clips,
    /// One folder per app.
    Apps,
    App(String),
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
    /// Where it came from.
    pub app: String,
    /// Seconds, for clips.
    pub secs: i64,
}

impl Item {
    /// Older indexes could store a hosted window's title in the app field.
    /// Keep those labels in the existing shelf, but don't publish them as new
    /// folder names when generating a derivative from a pre-date-layout item.
    pub fn export_app(&self) -> &str {
        let Some(dir) = self.path.parent() else { return "" };
        let day = dir.parent().and_then(Path::file_name).and_then(|n| n.to_str()).unwrap_or("");
        let expected_app_folder = app_folder(&self.app);
        if is_date_folder(day) && dir.file_name().and_then(|n| n.to_str()) == Some(expected_app_folder.as_str()) {
            &self.app
        } else {
            ""
        }
    }
}

#[derive(Clone)]
pub struct Settings {
    pub folder: PathBuf,
    /// Hotkeys as (modifiers, virtual key); key 0 is unset.
    pub region: (u32, u32),
    pub screen: (u32, u32),
    pub window: (u32, u32),
    pub clip: (u32, u32),
    /// Keep images copied in other apps.
    pub history: bool,
    pub keep_days: i64,
    pub delay: i64,
    pub interval: i64,
    pub last: Option<[i32; 4]>,
}

pub struct Store {
    db: Connection,
    pub items: Vec<Item>,
    pub settings: Settings,
    pub filter: Filter,
    next_file: Cell<u64>,
}

/// Folders are independent of capture stacks and show the most recent app first.
fn group_by_app(items: &[Item]) -> Vec<Vec<i64>> {
    let mut by_time: Vec<&Item> = items.iter().collect();
    by_time.sort_by_key(|i| std::cmp::Reverse((i.created, i.id)));
    let mut indices = std::collections::HashMap::<&str, usize>::new();
    let mut folders: Vec<Vec<i64>> = Vec::new();
    for i in by_time {
        let idx = *indices.entry(i.app.trim()).or_insert_with(|| {
            folders.push(Vec::new());
            folders.len() - 1
        });
        folders[idx].push(i.id);
    }
    folders
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_default())
}

/// Use Windows-compatible, readable components without allowing application names
/// to change the parent directory. Names shortened or escaped get a stable suffix
/// so two different application names don't collapse into the same folder.
fn app_folder(app: &str) -> String {
    let app = app.trim();
    if app.is_empty() {
        return "Other".into();
    }

    let mut safe = String::new();
    let mut units = 0;
    for ch in app.chars() {
        let ch = if ch.is_control() || "\\/:*?\"<>|~".contains(ch) { '_' } else { ch };
        units += ch.len_utf16();
        if units > 56 {
            break;
        }
        safe.push(ch);
    }
    let safe = safe.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let base = safe.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") ||
        ["COM", "LPT"].into_iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
        });
    if safe == app && !reserved {
        return safe.into();
    }
    let hash = app.to_lowercase().bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    // Prefix reserved device names, even if they appear before a dot.
    let prefix = if reserved { "_" } else { "" };
    let label = if safe.is_empty() { "App" } else { safe };
    format!("{prefix}{label}~{hash:016x}")
}

fn is_date_folder(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| match i {
        4 | 7 => *c == b'-',
        _ => c.is_ascii_digit(),
    })
}

fn dated_location(root: &Path, app: &str, [year, month, day]: [u16; 3], [hour, min, sec, ms]: [u16; 4]) -> (PathBuf, String) {
    let date = format!("{year:04}-{month:02}-{day:02}");
    let dir = root.join(&date).join(app_folder(app));
    (dir, format!("{date} {hour:02}.{min:02}.{sec:02}.{ms:03}"))
}

impl Store {
    pub fn open() -> rusqlite::Result<Store> {
        // LEDGE_GRAB_DIR keeps test runs away from the real history.
        let test = std::env::var_os("LEDGE_GRAB_DIR").map(PathBuf::from);
        let data = test.clone().unwrap_or_else(|| env_dir("LOCALAPPDATA").join("Ledge Grab"));
        let pictures = windows_pictures().unwrap_or_else(|| env_dir("USERPROFILE").join("Pictures"));
        let folder = test.map(|t| t.join("shots")).unwrap_or_else(|| pictures.join("Ledge Grab"));
        Self::open_at(&data, folder)
    }

    fn open_at(data: &Path, folder: PathBuf) -> rusqlite::Result<Store> {
        let _ = std::fs::create_dir_all(data);
        let db = Connection::open(data.join("index.db"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-256;
             CREATE TABLE IF NOT EXISTS items(
               id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, kind INTEGER NOT NULL,
               created INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, ord REAL NOT NULL,
               stack INTEGER NOT NULL DEFAULT 0, thumb BLOB);
             CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT NOT NULL);",
        )?;
        // Added after the first version.
        for c in ["app TEXT NOT NULL DEFAULT ''", "preview TEXT NOT NULL DEFAULT ''", "body TEXT", "secs INTEGER NOT NULL DEFAULT 0"] {
            let _ = db.execute(&format!("ALTER TABLE items ADD COLUMN {c}"), []);
        }
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let mut s = Store {
            db,
            items: Vec::new(),
            settings: Settings {
                folder,
                region: ((MOD_CONTROL | MOD_SHIFT).0, 'S' as u32),
                screen: (0, VK_SNAPSHOT.0 as u32),
                window: (MOD_ALT.0, VK_SNAPSHOT.0 as u32),
                clip: (MOD_SHIFT.0, VK_SNAPSHOT.0 as u32),
                history: true,
                keep_days: 7,
                delay: 3,
                interval: 60,
                last: None,
            },
            filter: Filter::All,
            next_file: Cell::new(0),
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
            clip: pair(self.kv("clip"), d.clip),
            history: self.kv("history").map(|v| v == "1").unwrap_or(d.history),
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
        for (k, v) in [("region", s.region), ("screen", s.screen), ("window", s.window), ("clip", s.clip)] {
            put(k, format!("{},{}", v.0, v.1));
        }
        put("history", (s.history as i64).to_string());
        put("keep_days", s.keep_days.to_string());
        put("delay", s.delay.to_string());
        put("interval", s.interval.to_string());
        if let Some(l) = s.last {
            put("last", l.map(|x| x.to_string()).join(","));
        }
    }

    /// Re-read the index, dropping what expired or was deleted on disk.
    pub fn reload(&mut self) -> rusqlite::Result<()> {
        let mut q = self
            .db
            .prepare("SELECT id,path,kind,created,pinned,ord,stack,app,secs FROM items WHERE kind != 3 ORDER BY pinned DESC, ord DESC")?;
        let all: Vec<Item> = q
            .query_map([], |r| {
                Ok(Item {
                    id: r.get(0)?,
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    kind: Kind::of(r.get(2)?),
                    created: r.get(3)?,
                    pinned: r.get(4)?,
                    ord: r.get(5)?,
                    stack: r.get(6)?,
                    app: r.get(7)?,
                    secs: r.get(8)?,
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
    fn groups(&self, keep: impl Fn(&Item) -> bool) -> Vec<Vec<i64>> {
        let mut out: Vec<(i64, Vec<i64>)> = Vec::new();
        for i in self.items.iter().filter(|i| keep(i)) {
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

    /// The notch's first row.
    pub fn recent(&self) -> Vec<Vec<i64>> {
        self.groups(|_| true)
    }

    pub fn shelf(&self) -> Vec<Vec<i64>> {
        match &self.filter {
            Filter::All => self.groups(|_| true),
            Filter::Clips => self.groups(|i| matches!(i.kind, Kind::Clip | Kind::Gif)),
            Filter::App(a) => self.groups(|i| i.app.trim() == a.as_str()),
            Filter::Apps => group_by_app(&self.items),
        }
    }

    pub fn has(&self, f: impl Fn(&Item) -> bool) -> bool {
        self.items.iter().any(f)
    }

    /// A new file path under the capture's local day and source app.
    /// Existing items hold their exact paths in SQLite; no migration is needed.
    pub fn new_path(&self, ext: &str, app: &str) -> std::io::Result<PathBuf> {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let (dir, stem) = dated_location(
            &self.settings.folder,
            app,
            [t.wYear, t.wMonth, t.wDay],
            [t.wHour, t.wMinute, t.wSecond, t.wMilliseconds],
        );
        std::fs::create_dir_all(&dir)?;
        // A GIF batch can queue multiple writes before any of them reaches SQLite
        // or disk. Allocate a distinct stem to each worker, including its .part file.
        loop {
            let n = self.next_file.get() + 1;
            self.next_file.set(n);
            let p = dir.join(format!("{stem} {n}.{ext}"));
            if !p.exists() && !p.with_extension("part").exists() && !self.items.iter().any(|i| i.path == p) {
                return Ok(p);
            }
        }
    }

    pub fn add(&mut self, path: &Path, kind: Kind, thumb: &Frame, app: &str, secs: i64) -> rusqlite::Result<i64> {
        let (t, ord) = self.top();
        self.db.execute(
            "INSERT INTO items(path,kind,created,ord,thumb,app,secs) VALUES(?,?,?,?,?,?,?)",
            params![path.to_string_lossy(), kind as i64, t, ord, encode_png(thumb), app, secs],
        )?;
        let id = self.db.last_insert_rowid();
        self.reload()?;
        Ok(id)
    }

    fn top(&self) -> (i64, f64) {
        let t = now_ms();
        let top = self.items.iter().filter(|i| !i.pinned).map(|i| i.ord).fold(0.0, f64::max);
        (t, (t as f64).max(top + 1.0))
    }

    /// Give a shot's file a readable name. Returns the new path.
    pub fn rename(&mut self, id: i64, title: &str) -> Option<PathBuf> {
        let from = self.get(id)?.path.clone();
        let (dir, ext) = (from.parent()?.to_path_buf(), from.extension()?.to_string_lossy().into_owned());
        let mut to = dir.join(format!("{title}.{ext}"));
        let mut n = 2;
        while to.exists() || self.items.iter().any(|i| i.path == to) {
            to = dir.join(format!("{title} {n}.{ext}"));
            n += 1;
        }
        std::fs::rename(&from, &to).ok()?;
        self.db.execute("UPDATE items SET path=? WHERE id=?", params![to.to_string_lossy(), id]).ok()?;
        self.reload().ok()?;
        Some(to)
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
            self.shelf().into_iter().filter(|g| !g.iter().any(|i| ids.contains(i))).collect();
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

/// A file name out of what a picture says: its biggest line of text.
pub fn title(words: &[crate::ocr::Word]) -> Option<String> {
    let mut best: Option<(f32, String)> = None;
    for line in words.chunk_by(|a, b| a.line == b.line) {
        let text = line.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let letters = text.chars().filter(|c| c.is_alphabetic()).count();
        if letters < 4 || letters * 2 < text.chars().count() || !crate::secrets::spans(&text).is_empty() {
            continue;
        }
        let size = line.iter().map(|w| w.r[3]).fold(0.0, f32::max);
        // Letters like y make a line look taller. Near ties go to the one nearer the top, which comes first.
        if best.as_ref().is_none_or(|b| size > b.0 * 1.4) {
            best = Some((size, text));
        }
    }
    let clean: String =
        best?.1.chars().map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { ' ' } else { c }).collect();
    let mut out = String::new();
    for w in clean.split_whitespace() {
        if !out.is_empty() && out.chars().count() + w.chars().count() > 47 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(w.chars().take(48));
    }
    // Windows drops trailing dots, and keeps a few names to itself.
    let out = out.trim_matches(|c: char| c == '.' || c == '-' || c.is_whitespace()).to_string();
    let reserved = ["con", "prn", "aux", "nul"].contains(&out.to_lowercase().as_str());
    (out.chars().count() >= 4 && !reserved).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::Word;

    #[test]
    fn app_folders_include_unknown_and_follow_capture_recency() {
        let item = |id, app: &str, created, stack| Item {
            id, app: app.into(), created, stack, path: PathBuf::new(), kind: Kind::Shot,
            pinned: false, ord: 0.0, secs: 0,
        };
        let captures = [
            item(1, "Premiere Pro", 100, 7),
            item(2, "   ", 120, 0),
            item(3, "Firefox", 130, 7),
            item(4, " Premiere Pro ", 160, 0),
            item(5, "", 120, 0),
        ];
        assert_eq!(group_by_app(&captures), vec![vec![4, 1], vec![3], vec![5, 2]]);
        assert!(group_by_app(&[]).is_empty());
    }

    #[test]
    fn dated_paths_use_local_day_and_source_app() {
        let root = Path::new("Captures");
        let first = dated_location(root, "Adobe Premiere Pro", [2026, 11, 25], [23, 59, 59, 998]);
        let next = dated_location(root, "Adobe Premiere Pro", [2026, 11, 26], [0, 0, 0, 0]);
        assert_eq!(first.0, root.join("2026-11-25").join("Adobe Premiere Pro"));
        assert_eq!(first.1, "2026-11-25 23.59.59.998");
        assert_eq!(next.0, root.join("2026-11-26").join("Adobe Premiere Pro"));
        assert_eq!(dated_location(root, "", [2026, 11, 26], [0; 4]).0, root.join("2026-11-26").join("Other"));
    }

    #[test]
    fn app_folders_cannot_traverse_or_alias_windows_devices() {
        for name in [".", "..", "C:\\Temp", "app/another", "COM¹.txt", "con.txt", "NUL", "LPT9", "COM1 .ext", "CONIN$", "AUX", ".app.", "My\0App"] {
            let component = app_folder(name);
            assert_eq!(Path::new(&component).components().count(), 1, "{name}");
            assert!(component.contains('~'), "{name}: {component}");
            assert!(!component.ends_with(['.', ' ']), "{component}");
        }
        assert_ne!(app_folder("A/B"), app_folder("A:B"));
        assert_ne!(app_folder("A/B"), app_folder("A_B"));
        assert_eq!(app_folder("Café 3D"), "Café 3D");
        let long = "😊".repeat(100);
        assert!(app_folder(&long).encode_utf16().count() <= 73);
    }

    #[test]
    fn saving_new_captures_keeps_legacy_paths_in_the_index() {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let temp = Cleanup(std::env::temp_dir().join(format!("ledge-path-test-{}-{nonce}", std::process::id())));
        let base = temp.0.join("shots");
        let mut store = Store::open_at(&temp.0, base.clone()).unwrap();
        let legacy = base.join("2026-11").join("old capture.png");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let thumb = Frame::new(2, 2);
        write_file(&legacy, &encode_png(&thumb)).unwrap();
        let old_id = store.add(&legacy, Kind::Shot, &thumb, "Explorer", 0).unwrap();

        let path = store.new_path("png", "Explorer").unwrap();
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "Explorer");
        assert_eq!(path.strip_prefix(&base).unwrap().components().count(), 3);
        let another = store.new_path("png", "Explorer").unwrap();
        assert_ne!(path, another);
        write_file(&path, &encode_png(&thumb)).unwrap();
        let new_id = store.add(&path, Kind::Shot, &thumb, "Explorer", 0).unwrap();
        drop(store);

        let reopened = Store::open_at(&temp.0, base).unwrap();
        let old_item = reopened.get(old_id).unwrap();
        let new_item = reopened.get(new_id).unwrap();
        assert_eq!(old_item.path, legacy);
        assert_eq!(new_item.path, path);
        assert_eq!(old_item.export_app(), "");
        assert_eq!(new_item.export_app(), "Explorer");
        assert!(legacy.is_file() && path.is_file());
    }

    #[test]
    fn names_from_the_headline() {
        let l = |n: usize, t: &str, h: f32| -> Vec<Word> {
            t.split(' ').map(|w| Word { text: w.into(), r: [0.0, 0.0, 10.0, h], line: n }).collect()
        };
        let page = [l(0, "File Edit View", 12.0), l(1, "March invoices: 2026", 34.0), l(2, "$8,025.50", 40.0), l(3, "ada@sun.example", 50.0)];
        assert_eq!(title(&page.concat()).unwrap(), "March invoices 2026");
        assert_eq!(title(&l(0, "12:40 99%", 30.0)), None);
        assert!(title(&l(0, &"word ".repeat(40), 30.0)).unwrap().chars().count() <= 48);
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
            4 => {
                // See-through pictures land on white.
                let on = |c: u8| ((c as u32 * s[3] as u32 + 255 * (255 - s[3] as u32)) / 255) as u8;
                d.copy_from_slice(&[on(s[2]), on(s[1]), on(s[0]), 255])
            }
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
