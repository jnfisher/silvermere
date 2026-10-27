//! `silvermere` with no subcommand (T-109): what a double-click in Explorer runs. Builds the
//! stock world into an empty database (T-107), asks once for a sysop account when there is
//! none, then serves as `serve` does, with a plain line saying how to connect. A fatal error
//! waits for Enter when the console is interactive ([`finish`]), so a double-clicked failure
//! stays on screen long enough to read.

use std::io::{BufRead, Write};
use std::path::Path;
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

/// `silvermere setup`: [`prepare`] on the console, without serving (T-116). The backup
/// question is asked on a terminal without `--yes` (T-150), its answer written to
/// `settings`.
pub fn setup(cfg: &Config, yes: bool, settings: &Path) -> anyhow::Result<ExitCode> {
    use std::io::IsTerminal as _;
    let hide = std::io::stdin().is_terminal();
    let made = prepare(
        cfg,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        hide,
        (hide && !yes).then_some(settings),
    )?;
    crate::stock::ensure_text_assets(&cfg.data_dir);
    println!("Ready. Start it with: silvermere serve");
    println!("{}", next_steps(made.as_deref(), &cfg.server.telnet_bind));
    Ok(ExitCode::SUCCESS)
}

/// The closing lines of `setup` and `service install`: the sysop account just made (if any),
/// where to connect (`telnet_bind` is `server.telnet_bind`), and where the sysop commands are.
pub fn next_steps(sysop: Option<&str>, telnet_bind: &str) -> String {
    let (host, port) = telnet_bind.rsplit_once(':').unwrap_or((telnet_bind, ""));
    let host = match host.trim_matches(['[', ']']) {
        "" | "0.0.0.0" | "::" => "localhost",
        h => h,
    };
    let who = match sysop {
        Some(name) => format!("Log in as {name} (the sysop account just made). "),
        None => String::new(),
    };
    let at = if port.is_empty() {
        host.to_owned()
    } else {
        format!("{host} port {port}")
    };
    format!(
        "{who}Connect to {at}. Type SYSOP in the game for the sysop commands; \
         see docs/sysop.md."
    )
}

/// The whole first start: [`prepare`] on the console (the backup answer written to
/// `settings`), then `serve`.
pub fn run(cfg: &Config, settings: &Path) -> anyhow::Result<ExitCode> {
    use std::io::IsTerminal as _;
    let hide = std::io::stdin().is_terminal();
    prepare(
        cfg,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
        hide,
        hide.then_some(settings),
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
/// the password on the console without echo instead of from `input`. When the world was just
/// built and `backup_to` is given (a person answers), the backup question follows
/// ([`ask_backup`], T-150), its answer written there; otherwise the backup defaults stand and
/// a line says what they are.
pub fn prepare(
    cfg: &Config,
    input: &mut impl BufRead,
    out: &mut impl Write,
    hide: bool,
    backup_to: Option<&Path>,
) -> anyhow::Result<Option<String>> {
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
    let made = if sysop {
        None
    } else {
        create_sysop(cfg, input, out, hide)?
    };
    if build {
        // the release archives' silvermere.toml lists every key, `backup.*` too: the answer
        // replaces those values in place
        match backup_to {
            Some(target) => {
                let answer = ask_backup(cfg, input, out)?;
                answer.write(target, &cfg.data_dir)?;
                writeln!(out, "{}", answer.wrote(target))?;
            }
            None => writeln!(out, "{}", BackupAnswer::of(cfg).defaults_line())?,
        }
    }
    Ok(made)
}

/// The first start's answer to the backup question (T-150).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupAnswer {
    pub enabled: bool,
    pub at: String,
    pub keep: u32,
    pub after: String,
    /// `game.cleanup.tz`, the zone `at` is read in (not asked).
    pub tz: String,
}

impl BackupAnswer {
    /// The settings in force.
    pub fn of(cfg: &Config) -> Self {
        Self {
            enabled: cfg.backup.enabled,
            at: cfg.backup.at.clone(),
            keep: cfg.backup.keep,
            after: cfg.backup.after.clone(),
            tz: cfg.game.cleanup.tz.clone(),
        }
    }

    /// The `backup.*` settings it writes.
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
        let mut v = vec![("backup.enabled", self.enabled.to_string())];
        if self.enabled {
            v.push(("backup.at", q(&self.at)));
            v.push(("backup.keep", self.keep.to_string()));
            if !self.after.is_empty() {
                v.push(("backup.after", q(&self.after)));
            }
        }
        v
    }

    /// Writes [`Self::pairs`] into the settings file at `target`.
    pub fn write(&self, target: &Path, data_dir: &Path) -> anyhow::Result<()> {
        let pairs = self.pairs();
        let pairs: Vec<(&str, String)> = pairs.iter().map(|(k, v)| (*k, v.clone())).collect();
        super::config::write_settings(target, data_dir, &pairs)
    }

    fn schedule(&self) -> String {
        format!(
            "every day at {} ({} time), keeping the last {}, in data/backups",
            self.at, self.tz, self.keep
        )
    }

    /// The line after the answer is written.
    pub fn wrote(&self, target: &Path) -> String {
        let shown = std::path::absolute(target).unwrap_or_else(|_| target.to_path_buf());
        if self.enabled {
            format!(
                "Backups: {} while the server runs. Saved in {}; change them with \
                 `silvermere config set backup.at 03:15` (or backup.keep, backup.after).",
                self.schedule(),
                shown.display()
            )
        } else {
            format!(
                "Backups: off. Saved in {}; turn them on with \
                 `silvermere config set backup.enabled true`.",
                shown.display()
            )
        }
    }

    /// The line when nothing was asked.
    pub fn defaults_line(&self) -> String {
        if self.enabled {
            format!(
                "Backups: {} while the server runs (backup.* in docs/config.md).",
                self.schedule()
            )
        } else {
            "Backups: off (backup.enabled = false).".to_owned()
        }
    }
}

