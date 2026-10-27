//! Backups (T-108, T-150): the one way a backup is taken, by `silvermere backup` and by the
//! server's daily schedule ([`schedule`]), then pruned to `backup.keep` and handed to the
//! after-backup command ([`hook`]).
//!
//! A backup is a consistent copy of the database while the server runs (`VACUUM INTO` on
//! SQLite to `.db`, `pg_dump` on PostgreSQL to `.dump`), with the asset files of
//! `data/assets/` copied to `<backup>.assets/` beside it. The last scheduled run is kept in
//! `data/backups/last-scheduled-run.json` ([`STATUS_FILE`]), which `backup --list` and the
//! admin API read.

pub mod hook;
pub mod schedule;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use silvermere_db::backup::{self, BackupError};
use silvermere_db::{Db, Dialect};

use crate::config::Config;
use hook::HookOutcome;

/// The last scheduled run, in the backups folder.
pub const STATUS_FILE: &str = "last-scheduled-run.json";

/// The backups folder: `<data dir>/backups`.
pub fn folder(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("backups")
}

/// A backup made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The database copy.
    pub file: PathBuf,
    /// Its size.
    pub bytes: u64,
    /// The asset files copied beside it; `None` when there is no `data/assets/`.
    pub assets: Option<usize>,
}

/// Why no backup was made.
#[derive(Debug)]
pub enum TakeError {
    /// Refused before anything was written (the destination exists, an in-memory database).
    Refused(String),
    /// `pg_dump` is not on `PATH`.
    ToolNotFound(&'static str),
    /// It failed on the way; nothing is left half written.
    Failed(anyhow::Error),
}

impl std::fmt::Display for TakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TakeError::Refused(why) => write!(f, "backup refused: {why}"),
            TakeError::ToolNotFound(tool) => write!(
                f,
                "{tool} not found: install postgresql-client or back up with your own tooling"
            ),
            TakeError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

/// Writes a backup to `dest`: the assets first, the database last, so a database copy in
/// place always has its assets beside it (a failure removes the half written). Blocks; it
/// runs the database part on a runtime of its own.
pub fn take(cfg: &Config, dest: &Path) -> Result<Taken, TakeError> {
    let fail = |e: anyhow::Error| TakeError::Failed(e);
    let dialect = Dialect::from_url(&cfg.database.url).map_err(|e| fail(e.into()))?;
    if dest.exists() {
        return Err(TakeError::Refused(format!(
            "{} already exists",
            dest.display()
        )));
    }
    let assets = cfg.data_dir.join("assets");
    let assets_to = backup::assets_dir(dest);
    let copied = if assets.is_dir() {
        if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| fail(e.into()))?;
        }
        // an `.assets` directory left after its database file was deleted
        if assets_to.is_dir() {
            std::fs::remove_dir_all(&assets_to).map_err(|e| fail(e.into()))?;
        }
        Some(backup::copy_assets(&assets, &assets_to).map_err(|e| {
            fail(anyhow::anyhow!(
                "copying {} to {}: {e}",
                assets.display(),
                assets_to.display()
            ))
        })?)
    } else {
        None
    };
    let made: Result<(), BackupError> = (|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            match dialect {
                Dialect::Sqlite => {
                    if cfg.sqlite_path().is_none() {
                        return Err(BackupError::Refused(
                            "backup needs a database file, not an in-memory SQLite URL".into(),
                        ));
                    }
                    let db = Db::connect(&cfg.database.url).await?;
                    let r = backup::backup_sqlite(&db.pool(), dest).await;
                    db.close().await;
                    r
                }
                Dialect::Postgres => backup::backup_postgres(&cfg.database.url, dest),
            }
        })
    })();
    if made.is_err() && copied.is_some() {
        let _ = std::fs::remove_dir_all(&assets_to);
    }
    match made {
        Ok(()) => {}
        Err(BackupError::ToolNotFound { tool }) => return Err(TakeError::ToolNotFound(tool)),
        Err(BackupError::Refused(why)) => return Err(TakeError::Refused(why)),
        Err(e) => return Err(fail(e.into())),
    }
    let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    Ok(Taken {
        file: dest.to_path_buf(),
        bytes,
        assets: copied,
    })
}

/// What the after-backup command may not see in a variable: the database URL and its
/// password.
pub fn secrets(cfg: &Config) -> Vec<String> {
    let (_, password) = silvermere_db::split_password(&cfg.database.url);
    let mut s = vec![cfg.database.url.clone()];
    s.extend(password);
    s.retain(|v| !v.is_empty() && !v.starts_with("sqlite:"));
    s
}

/// Runs `backup.after` for the backup `file`, logging how it ended; `None` when there is
/// none.
pub fn run_hook(cfg: &Config, file: &Path) -> Option<HookOutcome> {
    let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let dir = file
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let timeout = Duration::from_secs(u64::from(cfg.backup.after_timeout_secs));
    let out = hook::run(&cfg.backup.after, &file, &dir, timeout, &secrets(cfg))?;
    let tail = out.tail.join("\n");
    if out.ok {
        tracing::info!(file = %file.display(), result = %out.summary(), output = %tail, "after-backup command");
    } else {
        tracing::error!(file = %file.display(), result = %out.summary(), output = %tail, "after-backup command failed; the backup is kept");
    }
    Some(out)
}

