//! Shops: `shops` and `shop_items` (content) with `shop_stock` (the live `qty[]`).
//!
//! Replaces the WCCSHOPS reads of one shop and the update on save (stock levels
//! change).

use std::collections::HashMap;

use silvermere_types::ids::{ItemNo, ShopNo};
use silvermere_types::records::{Shop, ShopSlot};
use silvermere_types::sizes::NSHOPITEMS;

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, StateWrite, delete_children, delete_children_in, insert_many,
    insert_many_bulk, insert_sql, last_per_key, select_sql, upsert_sql,
};

/// The live stock of a shop: `qty[NSHOPITEMS]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopStock {
    /// The shop.
    pub number: ShopNo,
    /// Quantity in stock per slot.
    pub qty: [u16; NSHOPITEMS],
}

impl ShopStock {
    /// The live part of `shop`.
    pub fn of(shop: &Shop) -> Self {
        let mut qty = [0; NSHOPITEMS];
        for (q, slot) in qty.iter_mut().zip(shop.stock.iter()) {
            *q = slot.qty;
        }
        Self {
            number: shop.number,
            qty,
        }
    }
}

const COLS: &[&str] = &[
    "number",
    "name",
    "desc0",
    "desc1",
    "desc2",
    "shop_type",
    "minlevel",
    "maxlevel",
    "markup",
    "ownable",
    "expandable",
    "trainclass",
];

const EMPTY_SLOT: ShopSlot = ShopSlot {
    item: ItemNo(0),
    maxqty: 0,
    qty: 0,
    regentime: 0,
    regennum: 0,
    regenpct: 0,
};

fn args(s: &Shop) -> Args {
    let mut a = Args::new();
    a.add(s.number.0)
        .add(&s.name)
        .add(&s.desc[0])
        .add(&s.desc[1])
        .add(&s.desc[2])
        .add(s.shop_type.code())
        .add(s.minlevel)
        .add(s.maxlevel)
        .add(s.markup)
        .add(s.ownable)
        .add(s.expandable)
        .add(s.trainclass.0);
    a
}

fn slot_in_use(slot: &ShopSlot) -> bool {
    // Gang shops reuse the arrays for the owner and price, so a slot is
    // kept when any of its definition fields is set, not only when it names an item.
    slot.item.0 != 0
        || slot.maxqty != 0
        || slot.regentime != 0
        || slot.regennum != 0
        || slot.regenpct != 0
}

const ITEM_COLS: &[&str] = &[
    "shop",
    "slot",
    "item",
    "maxqty",
    "regentime",
    "regennum",
    "regenpct",
];

async fn write_items(conn: &mut Conn, s: &Shop) -> Result<()> {
    insert_many(conn, "shop_items", ITEM_COLS, item_rows(s)).await
}

/// The `shop_items` rows of `s`.
fn item_rows(s: &Shop) -> Vec<Args> {
    s.stock
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot_in_use(slot))
        .map(|(i, slot)| {
            let mut a = Args::new();
            a.add(s.number.0)
                .add(i as u16)
                .add(slot.item.0)
                .add(slot.maxqty)
                .add(slot.regentime)
                .add(slot.regennum)
                .add(slot.regenpct);
            a
        })
        .collect()
}

/// Writes `stock`: every slot with a non-zero quantity (upsert; zero slots are deleted).
pub(crate) async fn put_stock(conn: &mut Conn, stock: &ShopStock) -> Result<()> {
    delete_children(conn, "shop_stock", "shop_id", i64::from(stock.number.0)).await?;
    insert_many(conn, "shop_stock", STOCK_COLS, stock_rows(stock)).await
}

const STOCK_COLS: &[&str] = &["shop_id", "slot", "qty"];

/// [`put_stock`] for many shops: one delete and one multi-row insert (T-113). The shops
/// must not repeat.
pub(crate) async fn put_stocks(conn: &mut Conn, stocks: &[&ShopStock]) -> Result<()> {
    if stocks.is_empty() {
        return Ok(());
    }
    let shops: Vec<i64> = stocks.iter().map(|s| i64::from(s.number.0)).collect();
    delete_children_in(conn, "shop_stock", "shop_id", &shops).await?;
    let rows = stocks.iter().flat_map(|s| stock_rows(s)).collect();
    insert_many_bulk(conn, "shop_stock", STOCK_COLS, rows).await
}

/// The `shop_stock` rows of `stock`.
fn stock_rows(stock: &ShopStock) -> Vec<Args> {
    stock
        .qty
        .iter()
        .enumerate()
        .filter(|(_, q)| **q != 0)
        .map(|(slot, q)| {
            let mut a = Args::new();
            a.add(stock.number.0).add(slot as u16).add(*q);
            a
        })
        .collect()
}

/// Inserts `s` (content and its stock from the record).
pub(crate) async fn insert(conn: &mut Conn, s: &Shop) -> Result<()> {
    args(s)
        .query(insert_sql("shops", COLS))
        .execute(&mut *conn)
        .await?;
    write_items(conn, s).await?;
    put_stock(conn, &ShopStock::of(s)).await
}

