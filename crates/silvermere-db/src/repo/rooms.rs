//! Rooms: `rooms`, `room_exits`, `room_placed_items` (content) and `room_state`,
//! `room_floor_items`, `room_monsters` (state).
//!
//! Replaces the WCCMP001 reads of one room (key 0), the whole-map walks of the preload and
//! the highest room number.

use std::collections::{HashMap, HashSet};

use silvermere_types::enums::ExitType;
use silvermere_types::flags::RoomFlags;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, RoomId, RoomKey};
use silvermere_types::records::{Coins, Exit, Room, RoomItem};
use silvermere_types::sizes::{NDIRS, NHIDITEMS, NPLACED, NROOMITEMS, NROOMMONS};

use crate::pool::Db;
use crate::repo::{
    Args, Conn, Get, Scope, StateWrite, delete_children, delete_children_in, insert_many,
    insert_many_bulk, parents_per_chunk, select_sql, upsert_many, upsert_sql,
};
use crate::{DbError, Result};

/// The live part of `struct room`, one-to-one with the content row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomState {
    /// The room.
    pub key: RoomKey,
    /// Coins lying here.
    pub coins: Coins,
    /// Hidden coins.
    pub hidcoins: Coins,
    /// Owner's user-id (`RF_OWNED`).
    pub owner: String,
    /// `RF_SPECALIVE`: the room's specific monster is alive (set when it is generated,
    /// the room update clears it). The other `RF_*` bits are content, in `rooms.flags`.
    pub specalive: bool,
    /// Hidden slots found.
    pub foundmask: u32,
    /// Minute of last kill.
    pub lastkill: u16,
    /// Area monsters now.
    pub areacount: i16,
    /// Players who found hidden items.
    pub searchers: u8,
    /// Monsters generated here now.
    pub nummonsters: u8,
    /// Visible items.
    pub items: [RoomItem; NROOMITEMS],
    /// Hidden items.
    pub hiditems: [RoomItem; NHIDITEMS],
    /// Active monsters here.
    pub monsters: [ActiveMonsterId; NROOMMONS],
}

impl RoomState {
    /// The live part of `room`.
    pub fn of(room: &Room) -> Self {
        Self {
            key: room.key,
            coins: room.coins,
            hidcoins: room.hidcoins,
            owner: room.owner.clone(),
            specalive: room.flags.contains(RoomFlags::SPECALIVE),
            foundmask: room.foundmask,
            lastkill: room.lastkill,
            areacount: room.areacount,
            searchers: room.searchers,
            nummonsters: room.nummonsters,
            items: room.items,
            hiditems: room.hiditems,
            monsters: room.monsters,
        }
    }

    /// An empty state for a room with nothing live yet.
    pub fn empty(key: RoomKey) -> Self {
        Self {
            key,
            coins: Coins::default(),
            hidcoins: Coins::default(),
            owner: String::new(),
            specalive: false,
            foundmask: 0,
            lastkill: 0,
            areacount: 0,
            searchers: 0,
            nummonsters: 0,
            items: [EMPTY_ITEM; NROOMITEMS],
            hiditems: [EMPTY_ITEM; NHIDITEMS],
            monsters: [ActiveMonsterId(0); NROOMMONS],
        }
    }

    /// Copies this state into `room`.
    pub fn apply_to(&self, room: &mut Room) {
        room.coins = self.coins;
        room.hidcoins = self.hidcoins;
        room.owner = self.owner.clone();
        room.flags.set(RoomFlags::SPECALIVE, self.specalive);
        room.foundmask = self.foundmask;
        room.lastkill = self.lastkill;
        room.areacount = self.areacount;
        room.searchers = self.searchers;
        room.nummonsters = self.nummonsters;
        room.items = self.items;
        room.hiditems = self.hiditems;
        room.monsters = self.monsters;
    }
}

const EMPTY_ITEM: RoomItem = RoomItem {
    item: ItemNo(0),
    uses: 0,
    count: 0,
};

const EMPTY_EXIT: Exit = Exit {
    target: RoomId(0),
    kind: ExitType::Normal,
    par1: 0,
    par2: 0,
    par3: 0,
    par4: 0,
};

