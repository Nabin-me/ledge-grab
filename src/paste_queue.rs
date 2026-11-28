//! Sequential Copy: one shelf image on the clipboard at a time, advanced only when
//! the target app has really read it.
//!
//! Every queued image is placed with delayed rendering, so the first read by any app
//! reaches us as WM_RENDERFORMAT. A paste keystroke arms the queue; the read that
//! follows advances it. A paste that arrives while the next image is still being
//! placed is held back and replayed once the swap is done, so rapid presses neither
//! collapse into one advance nor paste the same image twice. When we can't tell
//! whether a paste happened, the queue stops and asks for a manual Next instead of
//! guessing.
use std::time::{Duration, Instant};

/// A paste press with no clipboard read after this long is treated as not seen.
pub const READ_TIMEOUT: Duration = Duration::from_millis(1500);
/// Quiet time after the target's read before the next image replaces it.
pub const SETTLE: Duration = Duration::from_millis(250);
/// Attempts to place the next image while another app holds the clipboard.
pub const PLACE_TRIES: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stall {
    /// A paste key was pressed but nothing read the clipboard.
    NotRead,
    /// Something read the clipboard without a paste key (menu paste, a monitor).
    ReadElsewhere,
    /// The clipboard stayed busy, or the swap failed and left it empty.
    Busy,
    /// The next item can't be loaded. The clipboard is emptied so a paste can't
    /// repeat the previous image.
    Missing,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// The current image is on the clipboard and nobody has pasted it yet.
    Ready,
    /// A paste key went to the target app; waiting for it to read.
    Pressed(Instant),
    /// The target read the current image; the next one is being placed.
    Swapping,
    Stalled(Stall),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Let the key reach the target app.
    Pass,
    /// Hold the key back; it is replayed when the next image is in place.
    Defer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placed {
    Ok,
    /// Nothing changed; the clipboard still holds the previous image.
    Busy,
    /// The clipboard was emptied but the image could not be set.
    Cleared,
    /// The item could not be loaded.
    Missing,
}

/// What the app should do after a step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Then {
    Nothing,
    /// Put `items[pos]` on the clipboard and report back with `placed`.
    Place,
    /// Replay one held paste keystroke.
    Replay,
    /// Every item has been pasted.
    Finished,
    /// Try placing again shortly.
    Retry,
}

#[derive(Clone, Debug)]
pub struct Queue {
    pub items: Vec<i64>,
    /// Index of the item on the clipboard, or about to be.
    pub pos: usize,
    pub phase: Phase,
    /// Paste presses held back during a swap.
    pub held: u32,
    tries: u8,
}

impl Queue {
    /// `items` in shelf order. The first one must already be on the clipboard.
    pub fn new(items: Vec<i64>) -> Self {
        Queue { items, pos: 0, phase: Phase::Ready, held: 0, tries: 0 }
    }

    pub fn current(&self) -> Option<i64> {
        self.items.get(self.pos).copied()
    }

    /// 1-based position and total, for "Next 1 of 3".
    pub fn progress(&self) -> (usize, usize) {
        ((self.pos + 1).min(self.items.len()), self.items.len())
    }

    /// A paste keystroke, decided before the target app sees it.
    pub fn key(&mut self, now: Instant) -> Key {
        match self.phase {
            Phase::Ready => {
                self.phase = Phase::Pressed(now);
                Key::Pass
            }
            // The previous press hasn't been read yet, or the swap is under way:
            // letting this one through would paste the same image again.
            Phase::Pressed(_) | Phase::Swapping => {
                self.held += 1;
                Key::Defer
            }
            // Retrying a paste the app didn't read: the image is still current.
            Phase::Stalled(Stall::NotRead | Stall::ReadElsewhere) => {
                self.phase = Phase::Pressed(now);
                Key::Pass
            }
            // Clipboard holds the previous image (Busy) or nothing (Missing).
            // Hold the press for Busy so it lands on the right image once placed.
            Phase::Stalled(Stall::Busy) => {
                self.held += 1;
                Key::Defer
            }
            Phase::Stalled(Stall::Missing) => Key::Pass,
        }
    }

