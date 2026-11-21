//! Something Rift started and whose echo it expects shortly: a raise's completion, an app's
//! activation. Until the echo arrives, or `ECHO_WINDOW` passes, matching input is read as Rift's
//! own doing; after that the record is as good as gone. One type for the windows that used to be
//! four separate `Instant` comparisons, so the expiry is checked wherever the record is read.

use std::time::{Duration, Instant};

/// How long Rift expects an echo of its own action.
pub const ECHO_WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
pub struct Expecting<T> {
    value: T,
    since: Instant,
}

/// What an expectation turned out to be when its echo arrived.
pub enum Echo<T> {
    /// Within the window: the echo is of Rift's action.
    Ours(T),
    /// Too late: whatever arrived is the user's, or something else's.
    Late(T),
}

impl<T> Expecting<T> {
    pub fn now(value: T) -> Self { Self::since(value, Instant::now()) }

    pub fn since(value: T, since: Instant) -> Self { Self { value, since } }

    /// The expectation if it is still within the window at `now`.
    pub fn live(&self, now: Instant) -> Option<&T> {
        (now.saturating_duration_since(self.since) < ECHO_WINDOW).then_some(&self.value)
    }

    /// What is expected, whatever the clock says: for bookkeeping that follows identities
    /// (a retired or rekeyed window), never for deciding whether an echo is Rift's.
    pub fn peek(&self) -> &T { &self.value }

    /// The same expectation, about something else (a window that changed identity): the clock
    /// keeps running from when it started.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Expecting<U> {
        Expecting {
            value: f(self.value),
            since: self.since,
        }
    }

    /// Drops the expectation without an echo (superseded, or its owner is going away); the value
    /// comes back for cleanup only.
    pub fn forget(self) -> T { self.value }

    /// Ends the expectation on its echo, arriving at `now`.
    pub fn settle(self, now: Instant) -> Echo<T> {
        if now.saturating_duration_since(self.since) < ECHO_WINDOW {
            Echo::Ours(self.value)
        } else {
            Echo::Late(self.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expectation_lapses_after_the_window() {
        let start = Instant::now();
        let fresh = Expecting::since(7, start);
        assert_eq!(fresh.live(start + ECHO_WINDOW / 2), Some(&7));
        assert!(matches!(fresh.settle(start + ECHO_WINDOW / 2), Echo::Ours(7)));
        assert_eq!(fresh.live(start + ECHO_WINDOW), None);
        assert!(matches!(fresh.settle(start + ECHO_WINDOW), Echo::Late(7)));
    }
}
