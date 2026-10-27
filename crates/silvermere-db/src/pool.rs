//! The connection pool ([`Db`]) and the throwaway databases tests use ([`TestDb`]).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use sqlx::AnyPool;
use sqlx::any::AnyPoolOptions;
use sqlx::{Connection, Executor};

use crate::dialect::Dialect;
use crate::events::Events;
use crate::repo::accounts::Accounts;
use crate::repo::active_monsters::ActiveMonsters;
use crate::repo::bank_books::BankBooks;
use crate::repo::characters::Characters;
use crate::repo::classes::Classes;
use crate::repo::evil_points::EvilPoints;
use crate::repo::gangs::Gangs;
use crate::repo::item_ownership::ItemOwnerships;
use crate::repo::items::Items;
use crate::repo::messages::Messages;
use crate::repo::monsters::{MonsterStates, Monsters};
use crate::repo::profiles::Profiles;
use crate::repo::races::Races;
use crate::repo::rooms::{RoomStates, Rooms};
use crate::repo::shops::{ShopStocks, Shops};
use crate::repo::spells::Spells;
use crate::repo::text_blocks::{Actions, TextBlocks};
use crate::{DbError, Result};

/// A database: the pool and the dialect of its URL.
///
/// Clone-cheap handles come from wrapping it in an `Arc`. The pool can be closed and
/// reopened in place ([`Db::close`], [`Db::reconnect`]) so a worker holding the `Db`
/// survives an outage.
#[derive(Debug)]
pub struct Db {
    url: String,
    dialect: Dialect,
    pool: RwLock<AnyPool>,
}

impl Db {
    /// Opens a pool on `url` (`sqlite:` or `postgres:`). A SQLite file is created when
    /// missing and opened in WAL mode with `synchronous=NORMAL` and foreign keys on.
    pub async fn connect(url: &str) -> Result<Self> {
        let dialect = Dialect::from_url(url)?;
        let url = normalise_url(dialect, url);
        let pool = open_pool(dialect, &url).await?;
        Ok(Self {
            url,
            dialect,
            pool: RwLock::new(pool),
        })
    }

    /// Opens the pool and applies the migrations.
    pub async fn connect_and_migrate(url: &str) -> Result<Self> {
        let db = Self::connect(url).await?;
        db.migrate().await?;
        Ok(db)
    }

    /// Applies the backend's migrations (`sqlx::migrate!`), a no-op once applied.
    pub async fn migrate(&self) -> Result<()> {
        self.dialect.migrator().run(&self.pool()).await?;
        Ok(())
    }

    /// The backend.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The connection URL as opened.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The current pool (a cheap handle).
    pub fn pool(&self) -> AnyPool {
        self.pool.read().expect("pool lock").clone()
    }

    /// Closes the pool: every later query fails until [`Db::reconnect`].
    pub async fn close(&self) {
        self.pool().close().await;
    }

    /// Opens a fresh pool on the same URL, replacing the current one.
    pub async fn reconnect(&self) -> Result<()> {
        let pool = open_pool(self.dialect, &self.url).await?;
        let old = std::mem::replace(&mut *self.pool.write().expect("pool lock"), pool);
        if !old.is_closed() {
            old.close().await;
        }
        Ok(())
    }

