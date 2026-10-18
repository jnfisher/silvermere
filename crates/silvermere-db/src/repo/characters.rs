//! Characters: `characters` and its child tables (WCCUSERS, the player record).
//!
//! Replaces the WCCUSERS reads: loading a player by userid (key 0), finding one by name
//! (key 1), the nearest-name scan (key 1), the top list (key 2, exp descending), the full
//! walks at startup and in the midnight cleanup, and the writes: save, create and
//! delete/reroll.
//!
//! The scalar column list [`SCALAR_COLS`] is checked against `layout::PLAYER` by a unit
//! test so the schema cannot drift from the record: every member of the player record is
//! either a column here, a child table, or one of the session-only members in
//! [`SESSION_ONLY`].

use std::collections::HashMap;

use silvermere_types::enums::Ability;
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{AccountId, CharacterId, ItemNo, MapId, RoomId, SpellNo};
use silvermere_types::records::Character;
use silvermere_types::sizes::{NABILITY, NINVITEMS, NKEYS, NSPELLBOOK, NSPELLCAST, NTRAIL, NWORN};

use crate::pool::Db;
use crate::repo::{
    Args, Batch, Conn, Get, Scope, ability_rows, delete_children, insert_sql, is_postgres,
    last_per_key, placeholders, read_abilities, select_sql,
};
use crate::{DbError, Result};

/// A stored character: its key, its account and the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRow {
    /// The primary key.
    pub id: CharacterId,
    /// The owning account.
    pub account: AccountId,
    /// The record.
    pub character: Character,
}

/// The scalar columns of `characters`, in bind order: the player record members that
/// are not arrays (those are the child tables) and not session-only.
pub const SCALAR_COLS: &[&str] = &[
    "userid",
    "firstname",
    "lastname",
    "exp",
    "title",
    "race",
    "class",
    "level",
    "basint",
    "baswil",
    "basstr",
    "bashea",
    "basagi",
    "bascha",
    "intel",
    "wil",
    "str",
    "hea",
    "agi",
    "cha",
    "maxhp",
    "hp",
    "maxwgt",
    "wgt",
    "maxenergy",
    "energy",
    "poison",
    "maxmr",
    "mr",
    "map",
    "room",
    "light",
    "hunger",
    "thirst",
    "ninvslots",
    "nkeyslots",
    "qflags2",
    "hpbonus",
    "manabonus",
    "expbil",
    "exprem",
    "nspellslots",
    "evil",
    "lawful",
    "verbose_mode",
    "bchannel",
    "fasttalk",
    "statline",
    "hidden",
    "perception",
    "stealth",
    "martial",
    "thievery",
    "maxmana",
    "mana",
    "spellcast",
    "traps",
    "disarm",
    "picklocks",
    "tracking",
    "copper",
    "silver",
    "gold",
    "platinum",
    "runic",
    "weapon",
    "autoengage",
    "lives",
    "aided",
    "rank",
    "lighttime",
    "readied",
    "delay",
    "roomlight",
    "gang",
    "hairlen",
    "haircol",
    "eyecol",
    "paging",
    "cp",
    "suicpw",
    "attackers",
    "status",
    "movedir",
    "dragging",
    "totalcp",
    "immortal",
    "pagecnt",
    "pagemax",
    "settings",
    "gossmax",
    "auctmax",
    "gosscnt",
    "auctcnt",
    "encumpct",
    "avbonus",
    "acbonus",
    "dmgbonus",
    "mabase",
    "arenahp",
    "qflags",
    "version",
    "msglevel",
    "pvpcnt",
    "randomhp",
    "resttick",
    "critbonus",
    "drbonus",
    "drpct",
    "energyuse",
    "shield",
    "defmod",
    "sesstime",
    "lifemins",
    "moves",
    "lastdate",
    "status2",
    "palette",
    "deathdate",
    "flags",
    "sex",
    "healrate",
    "manarate",
    "racechg",
    "lastmon",
    "lastmoncnt",
    "pvptimer",
    "medtick",
];

/// Player record members kept as child tables: `(table, members)`.
pub const CHILD_TABLES: &[(&str, &[&str])] = &[
    (
        "character_spells_cast",
        &["spells", "spellvals", "spellrnds"],
    ),
    ("character_inventory", &["items", "itemuses"]),
    ("character_keys", &["keys", "keyuses"]),
    ("character_spellbook", &["spellbook"]),
    ("character_worn", &["worn", "wornuses"]),
    ("character_trail", &["trailmaps", "trailrooms"]),
    ("character_abilities", &["abils", "abilvals"]),
];