    /// A paste key has gone out and its read hasn't arrived yet.
    pub fn waiting(&self) -> bool {
        matches!(self.phase, Phase::Pressed(_))
    }

    /// An app asked for the current image's data.
    pub fn read(&mut self) -> Then {
        match self.phase {
            Phase::Pressed(_) => {
                self.phase = Phase::Swapping;
                self.pos += 1;
                self.tries = 0;
                if self.pos >= self.items.len() { Then::Finished } else { Then::Place }
            }
            Phase::Ready => {
                self.phase = Phase::Stalled(Stall::ReadElsewhere);
                Then::Nothing
            }
            _ => Then::Nothing,
        }
    }

    /// Report how placing `items[pos]` went.
    pub fn placed(&mut self, r: Placed) -> Then {
        match r {
            Placed::Ok => {
                self.tries = 0;
                if self.held > 0 {
                    self.held -= 1;
                    // The replayed key arrives through `key` like any other.
                    self.phase = Phase::Ready;
                    Then::Replay
                } else {
                    self.phase = Phase::Ready;
                    Then::Nothing
                }
            }
            Placed::Busy if self.tries + 1 < PLACE_TRIES => {
                self.tries += 1;
                Then::Retry
            }
            Placed::Busy | Placed::Cleared => {
                self.phase = Phase::Stalled(Stall::Busy);
                Then::Nothing
            }
            Placed::Missing => {
                self.held = 0;
                self.phase = Phase::Stalled(Stall::Missing);
                Then::Nothing
            }
        }
    }

    /// Periodic check for a paste that was never read.
    pub fn tick(&mut self, now: Instant) {
        if let Phase::Pressed(at) = self.phase {
            if now.duration_since(at) >= READ_TIMEOUT {
                // Held presses would only repeat the unread image.
                self.held = 0;
                self.phase = Phase::Stalled(Stall::NotRead);
            }
        }
    }

    /// The user says the current item is done (or should be skipped).
    pub fn next(&mut self) -> Then {
        if self.phase == Phase::Swapping {
            return Then::Nothing;
        }
        self.held = 0;
        self.tries = 0;
        self.pos += 1;
        self.phase = Phase::Swapping;
        if self.pos >= self.items.len() { Then::Finished } else { Then::Place }
    }

