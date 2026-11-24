//! When a second Ctrl+C closes a terminal pane. Every Ctrl+C still reaches
//! the shell as an interrupt; two in quick succession at an idle prompt also
//! close the pane. Pure: callers pass the clock reading and whether the
//! shell sits idle at its prompt (nothing running in the foreground).

use std::time::{Duration, Instant};

/// How soon the second Ctrl+C must follow the first to close the pane; the
/// hint after the first press lasts this long.
pub const WINDOW: Duration = Duration::from_millis(700);

/// What a Ctrl+C does besides interrupting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Only the interrupt: something runs, the press is a key repeat, or
    /// whether the shell is idle can't be told.
    Interrupt,
    /// The first press at an idle prompt: show the hint; another one within
    /// [`WINDOW`] closes the pane.
    Hint,
    /// The second press within [`WINDOW`], idle at both: close the pane.
    Close,
}

/// The pending first press of a double Ctrl+C.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DoubleInterrupt {
    armed: Option<Instant>,
}

impl DoubleInterrupt {
    /// A Ctrl+C at `now`. `idle`: the shell is at its prompt with nothing in
    /// the foreground (`false` when unknown). `repeat`: the key is held, so
    /// it never counts as a second press and drops a pending first one.
    ///
    /// A first press while something runs never arms: the interrupt stops
    /// it and the shell is idle again by the second press, which must not
    /// close the pane the user only meant to free.
    pub fn press(&mut self, now: Instant, idle: bool, repeat: bool) -> Outcome {
        let armed = self.armed.take();
        if repeat || !idle {
            return Outcome::Interrupt;
        }
        if armed.is_some_and(|at| now.saturating_duration_since(at) <= WINDOW) {
            return Outcome::Close;
        }
        self.armed = Some(now);
        Outcome::Hint
    }

    /// Any other key: the next Ctrl+C starts over. Whether a first press was
    /// pending (its hint should go).
    pub fn reset(&mut self) -> bool {
        self.armed.take().is_some()
    }

    /// When the pending first press came, if one is pending: its hint shows
    /// until [`WINDOW`] after it, then [`DoubleInterrupt::reset`] drops it.
    pub fn armed(&self) -> Option<Instant> {
        self.armed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn two_quick_presses_at_an_idle_prompt_close() {
        let t = Instant::now();
        let mut d = DoubleInterrupt::default();
        assert_eq!(d.press(t, true, false), Outcome::Hint);
        assert_eq!(d.armed(), Some(t));
        assert_eq!(d.press(t + ms(300), true, false), Outcome::Close);
        assert_eq!(d.armed(), None);
        // Closing disarms: a third press starts over.
        assert_eq!(d.press(t + ms(400), true, false), Outcome::Hint);
    }

    #[test]
    fn the_window_edge_still_closes() {
        let t = Instant::now();
        let mut d = DoubleInterrupt::default();
        d.press(t, true, false);
        assert_eq!(d.press(t + WINDOW, true, false), Outcome::Close);
    }

    #[test]
    fn a_slow_second_press_only_hints_again() {
        let t = Instant::now();
        let mut d = DoubleInterrupt::default();
        d.press(t, true, false);
        let later = t + WINDOW + ms(1);
        assert_eq!(d.press(later, true, false), Outcome::Hint);
        assert_eq!(d.armed(), Some(later));
        assert_eq!(d.press(later + ms(200), true, false), Outcome::Close);
    }

    #[test]
    fn interrupting_a_running_process_never_closes() {
        let t = Instant::now();
        let mut d = DoubleInterrupt::default();
        // `sleep 30`: the first press stops it, the shell is idle again by
        // the second, which only arms.
        assert_eq!(d.press(t, false, false), Outcome::Interrupt);
        assert_eq!(d.press(t + ms(150), true, false), Outcome::Hint);
        // Something started between the presses: no close, disarmed.
        let mut d = DoubleInterrupt::default();
        d.press(t, true, false);
        assert_eq!(d.press(t + ms(150), false, false), Outcome::Interrupt);
        assert_eq!(d.press(t + ms(300), true, false), Outcome::Hint);
    }

    #[test]
    fn held_keys_never_count_and_drop_the_first_press() {
        let t = Instant::now();
        let mut d = DoubleInterrupt::default();
        d.press(t, true, false);
        assert_eq!(d.press(t + ms(250), true, true), Outcome::Interrupt);
        assert_eq!(d.press(t + ms(280), true, true), Outcome::Interrupt);
        assert_eq!(d.press(t + ms(400), true, false), Outcome::Hint);
    }

    #[test]
    fn another_key_between_presses_starts_over() {
        let t = Instant::now();
        let mut d = DoubleInterrupt::default();
        assert!(!d.reset());
        d.press(t, true, false);
        assert!(d.reset());
        assert_eq!(d.armed(), None);
        assert_eq!(d.press(t + ms(200), true, false), Outcome::Hint);
    }
}
