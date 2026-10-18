//! `import`: calls `silvermere-import` (T-008; patches T-009). The flags and the body
//! are in [`crate::cmd_import`]. A run that writes takes the single-writer guard (T-073).

use std::process::ExitCode;

use crate::cli::ImportArgs;
use crate::config::Config;

pub fn run(cfg: &Config, args: &ImportArgs) -> anyhow::Result<ExitCode> {
    if args.dry_run {
        return crate::cmd_import::run(cfg, args);
    }
    super::with_guard(cfg, "import", || crate::cmd_import::run(cfg, args))
}
