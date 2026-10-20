//! `account`: calls `silvermere-auth::cli` (T-013).
//!
//! Passwords are read as one line from stdin (`--password-stdin` for `create`; always
//! for `passwd NAME`), so scripts can pipe them in; `passwd --missing` makes them up. Role
//! changes take effect at the account's next login.
//!
//! `delete` goes through the running server's `DELETE /v1/accounts/{id}` when one answers
//! on `server.admin_bind` (T-069), so the engine's write-behind cannot bring the rows
//! back. Otherwise it deletes directly only when it can take the store's single-writer
//! guard itself (T-072): with the guard held by a server whose admin API cannot be
//! reached, it refuses.

use std::io::{BufRead as _, Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use silvermere_auth::Role;
use silvermere_auth::cli::{self, Grant};
use silvermere_db::{Db, TryAcquire, WriterGuard};

use crate::cli::AccountCmd;
use crate::config::{Config, TokenScope};

fn read_password(prompt: &str) -> anyhow::Result<String> {
    eprint!("{prompt}");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the password from stdin")?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

pub fn run(cfg: &Config, action: &AccountCmd) -> anyhow::Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        super::ensure_sqlite_dir(cfg)?;
        let db = Db::connect_and_migrate(&cfg.database.url)
            .await
            .with_context(|| format!("opening {}", cfg.database_shown()))?;
        let result = run_on(cfg, &db, action).await;
        db.close().await;
        result
    })
}

async fn run_on(cfg: &Config, db: &Db, action: &AccountCmd) -> anyhow::Result<ExitCode> {
    match action {
        AccountCmd::Create {
            name,
            sysop,
            gameop,
            password_stdin: _,
        } => {
            let password = read_password("Password: ")?;
            let mut roles = vec![Role::Player];
            if *sysop {
                roles.push(Role::Sysop);
            }
            if *gameop {
                roles.push(Role::GameOp);
            }
            let account = cli::create(db, name, &password, &roles).await?;
            eprintln!("created account {} (id {})", account.username, account.id.0);
        }
        AccountCmd::Passwd {
            missing: true,
            dry_run,
            out,
            ..
        } => set_missing(db, *dry_run, out.as_deref()).await?,
        AccountCmd::Passwd { name, .. } => {
            let name = name.as_deref().context("a NAME or --missing")?;
            let password = read_password("New password: ")?;
            cli::set_password(db, name, &password).await?;
            eprintln!("password changed for {name}");
        }
        AccountCmd::Grant { name, role } => {
            let what: Grant = role.parse()?;
            cli::grant(db, name, what).await?;
            eprintln!("granted {what} to {name}");
        }
        AccountCmd::Revoke { name, role } => {
            let what: Grant = role.parse()?;
            cli::revoke(db, name, what).await?;
            eprintln!("revoked {what} from {name}");
        }
        AccountCmd::List => {
            for account in cli::list(db).await? {
                println!("{}", cli::describe(&account));
            }
        }
        AccountCmd::Delete { name, yes } => {
            if !yes {
                anyhow::bail!("deleting {name} removes its character too; pass --yes");
            }
            match probe(&cfg.server.admin_bind) {
                Server::Up(addr) => {
                    delete_through(cfg, db, addr, name).await?;
                    eprintln!("deleted account {name} through the running server");
                }
                other => match WriterGuard::try_acquire(&cfg.database.url).await? {
                    // no server holds the store: nothing can write the rows back
                    TryAcquire::Taken(guard) => {
                        let deleted = cli::delete(db, name).await;
                        guard.release().await;
                        if !deleted? {
                            anyhow::bail!("no account named {name}");
                        }
                        eprintln!("deleted account {name}");
                    }
                    TryAcquire::Held(by) => {
                        let why = match other {
                            Server::Unusable(why) => why,
                            _ => "nothing answers there".into(),
                        };
                        anyhow::bail!(
                            "a server is running on this store ({by}) but its admin API on {} \
                             cannot be used ({why}); not deleting {name} behind its back, since \
                             its write-behind could bring the rows back. Fix server.admin_bind \
                             or the admin API, or stop the server, then try again",
                            cfg.server.admin_bind
                        )
                    }
                },
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `account passwd --missing`: a random password for every account without one, one
/// `name<TAB>password` line each to stdout or to `out`, never to the log. The file is
/// opened before any password changes, so a refused `--out` changes nothing, and each line
/// is written and flushed before its password is stored, so a failed write (stdout closed
/// by `| head`, a full disk) never leaves an account with a password nobody saw.
async fn set_missing(db: &Db, dry_run: bool, out: Option<&Path>) -> anyhow::Result<()> {
    let accounts = cli::without_password(db).await?;
    if dry_run {
        for account in &accounts {
            println!("{}", account.username);
        }
        eprintln!(
            "{} account(s) have no password; nothing changed (--dry-run)",
            accounts.len()
        );
        return Ok(());
    }
    let mut sink: Box<dyn std::io::Write> = match out {
        Some(path) => Box::new(create_private(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    let mut set = 0usize;
    for account in &accounts {
        let name = &account.username;
        let password = cli::generate_password();
        writeln!(sink, "{name}\t{password}")
            .and_then(|()| sink.flush())
            .with_context(|| {
                format!(
                    "writing the line for {name}; its password and those after it were not \
                     changed ({set} set before it)"
                )
            })?;
        match cli::set_missing_password(db, name, &password).await {
            Ok(true) => set += 1,
            Ok(false) => eprintln!(
                "{name} was given a password meanwhile; the line written for it does not apply"
            ),
            Err(e) => {
                return Err(anyhow::Error::new(e).context(format!(
                    "storing the password for {name} failed; the line written for it does not \
                     apply and the accounts after it were not changed ({set} set before it)"
                )));
            }
        }
    }
    match out {
        Some(path) => eprintln!(
            "set a password on {set} account(s); written to {}",
            path.display()
        ),
        None => eprintln!("set a password on {set} account(s)"),
    }
    Ok(())
}

/// A new file only its owner can read (mode 600 on Unix); refused if `path` exists.
fn create_private(path: &Path) -> anyhow::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::anyhow!("{} exists; not overwriting it", path.display())
        } else {
            anyhow::Error::new(e).context(format!("creating {}", path.display()))
        }
    })
}

/// How long the probe and the delete call wait on the admin API.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// What `account delete` found on `server.admin_bind` (T-069). Only `Up` decides the
/// route; otherwise the single-writer guard (T-072) says whether a server holds the store.
#[derive(Debug)]
enum Server {
    /// Connection refused.
    Down,
    /// `/healthz` answered `200`: delete through `DELETE /v1/accounts/{id}`.
    Up(SocketAddr),
    /// Something listens but the API cannot be used.
    Unusable(String),
}

/// The address a client reaches `bind` on: an unspecified address is reached on loopback.
fn reach(bind: &str) -> Option<SocketAddr> {
    let mut addr: SocketAddr = bind.parse().ok()?;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    Some(addr)
}

fn probe(bind: &str) -> Server {
    let Some(addr) = reach(bind) else {
        return Server::Unusable(format!("server.admin_bind `{bind}` is not host:port"));
    };
    match call(addr, "GET", "/healthz", None, PROBE_TIMEOUT) {
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => Server::Down,
        Err(e) => Server::Unusable(e.to_string()),
        Ok((200, _)) => Server::Up(addr),
        Ok((status, _)) => Server::Unusable(format!("/healthz answered {status}")),
    }
}

/// One HTTP/1.1 request with `Connection: close`; the status and the body.
fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    timeout: Duration,
) -> std::io::Result<(u16, String)> {
    let mut s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{auth}Content-Length: 0\r\n\r\n"
    )?;
    let mut raw = Vec::new();
    s.take(1 << 20).read_to_end(&mut raw)?;
    let raw = String::from_utf8_lossy(&raw);
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| std::io::Error::other("not an HTTP answer"))?;
    let body = raw.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    Ok((status, body.to_string()))
}

