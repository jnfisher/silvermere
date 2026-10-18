//! Limited-item ownership: `item_ownership` (WCCITOWN, key 0 `id`, key 1
//! `(itemnum, owntype)`): the reads and deletes as limited items change hands and the
//! inserts of the reindex.

use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, RoomId};
use silvermere_types::records::ItemOwnership;

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Conn, Get, StateWrite, delete_children_in, insert_many_bulk, insert_sql, select_sql,
};

const COLS: &[&str] = &[
    "id", "itemnum", "owntype", "mapnum", "roomnum", "userid", "monnum",
];

fn read(row: &sqlx::any::AnyRow) -> Result<ItemOwnership> {
    let g = Get::new(row, "item_ownership");
    Ok(ItemOwnership {
        id: g.int("id")?,
        itemnum: ItemNo(g.int("itemnum")?),
        owntype: g.code("owntype")?,
        mapnum: MapId(g.int("mapnum")?),
        roomnum: RoomId(g.int("roomnum")?),
        userid: g.text("userid")?,
        monnum: ActiveMonsterId(g.int("monnum")?),
    })
}

fn args(own: &ItemOwnership) -> Args {
    let mut a = Args::new();
    a.add(own.id)
        .add(own.itemnum.0)
        .add(own.owntype.code())
        .add(own.mapnum.0)
        .add(own.roomnum.0)
        .add(&own.userid)
        .add(own.monnum.0);
    a
}

/// [`insert`] for many records in multi-row statements (T-113).
pub(crate) async fn insert_many_of(conn: &mut Conn, rows: &[&ItemOwnership]) -> Result<()> {
    let rows = rows.iter().map(|o| args(o)).collect();
    insert_many_bulk(conn, "item_ownership", COLS, rows).await
}

/// [`delete`] for many records in one statement (T-113).
pub(crate) async fn delete_many(conn: &mut Conn, ids: &[i32]) -> Result<()> {
    let ids: Vec<i64> = ids.iter().map(|&id| i64::from(id)).collect();
    delete_children_in(conn, "item_ownership", "id", &ids).await?;
    Ok(())
}

/// Inserts `own`; `DbError::Duplicate` when its id exists.
pub(crate) async fn insert(conn: &mut Conn, own: &ItemOwnership) -> Result<()> {
    args(own)
        .query(insert_sql("item_ownership", COLS))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Whether record `id` exists; it is deleted first when `delete_it`.
async fn delete_if(conn: &mut Conn, id: i32, delete_it: bool) -> Result<bool> {
    if delete_it {
        return delete(conn, id).await;
    }
    let mut a = Args::new();
    a.add(id);
    Ok(
        a.query("SELECT id FROM item_ownership WHERE id = $1".to_string())
            .fetch_optional(&mut *conn)
            .await?
            .is_some(),
    )
}

/// Deletes record `id`.
pub(crate) async fn delete(conn: &mut Conn, id: i32) -> Result<bool> {
    let mut a = Args::new();
    a.add(id);
    let done = a
        .query("DELETE FROM item_ownership WHERE id = $1".to_string())
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Limited-item ownership.
#[derive(Debug, Clone, Copy)]
pub struct ItemOwnerships<'a> {
    db: &'a Db,
}

impl<'a> ItemOwnerships<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Every copy of `item` in play, in id order (the key 1 range read).
    pub async fn holders_of(&self, item: ItemNo) -> Result<Vec<ItemOwnership>> {
        let mut a = Args::new();
        a.add(item.0);
        let rows = a
            .query(select_sql(
                "item_ownership",
                COLS,
                "WHERE itemnum = $1 ORDER BY owntype, id",
            ))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }

    /// Record `id`.
    pub async fn get(&self, id: i32) -> Result<Option<ItemOwnership>> {
        let mut a = Args::new();
        a.add(id);
        let row = a
            .query(select_sql("item_ownership", COLS, "WHERE id = $1"))
            .fetch_optional(&self.db.pool())
            .await?;
        row.as_ref().map(read).transpose()
    }

    /// Every record in id order (the reindex scan).
    pub async fn all(&self) -> Result<Vec<ItemOwnership>> {
        let rows = Args::new()
            .query(select_sql("item_ownership", COLS, "ORDER BY id"))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }

    /// The highest record id, 0 when there are none (the ahibtv in the reindex).
    pub async fn max_id(&self) -> Result<i32> {
        let row = Args::new()
            .query("SELECT id FROM item_ownership ORDER BY id DESC LIMIT 1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        Ok(match row {
            Some(r) => Get::new(&r, "item_ownership").int("id")?,
            None => 0,
        })
    }

    /// Inserts `own`; `DbError::Duplicate` when the id exists.
    pub async fn insert(&self, own: &ItemOwnership) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        insert(&mut conn, own).await
    }

    /// Writes many ownership rows (state) in one transaction: with [`StateWrite::Reset`]
    /// each replaces the row of its id, with [`StateWrite::CreateMissing`] only new ids are
    /// inserted, with [`StateWrite::Untouched`] nothing is written. The importer's table
    /// writer.
    pub async fn put_many(&self, rows: &[ItemOwnership], state: StateWrite) -> Result<()> {
        if state == StateWrite::Untouched {
            return Ok(());
        }
        let mut tx = self.db.pool().begin().await?;
        for own in rows {
            let existed = delete_if(&mut tx, own.id, state == StateWrite::Reset).await?;
            if state == StateWrite::Reset || !existed {
                insert(&mut tx, own).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deletes record `id`.
    pub async fn delete(&self, id: i32) -> Result<bool> {
        let mut conn = self.db.pool().acquire().await?;
        delete(&mut conn, id).await
    }
}
