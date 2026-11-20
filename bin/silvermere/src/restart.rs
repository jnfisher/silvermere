//! Restarting the server from the inside (`silvermere sysop restart`, `/v1/restart`, the web
//! console), so that whatever runs it (systemd, Docker, NSSM, a console window) never sees the
//! process stop.
//!
//! After the planned countdown the server stops as it does on SIGTERM, every character
//! saved and the store released; a backup is taken; then:
//!
//! - on Unix the process replaces itself with the program file it was started from, with
//!   the same arguments and environment ([`relaunch`]). The process id does not change, so
//!   systemd keeps the service `active` and Docker's main process never exits. A program
//!   file replaced before the restart is the one that starts: an upgrade.
//! - elsewhere (Windows, where a running program file cannot be replaced anyway), and should
//!   the replacement fail, `serve` runs again in the same process with the settings read
//!   anew.
//!
//! [`check`] runs before the countdown is accepted and again before the server stops: a
//! settings file the server would refuse, or a program file that does not run, refuses the
//! restart and the server keeps running.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::{Config, invocation};

/// How long the program file may take to print its version.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// The program file this process was started from. On Linux a file replaced since shows as
/// `<path> (deleted)`: the path is where the new one now is.
pub fn program() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let text = exe.to_string_lossy();
    Ok(match text.strip_suffix(" (deleted)") {
        Some(path) => PathBuf::from(path),
        None => exe,
    })
}

/// The settings as a start now would read them: the same command line, the file as it is.
pub fn reload_config() -> anyhow::Result<Config> {
    let (data_dir, ov) = invocation()
        .cloned()
        .unwrap_or((None, crate::config::Overrides::default()));
    Config::load(data_dir.as_deref(), &ov)
}

/// Whether a restart would come back: the settings load and the program file runs.
pub fn check() -> Result<(), String> {
    reload_config().map_err(|e| format!("the settings would not load: {e:#}"))?;
    let exe = program().map_err(|e| format!("the program file cannot be found: {e}"))?;
    let mut child = Command::new(&exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{} does not run: {e}", exe.display()))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!("{} --version failed ({status})", exe.display()));
            }
            Ok(None) if started.elapsed() > CHECK_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("{} --version did not answer", exe.display()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("{}: {e}", exe.display())),
        }
    }
}

/// The backup before the restart: as a scheduled one (pruned to `backup.keep`, the
/// after-backup command run). A failure is logged and the restart goes on.
pub fn backup_first(cfg: &Config) {
    if !cfg.backup.enabled {
        return;
    }
    let run = crate::backups::scheduled_run(cfg, jiff::Timestamp::now());
    if run.ok {
        tracing::info!(file = ?run.file, "backup before the restart made");
    } else {
        tracing::error!(error = ?run.error, "backup before the restart failed; restarting all the same");
    }
    let _ = crate::backups::write_status(&crate::backups::folder(cfg), &run);
}

/// Replaces this process with the program file, same arguments and environment. Returns
/// only when that failed.
#[cfg(unix)]
pub fn relaunch() -> std::io::Error {
    use std::os::unix::process::CommandExt as _;
    let exe = match program() {
        Ok(p) => p,
        Err(e) => return e,
    };
    crate::flush_log();
    Command::new(exe).args(std::env::args_os().skip(1)).exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_file_runs_and_names_itself() {
        let exe = program().unwrap();
        assert!(exe.exists(), "{}", exe.display());
    }
}
