//! The single-writer guard (T-072): at most one process writes a store at a time.
//!
//! `serve` takes the guard at start and holds it for its lifetime; `account delete`
//! deletes rows directly only when it can take the guard itself. The guard is
//!
//! - on SQLite, an exclusive lock on `<database>.lock` beside the database file (the file
//!   stays; the lock goes with the process). The holder writes its pid into it.
//! - on PostgreSQL, the session advisory lock `pg_try_advisory_lock(GUARD_CLASS,
//!   hashtext(current_schema()))` on a connection of its own, so one store is one database
//!   and schema (tests run many schemas on one database). The lock goes when that
//!   connection closes, so it needs a direct connection: a transaction-mode pooler is
//!   refused (T-108).
//! - on an in-memory SQLite database, nothing: no other process can open it.
//!
//! A file lock cannot be lost while the process lives. An advisory lock goes with its
//! connection, so [`WriterGuard::watch`] pings that connection and re-takes the lock on a
//! fresh one when the connection dies (T-073).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::{AnyConnection, Connection as _, Executor as _};

use crate::dialect::Dialect;
use crate::{DbError, Result};

/// The first key of the PostgreSQL advisory lock: `SMWR` as a big-endian `i32`.
pub const GUARD_CLASS: i32 = 0x534D_5752;

/// A held single-writer guard; dropping it releases the lock.
#[derive(Debug)]
pub struct WriterGuard {
    held: Held,
}

#[derive(Debug)]
enum Held {
    File { _file: File, path: PathBuf },
    Postgres(PgHeld),
    Memory,
}

#[derive(Debug)]
struct PgHeld {
    url: String,
    /// `None` once the connection died and the lock is being re-taken.
    conn: Option<AnyConnection>,
    /// The backend that holds the lock, to tell our own dead session from another holder.
    pid: i32,
}

/// How often [`WriterGuard::watch`] checks the guard and how long it tries to re-take a
/// lost one.
#[derive(Debug, Clone, Copy)]
pub struct Watchdog {
    /// Time between two pings of the lock's connection.
    pub every: Duration,
    /// How long a lost lock is re-tried before giving up.
    pub give_up: Duration,
}

impl Default for Watchdog {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(15),
            give_up: Duration::from_secs(60),
        }
    }
}

/// Why [`WriterGuard::watch`] gave the guard up.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardLost {
    /// Another process took the lock while ours was gone.
    #[error("the writer guard was lost and is now held elsewhere: {0}")]
    HeldElsewhere(String),
    /// The lock could not be re-taken in time.
    #[error("the writer guard was lost and could not be re-taken within {secs} s: {error}")]
    Unreachable {
        /// The time spent trying.
        secs: u64,
        /// The last error.
        error: String,
    },
}

/// What [`WriterGuard::try_acquire`] found.
#[derive(Debug)]
pub enum TryAcquire {
    /// The guard is ours until dropped.
    Taken(WriterGuard),
    /// Another process holds it; what is known of the holder.
    Held(String),
}

impl WriterGuard {
    /// Tries to take the guard of the store at `url` without waiting.
    pub async fn try_acquire(url: &str) -> Result<TryAcquire> {
        match Dialect::from_url(url)? {
            Dialect::Sqlite => match sqlite_file(url) {
                Some(db) => lock_file(&lock_path(&db)),
                None => Ok(TryAcquire::Taken(Self { held: Held::Memory })),
            },
            Dialect::Postgres => lock_postgres(url).await,
        }
    }

    /// Releases the guard. Dropping it does the same; this waits for PostgreSQL to close
    /// the lock's connection.
    pub async fn release(self) {
        if let Held::Postgres(PgHeld {
            conn: Some(conn), ..
        }) = self.held
        {
            let _ = conn.close().await;
        }
    }

    /// Keeps the guard: on PostgreSQL, pings the lock's connection every `wd.every` and,
    /// when it fails, re-takes the lock on a fresh connection. Returns only when the lock
    /// is held by someone else or cannot be re-taken within `wd.give_up`; a file lock or
    /// an in-memory database never returns.
    pub async fn watch(&mut self, wd: Watchdog) -> GuardLost {
        let Held::Postgres(pg) = &mut self.held else {
            return std::future::pending().await;
        };
        loop {
            tokio::time::sleep(wd.every).await;
            let error = match pg.conn.as_mut() {
                Some(conn) => match ping(conn, wd.every).await {
                    Ok(()) => continue,
                    Err(e) => e,
                },
                None => "no connection".into(),
            };
            tracing::warn!(%error, "writer guard connection failed; re-taking the lock");
            if let Some(old) = pg.conn.take() {
                let _ = tokio::time::timeout(Duration::from_secs(2), old.close_hard()).await;
            }
            match retake(pg, wd).await {
                Ok(()) => tracing::info!(pid = pg.pid, "writer guard re-taken"),
                Err(lost) => return lost,
            }
        }
    }

