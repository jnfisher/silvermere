//! Lockout after repeated failed logins.
//!
//! The BBS had no such rule, so the numbers are silvermere's and configurable
//! (`[lockout]`): `max_failures` wrong passwords for one account within `window_secs`
//! lock it for `lock_secs`; while locked even the right password is refused. Separately,
//! one connection gets three tries (`LoginConfig::max_attempts`) before it is closed.
//!
//! The tracker is in memory, shared by every session of the server: a restart forgets
//! failures and lifts locks, which bounds how long a forgotten lock can matter and needs
//! no schema change.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use jiff::{SignedDuration, Timestamp};
use silvermere_types::ids::AccountId;

/// The `[lockout]` settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockoutConfig {
    /// Failures inside the window that lock the account (default 10).
    pub max_failures: u32,
    /// The window, in seconds (default 900).
    pub window_secs: u64,
    /// How long a lock lasts, in seconds (default 900).
    pub lock_secs: u64,
}

impl Default for LockoutConfig {
    fn default() -> Self {
        Self {
            max_failures: 10,
            window_secs: 15 * 60,
            lock_secs: 15 * 60,
        }
    }
}

#[derive(Debug, Default)]
struct Entry {
    failures: VecDeque<Timestamp>,
    locked_until: Option<Timestamp>,
}

/// The per-account failure tracker.
#[derive(Debug, Default)]
pub struct Lockout {
    config: LockoutConfig,
    entries: Mutex<HashMap<AccountId, Entry>>,
}

fn secs(n: u64) -> SignedDuration {
    SignedDuration::from_secs(i64::try_from(n).unwrap_or(i64::MAX))
}

impl Lockout {
    /// A tracker with `config`.
    pub fn new(config: LockoutConfig) -> Self {
        Self {
            config,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The settings.
    pub fn config(&self) -> LockoutConfig {
        self.config
    }

    fn with<T>(&self, id: AccountId, now: Timestamp, f: impl FnOnce(&mut Entry) -> T) -> T {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries.entry(id).or_default();
        // An expired lock is lifted and its failures forgotten.
        if entry.locked_until.is_some_and(|until| until <= now) {
            *entry = Entry::default();
        }
        let window_start = now
            .checked_sub(secs(self.config.window_secs))
            .unwrap_or(now);
        while entry.failures.front().is_some_and(|t| *t <= window_start) {
            entry.failures.pop_front();
        }
        let out = f(entry);
        if entry.failures.is_empty() && entry.locked_until.is_none() {
            entries.remove(&id);
        }
        out
    }

    /// When the lock on `id` ends, if it is locked at `now`.
    pub fn locked_until(&self, id: AccountId, now: Timestamp) -> Option<Timestamp> {
        self.with(id, now, |e| e.locked_until)
    }

    /// Failures of `id` inside the window at `now`.
    pub fn failed_logins(&self, id: AccountId, now: Timestamp) -> u32 {
        self.with(id, now, |e| {
            u32::try_from(e.failures.len()).unwrap_or(u32::MAX)
        })
    }

    /// Records a wrong password for `id` at `now`; returns the lock's end when this
    /// failure locked the account.
    pub fn record_failure(&self, id: AccountId, now: Timestamp) -> Option<Timestamp> {
        let config = self.config;
        self.with(id, now, |e| {
            e.failures.push_back(now);
            if e.locked_until.is_none() && e.failures.len() as u64 >= u64::from(config.max_failures)
            {
                let until = now.checked_add(secs(config.lock_secs)).unwrap_or(now);
                e.locked_until = Some(until);
                Some(until)
            } else {
                None
            }
        })
    }

    /// A successful login: forgets the failures of `id`.
    pub fn record_success(&self, id: AccountId) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.remove(&id);
    }

    /// Whole minutes left on a lock ending at `until`, at least 1.
    pub fn minutes_left(until: Timestamp, now: Timestamp) -> u32 {
        let secs = until.duration_since(now).as_secs().max(0);
        u32::try_from((secs + 59) / 60).unwrap_or(u32::MAX).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + s).unwrap()
    }

    #[test]
    fn old_failures_leave_the_window() {
        let l = Lockout::new(LockoutConfig::default());
        let id = AccountId(1);
        for i in 0..9 {
            assert_eq!(l.record_failure(id, at(i)), None);
        }
        assert_eq!(l.failed_logins(id, at(9)), 9);
        // 901 s later the first nine are gone, so a tenth does not lock.
        assert_eq!(l.record_failure(id, at(910)), None);
        assert_eq!(l.failed_logins(id, at(910)), 1);
    }

    #[test]
    fn minutes_round_up() {
        assert_eq!(Lockout::minutes_left(at(900), at(0)), 15);
        assert_eq!(Lockout::minutes_left(at(61), at(0)), 2);
        assert_eq!(Lockout::minutes_left(at(0), at(0)), 1);
    }
}
