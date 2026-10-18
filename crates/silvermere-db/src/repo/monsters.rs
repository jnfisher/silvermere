//! Known monsters: `monsters` with its child tables (content) and `monster_state`.
//!
//! Replaces the WCCKNMSR reads of one monster, the startup scan that builds the monster
//! quick reference (`monqref`), the highest monster number and the write-back of a
//! monster's kill time.

use std::collections::HashMap;

use silvermere_types::enums::Ability;
use silvermere_types::ids::{ItemNo, MonsterNo, SpellNo};
use silvermere_types::records::{KnownMonster, MonsterAttack, MonsterItem, MonsterSpell};
use silvermere_types::sizes::{NMONATTACKS, NMONITEMS, NMONSPELLS, NRECABILS};

use crate::Result;
use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, StateWrite, insert_sql, last_per_key, read_abilities, select_sql,
};

/// The live part of `struct monster`: `curalive` and `killed_at` (`killdate`, `killtime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterState {
    /// The known monster.
    pub number: MonsterNo,
    /// Alive now.
    pub curalive: u16,
    /// When last killed, Unix seconds UTC; 0 = never.
    pub killed_at: i64,
}

impl MonsterState {
    /// The live part of `monster`.
    pub fn of(monster: &KnownMonster) -> Self {
        Self {
            number: monster.number,
            curalive: monster.curalive,
            killed_at: monster.killed_at,
        }
    }
}

const COLS: &[&str] = &[
    "number",
    "name",
    "montype",
    "level",
    "genindex",
    "wornitem",
    "weapon",
    "dr",
    "ac",
    "group_num",
    "followpct",
    "magicres",
    "bsdefense",
    "exp",
    "maxhp",
    "energy",
    "hpregen",
    "maxalive",
    "grouprole",
    "maxfollow",
    "undead",
    "alignment",
    "regenhrs",
    "arrivemsg",
    "deathmsg",
    "cash_copper",
    "cash_silver",
    "cash_gold",
    "cash_platinum",
    "cash_runic",
    "talktext",
    "charmlvl",
    "namelist",
    "randtext",
    "charmres",
    "deathspell",
    "createspell",
    "desc0",
    "desc1",
    "desc2",
    "desc3",
    "gender",
];

const ATTACK_COLS: &[&str] = &[
    "monster", "slot", "kind", "accuracy", "pct", "min", "max", "hitmsg", "defmsg", "missmsg",
    "energy", "hitspell",
];

fn args(m: &KnownMonster) -> Args {
    let mut a = Args::new();
    a.add(m.number.0)
        .add(&m.name)
        .add(m.montype.code())
        .add(m.level)
        .add(m.genindex)
        .add(m.wornitem.0)
        .add(m.weapon.0)
        .add(m.dr)
        .add(m.ac)
        .add(m.group)
        .add(m.followpct)
        .add(m.magicres)
        .add(m.bsdefense)
        .add(m.exp)
        .add(m.maxhp)
        .add(m.energy)
        .add(m.hpregen)
        .add(m.maxalive)
        .add(m.grouprole.code())
        .add(m.maxfollow)
        .add(m.undead)
        .add(m.alignment.code())
        .add(m.regenhrs)
        .add(m.arrivemsg.0)
        .add(m.deathmsg.0)
        .coins(m.cash)
        .add(m.talktext.0)
        .add(m.charmlvl)
        .add(m.namelist)
        .add(m.randtext.0)
        .add(m.charmres)
        .add(m.deathspell.0)
        .add(m.createspell.0);
    for line in &m.desc {
        a.add(line);
    }
    a.add(m.gender.code());
    a
}

fn empty_attack() -> MonsterAttack {
    MonsterAttack {
        kind: silvermere_types::enums::MonsterAttackType::from_code(0),
        accuracy: 0,
        pct: 0,
        min: 0,
        max: 0,
        hitmsg: silvermere_types::ids::MessageNo(0),
        defmsg: silvermere_types::ids::MessageNo(0),
        missmsg: silvermere_types::ids::MessageNo(0),
        energy: 0,
        hitspell: SpellNo(0),
    }
}

fn attack_is_empty(a: &MonsterAttack) -> bool {
    *a == empty_attack()
}