const ROOM_COLS: &[&str] = &[
    "map",
    "room",
    "name",
    "desc0",
    "desc1",
    "desc2",
    "desc3",
    "desc4",
    "desc5",
    "desc6",
    "mapfile",
    "room_type",
    "shopnum",
    "minlevel",
    "maxlevel",
    "genmonster",
    "light",
    "ganghouse",
    "maxmonsters",
    "mongroup",
    "flags",
    "deathroom",
    "huproom",
    "cmdtext",
    "regendelay",
    "areamax",
    "ctlroom",
    "specmon",
    "protmsg",
    "picktrap",
    "roomspell",
];

const STATE_COLS: &[&str] = &[
    "room_id",
    "copper",
    "silver",
    "gold",
    "platinum",
    "runic",
    "hidcopper",
    "hidsilver",
    "hidgold",
    "hidplatinum",
    "hidrunic",
    "owner",
    "specalive",
    "foundmask",
    "lastkill",
    "areacount",
    "searchers",
    "nummonsters",
];

fn room_args(room: &Room) -> Args {
    let mut a = Args::new();
    a.add(room.key.map.0).add(room.key.room.0).add(&room.name);
    for line in &room.desc {
        a.add(line);
    }
    a.add(&room.mapfile)
        .add(room.room_type.code())
        .add(room.shopnum.0)
        .add(room.minlevel)
        .add(room.maxlevel)
        .add(room.genmonster.0)
        .add(room.light)
        .add(room.ganghouse.code())
        .add(room.maxmonsters)
        .add(room.mongroup)
        .add((room.flags - RoomFlags::SPECALIVE).bits())
        .add(room.deathroom.0)
        .add(room.huproom.0)
        .add(room.cmdtext.0)
        .add(room.regendelay)
        .add(room.areamax)
        .add(room.ctlroom.0)
        .add(room.specmon.0)
        .add(room.protmsg.0)
        .add(room.picktrap)
        .add(room.roomspell);
    a
}

fn state_args(room_id: i64, s: &RoomState) -> Args {
    let mut a = Args::new();
    a.add(room_id)
        .coins(s.coins)
        .add(s.hidcoins.copper)
        .add(s.hidcoins.silver)
        .add(s.hidcoins.gold)
        .add(s.hidcoins.platinum)
        .add(s.hidcoins.runic)
        .add(&s.owner)
        .add(s.specalive)
        .add(s.foundmask)
        .add(s.lastkill)
        .add(s.areacount)
        .add(s.searchers)
        .add(s.nummonsters);
    a
}

/// The surrogate id of a room, if it exists.
pub(crate) async fn room_id(conn: &mut Conn, key: RoomKey) -> Result<Option<i64>> {
    let mut a = Args::new();
    a.add(key.map.0).add(key.room.0);
    let row = a
        .query("SELECT id FROM rooms WHERE map = $1 AND room = $2".to_string())
        .fetch_optional(&mut *conn)
        .await?;
    row.map(|r| Get::new(&r, "rooms").i64("id")).transpose()
}

/// Writes the content rows of `room` (upsert) and returns the room id. Children are
/// replaced.
pub(crate) async fn put_content(conn: &mut Conn, room: &Room) -> Result<i64> {
    let sql = format!(
        "{} RETURNING id",
        upsert_sql("rooms", ROOM_COLS, &["map", "room"])
    );
    let row = room_args(room).query(sql).fetch_one(&mut *conn).await?;
    let id = Get::new(&row, "rooms").i64("id")?;
    delete_children(conn, "room_exits", "room_id", id).await?;
    delete_children(conn, "room_placed_items", "room_id", id).await?;
    let exits: Vec<Args> = room
        .exits
        .iter()
        .enumerate()
        .filter(|(_, e)| **e != EMPTY_EXIT)
        .map(|(slot, e)| {
            let mut a = Args::new();
            a.add(id)
                .add(slot as u16)
                .add(slot as u16)
                .add(e.target.0)
                .add(e.kind.code())
                .add(e.par1)
                .add(e.par2)
                .add(e.par3)
                .add(e.par4);
            a
        })
        .collect();
    insert_many(
        conn,
        "room_exits",
        &[
            "room_id",
            "slot",
            "direction",
            "target",
            "exit_type",
            "par1",
            "par2",
            "par3",
            "par4",
        ],
        exits,
    )
    .await?;
    let placed: Vec<Args> = room
        .placed
        .iter()
        .enumerate()
        .filter(|(_, item)| item.0 != 0)
        .map(|(slot, item)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(item.0);
            a
        })
        .collect();
    insert_many(
        conn,
        "room_placed_items",
        &["room_id", "slot", "item"],
        placed,
    )
    .await?;
    Ok(id)
}