    /// Where the guard lives, for messages.
    pub fn describe(&self) -> String {
        match &self.held {
            Held::File { path, .. } => format!("lock file {}", path.display()),
            Held::Postgres(_) => "a PostgreSQL advisory lock".into(),
            Held::Memory => "an in-memory database".into(),
        }
    }
}

/// The file of a `sqlite:` URL, `None` for an in-memory database.
fn sqlite_file(url: &str) -> Option<PathBuf> {
    if url.contains(":memory:") || url.contains("mode=memory") {
        return None;
    }
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))?;
    let rest = rest.split('?').next().unwrap_or(rest);
    (!rest.is_empty()).then(|| PathBuf::from(rest))
}

/// `<database>.lock`.
pub fn lock_path(db: &Path) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

fn lock_file(path: &Path) -> Result<TryAcquire> {
    let io = |e| DbError::Sqlx(sqlx::Error::Io(e));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    // no truncation before the lock is ours: the holder's pid stays readable
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(io)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let mut pid = String::new();
            let _ = file.read_to_string(&mut pid);
            let pid = pid.trim();
            let by = if pid.is_empty() {
                String::new()
            } else {
                format!(" (pid {pid})")
            };
            return Ok(TryAcquire::Held(format!(
                "lock file {} is held{by}",
                path.display()
            )));
        }
        Err(TryLockError::Error(e)) => return Err(io(e)),
    }
    file.set_len(0).map_err(io)?;
    file.rewind().map_err(io)?;
    writeln!(file, "{}", std::process::id()).map_err(io)?;
    file.flush().map_err(io)?;
    Ok(TryAcquire::Taken(WriterGuard {
        held: Held::File {
            _file: file,
            path: path.to_path_buf(),
        },
    }))
}

/// One check of the lock's connection.
async fn ping(conn: &mut AnyConnection, limit: Duration) -> std::result::Result<(), String> {
    match tokio::time::timeout(limit, conn.execute("SELECT 1")).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("no answer within {} s", limit.as_secs())),
    }
}

/// Re-takes a lost advisory lock on a fresh connection, trying until `wd.give_up`.
async fn retake(pg: &mut PgHeld, wd: Watchdog) -> std::result::Result<(), GuardLost> {
    retake_with(pg, wd, |url| async move { try_postgres(&url).await }).await
}

