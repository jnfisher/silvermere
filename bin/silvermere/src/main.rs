//! The server binary: `serve`, `import`, `migrate`, `account`, `sysop` subcommands.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match silvermere::run(std::env::args_os()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}