/// Writes `state` for room `room_id` (upsert), replacing the floor items and monsters.
pub(crate) async fn put_state_by_id(
    conn: &mut Conn,
    room_id: i64,
    state: &RoomState,
) -> Result<()> {
    state_args(room_id, state)
        .query(upsert_sql("room_state", STATE_COLS, &["room_id"]))
        .execute(&mut *conn)
        .await?;
    delete_children(conn, "room_floor_items", "room_id", room_id).await?;
    delete_children(conn, "room_monsters", "room_id", room_id).await?;
    let mut floor = Vec::new();
    for (hidden, items) in [(false, &state.items[..]), (true, &state.hiditems[..])] {
        for (slot, it) in items.iter().enumerate().filter(|(_, it)| it.item.0 != 0) {
            let mut a = Args::new();
            a.add(room_id)
                .add(hidden)
                .add(slot as u16)
                .add(it.item.0)
                .add(it.uses)
                .add(it.count);
            floor.push(a);
        }
    }
    insert_many(
        conn,
        "room_floor_items",
        &["room_id", "hidden", "slot", "item", "uses", "count"],
        floor,
    )
    .await?;
    let monsters: Vec<Args> = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, m)| m.0 != 0)
        .map(|(slot, m)| {
            let mut a = Args::new();
            a.add(room_id).add(slot as u16).add(m.0);
            a
        })
        .collect();
    insert_many(
        conn,
        "room_monsters",
        &["room_id", "slot", "active_monster"],
        monsters,
    )
    .await
}

/// Writes `state` for the room it names; an error when the room does not exist.
pub(crate) async fn put_state(conn: &mut Conn, state: &RoomState) -> Result<()> {
    let Some(id) = room_id(conn, state.key).await? else {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    };
    put_state_by_id(conn, id, state).await
}

/// Rooms per batch of [`Rooms::put_many`] on SQLite (each batch is a few multi-row
/// statements; ten times as many on PostgreSQL, [`parents_per_chunk`]).
const BULK_ROOMS: usize = 500;

const EXIT_COLS: &[&str] = &[
    "room_id",
    "slot",
    "direction",
    "target",
    "exit_type",
    "par1",
    "par2",
    "par3",
    "par4",
];

/// The `room_exits` and `room_placed_items` rows of `room` (id `id`): the used slots.
fn content_child_rows(id: i64, room: &Room) -> (Vec<Args>, Vec<Args>) {
    let exits = room
        .exits
        .iter()
        .enumerate()
        .filter(|(_, e)| **e != EMPTY_EXIT)
        .map(|(slot, e)| {
            let mut a = Args::new();
            a.add(id)
                .add(slot as u16)
                .add(slot as u16)
                .add(e.target.0)
                .add(e.kind.code())
                .add(e.par1)
                .add(e.par2)
                .add(e.par3)
                .add(e.par4);
            a
        })
        .collect();
    let placed = room
        .placed
        .iter()
        .enumerate()
        .filter(|(_, item)| item.0 != 0)
        .map(|(slot, item)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(item.0);
            a
        })
        .collect();
    (exits, placed)
}

/// The `room_floor_items` and `room_monsters` rows of `state` (room id `id`).
fn state_child_rows(id: i64, state: &RoomState) -> (Vec<Args>, Vec<Args>) {
    let mut floor = Vec::new();
    for (hidden, items) in [(false, &state.items[..]), (true, &state.hiditems[..])] {
        for (slot, it) in items.iter().enumerate().filter(|(_, it)| it.item.0 != 0) {
            let mut a = Args::new();
            a.add(id)
                .add(hidden)
                .add(slot as u16)
                .add(it.item.0)
                .add(it.uses)
                .add(it.count);
            floor.push(a);
        }
    }
    let monsters = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, m)| m.0 != 0)
        .map(|(slot, m)| {
            let mut a = Args::new();
            a.add(id).add(slot as u16).add(m.0);
            a
        })
        .collect();
    (floor, monsters)
}

