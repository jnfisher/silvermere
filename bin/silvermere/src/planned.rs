//! A planned shutdown (`POST /v1/shutdown`, `silvermere sysop shutdown`): the players in
//! the game are warned as the time comes closer, then [`PlannedShutdown::due`] fires and
//! `serve` stops as it does on SIGTERM, every character saved.
//!
//! The warnings are broadcasts, as `SYSOP SPEAK` sends them: at the time the plan is made,
//! then at each of [`WARN_AT`] still ahead. Scheduling again replaces the plan; cancelling
//! tells the players it is off.
//!
//! A planned restart (`POST /v1/restart`) counts down the same way; when it is due `serve`
//! asks [`PlannedShutdown::restart_due`] and, after the stop, starts again
//! ([`crate::restart`]). It is refused up front when a restart would not come back.

use std::sync::Mutex;
use std::sync::mpsc::Sender;
use std::time::Duration;

use silvermere_api::{Planned, ShutdownPlan};
use silvermere_engine::{AdminRequest, EngineInput};
use tokio::sync::watch;
use tokio::time::Instant;

/// Seconds before the stop at which the players are warned (besides when it is planned).
pub const WARN_AT: &[u64] = &[3600, 1800, 900, 600, 300, 120, 60, 30, 10];

/// The plan and the countdown that carries it out.
pub struct PlannedShutdown {
    engine: Mutex<Sender<EngineInput>>,
    due: watch::Sender<bool>,
    plan: Mutex<Option<(Planned, tokio::task::AbortHandle)>>,
    runtime: tokio::runtime::Handle,
    /// Asked before a restart is planned; `Err` refuses it ([`crate::restart::check`]).
    restart_check: fn() -> Result<(), String>,
}

impl PlannedShutdown {
    /// No plan yet. Must be made inside the server's runtime: the countdown runs on it.
    pub fn new(engine: Sender<EngineInput>) -> Self {
        PlannedShutdown {
            engine: Mutex::new(engine),
            due: watch::channel(false).0,
            plan: Mutex::new(None),
            runtime: tokio::runtime::Handle::current(),
            restart_check: crate::restart::check,
        }
    }

    /// [`PlannedShutdown::new`] with another restart check (tests).
    pub fn with_restart_check(mut self, check: fn() -> Result<(), String>) -> Self {
        self.restart_check = check;
        self
    }

    /// A restart that came due but would not come back: the plan is dropped, the server keeps
    /// running and the players are told.
    pub fn call_off_restart(&self, why: &str) {
        tracing::error!(%why, "planned restart called off; the server keeps running");
        self.lock().take();
        self.due.send_replace(false);
        let engine = self
            .engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Self::say(&engine, RESTART_CANCELLED.into());
    }

    /// Whether the plan that came due is a restart.
    pub fn restart_due(&self) -> bool {
        *self.due.borrow() && self.lock().as_ref().is_some_and(|(p, _)| p.restart)
    }

    /// Becomes `true` when a plan's time has come.
    pub fn due(&self) -> watch::Receiver<bool> {
        self.due.subscribe()
    }