/// Player record members that are session-only and not stored.
pub const SESSION_ONLY: &[&str] = &["leader", "group", "scanpos"];

/// Members renamed because the record's member name is a SQL keyword: `(member, column)`.
pub const RENAMED: &[(&str, &str)] = &[("verbose", "verbose_mode")];

fn scalar_args(c: &Character) -> Args {
    let mut a = Args::new();
    a.add(&c.userid)
        .add(&c.firstname)
        .add(&c.lastname)
        .add(c.exp)
        .add(&c.title)
        .add(c.race.0)
        .add(c.class.0)
        .add(c.level)
        .add(c.basint)
        .add(c.baswil)
        .add(c.basstr)
        .add(c.bashea)
        .add(c.basagi)
        .add(c.bascha)
        .add(c.intel)
        .add(c.wil)
        .add(c.str)
        .add(c.hea)
        .add(c.agi)
        .add(c.cha)
        .add(c.maxhp)
        .add(c.hp)
        .add(c.maxwgt)
        .add(c.wgt)
        .add(c.maxenergy)
        .add(c.energy)
        .add(c.poison)
        .add(c.maxmr)
        .add(c.mr)
        .add(c.map.0)
        .add(c.room.0)
        .add(c.light)
        .add(c.hunger)
        .add(c.thirst)
        .add(c.ninvslots)
        .add(c.nkeyslots)
        .add(c.qflags2)
        .add(c.hpbonus)
        .add(c.manabonus)
        .add(c.expbil)
        .add(c.exprem)
        .add(c.nspellslots)
        .add(c.evil)
        .add(c.lawful)
        .add(c.verbose)
        .add(c.bchannel)
        .add(c.fasttalk)
        .add(c.statline.code())
        .add(c.hidden)
        .add(c.perception)
        .add(c.stealth)
        .add(c.martial)
        .add(c.thievery)
        .add(c.maxmana)
        .add(c.mana)
        .add(c.spellcast)
        .add(c.traps)
        .add(c.disarm)
        .add(c.picklocks)
        .add(c.tracking)
        .coins(c.coins)
        .add(c.weapon.0)
        .add(c.autoengage)
        .add(c.lives)
        .add(c.aided)
        .add(c.rank)
        .add(c.lighttime)
        .add(c.readied)
        .add(c.delay)
        .add(c.roomlight)
        .add(&c.gang)
        .add(c.hairlen)
        .add(c.haircol)
        .add(c.eyecol)
        .add(c.paging)
        .add(c.cp)
        .add(&c.suicpw)
        .add(c.attackers)
        .add(c.status.bits())
        .add(c.movedir)
        .add(c.dragging)
        .add(c.totalcp)
        .add(c.immortal)
        .add(c.pagecnt)
        .add(c.pagemax)
        .add(c.settings.bits())
        .add(c.gossmax)
        .add(c.auctmax)
        .add(c.gosscnt)
        .add(c.auctcnt)
        .add(c.encumpct)
        .add(c.avbonus)
        .add(c.acbonus)
        .add(c.dmgbonus)
        .add(c.mabase)
        .add(c.arenahp)
        .add(c.qflags)
        .add(c.version)
        .add(c.msglevel.code())
        .add(c.pvpcnt)
        .add(c.randomhp)
        .add(c.resttick)
        .add(c.critbonus)
        .add(c.drbonus)
        .add(c.drpct)
        .add(c.energyuse)
        .add(c.shield)
        .add(c.defmod)
        .add(c.sesstime)
        .add(c.lifemins)
        .add(c.moves)
        .add(c.lastdate)
        .add(c.status2.bits())
        .add(c.palette)
        .add(c.deathdate)
        .add(c.flags.bits())
        .add(c.sex)
        .add(c.healrate)
        .add(c.manarate)
        .add(c.racechg)
        .add(c.lastmon)
        .add(c.lastmoncnt)
        .add(c.pvptimer)
        .add(c.medtick);
    debug_assert_eq!(a.len(), SCALAR_COLS.len());
    a
}

