//! Conservative, platform-independent overlap and pagination for Scroll to PDF.
//! Only accept a unique, strongly matching forward displacement; never guess a seam.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Still,
    Forward(usize),
    Uncertain,
}

#[derive(Clone, Copy)]
struct Row {
    hash: u64,
    ink: bool,
}

fn fingerprints(pixels: &[u8], width: usize) -> impl Iterator<Item = Row> + '_ {
    pixels.chunks_exact(width * 4).map(move |row| {
        // Omit the outer margins, including a changing scrollbar. Sample
        // alternate pixels to keep the matcher inexpensive on large panes.
        let margin = (width / 16).max(1);
        let mut hash = 0xcbf29ce484222325u64;
        let mut low = 255u8;
        let mut high = 0u8;
        for bgr in row[margin * 4..(width - margin) * 4].chunks_exact(4).step_by(2) {
            let y = ((bgr[0] as u16 + bgr[1] as u16 * 2 + bgr[2] as u16) / 4) as u8;
            low = low.min(y);
            high = high.max(y);
            let rgb = u32::from_le_bytes([bgr[0], bgr[1], bgr[2], 0]);
            hash = (hash ^ u64::from(rgb)).wrapping_mul(0x100000001b3);
        }
        Row { hash, ink: high.saturating_sub(low) > 10 }
    })
}

/// Classify change at a fixed position. This is only a gate for collecting
/// frames, not evidence that any two scrolling views overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stability {
    Identical,
    LocalChange,
    BroadChange,
}

pub fn stability(anchor: &[u8], sample: &[u8], width: usize) -> Stability {
    let Some(stride) = width.checked_mul(4) else { return Stability::BroadChange };
    if width < 24 || anchor.is_empty() || anchor.len() != sample.len() || !anchor.len().is_multiple_of(stride) {
        return Stability::BroadChange;
    }
    let margin = (width / 16).max(1);
    let mut lo_x = width;
    let mut lo_y = anchor.len() / stride;
    let (mut hi_x, mut hi_y, mut changed) = (0, 0, 0);
    // Let a small caret or status icon blink; never treat wide, tall, or
    // separately located changes as settled. Ignore alpha, as in overlap.
    let max_pixels = (anchor.len() / 4 / 100).clamp(1, 680);
    for (y, (a, b)) in anchor.chunks_exact(stride).zip(sample.chunks_exact(stride)).enumerate() {
        let keep = margin * 4..(width - margin) * 4;
        if a[keep.clone()] == b[keep.clone()] { continue; }
        for (i, (p, q)) in a[keep.clone()].chunks_exact(4).zip(b[keep].chunks_exact(4)).enumerate() {
            if p[..3] == q[..3] { continue; }
            let x = margin + i;
            lo_x = lo_x.min(x);
            lo_y = lo_y.min(y);
            hi_x = hi_x.max(x);
            hi_y = hi_y.max(y);
            changed += 1;
            if hi_x - lo_x >= 32 || hi_y - lo_y >= 36 || changed > max_pixels {
                return Stability::BroadChange;
            }
        }
    }
    if changed == 0 { Stability::Identical } else { Stability::LocalChange }
}

pub fn compare(previous: &[u8], current: &[u8], width: usize, height: usize) -> Step {
    let sized = width.checked_mul(height).and_then(|n| n.checked_mul(4));
    if width < 24 || height < 48 || sized != Some(previous.len()) || sized != Some(current.len()) {
        return Step::Uncertain;
    }
    let a: Vec<_> = fingerprints(previous, width).collect();
    let b: Vec<_> = fingerprints(current, width).collect();
    if a.iter().zip(&b).all(|(x, y)| x.hash == y.hash) {
        return Step::Still;
    }
    let min_overlap = (height / 3).max(48);
    let mut best = None;
    let mut ambiguous = false;
    for offset in 1..=height - min_overlap {
        let pairs = a[offset..].iter().zip(&b);
        let mut informative = 0usize;
        let mut matched = 0usize;
        let mut all_matched = 0usize;
        for (x, y) in pairs {
            let same = x.hash == y.hash;
            all_matched += usize::from(same);
            if x.ink || y.ink {
                informative += 1;
                matched += usize::from(same);
            }
        }
        let overlap = height - offset;
        if informative < 12 || matched * 100 < informative * 96 || all_matched * 100 < overlap * 96 {
            continue;
        }
        let score = (matched * 10_000 / informative, all_matched * 10_000 / overlap);
        match best {
            None => best = Some((score, offset)),
            Some((s, _)) if score > s => {
                best = Some((score, offset));
                ambiguous = false;
            }
            Some((s, _)) if score == s => ambiguous = true,
            _ => {}
        }
    }
    match best {
        Some((_, offset)) if !ambiguous => Step::Forward(offset),
        _ => Step::Uncertain,
    }
}

