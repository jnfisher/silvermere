//! The systemd side of `silvermere service` (T-116): the unit and env file text, the
//! install and uninstall steps. Unix only (file modes); `run` refuses anything but Linux.

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};

use super::{ModeArgs, ServiceCmd};
use crate::config::{Config, Overrides};

/// The unit's name, for systemctl and journalctl.
const UNIT: &str = "silvermere";
/// The account `--system` runs the server as.
const SYSTEM_USER: &str = "silvermere";
/// How long `install` waits for the listening line.
const WAIT: Duration = Duration::from_secs(60);

/// Which systemd manager the unit belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    User,
    System,
}

/// What the unit runs, all paths absolute.
#[derive(Debug, Clone)]
pub struct Plan {
    pub mode: Mode,
    /// The running binary.
    pub exe: PathBuf,
    /// Where `serve` runs: data/ and silvermere.toml are relative to it.
    pub workdir: PathBuf,
    /// `--config`, passed through.
    pub config: Option<PathBuf>,
    /// `--data-dir`, passed through.
    pub data_dir: Option<PathBuf>,
    /// The env file, when there is a database URL.
    pub env_file: Option<PathBuf>,
}

/// The box `install` acts on: the home directory, whether we are root, and (for tests) the
/// PATH the systemd tools are looked up on.
#[derive(Debug, Clone)]
pub struct Host {
    pub home: PathBuf,
    pub root: bool,
    pub user: String,
    pub path: Option<OsString>,
    pub wait: Duration,
}

impl Host {
    fn current() -> anyhow::Result<Host> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        let user = match std::env::var("USER") {
            Ok(u) if !u.is_empty() => u,
            _ => output(Command::new("id").arg("-un"))?.trim().to_string(),
        };
        Ok(Host {
            home,
            root: euid_is_root(),
            user,
            path: None,
            wait: WAIT,
        })
    }

    fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        if let Some(p) = &self.path {
            c.env("PATH", p);
        }
        c
    }

    /// `systemctl` (with `--user` for a user unit).
    fn systemctl(&self, mode: Mode) -> Command {
        let mut c = self.cmd("systemctl");
        if mode == Mode::User {
            c.arg("--user");
        }
        c
    }

    fn journalctl(&self, mode: Mode) -> Command {
        let mut c = self.cmd("journalctl");
        if mode == Mode::User {
            c.arg("--user");
        }
        c
    }

    fn unit_path(&self, mode: Mode) -> PathBuf {
        match mode {
            Mode::User => self.home.join(".config/systemd/user/silvermere.service"),
            Mode::System => PathBuf::from("/etc/systemd/system/silvermere.service"),
        }
    }

    fn env_dir(&self, mode: Mode) -> PathBuf {
        match mode {
            Mode::User => self.home.join(".config/silvermere"),
            Mode::System => PathBuf::from("/etc/silvermere"),
        }
    }

    fn mode(&self, m: &ModeArgs) -> anyhow::Result<Mode> {
        match (m.user, m.system, self.root) {
            (_, true, false) => bail!(
                "--system needs root: run it with sudo (sudo {} service install --system)",
                std::env::current_exe()?.display()
            ),
            (true, _, true) => bail!(
                "--user as root would install a unit for root; use --system, or run as the \
                 user who runs the server"
            ),
            (true, _, false) | (false, false, false) => Ok(Mode::User),
            (_, true, true) | (false, false, true) => Ok(Mode::System),
        }
    }
}

/// Whether the effective user is root, from /proc (no libc call).
fn euid_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|u| u == "0"))
        })
        .unwrap_or(false)
}

