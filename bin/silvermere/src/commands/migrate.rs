//! `migrate`: applies the `silvermere-db` migrations to the configured database, holding
//! the single-writer guard (T-073).

use std::process::ExitCode;

use anyhow::Context;
use silvermere_db::Db;

use crate::config::Config;

pub fn run(cfg: &Config) -> anyhow::Result<ExitCode> {
    super::with_guard(cfg, "migrate", || migrate(cfg))
}

fn migrate(cfg: &Config) -> anyhow::Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        super::ensure_sqlite_dir(cfg)?;
        let db = Db::connect_and_migrate(&cfg.database.url)
            .await
            .with_context(|| format!("migrating {}", cfg.database_shown()))?;
        db.close().await;
        eprintln!("migrations applied to {}", cfg.database_shown());
        Ok(ExitCode::SUCCESS)
    })
}
