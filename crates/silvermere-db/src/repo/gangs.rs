//! Gangs: `gangs` (WCCGANGS, key 0 the upper-cased name, key 1 `exp`): reading a gang,
//! saving one (update if it exists, insert otherwise), the gang top list (key 1) and the
//! delete in the midnight cleanup.

use silvermere_types::flags::GangFlags;
use silvermere_types::records::Gang;

use crate::pool::Db;
use crate::repo::{Args, Conn, Get, StateWrite, insert_sql, select_sql};
use crate::{DbError, Result};

const COLS: &[&str] = &[
    "key",
    "name",
    "exp",
    "leader",
    "created",
    "members",
    "flags",
    "overexp",
    "overflows",
];

fn args(g: &Gang) -> Args {
    let mut a = Args::new();
    a.add(&g.key)
        .add(&g.name)
        .add(g.exp)
        .add(&g.leader)
        .add(g.created)
        .add(g.members)
        .add(g.flags.bits())
        .add(g.overexp)
        .add(g.overflows);
    a
}

fn read(row: &sqlx::any::AnyRow) -> Result<Gang> {
    let g = Get::new(row, "gangs");
    Ok(Gang {
        key: g.text("key")?,
        name: g.text("name")?,
        exp: g.int("exp")?,
        leader: g.text("leader")?,
        created: g.int("created")?,
        members: g.int("members")?,
        flags: GangFlags::from_bits_retain(g.int("flags")?),
        overexp: g.int("overexp")?,
        overflows: g.int("overflows")?,
    })
}

/// Inserts `gang`; `DbError::Duplicate` when its key exists in any case.
pub(crate) async fn insert(conn: &mut Conn, gang: &Gang) -> Result<()> {
    args(gang)
        .query(insert_sql("gangs", COLS))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Rewrites the gang whose key matches `gang.key` case-insensitively; inserts it when
/// there is none (the gang's write-back).
pub(crate) async fn update(conn: &mut Conn, db: &Db, gang: &Gang) -> Result<()> {
    let sets: Vec<String> = COLS
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{c} = ${}", i + 1))
        .collect();
    let mut a = args(gang);
    a.add(&gang.key);
    let done = a
        .query(format!(
            "UPDATE gangs SET {} WHERE {}",
            sets.join(", "),
            db.dialect().ci_eq("key", &format!("${}", COLS.len() + 1))
        ))
        .execute(&mut *conn)
        .await?;
    if done.rows_affected() == 0 {
        insert(conn, gang).await?;
    }
    Ok(())
}

/// Deletes the gang keyed `key` (case-insensitive).
pub(crate) async fn delete(conn: &mut Conn, db: &Db, key: &str) -> Result<bool> {
    let mut a = Args::new();
    a.add(key);
    let done = a
        .query(format!(
            "DELETE FROM gangs WHERE {}",
            db.dialect().ci_eq("key", "$1")
        ))
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Gangs.
#[derive(Debug, Clone, Copy)]
pub struct Gangs<'a> {
    db: &'a Db,
}

impl<'a> Gangs<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The gang named `name`, case-insensitive on the 20-character key (acqbtv key 0).
    pub async fn by_name(&self, name: &str) -> Result<Option<Gang>> {
        let mut a = Args::new();
        a.add(name);
        let row = a
            .query(select_sql(
                "gangs",
                COLS,
                &format!("WHERE {}", self.db.dialect().ci_eq("key", "$1")),
            ))
            .fetch_optional(&self.db.pool())
            .await?;
        row.as_ref().map(read).transpose()
    }

    /// The `n` gangs with the most experience, descending (alobtv key 1 and on).
    pub async fn top_by_exp(&self, n: usize) -> Result<Vec<Gang>> {
        let mut a = Args::new();
        a.add(i64::try_from(n).map_err(|_| DbError::Decode {
            table: "gangs",
            column: "exp".to_string(),
            reason: "limit out of range".to_string(),
        })?);
        let rows = a
            .query(select_sql("gangs", COLS, "ORDER BY exp DESC, key LIMIT $1"))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }

    /// Every gang in key order.
    pub async fn all(&self) -> Result<Vec<Gang>> {
        let order = self.db.dialect().ci_column("key");
        let rows = Args::new()
            .query(select_sql("gangs", COLS, &format!("ORDER BY {order}, id")))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }

    /// Writes many gangs (state) in one transaction: with [`StateWrite::Reset`] each is
    /// rewritten, with [`StateWrite::CreateMissing`] only gangs whose key (in any case)
    /// is not there yet are inserted, with [`StateWrite::Untouched`] nothing is written.
    /// The importer's table writer.
    pub async fn put_many(&self, gangs: &[Gang], state: StateWrite) -> Result<()> {
        if state == StateWrite::Untouched {
            return Ok(());
        }
        let mut tx = self.db.pool().begin().await?;
        for gang in gangs {
            if state == StateWrite::Reset {
                update(&mut tx, self.db, gang).await?;
                continue;
            }
            let mut a = Args::new();
            a.add(&gang.key);
            let exists = a
                .query(format!(
                    "SELECT key FROM gangs WHERE {}",
                    self.db.dialect().ci_eq("key", "$1")
                ))
                .fetch_optional(&mut *tx)
                .await?
                .is_some();
            if !exists {
                insert(&mut tx, gang).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Inserts `gang`; `DbError::Duplicate` when the key exists in any case.
    pub async fn insert(&self, gang: &Gang) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        insert(&mut conn, gang).await
    }

    /// Writes `gang` by key (insert when new).
    pub async fn update(&self, gang: &Gang) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        update(&mut conn, self.db, gang).await
    }

    /// Deletes the gang keyed `key`.
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let mut conn = self.db.pool().acquire().await?;
        delete(&mut conn, self.db, key).await
    }
}