/// Runs `service` (see [`super::run`]).
pub fn run(cmd: &ServiceCmd, data_dir: Option<&Path>, ov: &Overrides) -> anyhow::Result<ExitCode> {
    ensure!(
        cfg!(target_os = "linux"),
        "`silvermere service` writes systemd units and runs on Linux only"
    );
    let host = Host::current()?;
    match cmd {
        ServiceCmd::Install(a) => {
            let mode = host.mode(&a.mode)?;
            let url = if a.database_url_from_stdin {
                Some(read_url()?)
            } else {
                None
            };
            let cwd = std::env::current_dir()?;
            let plan = Plan {
                mode,
                exe: std::env::current_exe()?.canonicalize()?,
                workdir: cwd.canonicalize()?,
                config: ov.config_file.as_deref().map(|p| cwd.join(p)),
                data_dir: data_dir.map(|p| cwd.join(p)),
                env_file: None,
            };
            install(&host, plan, url.as_deref(), &mut std::io::stdout(), |url| {
                setup(data_dir, ov, url)
            })?;
        }
        ServiceCmd::Uninstall(m) => uninstall(&host, host.mode(m)?, &mut std::io::stdout())?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Reads the database URL: hidden from a terminal, else one line of stdin.
fn read_url() -> anyhow::Result<String> {
    use std::io::IsTerminal as _;
    let line = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Database URL: ")?
    } else {
        let mut l = String::new();
        std::io::stdin().lock().read_line(&mut l)?;
        l
    };
    let url = line.trim().to_string();
    ensure!(!url.is_empty(), "no database URL on stdin");
    Ok(url)
}

/// The first start without serving (`first_start::prepare`) on the configuration the unit
/// will use: the stock world if the database is empty, a sysop if there is none. The prompts
/// read the terminal, since stdin may have carried the URL.
fn setup(data_dir: Option<&Path>, ov: &Overrides, url: Option<&str>) -> anyhow::Result<()> {
    let mut ov = Overrides {
        config_file: ov.config_file.clone(),
        flag_lines: ov.flag_lines.clone(),
    };
    if let Some(u) = url {
        ov.flag_lines.push(format!(
            "database.url = {}",
            toml::Value::String(u.to_owned())
        ));
    }
    let cfg = Config::load(data_dir, &ov)?;
    match std::fs::File::open("/dev/tty") {
        Ok(tty) => crate::commands::first_start::prepare(
            &cfg,
            &mut std::io::BufReader::new(tty),
            &mut std::io::stdout(),
            true,
        ),
        Err(_) => crate::commands::first_start::prepare(
            &cfg,
            &mut std::io::stdin().lock(),
            &mut std::io::stdout(),
            false,
        ),
    }
}

/// The install, step by step. `prepare` builds the world and asks for the sysop, given the
/// database URL the unit will use.
pub fn install(
    host: &Host,
    mut plan: Plan,
    url: Option<&str>,
    out: &mut impl Write,
    prepare: impl FnOnce(Option<&str>) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mode = plan.mode;
    let env_file = host.env_dir(mode).join("env");
    // Refuse a bad URL before anything is written.
    let env = url.map(env_text).transpose()?;
    if mode == Mode::System {
        ensure_system_user(host, &plan.workdir, out)?;
        check_reachable(&plan)?;
    }
    // A re-run without a URL keeps the one written before.
    let url = match url {
        Some(u) => Some(u.to_string()),
        None if env_file.exists() => Some(sourced_url(host, &env_file)?),
        None => None,
    };
    let url = url.filter(|u| !u.is_empty());
    prepare(url.as_deref())?;
    if mode == Mode::System {
        let data = plan.data_dir.clone().unwrap_or(plan.workdir.join("data"));
        std::fs::create_dir_all(&data)?;
        run_ok(
            host.cmd("chown")
                .args(["-R", &format!("{SYSTEM_USER}:{SYSTEM_USER}")])
                .arg(&data),
        )?;
    }
    if let (Some(text), Some(u)) = (&env, url.as_deref()) {
        write_env(host, mode, &env_file, text, u)?;
        writeln!(
            out,
            "Wrote the database URL to {} (mode 600).",
            env_file.display()
        )?;
    }
    if env_file.exists() {
        plan.env_file = Some(env_file.clone());
    }

    let unit = unit_text(&plan)?;
    let path = host.unit_path(mode);
    let before = std::fs::read_to_string(&path).ok();
    if before.as_deref() != Some(unit.as_str()) {
        std::fs::create_dir_all(path.parent().expect("unit path has a parent"))?;
        write_file(&path, &unit, 0o644)?;
        let verb = if before.is_some() { "Updated" } else { "Wrote" };
        writeln!(out, "{verb} {}.", path.display())?;
    } else {
        writeln!(out, "{} is up to date.", path.display())?;
    }
    if mode == Mode::User {
        linger(host, out)?;
    }

    run_ok(host.systemctl(mode).arg("daemon-reload"))?;
    run_ok(host.systemctl(mode).args(["enable", UNIT]))?;
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    run_ok(host.systemctl(mode).args(["restart", UNIT]))?;
    wait_listening(host, mode, since, out)?;

    let status = host
        .systemctl(mode)
        .args(["status", UNIT, "--no-pager"])
        .output()?;
    out.write_all(&status.stdout)?;
    writeln!(out)?;
    let u = if mode == Mode::User { " --user" } else { "" };
    let sudo = if mode == Mode::User { "" } else { "sudo " };
    writeln!(out, "Logs:      {sudo}journalctl{u} -u {UNIT} -f")?;
    writeln!(out, "Restart:   {sudo}systemctl{u} restart {UNIT}")?;
    let crontab = match mode {
        Mode::User => "crontab -e".to_string(),
        Mode::System => format!("sudo crontab -u {SYSTEM_USER} -e"),
    };
    writeln!(
        out,
        "Backups:   add this line with `{crontab}` (nightly at 04:00, keeps 14):\n  {}",
        cron_line(&plan)
    )?;
    Ok(())
}

/// Stops and removes the unit. The data and the env file stay.
pub fn uninstall(host: &Host, mode: Mode, out: &mut impl Write) -> anyhow::Result<()> {
    let path = host.unit_path(mode);
    let unit = std::fs::read_to_string(&path).ok();
    // disable --now fails when the unit is already gone; that is not an error here
    let _ = host
        .systemctl(mode)
        .args(["disable", "--now", UNIT])
        .stderr(Stdio::null())
        .status();
    if unit.is_some() {
        std::fs::remove_file(&path)?;
        writeln!(out, "Removed {}.", path.display())?;
    } else {
        writeln!(out, "No unit at {}.", path.display())?;
    }
    run_ok(host.systemctl(mode).arg("daemon-reload"))?;
    let _ = host
        .systemctl(mode)
        .args(["reset-failed", UNIT])
        .stderr(Stdio::null())
        .status();
    if let Some(dir) = unit.as_deref().and_then(|u| {
        u.lines()
            .find_map(|l| l.strip_prefix("WorkingDirectory="))
            .map(|d| d.replace("%%", "%"))
    }) {
        writeln!(out, "The data in {dir} is still there.")?;
    }
    let env = host.env_dir(mode).join("env");
    if env.exists() {
        writeln!(out, "The database URL in {} is still there.", env.display())?;
    }
    Ok(())
}

/// The unit file. `StartLimitIntervalSec` belongs in `[Unit]`: in `[Service]` systemd
/// ignores it.
pub fn unit_text(plan: &Plan) -> anyhow::Result<String> {
    let mut exec = vec![plan.exe.clone()];
    if let Some(c) = &plan.config {
        exec.push("--config".into());
        exec.push(c.clone());
    }
    if let Some(d) = &plan.data_dir {
        exec.push("--data-dir".into());
        exec.push(d.clone());
    }
    exec.push("serve".into());
    let exec = exec
        .iter()
        .map(|a| exec_arg(a))
        .collect::<anyhow::Result<Vec<_>>>()?
        .join(" ");
    let workdir = path_value(&plan.workdir)?;
    let mut s = String::from("# Written by `silvermere service install`; a re-run rewrites it.\n");
    s += "[Unit]\n";
    match plan.mode {
        Mode::User => s += "Description=silvermere (user unit)\n",
        Mode::System => {
            s += "Description=silvermere\n";
            s += "Wants=network-online.target\n";
            s += "After=network-online.target\n";
        }
    }
    s += "StartLimitIntervalSec=0\n\n[Service]\n";
    if plan.mode == Mode::System {
        s += &format!("User={SYSTEM_USER}\nGroup={SYSTEM_USER}\n");
    }
    s += &format!("WorkingDirectory={workdir}\nExecStart={exec}\n");
    if let Some(e) = &plan.env_file {
        s += &format!("EnvironmentFile={}\n", path_value(e)?);
    }
    s += "Restart=on-failure\nRestartSec=5\nTimeoutStopSec=60\nUMask=0027\n\n[Install]\n";
    s += match plan.mode {
        Mode::User => "WantedBy=default.target\n",
        Mode::System => "WantedBy=multi-user.target\n",
    };
    Ok(s)
}

/// A path as a unit setting's value: `%` doubled (systemd's specifiers).
fn path_value(p: &Path) -> anyhow::Result<String> {
    let s = p
        .to_str()
        .with_context(|| format!("{} is not UTF-8", p.display()))?;
    ensure!(
        !s.chars().any(|c| c.is_control() || c == '"' || c == '\\'),
        "{s:?}: a path with a quote, a backslash or a control character cannot go in a unit"
    );
    Ok(s.replace('%', "%%"))
}

/// One `ExecStart` word: `%` and `$` doubled, quoted when it holds a space.
fn exec_arg(p: &Path) -> anyhow::Result<String> {
    let s = path_value(p)?.replace('$', "$$");
    Ok(if s.contains(char::is_whitespace) {
        format!("\"{s}\"")
    } else {
        s
    })
}

/// The env file: the URL single-quoted, so the shell that loads it for cron or by hand keeps
/// `&` and `$` literal (an unquoted `&` ends the line). A URL with a quote or a line break
/// is refused: percent-encode it (`%27`).
pub fn env_text(url: &str) -> anyhow::Result<String> {
    ensure!(
        !url.contains(['\'', '\n', '\r', '\0']),
        "the database URL holds a single quote or a line break; percent-encode it (' is %27)"
    );
    ensure!(
        ["postgres://", "postgresql://", "sqlite:"]
            .iter()
            .any(|p| url.starts_with(p)),
        "the database URL should start with postgres://, postgresql:// or sqlite:"
    );
    if let Some(what) = silvermere_db::guard::known_pooler(url) {
        bail!(
            "the database URL goes through a transaction-mode connection pooler ({what}), \
             which cannot keep the server's single-writer lock. Use the direct (unpooled) \
             connection string; on Neon, the host without `-pooler`."
        );
    }
    Ok(format!(
        "# Written by `silvermere service install`. Single quotes keep & and $ literal.\n\
         SILVERMERE_DATABASE_URL='{url}'\n"
    ))
}

/// Writes the env file mode 600 in a 700 directory, then checks the URL comes back
/// unchanged through `sh` (as cron and the docs' `set -a; . env` load it).
fn write_env(host: &Host, mode: Mode, path: &Path, text: &str, url: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    let dir = path.parent().expect("env file has a parent");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    write_file(path, text, 0o600)?;
    if mode == Mode::System {
        run_ok(
            host.cmd("chown")
                .args(["-R", &format!("{SYSTEM_USER}:{SYSTEM_USER}")])
                .arg(dir),
        )?;
    }
    let back = sourced_url(host, path)?;
    if back != url {
        let _ = std::fs::remove_file(path);
        bail!(
            "the database URL did not survive the shell's reading of {} (removed it); \
             percent-encode its unusual characters",
            path.display()
        );
    }
    Ok(())
}

/// `SILVERMERE_DATABASE_URL` as `sh` reads it from `path`.
fn sourced_url(host: &Host, path: &Path) -> anyhow::Result<String> {
    output(
        host.cmd("sh")
            .args([
                "-c",
                "set -a; . \"$1\"; printf %s \"${SILVERMERE_DATABASE_URL-}\"",
                "sh",
            ])
            .arg(path),
    )
}

/// Writes `text` to `path` with `mode` set before any byte is written, through a temporary
/// file and a rename, so a reader never sees half a file.
fn write_file(path: &Path, text: &str, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)
        .with_context(|| format!("writing {}", tmp.display()))?;
    f.set_permissions(std::fs::Permissions::from_mode(mode))?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// The nightly backup line for the crontab of whoever runs the server. cron does not read
/// the unit's `EnvironmentFile`, so the line loads it.
pub fn cron_line(plan: &Plan) -> String {
    let q = |p: &Path| sh_quote(&p.to_string_lossy());
    let mut line = format!("0 4 * * * cd {} && ", q(&plan.workdir));
    if let Some(e) = &plan.env_file {
        line += &format!("set -a && . {} && set +a && ", q(e));
    }
    line += &q(&plan.exe);
    if let Some(c) = &plan.config {
        line += &format!(" --config {}", q(c));
    }
    if let Some(d) = &plan.data_dir {
        line += &format!(" --data-dir {}", q(d));
    }
    line + " backup --keep 14"
}

/// `s` as one shell word; a crontab line also treats `%` as a line break, so it is escaped.
fn sh_quote(s: &str) -> String {
    let s = s.replace('%', "\\%");
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+:@,\\%".contains(c))
    {
        s
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Creates the `silvermere` system user when missing.
fn ensure_system_user(host: &Host, home: &Path, out: &mut impl Write) -> anyhow::Result<()> {
    let have = host
        .cmd("id")
        .arg(SYSTEM_USER)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    if !have {
        run_ok(
            host.cmd("useradd")
                .args(["--system", "--user-group", "--home-dir"])
                .arg(home)
                .args(["--shell", "/usr/sbin/nologin", SYSTEM_USER]),
        )?;
        writeln!(out, "Created the system user {SYSTEM_USER}.")?;
    }
    Ok(())
}

/// The `silvermere` user must be able to reach the binary and its working directory: every
/// folder above them searchable by others (a home directory is often 750).
fn check_reachable(plan: &Plan) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    for target in [&plan.exe, &plan.workdir] {
        for dir in target.ancestors().skip(1) {
            let m = std::fs::metadata(dir)?.permissions().mode();
            ensure!(
                m & 0o001 != 0,
                "the {SYSTEM_USER} user cannot reach {} ({} is mode {:o}). Install it \
                 where every user can read it, such as /opt/silvermere \
                 (install.sh --system does), and run this from there",
                target.display(),
                dir.display(),
                m & 0o777
            );
        }
    }
    Ok(())
}

/// Turns on lingering so the user manager, and the server, start at boot without a login.
fn linger(host: &Host, out: &mut impl Write) -> anyhow::Result<()> {
    let on = host
        .cmd("loginctl")
        .args(["show-user", &host.user, "--property=Linger", "--value"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
        .unwrap_or(false);
    if on {
        return Ok(());
    }
    let ok = host
        .cmd("loginctl")
        .args(["enable-linger", &host.user])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        writeln!(
            out,
            "Enabled lingering for {}: the server starts at boot.",
            host.user
        )?;
    } else {
        writeln!(
            out,
            "Lingering is off, so the server would not start at boot until you log in. \
             Turn it on with:\n  sudo loginctl enable-linger {}",
            host.user
        )?;
    }
    Ok(())
}

/// Waits for `serve`'s listening line in the journal, from `since` (seconds since the epoch)
/// on. Fails early when the unit fails.
fn wait_listening(host: &Host, mode: Mode, since: u64, out: &mut impl Write) -> anyhow::Result<()> {
    let start = Instant::now();
    let journal = || {
        host.journalctl(mode)
            .args(["-u", UNIT, "--no-pager", "-o", "cat"])
            .arg(format!("--since=@{since}"))
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    loop {
        let log = journal();
        if let Some(line) = log.lines().find(|l| l.contains("telnet listening on")) {
            writeln!(out, "{}", line.trim())?;
            return Ok(());
        }
        let state = host
            .systemctl(mode)
            .args(["is-active", UNIT])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if state == "failed" || start.elapsed() >= host.wait {
            let tail: Vec<&str> = log.lines().rev().take(20).collect();
            for l in tail.iter().rev() {
                writeln!(out, "  {l}")?;
            }
            if state == "active" {
                writeln!(
                    out,
                    "The service is running, but its listening line did not show in the \
                     journal; check the logs below."
                )?;
                return Ok(());
            }
            bail!("the service did not start ({state}); the journal's last lines are above");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn run_ok(c: &mut Command) -> anyhow::Result<()> {
    let what = format!("{c:?}").replace('"', "");
    let s = c.status().with_context(|| format!("running {what}"))?;
    ensure!(s.success(), "{what} failed ({s})");
    Ok(())
}

fn output(c: &mut Command) -> anyhow::Result<String> {
    let what = format!("{c:?}").replace('"', "");
    let o = c
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running {what}"))?;
    ensure!(o.status.success(), "{what} failed ({})", o.status);
    Ok(String::from_utf8(o.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(mode: Mode) -> Plan {
        Plan {
            mode,
            exe: "/opt/silver mere/silvermere".into(),
            workdir: "/opt/silver mere".into(),
            config: None,
            data_dir: Some("/srv/100%/data".into()),
            env_file: Some("/etc/silvermere/env".into()),
        }
    }

    /// The lines of `section` in `unit`.
    fn section<'a>(unit: &'a str, section: &str) -> Vec<&'a str> {
        unit.split(&format!("[{section}]\n"))
            .nth(1)
            .unwrap_or_default()
            .split("\n[")
            .next()
            .unwrap()
            .lines()
            .collect()
    }

    #[test]
    fn unit_settings_are_in_their_sections() {
        let u = unit_text(&plan(Mode::System)).unwrap();
        let unit = section(&u, "Unit");
        let service = section(&u, "Service");
        assert!(unit.contains(&"StartLimitIntervalSec=0"), "{u}");
        assert!(unit.contains(&"After=network-online.target"));
        assert!(!service.iter().any(|l| l.starts_with("StartLimit")));
        for want in [
            "User=silvermere",
            "WorkingDirectory=/opt/silver mere",
            "ExecStart=\"/opt/silver mere/silvermere\" --data-dir /srv/100%%/data serve",
            "EnvironmentFile=/etc/silvermere/env",
            "Restart=on-failure",
            "RestartSec=5",
            "TimeoutStopSec=60",
            "UMask=0027",
        ] {
            assert!(service.contains(&want), "{want} not in [Service]:\n{u}");
        }
        assert_eq!(section(&u, "Install"), ["WantedBy=multi-user.target"]);
    }

    #[test]
    fn user_unit_has_no_user_or_network_target() {
        let mut p = plan(Mode::User);
        p.env_file = None;
        p.data_dir = None;
        let u = unit_text(&p).unwrap();
        assert!(!u.contains("User="), "{u}");
        assert!(!u.contains("network-online"));
        assert!(!u.contains("EnvironmentFile"));
        assert!(section(&u, "Unit").contains(&"StartLimitIntervalSec=0"));
        assert!(
            section(&u, "Service").contains(&"ExecStart=\"/opt/silver mere/silvermere\" serve")
        );
        assert_eq!(section(&u, "Install"), ["WantedBy=default.target"]);
    }

    #[test]
    fn a_path_with_a_quote_is_refused() {
        let mut p = plan(Mode::User);
        p.workdir = "/home/a\"b".into();
        assert!(unit_text(&p).is_err());
    }

    #[test]
    fn env_file_quotes_and_round_trips_through_sh() {
        let url = "postgres://u:p$a&ss@ep-x.neon.tech/db?sslmode=require&channel_binding=require";
        let text = env_text(url).unwrap();
        assert!(
            text.contains(&format!("SILVERMERE_DATABASE_URL='{url}'\n")),
            "{text}"
        );
        let dir = tmp("env");
        let f = dir.join("env");
        std::fs::write(&f, &text).unwrap();
        assert_eq!(sourced_url(&host(&dir, None), &f).unwrap(), url);
    }

    #[test]
    fn env_refuses_quotes_newlines_and_poolers() {
        assert!(env_text("postgres://u:it's@h/db").is_err());
        assert!(env_text("postgres://u:p@h/db\nX=1").is_err());
        assert!(env_text("mysql://h/db").is_err());
        let e = env_text("postgres://u:p@ep-x-pooler.us-east-2.aws.neon.tech/db")
            .unwrap_err()
            .to_string();
        assert!(e.contains("pooler"), "{e}");
    }

    #[test]
    fn cron_line_loads_the_env_file() {
        let mut p = plan(Mode::User);
        p.data_dir = None;
        assert_eq!(
            cron_line(&p),
            "0 4 * * * cd '/opt/silver mere' && set -a && . /etc/silvermere/env && set +a && \
             '/opt/silver mere/silvermere' backup --keep 14"
        );
    }

    fn tmp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("silvermere-service-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn host(home: &Path, fake: Option<&Path>) -> Host {
        let path = fake.map(|f| {
            let mut p = OsString::from(f);
            p.push(":");
            p.push(std::env::var_os("PATH").unwrap_or_default());
            p
        });
        Host {
            home: home.to_path_buf(),
            root: false,
            user: "you".into(),
            path,
            wait: Duration::from_secs(5),
        }
    }

    /// Fake systemctl, journalctl and loginctl that append their arguments to `log`.
    fn fakes(dir: &Path, log: &Path, linger: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = log.display();
        let scripts = [
            (
                "systemctl",
                format!(
                    "echo \"systemctl $*\" >> '{log}'\n\
                     case \"$*\" in *is-active*) echo active;; *status*) echo '● silvermere.service';; esac"
                ),
            ),
            (
                "journalctl",
                format!(
                    "echo \"journalctl $*\" >> '{log}'\n\
                     echo 'silvermere: telnet listening on 0.0.0.0:2323'"
                ),
            ),
            (
                "loginctl",
                format!(
                    "echo \"loginctl $*\" >> '{log}'\n\
                     case \"$1\" in show-user) echo {linger};; esac"
                ),
            ),
        ];
        for (name, body) in scripts {
            let p = bin.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin
    }

    #[test]
    fn install_and_uninstall_run_systemctl_in_order() {
        let dir = tmp("install");
        let home = dir.join("home");
        let log = dir.join("log");
        let bin = fakes(&dir, &log, "no");
        let h = host(&home, Some(&bin));
        let p = Plan {
            mode: Mode::User,
            exe: home.join("silvermere/silvermere"),
            workdir: home.join("silvermere"),
            config: None,
            data_dir: None,
            env_file: None,
        };
        let url = "postgres://u:p&q@db.example.com/sm?sslmode=require";
        let mut out = Vec::new();
        let mut seen = None;
        install(&h, p.clone(), Some(url), &mut out, |u| {
            seen = u.map(str::to_string);
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.as_deref(), Some(url));
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("telnet listening on 0.0.0.0:2323"), "{out}");
        assert!(out.contains("backup --keep 14"), "{out}");

        let calls = std::fs::read_to_string(&log).unwrap();
        let calls: Vec<&str> = calls
            .lines()
            .filter(|l| !l.contains("is-active"))
            .map(|l| l.split(" --since").next().unwrap())
            .collect();
        assert_eq!(
            calls,
            [
                "loginctl show-user you --property=Linger --value",
                "loginctl enable-linger you",
                "systemctl --user daemon-reload",
                "systemctl --user enable silvermere",
                "systemctl --user restart silvermere",
                "journalctl --user -u silvermere --no-pager -o cat",
                "systemctl --user status silvermere --no-pager",
            ]
        );

        use std::os::unix::fs::PermissionsExt as _;
        let env = home.join(".config/silvermere/env");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&env), 0o600);
        assert_eq!(mode(env.parent().unwrap()), 0o700);
        let unit_path = home.join(".config/systemd/user/silvermere.service");
        let unit = std::fs::read_to_string(&unit_path).unwrap();
        assert!(
            unit.contains(&format!("EnvironmentFile={}", env.display())),
            "{unit}"
        );

        // a re-run without a URL keeps the env file and passes its URL to the setup
        std::fs::remove_file(&log).unwrap();
        let mut out = Vec::new();
        let mut seen = None;
        install(&h, p, None, &mut out, |u| {
            seen = u.map(str::to_string);
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.as_deref(), Some(url));
        assert_eq!(std::fs::read_to_string(&unit_path).unwrap(), unit);
        assert!(String::from_utf8(out).unwrap().contains("is up to date"));

        std::fs::remove_file(&log).unwrap();
        let mut out = Vec::new();
        uninstall(&h, Mode::User, &mut out).unwrap();
        assert!(!unit_path.exists());
        assert!(env.exists(), "uninstall keeps the env file");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            [
                "systemctl --user disable --now silvermere",
                "systemctl --user daemon-reload",
                "systemctl --user reset-failed silvermere",
            ]
        );
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("still there"), "{out}");
    }

    #[test]
    fn linger_without_rights_prints_the_sudo_command() {
        let dir = tmp("linger");
        let log = dir.join("log");
        let bin = fakes(&dir, &log, "no");
        // a loginctl that refuses enable-linger
        std::fs::write(
            bin.join("loginctl"),
            "#!/bin/sh\ncase \"$1\" in show-user) echo no;; *) exit 1;; esac\n",
        )
        .unwrap();
        let mut out = Vec::new();
        linger(&host(&dir, Some(&bin)), &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("sudo loginctl enable-linger you"), "{out}");
    }

    #[test]
    fn mode_defaults_and_refusals() {
        let mut h = host(Path::new("/h"), None);
        assert_eq!(h.mode(&ModeArgs::default()).unwrap(), Mode::User);
        assert!(
            h.mode(&ModeArgs {
                user: false,
                system: true
            })
            .is_err()
        );
        h.root = true;
        assert_eq!(h.mode(&ModeArgs::default()).unwrap(), Mode::System);
        assert!(
            h.mode(&ModeArgs {
                user: true,
                system: false
            })
            .is_err()
        );
    }
}
