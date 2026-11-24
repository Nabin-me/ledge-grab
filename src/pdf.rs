//! PNGs to PDF, one page each at its own size, pixels copied without re-encoding.
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
use std::path::PathBuf;

struct Png {
    w: u32,
    h: u32,
    data: Vec<u8>,
}

/// Header and raw IDAT stream of one of our own RGB8 PNGs.
fn parse(b: &[u8]) -> Option<Png> {
    let word = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    if b.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let (w, h) = (word(16)?, word(20)?);
    // 8-bit RGB, not interlaced
    if b.get(24..26)? != [8, 2] || *b.get(28)? != 0 {
        return None;
    }
    let mut data = Vec::new();
    let mut i = 8;
    while let Some(n) = word(i) {
        let n = n as usize;
        match b.get(i + 4..i + 8)? {
            b"IDAT" => data.extend_from_slice(b.get(i + 8..i + 8 + n)?),
            b"IEND" => break,
            _ => {}
        }
        i += n + 12;
    }
    Some(Png { w, h, data })
}

fn load(bytes: Vec<u8>) -> Option<Png> {
    parse(&bytes).or_else(|| parse(&crate::store::encode_png(&crate::store::decode_png(&bytes)?)))
}

pub fn merge(files: &[PathBuf]) -> Option<Vec<u8>> {
    let pages: Vec<Png> = files.iter().filter_map(|f| load(std::fs::read(f).ok()?)).collect();
    if pages.len() != files.len() || pages.is_empty() {
        return None;
    }
    let mut pdf = Pdf::new();
    let (cat, tree) = (Ref::new(1), Ref::new(2));
    let id = |n: usize, k: i32| Ref::new(3 + n as i32 * 3 + k);
    pdf.catalog(cat).pages(tree);
    pdf.pages(tree).kids((0..pages.len()).map(|n| id(n, 0))).count(pages.len() as i32);
    for (n, p) in pages.iter().enumerate() {
        // 96 px per inch
        let (w, h) = (p.w as f32 * 0.75, p.h as f32 * 0.75);
        let mut page = pdf.page(id(n, 0));
        page.media_box(Rect::new(0.0, 0.0, w, h)).parent(tree).contents(id(n, 1));
        page.resources().x_objects().pair(Name(b"I"), id(n, 2));
        page.finish();
        let mut c = Content::new();
        c.save_state();
        c.transform([w, 0.0, 0.0, h, 0.0, 0.0]);
        c.x_object(Name(b"I"));
        c.restore_state();
        pdf.stream(id(n, 1), &c.finish());
        let mut im = pdf.image_xobject(id(n, 2), &p.data);
        im.filter(Filter::FlateDecode);
        im.width(p.w as i32);
        im.height(p.h as i32);
        im.color_space().device_rgb();
        im.bits_per_component(8);
        im.decode_parms().predictor(pdf_writer::types::Predictor::PngOptimum).colors(3).columns(p.w as i32);
        im.finish();
    }
    Some(pdf.finish())
}

#[cfg(test)]
mod tests {
    use crate::capture::Frame;

    #[test]
    fn merges_in_order() {
        let dir = std::env::temp_dir().join("signet-pdf-test");
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<_> = [(40, 20), (30, 60), (100, 10)]
            .iter()
            .enumerate()
            .map(|(n, (w, h))| {
                let mut f = Frame::new(*w, *h);
                f.px.iter_mut().enumerate().for_each(|(i, p)| *p = (i * 7 + n) as u8);
                let p = dir.join(format!("{n}.png"));
                std::fs::write(&p, crate::store::encode_png(&f)).unwrap();
                p
            })
            .collect();
        let out = super::merge(&files).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(out.starts_with(b"%PDF"));
        let pos: Vec<_> = ["[0 0 30 15]", "[0 0 22.5 45]", "[0 0 75 7.5]"].iter().map(|m| s.find(m).expect(m)).collect();
        assert!(pos[0] < pos[1] && pos[1] < pos[2]);
        if let Some(keep) = std::env::var_os("SIGNET_PDF_OUT") {
            std::fs::write(keep, &out).unwrap();
        }
        assert!(super::merge(&[dir.join("missing.png")]).is_none());
    }

    #[test]
    fn png_round_trip() {
        let mut f = Frame::new(33, 17);
        f.px.chunks_exact_mut(4).enumerate().for_each(|(i, p)| p.copy_from_slice(&[i as u8, (i * 3) as u8, (i * 5) as u8, 255]));
        assert_eq!(crate::store::decode_png(&crate::store::encode_png(&f)).unwrap().px, f.px);
    }
}
