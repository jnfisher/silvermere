//! Accounts: `accounts` and `account_roles`. These writes are awaited by the engine, never queued.

use jiff::Timestamp;
use silvermere_types::ids::AccountId;

use crate::pool::Db;
use crate::repo::{Args, Conn, Get, delete_children, insert_many, insert_sql, select_sql};
use crate::{DbError, Result};

/// An account's columns, without its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Login name, unique case-insensitive.
    pub username: String,
    /// argon2id PHC string, empty until set.
    pub password_hash: String,
    /// Optional contact address.
    pub email: Option<String>,
    /// When the account was created.
    pub created: Timestamp,
    /// The last successful login, if any.
    pub last_login: Option<Timestamp>,
    /// Account flags (locked, must change password; defined by T-013).
    pub flags: u32,
    /// Roles: `player`, `sysop`, `gameop`, ...
    pub roles: Vec<String>,
}

impl Account {
    /// A plain player account created at `created`.
    pub fn new(username: &str, password_hash: &str, created: Timestamp) -> Self {
        Self {
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            email: None,
            created,
            last_login: None,
            flags: 0,
            roles: vec!["player".to_string()],
        }
    }
}

/// A stored account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    /// The primary key.
    pub id: AccountId,
    /// The columns.
    pub account: Account,
}

const COLS: &[&str] = &[
    "username",
    "password_hash",
    "email",
    "created",
    "last_login",
    "flags",
];

fn args(a: &Account) -> Args {
    let mut args = Args::new();
    args.add(&a.username)
        .add(&a.password_hash)
        .add(a.email.as_deref())
        .add(a.created.to_string())
        .add(a.last_login.map(|t| t.to_string()))
        .add(a.flags);
    args
}

fn parse_ts(table: &'static str, column: &str, text: &str) -> Result<Timestamp> {
    text.parse().map_err(|e: jiff::Error| DbError::Decode {
        table,
        column: column.to_string(),
        reason: e.to_string(),
    })
}

async fn write_roles(conn: &mut Conn, id: i64, roles: &[String]) -> Result<()> {
    delete_children(conn, "account_roles", "account_id", id).await?;
    let mut seen: Vec<&String> = Vec::new();
    let rows: Vec<Args> = roles
        .iter()
        .filter(|r| {
            if seen.contains(r) {
                false
            } else {
                seen.push(r);
                true
            }
        })
        .map(|r| {
            let mut a = Args::new();
            a.add(id).add(r);
            a
        })
        .collect();
    insert_many(conn, "account_roles", &["account_id", "role"], rows).await
}

async fn load(conn: &mut Conn, where_: &str, args: Args) -> Result<Option<AccountRow>> {
    let mut cols = vec!["id"];
    cols.extend_from_slice(COLS);
    let row = args
        .query(select_sql("accounts", &cols, where_))
        .fetch_optional(&mut *conn)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let g = Get::new(&row, "accounts");
    let id = g.i64("id")?;
    let mut account = Account {
        username: g.text("username")?,
        password_hash: g.text("password_hash")?,
        email: g.opt_text("email")?,
        created: parse_ts("accounts", "created", &g.text("created")?)?,
        last_login: g
            .opt_text("last_login")?
            .map(|t| parse_ts("accounts", "last_login", &t))
            .transpose()?,
        flags: g.int("flags")?,
        roles: Vec::new(),
    };
    let mut a = Args::new();
    a.add(id);
    for row in a
        .query("SELECT role FROM account_roles WHERE account_id = $1 ORDER BY role".to_string())
        .fetch_all(&mut *conn)
        .await?
    {
        account
            .roles
            .push(Get::new(&row, "account_roles").text("role")?);
    }
    Ok(Some(AccountRow {
        id: AccountId(id),
        account,
    }))
}

/// Accounts.
#[derive(Debug, Clone, Copy)]
pub struct Accounts<'a> {
    db: &'a Db,
}

impl<'a> Accounts<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The account named `username`, case-insensitive.
    pub async fn by_username(&self, username: &str) -> Result<Option<AccountRow>> {
        let mut conn = self.db.pool().acquire().await?;
        let mut a = Args::new();
        a.add(username);
        let where_ = format!("WHERE {}", self.db.dialect().ci_eq("username", "$1"));
        load(&mut conn, &where_, a).await
    }

    /// The account with `id`.
    pub async fn get(&self, id: AccountId) -> Result<Option<AccountRow>> {
        let mut conn = self.db.pool().acquire().await?;
        let mut a = Args::new();
        a.add(id.0);
        load(&mut conn, "WHERE id = $1", a).await
    }

    /// Every account, by id (`silvermere account list`).
    pub async fn all(&self) -> Result<Vec<AccountRow>> {
        let mut conn = self.db.pool().acquire().await?;
        let ids = Args::new()
            .query("SELECT id FROM accounts ORDER BY id".to_string())
            .fetch_all(&mut *conn)
            .await?
            .iter()
            .map(|row| Get::new(row, "accounts").i64("id"))
            .collect::<Result<Vec<i64>>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let mut a = Args::new();
            a.add(id);
            if let Some(row) = load(&mut conn, "WHERE id = $1", a).await? {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// Whether an account named `username` exists, case-insensitive (`accbb` cross-reference).
    pub async fn exists(&self, username: &str) -> Result<bool> {
        let mut a = Args::new();
        a.add(username);
        let row = a
            .query(format!(
                "SELECT id FROM accounts WHERE {}",
                self.db.dialect().ci_eq("username", "$1")
            ))
            .fetch_optional(&self.db.pool())
            .await?;
        Ok(row.is_some())
    }

    /// Inserts `account`; `DbError::Duplicate` when the username exists in any case.
    pub async fn insert(&self, account: &Account) -> Result<AccountId> {
        let mut tx = self.db.pool().begin().await?;
        let row = args(account)
            .query(format!("{} RETURNING id", insert_sql("accounts", COLS)))
            .fetch_one(&mut *tx)
            .await?;
        let id = Get::new(&row, "accounts").i64("id")?;
        write_roles(&mut tx, id, &account.roles).await?;
        tx.commit().await?;
        Ok(AccountId(id))
    }

    /// Rewrites the account `id`; `false` when there is no such account.
    pub async fn update(&self, id: AccountId, account: &Account) -> Result<bool> {
        let mut tx = self.db.pool().begin().await?;
        let sets: Vec<String> = COLS
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c} = ${}", i + 1))
            .collect();
        let mut a = args(account);
        a.add(id.0);
        let done = a
            .query(format!(
                "UPDATE accounts SET {} WHERE id = ${}",
                sets.join(", "),
                COLS.len() + 1
            ))
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
        write_roles(&mut tx, id.0, &account.roles).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Deletes account `id` and everything keyed to it (its character, profile, evil
    /// points and bank books, as the game cleared them when a BBS account went).
    pub async fn delete(&self, id: AccountId) -> Result<bool> {
        let mut a = Args::new();
        a.add(id.0);
        let done = a
            .query("DELETE FROM accounts WHERE id = $1".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}
