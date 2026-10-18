//! `TestClock`: a hand-driven clock for the engine's `Clock` trait.
//!
//! The engine reads time through `Clock::now()`; tests
//! advance this one explicitly, so nothing depends on wall-clock time.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use silvermere_rules::Clock;

/// The engine's fast tick: one per second, as the original's fast cycle.
pub const FAST_TICK: Duration = Duration::from_secs(1);

/// The wall-clock time a new [`TestClock`] starts at: 2026-01-01T00:00:00Z.
const START: jiff::Timestamp = jiff::Timestamp::constant(1_767_225_600, 0);

/// A clock that only moves when told to. Clones share one timeline.
#[derive(Debug, Clone)]
pub struct TestClock {
    base: Instant,
    wall_base: jiff::Timestamp,
    offset: Arc<Mutex<Duration>>,
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            wall_base: START,
            offset: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    /// The current instant: the starting instant plus everything advanced so far.
    pub fn now(&self) -> Instant {
        self.base + self.elapsed()
    }

    /// The wall-clock time: the fixed start plus everything advanced so far.
    pub fn wall(&self) -> jiff::Timestamp {
        self.wall_base
            .checked_add(self.elapsed())
            .expect("test clock stays in range")
    }

    /// Time advanced since the clock was made.
    pub fn elapsed(&self) -> Duration {
        *self.offset.lock().expect("clock lock")
    }

    pub fn advance(&self, by: Duration) {
        *self.offset.lock().expect("clock lock") += by;
    }

    /// Advances `n` fast ticks ([`FAST_TICK`] each). When the engine's `Ticker` (T-015) is
    /// wired in, this is where it is driven; until then it only moves the clock.
    pub fn advance_ticks(&self, n: u32) {
        self.advance(FAST_TICK * n);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        TestClock::now(self)
    }

    fn today(&self) -> jiff::civil::Date {
        self.wall().to_zoned(jiff::tz::TimeZone::UTC).date()
    }

    fn wall(&self) -> jiff::Timestamp {
        TestClock::wall(self)
    }
}
