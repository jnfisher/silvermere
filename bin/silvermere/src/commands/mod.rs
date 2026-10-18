//! Subcommand bodies. Each file is replaced by the card that owns the crate it calls.

pub mod account;
pub mod admin;
pub mod backup;
pub mod config;
pub mod first_start;
pub mod healthcheck;
pub mod import;
pub mod migrate;
pub mod restore;
pub mod serve;
pub mod service;
pub mod sysop;

use std::process::ExitCode;

/// Exit code for a subcommand whose crate has not merged yet.
pub const EXIT_UNIMPLEMENTED: u8 = 2;
/// Exit code when `serve` finds no world data, or a world an import left half written.
pub const EXIT_NO_WORLD: u8 = 3;

/// What `serve` says of a world whose last import did not finish (T-109).
pub const UNFINISHED_IMPORT: &str = "The last import did not finish, so the world is incomplete. \
     Run the same import again (silvermere import, with the same --from if you gave one): it \
     writes the tables the interrupted run did not.";

/// Prints the stub line and returns the exit code.
pub fn not_implemented(card: &str) -> ExitCode {
    eprintln!("not implemented yet ({card})");
    ExitCode::from(EXIT_UNIMPLEMENTED)
}

/// Creates the directory of a SQLite database file so the pool can create the file.
pub fn ensure_sqlite_dir(cfg: &crate::config::Config) -> anyhow::Result<()> {
    if let Some(dir) = cfg
        .sqlite_path()
        .as_deref()
        .and_then(std::path::Path::parent)
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Runs `f` holding the store's single-writer guard, so `what` refuses while a server
/// holds it (T-073): the server's write-behind would overwrite what `what` writes.
pub fn with_guard<T>(
    cfg: &crate::config::Config,
    what: &str,
    f: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use anyhow::Context as _;
    use silvermere_db::{TryAcquire, WriterGuard};

    // A runtime of its own that outlives `f` (which may build another): a PostgreSQL
    // guard is a connection on it.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let url = cfg.database_shown();
    let guard = match rt
        .block_on(WriterGuard::try_acquire(&cfg.database.url))
        .with_context(|| format!("taking the writer guard of {url}"))?
    {
        TryAcquire::Taken(guard) => guard,
        TryAcquire::Held(by) => anyhow::bail!(
            "a silvermere server is running on {url} ({by}); stop it before `{what}`, or \
             its saves would overwrite what `{what}` writes"
        ),
    };
    let result = f();
    rt.block_on(guard.release());
    result
}
