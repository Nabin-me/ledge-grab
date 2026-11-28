//! Things on screen most people would not want in a screenshot.
use crate::frame::Frame;
use crate::ocr::Word;
use regex_lite::Regex;
use std::ops::Range;
use std::sync::OnceLock;

fn rules() -> &'static [(Regex, fn(&str) -> bool)] {
    static R: OnceLock<Vec<(Regex, fn(&str) -> bool)>> = OnceLock::new();
    R.get_or_init(|| {
        let any: fn(&str) -> bool = |_| true;
        // Text recognition likes to put a space around @ and dots.
        [
            (r"[A-Za-z0-9][\w.+-]*\s?@\s?[A-Za-z0-9][\w-]*(\s?\.\s?[A-Za-z][\w-]*)+", any),
            (r"\b(sk|pk|rk|ghp|gho|ghs|ghu|glpat|npm|hf|xox[abpr])[-_][A-Za-z0-9_-]{10,}", any),
            (r"\b(github_pat_|AKIA|ASIA|AIza|ya29\.)[A-Za-z0-9_-]{10,}", any),
            (r"\beyJ[A-Za-z0-9_-]{8,}(\.[A-Za-z0-9_-]+)*", any),
            (r"\b[A-Z]{2}\d{2}(\s?[A-Z0-9]{4}){3,7}(\s?[A-Z0-9]{1,3})?\b", iban as fn(&str) -> bool),
            (r"\b\d([ -]?\d){12,18}\b", luhn),
            (r"\b\d{3}-\d{2}-\d{4}\b", any),
            (r"\+?\(?\d[\d ().-]{7,18}\d", phone),
            (r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b", ip),
            (r"[A-Za-z0-9][A-Za-z0-9_+=-]{15,}", key),
        ]
        .into_iter()
        .map(|(p, f)| (Regex::new(p).unwrap(), f))
        .collect()
    })
}

/// "password: hunter2" gives away the part after the colon.
fn labelled() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(pass(?:word|code|phrase)|pwd|pin|secret|token|api[ _-]?key|key|cvv|cvc|otp)\s*[:=]\s*(\S{3,}(?: ?[A-Za-z0-9]{4,})?)",
        )
        .unwrap()
    })
}

fn digits(s: &str) -> Vec<u32> {
    s.chars().filter_map(|c| c.to_digit(10)).collect()
}

fn luhn(s: &str) -> bool {
    let d = digits(s);
    (13..=19).contains(&d.len())
        && d.iter().rev().enumerate().map(|(i, &v)| if i % 2 == 1 { (v * 2) / 10 + (v * 2) % 10 } else { v }).sum::<u32>() % 10 == 0
        && d.iter().any(|v| *v != d[0])
}

fn iban(s: &str) -> bool {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !(15..=34).contains(&s.len()) {
        return false;
    }
    let turned = s[4..].chars().chain(s[..4].chars());
    let mut m = 0u32;
    for c in turned {
        let v = c.to_digit(36).unwrap_or(0);
        m = if v > 9 { (m * 100 + v) % 97 } else { (m * 10 + v) % 97 };
    }
    m == 1
}

fn phone(s: &str) -> bool {
    let n = digits(s).len();
    let dates = s.matches('-').count() == 2 && n == 8 || s.matches('.').count() >= 3;
    let breaks = s.chars().filter(|c| " -().".contains(*c)).count();
    !dates && (s.starts_with('+') && (8..=15).contains(&n) || (10..=12).contains(&n) && breaks >= 2)
}

fn ip(s: &str) -> bool {
    s.split('.').all(|p| p.parse::<u32>().is_ok_and(|v| v < 256)) && !s.starts_with("0.")
}

