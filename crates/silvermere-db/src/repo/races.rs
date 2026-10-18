//! Races: `races` and `race_abilities` (WCCRACE: one race, and the list displays).

use std::collections::HashMap;

use silvermere_types::enums::Ability;
use silvermere_types::ids::RaceNo;
use silvermere_types::records::Race;
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
    "minint",
    "minwil",
    "minstr",
    "minhea",
    "minagi",
    "mincha",
    "hpbonus",
    "maxenergy",
    "restored",
    "basecp",
    "exp",
    "maxint",
    "maxwil",
    "maxstr",
    "maxhea",
    "maxagi",
    "maxcha",
];

fn args(r: &Race) -> Args {
    let mut a = Args::new();
    a.add(r.number.0)
        .add(&r.name)
        .add(r.minint)
        .add(r.minwil)
        .add(r.minstr)
        .add(r.minhea)
        .add(r.minagi)
        .add(r.mincha)
        .add(r.hpbonus)
        .add(r.maxenergy)
        .add(r.restored)
        .add(r.basecp)
        .add(r.exp)
        .add(r.maxint)
        .add(r.maxwil)
        .add(r.maxstr)
        .add(r.maxhea)
        .add(r.maxagi)
        .add(r.maxcha);
    a
}

/// Inserts `r` with its abilities.
pub(crate) async fn insert(conn: &mut Conn, r: &Race) -> Result<()> {
    args(r)
        .query(insert_sql("races", COLS))
        .execute(&mut *conn)
        .await?;
    write_abilities(
        conn,
        "race_abilities",
        "race",
        i64::from(r.number.0),
        &r.abilities,
    )
    .await
}

/// Upserts `r`, replacing its abilities.
pub(crate) async fn put(conn: &mut Conn, r: &Race) -> Result<()> {
    args(r)
        .query(upsert_sql("races", COLS, &["number"]))
        .execute(&mut *conn)
        .await?;
    delete_children(conn, "race_abilities", "race", i64::from(r.number.0)).await?;
    write_abilities(
        conn,
        "race_abilities",
        "race",
        i64::from(r.number.0),
        &r.abilities,
    )
    .await
}

/// Loads the races in `scope` in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<Race>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "races",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "races");
        out.push(Race {
            number: RaceNo(g.int("number")?),
            name: g.text("name")?,
            minint: g.int("minint")?,
            minwil: g.int("minwil")?,
            minstr: g.int("minstr")?,
            minhea: g.int("minhea")?,
            minagi: g.int("minagi")?,
            mincha: g.int("mincha")?,
            hpbonus: g.int("hpbonus")?,
            maxenergy: g.int("maxenergy")?,
            restored: g.int("restored")?,
            abilities: [(Ability::None, 0); NRECABILS],
            basecp: g.int("basecp")?,
            exp: g.int("exp")?,
            maxint: g.int("maxint")?,
            maxwil: g.int("maxwil")?,
            maxstr: g.int("maxstr")?,
            maxhea: g.int("maxhea")?,
            maxagi: g.int("maxagi")?,
            maxcha: g.int("maxcha")?,
        });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let index: HashMap<i64, usize> = out
        .iter()
        .enumerate()
        .map(|(i, r)| (i64::from(r.number.0), i))
        .collect();
    for (n, abilities) in read_abilities::<NRECABILS>(conn, "race_abilities", "race", scope).await?
    {
        if let Some(&i) = index.get(&n) {
            out[i].abilities = abilities;
        }
    }
    Ok(out)
}

/// Races.
#[derive(Debug, Clone, Copy)]
pub struct Races<'a> {
    db: &'a Db,
}

impl<'a> Races<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Race `n`.
    pub async fn get(&self, n: RaceNo) -> Result<Option<Race>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// Every race in number order.
    pub async fn all(&self) -> Result<Vec<Race>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// Inserts `r`.
    pub async fn insert(&self, r: &Race) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, r).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many races in one transaction (upsert by number, children replaced): the
    /// importer's table writer. Content only, so there is no state mode.
    pub async fn put_many(&self, rows: &[Race]) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        // T-113: in multi-row statements, as the other content tables (T-108)
        let mut b = Batch::default();
        for r in last_per_key(rows, |r| r.number.0) {
            let n = i64::from(r.number.0);
            b.upsert("races", COLS, &["number"], n, args(r));
            b.delete_children("race_abilities", "race", n);
            b.abilities("race_abilities", "race", n, &r.abilities);
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts `r`.
    pub async fn put(&self, r: &Race) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put(&mut tx, r).await?;
        tx.commit().await?;
        Ok(())
    }
}
