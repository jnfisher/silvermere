//! `backup [--to PATH] [--keep N]`: a consistent copy of the database while the server
//! runs (`VACUUM INTO` on SQLite to `.db`, `pg_dump` on PostgreSQL to `.dump`), with the
//! asset files of `data/assets/` copied to `<backup>.assets/` beside it (T-108), then
//! pruned and handed to `backup.after` (T-150). `backup --list` shows the backups and the
//! last scheduled run.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use silvermere_db::Dialect;
use silvermere_db::backup;

use crate::backups::{self, TakeError};
use crate::cli::BackupArgs;
use crate::config::Config;

pub fn run(cfg: &Config, args: &BackupArgs) -> anyhow::Result<ExitCode> {
    if args.list {
        print!("{}", listing(cfg));
        return Ok(ExitCode::SUCCESS);
    }
    run_at(cfg, args, jiff::Timestamp::now())
}

/// [`run`] with the clock given (the file name carries the time, in UTC).
pub fn run_at(cfg: &Config, args: &BackupArgs, now: jiff::Timestamp) -> anyhow::Result<ExitCode> {
    let dir = backups::folder(cfg);
    let dialect = Dialect::from_url(&cfg.database.url)?;
    let dest = args
        .to
        .clone()
        .unwrap_or_else(|| dir.join(backup::backup_name(now, dialect)));
    let taken = match backups::take(cfg, &dest) {
        Ok(t) => t,
        Err(e @ TakeError::ToolNotFound(_)) => {
            eprintln!("{e}");
            return Ok(ExitCode::from(super::EXIT_UNIMPLEMENTED));
        }
        Err(e @ TakeError::Refused(_)) => {
            eprintln!("{e}");
            return Ok(ExitCode::from(1));
        }
        Err(TakeError::Failed(e)) => return Err(e),
    };
    println!("{} ({} bytes)", taken.file.display(), taken.bytes);
    match taken.assets {
        Some(n) => println!("{} ({n} files)", backup::assets_dir(&dest).display()),
        None => eprintln!(
            "no {} to copy: the backup holds the database only",
            cfg.data_dir.join("assets").display()
        ),
    }
    // T-150: into the backups folder, the same count the schedule keeps
    let keep = args
        .keep
        .or_else(|| args.to.is_none().then_some(cfg.backup.keep as usize));
    if let Some(keep) = keep {
        let folder: PathBuf = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        for gone in backup::prune(&folder, keep)? {
            println!("removed {}", gone.display());
        }
    }
    if let Some(out) = backups::run_hook(cfg, &dest) {
        if out.ok {
            println!("after-backup command (backup.after): {}", out.summary());
        } else {
            eprintln!(
                "after-backup command (backup.after) failed: {}; the backup is kept",
                out.summary()
            );
            for line in &out.tail {
                eprintln!("  | {line}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// What `backup --list` prints: the backups, newest first, the schedule and its last run.
pub fn listing(cfg: &Config) -> String {
    use std::fmt::Write as _;
    let dir = backups::folder(cfg);
    let b = &cfg.backup;
    let mut s = String::new();
    let list = backups::list(&dir);
    let _ = writeln!(
        s,
        "Backups in {} ({} kept, backup.keep = {}):",
        dir.display(),
        list.len(),
        b.keep
    );
    if list.is_empty() {
        let _ = writeln!(s, "  none yet");
    }
    for l in &list {
        let assets = l
            .assets
            .map_or_else(String::new, |n| format!(", .assets with {n} files"));
        let _ = writeln!(s, "  {}  {}{assets}", l.name, size(l.bytes));
    }
    if b.enabled {
        let _ = writeln!(
            s,
            "Schedule: daily at {} ({}) while the server runs.",
            b.at, cfg.game.cleanup.tz
        );
    } else {
        let _ = writeln!(s, "Schedule: off (backup.enabled = false).");
    }
    if b.after.trim().is_empty() {
        let _ = writeln!(s, "After each backup: nothing (backup.after is empty).");
    } else {
        let _ = writeln!(s, "After each backup: {}", b.after);
    }
    match backups::read_status(&dir) {
        None => {
            let _ = writeln!(s, "Last scheduled run: none yet.");
        }
        Some(r) => {
            let what = match (&r.file, &r.error) {
                (Some(f), None) => format!("ok, {}", name_of(f)),
                (Some(f), Some(e)) => format!("ok, {} ({e})", name_of(f)),
                (None, e) => format!("FAILED: {}", e.as_deref().unwrap_or("unknown")),
            };
            let _ = writeln!(s, "Last scheduled run: {}, {what}", local(&r.at, cfg));
            if let Some(h) = &r.hook {
                let state = if h.ok { "ok" } else { "FAILED" };
                let _ = writeln!(s, "  after-backup command: {state}, {}", h.summary());
                if !h.ok {
                    for line in &h.tail {
                        let _ = writeln!(s, "  | {line}");
                    }
                }
            }
        }
    }
    s
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `at` (RFC 3339) in the schedule's zone.
fn local(at: &str, cfg: &Config) -> String {
    let tz = jiff::tz::TimeZone::get(&cfg.game.cleanup.tz).unwrap_or(jiff::tz::TimeZone::UTC);
    at.parse::<jiff::Timestamp>().map_or_else(
        |_| at.to_owned(),
        |t| t.to_zoned(tz).strftime("%Y-%m-%d %H:%M %Z").to_string(),
    )
}

fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / MB)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} bytes")
    }
}
