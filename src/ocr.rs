//! Windows' own offline text recognition.
use crate::capture::Frame;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::Result;

#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    /// x, y, w, h in frame pixels.
    pub r: [f32; 4],
    pub line: usize,
}

/// Blocking; call from a worker thread.
pub fn words(f: &Frame) -> Result<Vec<Word>> {
    unsafe {
        use windows::Win32::System::WinRT::*;
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let w = DataWriter::new()?;
    w.WriteBytes(&f.px)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&w.DetachBuffer()?, BitmapPixelFormat::Bgra8, f.w, f.h)?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
    let res = engine.RecognizeAsync(&bmp)?.join()?;
    let mut out = Vec::new();
    for (n, l) in res.Lines()?.into_iter().enumerate() {
        for word in l.Words()? {
            let r = word.BoundingRect()?;
            out.push(Word { text: word.Text()?.to_string(), r: [r.X, r.Y, r.Width, r.Height], line: n });
        }
    }
    Ok(out)
}

pub fn text(words: &[Word]) -> String {
    let mut s = String::new();
    let mut line = 0;
    for w in words {
        if !s.is_empty() {
            s.push(if w.line == line { ' ' } else { '\n' });
        }
        line = w.line;
        s.push_str(&w.text);
    }
    s
}