/// Bulk content and state write (see [`Rooms::put_many`]), on an open transaction.
pub(crate) async fn put_many(conn: &mut Conn, rooms: &[Room], state: StateWrite) -> Result<()> {
    // Rooms that already have state keep it unless the mode resets it.
    let mut has_state: HashSet<RoomKey> = HashSet::new();
    if state == StateWrite::CreateMissing {
        let sql = "SELECT r.map, r.room FROM rooms r JOIN room_state s ON s.room_id = r.id";
        for row in Args::new()
            .query(sql.to_string())
            .fetch_all(&mut *conn)
            .await?
        {
            let g = Get::new(&row, "rooms");
            has_state.insert(RoomKey::new(g.int("map")?, g.int("room")?));
        }
    }
    let per_chunk = parents_per_chunk(conn, BULK_ROOMS);
    for chunk in rooms.chunks(per_chunk) {
        let returned = upsert_many(
            conn,
            "rooms",
            ROOM_COLS,
            &["map", "room"],
            chunk.iter().map(room_args).collect(),
            Some("id, map, room"),
        )
        .await?;
        let mut ids: HashMap<RoomKey, i64> = HashMap::with_capacity(returned.len());
        for row in &returned {
            let g = Get::new(row, "rooms");
            ids.insert(RoomKey::new(g.int("map")?, g.int("room")?), g.i64("id")?);
        }
        let id_of = |room: &Room| {
            ids.get(&room.key).copied().ok_or_else(|| DbError::Decode {
                table: "rooms",
                column: "id".to_string(),
                reason: format!("no id returned for {}/{}", room.key.map, room.key.room),
            })
        };
        let chunk_ids: Vec<i64> = chunk.iter().map(id_of).collect::<Result<_>>()?;
        delete_children_in(conn, "room_exits", "room_id", &chunk_ids).await?;
        delete_children_in(conn, "room_placed_items", "room_id", &chunk_ids).await?;
        let (mut exits, mut placed) = (Vec::new(), Vec::new());
        for (room, &id) in chunk.iter().zip(&chunk_ids) {
            let (e, p) = content_child_rows(id, room);
            exits.extend(e);
            placed.extend(p);
        }
        insert_many_bulk(conn, "room_exits", EXIT_COLS, exits).await?;
        insert_many_bulk(
            conn,
            "room_placed_items",
            &["room_id", "slot", "item"],
            placed,
        )
        .await?;

        let states: Vec<(i64, RoomState)> = chunk
            .iter()
            .zip(&chunk_ids)
            .filter(|(room, _)| state.writes(has_state.contains(&room.key)))
            .map(|(room, &id)| (id, RoomState::of(room)))
            .collect();
        write_states(conn, states.iter().map(|(id, s)| (*id, s))).await?;
    }
    Ok(())
}

/// Upserts room states by room id with their floor items and monsters replaced, in
/// multi-row statements. The ids must not repeat.
async fn write_states<'s>(
    conn: &mut Conn,
    states: impl Iterator<Item = (i64, &'s RoomState)>,
) -> Result<()> {
    let mut state_ids = Vec::new();
    let (mut state_rows, mut floor, mut monsters) = (Vec::new(), Vec::new(), Vec::new());
    for (id, s) in states {
        state_ids.push(id);
        state_rows.push(state_args(id, s));
        let (f, m) = state_child_rows(id, s);
        floor.extend(f);
        monsters.extend(m);
    }
    if state_ids.is_empty() {
        return Ok(());
    }
    upsert_many(
        conn,
        "room_state",
        STATE_COLS,
        &["room_id"],
        state_rows,
        None,
    )
    .await?;
    delete_children_in(conn, "room_floor_items", "room_id", &state_ids).await?;
    delete_children_in(conn, "room_monsters", "room_id", &state_ids).await?;
    insert_many_bulk(
        conn,
        "room_floor_items",
        &["room_id", "hidden", "slot", "item", "uses", "count"],
        floor,
    )
    .await?;
    insert_many_bulk(
        conn,
        "room_monsters",
        &["room_id", "slot", "active_monster"],
        monsters,
    )
    .await
}

