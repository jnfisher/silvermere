//! Active monsters: `active_monsters` with `active_monster_trail`, `active_monster_hunters`,
//! `active_monster_items`, `active_monster_spells` (WCCACMSR).
//!
//! Holds the active-monster writes (insert on generate, update on save, delete on remove),
//! the highest active number and the start-up scan that finds the numbers in use.

use std::collections::HashMap;

use silvermere_types::flags::ActiveMonsterFlags;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, MonsterNo, RoomId, SpellNo};
use silvermere_types::records::{ActiveMonster, ActiveSpell};
use silvermere_types::sizes::{NAMTRAIL, NMONITEMS, NMONSPELLS};

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, StateWrite, delete_children, delete_children_in,
    insert_many_bulk, insert_sql, last_per_key, select_sql, upsert_sql,
};

const COLS: &[&str] = &[
    "number",
    "knownnum",
    "level",
    "map",
    "room",
    "poison",
    "energy",
    "hp",
    "following",
    "hunting",
    "name",
    "wornitem",
    "weapon",
    "cash_copper",
    "cash_silver",
    "cash_gold",
    "cash_platinum",
    "cash_runic",
    "maxhp",
    "alignment",
    "followpct",
    "dr",
    "ac",
    "exp",
    "maxenergy",
    "friendly",
    "genroom",
    "inactive",
    "flags",
    "montype",
    "target",
    "hpregen",
    "lastdir",
    "lastupd",
    "grouprole",
    "downrounds",
    "created",
];

const CHILD_TABLES: &[&str] = &[
    "active_monster_trail",
    "active_monster_hunters",
    "active_monster_items",
    "active_monster_spells",
];

fn args(m: &ActiveMonster) -> Args {
    let mut a = Args::new();
    a.add(m.number.0)
        .add(m.knownnum.0)
        .add(m.level)
        .add(m.map.0)
        .add(m.room.0)
        .add(m.poison)
        .add(m.energy)
        .add(m.hp)
        .add(&m.following)
        .add(m.hunting.0)
        .add(&m.name)
        .add(m.wornitem.0)
        .add(m.weapon.0)
        .coins(m.cash)
        .add(m.maxhp)
        .add(m.alignment.code())
        .add(m.followpct)
        .add(m.dr)
        .add(m.ac)
        .add(m.exp)
        .add(m.maxenergy)
        .add(m.friendly)
        .add(m.genroom.0)
        .add(m.inactive)
        .add(m.flags.bits())
        .add(m.montype.code())
        .add(m.target)
        .add(m.hpregen)
        .add(m.lastdir)
        .add(m.lastupd)
        .add(m.grouprole.code())
        .add(m.downrounds)
        .add(m.created);
    a
}

async fn write_children(conn: &mut Conn, m: &ActiveMonster) -> Result<()> {
    let mut b = Batch::default();
    child_rows(&mut b, m);
    b.flush(conn).await
}

/// The child-table rows of `m`, added to `out`.
fn child_rows(out: &mut Batch, m: &ActiveMonster) {
    let n = i64::from(m.number.0);
    let trail: Vec<Args> = m
        .trail
        .iter()
        .enumerate()
        .filter(|(_, r)| r.0 != 0)
        .map(|(slot, r)| {
            let mut a = Args::new();
            a.add(n).add(slot as u16).add(r.0);
            a
        })
        .collect();
    out.insert(
        "active_monster_trail",
        &["active_monster", "slot", "room"],
        trail,
    );
    let hunters: Vec<Args> = m
        .hunters
        .iter()
        .enumerate()
        .filter(|(_, h)| h.0 != 0)
        .map(|(slot, h)| {
            let mut a = Args::new();
            a.add(n).add(slot as u16).add(h.0);
            a
        })
        .collect();
    out.insert(
        "active_monster_hunters",
        &["active_monster", "slot", "hunter"],
        hunters,
    );
    let items: Vec<Args> = m
        .items
        .iter()
        .enumerate()
        .filter(|(_, (item, _))| item.0 != 0)
        .map(|(slot, (item, uses))| {
            let mut a = Args::new();
            a.add(n).add(slot as u16).add(item.0).add(*uses);
            a
        })
        .collect();
    out.insert(
        "active_monster_items",
        &["active_monster", "slot", "item", "uses"],
        items,
    );
    let spells: Vec<Args> = m
        .spells
        .iter()
        .enumerate()
        .filter(|(_, s)| s.spell.0 != 0)
        .map(|(slot, s)| {
            let mut a = Args::new();
            a.add(n)
                .add(slot as u16)
                .add(s.spell.0)
                .add(s.value)
                .add(s.rounds);
            a
        })
        .collect();
    out.insert(
        "active_monster_spells",
        &["active_monster", "slot", "spell", "value", "rounds"],
        spells,
    );
}