/// Whether a spell slot is stored. A monster casts at all only when its first slot holds a
/// spell, and then every slot's chance (and the picked slot's level) counts, so a chance or
/// level on a slot with no spell is behaviour and is kept.
fn spell_slot_in_use(s: &MonsterSpell) -> bool {
    s.spell.0 != 0 || s.pct != 0 || s.level != 0
}

/// The child rows of `m` into `b`.
fn children(b: &mut Batch, m: &KnownMonster) {
    let n = i64::from(m.number.0);
    b.abilities("monster_abilities", "monster", n, &m.abilities);
    let attacks: Vec<Args> = m
        .attacks
        .iter()
        .enumerate()
        .filter(|(_, atk)| !attack_is_empty(atk))
        .map(|(slot, atk)| {
            let mut a = Args::new();
            a.add(n)
                .add(slot as u16)
                .add(atk.kind.code())
                .add(atk.accuracy)
                .add(atk.pct)
                .add(atk.min)
                .add(atk.max)
                .add(atk.hitmsg.0)
                .add(atk.defmsg.0)
                .add(atk.missmsg.0)
                .add(atk.energy)
                .add(atk.hitspell.0);
            a
        })
        .collect();
    b.insert("monster_attacks", ATTACK_COLS, attacks);
    let items: Vec<Args> = m
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.item.0 != 0)
        .map(|(slot, it)| {
            let mut a = Args::new();
            a.add(n)
                .add(slot as u16)
                .add(it.item.0)
                .add(it.uses)
                .add(it.pct);
            a
        })
        .collect();
    b.insert(
        "monster_items",
        &["monster", "slot", "item", "uses", "pct"],
        items,
    );
    let spells: Vec<Args> = m
        .spells
        .iter()
        .enumerate()
        .filter(|(_, s)| spell_slot_in_use(s))
        .map(|(slot, s)| {
            let mut a = Args::new();
            a.add(n)
                .add(slot as u16)
                .add(s.spell.0)
                .add(s.pct)
                .add(s.level);
            a
        })
        .collect();
    b.insert(
        "monster_spells",
        &["monster", "slot", "spell", "pct", "level"],
        spells,
    );
}

/// Inserts `m` with its children and its state from the record.
pub(crate) async fn insert(conn: &mut Conn, m: &KnownMonster) -> Result<()> {
    args(m)
        .query(insert_sql("monsters", COLS))
        .execute(&mut *conn)
        .await?;
    let mut b = Batch::default();
    children(&mut b, m);
    state_row(&mut b, &MonsterState::of(m));
    b.flush(conn).await
}

/// The upsert of the content of `m` and the replacement of its children, into `b`; the
/// state from the record too when `with_state`.
fn put_rows(b: &mut Batch, m: &KnownMonster, with_state: bool) {
    b.upsert("monsters", COLS, &["number"], m.number.0, args(m));
    if with_state {
        state_row(b, &MonsterState::of(m));
    }
    let n = i64::from(m.number.0);
    for table in [
        "monster_abilities",
        "monster_attacks",
        "monster_items",
        "monster_spells",
    ] {
        b.delete_children(table, "monster", n);
    }
    children(b, m);
}

/// Upserts the content of `m`, replacing its children; the state is created from the
/// record only when the monster is new.
pub(crate) async fn put(conn: &mut Conn, m: &KnownMonster) -> Result<()> {
    let mut a = Args::new();
    a.add(i64::from(m.number.0));
    let has_state = a
        .query("SELECT monster_id FROM monster_state WHERE monster_id = $1".to_string())
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    let mut b = Batch::default();
    put_rows(&mut b, m, !has_state);
    b.flush(conn).await
}

const STATE_COLS: &[&str] = &["monster_id", "curalive", "killed_at"];

/// The upsert of `state` into `b`.
fn state_row(b: &mut Batch, state: &MonsterState) {
    let mut a = Args::new();
    a.add(state.number.0)
        .add(state.curalive)
        .add(state.killed_at);
    b.upsert(
        "monster_state",
        STATE_COLS,
        &["monster_id"],
        state.number.0,
        a,
    );
}

/// [`put_state`] for many monsters in multi-row statements (the persistence worker's
/// batch, T-113). A repeated number keeps its last state (`Batch::upsert`).
pub(crate) async fn put_states(conn: &mut Conn, states: &[&MonsterState]) -> Result<()> {
    let mut b = Batch::default();
    for s in states {
        state_row(&mut b, s);
    }
    b.flush(conn).await
}

