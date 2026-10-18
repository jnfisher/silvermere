//! Messages: `messages` (WCCMSG, one message at a time).

use silvermere_types::ids::MessageNo;
use silvermere_types::records::Message;

use crate::Result;
use crate::pool::Db;
use crate::repo::{Args, Batch, Conn, Get, Scope, insert_sql, select_sql, upsert_sql};

const COLS: &[&str] = &["number", "text0", "text1", "text2"];

fn args(m: &Message) -> Args {
    let mut a = Args::new();
    a.add(m.number.0)
        .add(&m.msgs[0])
        .add(&m.msgs[1])
        .add(&m.msgs[2]);
    a
}

/// Inserts `m`.
pub(crate) async fn insert(conn: &mut Conn, m: &Message) -> Result<()> {
    args(m)
        .query(insert_sql("messages", COLS))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Upserts `m`.
pub(crate) async fn put(conn: &mut Conn, m: &Message) -> Result<()> {
    args(m)
        .query(upsert_sql("messages", COLS, &["number"]))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Loads the messages in `scope` in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<Message>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "messages",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    rows.iter()
        .map(|row| {
            let g = Get::new(row, "messages");
            Ok(Message {
                number: MessageNo(g.int("number")?),
                msgs: [g.text("text0")?, g.text("text1")?, g.text("text2")?],
            })
        })
        .collect()
}

/// Messages (WCCMSG).
#[derive(Debug, Clone, Copy)]
pub struct Messages<'a> {
    db: &'a Db,
}

impl<'a> Messages<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Message `n`.
    pub async fn get(&self, n: MessageNo) -> Result<Option<Message>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// Every message in number order.
    pub async fn all(&self) -> Result<Vec<Message>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// Inserts `m`.
    pub async fn insert(&self, m: &Message) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        insert(&mut conn, m).await
    }

    /// Writes many messages in one transaction (upsert by number, children replaced): the
    /// importer's table writer. Content only, so there is no state mode.
    pub async fn put_many(&self, rows: &[Message]) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let mut b = Batch::default();
        for r in rows {
            b.upsert("messages", COLS, &["number"], r.number.0, args(r));
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts `m`.
    pub async fn put(&self, m: &Message) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        put(&mut conn, m).await
    }
}
