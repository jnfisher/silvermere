//! `sysop`: HTTP calls to the admin API (T-018).

use std::process::ExitCode;

use crate::cli::SysopCmd;
use crate::config::Config;

pub fn run(_cfg: &Config, _action: &SysopCmd) -> anyhow::Result<ExitCode> {
    Ok(super::not_implemented("T-018"))
}