/// Inserts `m` (the dinsbtv on generate).
pub(crate) async fn insert(conn: &mut Conn, m: &ActiveMonster) -> Result<()> {
    args(m)
        .query(insert_sql("active_monsters", COLS))
        .execute(&mut *conn)
        .await?;
    write_children(conn, m).await
}

/// Upserts `m` (the dupdbtv on save), replacing its children.
pub(crate) async fn update(conn: &mut Conn, m: &ActiveMonster) -> Result<()> {
    args(m)
        .query(upsert_sql("active_monsters", COLS, &["number"]))
        .execute(&mut *conn)
        .await?;
    for table in CHILD_TABLES {
        delete_children(conn, table, "active_monster", i64::from(m.number.0)).await?;
    }
    write_children(conn, m).await
}

/// [`insert`] for many monsters in multi-row statements (T-113). The numbers must not
/// repeat.
pub(crate) async fn insert_many_of(conn: &mut Conn, monsters: &[&ActiveMonster]) -> Result<()> {
    let rows = monsters.iter().map(|m| args(m)).collect();
    insert_many_bulk(conn, "active_monsters", COLS, rows).await?;
    let mut b = Batch::default();
    for m in monsters {
        child_rows(&mut b, m);
    }
    b.flush(conn).await
}

/// [`update`] for many monsters in multi-row statements (T-113): one upsert, one delete
/// and one insert per child table. The numbers must not repeat.
pub(crate) async fn update_many(conn: &mut Conn, monsters: &[&ActiveMonster]) -> Result<()> {
    let mut b = Batch::default();
    // a repeated number keeps its last record, as one update per write would
    for m in last_per_key(monsters, |m| m.number.0) {
        b.upsert("active_monsters", COLS, &["number"], m.number.0, args(m));
        for table in CHILD_TABLES {
            b.delete_children(table, "active_monster", i64::from(m.number.0));
        }
        child_rows(&mut b, m);
    }
    b.flush(conn).await
}

/// [`delete`] for many monsters in one statement (T-113).
pub(crate) async fn delete_many(conn: &mut Conn, numbers: &[ActiveMonsterId]) -> Result<()> {
    let numbers: Vec<i64> = numbers.iter().map(|n| i64::from(n.0)).collect();
    delete_children_in(conn, "active_monsters", "number", &numbers).await?;
    Ok(())
}

