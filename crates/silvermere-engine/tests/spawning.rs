//! T-035: the spawn pass, `generate_monster`, wandering, following, the active-monster
//! numbers and their persistence, and the start-up exit reset, on a hand-built world
//! with a seeded `Rng`.
//!
//! The arena is Newhaven Arena 1:2150 as the stock data has it (type Arena, `mongroup`
//! 6, levels 1-3, `maxmonsters` 2) with the group-6 genindex-1 monsters of the stock
//! WCCKNMSR (giant rat, lashworm, filthbug, carrion beast, acid slime, kobold thief).

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::{TestClock, drain, enter, test_config, tiny_snapshot};
use silvermere_engine::monsters::active::remove_active_monster;
use silvermere_engine::monsters::following::fast_update_monster;
use silvermere_engine::monsters::generate::{Request, generate_monster};
use silvermere_engine::monsters::move_monster::{Moved, move_monster};
use silvermere_engine::movement::exits::live_exit;
use silvermere_engine::movement::move_user::handle_following_monsters;
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineOutput, PersistRequest, Routine, SeededRng, SessionId, World, WorldSnapshot,
};
use silvermere_text::strip::strip_ansi;
use silvermere_types::enums::{
    Direction, ExitType, GroupRole, MessageLevel, MonsterType, RoomType,
};
use silvermere_types::flags::{ActiveMonsterFlags, RoomFlags};
use silvermere_types::ids::{ActiveMonsterId, MessageNo, MonsterNo, RoomId, RoomKey, TextBlockNo};
use silvermere_types::records::{Exit, KnownMonster, Message, Room, TextBlock};

const ARENA: u32 = 2150;
const NORMAL: u32 = 2200;
const NEIGHBOUR: u32 = 2201;
const LAIR: u32 = 2300;
const LAIRS: u32 = 3000;
const ARENAS: u32 = 4000;

/// The group-6, genindex-1 monsters of the stock data (number, name).
const POOL: [(i32, &str); 6] = [
    (1, "giant rat"),
    (2, "lashworm"),
    (3, "filthbug"),
    (4, "carrion beast"),
    (5, "acid slime"),
    (7, "kobold thief"),
];

fn exit(target: u32, kind: ExitType) -> Exit {
    Exit {
        target: RoomId(target),
        kind,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    }
}

fn room(n: u32, kind: RoomType, maxmonsters: u8) -> Room {
    let mut r = blank::clear_room(RoomKey::new(1, n));
    r.name = format!("Room {n}");
    r.room_type = kind;
    r.mongroup = 6;
    r.minlevel = 1;
    r.maxlevel = 3;
    r.maxmonsters = maxmonsters;
    r
}

fn known(n: i32, name: &str, montype: MonsterType) -> KnownMonster {
    let mut k = blank::clear_known_monster(MonsterNo(n));
    k.name = name.into();
    k.montype = montype;
    k.genindex = 1;
    k.level = 1;
    k.maxhp = 12;
    k.followpct = 20;
    k
}

fn snapshot() -> WorldSnapshot {
    let mut s = tiny_snapshot();
    s.monsters.clear();
    for (n, name) in POOL {
        s.monsters.push(known(n, name, MonsterType::Group1));
    }
    // a monster of another group and level, never in the arena's pool
    let mut other = known(50, "orc", MonsterType::Group2);
    other.genindex = 5;
    s.monsters.push(other);
    let mut arena = room(ARENA, RoomType::Arena, 2);
    arena.exits[8] = exit(2146, ExitType::Normal);
    s.rooms.push(arena);
    s.rooms.push(room(2146, RoomType::Normal, 0));
    let mut normal = room(NORMAL, RoomType::Normal, 15);
    normal.exits[2] = exit(NEIGHBOUR, ExitType::Normal);
    s.rooms.push(normal);
    s.rooms.push(room(NEIGHBOUR, RoomType::Normal, 15));
    s.rooms.push(room(LAIR, RoomType::Lair, 4));
    for i in 0..20 {
        s.rooms.push(room(LAIRS + i, RoomType::Lair, 1));
        s.rooms.push(room(ARENAS + i, RoomType::Arena, 2));
    }
    s
}

fn world_of(s: WorldSnapshot) -> World {
    let mut w = World::load(test_config(), 32, s).expect("world loads");
    w.rng = SeededRng::new(35);
    w.set_wall(1_000_000_000);
    w
}

fn world() -> World {
    world_of(snapshot())
}

fn player(w: &mut World, name: &str, room: u32) -> SessionId {
    enter(w, name, room, MessageLevel::Medium)
}

fn key(n: u32) -> RoomKey {
    RoomKey::new(1, n)
}

/// The active monsters in a room, in slot order.
fn here(w: &World, n: u32) -> Vec<ActiveMonsterId> {
    w.room_state[&key(n)]
        .monsters
        .iter()
        .copied()
        .filter(|m| m.0 != 0)
        .collect()
}

fn known_of(w: &World, m: ActiveMonsterId) -> MonsterNo {
    w.active_monsters.by_number(m).unwrap().knownnum
}

/// Takes every monster out of `room` (no kill time).
fn clear(w: &mut World, n: u32) {
    for m in here(w, n) {
        remove_active_monster(w, m, false);
    }
}