/// Upserts `state`.
pub(crate) async fn put_state(conn: &mut Conn, state: &MonsterState) -> Result<()> {
    let mut b = Batch::default();
    state_row(&mut b, state);
    b.flush(conn).await
}

/// Loads the known monsters in `scope`, state merged, in number order.
pub(crate) async fn load(conn: &mut Conn, scope: &Scope) -> Result<Vec<KnownMonster>> {
    let (where_, a) = scope.child_where("number");
    let rows = a
        .query(select_sql(
            "monsters",
            COLS,
            &format!("{where_} ORDER BY number"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "monsters");
        out.push(KnownMonster {
            number: MonsterNo(g.int("number")?),
            name: g.text("name")?,
            montype: g.code("montype")?,
            level: g.int("level")?,
            genindex: g.int("genindex")?,
            wornitem: ItemNo(g.int("wornitem")?),
            weapon: ItemNo(g.int("weapon")?),
            dr: g.int("dr")?,
            ac: g.int("ac")?,
            group: g.int("group_num")?,
            followpct: g.int("followpct")?,
            magicres: g.int("magicres")?,
            bsdefense: g.int("bsdefense")?,
            exp: g.int("exp")?,
            maxhp: g.int("maxhp")?,
            energy: g.int("energy")?,
            hpregen: g.int("hpregen")?,
            abilities: [(Ability::None, 0); NRECABILS],
            maxalive: g.int("maxalive")?,
            curalive: 0,
            grouprole: g.code("grouprole")?,
            maxfollow: g.int("maxfollow")?,
            undead: g.int("undead")?,
            alignment: g.code("alignment")?,
            regenhrs: g.int("regenhrs")?,
            killed_at: 0,
            arrivemsg: g.int::<i32>("arrivemsg")?.into(),
            deathmsg: g.int::<i32>("deathmsg")?.into(),
            items: [MonsterItem {
                item: ItemNo(0),
                uses: 0,
                pct: 0,
            }; NMONITEMS],
            cash: g.coins("cash_")?,
            talktext: g.int::<i32>("talktext")?.into(),
            charmlvl: g.int("charmlvl")?,
            namelist: g.int("namelist")?,
            attacks: [empty_attack(); NMONATTACKS],
            randtext: g.int::<i32>("randtext")?.into(),
            charmres: g.int("charmres")?,
            deathspell: SpellNo(g.int("deathspell")?),
            createspell: SpellNo(g.int("createspell")?),
            spells: [MonsterSpell {
                spell: SpellNo(0),
                pct: 0,
                level: 0,
            }; NMONSPELLS],
            desc: [
                g.text("desc0")?,
                g.text("desc1")?,
                g.text("desc2")?,
                g.text("desc3")?,
            ],
            gender: g.code("gender")?,
        });
    }
    let index: HashMap<i64, usize> = out
        .iter()
        .enumerate()
        .map(|(i, m)| (i64::from(m.number.0), i))
        .collect();
    for (n, abilities) in
        read_abilities::<NRECABILS>(conn, "monster_abilities", "monster", scope).await?
    {
        if let Some(&i) = index.get(&n) {
            out[i].abilities = abilities;
        }
    }
    let (where_, a) = scope.child_where("monster");
    let rows = a
        .query(select_sql("monster_attacks", ATTACK_COLS, &where_))
        .fetch_all(&mut *conn)
        .await?;
    for row in rows {
        let g = Get::new(&row, "monster_attacks");
        let Some(&i) = index.get(&g.i64("monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NMONATTACKS {
            out[i].attacks[slot] = MonsterAttack {
                kind: g.code("kind")?,
                accuracy: g.int("accuracy")?,
                pct: g.int("pct")?,
                min: g.int("min")?,
                max: g.int("max")?,
                hitmsg: g.int::<i32>("hitmsg")?.into(),
                defmsg: g.int::<i32>("defmsg")?.into(),
                missmsg: g.int::<i32>("missmsg")?.into(),
                energy: g.int("energy")?,
                hitspell: SpellNo(g.int("hitspell")?),
            };
        }
    }
    let (where_, a) = scope.child_where("monster");
    let sql = format!("SELECT monster, slot, item, uses, pct FROM monster_items {where_}");
    for row in a.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "monster_items");
        let Some(&i) = index.get(&g.i64("monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NMONITEMS {
            out[i].items[slot] = MonsterItem {
                item: ItemNo(g.int("item")?),
                uses: g.int("uses")?,
                pct: g.int("pct")?,
            };
        }
    }
    let (where_, a) = scope.child_where("monster");
    let sql = format!("SELECT monster, slot, spell, pct, level FROM monster_spells {where_}");
    for row in a.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "monster_spells");
        let Some(&i) = index.get(&g.i64("monster")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NMONSPELLS {
            out[i].spells[slot] = MonsterSpell {
                spell: SpellNo(g.int("spell")?),
                pct: g.int("pct")?,
                level: g.int("level")?,
            };
        }
    }
    let (where_, a) = scope.child_where("monster_id");
    let sql = format!("SELECT monster_id, curalive, killed_at FROM monster_state {where_}");
    for row in a.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "monster_state");
        let Some(&i) = index.get(&g.i64("monster_id")?) else {
            continue;
        };
        out[i].curalive = g.int("curalive")?;
        out[i].killed_at = g.i64("killed_at")?;
    }
    Ok(out)
}

