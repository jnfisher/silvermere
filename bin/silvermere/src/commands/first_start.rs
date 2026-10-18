//! `silvermere` with no subcommand (T-109): what a double-click in Explorer runs. Builds the
//! stock world into an empty database (T-107), asks once for a sysop account when there is
//! none, then serves as `serve` does, with a plain line saying how to connect. A fatal error
//! waits for Enter when the console is interactive ([`finish`]), so a double-clicked failure
//! stays on screen long enough to read.

use std::io::{BufRead, Write};
use std::process::ExitCode;

use anyhow::Context;
use silvermere_auth::{AuthError, Role, has_role};
use silvermere_db::{Db, WorldImport};

use crate::cli::{ImportArgs, ServeArgs};
use crate::config::Config;

/// The line printed once the server listens; `port` is the bound telnet port.
pub fn running_line(port: u16) -> String {
    format!(
        "Silvermere is running. Connect MegaMud or a telnet client to localhost port {port}. \
         Close this window or press Ctrl-C to stop."
    )
}

/// `silvermere setup`: [`prepare`] on the console, without serving (T-116).
pub fn setup(cfg: &Config) -> anyhow::Result<ExitCode> {
    use std::io::IsTerminal as _;
    let hide = std::io::stdin().is_terminal();
    prepare(
        cfg,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        hide,
    )?;
    println!("Ready. Start it with: silvermere serve");
    Ok(ExitCode::SUCCESS)
}

/// The whole first start: [`prepare`] on the console, then `serve`.
pub fn run(cfg: &Config) -> anyhow::Result<ExitCode> {
    use std::io::IsTerminal as _;
    let hide = std::io::stdin().is_terminal();
    prepare(
        cfg,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        hide,
    )?;
    println!(
        "The world, the accounts and the logs are kept in {}.",
        data_dir(cfg).display()
    );
    super::serve::run_with(cfg, &ServeArgs::default(), true)
}

/// Builds the stock world when the database has no rooms, or finishes it when the last
/// import was cut off and `input` says so (Enter); then asks for a sysop account on `input`
/// when there is none. An empty name or the end of `input` skips the account. `hide` reads
/// the password on the console without echo instead of from `input`.
pub fn prepare(
    cfg: &Config,
    input: &mut impl BufRead,
    out: &mut impl Write,
    hide: bool,
) -> anyhow::Result<()> {
    let (rooms, import) = with_db(cfg, async |db| {
        let rooms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rooms")
            .fetch_one(&db.pool())
            .await?;
        Ok((rooms, silvermere_db::world_import::state(db).await?))
    })?;
    let build = if rooms == 0 {
        writeln!(
            out,
            "First start: building the world in {} (this downloads the stock distribution once, about 6.7 MB, and takes a minute or two)...",
            data_dir(cfg).display()
        )?;
        true
    } else if import == WorldImport::Unfinished {
        writeln!(
            out,
            "The last import did not finish (was the window closed while it ran?), so the \
             world in {} is incomplete.",
            data_dir(cfg).display()
        )?;
        let answer = ask(
            input,
            out,
            "Press Enter to build the stock world again, or type N if you imported \
             your own files (then run that import again): ",
        )?;
        match answer {
            Some(a) if a.is_empty() || a.eq_ignore_ascii_case("y") => true,
            _ => anyhow::bail!("{}", super::UNFINISHED_IMPORT),
        }
    } else {
        false
    };
    if build {
        out.flush()?;
        if super::import::run(cfg, &stock_import())? != ExitCode::SUCCESS {
            anyhow::bail!("building the stock world failed (see above)");
        }
        writeln!(out, "The world is built.")?;
    }
    let sysop = with_db(cfg, async |db| {
        Ok(silvermere_auth::cli::list(db)
            .await?
            .iter()
            .any(|a| has_role(a, Role::Sysop)))
    })?;
    if !sysop {
        create_sysop(cfg, input, out, hide)?;
    }
    Ok(())
}

/// The data directory as an absolute path, for messages.
fn data_dir(cfg: &Config) -> std::path::PathBuf {
    std::path::absolute(&cfg.data_dir).unwrap_or_else(|_| cfg.data_dir.clone())
}

/// `import` with every flag at its default: the stock world, downloaded once (T-120).
fn stock_import() -> ImportArgs {
    use clap::{Args as _, FromArgMatches as _};
    let m = ImportArgs::augment_args(clap::Command::new("import")).get_matches_from(["import"]);
    ImportArgs::from_arg_matches(&m).expect("the import defaults parse")
}

