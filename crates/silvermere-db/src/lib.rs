//! sqlx schema, migrations, repositories, persistence worker.
//!
//! Persistence for SQLite (the default) and PostgreSQL, selected at runtime by connection
//! URL. Holds the per-backend migrations, the repositories the engine's persistence
//! requests are applied through, and the append-only events table. Nothing above this
//! crate sees a Btrieve record: the repositories exchange the record structs of
//! `silvermere-types` and the state structs defined here.
//!
//! - [`Db`]: a connection pool plus its [`Dialect`]; `Db::connect(url)` then
//!   `db.migrate()`. `db.rooms()`, `db.characters()`, ... are the repositories
//!   ([`repo`]), one per table group, each a set of named lookups that replace one
//!   Btrieve call site family.
//! - [`Persist`]: the write-behind worker. The engine sends typed [`Write`]s through a
//!   channel; the worker batches them into short transactions, retries on failure and
//!   reports [`PersistStatus`]. `flush().await` returns once the queue is empty.
//! - [`WriterGuard`]: the single-writer guard `serve` holds (T-072), a lock file on
//!   SQLite and an advisory lock on PostgreSQL.
//! - [`Events`]: the append-only `events` table with `append` and `prune`.
//! - [`TestDb`]: a throwaway database for tests (a temp SQLite file by default, a
//!   private schema on the PostgreSQL named by `SILVERMERE_DATABASE_URL`).
//!
//! Queries are runtime-checked (`sqlx::query`, bound by position) on the `Any` driver
//! so one query text serves both backends; there is no `sqlx prepare` offline data.
//! The portable subset is: `$1` placeholders, `INSERT ... ON CONFLICT ... DO UPDATE SET
//! col = excluded.col`, `RETURNING id`, `lower()`. The dialect branches (case-insensitive
//! keys, booleans) live in [`dialect`].

#![forbid(unsafe_code)]

pub mod backup;
pub mod dialect;
pub mod events;
pub mod guard;
pub mod latency;
pub mod persist;
pub mod pool;
pub mod repo;
pub mod world_import;

pub use dialect::{Dialect, drop_ignored_params, redact_url, split_password};
pub use events::{Event, Events};
pub use guard::{GuardLost, TryAcquire, Watchdog, WriterGuard};
pub use persist::{Persist, PersistConfig, PersistStatus, Write};
pub use pool::{Db, TestDb};
pub use world_import::WorldImport;

/// The errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// A unique key already holds this value (Btrieve's duplicate-key error in the original).
    #[error("duplicate key: {0}")]
    Duplicate(String),
    /// The connection URL names no supported backend.
    #[error("unsupported database URL: {0}")]
    UnsupportedUrl(String),
    /// A stored value does not fit the record field it is read into.
    #[error("{table}.{column}: {reason}")]
    Decode {
        /// The table read.
        table: &'static str,
        /// The column read.
        column: String,
        /// What was wrong.
        reason: String,
    },
    /// The persistence worker has stopped.
    #[error("persistence worker stopped")]
    PersistClosed,
    /// Applying the migrations failed.
    #[error("migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// Any other database error.
    #[error("database: {0}")]
    Sqlx(sqlx::Error),
}

impl From<sqlx::Error> for DbError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Duplicate(db.message().to_string())
            }
            other => Self::Sqlx(other),
        }
    }
}

/// `Result` with [`DbError`].
pub type Result<T, E = DbError> = std::result::Result<T, E>;