/// [`put_state`] for many rooms (the persistence worker's batch, T-113): the room ids in
/// one read, then [`write_states`]. `RowNotFound` when a room does not exist, and nothing
/// is written. The keys must not repeat.
pub(crate) async fn put_states(conn: &mut Conn, states: &[&RoomState]) -> Result<()> {
    let mut ids: HashMap<RoomKey, i64> = HashMap::with_capacity(states.len());
    for chunk in states.chunks(1000) {
        let mut a = Args::new();
        let mut tuples = Vec::with_capacity(chunk.len());
        for s in chunk {
            tuples.push(format!("(${}, ${})", a.len() + 1, a.len() + 2));
            a.add(s.key.map.0).add(s.key.room.0);
        }
        let sql = format!(
            "SELECT id, map, room FROM rooms WHERE (map, room) IN (VALUES {})",
            tuples.join(", ")
        );
        for row in a.query(sql).fetch_all(&mut *conn).await? {
            let g = Get::new(&row, "rooms");
            ids.insert(RoomKey::new(g.int("map")?, g.int("room")?), g.i64("id")?);
        }
    }
    let with_ids: Vec<(i64, &RoomState)> = states
        .iter()
        .map(|s| {
            ids.get(&s.key)
                .map(|&id| (id, *s))
                .ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
        })
        .collect::<Result<_>>()?;
    write_states(conn, with_ids.into_iter()).await
}

/// Loads the rooms in `scope` (a subquery over `rooms`), content and state merged, in
/// `(map, room)` order.
pub(crate) async fn load(conn: &mut Conn, db: &Db, scope: &Scope) -> Result<Vec<Room>> {
    let (where_, args) = scope.child_where("id");
    let mut cols = vec!["id"];
    cols.extend_from_slice(ROOM_COLS);
    let sql = select_sql("rooms", &cols, &format!("{where_} ORDER BY map, room"));
    let rows = args.query(sql).fetch_all(&mut *conn).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::with_capacity(rows.len());
    let mut rooms = Vec::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "rooms");
        ids.push(g.i64("id")?);
        rooms.push(Room {
            key: RoomKey {
                map: MapId(g.int("map")?),
                room: RoomId(g.int("room")?),
            },
            name: g.text("name")?,
            desc: [
                g.text("desc0")?,
                g.text("desc1")?,
                g.text("desc2")?,
                g.text("desc3")?,
                g.text("desc4")?,
                g.text("desc5")?,
                g.text("desc6")?,
            ],
            mapfile: g.text("mapfile")?,
            exits: [EMPTY_EXIT; NDIRS],
            monsters: [ActiveMonsterId(0); NROOMMONS],
            room_type: g.code("room_type")?,
            shopnum: g.int::<i32>("shopnum")?.into(),
            owner: String::new(),
            minlevel: g.int("minlevel")?,
            maxlevel: g.int("maxlevel")?,
            genmonster: g.int::<i32>("genmonster")?.into(),
            light: g.int("light")?,
            ganghouse: g.code("ganghouse")?,
            items: [EMPTY_ITEM; NROOMITEMS],
            hiditems: [EMPTY_ITEM; NHIDITEMS],
            coins: Coins::default(),
            hidcoins: Coins::default(),
            maxmonsters: g.int("maxmonsters")?,
            mongroup: g.int("mongroup")?,
            lastkill: 0,
            flags: RoomFlags::from_bits_retain(g.int("flags")?) - RoomFlags::SPECALIVE,
            foundmask: 0,
            deathroom: RoomId(g.int("deathroom")?),
            huproom: RoomId(g.int("huproom")?),
            cmdtext: g.int::<i32>("cmdtext")?.into(),
            regendelay: g.int("regendelay")?,
            areamax: g.int("areamax")?,
            areacount: 0,
            ctlroom: RoomId(g.int("ctlroom")?),
            specmon: g.int::<i32>("specmon")?.into(),
            placed: [ItemNo(0); NPLACED],
            protmsg: g.int::<i32>("protmsg")?.into(),
            picktrap: g.int("picktrap")?,
            roomspell: g.int("roomspell")?,
            searchers: 0,
            nummonsters: 0,
        });
    }
    let index: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    // exits
    let (where_, args) = scope.child_where("room_id");
    let sql = format!(
        "SELECT room_id, slot, target, exit_type, par1, par2, par3, par4 FROM room_exits {where_}"
    );
    for row in args.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "room_exits");
        let Some(&i) = index.get(&g.i64("room_id")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NDIRS {
            rooms[i].exits[slot] = Exit {
                target: RoomId(g.int("target")?),
                kind: g.code("exit_type")?,
                par1: g.int("par1")?,
                par2: g.int("par2")?,
                par3: g.int("par3")?,
                par4: g.int("par4")?,
            };
        }
    }
    // placed items
    let (where_, args) = scope.child_where("room_id");
    let sql = format!("SELECT room_id, slot, item FROM room_placed_items {where_}");
    for row in args.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "room_placed_items");
        let Some(&i) = index.get(&g.i64("room_id")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NPLACED {
            rooms[i].placed[slot] = ItemNo(g.int("item")?);
        }
    }
    // state
    let states = load_states(conn, db, scope, |id| index.get(&id).map(|i| rooms[*i].key)).await?;
    for (id, state) in states {
        if let Some(&i) = index.get(&id) {
            state.apply_to(&mut rooms[i]);
        }
    }
    Ok(rooms)
}

