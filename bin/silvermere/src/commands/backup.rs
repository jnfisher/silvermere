//! `backup [--to PATH] [--keep N]`: a consistent copy of the database while the server
//! runs (`VACUUM INTO` on SQLite to `.db`, `pg_dump` on PostgreSQL to `.dump`), with the
//! asset files of `data/assets/` copied to `<backup>.assets/` beside it (T-108).

use std::path::PathBuf;
use std::process::ExitCode;

use silvermere_db::backup::{self, BackupError};
use silvermere_db::{Db, Dialect};

use crate::cli::BackupArgs;
use crate::config::Config;

pub fn run(cfg: &Config, args: &BackupArgs) -> anyhow::Result<ExitCode> {
    run_at(cfg, args, jiff::Timestamp::now())
}

/// [`run`] with the clock given (the file name carries the time, in UTC).
pub fn run_at(cfg: &Config, args: &BackupArgs, now: jiff::Timestamp) -> anyhow::Result<ExitCode> {
    let dir = cfg.data_dir.join("backups");
    let dialect = Dialect::from_url(&cfg.database.url)?;
    let dest = args
        .to
        .clone()
        .unwrap_or_else(|| dir.join(backup::backup_name(now, dialect)));
    if dest.exists() {
        eprintln!("backup refused: {} already exists", dest.display());
        return Ok(ExitCode::from(1));
    }
    // T-108: the assets first, the database last, so a database copy in place always has
    // its assets beside it (a failure removes the half written)
    let assets = cfg.data_dir.join("assets");
    let assets_to = backup::assets_dir(&dest);
    let copied = if assets.is_dir() {
        if let Some(dir) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        // an `.assets` directory left after its database file was deleted
        if assets_to.is_dir() {
            std::fs::remove_dir_all(&assets_to)?;
        }
        Some(backup::copy_assets(&assets, &assets_to).map_err(|e| {
            anyhow::anyhow!(
                "copying {} to {}: {e}",
                assets.display(),
                assets_to.display()
            )
        })?)
    } else {
        None
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let made: Result<(), BackupError> = rt.block_on(async {
        match dialect {
            Dialect::Sqlite => {
                if cfg.sqlite_path().is_none() {
                    return Err(BackupError::Refused(
                        "backup needs a database file, not an in-memory SQLite URL".into(),
                    ));
                }
                let db = Db::connect(&cfg.database.url).await?;
                let r = backup::backup_sqlite(&db.pool(), &dest).await;
                db.close().await;
                r
            }
            Dialect::Postgres => backup::backup_postgres(&cfg.database.url, &dest),
        }
    });
    if made.is_err() && copied.is_some() {
        let _ = std::fs::remove_dir_all(&assets_to);
    }
    match made {
        Ok(()) => {}
        Err(BackupError::ToolNotFound { tool }) => {
            eprintln!(
                "{tool} not found: install postgresql-client or back up with your own tooling"
            );
            return Ok(ExitCode::from(super::EXIT_UNIMPLEMENTED));
        }
        Err(BackupError::Refused(why)) => {
            eprintln!("backup refused: {why}");
            return Ok(ExitCode::from(1));
        }
        Err(e) => return Err(e.into()),
    }
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    println!("{} ({size} bytes)", dest.display());
    match copied {
        Some(n) => println!("{} ({n} files)", assets_to.display()),
        None => eprintln!(
            "no {} to copy: the backup holds the database only",
            assets.display()
        ),
    }
    if let Some(keep) = args.keep {
        let folder: PathBuf = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        for gone in backup::prune(&folder, keep)? {
            println!("removed {}", gone.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}