#[derive(PartialEq)]
enum Pick {
    Take,
    Off,
    Change,
}

/// Asks whether to back up daily and how, on `input`: Enter (or Y) takes the schedule in
/// force (by default 04:30, keeping 30), N turns it off, C asks for the time and the count.
/// Then, when on, the after-backup command (empty: none). The end of `input` takes what is
/// in force.
pub fn ask_backup(
    cfg: &Config,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> anyhow::Result<BackupAnswer> {
    let mut a = BackupAnswer::of(cfg);
    a.enabled = true;
    writeln!(out)?;
    let first = loop {
        let q = format!(
            "Back up the world every day at {} ({} time) and keep the last {}? \
             [Y/n, or C to change the time and the count]: ",
            a.at, a.tz, a.keep
        );
        match ask(input, out, &q)? {
            None => return Ok(a),
            Some(x)
                if x.is_empty() || x.eq_ignore_ascii_case("y") || x.eq_ignore_ascii_case("yes") =>
            {
                break Pick::Take;
            }
            Some(x) if x.eq_ignore_ascii_case("n") || x.eq_ignore_ascii_case("no") => {
                break Pick::Off;
            }
            Some(x) if x.eq_ignore_ascii_case("c") => break Pick::Change,
            Some(_) => writeln!(out, "Answer Y, N or C.")?,
        }
    };
    if first == Pick::Off {
        a.enabled = false;
        return Ok(a);
    }
    if first == Pick::Change {
        loop {
            let Some(t) = ask(
                input,
                out,
                &format!("Time of day, HH:MM (24-hour) [{}]: ", a.at),
            )?
            else {
                return Ok(a);
            };
            if t.is_empty() {
                break;
            }
            if silvermere_types::config::hour_minute(&t).is_some() {
                a.at = t;
                break;
            }
            writeln!(out, "{t:?} is not a time such as 04:30.")?;
        }
        loop {
            let Some(n) = ask(input, out, &format!("Backups to keep [{}]: ", a.keep))? else {
                return Ok(a);
            };
            if n.is_empty() {
                break;
            }
            match n.parse::<u32>() {
                Ok(k) if (1..=10_000).contains(&k) => {
                    a.keep = k;
                    break;
                }
                _ => writeln!(out, "{n:?} is not a number from 1 to 10000.")?,
            }
        }
    }
    loop {
        let Some(cmd) = ask(
            input,
            out,
            "Also copy each backup somewhere else? Enter a command, such as \
             `rclone copy {file} remote:silvermere/`, or leave it empty \
             (recipes: docs/install.md, Backup and restore): ",
        )?
        else {
            return Ok(a);
        };
        match crate::backups::hook::split(&cmd) {
            Ok(_) => {
                a.after = cmd;
                return Ok(a);
            }
            Err(e) => writeln!(out, "That command cannot be read: {e}.")?,
        }
    }
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
) -> anyhow::Result<Option<String>> {
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
            return Ok(None);
        };
        if name.is_empty() {
            writeln!(out, "{LATER}")?;
            return Ok(None);
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
            return Ok(None);
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
                return Ok(Some(account.username));
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
    fn next_steps_name_the_account_the_host_and_the_sysop_page() {
        let t = next_steps(Some("Ada"), "0.0.0.0:2323");
        assert!(t.contains("Log in as Ada"), "{t}");
        assert!(t.contains("localhost port 2323"), "{t}");
        assert!(
            t.contains("Type SYSOP") && t.contains("docs/sysop.md"),
            "{t}"
        );
        let t = next_steps(None, "10.0.0.5:4000");
        assert!(!t.contains("Log in as"), "{t}");
        assert!(t.contains("10.0.0.5 port 4000"), "{t}");
        let t = next_steps(None, "example.org");
        assert!(t.contains("Connect to example.org. Type"), "{t}");
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

    fn answered(lines: &str) -> (BackupAnswer, String) {
        let mut input = std::io::Cursor::new(lines.as_bytes().to_vec());
        let mut out = Vec::new();
        let a = ask_backup(&Config::default(), &mut input, &mut out).unwrap();
        (a, String::from_utf8(out).unwrap())
    }

    /// T-150: Enter takes the defaults, N turns backups off, C changes the time and the
    /// count, and the command is asked for last; the end of input takes the defaults.
    #[test]
    fn the_backup_question() {
        let (a, out) = answered("\n\n");
        assert!(
            out.contains("Back up the world every day at 04:30 (America/Detroit time) and keep the last 30? [Y/n"),
            "{out}"
        );
        assert!(
            out.contains("Also copy each backup somewhere else?"),
            "{out}"
        );
        assert!(out.contains("docs/install.md, Backup and restore"), "{out}");
        assert_eq!(
            (a.enabled, a.at.as_str(), a.keep, a.after.as_str()),
            (true, "04:30", 30, "")
        );

        let (a, out) = answered("n\n");
        assert!(!a.enabled);
        assert!(!out.contains("Also copy"), "{out}");

        let (a, out) = answered(
            "c\n25:00\n03:15\nzero\n14\nrclone copy {file} 'b2:sm\nrclone copy {file} b2:sm/\n",
        );
        assert!(out.contains("\"25:00\" is not a time"), "{out}");
        assert!(out.contains("\"zero\" is not a number"), "{out}");
        assert!(out.contains("cannot be read"), "{out}");
        assert_eq!(
            (a.enabled, a.at.as_str(), a.keep, a.after.as_str()),
            (true, "03:15", 14, "rclone copy {file} b2:sm/")
        );

        let (a, _) = answered("");
        assert_eq!((a.enabled, a.at.as_str(), a.keep), (true, "04:30", 30));
        let (a, out) = answered("maybe\ny\n");
        assert!(a.enabled && out.contains("Answer Y, N or C."), "{out}");
    }

    /// The answer goes into the settings file next to what is there, and loads back.
    #[test]
    fn the_backup_answer_is_written_to_the_settings_file() {
        let dir = std::env::temp_dir().join(format!("sm-backup-answer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("silvermere.toml");
        std::fs::write(&target, "# mine\n[game]\nmax_top = 20\n").unwrap();
        let (a, _) = answered("c\n03:15\n14\nrclone copy {file} \"b2:my sm/\"\n");
        a.write(&target, &dir).unwrap();
        let text = std::fs::read_to_string(&target).unwrap();
        assert!(text.starts_with("# mine\n[game]\nmax_top = 20\n"), "{text}");
        let cfg = Config::check_file_text(&text).unwrap();
        assert!(cfg.backup.enabled);
        assert_eq!(cfg.backup.at, "03:15");
        assert_eq!(cfg.backup.keep, 14);
        assert_eq!(cfg.backup.after, "rclone copy {file} \"b2:my sm/\"");
        assert_eq!(cfg.game.max_top, 20);
        assert!(
            a.wrote(&target).contains("every day at 03:15"),
            "{}",
            a.wrote(&target)
        );

        let (off, _) = answered("n\n");
        let target = dir.join("off.toml");
        off.write(&target, &dir).unwrap();
        let cfg = Config::check_file_text(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert!(!cfg.backup.enabled);
        assert!(off.wrote(&target).contains("Backups: off"));
    }

    /// The release archives ship the example settings file, which lists `backup.*`: the
    /// answer replaces those values, the comments stay.
    #[test]
    fn the_backup_answer_replaces_the_templates_values() {
        let dir = std::env::temp_dir().join(format!("sm-backup-template-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("silvermere.toml");
        std::fs::write(&target, include_str!("../../../../silvermere.example.toml")).unwrap();
        let (a, _) = answered("c\n02:10\n7\n\n");
        a.write(&target, &dir).unwrap();
        let text = std::fs::read_to_string(&target).unwrap();
        assert!(
            text.contains("at = \"02:10\"   # local time, in game.cleanup.tz"),
            "{text}"
        );
        assert!(text.contains("keep = 7   # backups kept"), "{text}");
        assert_eq!(text.matches("[backup]").count(), 1, "{text}");
        let cfg = Config::check_file_text(&text).unwrap();
        assert_eq!((cfg.backup.at.as_str(), cfg.backup.keep), ("02:10", 7));
        assert_eq!(cfg.game.cleanup.at, "03:00");
    }

    /// Not asked (`--yes`, no terminal): the defaults stand, and a line says what they are.
    #[test]
    fn the_defaults_line() {
        let l = BackupAnswer::of(&Config::default()).defaults_line();
        assert_eq!(
            l,
            "Backups: every day at 04:30 (America/Detroit time), keeping the last 30, in \
             data/backups while the server runs (backup.* in docs/config.md)."
        );
    }
}