/// One backup run, as the status file and the admin API show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// When it started (RFC 3339, UTC).
    pub at: String,
    /// Whether the backup was made.
    pub ok: bool,
    /// The database copy, when made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Its size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Why it failed (or why pruning did).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The old backups deleted after it.
    #[serde(default)]
    pub pruned: Vec<PathBuf>,
    /// How the after-backup command ended; `None` when there is none (or no backup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookOutcome>,
}

/// A scheduled backup at `now` into the backups folder: made, pruned to `backup.keep`, then
/// the after-backup command. A failure is logged at error level and returned, never raised.
pub fn scheduled_run(cfg: &Config, now: jiff::Timestamp) -> Run {
    let dir = folder(cfg);
    let mut run = Run {
        at: now.to_string(),
        ok: false,
        file: None,
        bytes: None,
        error: None,
        pruned: Vec::new(),
        hook: None,
    };
    let dest = match Dialect::from_url(&cfg.database.url) {
        Ok(d) => dir.join(backup::backup_name(now, d)),
        Err(e) => {
            run.error = Some(e.to_string());
            tracing::error!(error = %e, "scheduled backup failed");
            return run;
        }
    };
    match take(cfg, &dest) {
        Ok(t) => {
            tracing::info!(file = %t.file.display(), bytes = t.bytes, assets = ?t.assets, "scheduled backup made");
            run.ok = true;
            run.file = Some(t.file);
            run.bytes = Some(t.bytes);
        }
        Err(e) => {
            tracing::error!(error = %e, "scheduled backup failed; the server keeps running");
            run.error = Some(e.to_string());
            return run;
        }
    }
    match backup::prune(&dir, cfg.backup.keep as usize) {
        Ok(gone) => {
            for g in &gone {
                tracing::info!(removed = %g.display(), "old backup removed (backup.keep)");
            }
            run.pruned = gone;
        }
        Err(e) => {
            tracing::error!(error = %e, dir = %dir.display(), "removing old backups failed");
            run.error = Some(format!("removing old backups: {e}"));
        }
    }
    run.hook = run_hook(cfg, &dest);
    run
}

/// Reads the last scheduled run from the backups folder.
pub fn read_status(dir: &Path) -> Option<Run> {
    let text = std::fs::read_to_string(dir.join(STATUS_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the last scheduled run to the backups folder (through a file renamed over it).
pub fn write_status(dir: &Path, run: &Run) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{STATUS_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(run)?)?;
    std::fs::rename(&tmp, dir.join(STATUS_FILE))
}

/// One backup in the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listed {
    /// Its file name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// The files in its `.assets` directory, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<usize>,
}

/// The backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Listed> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<Listed> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            backup::is_backup_name(&name).then(|| {
                let assets = count_files(&backup::assets_dir(&e.path()));
                Listed {
                    bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                    name,
                    assets,
                }
            })
        })
        .collect();
    v.sort_by(|a, b| b.name.cmp(&a.name));
    v
}

fn count_files(dir: &Path) -> Option<usize> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut n = 0;
    for e in entries.filter_map(|e| e.ok()) {
        match e.file_type() {
            Ok(t) if t.is_dir() => n += count_files(&e.path()).unwrap_or(0),
            Ok(_) => n += 1,
            Err(_) => {}
        }
    }
    Some(n)
}

/// The schedule's state the admin API reads: the next run and the last one.
#[derive(Debug, Default)]
pub struct State {
    /// When the next scheduled run is due (RFC 3339, UTC); `None` when off.
    pub next: Option<String>,
    /// The last scheduled run.
    pub last: Option<Run>,
}

/// [`State`], shared between the schedule and the admin API.
#[derive(Debug, Clone)]
pub struct Shared {
    pub cfg: Arc<Config>,
    pub state: Arc<Mutex<State>>,
}

impl Shared {
    /// The state as the backups folder has it (the last run from [`STATUS_FILE`]).
    pub fn load(cfg: &Config) -> Self {
        let last = read_status(&folder(cfg));
        Self {
            cfg: Arc::new(cfg.clone()),
            state: Arc::new(Mutex::new(State { next: None, last })),
        }
    }
}

impl silvermere_api::BackupProbe for Shared {
    fn last_ok(&self) -> Option<bool> {
        self.state.lock().ok()?.last.as_ref().map(|r| r.ok)
    }

    fn status(&self) -> serde_json::Value {
        let b = &self.cfg.backup;
        let (next, last) = self
            .state
            .lock()
            .map(|s| (s.next.clone(), s.last.clone()))
            .unwrap_or_default();
        serde_json::json!({
            "enabled": b.enabled,
            "at": b.at,
            "tz": self.cfg.game.cleanup.tz,
            "keep": b.keep,
            "after": !b.after.trim().is_empty(),
            "next": next,
            "last": last,
            "backups": list(&folder(&self.cfg)),
        })
    }
}
