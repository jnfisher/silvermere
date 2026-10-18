//! `silvermere service install` and `service uninstall` (T-116), Linux only: writes the
//! systemd unit docs/install.md describes, the database URL's env file, enables lingering for
//! a user unit, then starts the server and waits for its listening line. A re-run updates the
//! unit and keeps the data; `uninstall` removes the unit and leaves the data. The flags are
//! here; the systemd side, Unix only, is in [`systemd`].

use std::path::Path;
use std::process::ExitCode;

use clap::{Args, Subcommand};

use crate::config::Overrides;

#[cfg(unix)]
pub mod systemd;

/// `service` subcommands.
#[derive(Debug, Subcommand)]
pub enum ServiceCmd {
    /// Write and start the systemd unit (Linux). Run it from the folder that holds data/ and
    /// silvermere.toml: that folder becomes the unit's working directory.
    Install(InstallArgs),
    /// Stop the service and remove its unit; the data and the env file stay.
    Uninstall(ModeArgs),
}

/// `--user` or `--system`.
#[derive(Debug, Default, Args)]
pub struct ModeArgs {
    /// A user unit in ~/.config/systemd/user (the default when not root).
    #[arg(long, conflicts_with = "system")]
    pub user: bool,
    /// A system unit in /etc/systemd/system, run as the `silvermere` user (the default as root).
    #[arg(long)]
    pub system: bool,
}

/// `service install` flags.
#[derive(Debug, Default, Args)]
pub struct InstallArgs {
    #[command(flatten)]
    pub mode: ModeArgs,
    /// Read a database URL (a hosted PostgreSQL) from stdin and write it to the unit's env
    /// file. Never on the command line, where `ps` and the shell history would keep it.
    #[arg(long)]
    pub database_url_from_stdin: bool,
}

/// Runs `service`. `ov` and `data_dir` are the global flags, which the unit passes on.
pub fn run(cmd: &ServiceCmd, data_dir: Option<&Path>, ov: &Overrides) -> anyhow::Result<ExitCode> {
    #[cfg(unix)]
    return systemd::run(cmd, data_dir, ov);
    #[cfg(not(unix))]
    {
        let _ = (cmd, data_dir, ov);
        anyhow::bail!("`silvermere service` writes systemd units and runs on Linux only")
    }
}
