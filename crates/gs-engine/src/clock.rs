//! The engine's clock: the one place in the engine that reads the time (clippy bans
//! `Instant::now` everywhere else, ARCHITECTURE 8). Only autosave asks it, to know when to
//! save (decision 0016, section 10); nothing a command computes depends on it, so results never
//! depend on when they ran. Tests give the engine a clock they move by hand.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Time since some fixed start.
pub trait Clock: Send {
    fn now(&self) -> Duration;
}

/// The machine's monotonic clock.
pub(crate) struct SystemClock {
    start: Instant,
}

impl SystemClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "the engine's clock module is the one place that reads the clock"
    )]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
}

/// A clock that only moves when told to. Clones share the time.
#[derive(Clone, Default)]
pub struct ManualClock(Arc<Mutex<Duration>>);

impl ManualClock {
    /// Moves the time forward by `by`.
    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
