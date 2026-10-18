//! Evil points: `evil_points`, the genbb `evilrec` ("WCC MAJOR MUD EVIL") that keeps a
//! player's evil points across logoffs: saved when the player leaves, read back on return.

use silvermere_types::ids::AccountId;
use silvermere_types::records::EvilRecord;

use crate::Result;
use crate::pool::Db;
use crate::repo::{Args, Conn, Get, delete_children_in, last_per_key, upsert_many, upsert_sql};

const COLS: &[&str] = &["account_id", "userid", "evil"];

fn args(account: AccountId, e: &EvilRecord) -> Args {
    let mut a = Args::new();
    a.add(account.0).add(&e.userid).add(e.evil);
    a
}

/// [`put`] for many accounts in multi-row statements (T-113). The accounts.
pub(crate) async fn put_many_of(conn: &mut Conn, rows: &[(AccountId, &EvilRecord)]) -> Result<()> {
    // a repeated key keeps its last record, as one upsert per write would
    let rows = last_per_key(rows, |(a, _)| a.0)
        .into_iter()
        .map(|(a, e)| args(*a, e))
        .collect();
    upsert_many(conn, "evil_points", COLS, &["account_id"], rows, None).await?;
    Ok(())
}

/// [`delete`] for many accounts in one statement (T-113).
pub(crate) async fn delete_many(conn: &mut Conn, accounts: &[AccountId]) -> Result<()> {
    let ids: Vec<i64> = accounts.iter().map(|a| a.0).collect();
    delete_children_in(conn, "evil_points", "account_id", &ids).await?;
    Ok(())
}

/// Upserts the evil points of `account`.
pub(crate) async fn put(conn: &mut Conn, account: AccountId, e: &EvilRecord) -> Result<()> {
    args(account, e)
        .query(upsert_sql("evil_points", COLS, &["account_id"]))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Deletes the evil points of `account`.
pub(crate) async fn delete(conn: &mut Conn, account: AccountId) -> Result<bool> {
    let mut a = Args::new();
    a.add(account.0);
    let done = a
        .query("DELETE FROM evil_points WHERE account_id = $1".to_string())
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Evil points kept over a death.
#[derive(Debug, Clone, Copy)]
pub struct EvilPoints<'a> {
    db: &'a Db,
}

impl<'a> EvilPoints<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The saved evil points of `account`.
    pub async fn get(&self, account: AccountId) -> Result<Option<EvilRecord>> {
        let mut a = Args::new();
        a.add(account.0);
        let row = a
            .query("SELECT userid, evil FROM evil_points WHERE account_id = $1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        row.map(|r| {
            let g = Get::new(&r, "evil_points");
            Ok(EvilRecord {
                userid: g.text("userid")?,
                evil: g.int("evil")?,
            })
        })
        .transpose()
    }

    /// Writes the evil points of `account` (`save_evil_points`).
    pub async fn put(&self, account: AccountId, e: &EvilRecord) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        put(&mut conn, account, e).await
    }

    /// Deletes the evil points of `account`.
    pub async fn delete(&self, account: AccountId) -> Result<bool> {
        let mut conn = self.db.pool().acquire().await?;
        delete(&mut conn, account).await
    }
}