async fn retake_with<F, Fut>(
    pg: &mut PgHeld,
    wd: Watchdog,
    mut try_once: F,
) -> std::result::Result<(), GuardLost>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<PgTry>>,
{
    let deadline = tokio::time::Instant::now() + wd.give_up;
    let pause = (wd.every / 4).clamp(Duration::from_millis(50), Duration::from_secs(2));
    loop {
        let error = match tokio::time::timeout_at(deadline, try_once(pg.url.clone())).await {
            Ok(Ok(PgTry::Taken(conn, pid))) => {
                pg.conn = Some(conn);
                pg.pid = pid;
                return Ok(());
            }
            // our own session, not yet reaped by the server: wait for it to go
            Ok(Ok(PgTry::Held(Some(holder)))) if holder == pg.pid => {
                format!("backend pid {holder} (the lost session) still holds it")
            }
            // busy but no holder found: it let go between the try and the lookup, or the
            // lost session is being reaped; try again
            Ok(Ok(PgTry::Held(None))) => "busy, holder unknown".into(),
            Ok(Ok(PgTry::Held(holder))) => {
                return Err(GuardLost::HeldElsewhere(held_message(holder)));
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "timed out".into(),
        };
        if tokio::time::Instant::now() + pause >= deadline {
            return Err(GuardLost::Unreachable {
                secs: wd.give_up.as_secs(),
                error,
            });
        }
        tracing::warn!(%error, "writer guard not re-taken yet");
        tokio::time::sleep(pause).await;
    }
}

/// What one try at the advisory lock found.
enum PgTry {
    /// The lock and our backend pid.
    Taken(AnyConnection, i32),
    /// Someone holds it: their backend pid when known.
    Held(Option<i32>),
}

/// The keepalive settings of the lock's connection: the server notices a vanished client
/// in about 30 s and frees the lock. sqlx sets no client-side keepalive; the watchdog's
/// ping covers that side. Ignored on a Unix socket.
const KEEPALIVE: &str = "SET tcp_keepalives_idle = 15; SET tcp_keepalives_interval = 5; \
     SET tcp_keepalives_count = 3; SET application_name = 'silvermere writer guard'";

async fn try_postgres(url: &str) -> Result<PgTry> {
    try_postgres_with(url, pooled).await
}

/// [`try_postgres`] with the pooler verdict given, so a test can make it fire.
async fn try_postgres_with(url: &str, pooled: fn(&Pids) -> Option<String>) -> Result<PgTry> {
    sqlx::any::install_default_drivers();
    let mut conn = AnyConnection::connect(url).await?;
    let schema: Option<String> = sqlx::query_scalar("SELECT current_schema()::text")
        .fetch_one(&mut conn)
        .await?;
    if schema.is_none() {
        let _ = conn.close().await;
        return Err(DbError::Sqlx(sqlx::Error::Configuration(
            "the PostgreSQL search_path names no schema that exists (current_schema() is \
             NULL): create the schema, or fix search_path in the database URL's options \
             or the role's settings"
                .into(),
        )));
    }
    // T-108: the lock is taken only on a connection that looks direct, so a pooler found
    // here leaves no lock behind on a pooled backend
    let pids = backend_pids(&mut conn, url).await?;
    if let Some(what) = pooled(&pids) {
        let _ = conn.close().await;
        return Err(pooler_error(&what));
    }
    conn.execute(sqlx::raw_sql(KEEPALIVE)).await?;
    let taken: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_lock($1, hashtext(current_schema()))")
            .bind(GUARD_CLASS)
            .fetch_one(&mut conn)
            .await?;
    if taken {
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut conn)
            .await?;
        // a last check, best effort: a pooler the probe missed may still show here; the
        // unlock can then land on another backend, but refusing beats guarding nothing
        if let Some(holder) = holder_pid(&mut conn).await?.filter(|h| *h != pid) {
            let _ = sqlx::query("SELECT pg_advisory_unlock_all()")
                .execute(&mut conn)
                .await;
            let _ = conn.close().await;
            return Err(pooler_error(&format!(
                "the lock was taken by backend pid {holder}, the next statement ran on pid {pid}"
            )));
        }
        return Ok(PgTry::Taken(conn, pid));
    }
    let holder = holder_pid(&mut conn).await.ok().flatten();
    let _ = conn.close().await;
    Ok(PgTry::Held(holder))
}

/// The backends three statements ran on: `conn`, a second connection opened meanwhile,
/// then `conn` again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pids {
    first: i32,
    other: i32,
    again: i32,
}

async fn backend_pids(conn: &mut AnyConnection, url: &str) -> Result<Pids> {
    let pid = "SELECT pg_backend_pid()";
    let first: i32 = sqlx::query_scalar(pid).fetch_one(&mut *conn).await?;
    let mut other_conn = AnyConnection::connect(url).await?;
    let other = sqlx::query_scalar(pid).fetch_one(&mut other_conn).await;
    // with the second connection still open, so a pool with a backend to spare shows it
    let again = sqlx::query_scalar(pid).fetch_one(&mut *conn).await;
    let _ = other_conn.close().await;
    Ok(Pids {
        first,
        other: other?,
        again: again?,
    })
}

/// Whether the backends of [`backend_pids`] give away a transaction-mode pooler (T-108),
/// best effort: two direct connections never share a backend and one keeps its own, while
/// such a pooler hands each statement to a free backend (PgBouncer the one used last).
fn pooled(p: &Pids) -> Option<String> {
    if p.other == p.first {
        Some(format!(
            "a second connection ran on the same backend, pid {}",
            p.first
        ))
    } else if p.again != p.first {
        Some(format!(
            "one connection's statements ran on backends {} and {}",
            p.first, p.again
        ))
    } else {
        None
    }
}