    /// Rooms content: `struct room` minus its live fields.
    pub fn rooms(&self) -> Rooms<'_> {
        Rooms::new(self)
    }
    /// The live part of rooms.
    pub fn room_state(&self) -> RoomStates<'_> {
        RoomStates::new(self)
    }
    /// Items.
    pub fn items(&self) -> Items<'_> {
        Items::new(self)
    }
    /// Known monsters.
    pub fn monsters(&self) -> Monsters<'_> {
        Monsters::new(self)
    }
    /// The live part of known monsters.
    pub fn monster_state(&self) -> MonsterStates<'_> {
        MonsterStates::new(self)
    }
    /// Active monsters.
    pub fn active_monsters(&self) -> ActiveMonsters<'_> {
        ActiveMonsters::new(self)
    }
    /// Spells.
    pub fn spells(&self) -> Spells<'_> {
        Spells::new(self)
    }
    /// Classes.
    pub fn classes(&self) -> Classes<'_> {
        Classes::new(self)
    }
    /// Races.
    pub fn races(&self) -> Races<'_> {
        Races::new(self)
    }
    /// Shops.
    pub fn shops(&self) -> Shops<'_> {
        Shops::new(self)
    }
    /// Live shop stock.
    pub fn shop_stock(&self) -> ShopStocks<'_> {
        ShopStocks::new(self)
    }
    /// Messages (WCCMSG).
    pub fn messages(&self) -> Messages<'_> {
        Messages::new(self)
    }
    /// Text blocks (WCCTEXT).
    pub fn text_blocks(&self) -> TextBlocks<'_> {
        TextBlocks::new(self)
    }
    /// Social actions (WCCACTS).
    pub fn actions(&self) -> Actions<'_> {
        Actions::new(self)
    }
    /// Characters.
    pub fn characters(&self) -> Characters<'_> {
        Characters::new(self)
    }
    /// Character profiles (`permrec`).
    pub fn profiles(&self) -> Profiles<'_> {
        Profiles::new(self)
    }
    /// Evil points (`evilrec`).
    pub fn evil_points(&self) -> EvilPoints<'_> {
        EvilPoints::new(self)
    }
    /// Bank books.
    pub fn bank_books(&self) -> BankBooks<'_> {
        BankBooks::new(self)
    }
    /// Gangs.
    pub fn gangs(&self) -> Gangs<'_> {
        Gangs::new(self)
    }
    /// Limited-item ownership.
    pub fn item_ownership(&self) -> ItemOwnerships<'_> {
        ItemOwnerships::new(self)
    }
    /// Accounts.
    pub fn accounts(&self) -> Accounts<'_> {
        Accounts::new(self)
    }
    /// The events table.
    pub fn events(&self) -> Events<'_> {
        Events::new(self)
    }
}

/// `sqlite:` URLs get `mode=rwc` so the file is created; a bare `sqlite://` means an
/// in-memory database (one connection, see [`open_pool`]).
fn normalise_url(dialect: Dialect, url: &str) -> String {
    match dialect {
        Dialect::Sqlite => {
            if is_memory_sqlite(url) || url.contains("mode=") {
                url.to_string()
            } else if url.contains('?') {
                format!("{url}&mode=rwc")
            } else {
                format!("{url}?mode=rwc")
            }
        }
        Dialect::Postgres => url.to_string(),
    }
}

fn is_memory_sqlite(url: &str) -> bool {
    url == "sqlite://"
        || url == "sqlite:"
        || url.contains(":memory:")
        || url.contains("mode=memory")
}

async fn open_pool(dialect: Dialect, url: &str) -> Result<AnyPool> {
    open_pool_from(AnyPoolOptions::new(), dialect, url).await
}

/// [`open_pool`] from `options`, so tests can shorten the lifetimes it keeps.
///
/// Lifetimes: an in-memory SQLite database lives in its one connection, so that
/// connection is never reaped (no `idle_timeout`, no `max_lifetime`); reaping it would
/// empty the database. A file or PostgreSQL store keeps sqlx's defaults (idle connections
/// closed after 10 minutes, every connection replaced after 30): nothing is lost when a
/// connection goes, and replacing them lets a server-side change (a failover, a rotated
/// password, a restarted PostgreSQL) reach the pool.
async fn open_pool_from(options: AnyPoolOptions, dialect: Dialect, url: &str) -> Result<AnyPool> {
    sqlx::any::install_default_drivers();
    let mut options = options.acquire_timeout(Duration::from_secs(30));
    options = match dialect {
        Dialect::Sqlite => {
            // One connection for an in-memory database (each connection would otherwise
            // see its own empty database); a small pool for a file.
            let max = if is_memory_sqlite(url) {
                options = options.idle_timeout(None).max_lifetime(None);
                1
            } else {
                4
            };
            options.max_connections(max).after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA journal_mode = WAL").await?;
                    conn.execute("PRAGMA synchronous = NORMAL").await?;
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    conn.execute("PRAGMA busy_timeout = 10000").await?;
                    Ok(())
                })
            })
        }
        Dialect::Postgres => options.max_connections(8),
    };
    Ok(options.connect(url).await?)
}

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// A throwaway database for tests: a temp SQLite file by default, or a private database
/// on the PostgreSQL server named by `SILVERMERE_DATABASE_URL`, cloned from a template
/// database migrated once per set of migrations (T-147). Migrated and empty when
/// returned. The SQLite directory is removed on drop; a PostgreSQL database (or schema)
/// is removed by [`TestDb::close`] (CI's server is discarded with the job).
#[derive(Debug)]
pub struct TestDb {
    db: Arc<Db>,
    cleanup: Cleanup,
}