/// The session's output since the last call, ANSI stripped, as lines.
fn text(w: &mut World, id: SessionId) -> Vec<String> {
    let palette = palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let out = s.output.render(palette, b"<P>");
    s.output.clear();
    // a direction name carries its anti-script letter and a backspace over it
    let mut plain = String::new();
    for c in String::from_utf8_lossy(&strip_ansi(&out)).chars() {
        if c == '\u{8}' {
            plain.pop();
        } else {
            plain.push(c);
        }
    }
    plain
        .split("\r\n")
        .map(|l| l.replace("<P>", "").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// A specific known monster forced into a room (`generate_monster(..., force 1)`).
fn place(w: &mut World, n: i32, room: u32) -> ActiveMonsterId {
    let req = Request {
        areatype: -1,
        monnum: MonsterNo(n),
        minlevel: 0,
        maxlevel: 0,
        owner: None,
        ally: false,
        force: true,
    };
    let n = generate_monster(w, key(room), req).expect("placed");
    // generate_monster writes the monster to the file only: read it into
    // the cache, as a player looking at the room would, so that the
    // background passes see it (T-086)
    w.active_monsters.touch(n);
    n
}

/// A slow cycle with its whole monster pass.
fn slow_cycle(w: &mut World) {
    w.slow_cycle();
    silvermere_engine::cycles::monsters::run_pending_passes(w);
}

/// One whole medium pass over the monster cache, as the polls of a
/// player in the game would run it.
fn medium_update_monsters(w: &mut World) {
    silvermere_engine::cycles::monsters::medium_update_monsters(w);
    silvermere_engine::cycles::monsters::run_pending_passes(w);
}

/// Pearson's chi-square of `hits` out of `n` trials against probability `p` (1 df).
fn chi_square(hits: u64, n: u64, p: f64) -> f64 {
    let e1 = n as f64 * p;
    let e0 = n as f64 * (1.0 - p);
    let o1 = hits as f64;
    let o0 = (n - hits) as f64;
    (o1 - e1).powi(2) / e1 + (o0 - e0).powi(2) / e0
}

/// The chi-square a 1-df sample exceeds with probability 0.001.
const CHI1_0001: f64 = 10.828;

// --- the spawn pass ----------------------------------------------------------------

// one player in the arena; over 100 spawn ticks
// the arena's pool appears, never more monsters than players
#[test]
fn arena_spawns_its_pool_never_more_than_players() {
    let mut w = world();
    player(&mut w, "Aldric", ARENA);
    let mut seen = std::collections::BTreeSet::new();
    let mut spawned = 0;
    for _ in 0..100 {
        w.spawn_cycle();
        let mons = here(&w, ARENA);
        assert!(mons.len() <= 1, "{mons:?}");
        if let Some(m) = mons.first() {
            spawned += 1;
            seen.insert(known_of(&w, *m).0);
        }
        clear(&mut w, ARENA);
    }
    let pool: std::collections::BTreeSet<i32> = POOL.iter().map(|(n, _)| *n).collect();
    assert!(seen.is_subset(&pool), "{seen:?}");
    assert!(seen.len() >= 4, "{seen:?}");
    // 89 of 99 rolls; 100 trials
    assert!((80..=97).contains(&spawned), "{spawned}");
}

// the arena with one player spawns while empty on 89 rolls of 99
// (90 above a roll of 1 to 99, survey 2.5 "90%")
#[test]
fn arena_rate_over_ten_thousand_ticks() {
    let mut w = world();
    player(&mut w, "Aldric", ARENA);
    let n = 10_000;
    let mut hits = 0;
    for _ in 0..n {
        w.spawn_cycle();
        if !here(&w, ARENA).is_empty() {
            hits += 1;
            clear(&mut w, ARENA);
        }
    }
    let p = 89.0 / 99.0;
    let chi = chi_square(hits, n, p);
    assert!(chi < CHI1_0001, "{hits}/{n}: chi {chi}");
}

// spawns scale with the player count: each player in the room rolls with the counts of the
// first (the table entry is not updated after a spawn), so the empty arena gains about one
// monster per player per tick
#[test]
fn arena_spawns_scale_with_players() {
    for players in 1..=3u64 {
        let mut s = snapshot();
        s.rooms
            .iter_mut()
            .find(|r| r.key.room.0 == ARENA)
            .unwrap()
            .maxmonsters = 15;
        let mut w = world_of(s);
        for i in 0..players {
            player(&mut w, &format!("P{i}"), ARENA);
        }
        let n = 3000u64;
        let mut total = 0u64;
        for _ in 0..n {
            w.spawn_cycle();
            let k = here(&w, ARENA).len() as u64;
            assert!(k <= players, "{k} > {players}");
            total += k;
            clear(&mut w, ARENA);
        }
        let mean = total as f64 / n as f64;
        let expect = players as f64 * 89.0 / 99.0;
        assert!(
            (mean - expect).abs() < 0.05,
            "{players}: {mean} vs {expect}"
        );
    }
}

// a Normal room spawns on rolls 1-4 of 1-99 (`5 > roll`)
#[test]
fn normal_room_rate() {
    let mut s = snapshot();
    // no neighbour to spawn into
    s.rooms
        .iter_mut()
        .find(|r| r.key.room.0 == NORMAL)
        .unwrap()
        .exits = std::array::from_fn(|_| exit(0, ExitType::Normal));
    let mut w = world_of(s);
    player(&mut w, "Aldric", NORMAL);
    let n = 20_000;
    let mut hits = 0;
    for _ in 0..n {
        w.spawn_cycle();
        if !here(&w, NORMAL).is_empty() {
            hits += 1;
            clear(&mut w, NORMAL);
        }
    }
    let chi = chi_square(hits, n, 4.0 / 99.0);
    assert!(chi < CHI1_0001, "{hits}/{n}: chi {chi}");
}

// a Lair fills while generate_monster succeeds
#[test]
fn lair_fills_to_maxmonsters() {
    let mut w = world();
    player(&mut w, "Aldric", LAIR);
    w.spawn_cycle();
    assert_eq!(here(&w, LAIR).len(), 4);
    assert_eq!(w.room_state[&key(LAIR)].nummonsters, 4);
    w.spawn_cycle();
    assert_eq!(here(&w, LAIR).len(), 4);
}

// at the ninth monster (`made > MAXGENERATE`) the pass remembers
// the player and returns; the next pass resumes there
#[test]
fn nine_per_pass_and_the_resume_index() {
    let mut w = world();
    let ids: Vec<SessionId> = (0..20)
        .map(|i| player(&mut w, &format!("P{i}"), ARENAS + i))
        .collect();
    w.spawn_cycle();
    let filled: Vec<u32> = (0..20)
        .filter(|i| !here(&w, ARENAS + i).is_empty())
        .collect();
    assert_eq!(filled.len(), 9, "{filled:?}");
    let resume = w.spawn.gennextusr;
    let last = *filled.last().unwrap();
    assert_eq!(
        resume, ids[last as usize].0,
        "resumes at the ninth's player"
    );
    w.spawn_cycle();
    // the next pass starts at that player: the rooms before it get nothing new
    let before: usize = (0..last).map(|i| here(&w, ARENAS + i).len()).sum();
    assert_eq!(before, filled.iter().filter(|i| **i < last).count());
    let after: usize = (last + 1..20).map(|i| here(&w, ARENAS + i).len()).sum();
    assert!(after > 0, "the second pass went on past the resume point");
}

// (sic) a Lair's fill calls generate_monster before it tests the
// count and never returns, so 20 one-monster lairs all fill in one pass
#[test]
fn lair_fills_go_past_the_cap() {
    let mut w = world();
    for i in 0..20 {
        player(&mut w, &format!("P{i}"), LAIRS + i);
    }
    w.spawn_cycle();
    let filled = (0..20).filter(|i| !here(&w, LAIRS + i).is_empty()).count();
    assert_eq!(filled, 20);
    assert_eq!(
        w.spawn.gennextusr, 0,
        "a complete run resets the resume index"
    );
}

// each exit of an occupied room spawns in the room beyond on rolls
// 1-5 of 1-99, while it has fewer monsters than the occupied room has players
#[test]
fn neighbour_spawns_per_exit() {
    let mut s = snapshot();
    // the occupied room itself never spawns (an arena with no room for monsters)
    let r = s.rooms.iter_mut().find(|r| r.key.room.0 == NORMAL).unwrap();
    r.room_type = RoomType::Arena;
    r.maxmonsters = 0;
    let mut w = world_of(s);
    player(&mut w, "Aldric", NORMAL);
    let n = 20_000;
    let mut hits = 0;
    for _ in 0..n {
        w.spawn_cycle();
        assert!(here(&w, NORMAL).is_empty());
        let k = here(&w, NEIGHBOUR).len();
        assert!(k <= 1);
        if k == 1 {
            hits += 1;
            clear(&mut w, NEIGHBOUR);
        }
    }
    let chi = chi_square(hits, n, 5.0 / 99.0);
    assert!(chi < CHI1_0001, "{hits}/{n}: chi {chi}");
}

// a genmonster room spawns only that monster
#[test]
fn genmonster_room_spawns_only_that_monster() {
    let mut s = snapshot();
    s.rooms
        .iter_mut()
        .find(|r| r.key.room.0 == ARENA)
        .unwrap()
        .genmonster = MonsterNo(5);
    let mut w = world_of(s);
    player(&mut w, "Aldric", ARENA);
    let mut n = 0;
    for _ in 0..200 {
        w.spawn_cycle();
        for m in here(&w, ARENA) {
            assert_eq!(known_of(&w, m), MonsterNo(5));
            n += 1;
        }
        clear(&mut w, ARENA);
    }
    assert!(n > 100);
}

// after a kill no monster for `regenmins` (5) minutes, except arenas
#[test]
fn regen_delay_blocks_five_minutes_after_a_kill() {
    let mut w = world();
    let t0 = w.wall;
    let stamp = w.minute_stamp();
    for n in [NORMAL, ARENA] {
        w.room_state.get_mut(&key(n)).unwrap().lastkill = stamp;
    }
    let normal = Request::for_room(&w, key(NORMAL)).unwrap();
    let arena = Request::for_room(&w, key(ARENA)).unwrap();
    assert!(generate_monster(&mut w, key(NORMAL), normal).is_none());
    assert!(generate_monster(&mut w, key(ARENA), arena).is_some());
    w.set_wall(t0 + 5 * 60);
    assert!(generate_monster(&mut w, key(NORMAL), normal).is_none());
    w.set_wall(t0 + 6 * 60);
    assert!(generate_monster(&mut w, key(NORMAL), normal).is_some());
    // the room's own regendelay wins over regenmins; force ignores it
    w.room_state.get_mut(&key(NORMAL)).unwrap().lastkill = w.minute_stamp();
    w.rooms.get_mut(&key(NORMAL)).unwrap().regendelay = 20;
    w.set_wall(t0 + 6 * 60 + 15 * 60);
    assert!(generate_monster(&mut w, key(NORMAL), normal).is_none());
    let forced = Request {
        force: true,
        ..normal
    };
    assert!(generate_monster(&mut w, key(NORMAL), forced).is_some());
}

// a unique monster comes back after regenhrs hours +- an eighth
#[test]
fn unique_monster_respects_regenhrs() {
    let mut w = world();
    let now = w.wall;
    let k = w.monsters.get_mut(&MonsterNo(50)).unwrap();
    k.maxalive = 1;
    k.regenhrs = 4;
    k.killed_at = now - 3 * 3600;
    let req = Request {
        areatype: -1,
        monnum: MonsterNo(50),
        minlevel: 0,
        maxlevel: 0,
        owner: None,
        ally: false,
        force: false,
    };
    for _ in 0..50 {
        assert!(generate_monster(&mut w, key(NORMAL), req).is_none());
    }
    w.monsters.get_mut(&MonsterNo(50)).unwrap().killed_at = now - 5 * 3600;
    let m = generate_monster(&mut w, key(NORMAL), req).expect("ready after 5 h");
    assert_eq!(w.monsters[&MonsterNo(50)].curalive, 1);
    // one alive at most
    assert!(generate_monster(&mut w, key(NORMAL), req).is_none());
    // removed with the kill time: it waits again
    remove_active_monster(&mut w, m, true);
    assert_eq!(w.monsters[&MonsterNo(50)].curalive, 0);
    assert_eq!(w.monsters[&MonsterNo(50)].killed_at, now);
    assert!(generate_monster(&mut w, key(NORMAL), req).is_none());
}

// the arrival lines, built-in and WCCMSG, with the direction
#[test]
fn arrival_texts_and_directions() {
    let mut s = snapshot();
    s.messages.push(Message {
        number: MessageNo(26),
        msgs: [
            "A %s creeps into the room from %s.".into(),
            "The %s creeps out of the room to %s.".into(),
            "The giant rat creeps in after you!".into(),
        ],
    });
    s.text_blocks.push(TextBlock {
        part: 0,
        number: TextBlockNo(2001),
        link: TextBlockNo(0),
        text: "N:".into(),
    });
    let k = s.monsters.iter_mut().find(|m| m.number.0 == 1).unwrap();
    k.arrivemsg = MessageNo(26);
    k.namelist = 2001;
    // a known monster whose arrival message is missing
    s.monsters
        .iter_mut()
        .find(|m| m.number.0 == 2)
        .unwrap()
        .arrivemsg = MessageNo(99);
    // the arena's one compass exit is east
    let arena = s.rooms.iter_mut().find(|r| r.key.room.0 == ARENA).unwrap();
    arena.exits[2] = exit(2146, ExitType::Normal);
    arena.maxmonsters = 15;
    let mut w = world_of(s);
    let id = player(&mut w, "Aldric", ARENA);
    let watcher = player(&mut w, "Bob", NORMAL);
    place(&mut w, 1, ARENA);
    assert_eq!(
        text(&mut w, id),
        ["A giant rat creeps into the room from the east."]
    );
    place(&mut w, 2, ARENA);
    assert_eq!(
        text(&mut w, id),
        ["lashworm moves into the room from the east."]
    );
    place(&mut w, 3, ARENA);
    assert_eq!(text(&mut w, id), ["filthbug just arrived from the east."]);
    // no compass exit of the Normal kind: from nowhere
    w.rooms.get_mut(&key(NORMAL)).unwrap().exits[2].kind = ExitType::Door;
    place(&mut w, 3, NORMAL);
    assert_eq!(
        text(&mut w, watcher),
        ["filthbug just arrived from nowhere."]
    );
    place(&mut w, 1, NORMAL);
    assert_eq!(
        text(&mut w, watcher),
        ["A giant rat creeps into the room from nowhere."]
    );
    place(&mut w, 2, NORMAL);
    assert_eq!(
        text(&mut w, watcher),
        ["lashworm moves into the room from nowhere."]
    );
}

// --- wandering and following -------------------------------------------------------

/// A room of monster group `group` with exits `exits`.
fn add_room(w: &mut World, n: u32, group: i16, flags: RoomFlags, exits: &[(usize, u32, ExitType)]) {
    let mut r = room(n, RoomType::Normal, 15);
    r.mongroup = group;
    r.flags = flags;
    for (d, t, k) in exits {
        r.exits[*d] = exit(*t, *k);
    }
    w.room_state
        .insert(r.key, silvermere_engine::RoomState::of(&r));
    w.rooms.insert(r.key, r);
}

fn add_known(w: &mut World, n: i32, montype: MonsterType, followpct: i16) {
    let mut k = known(n, &format!("m{n}"), montype);
    k.followpct = followpct;
    w.monsters.insert(k.number, k);
}

const A: u32 = 5000;
const B: u32 = 5001;
const C: u32 = 5002;

// (sic, the precedence) an Angel goes only into rooms without flags
// (so never into a protected one); a monster never into a room of another group
#[test]
fn wanderers_keep_out_of_protected_and_foreign_rooms() {
    let mut w = world();
    // A (group Wanderer) -> north B (protected, group 6), south C (no flags, group 6)
    let g = MonsterType::Wanderer.code() as i16;
    add_room(
        &mut w,
        A,
        g,
        RoomFlags::empty(),
        &[(0, B, ExitType::Normal), (1, C, ExitType::Normal)],
    );
    add_room(
        &mut w,
        B,
        6,
        RoomFlags::PROTECTED,
        &[(1, A, ExitType::Normal)],
    );
    add_room(
        &mut w,
        C,
        6,
        RoomFlags::empty(),
        &[(0, A, ExitType::Normal)],
    );
    add_known(&mut w, 60, MonsterType::Angel, 0);
    add_known(&mut w, 61, MonsterType::Wanderer, 0);
    let angel = place(&mut w, 60, A);
    let wanderer = place(&mut w, 61, A);
    assert_eq!(
        move_monster(&mut w, angel, Direction::North, None, false),
        Moved::No
    );
    assert_eq!(
        move_monster(&mut w, wanderer, Direction::North, None, false),
        Moved::No
    );
    assert_eq!(
        move_monster(&mut w, wanderer, Direction::South, None, false),
        Moved::No
    );
    assert_eq!(
        move_monster(&mut w, angel, Direction::South, None, false),
        Moved::Yes
    );
    // and over many medium passes neither ever reaches B
    for _ in 0..300 {
        medium_update_monsters(&mut w);
        assert!(here(&w, B).is_empty());
        slow_cycle(&mut w);
    }
}

// a door, locked at start-up, stops a
// wanderer; a guard and a quest monster go through
#[test]
fn closed_doors_stop_wanderers() {
    let mut w = world();
    add_room(&mut w, A, 6, RoomFlags::PATROL, &[(0, B, ExitType::Door)]);
    add_room(&mut w, B, 6, RoomFlags::PATROL, &[(1, A, ExitType::Door)]);
    let k = RoomKey::new(1, A);
    silvermere_engine::movement::exits::set_par1(&mut w, k, Direction::North, 2);
    add_known(&mut w, 62, MonsterType::Group1, 0);
    add_known(&mut w, 63, MonsterType::Guard, 0);
    add_known(&mut w, 64, MonsterType::Quest, 0);
    let m = place(&mut w, 62, A);
    for _ in 0..300 {
        medium_update_monsters(&mut w);
        slow_cycle(&mut w);
    }
    assert!(here(&w, B).is_empty());
    assert_eq!(
        move_monster(&mut w, m, Direction::North, None, false),
        Moved::No
    );
    let g = place(&mut w, 63, A);
    assert_eq!(
        move_monster(&mut w, g, Direction::North, None, false),
        Moved::Yes
    );
    let q = place(&mut w, 64, A);
    assert_eq!(
        move_monster(&mut w, q, Direction::North, None, false),
        Moved::Yes
    );
}

// a Guard leaving for an RF_PATROL room of
// another group does not take a Block Guard exit; other monsters take it as a plain one
#[test]
fn block_guard_stops_only_a_guard_leaving_its_group() {
    let mut w = world();
    let guard = MonsterType::Guard.code() as i16;
    add_room(
        &mut w,
        A,
        guard,
        RoomFlags::empty(),
        &[(0, B, ExitType::BlockGuard), (1, C, ExitType::Normal)],
    );
    add_room(&mut w, B, 6, RoomFlags::PATROL, &[]);
    add_room(&mut w, C, 6, RoomFlags::PATROL, &[]);
    add_known(&mut w, 63, MonsterType::Guard, 0);
    add_known(&mut w, 62, MonsterType::Group1, 0);
    let g = place(&mut w, 63, A);
    assert_eq!(
        move_monster(&mut w, g, Direction::North, None, false),
        Moved::No
    );
    assert_eq!(
        move_monster(&mut w, g, Direction::South, None, false),
        Moved::Yes
    );
    let m = place(&mut w, 62, A);
    assert_eq!(
        move_monster(&mut w, m, Direction::North, None, false),
        Moved::Yes
    );
    // into a room of its own group the Block Guard exit does not apply
    w.rooms.get_mut(&key(B)).unwrap().mongroup = guard;
    let g2 = place(&mut w, 63, A);
    assert_eq!(
        move_monster(&mut w, g2, Direction::North, None, false),
        Moved::Yes
    );
}

// Lair and NPC monsters never wander
#[test]
fn npcs_and_lair_monsters_never_wander() {
    let mut w = world();
    let lair = MonsterType::Lair.code() as i16;
    add_room(
        &mut w,
        A,
        lair,
        RoomFlags::empty(),
        &[(0, B, ExitType::Normal)],
    );
    add_room(
        &mut w,
        B,
        lair,
        RoomFlags::empty(),
        &[(1, A, ExitType::Normal)],
    );
    add_known(&mut w, 65, MonsterType::Lair, 0);
    add_known(&mut w, 66, MonsterType::Npc, 0);
    w.rooms.get_mut(&key(B)).unwrap().mongroup = MonsterType::Npc.code() as i16;
    place(&mut w, 65, A);
    place(&mut w, 66, A);
    for _ in 0..300 {
        medium_update_monsters(&mut w);
        slow_cycle(&mut w);
    }
    assert_eq!(here(&w, A).len(), 2);
}

// a wanderer moves with (100-followpct)/2 percent, at most
// MAXMOVERS (3) a pass, never back the way it came within a slow period
#[test]
fn wanderers_move_at_most_three_a_pass() {
    let mut w = world();
    add_room(
        &mut w,
        A,
        6,
        RoomFlags::empty(),
        &[(0, B, ExitType::Normal)],
    );
    add_room(
        &mut w,
        B,
        6,
        RoomFlags::empty(),
        &[(1, A, ExitType::Normal)],
    );
    add_known(&mut w, 67, MonsterType::Group1, 0);
    for _ in 0..10 {
        place(&mut w, 67, A);
    }
    let mut most = 0;
    for _ in 0..50 {
        let before = here(&w, A).len();
        medium_update_monsters(&mut w);
        let moved = before.abs_diff(here(&w, A).len());
        most = most.max(moved);
        assert!(moved <= 3);
        slow_cycle(&mut w);
    }
    assert_eq!(most, 3);
}

// a monster following the player comes along with followpct percent
#[test]
fn following_with_followpct_0_50_100() {
    for (pct, lo, hi) in [(0, 0, 0), (50, 400, 600), (100, 1000, 1000)] {
        let mut w = world();
        add_room(
            &mut w,
            A,
            6,
            RoomFlags::empty(),
            &[(0, B, ExitType::Normal)],
        );
        add_room(
            &mut w,
            B,
            6,
            RoomFlags::empty(),
            &[(1, A, ExitType::Normal)],
        );
        add_known(&mut w, 68, MonsterType::Group1, pct);
        let id = player(&mut w, "Aldric", A);
        let mut came = 0;
        for _ in 0..1000 {
            let m = place(&mut w, 68, A);
            w.active_monsters.by_number_mut(m).unwrap().following = "Aldric".into();
            handle_following_monsters(&mut w, id, key(A), Direction::North);
            if here(&w, B).contains(&m) {
                came += 1;
            }
            remove_active_monster(&mut w, m, false);
        }
        assert!((lo..=hi).contains(&came), "{pct}: {came}");
    }
}

// the fast pass walks a follower after its player along the
// player's trail; out of sight it loses rounds and gives up past 15
#[test]
fn fast_pass_follows_and_gives_up() {
    let mut w = world();
    add_room(
        &mut w,
        A,
        6,
        RoomFlags::empty(),
        &[(0, B, ExitType::Normal)],
    );
    add_room(
        &mut w,
        B,
        6,
        RoomFlags::empty(),
        &[(1, A, ExitType::Normal)],
    );
    add_known(&mut w, 69, MonsterType::Group1, 100);
    let id = player(&mut w, "Aldric", B);
    {
        let p = w.session_mut(id).unwrap().character.as_mut().unwrap();
        p.trailrooms[0] = RoomId(B);
        p.trailrooms[1] = RoomId(A);
        p.trailmaps[0] = silvermere_types::ids::MapId(1);
        p.trailmaps[1] = silvermere_types::ids::MapId(1);
    }
    let m = place(&mut w, 69, A);
    w.active_monsters.by_number_mut(m).unwrap().following = "Aldric".into();
    fast_update_monster(&mut w, m);
    assert_eq!(here(&w, B), [m]);
    // the player is gone: 16 lost rounds and it forgets
    w.active_monsters.by_number_mut(m).unwrap().following = "Nobody".into();
    for _ in 0..16 {
        fast_update_monster(&mut w, m);
    }
    assert!(w.active_monsters.by_number(m).unwrap().following.is_empty());
}

// a knocked-down monster gets up when its rounds run out
#[test]
fn knocked_down_monster_rises() {
    let mut w = world();
    let id = player(&mut w, "Aldric", ARENA);
    let m = place(&mut w, 1, ARENA);
    text(&mut w, id);
    let mon = w.active_monsters.by_number_mut(m).unwrap();
    mon.flags |= ActiveMonsterFlags::DOWN;
    mon.downrounds = 2;
    fast_update_monster(&mut w, m);
    assert!(text(&mut w, id).is_empty());
    fast_update_monster(&mut w, m);
    assert_eq!(
        text(&mut w, id),
        ["Slightly dazed the giant rat rises from the floor."]
    );
}

// a leader that wanders takes its group along
#[test]
fn a_leader_takes_its_group() {
    let mut w = world();
    add_room(
        &mut w,
        A,
        6,
        RoomFlags::empty(),
        &[(0, B, ExitType::Normal)],
    );
    add_room(
        &mut w,
        B,
        6,
        RoomFlags::empty(),
        &[(1, A, ExitType::Normal)],
    );
    add_known(&mut w, 70, MonsterType::Group1, 0);
    add_known(&mut w, 71, MonsterType::Group1, 0);
    for (n, role, maxfollow) in [(70, GroupRole::Leader, 2), (71, GroupRole::Follower, 2)] {
        let k = w.monsters.get_mut(&MonsterNo(n)).unwrap();
        k.grouprole = role;
        k.group = 9;
        k.maxfollow = maxfollow;
    }
    let leader = place(&mut w, 70, A);
    let f1 = place(&mut w, 71, A);
    let f2 = place(&mut w, 71, A);
    let f3 = place(&mut w, 71, A);
    // a follower on its own waits for its leader
    assert_eq!(
        move_monster(&mut w, f1, Direction::North, None, false),
        Moved::No
    );
    assert_eq!(
        move_monster(&mut w, leader, Direction::North, None, false),
        Moved::Yes
    );
    let b = here(&w, B);
    assert!(b.contains(&leader) && b.contains(&f1) && b.contains(&f2));
    assert_eq!(here(&w, A), [f3]);
}

// --- numbers and persistence -------------------------------------------------------

// freed numbers are reused oldest first
#[test]
fn numbers_are_reused_oldest_first() {
    let mut w = world();
    let a = place(&mut w, 1, NORMAL);
    let b = place(&mut w, 1, NORMAL);
    let c = place(&mut w, 1, NORMAL);
    assert_eq!([a.0, b.0, c.0], [1, 2, 3]);
    remove_active_monster(&mut w, b, false);
    remove_active_monster(&mut w, a, false);
    assert_eq!(place(&mut w, 1, NORMAL), b);
    assert_eq!(place(&mut w, 1, NORMAL), a);
    assert_eq!(place(&mut w, 1, NORMAL).0, 4);
}

fn persisted(
    rx: &std::sync::mpsc::Receiver<EngineOutput>,
    rows: &mut BTreeMap<i32, silvermere_types::records::ActiveMonster>,
) {
    for o in drain(rx) {
        match o {
            EngineOutput::Persist(PersistRequest::ActiveMonster(m)) => {
                rows.insert(m.number.0, *m);
            }
            EngineOutput::Persist(PersistRequest::ActiveMonsterDelete(n)) => {
                rows.remove(&n.0);
            }
            _ => {}
        }
    }
}

// The write-behind: upserts on generate and on the 60 s sweep, deletes on removal; a
// world loaded from what was written has the same monsters, numbers and free list
#[test]
fn active_monsters_persist_and_load_back() {
    let s = snapshot();
    let (tx, rx) = std::sync::mpsc::channel();
    let clock = TestClock::new();
    clock.advance(Duration::from_secs(1_000_000_000));
    let mut world = World::load(test_config(), 32, s.clone()).unwrap();
    let id = player(&mut world, "Aldric", LAIR);
    let mut engine = Engine::new(world, clock.clone(), SeededRng::new(9), tx);
    let mut rows = BTreeMap::new();
    // the first spawn pass is due 5 ticks in
    clock.advance_ticks(6, &mut engine);
    persisted(&rx, &mut rows);
    assert_eq!(rows.len(), 4, "the lair's four monsters were written");
    // the player leaves for a room that is not there (no more spawns); one monster
    // leaves the game, one is changed: a delete, and the 60 s sweep writes the change
    let w = &mut engine.world;
    w.session_mut(id).unwrap().character.as_mut().unwrap().room = RoomId(9999);
    let gone = here(w, LAIR)[1];
    remove_active_monster(w, gone, false);
    let changed = here(w, LAIR)[0];
    w.active_monsters.by_number_mut(changed).unwrap().hp = 3;
    clock.advance_ticks(61, &mut engine);
    persisted(&rx, &mut rows);
    let w = &engine.world;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[&changed.0].hp, 3);
    let numbers: Vec<i32> = w
        .active_monsters
        .numbers_in_use()
        .iter()
        .map(|n| n.0)
        .collect();
    assert_eq!(rows.keys().copied().collect::<Vec<_>>(), numbers);
    // load them back with the room's list
    let mut s2 = s;
    s2.active_monsters = rows.values().cloned().collect();
    let lair = s2.rooms.iter_mut().find(|r| r.key.room.0 == LAIR).unwrap();
    lair.monsters = w.room_state[&key(LAIR)].monsters;
    lair.nummonsters = w.room_state[&key(LAIR)].nummonsters;
    let mut back = World::load(test_config(), 32, s2).unwrap();
    for m in rows.values() {
        assert_eq!(back.active_monsters.by_number(m.number), Some(m));
    }
    assert_eq!(here(&back, LAIR).len(), 3);
    // the freed number is handed out first
    assert_eq!(place(&mut back, 1, NORMAL), gone);
}

// --- the start-up reset ------------------------------------------------------------

// doors and gates relock (exitpar1s 2), key exits lock
// (exitpar2s 2), each Timed exit's first swap is armed after exitpar2s steps
#[test]
fn start_up_relocks_doors_and_arms_timed_exits() {
    let mut s = snapshot();
    let r = s.rooms.iter_mut().find(|r| r.key.room.0 == NORMAL).unwrap();
    r.exits[0] = exit(NEIGHBOUR, ExitType::Door);
    r.exits[1] = exit(NEIGHBOUR, ExitType::Gate);
    r.exits[3] = exit(NEIGHBOUR, ExitType::Key);
    r.exits[4] = exit(NEIGHBOUR, ExitType::Timed);
    r.exits[4].par2 = 3;
    r.lastkill = 77;
    common::dense_map1(&mut s, NORMAL);
    let w = world_of(s);
    let k = key(NORMAL);
    assert_eq!(live_exit(&w, k, Direction::North).unwrap().par1, 2);
    assert_eq!(live_exit(&w, k, Direction::South).unwrap().par1, 2);
    assert_eq!(live_exit(&w, k, Direction::West).unwrap().par2, 2);
    // the content is untouched
    assert_eq!(w.rooms[&k].exits[0].par1, 0);
    assert_eq!(
        w.room_state[&k].lastkill, 0,
        "no recent kill after a cold start"
    );
    let armed: Vec<_> = w
        .scheduler
        .pending()
        .filter(|kick| {
            kick.routine
                == Routine::ExitRevert {
                    room: k,
                    dir: Direction::NorthEast.code() as u8,
                }
        })
        .map(|kick| kick.when)
        .collect();
    assert_eq!(armed, [300 * 3]);
}

// The engine's one Rng: commands and cycles draw from world.rng, which Engine::new
// seeds from the engine's Rng. The cycles' draws come out of world.rng (it advances),
// and changing world.rng's state alone changes what the spawn pass picks.
#[test]
fn one_rng_replays() {
    // the known monsters the arena spawns over 100 passes, emptied after each, and
    // world.rng's state at the end
    let run = |seed: u64, reseed: Option<u64>| {
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let mut world = World::load(test_config(), 32, snapshot()).unwrap();
        player(&mut world, "Aldric", ARENA);
        let mut engine = Engine::new(world, clock.clone(), SeededRng::new(seed), tx);
        if let Some(r) = reseed {
            engine.world.rng = SeededRng::new(r);
        }
        let start = engine.world.rng.clone();
        let mut kinds = Vec::new();
        for _ in 0..100 {
            clock.advance_ticks(5, &mut engine);
            for m in here(&engine.world, ARENA) {
                kinds.push(known_of(&engine.world, m).0);
            }
            clear(&mut engine.world, ARENA);
        }
        drain(&rx);
        assert_ne!(engine.world.rng, start, "the cycles drew from world.rng");
        (kinds, engine.world.rng.clone())
    };
    assert_eq!(run(4, None), run(4, None));
    // the same engine seed with only world.rng changed: a different game
    assert_ne!(run(4, Some(1)).0, run(4, Some(2)).0);
    // the engine's seed reaches world.rng
    assert_ne!(run(4, None).0, run(5, None).0);
}

// T-033's check_kill_monster: a monster
// below 0 hit points dies in the medium pass; its generating room counts it down once and
// restarts the regeneration clock, and its number is reused first
#[test]
fn a_medium_pass_death_frees_the_number() {
    let mut w = world();
    let m = place(&mut w, 1, NORMAL);
    let other = place(&mut w, 1, NORMAL);
    assert_eq!(w.room_state[&key(NORMAL)].nummonsters, 2);
    w.active_monsters.by_number_mut(m).unwrap().hp = -1;
    medium_update_monsters(&mut w);
    assert!(w.active_monsters.by_number(m).is_none());
    assert_eq!(here(&w, NORMAL), [other]);
    assert_eq!(w.room_state[&key(NORMAL)].nummonsters, 1);
    assert_eq!(w.room_state[&key(NORMAL)].lastkill, w.minute_stamp());
    assert_eq!(place(&mut w, 1, NORMAL), m);
}

// --- T-086: the passes walk the monster cache ------------------------------------------

use silvermere_engine::cycles::monsters as passes;
use silvermere_engine::monsters::cache::MonsterCache;

/// `generate_monster` without the cache read [`place`] adds.
fn place_uncached(w: &mut World, n: i32, room: u32) -> ActiveMonsterId {
    let req = Request {
        areatype: -1,
        monnum: MonsterNo(n),
        minlevel: 0,
        maxlevel: 0,
        owner: None,
        ally: false,
        force: true,
    };
    generate_monster(w, key(room), req).expect("placed")
}

/// A cache of `monsbuf + 1` slots (MONSBUF), empty.
fn small_cache(w: &mut World, monsbuf: usize) {
    *w.active_monsters.cache_mut() = MonsterCache::new(monsbuf);
}

thread_local! {
    static VISITS: std::cell::RefCell<Vec<ActiveMonsterId>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn record_visit(_: &mut World, n: ActiveMonsterId) {
    VISITS.with(|v| v.borrow_mut().push(n));
}

// a new monster is written to the file, not read into the cache (unless it has a creation
// spell); the first look-up by number reads it in
#[test]
fn a_generated_monster_is_not_cached_until_looked_up() {
    let mut w = world();
    let m = place_uncached(&mut w, 1, NORMAL);
    assert_eq!(w.active_monsters.cache().slot_of(m), None);
    let _ = w.active_monsters.by_number(m);
    assert!(w.active_monsters.cache().slot_of(m).is_some());
}

// the medium pass visits cache slots
// 0..monsbuf-1 in order. With MONSBUF 4, monsters 1-6 read in that order land in slots
// 1, 2, 3, 0 and 4 (5 probes past the full slots to the empty last one), and 6 finds no
// empty slot from its hashed slot 2 on, so it evicts monster 2 there. The pass visits 4,
// 1, 6, 3: not 2 (evicted), not 5 (slot 4 = monsbuf is never visited).
#[test]
fn the_medium_pass_walks_the_cache_slots() {
    let mut w = world();
    let ms: Vec<ActiveMonsterId> = (0..6).map(|_| place_uncached(&mut w, 1, NORMAL)).collect();
    assert_eq!(ms, (1..=6).map(ActiveMonsterId).collect::<Vec<_>>());
    small_cache(&mut w, 4);
    for &m in &ms {
        w.active_monsters.touch(m);
    }
    let slots: Vec<u32> = (0..=4)
        .map(|i| w.active_monsters.cache().number_at(i))
        .collect();
    assert_eq!(slots, [4, 1, 6, 3, 5]);
    w.cycles.hooks.medium_update_monster = record_visit;
    VISITS.with(|v| v.borrow_mut().clear());
    passes::medium_update_monsters(&mut w);
    passes::run_pending_passes(&mut w);
    let visits = VISITS.with(|v| v.borrow().clone());
    assert_eq!(visits, [4, 1, 6, 3].map(ActiveMonsterId));
    // the pass is over: the cursor is back at slot 0
    assert_eq!(
        w.cycles.monster_passes.medium,
        passes::PassCursor::default()
    );
}

// a changed monster pushed out of a full cache is written
// back, which clears its `dirty`
#[test]
fn a_full_cache_writes_the_evicted_monster_back() {
    let mut w = world();
    let ms: Vec<ActiveMonsterId> = (0..6).map(|_| place_uncached(&mut w, 1, NORMAL)).collect();
    small_cache(&mut w, 4);
    for &m in &ms[..5] {
        w.active_monsters.touch(m);
    }
    w.active_monsters.peek_mut(ms[1]).unwrap().hp -= 1;
    assert!(w.active_monsters.is_dirty(ms[1]));
    w.active_monsters.touch(ms[5]);
    assert!(w.active_monsters.is_dirty(ms[1]));
    w.poll_monster_passes();
    assert!(!w.active_monsters.is_dirty(ms[1]));
    assert_eq!(w.active_monsters.cache().slot_of(ms[1]), None);
}

// MAXMOVERS (3) places a medium pass, handed out in
// cache-slot order. Four guards, one exit each, read into a MONSBUF-4 cache: guard 4
// sits in slot 0 and goes first, so guards 4, 1 and 2 move and guard 3 stays (in number
// order it would be 1, 2 and 3).
#[test]
fn maxmovers_go_by_cache_slot() {
    let mut w = world();
    add_known(&mut w, 68, MonsterType::Guard, 0);
    let mut guards = Vec::new();
    for i in 0..4 {
        let (from, to) = (6000 + i, 6100 + i);
        add_room(
            &mut w,
            from,
            6,
            RoomFlags::PATROL,
            &[(0, to, ExitType::Normal)],
        );
        add_room(
            &mut w,
            to,
            6,
            RoomFlags::PATROL,
            &[(1, from, ExitType::Normal)],
        );
        let g = place_uncached(&mut w, 68, from);
        // no way back yet, and the change written so that the guard is not `dirty`
        w.active_monsters.peek_mut(g).unwrap().lastdir = -1;
        silvermere_engine::monsters::active::save_active_monster(&mut w, g);
        guards.push(g);
    }
    assert_eq!(guards, (1..=4).map(ActiveMonsterId).collect::<Vec<_>>());
    small_cache(&mut w, 4);
    for &g in &guards {
        w.active_monsters.touch(g);
    }
    passes::medium_update_monsters(&mut w);
    passes::run_pending_passes(&mut w);
    let moved: Vec<bool> = (0..4).map(|i| here(&w, 6100 + i).len() == 1).collect();
    assert_eq!(moved, [true, true, false, true]);
    assert_eq!(w.spawn.mediummoved, 3);
}

// a pass takes one step when it starts; the
// rest come from the polls of the players in the game. With nobody in, the slow pass
// reaches slot 1 on its second cycle; a new start while it is pending adds a step and
// does not start over; once a player is in, the polls finish it.
#[test]
fn with_nobody_in_the_game_a_pass_takes_one_slot_a_cycle() {
    let mut w = world();
    let m = place(&mut w, 1, NORMAL);
    assert_eq!(w.active_monsters.cache().slot_of(m), Some(1));
    passes::slow_update_monsters(&mut w);
    w.poll_monster_passes();
    assert_eq!(w.active_monsters.peek(m).unwrap().lastupd, 0);
    assert_eq!(w.cycles.monster_passes.slow.slot, 1);
    passes::slow_update_monsters(&mut w);
    w.poll_monster_passes();
    assert_eq!(w.active_monsters.peek(m).unwrap().lastupd, 2);
    assert_eq!(
        w.cycles.monster_passes.slow,
        passes::PassCursor { slot: 2, pend: 1 }
    );
    player(&mut w, "Kang", NEIGHBOUR);
    w.poll_monster_passes();
    assert_eq!(w.cycles.monster_passes.slow, passes::PassCursor::default());
}

// the fast pass updates only monsters that follow a player, so a
// knocked-down monster that follows nobody stays down
#[test]
fn the_fast_pass_skips_monsters_that_follow_nobody() {
    let mut w = world();
    let lone = place(&mut w, 1, NORMAL);
    let follower = place(&mut w, 1, NORMAL);
    for m in [lone, follower] {
        let mon = w.active_monsters.peek_mut(m).unwrap();
        mon.flags.insert(ActiveMonsterFlags::DOWN);
        mon.downrounds = 1;
    }
    w.active_monsters.peek_mut(follower).unwrap().following = "Kang".into();
    passes::fast_update_monsters(&mut w);
    passes::run_pending_passes(&mut w);
    let down = |w: &World, m| {
        w.active_monsters
            .peek(m)
            .unwrap()
            .flags
            .contains(ActiveMonsterFlags::DOWN)
    };
    assert!(down(&w, lone));
    assert!(!down(&w, follower));
}

// with no player of that name, a name that is part of a
// cached monster's name (contains_ignore_case: any case, anywhere) finds a monster; one outside the
// cache is not found
#[test]
fn any_action_target_finds_a_cached_monster_by_part_of_its_name() {
    use silvermere_engine::gangs::{Target, find_any_action_target};
    let mut w = world();
    let m = place_uncached(&mut w, 1, NORMAL);
    w.active_monsters.peek_mut(m).unwrap().name = "large orc".into();
    assert!(matches!(find_any_action_target(&w, "rc"), Target::None));
    w.active_monsters.touch(m);
    assert!(matches!(find_any_action_target(&w, "rc"), Target::Monster));
    assert!(matches!(
        find_any_action_target(&w, "LARGE"),
        Target::Monster
    ));
    assert!(matches!(find_any_action_target(&w, "elf"), Target::None));
}