/// Known monsters.
#[derive(Debug, Clone, Copy)]
pub struct Monsters<'a> {
    db: &'a Db,
}

impl<'a> Monsters<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Known monster `n`.
    pub async fn get(&self, n: MonsterNo) -> Result<Option<KnownMonster>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(i64::from(n.0))).await?.pop())
    }

    /// The highest monster number, 0 when there are none; no cap.
    pub async fn max_number(&self) -> Result<MonsterNo> {
        let row = Args::new()
            .query("SELECT number FROM monsters ORDER BY number DESC LIMIT 1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        Ok(MonsterNo(match row {
            Some(r) => Get::new(&r, "monsters").int("number")?,
            None => 0,
        }))
    }

    /// Every known monster in number order (the `monqref[]` startup scan).
    pub async fn all(&self) -> Result<Vec<KnownMonster>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All).await
    }

    /// Inserts `m` (content and its state from the record).
    pub async fn insert(&self, m: &KnownMonster) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        insert(&mut tx, m).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes many known monsters in one transaction: content upserted by number
    /// (children replaced), `monster_state` per `state` from the record. The importer's
    /// table writer.
    pub async fn put_many(&self, monsters: &[KnownMonster], state: StateWrite) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let mut has_state = std::collections::HashSet::new();
        if state == StateWrite::CreateMissing {
            let rows = Args::new()
                .query("SELECT monster_id FROM monster_state".to_string())
                .fetch_all(&mut *tx)
                .await?;
            for row in &rows {
                has_state.insert(Get::new(row, "monster_state").i64("monster_id")?);
            }
        }
        let mut b = Batch::default();
        for m in last_per_key(monsters, |m| m.number.0) {
            let n = i64::from(m.number.0);
            put_rows(&mut b, m, state.writes(has_state.contains(&n)));
        }
        b.flush(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts the content of `m` (content refresh); the state is kept when present.
    pub async fn put(&self, m: &KnownMonster) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put(&mut tx, m).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes known monster `n`.
    pub async fn delete(&self, n: MonsterNo) -> Result<bool> {
        let mut a = Args::new();
        a.add(n.0);
        let done = a
            .query("DELETE FROM monsters WHERE number = $1".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}

/// The live part of known monsters.
#[derive(Debug, Clone, Copy)]
pub struct MonsterStates<'a> {
    db: &'a Db,
}

impl<'a> MonsterStates<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The state of known monster `n`.
    pub async fn get(&self, n: MonsterNo) -> Result<Option<MonsterState>> {
        let mut a = Args::new();
        a.add(n.0);
        let row = a
            .query(
                "SELECT monster_id, curalive, killed_at FROM monster_state WHERE monster_id = $1"
                    .to_string(),
            )
            .fetch_optional(&self.db.pool())
            .await?;
        row.map(|r| {
            let g = Get::new(&r, "monster_state");
            Ok(MonsterState {
                number: MonsterNo(g.int("monster_id")?),
                curalive: g.int("curalive")?,
                killed_at: g.i64("killed_at")?,
            })
        })
        .transpose()
    }

    /// Writes `state` (the kill time and alive count).
    pub async fn put(&self, state: &MonsterState) -> Result<()> {
        let mut conn = self.db.pool().acquire().await?;
        put_state(&mut conn, state).await
    }
}
