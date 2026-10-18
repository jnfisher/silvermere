//! `restore FILE [--force]`: puts a backup back. SQLite is a checked file swap with the
//! server stopped (the single-writer guard refuses while one runs, T-073); PostgreSQL goes
//! through `pg_restore` and needs `--force`. The file's kind is read from its first bytes,
//! whatever its name (`.db`, `.dump`); `<FILE>.assets/`, when present, replaces
//! `data/assets/` (T-108).

use std::process::ExitCode;

use silvermere_db::Dialect;
use silvermere_db::backup::{self, BackupError};

use crate::cli::RestoreArgs;
use crate::config::Config;

pub fn run(cfg: &Config, args: &RestoreArgs) -> anyhow::Result<ExitCode> {
    let dialect = Dialect::from_url(&cfg.database.url)?;
    let outcome = super::with_guard(cfg, "restore", || Ok(restore(cfg, args, dialect)))?;
    match outcome {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(BackupError::Refused(why)) => {
            eprintln!("restore refused: {why}");
            Ok(ExitCode::from(1))
        }
        Err(BackupError::ToolNotFound { tool }) => {
            eprintln!(
                "{tool} not found: install postgresql-client or restore with your own tooling"
            );
            Ok(ExitCode::from(super::EXIT_UNIMPLEMENTED))
        }
        Err(e) => Err(e.into()),
    }
}

/// Refuses a backup made for the other backend, or one that is neither kind.
fn check_format(file: &std::path::Path, dialect: Dialect) -> Result<(), BackupError> {
    let what = |d: Dialect| match d {
        Dialect::Sqlite => "a SQLite database file",
        Dialect::Postgres => "a PostgreSQL pg_dump archive",
    };
    match backup::format_of(file) {
        Some(f) if f.dialect() == dialect => Ok(()),
        Some(f) => Err(BackupError::Refused(format!(
            "{} is {}, but the database is {}: restore it into a {} database",
            file.display(),
            what(f.dialect()),
            dialect.name(),
            f.dialect().name()
        ))),
        None => Err(BackupError::Refused(format!(
            "{} is not a SQLite database or a pg_dump archive (expected {})",
            file.display(),
            what(dialect)
        ))),
    }
}

/// The database and its assets, as a pair (T-108): the assets are staged before the
/// database is touched and swapped in right after it. When the swap fails, a SQLite restore
/// is undone; a PostgreSQL one cannot be, and the message says what is where.
fn restore(cfg: &Config, args: &RestoreArgs, dialect: Dialect) -> Result<(), BackupError> {
    check_format(&args.file, dialect)?;
    let at = jiff::Timestamp::now();
    let live_assets = cfg.data_dir.join("assets");
    let staged = backup::stage_assets(&args.file, &live_assets)?;
    let database = match dialect {
        Dialect::Sqlite => restore_sqlite(cfg, args, at),
        Dialect::Postgres => restore_postgres(cfg, args).map(|()| None),
    };
    let moved = match database {
        Ok(moved) => moved,
        Err(e) => {
            if let Some(s) = staged {
                s.discard();
            }
            return Err(e);
        }
    };
    let Some(staged) = staged else {
        eprintln!(
            "{} has no .assets directory beside it: {} left as it is (run `silvermere import` to write it)",
            args.file.display(),
            live_assets.display()
        );
        return Ok(());
    };
    let files = staged.files;
    match staged.swap_in(&live_assets, at) {
        Ok(old) => {
            if let Some(old) = old {
                println!("previous assets kept as {}", old.display());
            }
            println!("restored {files} asset files to {}", live_assets.display());
            Ok(())
        }
        Err(e) => match (dialect, cfg.sqlite_path()) {
            (Dialect::Sqlite, Some(live)) => {
                let failed = backup::undo_restore_sqlite(&live, moved.as_deref(), at)?;
                Err(BackupError::Refused(format!(
                    "the assets could not be put in place ({e}), so the restore was undone: \
                     {} is as it was, the restored copy is {}",
                    live.display(),
                    failed.display()
                )))
            }
            _ => Err(BackupError::Refused(format!(
                "the database was restored, but its assets could not be put in place ({e}): \
                 {} is unchanged; copy {} there by hand",
                live_assets.display(),
                backup::assets_dir(&args.file).display()
            ))),
        },
    }
}

/// Puts the SQLite file back; returns where the previous database went.
fn restore_sqlite(
    cfg: &Config,
    args: &RestoreArgs,
    at: jiff::Timestamp,
) -> Result<Option<std::path::PathBuf>, BackupError> {
    let live = cfg.sqlite_path().ok_or_else(|| {
        BackupError::Refused("restore needs a database file, not an in-memory SQLite URL".into())
    })?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(backup::check_restorable(&args.file, &live))?;
    let moved = backup::restore_sqlite(&args.file, &live, at)?;
    if let Some(old) = &moved {
        println!("previous database kept as {}", old.display());
    }
    println!("restored {} to {}", args.file.display(), live.display());
    Ok(moved)
}

fn restore_postgres(cfg: &Config, args: &RestoreArgs) -> Result<(), BackupError> {
    if !args.force {
        return Err(BackupError::Refused(
            "restoring into PostgreSQL replaces objects in the target database; pass --force"
                .into(),
        ));
    }
    backup::restore_postgres(&cfg.database.url, &args.file)?;
    println!("restored {}", args.file.display());
    Ok(())
}