/// Letter pages with half-inch margins have a 540 × 720 pt content box.
/// Prefer a short low-detail band just before the boundary; do not omit rows.
pub fn page_ranges(pixels: &[u8], width: usize) -> Vec<std::ops::Range<usize>> {
    if width < 24 || pixels.is_empty() || width.checked_mul(4).is_none_or(|n| !pixels.len().is_multiple_of(n)) {
        return Vec::new();
    }
    let rows: Vec<_> = fingerprints(pixels, width).map(|r| r.ink).collect();
    let max_rows = width * 4 / 3;
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        let limit = (start + max_rows).min(rows.len());
        let seek = (max_rows / 8).min(100);
        let mut end = limit;
        if limit < rows.len() {
            for i in (limit.saturating_sub(seek).max(start + 8)..=limit).rev() {
                if !rows[i - 4..(i + 4).min(rows.len())].iter().any(|&v| v) {
                    end = i;
                    break;
                }
            }
        }
        ranges.push(start..end);
        start = end;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(width: usize, height: usize) -> Vec<u8> {
        let mut out = vec![255; width * height * 4];
        for y in 0..height {
            if y % 31 < 20 {
                for x in 0..width {
                    let v = if (x * 19 ^ y * 71 ^ x * y * 7) % 91 < 32 { 10 } else { 248 };
                    let i = (y * width + x) * 4;
                    out[i..i + 3].fill(v);
                }
            }
        }
        out
    }

    #[test]
    fn joins_distinct_overlapping_frames_from_arbitrary_start() {
        let (w, h) = (96, 150);
        let all = passage(w, 510);
        let stride = w * 4;
        let view = |top| &all[top * stride..(top + h) * stride];
        let offsets = [41, 114, 192, 211, 263];
        let mut result = view(offsets[0]).to_vec();
        for pair in offsets.windows(2) {
            let d = pair[1] - pair[0];
            assert_eq!(compare(view(pair[0]), view(pair[1]), w, h), Step::Forward(d));
            result.extend_from_slice(&view(pair[1])[(h - d) * stride..]);
        }
        assert_eq!(result, all[offsets[0] * stride..(offsets[4] + h) * stride]);
    }

    #[test]
    fn unchanged_ignores_moving_scrollbar_and_unrelated_frames_stop() {
        let (w, h) = (96, 150);
        let all = passage(w, 510);
        let first = &all[..w * h * 4];
        let mut scrollbar = first.to_vec();
        for row in scrollbar.chunks_exact_mut(w * 4) {
            row[w * 4 - 8..].fill(0);
        }
        assert_eq!(stability(first, &scrollbar, w), Stability::Identical);
        assert_eq!(compare(first, &scrollbar, w, h), Step::Still);
        assert_eq!(compare(first, &all[w * 190 * 4..w * 340 * 4], w, h), Step::Uncertain);
    }

    #[test]
    fn one_small_blink_is_tolerated_but_not_real_motion() {
        let (w, h) = (160, 220);
        let all = passage(w, h + 60);
        let a = &all[..w * h * 4];
        let mut b = a.to_vec();
        for y in 36..59 {
            for x in 45..50 {
                let at = (y * w + x) * 4;
                b[at..at + 3].fill(141);
            }
        }
        assert_eq!(stability(a, &b, w), Stability::LocalChange);
        assert_eq!(stability(a, &all[w * 4..(h + 1) * w * 4], w), Stability::BroadChange);
        assert_eq!(compare(a, &all[31 * w * 4..(h + 31) * w * 4], w, h), Step::Forward(31));
        b[(114 * w + 46) * 4] = 133;
        assert_eq!(stability(a, &b, w), Stability::BroadChange);
    }

    #[test]
    fn ambiguous_repeated_rows_are_not_stitched() {
        let w = 96;
        let h = 150;
        let mut strip = passage(w, 32);
        strip.resize(w * 32 * 4, 255);
        let repeated = strip.repeat(12);
        let stride = w * 4;
        assert_eq!(
            compare(&repeated[..h * stride], &repeated[7 * stride..(h + 7) * stride], w, h),
            Step::Uncertain
        );
    }

    #[test]
    fn pages_are_bounded_and_cover_every_row_once() {
        let (w, h) = (96, 510);
        let rows = page_ranges(&passage(w, h), w);
        assert!(rows.len() >= 4);
        assert_eq!(rows[0].start, 0);
        assert_eq!(rows.last().unwrap().end, h);
        assert!(rows.iter().all(|r| r.len() > 0 && r.len() <= w * 4 / 3));
        assert!(rows.windows(2).all(|p| p[0].end == p[1].start));
    }
}
