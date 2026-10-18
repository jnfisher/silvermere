//! The two SQL dialects and every statement fragment that differs between them.
//!
//! Queries are written once in the portable subset; the three things that differ are
//! here: the migration set, case-insensitive key comparison (`COLLATE NOCASE` columns on
//! SQLite, `lower()` expression indexes on PostgreSQL) and booleans (INTEGER 0/1 on
//! SQLite, since the `Any` driver cannot decode a SQLite BOOLEAN column, BOOLEAN on
//! PostgreSQL).

use sqlx::Row;
use sqlx::any::AnyRow;
use sqlx::migrate::Migrator;

use crate::DbError;

static SQLITE_MIGRATIONS: Migrator = sqlx::migrate!("../../migrations/sqlite");
static POSTGRES_MIGRATIONS: Migrator = sqlx::migrate!("../../migrations/postgres");

/// The backend a connection URL names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `sqlite:` URLs (the default backend).
    Sqlite,
    /// `postgres:` / `postgresql:` URLs.
    Postgres,
}

impl Dialect {
    /// The dialect of a connection URL.
    pub fn from_url(url: &str) -> Result<Self, DbError> {
        if url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Self::Postgres)
        } else {
            Err(DbError::UnsupportedUrl(url.to_string()))
        }
    }

    /// The migrations of this backend (`migrations/<dialect>/`).
    pub fn migrator(self) -> &'static Migrator {
        match self {
            Self::Sqlite => &SQLITE_MIGRATIONS,
            Self::Postgres => &POSTGRES_MIGRATIONS,
        }
    }

    /// The backend's name as sqlx reports it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sqlite => "SQLite",
            Self::Postgres => "PostgreSQL",
        }
    }

    /// A case-insensitive key column in a comparison or `ORDER BY`: the column itself on
    /// SQLite (declared `COLLATE NOCASE`), `lower(column)` on PostgreSQL (which has the
    /// matching expression index).
    pub fn ci_column(self, column: &str) -> String {
        match self {
            Self::Sqlite => column.to_string(),
            Self::Postgres => format!("lower({column})"),
        }
    }

    /// A bound parameter compared against a case-insensitive key column.
    pub fn ci_param(self, placeholder: &str) -> String {
        match self {
            Self::Sqlite => placeholder.to_string(),
            Self::Postgres => format!("lower({placeholder})"),
        }
    }

    /// `column = $n` on a case-insensitive key (ACS ALLCAPS in the original).
    pub fn ci_eq(self, column: &str, placeholder: &str) -> String {
        format!(
            "{} = {}",
            self.ci_column(column),
            self.ci_param(placeholder)
        )
    }

    /// `column >= $n` on a case-insensitive key (the `agebtv` range read).
    pub fn ci_ge(self, column: &str, placeholder: &str) -> String {
        format!(
            "{} >= {}",
            self.ci_column(column),
            self.ci_param(placeholder)
        )
    }

    /// Reads a boolean column: BOOLEAN on PostgreSQL, INTEGER 0/1 on SQLite.
    pub fn get_bool(self, row: &AnyRow, column: &str) -> Result<bool, sqlx::Error> {
        match self {
            Self::Sqlite => row.try_get::<i64, _>(column).map(|v| v != 0),
            Self::Postgres => row.try_get::<bool, _>(column),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_picks_the_dialect() {
        assert_eq!(Dialect::from_url("sqlite://x.db").unwrap(), Dialect::Sqlite);
        assert_eq!(
            Dialect::from_url("postgres://u:p@h/d").unwrap(),
            Dialect::Postgres
        );
        assert_eq!(
            Dialect::from_url("postgresql://u:p@h/d").unwrap(),
            Dialect::Postgres
        );
        assert!(matches!(
            Dialect::from_url("mysql://x"),
            Err(DbError::UnsupportedUrl(_))
        ));
    }

    #[test]
    fn case_insensitive_fragments() {
        assert_eq!(Dialect::Sqlite.ci_eq("userid", "$1"), "userid = $1");
        assert_eq!(
            Dialect::Postgres.ci_eq("userid", "$1"),
            "lower(userid) = lower($1)"
        );
        assert_eq!(
            Dialect::Postgres.ci_ge("firstname", "$2"),
            "lower(firstname) >= lower($2)"
        );
    }

    #[test]
    fn both_migration_sets_have_the_same_versions() {
        let sqlite: Vec<_> = SQLITE_MIGRATIONS.iter().map(|m| m.version).collect();
        let postgres: Vec<_> = POSTGRES_MIGRATIONS.iter().map(|m| m.version).collect();
        assert_eq!(sqlite, postgres);
        // 0001 the schema, 0002 world_import (T-109)
        assert_eq!(sqlite, vec![1, 2]);
    }
}

/// `url` with its password replaced by `****`, for messages and logs (T-108): the
/// userinfo's `user:password@` and a `password=` query parameter.
pub fn redact_url(url: &str) -> String {
    let (bare, password) = split_password(url);
    match password {
        None => bare,
        Some(_) => mark_password(url),
    }
}

/// `url` without its password, and the password (percent-decoded): for a tool that takes
/// it from the environment instead (`PGPASSWORD` for `pg_dump`), so it stays off the
/// command line (T-108). The userinfo ends at the last `@`, so a password may hold an
/// unencoded `/`, `?` or `#`; an `@` after the host must be percent-encoded.
pub fn split_password(url: &str) -> (String, Option<String>) {
    let mut out = url.to_string();
    let mut password = None;
    if let Some(start) = out.find("://").map(|i| i + 3)
        && let Some(at) = out[start..].rfind('@').map(|i| start + i)
        && let Some(colon) = out[start..at].find(':').map(|i| start + i)
    {
        password = Some(percent_decode(&out[colon + 1..at]));
        out.replace_range(colon..at, "");
    }
    let host_end = out.rfind('@').map_or(0, |i| i + 1);
    if let Some(q) = out[host_end..].find('?').map(|i| host_end + i) {
        let mut kept = Vec::new();
        for kv in out[q + 1..].split('&') {
            match kv.split_once('=') {
                Some((k, v)) if k.eq_ignore_ascii_case("password") => {
                    password = Some(percent_decode(v));
                }
                _ => kept.push(kv),
            }
        }
        let query = kept.join("&");
        out = format!(
            "{}{}{query}",
            &out[..q],
            if query.is_empty() { "" } else { "?" }
        );
    }
    (out, password)
}

/// Connection parameters of a PostgreSQL URL that silvermere's driver does not know: it
/// ignores them and warns on every connection. Neon's copied URL ends in
/// `channel_binding=require`. The driver never did channel binding, so dropping the parameter
/// changes nothing but the warning (T-108).
const IGNORED_PARAMS: &[&str] = &["channel_binding"];

/// A `postgres:` URL without the parameters in [`IGNORED_PARAMS`]; other URLs unchanged.
pub fn drop_ignored_params(url: &str) -> String {
    if !url.starts_with("postgres") {
        return url.to_string();
    }
    let host_end = url.rfind('@').map_or(0, |i| i + 1);
    let Some(q) = url[host_end..].find('?').map(|i| host_end + i) else {
        return url.to_string();
    };
    let kept: Vec<&str> = url[q + 1..]
        .split('&')
        .filter(|kv| {
            let key = kv.split_once('=').map_or(*kv, |(k, _)| k);
            !IGNORED_PARAMS.iter().any(|p| key.eq_ignore_ascii_case(p))
        })
        .collect();
    match kept.join("&") {
        query if query.is_empty() => url[..q].to_string(),
        query => format!("{}?{query}", &url[..q]),
    }
}

/// `url` with the password itself replaced by `****`, everything else as it was.
fn mark_password(url: &str) -> String {
    let mut out = url.to_string();
    if let Some(start) = out.find("://").map(|i| i + 3)
        && let Some(at) = out[start..].rfind('@').map(|i| start + i)
        && let Some(colon) = out[start..at].find(':').map(|i| start + i)
    {
        out.replace_range(colon + 1..at, "****");
    }
    let host_end = out.rfind('@').map_or(0, |i| i + 1);
    if let Some(q) = out[host_end..].find('?').map(|i| host_end + i) {
        let query: Vec<String> = out[q + 1..]
            .split('&')
            .map(|kv| match kv.split_once('=') {
                Some((k, _)) if k.eq_ignore_ascii_case("password") => format!("{k}=****"),
                _ => kv.to_string(),
            })
            .collect();
        out = format!("{}?{}", &out[..q], query.join("&"));
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod redact_tests {
    use super::redact_url;

    #[test]
    fn passwords_never_show() {
        assert_eq!(
            redact_url("postgres://sm:s3cr@t:x@ep-a.neon.tech/db?sslmode=require"),
            "postgres://sm:****@ep-a.neon.tech/db?sslmode=require"
        );
        assert_eq!(
            redact_url("postgres://h/db?user=u&password=pw&sslmode=require"),
            "postgres://h/db?user=u&password=****&sslmode=require"
        );
        assert_eq!(redact_url("postgres://sm@h/db"), "postgres://sm@h/db");
        assert_eq!(redact_url("sqlite://data/x.db"), "sqlite://data/x.db");
        assert_eq!(redact_url("sqlite://"), "sqlite://");
        assert_eq!(
            redact_url("postgres://u:a/b?c#d@h:5432/db?sslmode=require"),
            "postgres://u:****@h:5432/db?sslmode=require"
        );
    }

    #[test]
    fn neons_channel_binding_is_dropped() {
        use super::drop_ignored_params as d;
        assert_eq!(
            d("postgres://u:p@ep-a.neon.tech/db?sslmode=require&channel_binding=require"),
            "postgres://u:p@ep-a.neon.tech/db?sslmode=require"
        );
        assert_eq!(
            d("postgres://u:p@h/db?channel_binding=require&sslmode=require"),
            "postgres://u:p@h/db?sslmode=require"
        );
        assert_eq!(
            d("postgres://u:p@h/db?channel_binding=require"),
            "postgres://u:p@h/db"
        );
        assert_eq!(d("postgres://u:p@h/db"), "postgres://u:p@h/db");
        // a password holding the text is not a parameter
        assert_eq!(
            d("postgres://u:x?channel_binding=1@h/db?sslmode=require"),
            "postgres://u:x?channel_binding=1@h/db?sslmode=require"
        );
        assert_eq!(
            d("sqlite://data/x.db?channel_binding=1"),
            "sqlite://data/x.db?channel_binding=1"
        );
    }

    #[test]
    fn the_password_comes_apart() {
        use super::split_password;
        assert_eq!(
            split_password("postgres://sm:p%40ss%2Fw@ep-a.neon.tech/db?sslmode=require"),
            (
                "postgres://sm@ep-a.neon.tech/db?sslmode=require".into(),
                Some("p@ss/w".into())
            )
        );
        assert_eq!(
            split_password("postgres://h/db?user=u&password=pw&sslmode=require"),
            (
                "postgres://h/db?user=u&sslmode=require".into(),
                Some("pw".into())
            )
        );
        assert_eq!(
            split_password("postgres://h/db?password=pw"),
            ("postgres://h/db".into(), Some("pw".into()))
        );
        assert_eq!(
            split_password("postgres://sm@h/db"),
            ("postgres://sm@h/db".into(), None)
        );
        assert_eq!(
            split_password("postgres://u:a/b?c#d@h/db"),
            ("postgres://u@h/db".into(), Some("a/b?c#d".into()))
        );
    }
}