fn read_scalars(g: &Get<'_>) -> Result<Character> {
    Ok(Character {
        userid: g.text("userid")?,
        firstname: g.text("firstname")?,
        lastname: g.text("lastname")?,
        exp: g.int("exp")?,
        spells: [SpellNo(0); NSPELLCAST],
        spellvals: [0; NSPELLCAST],
        spellrnds: [0; NSPELLCAST],
        title: g.text("title")?,
        race: g.int::<u16>("race")?.into(),
        class: g.int::<u16>("class")?.into(),
        level: g.int("level")?,
        basint: g.int("basint")?,
        baswil: g.int("baswil")?,
        basstr: g.int("basstr")?,
        bashea: g.int("bashea")?,
        basagi: g.int("basagi")?,
        bascha: g.int("bascha")?,
        intel: g.int("intel")?,
        wil: g.int("wil")?,
        str: g.int("str")?,
        hea: g.int("hea")?,
        agi: g.int("agi")?,
        cha: g.int("cha")?,
        maxhp: g.int("maxhp")?,
        hp: g.int("hp")?,
        maxwgt: g.int("maxwgt")?,
        wgt: g.int("wgt")?,
        maxenergy: g.int("maxenergy")?,
        energy: g.int("energy")?,
        poison: g.int("poison")?,
        maxmr: g.int("maxmr")?,
        mr: g.int("mr")?,
        map: MapId(g.int("map")?),
        room: RoomId(g.int("room")?),
        light: g.int("light")?,
        hunger: g.int("hunger")?,
        thirst: g.int("thirst")?,
        ninvslots: g.int("ninvslots")?,
        items: vec![ItemNo(0); NINVITEMS],
        itemuses: vec![0; NINVITEMS],
        nkeyslots: g.int("nkeyslots")?,
        keys: vec![ItemNo(0); NKEYS],
        keyuses: vec![0; NKEYS],
        qflags2: g.int("qflags2")?,
        hpbonus: g.int("hpbonus")?,
        manabonus: g.int("manabonus")?,
        expbil: g.int("expbil")?,
        exprem: g.int("exprem")?,
        nspellslots: g.int("nspellslots")?,
        spellbook: vec![SpellNo(0); NSPELLBOOK],
        evil: g.int("evil")?,
        lawful: g.int("lawful")?,
        trailmaps: [MapId(0); NTRAIL],
        trailrooms: [RoomId(0); NTRAIL],
        verbose: g.int("verbose_mode")?,
        bchannel: g.int("bchannel")?,
        fasttalk: g.int("fasttalk")?,
        statline: g.code("statline")?,
        hidden: g.int("hidden")?,
        perception: g.int("perception")?,
        stealth: g.int("stealth")?,
        martial: g.int("martial")?,
        thievery: g.int("thievery")?,
        maxmana: g.int("maxmana")?,
        mana: g.int("mana")?,
        spellcast: g.int("spellcast")?,
        traps: g.int("traps")?,
        disarm: g.int("disarm")?,
        picklocks: g.int("picklocks")?,
        tracking: g.int("tracking")?,
        coins: g.coins("")?,
        weapon: ItemNo(g.int("weapon")?),
        worn: [ItemNo(0); NWORN],
        wornuses: [0; NWORN],
        autoengage: g.int("autoengage")?,
        lives: g.int("lives")?,
        aided: g.int("aided")?,
        rank: g.int("rank")?,
        lighttime: g.int("lighttime")?,
        readied: g.int("readied")?,
        delay: g.int("delay")?,
        roomlight: g.int("roomlight")?,
        gang: g.text("gang")?,
        hairlen: g.int("hairlen")?,
        haircol: g.int("haircol")?,
        eyecol: g.int("eyecol")?,
        paging: g.int("paging")?,
        cp: g.int("cp")?,
        suicpw: g.text("suicpw")?,
        attackers: g.int("attackers")?,
        status: PlayerStatus::from_bits_retain(g.int("status")?),
        movedir: g.int("movedir")?,
        dragging: g.int("dragging")?,
        totalcp: g.int("totalcp")?,
        immortal: g.int("immortal")?,
        pagecnt: g.int("pagecnt")?,
        pagemax: g.int("pagemax")?,
        settings: PlayerSettings::from_bits_retain(g.int("settings")?),
        gossmax: g.int("gossmax")?,
        auctmax: g.int("auctmax")?,
        gosscnt: g.int("gosscnt")?,
        auctcnt: g.int("auctcnt")?,
        encumpct: g.int("encumpct")?,
        avbonus: g.int("avbonus")?,
        acbonus: g.int("acbonus")?,
        dmgbonus: g.int("dmgbonus")?,
        mabase: g.int("mabase")?,
        arenahp: g.int("arenahp")?,
        qflags: g.int("qflags")?,
        version: g.int("version")?,
        msglevel: g.code("msglevel")?,
        pvpcnt: g.int("pvpcnt")?,
        randomhp: g.int("randomhp")?,
        abils: [(Ability::None, 0); NABILITY],
        resttick: g.int("resttick")?,
        critbonus: g.int("critbonus")?,
        drbonus: g.int("drbonus")?,
        drpct: g.int("drpct")?,
        energyuse: g.int("energyuse")?,
        shield: g.int("shield")?,
        defmod: g.int("defmod")?,
        sesstime: g.int("sesstime")?,
        lifemins: g.int("lifemins")?,
        moves: g.int("moves")?,
        lastdate: g.int("lastdate")?,
        status2: PlayerStatus2::from_bits_retain(g.int("status2")?),
        palette: g.int("palette")?,
        deathdate: g.int("deathdate")?,
        flags: PlayerFlags::from_bits_retain(g.int("flags")?),
        sex: g.int("sex")?,
        healrate: g.int("healrate")?,
        manarate: g.int("manarate")?,
        racechg: g.int("racechg")?,
        lastmon: g.int("lastmon")?,
        lastmoncnt: g.int("lastmoncnt")?,
        pvptimer: g.int("pvptimer")?,
        medtick: g.int("medtick")?,
    })
}