fn create_sysop(
    cfg: &Config,
    input: &mut impl BufRead,
    out: &mut impl Write,
    hide: bool,
) -> anyhow::Result<()> {
    const LATER: &str = "No sysop account made. Make one later with: \
                         silvermere account create NAME --sysop";
    writeln!(
        out,
        "\nThere is no sysop account yet. Choose the name and password you will log in with \
         to run this realm (leave the name empty to skip)."
    )?;
    loop {
        let Some(name) = ask(input, out, "Sysop name: ")? else {
            writeln!(out, "\n{LATER}")?;
            return Ok(());
        };
        if name.is_empty() {
            writeln!(out, "{LATER}")?;
            return Ok(());
        }
        let mut password = |prompt: &str| -> anyhow::Result<Option<String>> {
            if hide {
                out.flush()?;
                Ok(Some(rpassword::prompt_password(prompt)?))
            } else {
                ask(input, out, prompt)
            }
        };
        let (Some(password), Some(again)) =
            (password("Password: ")?, password("Password again: ")?)
        else {
            writeln!(out, "\n{LATER}")?;
            return Ok(());
        };
        if password != again {
            writeln!(out, "The two passwords differ; try again.")?;
            continue;
        }
        let made: Result<_, AuthError> = with_db(cfg, async |db| {
            Ok(
                silvermere_auth::cli::create(db, &name, &password, &[Role::Player, Role::Sysop])
                    .await,
            )
        })?;
        match made {
            Ok(account) => {
                writeln!(out, "Created the sysop account {}.", account.username)?;
                return Ok(());
            }
            Err(e) => writeln!(out, "{e}; try again.")?,
        }
    }
}

/// Prints `prompt` and reads one line without its line ending; `None` at the end of input.
fn ask(
    input: &mut impl BufRead,
    out: &mut impl Write,
    prompt: &str,
) -> anyhow::Result<Option<String>> {
    write!(out, "{prompt}")?;
    out.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).trim().to_string()))
}

/// Runs `f` on the configured database, opened and migrated on a runtime of its own (the
/// import and `serve` build theirs).
fn with_db<T>(cfg: &Config, f: impl AsyncFnOnce(&Db) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        super::ensure_sqlite_dir(cfg)?;
        let db = Db::connect_and_migrate(&cfg.database.url)
            .await
            .with_context(|| format!("opening {}", cfg.database.url))?;
        let r = f(&db).await;
        db.close().await;
        r
    })
}

/// The exit code of a first start, after printing `result`'s error. A failure waits for Enter
/// when `interactive` (a console a person sees), so the window does not close on the message.
pub fn finish(
    result: anyhow::Result<ExitCode>,
    interactive: bool,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> ExitCode {
    let code = match result {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(out, "error: {e:#}");
            ExitCode::from(1)
        }
    };
    if code != ExitCode::SUCCESS && interactive {
        let _ = write!(
            out,
            "\nSilvermere stopped. Press Enter to close this window."
        );
        let _ = out.flush();
        let _ = input.read_line(&mut String::new());
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(result: anyhow::Result<ExitCode>, interactive: bool) -> (ExitCode, String, usize) {
        let mut input = std::io::Cursor::new(b"\nleft over\n".to_vec());
        let mut out = Vec::new();
        let code = finish(result, interactive, &mut input, &mut out);
        (
            code,
            String::from_utf8(out).unwrap(),
            input.position() as usize,
        )
    }

    /// A fatal error waits for Enter only on an interactive console.
    #[test]
    fn a_failure_waits_for_enter_only_when_interactive() {
        let (code, out, read) = finished(Err(anyhow::anyhow!("port taken")), true);
        assert_eq!(code, ExitCode::from(1));
        assert!(out.contains("error: port taken"), "{out}");
        assert!(out.contains("Press Enter to close"), "{out}");
        assert_eq!(read, 1, "reads the one line Enter gives");

        let (code, out, read) = finished(Err(anyhow::anyhow!("port taken")), false);
        assert_eq!(code, ExitCode::from(1));
        assert!(
            out.contains("error: port taken") && !out.contains("Press Enter"),
            "{out}"
        );
        assert_eq!(read, 0);

        let (code, out, read) = finished(Ok(ExitCode::from(3)), true);
        assert_eq!(code, ExitCode::from(3));
        assert!(out.contains("Press Enter"), "{out}");
        assert_eq!(read, 1);
    }

    /// A clean stop closes at once, interactive or not.
    #[test]
    fn success_does_not_wait() {
        for interactive in [true, false] {
            let (code, out, read) = finished(Ok(ExitCode::SUCCESS), interactive);
            assert_eq!(code, ExitCode::SUCCESS);
            assert_eq!((out.as_str(), read), ("", 0));
        }
    }

    #[test]
    fn the_running_line_names_the_port() {
        assert_eq!(
            running_line(2323),
            "Silvermere is running. Connect MegaMud or a telnet client to localhost port 2323. \
             Close this window or press Ctrl-C to stop."
        );
    }

    #[test]
    fn the_stock_import_has_no_source() {
        let i = stock_import();
        assert!(i.from.is_none() && !i.dry_run && !i.live);
    }
}