    /// The user retries a stalled swap.
    pub fn retry(&mut self) -> Then {
        match self.phase {
            Phase::Stalled(Stall::Busy | Stall::Missing) => {
                self.tries = 0;
                self.phase = Phase::Swapping;
                Then::Place
            }
            _ => Then::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target app that reads the clipboard once per paste press it receives,
    /// and a clipboard owner that swaps items when told to.
    struct Sim {
        q: Queue,
        clipboard: Option<i64>,
        pasted: Vec<i64>,
        /// Keys that reached the target and haven't been processed yet.
        inbox: u32,
        now: Instant,
        replays: u32,
        finished: bool,
    }

    impl Sim {
        fn new(items: &[i64]) -> Self {
            Sim {
                clipboard: items.first().copied(),
                q: Queue::new(items.to_vec()),
                pasted: Vec::new(),
                inbox: 0,
                now: Instant::now(),
                replays: 0,
                finished: false,
            }
        }

        fn press(&mut self) {
            if self.q.key(self.now) == Key::Pass {
                self.inbox += 1;
            }
        }

        fn follow(&mut self, t: Then) {
            match t {
                Then::Place => {
                    self.clipboard = self.q.current();
                    let t = self.q.placed(Placed::Ok);
                    self.follow(t);
                }
                Then::Replay => {
                    self.replays += 1;
                    self.press();
                }
                Then::Finished => {
                    self.finished = true;
                    self.clipboard = None;
                }
                Then::Retry | Then::Nothing => {}
            }
        }

        /// The target processes one pending key: reads and pastes.
        fn target_reads(&mut self) {
            assert!(self.inbox > 0);
            self.inbox -= 1;
            if let Some(c) = self.clipboard {
                self.pasted.push(c);
            }
            let t = self.q.read();
            self.follow(t);
        }
    }

    #[test]
    fn three_rapid_presses_paste_a_b_c_once_each() {
        let mut s = Sim::new(&[1, 2, 3]);
        // All three arrive before the target has read anything.
        s.press();
        s.press();
        s.press();
        assert_eq!(s.inbox, 1, "only the first press may reach the target before a swap");
        assert_eq!(s.q.held, 2);
        while s.inbox > 0 {
            s.target_reads();
        }
        assert_eq!(s.pasted, [1, 2, 3]);
        assert_eq!(s.replays, 2);
        assert!(s.finished);
    }

    #[test]
    fn spaced_presses_advance_once_each() {
        let mut s = Sim::new(&[10, 20, 30]);
        for _ in 0..3 {
            s.press();
            s.target_reads();
        }
        assert_eq!(s.pasted, [10, 20, 30]);
        assert_eq!(s.replays, 0);
        assert!(s.finished);
    }

    #[test]
    fn a_read_without_a_press_stops_instead_of_guessing() {
        let mut q = Queue::new(vec![1, 2]);
        assert_eq!(q.read(), Then::Nothing);
        assert_eq!(q.phase, Phase::Stalled(Stall::ReadElsewhere));
        assert_eq!(q.current(), Some(1));
        assert_eq!(q.next(), Then::Place);
        assert_eq!(q.current(), Some(2));
    }

    #[test]
    fn an_unread_press_stalls_and_drops_held_keys() {
        let t0 = Instant::now();
        let mut q = Queue::new(vec![1, 2, 3]);
        assert_eq!(q.key(t0), Key::Pass);
        assert_eq!(q.key(t0), Key::Defer);
        q.tick(t0 + READ_TIMEOUT / 2);
        assert!(matches!(q.phase, Phase::Pressed(_)));
        q.tick(t0 + READ_TIMEOUT);
        assert_eq!(q.phase, Phase::Stalled(Stall::NotRead));
        assert_eq!((q.held, q.current()), (0, Some(1)));
        // Pressing again retries the same image.
        assert_eq!(q.key(t0 + READ_TIMEOUT * 2), Key::Pass);
        assert_eq!(q.read(), Then::Place);
        assert_eq!(q.current(), Some(2));
    }

    #[test]
    fn missing_items_are_never_skipped_silently() {
        let mut q = Queue::new(vec![1, 2, 3]);
        q.key(Instant::now());
        assert_eq!(q.read(), Then::Place);
        assert_eq!(q.placed(Placed::Missing), Then::Nothing);
        assert_eq!(q.phase, Phase::Stalled(Stall::Missing));
        assert_eq!(q.progress(), (2, 3));
        // Only an explicit Next moves past it.
        assert_eq!(q.next(), Then::Place);
        assert_eq!(q.placed(Placed::Ok), Then::Nothing);
        assert_eq!((q.current(), q.phase), (Some(3), Phase::Ready));
    }

    #[test]
    fn a_busy_clipboard_retries_then_stalls_holding_presses() {
        let mut q = Queue::new(vec![1, 2]);
        q.key(Instant::now());
        assert_eq!(q.read(), Then::Place);
        for _ in 1..PLACE_TRIES {
            assert_eq!(q.placed(Placed::Busy), Then::Retry);
        }
        assert_eq!(q.placed(Placed::Busy), Then::Nothing);
        assert_eq!(q.phase, Phase::Stalled(Stall::Busy));
        // A press now would paste item 1 again; hold it for item 2.
        assert_eq!(q.key(Instant::now()), Key::Defer);
        assert_eq!(q.retry(), Then::Place);
        assert_eq!(q.placed(Placed::Ok), Then::Replay);
        assert_eq!(q.current(), Some(2));
    }

    #[test]
    fn manual_next_finishes_the_queue() {
        let mut q = Queue::new(vec![1, 2]);
        assert_eq!(q.progress(), (1, 2));
        assert_eq!(q.next(), Then::Place);
        q.placed(Placed::Ok);
        assert_eq!(q.progress(), (2, 2));
        assert_eq!(q.next(), Then::Finished);
    }
}