async fn write_children(conn: &mut Conn, id: i64, c: &Character) -> Result<()> {
    let mut b = Batch::default();
    child_rows(&mut b, id, c);
    b.flush(conn).await
}

/// The child-table rows of character `id`, added to `out`.
fn child_rows(out: &mut Batch, id: i64, c: &Character) {
    let col = "character_id";
    let mut rows = Vec::new();
    for (slot, spell) in c.spells.iter().enumerate().filter(|(_, s)| s.0 != 0) {
        let mut a = Args::new();
        a.add(id)
            .add(slot as u16)
            .add(spell.0)
            .add(c.spellvals.get(slot).copied().unwrap_or(0))
            .add(c.spellrnds.get(slot).copied().unwrap_or(0));
        rows.push(a);
    }
    out.insert(
        "character_spells_cast",
        &[col, "slot", "spell", "value", "rounds"],
        rows,
    );

    let mut rows = Vec::new();
    for (slot, item) in c.items.iter().enumerate().filter(|(_, i)| i.0 != 0) {
        let mut a = Args::new();
        a.add(id)
            .add(slot as u16)
            .add(item.0)
            .add(c.itemuses.get(slot).copied().unwrap_or(0));
        rows.push(a);
    }
    out.insert("character_inventory", &[col, "slot", "item", "uses"], rows);

    let mut rows = Vec::new();
    for (slot, item) in c.keys.iter().enumerate().filter(|(_, i)| i.0 != 0) {
        let mut a = Args::new();
        a.add(id)
            .add(slot as u16)
            .add(item.0)
            .add(c.keyuses.get(slot).copied().unwrap_or(0));
        rows.push(a);
    }
    out.insert("character_keys", &[col, "slot", "item", "uses"], rows);

    let rows: Vec<Args> = c
        .spellbook
        .iter()
        .enumerate()
        .filter(|(_, s)| s.0 != 0)
        .map(|(slot, s)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(s.0);
            a
        })
        .collect();
    out.insert("character_spellbook", &[col, "slot", "spell"], rows);

    let rows: Vec<Args> = c
        .worn
        .iter()
        .enumerate()
        .filter(|(_, i)| i.0 != 0)
        .map(|(slot, item)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(item.0).add(c.wornuses[slot]);
            a
        })
        .collect();
    out.insert("character_worn", &[col, "slot", "item", "uses"], rows);

    let rows: Vec<Args> = c
        .trailrooms
        .iter()
        .enumerate()
        .filter(|(slot, r)| r.0 != 0 || c.trailmaps[*slot].0 != 0)
        .map(|(slot, room)| {
            let mut a = Args::new();
            a.add(id)
                .add(slot as u16)
                .add(c.trailmaps[slot].0)
                .add(room.0);
            a
        })
        .collect();
    out.insert("character_trail", &[col, "slot", "map", "room"], rows);

    out.insert(
        "character_abilities",
        &[col, "slot", "ability", "value"],
        ability_rows(id, &c.abils),
    );
}

async fn clear_children(conn: &mut Conn, id: i64) -> Result<()> {
    for (table, _) in CHILD_TABLES {
        delete_children(conn, table, "character_id", id).await?;
    }
    Ok(())
}