    /// The plan in force, if any, `in_secs` counted from now.
    pub fn current(&self) -> Option<Planned> {
        self.lock().as_ref().map(|(p, _)| {
            let left = p.at.parse::<jiff::Timestamp>().ok().map_or(0, |at| {
                (at.as_second() - jiff::Timestamp::now().as_second()).max(0)
            });
            Planned {
                in_secs: left as u64,
                ..p.clone()
            }
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(Planned, tokio::task::AbortHandle)>> {
        self.plan.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn say(engine: &Sender<EngineInput>, text: String) {
        // nobody waits for the count of players told
        let (reply, _) = std::sync::mpsc::channel();
        let _ = engine.send(EngineInput::Admin(AdminRequest::Broadcast { text, reply }));
    }
}

/// `in 1 hour 30 minutes`, `in 5 minutes`, `in 10 seconds`.
pub fn how_long(secs: u64) -> String {
    let unit = |n: u64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(unit(h, "hour"));
    }
    if m > 0 {
        parts.push(unit(m, "minute"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(unit(s, "second"));
    }
    parts.join(" ")
}

/// The warning `secs` before the stop.
pub fn warning(secs: u64, message: &str) -> String {
    warn(secs, message, false)
}

/// The warning `secs` before the stop or the restart.
fn warn(secs: u64, message: &str, restart: bool) -> String {
    let when = how_long(secs);
    let what = if restart { "restart" } else { "shut down" };
    if message.is_empty() {
        format!("The server will {what} in {when}.")
    } else {
        format!("The server will {what} in {when}: {message}")
    }
}

/// The broadcast when a plan is cancelled.
pub const CANCELLED: &str = "The planned shutdown is cancelled.";
/// The broadcast when a planned restart is cancelled.
pub const RESTART_CANCELLED: &str = "The planned restart is cancelled.";

impl PlannedShutdown {
    fn plan(&self, after: Duration, message: String, restart: bool) -> Result<Planned, String> {
        let at = jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::try_from(after).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let planned = Planned {
            at: at.to_string(),
            in_secs: after.as_secs(),
            restart,
        };
        let engine = self
            .engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let due = self.due.clone();
        let total = after.as_secs();
        let end = Instant::now() + after;
        let task = self.runtime.spawn(async move {
            if total > 0 {
                Self::say(&engine, warn(total, &message, restart));
            }
            for &secs in WARN_AT.iter().filter(|&&s| s < total) {
                tokio::time::sleep_until(end - Duration::from_secs(secs)).await;
                Self::say(&engine, warn(secs, &message, restart));
            }
            tokio::time::sleep_until(end).await;
            tracing::info!(restart, "planned shutdown: the time has come");
            due.send_replace(true);
        });
        tracing::info!(at = %planned.at, in_secs = planned.in_secs, restart, "planned shutdown scheduled");
        if let Some((_, old)) = self.lock().replace((planned.clone(), task.abort_handle())) {
            old.abort();
        }
        Ok(planned)
    }
}

impl ShutdownPlan for PlannedShutdown {
    fn schedule(&self, after: Duration, message: String) -> Result<Planned, String> {
        self.plan(after, message, false)
    }

    fn schedule_restart(&self, after: Duration, message: String) -> Result<Planned, String> {
        (self.restart_check)()?;
        self.plan(after, message, true)
    }

    fn current(&self) -> Option<Planned> {
        PlannedShutdown::current(self)
    }

    fn cancel(&self) -> bool {
        let Some((planned, task)) = self.lock().take() else {
            return false;
        };
        task.abort();
        tracing::info!(at = %planned.at, restart = planned.restart, "planned shutdown cancelled");
        if !*self.due.borrow() {
            let engine = self
                .engine
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let said = if planned.restart {
                RESTART_CANCELLED
            } else {
                CANCELLED
            };
            Self::say(&engine, said.into());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_long_reads_like_a_person() {
        assert_eq!(how_long(0), "0 seconds");
        assert_eq!(how_long(1), "1 second");
        assert_eq!(how_long(10), "10 seconds");
        assert_eq!(how_long(60), "1 minute");
        assert_eq!(how_long(90), "1 minute 30 seconds");
        assert_eq!(how_long(300), "5 minutes");
        assert_eq!(how_long(3600), "1 hour");
        assert_eq!(how_long(5400), "1 hour 30 minutes");
        assert_eq!(
            warning(300, "upgrading to 0.2"),
            "The server will shut down in 5 minutes: upgrading to 0.2"
        );
        assert_eq!(warning(60, ""), "The server will shut down in 1 minute.");
    }

    /// The broadcasts a countdown sends, with the time left at each.
    async fn told(
        rx: &std::sync::mpsc::Receiver<EngineInput>,
        until: &mut watch::Receiver<bool>,
    ) -> Vec<String> {
        let mut said = Vec::new();
        loop {
            while let Ok(input) = rx.try_recv() {
                if let EngineInput::Admin(AdminRequest::Broadcast { text, .. }) = input {
                    said.push(text);
                }
            }
            if *until.borrow() {
                return said;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_countdown_warns_then_comes_due() {
        let (tx, rx) = std::sync::mpsc::channel();
        let plan = PlannedShutdown::new(tx);
        let mut due = plan.due();
        let started = Instant::now();
        let planned = plan
            .schedule(Duration::from_secs(150), "upgrade".into())
            .unwrap();
        assert_eq!(planned.in_secs, 150);
        assert_eq!(plan.current(), Some(planned));
        let said = told(&rx, &mut due).await;
        assert!(started.elapsed() >= Duration::from_secs(150));
        assert_eq!(
            said,
            [
                "The server will shut down in 2 minutes 30 seconds: upgrade",
                "The server will shut down in 2 minutes: upgrade",
                "The server will shut down in 1 minute: upgrade",
                "The server will shut down in 30 seconds: upgrade",
                "The server will shut down in 10 seconds: upgrade",
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_stops_the_countdown_and_says_so() {
        let (tx, rx) = std::sync::mpsc::channel();
        let plan = PlannedShutdown::new(tx);
        let due = plan.due();
        plan.schedule(Duration::from_secs(60), String::new())
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(plan.cancel());
        assert!(!plan.cancel(), "nothing left to cancel");
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(!*due.borrow());
        let said: Vec<String> = rx
            .try_iter()
            .filter_map(|i| match i {
                EngineInput::Admin(AdminRequest::Broadcast { text, .. }) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(said, ["The server will shut down in 1 minute.", CANCELLED]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_restart_counts_down_as_a_restart_and_is_refused_when_it_would_not_come_back() {
        let (tx, rx) = std::sync::mpsc::channel();
        let plan = PlannedShutdown::new(tx.clone()).with_restart_check(|| Ok(()));
        let mut due = plan.due();
        let planned = plan
            .schedule_restart(Duration::from_secs(30), "upgrade".into())
            .unwrap();
        assert!(planned.restart);
        assert!(!plan.restart_due(), "not before its time");
        let said = told(&rx, &mut due).await;
        assert_eq!(
            said,
            [
                "The server will restart in 30 seconds: upgrade",
                "The server will restart in 10 seconds: upgrade",
            ]
        );
        assert!(plan.restart_due());
        // a restart that would not come back is called off and the players are told
        plan.call_off_restart("the settings would not load");
        assert!(!*due.borrow() && plan.current().is_none());
        let refused = PlannedShutdown::new(tx).with_restart_check(|| Err("broken".into()));
        assert_eq!(
            refused.schedule_restart(Duration::from_secs(30), String::new()),
            Err("broken".to_string())
        );
        assert!(refused.current().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_plan_replaces_the_old_one() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let plan = PlannedShutdown::new(tx);
        let mut due = plan.due();
        plan.schedule(Duration::from_secs(10), String::new())
            .unwrap();
        plan.schedule(Duration::from_secs(600), String::new())
            .unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert!(!*due.borrow(), "the first plan no longer runs");
        tokio::time::timeout(Duration::from_secs(600), due.wait_for(|d| *d))
            .await
            .expect("due at the second plan's time")
            .unwrap();
    }
}