#[derive(Debug)]
enum Cleanup {
    Dir(PathBuf),
    PgSchema {
        base_url: String,
        schema: String,
    },
    PgDatabase {
        base_url: String,
        name: String,
    },
    /// Handed to another `TestDb` ([`TestDb::into_cleanup`]).
    Handed,
}

impl TestDb {
    /// A fresh SQLite database in a temp directory.
    pub async fn sqlite_temp() -> Result<Self> {
        let n = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "silvermere-test-{}-{}-{}",
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).map_err(|e| DbError::Sqlx(sqlx::Error::Io(e)))?;
        let url = format!("sqlite://{}", dir.join("test.db").display());
        let db = Db::connect_and_migrate(&url).await?;
        Ok(Self {
            db: Arc::new(db),
            cleanup: Cleanup::Dir(dir),
        })
    }

    /// A fresh schema on the PostgreSQL at `base_url`.
    pub async fn postgres_schema(base_url: &str) -> Result<Self> {
        let n = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let schema = format!("t_{}_{}", std::process::id(), n);
        sqlx::any::install_default_drivers();
        let mut admin = sqlx::AnyConnection::connect(base_url).await?;
        admin
            .execute(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .await?;
        admin.close().await?;
        let url = schema_url(base_url, &schema);
        let db = Db::connect_and_migrate(&url).await?;
        Ok(Self {
            db: Arc::new(db),
            cleanup: Cleanup::PgSchema {
                base_url: base_url.to_string(),
                schema,
            },
        })
    }

    /// A fresh schema on the PostgreSQL `SILVERMERE_DATABASE_URL` names, reached through
    /// a [`LatencyProxy`](crate::latency::LatencyProxy) that delays each direction by
    /// `one_way` (T-113): a hosted database in another region, on CI's loopback one.
    /// `None` when the URL names no PostgreSQL. The schema is created and dropped
    /// directly; only the returned database's pool goes through the proxy.
    pub async fn postgres_with_latency(
        one_way: std::time::Duration,
    ) -> Result<Option<(Self, crate::latency::LatencyProxy)>> {
        let Ok(base_url) = std::env::var("SILVERMERE_DATABASE_URL") else {
            return Ok(None);
        };
        if !matches!(Dialect::from_url(&base_url), Ok(Dialect::Postgres)) {
            return Ok(None);
        }
        // the database is made directly: only what the test measures pays the delay
        let direct_db = Self::postgres_clone(&base_url).await?;
        let direct = direct_db.url().to_string();
        let cleanup = direct_db.into_cleanup().await;
        let (_, upstream) = crate::latency::redirect_url(&base_url, 0)
            .ok_or_else(|| DbError::UnsupportedUrl(base_url.clone()))?;
        let proxy = crate::latency::LatencyProxy::start(&upstream, one_way)
            .await
            .map_err(|e| DbError::Sqlx(sqlx::Error::Io(e)))?;
        let (via, _) = crate::latency::redirect_url(&direct, proxy.port())
            .ok_or_else(|| DbError::UnsupportedUrl(base_url.clone()))?;
        let db = Db::connect(&via).await?;
        Ok(Some((
            Self {
                db: Arc::new(db),
                cleanup,
            },
            proxy,
        )))
    }

    /// The backend `SILVERMERE_DATABASE_URL` names: a PostgreSQL schema for a
    /// `postgres:` URL, a temp SQLite file otherwise (including when unset or `sqlite://`).
    pub async fn from_env() -> Result<Self> {
        match std::env::var("SILVERMERE_DATABASE_URL") {
            Ok(url) if matches!(Dialect::from_url(&url), Ok(Dialect::Postgres)) => {
                Self::postgres_clone(&url).await
            }
            _ => Self::sqlite_temp().await,
        }
    }

    /// [`TestDb::from_env`], but on PostgreSQL a fresh schema that this call migrates
    /// instead of a clone of the migrated template: for the tests of the migrations
    /// themselves.
    pub async fn from_env_migrating() -> Result<Self> {
        match std::env::var("SILVERMERE_DATABASE_URL") {
            Ok(url) if matches!(Dialect::from_url(&url), Ok(Dialect::Postgres)) => {
                Self::postgres_schema(&url).await
            }
            _ => Self::sqlite_temp().await,
        }
    }

    /// A fresh database on the PostgreSQL server at `base_url`, cloned from the template
    /// [`pg_template`] keeps: a file copy on the server instead of running the migrations
    /// (and waiting for sqlx's migration lock, one per database, which every test held in
    /// turn). Falls back to [`TestDb::postgres_schema`] when the role may not create
    /// databases.
    pub async fn postgres_clone(base_url: &str) -> Result<Self> {
        match Self::try_postgres_clone(base_url).await {
            Ok(t) => Ok(t),
            Err(e) => {
                tracing::warn!(%e, "no template clone; migrating a schema instead");
                Self::postgres_schema(base_url).await
            }
        }
    }

    async fn try_postgres_clone(base_url: &str) -> Result<Self> {
        let template = pg_template(base_url).await?;
        let name = pg_test_name();
        sqlx::any::install_default_drivers();
        let mut admin = sqlx::AnyConnection::connect(base_url).await?;
        let created = admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {name} TEMPLATE {template}"
            )))
            .await;
        admin.close().await?;
        created?;
        let db = Db::connect(&database_url(base_url, &name)).await?;
        Ok(Self {
            db: Arc::new(db),
            cleanup: Cleanup::PgDatabase {
                base_url: base_url.to_string(),
                name,
            },
        })
    }

    /// The database.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// Closes the pool and removes the database (file or schema).
    pub async fn close(self) {
        self.db.close().await;
        let (base_url, drop) = match &self.cleanup {
            Cleanup::Dir(_) | Cleanup::Handed => return,
            Cleanup::PgSchema { base_url, schema } => {
                (base_url, format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            }
            Cleanup::PgDatabase { base_url, name } => (
                base_url,
                format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
            ),
        };
        if let Ok(mut admin) = sqlx::AnyConnection::connect(base_url).await {
            let _ = admin.execute(sqlx::AssertSqlSafe(drop)).await;
            let _ = admin.close().await;
        }
    }

    /// Closes this handle's pool and hands over what removes the database, which stays.
    async fn into_cleanup(mut self) -> Cleanup {
        self.db.close().await;
        std::mem::replace(&mut self.cleanup, Cleanup::Handed)
    }
}