/// Deletes active monster `n` (the delbtv on remove).
pub(crate) async fn delete(conn: &mut Conn, n: ActiveMonsterId) -> Result<bool> {
    let mut a = Args::new();
    a.add(n.0);
    let done = a
        .query("DELETE FROM active_monsters WHERE number = $1".to_string())
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Loads the active monsters in `scope` in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<ActiveMonster>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "active_monsters",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "active_monsters");
        out.push(ActiveMonster {
            number: ActiveMonsterId(g.int("number")?),
            knownnum: MonsterNo(g.int("knownnum")?),
            level: g.int("level")?,
            map: MapId(g.int("map")?),
            room: RoomId(g.int("room")?),
            poison: g.int("poison")?,
            energy: g.int("energy")?,
            hp: g.int("hp")?,
            following: g.text("following")?,
            trail: [RoomId(0); NAMTRAIL],
            hunters: [ActiveMonsterId(0); 10],
            hunting: ActiveMonsterId(g.int("hunting")?),
            name: g.text("name")?,
            wornitem: ItemNo(g.int("wornitem")?),
            weapon: ItemNo(g.int("weapon")?),
            items: [(ItemNo(0), 0); NMONITEMS],
            cash: g.coins("cash_")?,
            maxhp: g.int("maxhp")?,
            alignment: g.code("alignment")?,
            followpct: g.int("followpct")?,
            dr: g.int("dr")?,
            ac: g.int("ac")?,
            exp: g.int("exp")?,
            maxenergy: g.int("maxenergy")?,
            friendly: g.int("friendly")?,
            genroom: RoomId(g.int("genroom")?),
            inactive: g.int("inactive")?,
            flags: ActiveMonsterFlags::from_bits_retain(g.int("flags")?),
            montype: g.code("montype")?,
            target: g.int("target")?,
            hpregen: g.int("hpregen")?,
            lastdir: g.int("lastdir")?,
            lastupd: g.int("lastupd")?,
            grouprole: g.code("grouprole")?,
            spells: [ActiveSpell {
                spell: SpellNo(0),
                value: 0,
                rounds: 0,
            }; NMONSPELLS],
            downrounds: g.int("downrounds")?,
            created: g.int("created")?,
        });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let index: HashMap<i64, usize> = out
        .iter()
        .enumerate()
        .map(|(i, m)| (i64::from(m.number.0), i))
        .collect();
    let (where_, a) = scope.child_where("active_monster");
    for row in a
        .query(format!(
            "SELECT active_monster, slot, room FROM active_monster_trail {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "active_monster_trail");
        let Some(&i) = index.get(&g.i64("active_monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NAMTRAIL {
            out[i].trail[slot] = RoomId(g.int("room")?);
        }
    }
    let (where_, a) = scope.child_where("active_monster");
    for row in a
        .query(format!(
            "SELECT active_monster, slot, hunter FROM active_monster_hunters {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "active_monster_hunters");
        let Some(&i) = index.get(&g.i64("active_monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < 10 {
            out[i].hunters[slot] = ActiveMonsterId(g.int("hunter")?);
        }
    }
    let (where_, a) = scope.child_where("active_monster");
    for row in a
        .query(format!(
            "SELECT active_monster, slot, item, uses FROM active_monster_items {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "active_monster_items");
        let Some(&i) = index.get(&g.i64("active_monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NMONITEMS {
            out[i].items[slot] = (ItemNo(g.int("item")?), g.int("uses")?);
        }
    }
    let (where_, a) = scope.child_where("active_monster");
    for row in a
        .query(format!(
            "SELECT active_monster, slot, spell, value, rounds FROM active_monster_spells {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "active_monster_spells");
        let Some(&i) = index.get(&g.i64("active_monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NMONSPELLS {
            out[i].spells[slot] = ActiveSpell {
                spell: SpellNo(g.int("spell")?),
                value: g.int("value")?,
                rounds: g.int("rounds")?,
            };
        }
    }
    Ok(out)
}

/// Active monsters.
#[derive(Debug, Clone, Copy)]
pub struct ActiveMonsters<'a> {
    db: &'a Db,
}

impl<'a> ActiveMonsters<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Active monster `n`.
    pub async fn get(&self, n: ActiveMonsterId) -> Result<Option<ActiveMonster>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// Every active monster in number order (the recovery scan).
    pub async fn all(&self) -> Result<Vec<ActiveMonster>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// The highest active number, 0 when there are none.
    pub async fn max_id(&self) -> Result<ActiveMonsterId> {
        let row = Args::new()
            .query("SELECT number FROM active_monsters ORDER BY number DESC LIMIT 1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        Ok(ActiveMonsterId(match row {
            Some(r) => Get::new(&r, "active_monsters").int("number")?,
            None => 0,
        }))
    }

    /// Inserts `m`; `DbError::Duplicate` when the number exists.
    pub async fn insert(&self, m: &ActiveMonster) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, m).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes `m` (upsert).
    pub async fn update(&self, m: &ActiveMonster) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        update(&mut tx, m).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many active monsters (state) in one transaction: with [`StateWrite::Reset`]
    /// each is upserted, with [`StateWrite::CreateMissing`] only new numbers are inserted,
    /// with [`StateWrite::Untouched`] nothing is written. The importer's table writer.
    pub async fn put_many(&self, monsters: &[ActiveMonster], state: StateWrite) -> Result<()> {
        if state == StateWrite::Untouched {
            return Ok(());
        }
        let mut tx = self.db.pool().begin().await?;
        for m in monsters {
            if state == StateWrite::Reset {
                update(&mut tx, m).await?;
                continue;
            }
            let mut a = Args::new();
            a.add(m.number.0);
            let exists = a
                .query("SELECT number FROM active_monsters WHERE number = $1".to_string())
                .fetch_optional(&mut *tx)
                .await?
                .is_some();
            if !exists {
                insert(&mut tx, m).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deletes active monster `n`; `false` when it did not exist.
    pub async fn delete(&self, n: ActiveMonsterId) -> Result<bool> {
        let mut conn = self.db.pool().acquire().await?;
        delete(&mut conn, n).await
    }
}