/// The backend pid holding the guard of the store `conn` is on.
async fn holder_pid(conn: &mut AnyConnection) -> Result<Option<i32>> {
    Ok(sqlx::query_scalar(
        "SELECT pid FROM pg_locks WHERE locktype = 'advisory' AND granted AND objsubid = 2 \
         AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND classid::text::int = $1 AND objid::text::bigint = \
         (hashtext(current_schema())::bigint & 4294967295) LIMIT 1",
    )
    .bind(GUARD_CLASS)
    .fetch_optional(conn)
    .await?)
}

/// The backend pid holding the PostgreSQL guard of the store at `url`, if any.
pub async fn postgres_holder(url: &str) -> Result<Option<i32>> {
    sqlx::any::install_default_drivers();
    let mut conn = AnyConnection::connect(url).await?;
    let holder = holder_pid(&mut conn).await;
    let _ = conn.close().await;
    holder
}

fn held_message(holder: Option<i32>) -> String {
    let by = holder.map_or(String::new(), |p| format!(" (backend pid {p})"));
    format!("the PostgreSQL advisory lock ({GUARD_CLASS}, schema) is held{by}")
}

/// The host and port of a `postgres://` URL, the userinfo left out.
fn host_port(url: &str) -> Option<(&str, Option<u16>)> {
    let rest = url.split_once("://")?.1;
    // the last `@` ends the userinfo, even with an unencoded `/`, `?` or `#` in a password
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    // `[::1]:5432`: the port follows the bracket
    let colon = authority.rfind(']').map_or(0, |i| i + 1);
    match authority[colon..].find(':') {
        Some(i) => Some((
            &authority[..colon + i],
            authority[colon + i + 1..].parse().ok(),
        )),
        None => Some((authority, None)),
    }
}

/// A transaction-mode connection pooler known by its address: Neon's `-pooler` hosts and
/// Supabase's pooler on port 6543. Such a pooler runs each transaction on whichever
/// backend is free, so a session advisory lock taken through it stays on a backend other
/// clients share and guards nothing (T-108). Public so `service install` refuses one before
/// writing it to the env file (T-116).
pub fn known_pooler(url: &str) -> Option<&'static str> {
    let (host, port) = host_port(url)?;
    let host = host.to_ascii_lowercase();
    let first = host.split('.').next().unwrap_or_default();
    if first.ends_with("-pooler") {
        return Some("a Neon pooled host (`-pooler` in the host name)");
    }
    if host.ends_with(".pooler.supabase.com") && port == Some(6543) {
        return Some("Supabase's transaction pooler (port 6543)");
    }
    None
}

fn pooler_error(what: &str) -> DbError {
    DbError::Sqlx(sqlx::Error::Configuration(
        format!(
            "the database URL goes through a transaction-mode connection pooler ({what}): \
             the single-writer guard holds a session advisory lock and needs a direct \
             connection. Use the direct (unpooled) connection string; on Neon, the host \
             without `-pooler`. (A self-hosted server whose host name ends in `-pooler` \
             is treated the same way; connect by IP address or another name.)"
        )
        .into(),
    ))
}