impl std::ops::Deref for TestDb {
    type Target = Db;

    fn deref(&self) -> &Db {
        &self.db
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if let Cleanup::Dir(dir) = &self.cleanup {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// `base_url` with `search_path` set to `schema` through the libpq `options` parameter.
fn schema_url(base_url: &str, schema: &str) -> String {
    let sep = if base_url.contains('?') { '&' } else { '?' };
    format!("{base_url}{sep}options=-c%20search_path%3D{schema}")
}

/// The template database [`TestDb::postgres_clone`] copies: migrated once for each set of
/// migrations (the name carries their versions and checksums), by whichever test process
/// gets there first while the others wait on an advisory lock. Built under another name
/// and renamed when complete, so a builder that dies leaves no half-migrated template.
async fn pg_template(base_url: &str) -> Result<String> {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for m in Dialect::Postgres.migrator().iter() {
        for b in m.version.to_le_bytes().iter().chain(m.checksum.iter()) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let template = format!("silvermere_template_{h:016x}");
    sqlx::any::install_default_drivers();
    let mut admin = sqlx::AnyConnection::connect(base_url).await?;
    let exists = format!("SELECT 1 AS one FROM pg_database WHERE datname = '{template}'");
    if sqlx::query(sqlx::AssertSqlSafe(exists.clone()))
        .fetch_optional(&mut admin)
        .await?
        .is_none()
    {
        // one key for every template build on the server
        admin.execute("SELECT pg_advisory_lock(7147147147)").await?;
        let built = async {
            if sqlx::query(sqlx::AssertSqlSafe(exists))
                .fetch_optional(&mut admin)
                .await?
                .is_none()
            {
                let building = format!("{template}_b{}", std::process::id());
                admin
                    .execute(sqlx::AssertSqlSafe(format!(
                        "DROP DATABASE IF EXISTS {building} WITH (FORCE)"
                    )))
                    .await?;
                admin
                    .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {building}")))
                    .await?;
                Db::connect_and_migrate(&database_url(base_url, &building))
                    .await?
                    .close()
                    .await;
                admin
                    .execute(sqlx::AssertSqlSafe(format!(
                        "ALTER DATABASE {building} RENAME TO {template}"
                    )))
                    .await?;
            }
            Ok::<_, DbError>(())
        }
        .await;
        let _ = admin.execute("SELECT pg_advisory_unlock(7147147147)").await;
        built?;
    }
    admin.close().await?;
    Ok(template)
}

/// A database name no other test process or earlier run uses.
fn pg_test_name() -> String {
    let n = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("t_{}_{n}_{nanos}", std::process::id())
}

/// `base_url` naming the database `name` instead of its own.
fn database_url(base_url: &str, name: &str) -> String {
    let (scheme, rest) = base_url.split_once("://").unwrap_or(("postgres", base_url));
    let (before_query, query) = match rest.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (rest, None),
    };
    let authority = before_query
        .split_once('/')
        .map_or(before_query, |(a, _)| a);
    match query {
        Some(q) => format!("{scheme}://{authority}/{name}?{q}"),
        None => format!("{scheme}://{authority}/{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_urls_get_create_mode() {
        assert_eq!(
            normalise_url(Dialect::Sqlite, "sqlite:///tmp/x.db"),
            "sqlite:///tmp/x.db?mode=rwc"
        );
        assert_eq!(
            normalise_url(Dialect::Sqlite, "sqlite:///tmp/x.db?mode=ro"),
            "sqlite:///tmp/x.db?mode=ro"
        );
        assert_eq!(normalise_url(Dialect::Sqlite, "sqlite://"), "sqlite://");
        assert_eq!(
            normalise_url(Dialect::Postgres, "postgres://u@h/d"),
            "postgres://u@h/d"
        );
    }

    #[test]
    fn database_url_names_the_database() {
        assert_eq!(
            database_url("postgres://u:p@h/d", "t_1"),
            "postgres://u:p@h/t_1"
        );
        assert_eq!(
            database_url("postgres://u:p@h:5432/d?sslmode=disable", "t_1"),
            "postgres://u:p@h:5432/t_1?sslmode=disable"
        );
        assert_eq!(database_url("postgres://u@h", "t_1"), "postgres://u@h/t_1");
    }

    #[test]
    fn schema_url_sets_search_path() {
        assert_eq!(
            schema_url("postgres://u:p@h/d", "t_1_2"),
            "postgres://u:p@h/d?options=-c%20search_path%3Dt_1_2"
        );
        assert_eq!(
            schema_url("postgres://u:p@h/d?sslmode=disable", "t_1_2"),
            "postgres://u:p@h/d?sslmode=disable&options=-c%20search_path%3Dt_1_2"
        );
    }

    /// T-073: the reaper never takes an in-memory database's one connection, so the
    /// database outlives any idle timeout or lifetime the pool was given.
    #[tokio::test]
    async fn an_in_memory_store_survives_past_the_idle_timeout() {
        let short = || {
            AnyPoolOptions::new()
                .idle_timeout(Duration::from_millis(300))
                .max_lifetime(Duration::from_millis(300))
        };
        let pool = open_pool_from(short(), Dialect::Sqlite, "sqlite://")
            .await
            .unwrap();
        assert_eq!(pool.options().get_idle_timeout(), None);
        assert_eq!(pool.options().get_max_lifetime(), None);
        pool.execute("CREATE TABLE t (n INTEGER)").await.unwrap();
        pool.execute("INSERT INTO t VALUES (7)").await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let n: i64 = sqlx::query_scalar("SELECT n FROM t")
            .fetch_one(&pool)
            .await
            .expect("the in-memory table is still there");
        assert_eq!(n, 7);
        pool.close().await;

        // a file store keeps the lifetimes it was given
        let dir = std::env::temp_dir().join(format!("silvermere-t073-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let url = normalise_url(
            Dialect::Sqlite,
            &format!("sqlite://{}", dir.join("x.db").display()),
        );
        let pool = open_pool_from(short(), Dialect::Sqlite, &url)
            .await
            .unwrap();
        assert_eq!(
            pool.options().get_idle_timeout(),
            Some(Duration::from_millis(300))
        );
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