/// Inserts `c` for `account` and returns its id. `DbError::Duplicate` when the account
/// already has a character or the userid or first name exists in any case.
pub(crate) async fn insert(
    conn: &mut Conn,
    account: AccountId,
    c: &Character,
) -> Result<CharacterId> {
    let mut cols = vec!["account_id"];
    cols.extend_from_slice(SCALAR_COLS);
    let mut a = Args::new();
    a.add(account.0).extend(scalar_args(c));
    let row = a
        .query(format!("{} RETURNING id", insert_sql("characters", &cols)))
        .fetch_one(&mut *conn)
        .await?;
    let id = Get::new(&row, "characters").i64("id")?;
    write_children(conn, id, c).await?;
    Ok(CharacterId(id))
}

/// Rewrites character `id` from `c` (`save_player`); `false` when there is no such row.
pub(crate) async fn update(conn: &mut Conn, id: CharacterId, c: &Character) -> Result<bool> {
    let sets: Vec<String> = SCALAR_COLS
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{col} = ${}", i + 1))
        .collect();
    let mut a = scalar_args(c);
    a.add(id.0);
    let done = a
        .query(format!(
            "UPDATE characters SET {} WHERE id = ${}",
            sets.join(", "),
            SCALAR_COLS.len() + 1
        ))
        .execute(&mut *conn)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    clear_children(conn, id.0).await?;
    write_children(conn, id.0, c).await?;
    Ok(true)
}

/// [`update`] for many characters in multi-row statements (T-113): which rows exist in
/// one read, then one upsert by id and, per child table, one delete and one insert.
/// `RowNotFound` when a character has no row, and nothing is written. The ids must not
/// repeat.
pub(crate) async fn update_many(
    conn: &mut Conn,
    chars: &[(CharacterId, &Character)],
) -> Result<()> {
    let mut accounts: HashMap<i64, i64> = HashMap::with_capacity(chars.len());
    for chunk in chars.chunks(1000) {
        let mut a = Args::new();
        for (id, _) in chunk {
            a.add(id.0);
        }
        // On PostgreSQL the rows are locked until the batch commits: a character deleted
        // directly (account delete, cleanup) meanwhile either is gone before this read
        // (`RowNotFound`) or waits for the commit, so the upsert below never inserts one
        // back. SQLite's single writer lock gives the same.
        let lock = if is_postgres(conn) { " FOR UPDATE" } else { "" };
        let sql = format!(
            "SELECT id, account_id FROM characters WHERE id IN ({}){lock}",
            placeholders(1, chunk.len())
        );
        for row in a.query(sql).fetch_all(&mut *conn).await? {
            let g = Get::new(&row, "characters");
            accounts.insert(g.i64("id")?, g.i64("account_id")?);
        }
    }
    if chars.iter().any(|(id, _)| !accounts.contains_key(&id.0)) {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    let mut b = Batch::default();
    // a repeated character keeps its last record, as one update per write would
    for (id, c) in last_per_key(chars, |(id, _)| id.0) {
        let mut a = Args::new();
        a.add(id.0).add(accounts[&id.0]).extend(scalar_args(c));
        b.upsert("characters", upsert_cols(), &["id"], id.0, a);
        for (table, _) in CHILD_TABLES {
            b.delete_children(table, "character_id", id.0);
        }
        child_rows(&mut b, id.0, c);
    }
    b.flush(conn).await
}

/// `id`, `account_id` and [`SCALAR_COLS`]: the columns of [`update_many`]'s upsert.
fn upsert_cols() -> &'static [&'static str] {
    static COLS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    COLS.get_or_init(|| {
        let mut cols = vec!["id", "account_id"];
        cols.extend_from_slice(SCALAR_COLS);
        cols
    })
}

