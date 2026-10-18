//! Items: `items`, `item_abilities`, `item_classes`, `item_races`, `item_negates`.
//!
//! Replaces the WCCITEMS reads: one item by number (key 0) and the highest item number
//! (capped at [`MAX_ITEM_NUMBER`]).

use std::collections::HashMap;

use silvermere_types::enums::Ability;
use silvermere_types::ids::{ClassNo, ItemNo, RaceNo, SpellNo};
use silvermere_types::records::Item;
use silvermere_types::sizes::NITEMABILS;

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, StateWrite, insert_sql, last_per_key, read_abilities, select_sql,
};

/// The highest item number the game ever reports.
pub const MAX_ITEM_NUMBER: i32 = 50000;

const COLS: &[&str] = &[
    "number",
    "level",
    "limit_copies",
    "name",
    "desc0",
    "desc1",
    "desc2",
    "desc3",
    "desc4",
    "desc5",
    "hiddesc0",
    "hiddesc1",
    "hiddesc2",
    "active",
    "weight",
    "item_type",
    "charges",
    "energy",
    "cost",
    "mindmg",
    "maxdmg",
    "ac",
    "wpntype",
    "armtype",
    "worn",
    "accuracy",
    "dr",
    "gettable",
    "strreq",
    "cash_copper",
    "cash_silver",
    "cash_gold",
    "cash_platinum",
    "cash_runic",
    "speed",
    "hitmsg",
    "missmsg",
    "textnum",
    "destroymsg",
    "nodrop",
    "currency",
    "retain",
    "robable",
    "destroy",
];

fn args(item: &Item) -> Args {
    let mut a = Args::new();
    a.add(item.number.0)
        .add(item.level)
        .add(item.limit)
        .add(&item.name);
    for line in &item.desc {
        a.add(line);
    }
    for line in &item.hiddesc {
        a.add(line);
    }
    a.add(item.active)
        .add(item.weight)
        .add(item.item_type.code())
        .add(item.charges)
        .add(item.energy)
        .add(item.cost)
        .add(item.mindmg)
        .add(item.maxdmg)
        .add(item.ac)
        .add(item.wpntype.code())
        .add(item.armtype.code())
        .add(item.worn.code())
        .add(item.accuracy)
        .add(item.dr)
        .add(item.gettable)
        .add(item.strreq)
        .coins(item.cash)
        .add(item.speed)
        .add(item.hitmsg.0)
        .add(item.missmsg.0)
        .add(item.textnum.0)
        .add(item.destroymsg.0)
        .add(item.nodrop)
        .add(item.currency.code())
        .add(item.retain)
        .add(item.robable)
        .add(item.destroy);
    a
}

/// The child rows of `item` into `b`.
fn children(b: &mut Batch, item: &Item) {
    let n = item.number.0;
    b.abilities("item_abilities", "item", i64::from(n), &item.abilities);
    for (table, col, values) in [
        (
            "item_classes",
            "class",
            item.classes
                .iter()
                .map(|c| u32::from(c.0))
                .collect::<Vec<_>>(),
        ),
        (
            "item_races",
            "race",
            item.races.iter().map(|r| u32::from(r.0)).collect(),
        ),
        (
            "item_negates",
            "spell",
            item.negates.iter().map(|s| s.0 as u16 as u32).collect(),
        ),
    ] {
        let rows: Vec<Args> = values
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(slot, v)| {
                let mut a = Args::new();
                a.add(n).add(slot as u16).add(*v);
                a
            })
            .collect();
        b.insert(table, &["item", "slot", col], rows);
    }
}

/// Inserts `item` with its children.
pub(crate) async fn insert(conn: &mut Conn, item: &Item) -> Result<()> {
    args(item)
        .query(insert_sql("items", COLS))
        .execute(&mut *conn)
        .await?;
    let mut b = Batch::default();
    children(&mut b, item);
    b.flush(conn).await
}