/// Loads the states of the rooms in `scope` keyed by room id; `key_of` names each room.
async fn load_states(
    conn: &mut Conn,
    db: &Db,
    scope: &Scope,
    key_of: impl Fn(i64) -> Option<RoomKey>,
) -> Result<Vec<(i64, RoomState)>> {
    let (where_, args) = scope.child_where("room_id");
    let sql = select_sql("room_state", STATE_COLS, &where_);
    let rows = args.query(sql).fetch_all(&mut *conn).await?;
    let mut states: Vec<(i64, RoomState)> = Vec::with_capacity(rows.len());
    let mut index: HashMap<i64, usize> = HashMap::with_capacity(rows.len());
    for row in &rows {
        let g = Get::new(row, "room_state");
        let id = g.i64("room_id")?;
        let Some(key) = key_of(id) else { continue };
        let state = RoomState {
            key,
            coins: g.coins("")?,
            hidcoins: g.coins("hid")?,
            owner: g.text("owner")?,
            specalive: db.dialect().get_bool(row, "specalive")?,
            foundmask: g.int("foundmask")?,
            lastkill: g.int("lastkill")?,
            areacount: g.int("areacount")?,
            searchers: g.int("searchers")?,
            nummonsters: g.int("nummonsters")?,
            items: [EMPTY_ITEM; NROOMITEMS],
            hiditems: [EMPTY_ITEM; NHIDITEMS],
            monsters: [ActiveMonsterId(0); NROOMMONS],
        };
        index.insert(id, states.len());
        states.push((id, state));
    }
    if states.is_empty() {
        return Ok(states);
    }
    let (where_, args) = scope.child_where("room_id");
    let sql =
        format!("SELECT room_id, hidden, slot, item, uses, count FROM room_floor_items {where_}");
    for row in args.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "room_floor_items");
        let Some(&i) = index.get(&g.i64("room_id")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        let item = RoomItem {
            item: ItemNo(g.int("item")?),
            uses: g.int("uses")?,
            count: g.int("count")?,
        };
        let hidden = db.dialect().get_bool(&row, "hidden")?;
        let state = &mut states[i].1;
        if hidden {
            if slot < NHIDITEMS {
                state.hiditems[slot] = item;
            }
        } else if slot < NROOMITEMS {
            state.items[slot] = item;
        }
    }
    let (where_, args) = scope.child_where("room_id");
    let sql = format!("SELECT room_id, slot, active_monster FROM room_monsters {where_}");
    for row in args.query(sql).fetch_all(&mut *conn).await? {
        let g = Get::new(&row, "room_monsters");
        let Some(&i) = index.get(&g.i64("room_id")?) else {
            continue;
        };
        let slot: usize = g.int("slot")?;
        if slot < NROOMMONS {
            states[i].1.monsters[slot] = ActiveMonsterId(g.int("active_monster")?);
        }
    }
    Ok(states)
}

