//! The time source every timer reads.
//!
//! Game timers (regeneration, spawns, the swing timer) must be driveable by tests, so
//! nothing reads the system clock directly. The engine takes a
//! [`Clock`]; production passes [`SystemClock`], tests pass the `TestClock` in
//! `silvermere-testkit`, which implements this trait.

use std::time::Instant;

/// Monotonic time for timers plus the wall-clock date and time for timestamps.
pub trait Clock {
    /// Monotonic time, for intervals and timers.
    fn now(&self) -> Instant;

    /// Today's date in the server's time zone (midnight cleanup, daily limits).
    fn today(&self) -> jiff::civil::Date;

    /// The wall-clock time, for timestamps only.
    fn wall(&self) -> jiff::Timestamp;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn today(&self) -> jiff::civil::Date {
        jiff::Zoned::now().date()
    }

    fn wall(&self) -> jiff::Timestamp {
        jiff::Timestamp::now()
    }
}