/// Long, with letters and digits shuffled together: not a word anyone would read.
fn key(s: &str) -> bool {
    let (mut turns, mut last, mut up, mut low, mut dig) = (0, 0, 0, 0, 0);
    for c in s.chars() {
        let k = if c.is_ascii_digit() {
            dig += 1;
            1
        } else if c.is_ascii_alphabetic() {
            if c.is_ascii_uppercase() { up += 1 } else { low += 1 }
            2
        } else {
            continue;
        };
        turns += (k != last && last != 0) as usize;
        last = k;
    }
    let date = s.len() <= 19 && up + low <= 1;
    dig >= 3 && up + low >= 4 && turns >= 4 && !date
}

/// Byte ranges of one line.
pub fn spans(line: &str) -> Vec<Range<usize>> {
    let mut v: Vec<Range<usize>> = Vec::new();
    for (re, ok) in rules() {
        for m in re.find_iter(line).filter(|m| ok(m.as_str())) {
            v.push(m.range());
        }
    }
    v.extend(labelled().captures_iter(line).filter_map(|c| c.get(2)).map(|m| m.range()));
    v.sort_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in v {
        match out.last_mut() {
            Some(l) if r.start <= l.end => l.end = l.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Rectangles (x, y, w, h) covering every secret.
pub fn find(words: &[Word]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for line in words.chunk_by(|a, b| a.line == b.line) {
        let mut text = String::new();
        let mut at = Vec::new();
        for w in line {
            if !text.is_empty() {
                text.push(' ');
            }
            let s = text.len();
            text.push_str(&w.text);
            at.push(s..text.len());
        }
        for r in spans(&text) {
            let touched = |n: &usize| at[*n].start < r.end && r.start < at[*n].end;
            let Some(mut lo) = (0..line.len()).find(touched) else { continue };
            let mut hi = (0..line.len()).rev().find(touched).unwrap_or(lo);
            // Recognition may split one long word; pieces that touch go too.
            let joined = |a: &Word, b: &Word| b.r[0] - a.r[0] - a.r[2] < 0.2 * a.r[3].max(b.r[3]);
            while lo > 0 && joined(&line[lo - 1], &line[lo]) {
                lo -= 1;
            }
            while hi + 1 < line.len() && joined(&line[hi], &line[hi + 1]) {
                hi += 1;
            }
            let hit: Vec<[f32; 4]> = line[lo..=hi].iter().map(|w| w.r).collect();
            let Some(f) = hit.first() else { continue };
            out.push(hit.iter().fold(*f, |a, b| {
                let (x, y) = (a[0].min(b[0]), a[1].min(b[1]));
                [x, y, (a[0] + a[2]).max(b[0] + b[2]) - x, (a[1] + a[3]).max(b[1] + b[3]) - y]
            }));
        }
    }
    out
}

/// Recognition sometimes drops the last letter. Grow sideways to where the ink really ends,
/// but never over a word it did read. Scraps of a letter or two may be the missing end.
pub fn snug(f: &Frame, words: &[Word], r: [f32; 4]) -> [f32; 4] {
    let (x0, x1) = ((r[0] as i32).clamp(0, f.w), ((r[0] + r[2]).ceil() as i32).clamp(0, f.w));
    let (y0, y1) = ((r[1] as i32).clamp(0, f.h), ((r[1] + r[3]).ceil() as i32).clamp(0, f.h));
    let at = |x: i32, y: i32| {
        let i = ((y * f.w + x) * 4) as usize;
        [f.px[i], f.px[i + 1], f.px[i + 2]]
    };
    let mut seen: std::collections::HashMap<[u8; 3], (u32, [u8; 3])> = Default::default();
    for y in y0..y1 {
        for x in x0..x1 {
            let c = at(x, y);
            seen.entry(c.map(|v| v >> 4)).or_insert((0, c)).0 += 1;
        }
    }
    let Some((_, paper)) = seen.into_values().max_by_key(|v| v.0) else { return r };
    let ink = |x: i32| (y0..y1).any(|y| (0..3).any(|k| at(x, y)[k].abs_diff(paper[k]) > 48));
    // Letters of one word sit closer together than words do.
    let gap = ((r[3] * 0.36) as i32).max(2);
    let beside = || words.iter().filter(|w| w.text.len() > 2).map(|w| w.r).filter(|w| w[1] < r[1] + r[3] && r[1] < w[1] + w[3]);
    let lo = beside().map(|w| w[0] + w[2]).filter(|x| *x <= r[0] + 2.0).fold(0.0, f32::max) as i32;
    let hi = beside().map(|w| w[0]).filter(|x| *x >= r[0] + r[2] - 2.0).fold(f.w as f32, f32::min) as i32;
    let reach = (r[3] * 4.0) as i32;
    let walk = |from: i32, step: i32| {
        let (mut edge, mut x) = (from, from);
        while (x - edge).abs() <= gap && (x - from).abs() < reach && (lo..hi).contains(&x) {
            if ink(x) {
                edge = x;
            }
            x += step;
        }
        edge
    };
    let (a, b) = (walk(x0, -1), walk((x1 - 1).max(x0), 1) + 1);
    [a as f32, r[1], (b - a) as f32, r[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_matters() {
        for s in [
            "ada@northlight.example",
            "mail ada @ northlight . example now",
            concat!("ghp_", "16C7e42F292c6912E7710c838347Ae178B4a"),
            "nl-demo-7QX2-KM9P-40TV",
            concat!("AKIA", "IOSFODNN7EXAMPLE"),
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIi.abc",
            "card 4242 4242 4242 4242",
            "GB82 WEST 1234 5698 7654 32",
            "+977 984-1234567",
            "(415) 555-0132",
            "Password: hunter2",
            "192.168.1.20",
        ] {
            assert!(!spans(s).is_empty(), "missed {s}");
        }
    }

    #[test]
    fn leaves_the_rest() {
        for s in [
            "Quarter in review",
            "2026-11-23 19.22.06",
            "$1,240.00 on 4 Mar",
            "https docs rs windows latest",
            "internationalization_configuration",
            "version 10.0.26300 build",
            "1234 5678 9012 3456",
            "src/overlay.rs:1067",
        ] {
            assert!(spans(s).is_empty(), "wrongly hid {s}: {:?}", spans(s).iter().map(|r| &s[r.clone()]).collect::<Vec<_>>());
        }
    }

    #[test]
    fn grows_to_the_ink() {
        let mut f = Frame::new(200, 40);
        f.px.fill(255);
        // Letters 5 wide, 3 apart; a word gap of 9 before x = 10 and after x = 120.
        for x in (4..10).chain((19..120).filter(|x| (x - 19) % 8 < 5)).chain(129..150) {
            for y in 12..22 {
                let i = ((y * 200 + x) * 4) as usize;
                f.px[i..i + 3].fill(20);
            }
        }
        let word = |x: f32, w: f32| Word { text: "abc".into(), r: [x, 10.0, w, 14.0], line: 0 };
        let r = snug(&f, &[word(4.0, 6.0), word(129.0, 21.0)], [19.0, 9.0, 85.0, 20.0]);
        assert_eq!((r[0], r[0] + r[2]), (19.0, 120.0));
    }

    #[test]
    fn covers_split_words() {
        let w = |t: &str, x: f32| Word { text: t.into(), r: [x, 10.0, 50.0, 12.0], line: 0 };
        let r = find(&[w("To:", 0.0), w("ada@", 60.0), w("sun.example", 112.0), w("today", 200.0)]);
        assert_eq!(r, vec![[60.0, 10.0, 102.0, 12.0]]);
        let r = find(&[w("Key", 0.0), w("sk-demo_7QX2KM9P40TVaa1", 60.0), w("2", 111.0), w("ok", 170.0)]);
        assert_eq!(r, vec![[60.0, 10.0, 101.0, 12.0]]);
    }
}