fn one_room_scope(key: RoomKey) -> Scope {
    let mut args = Args::new();
    args.add(key.map.0).add(key.room.0);
    Scope::In(
        "SELECT id FROM rooms WHERE map = $1 AND room = $2".to_string(),
        args,
    )
}

fn map_scope(map: MapId) -> Scope {
    let mut args = Args::new();
    args.add(map.0);
    Scope::In("SELECT id FROM rooms WHERE map = $1".to_string(), args)
}

/// Room content with its state merged back into the `Room` record.
#[derive(Debug, Clone, Copy)]
pub struct Rooms<'a> {
    db: &'a Db,
}

impl<'a> Rooms<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The room at `key` (key 0).
    pub async fn get(&self, key: RoomKey) -> Result<Option<Room>> {
        let mut conn = self.db.pool().acquire().await?;
        Ok(load(&mut conn, self.db, &one_room_scope(key)).await?.pop())
    }

    /// The highest room number on `map` (per map; the last record of key 0 is the
    /// highest room of the highest map).
    pub async fn highest_in_map(&self, map: MapId) -> Result<Option<RoomId>> {
        let mut a = Args::new();
        a.add(map.0);
        let row = a
            .query("SELECT room FROM rooms WHERE map = $1 ORDER BY room DESC LIMIT 1".to_string())
            .fetch_optional(&self.db.pool())
            .await?;
        row.map(|r| Get::new(&r, "rooms").int("room").map(RoomId))
            .transpose()
    }

    /// Every room on `map` in room order.
    pub async fn all_in_map(&self, map: MapId) -> Result<Vec<Room>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, self.db, &map_scope(map)).await
    }

    /// Every room in `(map, room)` order: the startup load.
    pub async fn all(&self) -> Result<Vec<Room>> {
        let mut conn = self.db.pool().acquire().await?;
        load(&mut conn, self.db, &Scope::All).await
    }

    /// Inserts `room`: content plus its state (a fresh import copies the virgin state
    /// from the record). `DbError::Duplicate` when the key exists.
    pub async fn insert(&self, room: &Room) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        if room_id(&mut tx, room.key).await?.is_some() {
            return Err(DbError::Duplicate(format!(
                "room {}/{}",
                room.key.map, room.key.room
            )));
        }
        let id = put_content(&mut tx, room).await?;
        put_state_by_id(&mut tx, id, &RoomState::of(room)).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Upserts the content of `room`; the state is created from the record only when the
    /// room is new (a content-only re-import leaves play state alone).
    pub async fn put_content(&self, room: &Room) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        let existed = room_id(&mut tx, room.key).await?.is_some();
        let id = put_content(&mut tx, room).await?;
        if !existed {
            put_state_by_id(&mut tx, id, &RoomState::of(room)).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Writes many rooms in one transaction: content upserted by `(map, room)` (exits and
    /// placed items replaced), state per `state` from the record, in multi-row
    /// statements, 500 rooms per batch. The importer's table writer; `rooms` must not
    /// repeat a key.
    pub async fn put_many(&self, rooms: &[Room], state: StateWrite) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put_many(&mut tx, rooms, state).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the room and everything keyed to it.
    pub async fn delete(&self, key: RoomKey) -> Result<bool> {
        let mut a = Args::new();
        a.add(key.map.0).add(key.room.0);
        let done = a
            .query("DELETE FROM rooms WHERE map = $1 AND room = $2".to_string())
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }
}

/// The live part of rooms.
#[derive(Debug, Clone, Copy)]
pub struct RoomStates<'a> {
    db: &'a Db,
}

impl<'a> RoomStates<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// The state of the room at `key`.
    pub async fn get(&self, key: RoomKey) -> Result<Option<RoomState>> {
        let mut conn = self.db.pool().acquire().await?;
        let mut states =
            load_states(&mut conn, self.db, &one_room_scope(key), |_| Some(key)).await?;
        Ok(states.pop().map(|(_, s)| s))
    }

    /// Writes `state` (the room's write-back). The room must exist.
    pub async fn put(&self, state: &RoomState) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        put_state(&mut tx, state).await?;
        tx.commit().await?;
        Ok(())
    }
}