/// Upserts the content of `s`; the stock is taken from the record only when the shop is new.
pub(crate) async fn put(conn: &mut Conn, s: &Shop) -> Result<()> {
    let n = i64::from(s.number.0);
    let mut a = Args::new();
    a.add(n);
    let existed = a
        .query("SELECT number FROM shops WHERE number = $1".to_string())
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    args(s)
        .query(upsert_sql("shops", COLS, &["number"]))
        .execute(&mut *conn)
        .await?;
    delete_children(conn, "shop_items", "shop", n).await?;
    write_items(conn, s).await?;
    if !existed {
        put_stock(conn, &ShopStock::of(s)).await?;
    }
    Ok(())
}

/// Loads the shops in `scope`, stock merged, in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<Shop>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "shops",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "shops");
        out.push(Shop {
            number: ShopNo(g.int("number")?),
            name: g.text("name")?,
            desc: [g.text("desc0")?, g.text("desc1")?, g.text("desc2")?],
            shop_type: g.code("shop_type")?,
            minlevel: g.int("minlevel")?,
            maxlevel: g.int("maxlevel")?,
            markup: g.int("markup")?,
            ownable: g.int("ownable")?,
            expandable: g.int("expandable")?,
            trainclass: g.int::<u16>("trainclass")?.into(),
            stock: [EMPTY_SLOT; NSHOPITEMS],
        });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let index: HashMap<i64, usize> = out
        .iter()
        .enumerate()
        .map(|(i, s)| (i64::from(s.number.0), i))
        .collect();
    let (where_, a) = scope.child_where("shop");
    let sql = format!(
        "SELECT shop, slot, item, maxqty, regentime, regennum, regenpct FROM shop_items {where_}"
    );
    for row in a.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "shop_items");
        let Some(&i) = index.get(&g.i64("shop")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NSHOPITEMS {
            out[i].stock[slot] = ShopSlot {
                item: ItemNo(g.int("item")?),
                maxqty: g.int("maxqty")?,
                qty: 0,
                regentime: g.int("regentime")?,
                regennum: g.int("regennum")?,
                regenpct: g.int("regenpct")?,
            };
        }
    }
    let (where_, a) = scope.child_where("shop_id");
    let sql = format!("SELECT shop_id, slot, qty FROM shop_stock {where_}");
    for row in a.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "shop_stock");
        let Some(&i) = index.get(&g.i64("shop_id")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NSHOPITEMS {
            out[i].stock[slot].qty = g.int("qty")?;
        }
    }
    Ok(out)
}

/// Shops.
#[derive(Debug, Clone, Copy)]
pub struct Shops<'a> {
    db: &'a Db,
}

impl<'a> Shops<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Shop `n`, stock merged.
    pub async fn get(&self, n: ShopNo) -> Result<Option<Shop>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// Every shop in number order.
    pub async fn all(&self) -> Result<Vec<Shop>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// Inserts `s` (content plus the virgin stock).
    pub async fn insert(&self, s: &Shop) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, s).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many shops in one transaction: content upserted by number (`shop_items`
    /// replaced), `shop_stock` per `state` from the record's `qty[]`. The importer's
    /// table writer.
    pub async fn put_many(&self, shops: &[Shop], state: StateWrite) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let mut existing = std::collections::HashSet::new();
        if state == StateWrite::CreateMissing {
            let rows = Args::new()
                .query("SELECT number FROM shops".to_string())
                .fetch_all(&mut *tx)
                .await?;
            for row in &rows {
                existing.insert(Get::new(row, "shops").i64("number")?);
            }
        }
        // T-113: in multi-row statements, as the other content tables (T-108)
        let mut b = Batch::default();
        let mut stocks = Vec::new();
        for s in last_per_key(shops, |s| s.number.0) {
            let n = i64::from(s.number.0);
            b.upsert("shops", COLS, &["number"], n, args(s));
            b.delete_children("shop_items", "shop", n);
            b.insert("shop_items", ITEM_COLS, item_rows(s));
            if state.writes(existing.contains(&n)) {
                stocks.push(ShopStock::of(s));
            }
        }
        b.flush(&mut tx).await?;
        put_stocks(&mut tx, &stocks.iter().collect::<Vec<_>>()).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts the content of `s`; live stock is kept when present.
    pub async fn put(&self, s: &Shop) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put(&mut tx, s).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Live shop stock.
#[derive(Debug, Clone, Copy)]
pub struct ShopStocks<'a> {
    db: &'a Db,
}

impl<'a> ShopStocks<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The stock of shop `n`; `None` when the shop does not exist.
    pub async fn get(&self, n: ShopNo) -> Result<Option<ShopStock>> {
        let mut a = Args::new();
        a.add(n.0);
        let exists = a
            .clone()
            .query("SELECT number FROM shops WHERE number = $1".to_string())
            .fetch_optional(&self.db.pool())
            .await?
            .is_some();
        if !exists {
            return Ok(None);
        }
        let mut stock = ShopStock {
            number: n,
            qty: [0; NSHOPITEMS],
        };
        let rows = a
            .query("SELECT slot, qty FROM shop_stock WHERE shop_id = $1".to_string())
            .fetch_all(&self.db.pool())
            .await?;
        for row in rows {
            let g = Get::new(&row, "shop_stock");
            let slot: usize = g.int("slot")?;
            if slot < NSHOPITEMS {
                stock.qty[slot] = g.int("qty")?;
            }
        }
        Ok(Some(stock))
    }

    /// Writes `stock` (the shop save on stock change).
    pub async fn put(&self, stock: &ShopStock) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put_stock(&mut tx, stock).await?;
        tx.commit().await?;
        Ok(())
    }
}
