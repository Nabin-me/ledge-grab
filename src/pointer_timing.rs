//! Monotonic dwell and grace periods; window timers are only wake-ups.
use std::time::{Duration, Instant};

pub const OPEN_MS: u32 = 185;
pub const CLOSE_MS: u32 = 520;
pub const PANEL_CLOSE_MS: u32 = 800;

/// DIP coordinates inside the actual, narrow resting bar. A click can still
/// open the whole visible bar; only unattended hover uses the inset.
pub fn in_hover_zone(x: f32, y: f32, width: f32) -> bool {
    const SIDE_INSET: f32 = 24.0;
    const BAR_HEIGHT: f32 = 9.0;
    // The empty peek is 112 DIP wide. Keep even a resting bar's active
    // area inside it so the pointer is still over the newly opened panel.
    let half_width = (width / 2.0 - SIDE_INSET).min(54.0);
    (width / 2.0 - half_width..width / 2.0 + half_width).contains(&x)
        && (0.0..BAR_HEIGHT).contains(&y)
}

#[derive(Default)]
pub struct Dwell(Option<Instant>);

#[derive(Debug, PartialEq, Eq)]
pub enum Wake {
    Cancelled,
    Early(u32),
    Ready,
}

impl Dwell {
    /// Start on entry, not on every movement within the same zone.
    pub fn begin(&mut self, now: Instant, delay_ms: u32) -> bool {
        if self.0.is_some() {
            return false;
        }
        self.0 = Some(now + Duration::from_millis(u64::from(delay_ms)));
        true
    }

    pub fn cancel(&mut self) -> bool {
        self.0.take().is_some()
    }

    pub fn poll(&mut self, now: Instant) -> Wake {
        match self.0 {
            None => Wake::Cancelled,
            Some(at) if now < at => {
                // Round up since a posted timer can outlive KillTimer and reach
                // the message queue after a brand-new dwell starts.
                Wake::Early((at - now).as_nanos().div_ceil(1_000_000) as u32)
            }
            Some(_) => {
                self.0 = None;
                Wake::Ready
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_uses_the_center_bar_in_dips() {
        for (x, y) in [(105.0, 4.0), (51.0, 0.0), (158.9, 8.9)] {
            assert!(in_hover_zone(x, y, 210.0));
        }
        for (x, y) in [
            (0.0, 4.0),
            (210.0, 4.0),
            (159.0, 4.0),
            (50.0, 4.0),
            (105.0, 9.0),
            (105.0, -0.5),
        ] {
            assert!(!in_hover_zone(x, y, 210.0));
        }
    }

    #[test]
    fn movement_in_the_same_zone_does_not_postpone_opening() {
        let t = Instant::now();
        let at = |ms| t + Duration::from_millis(ms);
        let mut open = Dwell::default();
        assert!(open.begin(t, OPEN_MS));
        assert!(!open.begin(at(60), OPEN_MS));
        assert_eq!(open.poll(at(184)), Wake::Early(1));
        assert_eq!(open.poll(at(185)), Wake::Ready);
        assert_eq!(open.poll(at(200)), Wake::Cancelled);
    }

    #[test]
    fn grazing_and_stale_wake_ups_cannot_open_the_next_hover_early() {
        let t = Instant::now();
        let at = |ms| t + Duration::from_millis(ms);
        let mut open = Dwell::default();
        assert!(open.begin(t, OPEN_MS));
        assert!(open.cancel());
        assert_eq!(open.poll(at(120)), Wake::Cancelled);
        assert!(open.begin(at(150), OPEN_MS));
        assert_eq!(open.poll(at(185)), Wake::Early(150));
        assert_eq!(open.poll(at(335)), Wake::Ready);
    }

    #[test]
    fn reentering_starts_a_fresh_close_grace_on_the_next_exit() {
        let t = Instant::now();
        let at = |ms| t + Duration::from_millis(ms);
        let mut close = Dwell::default();
        assert!(close.begin(t, CLOSE_MS));
        assert_eq!(close.poll(at(280)), Wake::Early(240));
        assert!(close.cancel());
        assert!(close.begin(at(400), CLOSE_MS));
        assert_eq!(close.poll(at(520)), Wake::Early(400));
        assert_eq!(close.poll(at(920)), Wake::Ready);
    }

    #[test]
    fn panels_have_their_own_longer_close_grace() {
        let t = Instant::now();
        let mut close = Dwell::default();
        assert!(close.begin(t, PANEL_CLOSE_MS));
        assert_eq!(
            close.poll(t + Duration::from_millis(u64::from(CLOSE_MS))),
            Wake::Early(280)
        );
        assert_eq!(
            close.poll(t + Duration::from_millis(u64::from(PANEL_CLOSE_MS))),
            Wake::Ready
        );
    }
}
