//! Motion. Curves match Personal Desk's notch.
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum Curve {
    /// Growing or arriving: the iOS sheet curve.
    Emphasized,
    /// Shrinking or moving.
    Standard,
    /// Tactile spring catch with gentle ~7% overshoot.
    Spring,
}

fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let f = |a: f32, b: f32, t: f32| 3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut lo, mut hi, mut t) = (0.0f32, 1.0f32, x);
    for _ in 0..16 {
        if f(x1, x2, t) < x {
            lo = t
        } else {
            hi = t
        }
        t = (lo + hi) / 2.0;
    }
    f(y1, y2, t)
}

fn spring(x: f32) -> f32 {
    let raw = |t: f32| 1.0 - (-6.2 * t).exp() * (7.4 * t).cos();
    raw(x) / raw(1.0)
}

impl Curve {
    pub fn at(self, x: f32) -> f32 {
        match x {
            x if x <= 0.0 => 0.0,
            x if x >= 1.0 => 1.0,
            x => match self {
                Curve::Emphasized => bezier(0.32, 0.72, 0.0, 1.0, x),
                Curve::Standard => bezier(0.4, 0.0, 0.2, 1.0, x),
                Curve::Spring => spring(x),
            },
        }
    }
}

#[derive(Clone, Copy)]
pub struct Anim {
    from: f32,
    pub to: f32,
    start: Instant,
    dur: f32,
    curve: Curve,
}

impl Anim {
    pub fn new(v: f32) -> Self {
        Anim { from: v, to: v, start: Instant::now(), dur: 0.0, curve: Curve::Standard }
    }

    pub fn progress(&self, now: Instant) -> f32 {
        if self.dur <= 0.0 {
            return 1.0;
        }
        (now.saturating_duration_since(self.start).as_secs_f32() / self.dur).min(1.0)
    }

    pub fn get(&self, now: Instant) -> f32 {
        self.from + (self.to - self.from) * self.curve.at(self.progress(now))
    }

    pub fn busy(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    pub fn go(&mut self, to: f32, dur: f32, curve: Curve) {
        if self.to == to {
            return;
        }
        let now = Instant::now();
        *self = Anim { from: self.get(now), to, start: now, dur, curve };
    }

    pub fn jump(&mut self, v: f32) {
        *self = Anim::new(v);
    }
}

#[cfg(test)]
mod tests {
    use super::Curve;

    #[test]
    fn spring_catches_and_settles() {
        assert_eq!(Curve::Spring.at(0.0), 0.0);
        assert_eq!(Curve::Spring.at(1.0), 1.0);
        let peak = (1..100).map(|i| Curve::Spring.at(i as f32 / 100.0)).fold(0.0f32, f32::max);
        assert!(peak > 1.04 && peak < 1.12, "unexpected spring peak {peak}");
    }
}