/// `account delete` with a server running: the account's id from the store, then
/// `DELETE /v1/accounts/{id}` with an `admin` token from the config.
async fn delete_through(cfg: &Config, db: &Db, addr: SocketAddr, name: &str) -> anyhow::Result<()> {
    let Some(account) = silvermere_auth::AccountStore::by_username(db, name).await? else {
        anyhow::bail!("no account named {name}");
    };
    let Some(token) = cfg
        .admin
        .tokens
        .iter()
        .filter(|t| t.scopes.contains(&TokenScope::Admin))
        .find_map(|t| t.token.clone())
    else {
        anyhow::bail!(
            "the server on {} is running, so {name} must be deleted through its admin API, \
             but the config has no admin-scope token with its secret (set \
             SILVERMERE_ADMIN_TOKEN, or stop the server)",
            cfg.server.admin_bind
        );
    };
    let path = format!("/v1/accounts/{}", account.id.0);
    let (status, body) = tokio::task::spawn_blocking(move || {
        call(addr, "DELETE", &path, Some(&token), CALL_TIMEOUT)
    })
    .await?
    .with_context(|| format!("calling DELETE /v1/accounts on {addr}"))?;
    match status {
        200 => Ok(()),
        404 => anyhow::bail!("no account named {name}"),
        504 => anyhow::bail!(
            "the server queued the deletion of {name} but its engine did not answer in time; \
             the deletion may still complete. Check with `silvermere account list` before \
             trying again"
        ),
        _ => anyhow::bail!(
            "the server refused to delete {name}: {status} {}",
            body.trim()
        ),
    }
}
