//! `web`: the web pages without a server (`silvermere_web`).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use clap::Subcommand;

/// `web` subcommands.
#[derive(Debug, Subcommand)]
pub enum WebCmd {
    /// Write the documentation pages as static files, for a public copy.
    Export {
        /// The folder to write into (made if missing).
        dir: PathBuf,
        /// The path the copy is served under, e.g. /silvermere for a project site.
        #[arg(long, default_value = "")]
        base: String,
    },
}

pub fn run(action: &WebCmd) -> anyhow::Result<ExitCode> {
    match action {
        WebCmd::Export { dir, base } => {
            let n = silvermere_web::export(dir, base, env!("CARGO_PKG_VERSION"))
                .with_context(|| format!("writing the pages into {}", dir.display()))?;
            println!("wrote {n} files into {}", dir.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}