/// The upsert of `item` and the replacement of its children, into `b`.
fn put_rows(b: &mut Batch, item: &Item) {
    b.upsert("items", COLS, &["number"], item.number.0, args(item));
    for table in [
        "item_abilities",
        "item_classes",
        "item_races",
        "item_negates",
    ] {
        b.delete_children(table, "item", i64::from(item.number.0));
    }
    children(b, item);
}

/// Upserts `item`, replacing its children.
pub(crate) async fn put(conn: &mut Conn, item: &Item) -> Result<()> {
    let mut b = Batch::default();
    put_rows(&mut b, item);
    b.flush(conn).await
}

/// Loads the items in `scope` in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<Item>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "items",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "items");
        items.push(Item {
            number: ItemNo(g.int("number")?),
            level: g.int("level")?,
            limit: g.int("limit_copies")?,
            name: g.text("name")?,
            desc: [
                g.text("desc0")?,
                g.text("desc1")?,
                g.text("desc2")?,
                g.text("desc3")?,
                g.text("desc4")?,
                g.text("desc5")?,
            ],
            hiddesc: [
                g.text("hiddesc0")?,
                g.text("hiddesc1")?,
                g.text("hiddesc2")?,
            ],
            active: g.int("active")?,
            weight: g.int("weight")?,
            item_type: g.code("item_type")?,
            abilities: [(Ability::None, 0); NITEMABILS],
            charges: g.int("charges")?,
            energy: g.int("energy")?,
            cost: g.int("cost")?,
            classes: [ClassNo(0); 10],
            mindmg: g.int("mindmg")?,
            maxdmg: g.int("maxdmg")?,
            ac: g.int("ac")?,
            races: [RaceNo(0); 10],
            negates: [SpellNo(0); 10],
            wpntype: g.code("wpntype")?,
            armtype: g.code("armtype")?,
            worn: g.code("worn")?,
            accuracy: g.int("accuracy")?,
            dr: g.int("dr")?,
            gettable: g.int("gettable")?,
            strreq: g.int("strreq")?,
            cash: g.coins("cash_")?,
            speed: g.int("speed")?,
            hitmsg: g.int::<i32>("hitmsg")?.into(),
            missmsg: g.int::<i32>("missmsg")?.into(),
            textnum: g.int::<i32>("textnum")?.into(),
            destroymsg: g.int::<i32>("destroymsg")?.into(),
            nodrop: g.int("nodrop")?,
            currency: g.code("currency")?,
            retain: g.int("retain")?,
            robable: g.int("robable")?,
            destroy: g.int("destroy")?,
        });
    }
    let index: HashMap<i64, usize> = items
        .iter()
        .enumerate()
        .map(|(i, it)| (i64::from(it.number.0), i))
        .collect();
    for (n, abilities) in
        read_abilities::<NITEMABILS>(conn, "item_abilities", "item", scope).await?
    {
        if let Some(&i) = index.get(&n) {
            items[i].abilities = abilities;
        }
    }
    for (table, col) in [
        ("item_classes", "class"),
        ("item_races", "race"),
        ("item_negates", "spell"),
    ] {
        let (where_, a) = scope.child_where("item");
        let sql = format!("SELECT item, slot, {col} AS v FROM {table} {where_}");
        for row in a.query(sql).fetch_all(&mut *conn).await? {
            let g = Get::new(&row, "items");
            let Some(&i) = index.get(&g.i64("item")?) else {
                continue;
            };
            let slot: usize = g.int("slot")?;
            if slot >= 10 {
                continue;
            }
            match col {
                "class" => items[i].classes[slot] = ClassNo(g.int("v")?),
                "race" => items[i].races[slot] = RaceNo(g.int("v")?),
                _ => items[i].negates[slot] = SpellNo(g.int::<u16>("v")? as i16),
            }
        }
    }
    Ok(items)
}