/// Deletes character `id` (delbtv on delete and reroll).
pub(crate) async fn delete(conn: &mut Conn, id: CharacterId) -> Result<bool> {
    let mut a = Args::new();
    a.add(id.0);
    let done = a
        .query("DELETE FROM characters WHERE id = $1".to_string())
        .execute(&mut *conn)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Loads the characters in `scope` (over `id`) in `order` (an `ORDER BY ...` clause).
pub(crate) async fn load(conn: &mut Conn, scope: &Scope, order: &str) -> Result<Vec<CharacterRow>> {
    let (where_, a) = scope.child_where("id");
    let mut cols = vec!["id", "account_id"];
    cols.extend_from_slice(SCALAR_COLS);
    let rows = a
        .query(select_sql(
            "characters",
            &cols,
            &format!("{where_} {order}"),
        ))
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "characters");
        out.push(CharacterRow {
            id: CharacterId(g.i64("id")?),
            account: AccountId(g.i64("account_id")?),
            character: read_scalars(&g)?,
        });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let index: HashMap<i64, usize> = out.iter().enumerate().map(|(i, r)| (r.id.0, i)).collect();
    let col = "character_id";

    let (where_, a) = scope.child_where(col);
    for row in a
        .query(format!(
            "SELECT character_id, slot, spell, value, rounds FROM character_spells_cast {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "character_spells_cast");
        let Some(&i) = index.get(&g.i64(col)?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NSPELLCAST {
            let c = &mut out[i].character;
            c.spells[slot] = SpellNo(g.int("spell")?);
            c.spellvals[slot] = g.int("value")?;
            c.spellrnds[slot] = g.int("rounds")?;
        }
    }
    for (table, limit, is_keys) in [
        ("character_inventory", NINVITEMS, false),
        ("character_keys", NKEYS, true),
    ] {
        let (where_, a) = scope.child_where(col);
        for row in a
            .query(format!(
                "SELECT character_id, slot, item, uses FROM {table} {where_}"
            ))
            .fetch_all(&mut *conn)
            .await?
        {
            let g = Get::new(&row, "characters");
            let Some(&i) = index.get(&g.i64(col)?) else {
                continue;
            };
            let slot: usize = g.int("slot")?;
            if slot < limit {
                let c = &mut out[i].character;
                let (items, uses) = if is_keys {
                    (&mut c.keys, &mut c.keyuses)
                } else {
                    (&mut c.items, &mut c.itemuses)
                };
                items[slot] = ItemNo(g.int("item")?);
                uses[slot] = g.int("uses")?;
            }
        }
    }
    let (where_, a) = scope.child_where(col);
    for row in a
        .query(format!(
            "SELECT character_id, slot, spell FROM character_spellbook {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "character_spellbook");
        let Some(&i) = index.get(&g.i64(col)?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NSPELLBOOK {
            out[i].character.spellbook[slot] = SpellNo(g.int("spell")?);
        }
    }
    let (where_, a) = scope.child_where(col);
    for row in a
        .query(format!(
            "SELECT character_id, slot, item, uses FROM character_worn {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "character_worn");
        let Some(&i) = index.get(&g.i64(col)?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NWORN {
            out[i].character.worn[slot] = ItemNo(g.int("item")?);
            out[i].character.wornuses[slot] = g.int("uses")?;
        }
    }
    let (where_, a) = scope.child_where(col);
    for row in a
        .query(format!(
            "SELECT character_id, slot, map, room FROM character_trail {where_}"
        ))
        .fetch_all(&mut *conn)
        .await?
    {
        let g = Get::new(&row, "character_trail");
        let Some(&i) = index.get(&g.i64(col)?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NTRAIL {
            out[i].character.trailmaps[slot] = MapId(g.int("map")?);
            out[i].character.trailrooms[slot] = RoomId(g.int("room")?);
        }
    }
    for (id, abils) in read_abilities::<NABILITY>(conn, "character_abilities", col, scope).await? {
        if let Some(&i) = index.get(&id) {
            out[i].character.abils = abils;
        }
    }
    Ok(out)
}

/// Characters.
#[derive(Debug, Clone, Copy)]
pub struct Characters<'a> {
    db: &'a Db,
}

impl<'a> Characters<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    async fn one(&self, where_: String, args: Args) -> Result<Option<CharacterRow>> {
        let mut conn = self.db.pool().acquire().await?;
        let scope = Scope::In(format!("SELECT id FROM characters {where_}"), args);
        Ok(load(&mut conn, &scope, "ORDER BY id").await?.pop())
    }

    /// Character `id`.
    pub async fn get(&self, id: CharacterId) -> Result<Option<CharacterRow>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, &Scope::One(id.0), "ORDER BY id")
            .await?
            .pop())
    }

    /// The character of `account` (one per account).
    pub async fn by_account(&self, account: AccountId) -> Result<Option<CharacterRow>> {
        let mut a = Args::new();
        a.add(account.0);
        self.one("WHERE account_id = $1".to_string(), a).await
    }

    /// The character whose userid is `userid`, case-insensitive (key 0;
    /// WCCUSERS key 0 is case-insensitive).
    pub async fn by_userid(&self, userid: &str) -> Result<Option<CharacterRow>> {
        let mut a = Args::new();
        a.add(userid);
        self.one(
            format!("WHERE {}", self.db.dialect().ci_eq("userid", "$1")),
            a,
        )
        .await
    }

    /// The character whose first name is `name`, case-insensitive (key 1).
    pub async fn by_first_name(&self, name: &str) -> Result<Option<CharacterRow>> {
        let mut a = Args::new();
        a.add(name);
        self.one(
            format!("WHERE {}", self.db.dialect().ci_eq("firstname", "$1")),
            a,
        )
        .await
    }

    /// The key 1 scan behind name lookups (a player or gang member named by a prefix): the
    /// character with the first name at or after `name`, case-insensitive. The caller
    /// checks the prefix, as the original does afterwards.
    pub async fn by_first_name_prefix(&self, name: &str) -> Result<Option<CharacterRow>> {
        let d = self.db.dialect();
        let mut a = Args::new();
        a.add(name);
        self.one(
            format!(
                "WHERE {} ORDER BY {} LIMIT 1",
                d.ci_ge("firstname", "$1"),
                d.ci_column("firstname")
            ),
            a,
        )
        .await
    }

    /// The `n` characters with the most experience, descending (the top
    /// list; `NTOPLIST` 100).
    pub async fn top_by_exp(&self, n: usize) -> Result<Vec<CharacterRow>> {
        let mut conn = self.db.pool().acquire().await?;
        let mut a = Args::new();
        a.add(limit(n)?);
        let scope = Scope::In(
            "SELECT id FROM characters ORDER BY exp DESC, id LIMIT $1".to_string(),
            a,
        );
        load(&mut conn, &scope, "ORDER BY exp DESC, id").await
    }

    /// Every character in id order (the slobtv/snxbtv walks), passed to `f` one by one.
    pub async fn scan<F: FnMut(CharacterRow)>(&self, mut f: F) -> Result<usize> {
        let rows = self.all().await?;
        let n = rows.len();
        for row in rows {
            f(row);
        }
        Ok(n)
    }

    /// Every character in id order.
    pub async fn all(&self) -> Result<Vec<CharacterRow>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, &Scope::All, "ORDER BY id").await
    }

    /// How many characters there are (`cntrbtv`).
    pub async fn count(&self) -> Result<u64> {
        let row = Args::new()
            .query("SELECT COUNT(*) AS n FROM characters".to_string())
            .fetch_one(&self.db.pool())
            .await?;
        Get::new(&row, "characters").int("n")
    }

    /// Creates `c` for `account` (awaited, never queued). `DbError::Duplicate` when the
    /// account has a character or the userid or first name exists in any case.
    pub async fn insert(&self, account: AccountId, c: &Character) -> Result<CharacterId> {
        let mut tx = self.db.pool().begin().await?;
        let id = insert(&mut tx, account, c).await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Saves `c` as character `id` (`save_player`); `false` when there is no such row.
    pub async fn update(&self, id: CharacterId, c: &Character) -> Result<bool> {
        let mut tx = self.db.pool().begin().await?;
        let done = update(&mut tx, id, c).await?;
        tx.commit().await?;
        Ok(done)
    }

    /// Deletes character `id`.
    pub async fn delete(&self, id: CharacterId) -> Result<bool> {
        let mut conn = self.db.pool().acquire().await?;
        delete(&mut conn, id).await
    }
}

