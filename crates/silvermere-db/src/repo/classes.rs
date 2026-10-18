//! Classes: `classes` and `class_abilities` (WCCCLASS: one class, and the list
//! displays).

use std::collections::HashMap;

use silvermere_types::enums::Ability;
use silvermere_types::ids::ClassNo;
use silvermere_types::records::Class;
use silvermere_types::sizes::NRECABILS;

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, delete_children, insert_sql, last_per_key, read_abilities,
    select_sql, upsert_sql, write_abilities,
};

const COLS: &[&str] = &[
    "number",
    "name",
    "basehp",
    "hplevel",
    "exp",
    "attmult",
    "maxenergy",
    "restored",
    "magery",
    "magelvl",
    "weapon",
    "armour",
    "combat",
    "titles",
];

fn args(c: &Class) -> Args {
    let mut a = Args::new();
    a.add(c.number.0)
        .add(&c.name)
        .add(c.basehp)
        .add(c.hplevel)
        .add(c.exp)
        .add(c.attmult)
        .add(c.maxenergy)
        .add(c.restored)
        .add(c.magery.code())
        .add(c.magelvl)
        .add(c.weapon.code())
        .add(c.armour.code())
        .add(c.combat)
        .add(c.titles.0);
    a
}

/// Inserts `c` with its abilities.
pub(crate) async fn insert(conn: &mut Conn, c: &Class) -> Result<()> {
    args(c)
        .query(insert_sql("classes", COLS))
        .execute(&mut *conn)
        .await?;
    write_abilities(
        conn,
        "class_abilities",
        "class",
        i64::from(c.number.0),
        &c.abilities,
    )
    .await
}

/// Upserts `c`, replacing its abilities.
pub(crate) async fn put(conn: &mut Conn, c: &Class) -> Result<()> {
    args(c)
        .query(upsert_sql("classes", COLS, &["number"]))
        .execute(&mut *conn)
        .await?;
    delete_children(conn, "class_abilities", "class", i64::from(c.number.0)).await?;
    write_abilities(
        conn,
        "class_abilities",
        "class",
        i64::from(c.number.0),
        &c.abilities,
    )
    .await
}

/// Loads the classes in `scope` in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<Class>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "classes",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "classes");
        out.push(Class {
            number: ClassNo(g.int("number")?),
            name: g.text("name")?,
            basehp: g.int("basehp")?,
            hplevel: g.int("hplevel")?,
            exp: g.int("exp")?,
            attmult: g.int("attmult")?,
            maxenergy: g.int("maxenergy")?,
            restored: g.int("restored")?,
            abilities: [(Ability::None, 0); NRECABILS],
            magery: g.code("magery")?,
            magelvl: g.int("magelvl")?,
            weapon: g.code("weapon")?,
            armour: g.code("armour")?,
            combat: g.int("combat")?,
            titles: g.int::<i32>("titles")?.into(),
        });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let index: HashMap<i64, usize> = out
        .iter()
        .enumerate()
        .map(|(i, c)| (i64::from(c.number.0), i))
        .collect();
    for (n, abilities) in
        read_abilities::<NRECABILS>(conn, "class_abilities", "class", scope).await?
    {
        if let Some(&i) = index.get(&n) {
            out[i].abilities = abilities;
        }
    }
    Ok(out)
}

/// Classes.
#[derive(Debug, Clone, Copy)]
pub struct Classes<'a> {
    db: &'a Db,
}

impl<'a> Classes<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Class `n`.
    pub async fn get(&self, n: ClassNo) -> Result<Option<Class>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// Every class in number order.
    pub async fn all(&self) -> Result<Vec<Class>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// Inserts `c`.
    pub async fn insert(&self, c: &Class) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, c).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many classes in one transaction (upsert by number, children replaced): the
    /// importer's table writer. Content only, so there is no state mode.
    pub async fn put_many(&self, rows: &[Class]) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        // T-113: in multi-row statements, as the other content tables (T-108)
        let mut b = Batch::default();
        for r in last_per_key(rows, |r| r.number.0) {
            let n = i64::from(r.number.0);
            b.upsert("classes", COLS, &["number"], n, args(r));
            b.delete_children("class_abilities", "class", n);
            b.abilities("class_abilities", "class", n, &r.abilities);
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts `c`.
    pub async fn put(&self, c: &Class) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put(&mut tx, c).await?;
        tx.commit().await?;
        Ok(())
    }
}
