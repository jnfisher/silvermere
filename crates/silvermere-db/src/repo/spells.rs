//! Spells: `spells` and `spell_abilities`.
//!
//! Replaces the WCCSPELS reads of one spell and the highest spell number.

use std::collections::HashMap;

use silvermere_types::enums::Ability;
use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::SpellNo;
use silvermere_types::records::Spell;
use silvermere_types::sizes::NRECABILS;

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, insert_sql, last_per_key, read_abilities, select_sql,
    write_abilities,
};

const COLS: &[&str] = &[
    "number",
    "name",
    "desc0",
    "desc1",
    "resist",
    "levelcap",
    "flags",
    "energy",
    "level",
    "minval",
    "maxval",
    "spell_type",
    "resmode",
    "diff",
    "durlvl",
    "target",
    "duration",
    "element",
    "resstat",
    "magery",
    "msgnum",
    "mana",
    "maxinc",
    "maxlvls",
    "magelvl",
    "mininc",
    "minlvls",
    "durinc",
    "durlvls",
    "shortnm",
];

fn args(s: &Spell) -> Args {
    let mut a = Args::new();
    a.add(s.number.0)
        .add(&s.name)
        .add(&s.descs[0])
        .add(&s.descs[1])
        .add(s.resist)
        .add(s.levelcap)
        .add(s.flags.bits())
        .add(s.energy)
        .add(s.level)
        .add(s.minval)
        .add(s.maxval)
        .add(s.spell_type.code())
        .add(s.resmode.code())
        .add(s.diff)
        .add(s.durlvl)
        .add(s.target.code())
        .add(s.duration)
        .add(s.element.code())
        .add(s.resstat.code())
        .add(s.magery.code())
        .add(s.msgnum.0)
        .add(s.mana)
        .add(s.maxinc)
        .add(s.maxlvls)
        .add(s.magelvl)
        .add(s.mininc)
        .add(s.minlvls)
        .add(s.durinc)
        .add(s.durlvls)
        .add(&s.shortnm);
    a
}

/// Inserts `s` with its abilities.
pub(crate) async fn insert(conn: &mut Conn, s: &Spell) -> Result<()> {
    args(s)
        .query(insert_sql("spells", COLS))
        .execute(&mut *conn)
        .await?;
    write_abilities(
        conn,
        "spell_abilities",
        "spell",
        i64::from(s.number.0),
        &s.abilities,
    )
    .await
}

/// The upsert of `s` and the replacement of its abilities, into `b`.
fn put_rows(b: &mut Batch, s: &Spell) {
    let n = i64::from(s.number.0);
    b.upsert("spells", COLS, &["number"], s.number.0, args(s));
    b.delete_children("spell_abilities", "spell", n);
    b.abilities("spell_abilities", "spell", n, &s.abilities);
}

/// Upserts `s`, replacing its abilities.
pub(crate) async fn put(conn: &mut Conn, s: &Spell) -> Result<()> {
    let mut b = Batch::default();
    put_rows(&mut b, s);
    b.flush(conn).await
}

/// Loads the spells in `scope` in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<Spell>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "spells",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "spells");
        out.push(Spell {
            number: SpellNo(g.int("number")?),
            name: g.text("name")?,
            descs: [g.text("desc0")?, g.text("desc1")?],
            resist: g.int("resist")?,
            levelcap: g.int("levelcap")?,
            flags: SpellFlags::from_bits_retain(g.int("flags")?),
            abilities: [(Ability::None, 0); NRECABILS],
            energy: g.int("energy")?,
            level: g.int("level")?,
            minval: g.int("minval")?,
            maxval: g.int("maxval")?,
            spell_type: g.code("spell_type")?,
            resmode: g.code("resmode")?,
            diff: g.int("diff")?,
            durlvl: g.int("durlvl")?,
            target: g.code("target")?,
            duration: g.int("duration")?,
            element: g.code("element")?,
            resstat: g.code("resstat")?,
            magery: g.code("magery")?,
            msgnum: g.int::<i32>("msgnum")?.into(),
            mana: g.int("mana")?,
            maxinc: g.int("maxinc")?,
            maxlvls: g.int("maxlvls")?,
            magelvl: g.int("magelvl")?,
            mininc: g.int("mininc")?,
            minlvls: g.int("minlvls")?,
            durinc: g.int("durinc")?,
            durlvls: g.int("durlvls")?,
            shortnm: g.text("shortnm")?,
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
    for (n, abilities) in
        read_abilities::<NRECABILS>(conn, "spell_abilities", "spell", scope).await?
    {
        if let Some(&i) = index.get(&n) {
            out[i].abilities = abilities;
        }
    }
    Ok(out)
}

/// Spells.
#[derive(Debug, Clone, Copy)]
pub struct Spells<'a> {
    db: &'a Db,
}

impl<'a> Spells<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Spell `n`.
    pub async fn get(&self, n: SpellNo) -> Result<Option<Spell>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// Every spell in number order.
    pub async fn all(&self) -> Result<Vec<Spell>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// The highest spell number, 0 when there are none.
    pub async fn max_number(&self) -> Result<SpellNo> {
        let row = Args::new()
            .query("SELECT number FROM spells ORDER BY number DESC LIMIT 1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        Ok(SpellNo(match row {
            Some(r) => Get::new(&r, "spells").int("number")?,
            None => 0,
        }))
    }

    /// Inserts `s`.
    pub async fn insert(&self, s: &Spell) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, s).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many spells in one transaction (upsert by number, children replaced): the
    /// importer's table writer. Content only, so there is no state mode.
    pub async fn put_many(&self, rows: &[Spell]) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let mut b = Batch::default();
        for r in last_per_key(rows, |s| s.number.0) {
            put_rows(&mut b, r);
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts `s`.
    pub async fn put(&self, s: &Spell) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put(&mut tx, s).await?;
        tx.commit().await?;
        Ok(())
    }
}
