//! The daily backup, in the server (T-150): at `backup.at` local time (in `game.cleanup.tz`,
//! the daily cleanup's zone), a backup into `data/backups/`, pruned to `backup.keep`, then
//! the after-backup command. It runs on a thread of its own, so the game and the admin API
//! keep answering; on SQLite the copy is taken in one read transaction while the server
//! keeps writing, on PostgreSQL by `pg_dump`. A failure is logged at error level and kept
//! as the last run's result; the server keeps running and the next day tries again. A
//! stopped server takes no backups.
//!
//! In the server rather than a systemd timer: it works the same on every platform (Linux,
//! Docker, macOS, a double-clicked Windows console) with nothing to install.

use std::time::Duration;

use tokio::sync::watch;

use super::{Run, Shared, folder, scheduled_run, write_status};

/// The wall clock the schedule reads; the tests give it one that follows tokio's paused
/// time.
pub trait WallClock: Send + Sync + 'static {
    fn now(&self) -> jiff::Timestamp;
}

/// The system clock.
pub struct SystemWall;

impl WallClock for SystemWall {
    fn now(&self) -> jiff::Timestamp {
        jiff::Timestamp::now()
    }
}

/// Runs the schedule until `stop` turns true. Nothing when `backup.enabled` is false.
pub async fn run(shared: Shared, clock: impl WallClock, mut stop: watch::Receiver<bool>) {
    let cfg = shared.cfg.clone();
    if !cfg.backup.enabled {
        tracing::info!("scheduled backups are off (backup.enabled = false)");
        return;
    }
    let at = silvermere_types::config::hour_minute(&cfg.backup.at).unwrap_or((4, 30));
    // `game.cleanup.tz` is checked when the config loads; UTC if it somehow is not known
    let tz = jiff::tz::TimeZone::get(&cfg.game.cleanup.tz).unwrap_or(jiff::tz::TimeZone::UTC);
    // the time the last run was due: the next one is after it, even when the clock is
    // behind the timer
    let mut last_due = i64::MIN;
    loop {
        let now = clock.now().as_second().max(last_due);
        let delay = silvermere_engine::cleanup::seconds_until(now, at, &tz);
        let due = now.saturating_add(i64::try_from(delay).unwrap_or(86_400));
        let next = jiff::Timestamp::from_second(due).ok();
        if let Ok(mut s) = shared.state.lock() {
            s.next = next.map(|t| t.to_string());
        }
        tracing::info!(next = ?next, at = %cfg.backup.at, tz = %cfg.game.cleanup.tz, "next scheduled backup");
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(delay)) => {}
            _ = stop.changed() => return,
        }
        if *stop.borrow() {
            return;
        }
        last_due = due;
        let started = clock.now();
        let job_cfg = cfg.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Run>();
        // a thread of its own: the backup and the command block, and the database part
        // runs on a runtime of its own
        let spawned = std::thread::Builder::new()
            .name("backup".into())
            .spawn(move || {
                let _ = tx.send(scheduled_run(&job_cfg, started));
            });
        let run = match spawned {
            Ok(_) => rx
                .await
                .unwrap_or_else(|_| failed(started, "the backup thread stopped")),
            Err(e) => failed(started, &format!("starting the backup thread: {e}")),
        };
        if let Err(e) = write_status(&folder(&cfg), &run) {
            tracing::error!(error = %e, "writing the backup status failed");
        }
        if let Ok(mut s) = shared.state.lock() {
            s.last = Some(run);
        }
    }
}

fn failed(at: jiff::Timestamp, why: &str) -> Run {
    Run {
        at: at.to_string(),
        ok: false,
        file: None,
        bytes: None,
        error: Some(why.to_owned()),
        pruned: Vec::new(),
        hook: None,
    }
}