/// Items.
#[derive(Debug, Clone, Copy)]
pub struct Items<'a> {
    db: &'a Db,
}

impl<'a> Items<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Item `n`.
    pub async fn get(&self, n: ItemNo) -> Result<Option<Item>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// The highest item number, at most [`MAX_ITEM_NUMBER`] 50000; 0 when there are no
    /// items.
    pub async fn max_number(&self) -> Result<ItemNo> {
        let row = Args::new()
            .query("SELECT number FROM items ORDER BY number DESC LIMIT 1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        let last = match row {
            Some(r) => Get::new(&r, "items").int::<i32>("number")?,
            None => 0,
        };
        Ok(ItemNo(last.min(MAX_ITEM_NUMBER)))
    }

    /// Every item in number order.
    pub async fn all(&self) -> Result<Vec<Item>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// Inserts `item`; `DbError::Duplicate` when the number exists.
    pub async fn insert(&self, item: &Item) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, item).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many items in one transaction (upsert by number, children replaced). The
    /// live `active` count of an item that exists is kept unless `state` is
    /// [`StateWrite::Reset`]. The importer's table writer.
    pub async fn put_many(&self, items: &[Item], state: StateWrite) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let mut active: HashMap<i32, u16> = HashMap::new();
        if state != StateWrite::Reset {
            let rows = Args::new()
                .query("SELECT number, active FROM items".to_string())
                .fetch_all(&mut *tx)
                .await?;
            for row in &rows {
                let g = Get::new(row, "items");
                active.insert(g.int("number")?, g.int("active")?);
            }
        }
        let mut b = Batch::default();
        for item in last_per_key(items, |i| i.number.0) {
            match active.get(&item.number.0) {
                Some(&live) if live != item.active => {
                    let mut kept = item.clone();
                    kept.active = live;
                    put_rows(&mut b, &kept);
                }
                _ => put_rows(&mut b, item),
            }
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts `item` (content refresh).
    pub async fn put(&self, item: &Item) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put(&mut tx, item).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Sets the live `active` count of a limited item.
    pub async fn set_active(&self, n: ItemNo, active: u16) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        set_active(&mut conn, n, active).await
    }

    /// Deletes item `n`.
    pub async fn delete(&self, n: ItemNo) -> Result<bool> {
        let mut a = Args::new();
        a.add(n.0);
        let done = a
            .query("DELETE FROM items WHERE number = $1".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}

/// [`set_active`] for many items in one statement per 1,000 (T-113): `UPDATE ... SET
/// active = CASE number WHEN ... END`.
pub(crate) async fn set_active_many(conn: &mut Conn, items: &[(ItemNo, u16)]) -> Result<()> {
    // a repeated item keeps its last count, as one update per write would (a CASE takes
    // the first WHEN that matches)
    let items: Vec<(ItemNo, u16)> = last_per_key(items, |(n, _)| n.0)
        .into_iter()
        .copied()
        .collect();
    for chunk in items.chunks(1000) {
        let mut a = Args::new();
        let mut whens = Vec::with_capacity(chunk.len());
        for (n, active) in chunk {
            whens.push(format!("WHEN ${} THEN ${}", a.len() + 1, a.len() + 2));
            a.add(n.0).add(*active);
        }
        let numbers: Vec<String> = (0..chunk.len())
            .map(|i| format!("${}", 2 * i + 1))
            .collect();
        let sql = format!(
            "UPDATE items SET active = CASE number {} END WHERE number IN ({})",
            whens.join(" "),
            numbers.join(", ")
        );
        a.query(sql).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Sets `items.active` for item `n`.
pub(crate) async fn set_active(conn: &mut Conn, n: ItemNo, active: u16) -> Result<()> {
    let mut a = Args::new();
    a.add(active).add(n.0);
    a.query("UPDATE items SET active = $1 WHERE number = $2".to_string())
        .execute(&mut *conn)
        .await?;
    Ok(())
}