fn limit(n: usize) -> Result<i64> {
    i64::try_from(n).map_err(|_| DbError::Decode {
        table: "characters",
        column: "exp".to_string(),
        reason: "limit out of range".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silvermere_types::records::layout::PLAYER;

    /// Every member of the player record is a column, a child table or session-only, and
    /// every column is a member: the schema cannot drift from the record.
    #[test]
    fn scalar_columns_match_the_player_layout() {
        let children: Vec<&str> = CHILD_TABLES
            .iter()
            .flat_map(|(_, m)| m.iter().copied())
            .collect();
        let mut expected: Vec<&str> = PLAYER
            .fields
            .iter()
            .map(|f| f.name)
            .filter(|n| !n.starts_with("unk_"))
            .filter(|n| !SESSION_ONLY.contains(n))
            .filter(|n| !children.contains(n))
            .map(|n| {
                RENAMED
                    .iter()
                    .find(|(m, _)| *m == n)
                    .map_or(n, |(_, col)| *col)
            })
            .collect();
        let mut actual: Vec<&str> = SCALAR_COLS.to_vec();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert_eq!(
            SCALAR_COLS.len(),
            scalar_args(&crate::repo::characters::tests::blank()).len()
        );
    }

    /// The migration's `characters` table has exactly the scalar columns plus the keys.
    #[test]
    fn migration_columns_match() {
        for sql in [
            include_str!("../../../../migrations/sqlite/0001_schema.sql"),
            include_str!("../../../../migrations/postgres/0001_schema.sql"),
        ] {
            let start = sql
                .find("CREATE TABLE characters (")
                .expect("characters table");
            let body = &sql[start..];
            let end = body.find("\n);").expect("table end");
            let mut cols: Vec<&str> = body[..end]
                .lines()
                .skip(1)
                .filter_map(|l| l.split_whitespace().next())
                .filter(|c| !c.is_empty())
                .collect();
            cols.retain(|c| !["id", "account_id"].contains(c));
            assert_eq!(cols, SCALAR_COLS);
        }
    }

    pub(super) fn blank() -> Character {
        Character {
            userid: String::new(),
            firstname: String::new(),
            lastname: String::new(),
            exp: 0,
            spells: [SpellNo(0); NSPELLCAST],
            spellvals: [0; NSPELLCAST],
            spellrnds: [0; NSPELLCAST],
            title: String::new(),
            race: 0.into(),
            class: 0.into(),
            level: 0,
            basint: 0,
            baswil: 0,
            basstr: 0,
            bashea: 0,
            basagi: 0,
            bascha: 0,
            intel: 0,
            wil: 0,
            str: 0,
            hea: 0,
            agi: 0,
            cha: 0,
            maxhp: 0,
            hp: 0,
            maxwgt: 0,
            wgt: 0,
            maxenergy: 0,
            energy: 0,
            poison: 0,
            maxmr: 0,
            mr: 0,
            map: MapId(0),
            room: RoomId(0),
            light: 0,
            hunger: 0,
            thirst: 0,
            ninvslots: 0,
            items: Vec::new(),
            itemuses: Vec::new(),
            nkeyslots: 0,
            keys: Vec::new(),
            keyuses: Vec::new(),
            qflags2: 0,
            hpbonus: 0,
            manabonus: 0,
            expbil: 0,
            exprem: 0,
            nspellslots: 0,
            spellbook: Vec::new(),
            evil: 0,
            lawful: 0,
            trailmaps: [MapId(0); NTRAIL],
            trailrooms: [RoomId(0); NTRAIL],
            verbose: 0,
            bchannel: 0,
            fasttalk: 0,
            statline: silvermere_types::enums::StatlineMode::from_code(0),
            hidden: 0,
            perception: 0,
            stealth: 0,
            martial: 0,
            thievery: 0,
            maxmana: 0,
            mana: 0,
            spellcast: 0,
            traps: 0,
            disarm: 0,
            picklocks: 0,
            tracking: 0,
            coins: Default::default(),
            weapon: ItemNo(0),
            worn: [ItemNo(0); NWORN],
            wornuses: [0; NWORN],
            autoengage: 0,
            lives: 0,
            aided: 0,
            rank: 0,
            lighttime: 0,
            readied: 0,
            delay: 0,
            roomlight: 0,
            gang: String::new(),
            hairlen: 0,
            haircol: 0,
            eyecol: 0,
            paging: 0,
            cp: 0,
            suicpw: String::new(),
            attackers: 0,
            status: PlayerStatus::empty(),
            movedir: 0,
            dragging: 0,
            totalcp: 0,
            immortal: 0,
            pagecnt: 0,
            pagemax: 0,
            settings: PlayerSettings::empty(),
            gossmax: 0,
            auctmax: 0,
            gosscnt: 0,
            auctcnt: 0,
            encumpct: 0,
            avbonus: 0,
            acbonus: 0,
            dmgbonus: 0,
            mabase: 0,
            arenahp: 0,
            qflags: 0,
            version: 0,
            msglevel: silvermere_types::enums::MessageLevel::from_code(0),
            pvpcnt: 0,
            randomhp: 0,
            abils: [(Ability::None, 0); NABILITY],
            resttick: 0,
            critbonus: 0,
            drbonus: 0,
            drpct: 0,
            energyuse: 0,
            shield: 0,
            defmod: 0,
            sesstime: 0,
            lifemins: 0,
            moves: 0,
            lastdate: 0,
            status2: PlayerStatus2::empty(),
            palette: 0,
            deathdate: 0,
            flags: PlayerFlags::empty(),
            sex: 0,
            healrate: 0,
            manarate: 0,
            racechg: 0,
            lastmon: 0,
            lastmoncnt: 0,
            pvptimer: 0,
            medtick: 0,
        }
    }
}