async fn lock_postgres(url: &str) -> Result<TryAcquire> {
    if let Some(what) = known_pooler(url) {
        return Err(pooler_error(what));
    }
    Ok(match try_postgres(url).await? {
        PgTry::Taken(conn, pid) => TryAcquire::Taken(WriterGuard {
            held: Held::Postgres(PgHeld {
                url: url.to_string(),
                conn: Some(conn),
                pid,
            }),
        }),
        PgTry::Held(holder) => TryAcquire::Held(held_message(holder)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_urls_name_their_file() {
        assert_eq!(
            sqlite_file("sqlite:///tmp/x.db?mode=rwc"),
            Some(PathBuf::from("/tmp/x.db"))
        );
        assert_eq!(
            sqlite_file("sqlite://data/x.db"),
            Some(PathBuf::from("data/x.db"))
        );
        assert_eq!(sqlite_file("sqlite://"), None);
        assert_eq!(sqlite_file("sqlite::memory:"), None);
        assert_eq!(
            lock_path(Path::new("/tmp/x.db")),
            PathBuf::from("/tmp/x.db.lock")
        );
    }

    /// T-108: transaction poolers known by their address are refused before connecting.
    #[test]
    fn known_poolers_are_named() {
        let neon =
            "postgres://u:p@ep-cool-name-a1b2c3-pooler.us-east-2.aws.neon.tech/db?sslmode=require";
        assert!(known_pooler(neon).unwrap().contains("Neon"));
        let direct =
            "postgres://u:p@ep-cool-name-a1b2c3.us-east-2.aws.neon.tech/db?sslmode=require";
        assert_eq!(known_pooler(direct), None);
        let supa = "postgres://u.ref:p@aws-0-us-east-1.pooler.supabase.com:6543/postgres";
        assert!(known_pooler(supa).unwrap().contains("6543"));
        // Supabase's session pooler keeps one backend per client
        let session = "postgres://u.ref:p@aws-0-us-east-1.pooler.supabase.com:5432/postgres";
        assert_eq!(known_pooler(session), None);
        assert_eq!(
            known_pooler("postgres://silvermere:x@postgres:5432/silvermere"),
            None
        );
        assert_eq!(known_pooler("postgres://localhost/db"), None);
        // a password with `@` or `-pooler` in it is not the host
        assert_eq!(
            known_pooler("postgres://u:a@b-pooler@db.example.com/x"),
            None
        );
        assert_eq!(
            host_port("postgres://[::1]:5433/x"),
            Some(("[::1]", Some(5433)))
        );
        assert_eq!(host_port("postgres://[::1]/x"), Some(("[::1]", None)));
        assert_eq!(
            host_port("postgres://u:a/b?c#d@db-pooler.example.com:6432/x"),
            Some(("db-pooler.example.com", Some(6432)))
        );
    }

    #[test]
    fn backends_give_a_pooler_away() {
        let p = |first, other, again| {
            pooled(&Pids {
                first,
                other,
                again,
            })
        };
        assert_eq!(p(10, 11, 10), None);
        assert!(p(10, 10, 10).unwrap().contains("same backend"));
        assert!(p(10, 11, 11).unwrap().contains("backends 10 and 11"));
    }

    /// T-108: a pooler found by the probe leaves no lock behind: the guard is free after.
    #[tokio::test]
    async fn a_pooler_found_takes_no_lock() {
        let t = crate::TestDb::from_env().await.expect("test database");
        if t.db().dialect() != Dialect::Postgres {
            return t.close().await;
        }
        let url = t.db().url().to_string();
        let err = match try_postgres_with(&url, |_| Some("as the test says".into())).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("the probe fired: no guard"),
        };
        assert!(err.contains("as the test says"), "{err}");
        let mut c = AnyConnection::connect(&url).await.unwrap();
        assert_eq!(holder_pid(&mut c).await.unwrap(), None, "no lock left");
        let _ = c.close().await;
        match try_postgres(&url).await.unwrap() {
            PgTry::Taken(conn, _) => drop(conn),
            PgTry::Held(h) => panic!("held by {h:?}"),
        }
        t.close().await;
    }

    #[tokio::test]
    async fn a_pooled_url_is_refused() {
        let err = WriterGuard::try_acquire("postgres://u:p@ep-x-pooler.eu.neon.tech/db")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("direct (unpooled) connection"), "{err}");
    }

    fn fast() -> Watchdog {
        Watchdog {
            every: Duration::from_millis(200),
            give_up: Duration::from_secs(5),
        }
    }

    /// T-081: a busy lock with no holder found is tried again, not given up on.
    #[tokio::test]
    async fn held_with_no_holder_is_retried() {
        let t = crate::TestDb::from_env().await.expect("test database");
        if t.db().dialect() != Dialect::Postgres {
            return t.close().await;
        }
        let mut pg = PgHeld {
            url: t.db().url().to_string(),
            conn: None,
            pid: 0,
        };
        let mut calls = 0;
        let r = retake_with(&mut pg, fast(), |url| {
            calls += 1;
            let first = calls == 1;
            async move {
                if first {
                    Ok(PgTry::Held(None))
                } else {
                    try_postgres(&url).await
                }
            }
        })
        .await;
        assert!(r.is_ok(), "{r:?}");
        assert!(pg.conn.is_some() && pg.pid != 0, "re-taken");
        assert_eq!(calls, 2);
        drop(pg);
        t.close().await;
    }

    /// T-081: a holder that is not the lost session still ends the retries at once.
    #[tokio::test]
    async fn held_by_another_pid_fails_at_once() {
        let mut pg = PgHeld {
            url: String::new(),
            conn: None,
            pid: 7,
        };
        let mut calls = 0;
        let r = retake_with(&mut pg, fast(), |_| {
            calls += 1;
            async { Ok(PgTry::Held(Some(8))) }
        })
        .await;
        assert!(matches!(r, Err(GuardLost::HeldElsewhere(_))), "{r:?}");
        assert_eq!(calls, 1);
    }
}
