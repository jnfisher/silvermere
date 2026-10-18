//! T-076: the room pass of the start-up cycle (`preload_cycle`): each room's specific
//! monster, then the lair and shop fill, then on a cold start only the room reset, room by
//! room in `room_walk` order.

mod common;

use common::{test_config, tiny_snapshot};
use silvermere_engine::monsters::active::remove_active_monster;
use silvermere_engine::monsters::generate::{Request, generate_monster};
use silvermere_engine::monsters::{StartUp, preload_cycle, room_walk};
use silvermere_engine::world::blank;
use silvermere_engine::{SeededRng, World, WorldSnapshot};
use silvermere_types::enums::{MonsterType, RoomType};
use silvermere_types::flags::RoomFlags;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, MonsterNo, RoomId, RoomKey};
use silvermere_types::records::{ActiveMonster, Coins, KnownMonster, Room, RoomItem};

/// The room's specific monster (a boss).
const BOSS: MonsterNo = MonsterNo(9);
/// Rooms of map 1 1..=ROOMS exist (the tiny world's special rooms lie far past the walk's
/// end and are never visited).
const ROOMS: u32 = 10;
const SPEC: u32 = 3;
const LAIR: u32 = 5;
const SHOP: u32 = 6;
const NORMAL: u32 = 7;
const SMALL_LAIR: u32 = 8;

fn key(n: u32) -> RoomKey {
    RoomKey::new(1, n)
}

fn known(n: i32, name: &str) -> KnownMonster {
    let mut k = blank::clear_known_monster(MonsterNo(n));
    k.name = name.into();
    k.montype = MonsterType::Group1;
    k.genindex = 1;
    k.level = 1;
    k.maxhp = 12;
    // coins make every generation draw from the rng
    k.cash = Coins {
        copper: 50,
        silver: 20,
        ..Coins::default()
    };
    k
}

fn room(n: u32, kind: RoomType, maxmonsters: u8) -> Room {
    let mut r = blank::clear_room(key(n));
    r.name = format!("Room {n}");
    r.room_type = kind;
    r.mongroup = MonsterType::Group1.code() as i16;
    r.minlevel = 1;
    r.maxlevel = 3;
    r.maxmonsters = maxmonsters;
    r
}

/// Map 1 rooms 1..=10: a specific monster in 3, a Lair in 5 with room for 30, a Shop in 6
/// with room for 2, a Normal room in 7, a Lair in 8 with room for 3; the rest Normal with
/// no monsters allowed.
fn snapshot() -> WorldSnapshot {
    let mut s = tiny_snapshot();
    s.monsters.clear();
    for (n, name) in [(1, "giant rat"), (2, "lashworm"), (3, "filthbug")] {
        s.monsters.push(known(n, name));
    }
    let mut boss = known(BOSS.0, "kobold chieftain");
    boss.genindex = 0; // never in a pool
    s.monsters.push(boss);
    for n in 1..=ROOMS {
        let r = match n {
            SPEC => {
                let mut r = room(n, RoomType::Normal, 0);
                r.specmon = BOSS;
                r
            }
            LAIR => room(n, RoomType::Lair, 30),
            SHOP => room(n, RoomType::Shop, 2),
            NORMAL => room(n, RoomType::Normal, 5),
            SMALL_LAIR => room(n, RoomType::Lair, 3),
            _ => room(n, RoomType::Normal, 0),
        };
        s.rooms.push(r);
    }
    s
}

fn load(s: WorldSnapshot) -> World {
    World::load(test_config(), 8, s).expect("world loads")
}

fn here(w: &World, n: u32) -> Vec<ActiveMonsterId> {
    w.room_state[&key(n)]
        .monsters
        .iter()
        .copied()
        .filter(|m| m.0 != 0)
        .collect()
}

fn bosses(w: &World) -> Vec<&ActiveMonster> {
    w.active_monsters
        .values()
        .filter(|m| m.knownnum == BOSS)
        .collect()
}

/// An active copy of the boss generated in room `genroom`, lying in room `at`.
fn alive_boss(s: &mut WorldSnapshot, number: i32, genroom: u32, at: u32) {
    let mut m = blank::clear_active_monster(ActiveMonsterId(number));
    m.knownnum = BOSS;
    m.name = "kobold chieftain".into();
    m.map = MapId(1);
    m.room = RoomId(at);
    m.genroom = RoomId(genroom);
    m.hp = 12;
    m.maxhp = 12;
    m.target = -1;
    s.active_monsters.push(m);
    let r = s.rooms.iter_mut().find(|r| r.key == key(at)).unwrap();
    let slot = r.monsters.iter().position(|x| x.0 == 0).unwrap();
    r.monsters[slot] = ActiveMonsterId(number);
}

fn set_specalive(s: &mut WorldSnapshot, n: u32) {
    let r = s.rooms.iter_mut().find(|r| r.key == key(n)).unwrap();
    r.flags |= RoomFlags::SPECALIVE;
}

// --- the walk ------------------------------------------------------------------------

/// Map 1 room 1 on; (sic) a map's scan ends at its 50th
/// missing room number in all, not in a row, and the scan at the 100th empty map.
#[test]
fn the_walk_ends_a_map_at_its_fiftieth_missing_room() {
    let mut s = snapshot();
    // map 1: rooms 1..=10, then a room after 49 more holes (room 60: holes 11..=59) is
    // still visited; the next hole (61) is the 50th and ends the map
    s.rooms.push(room(60, RoomType::Normal, 0));
    s.rooms.push(room(62, RoomType::Normal, 0));
    // map 2: a hole at 1, room 2
    let mut r = blank::clear_room(RoomKey::new(2, 2));
    r.name = "Map 2".into();
    s.rooms.push(r);
    // map 103: past 100 empty maps (3..=102 are empty; with map 1's and map 2's no
    // empty map has been counted yet, so 3..=102 make 100)
    s.rooms.push(blank::clear_room(RoomKey::new(103, 1)));
    let w = load(s);
    let walk = room_walk(&w);
    let mut want: Vec<RoomKey> = (1..=ROOMS).map(key).collect();
    want.push(key(60));
    want.push(RoomKey::new(2, 2));
    assert_eq!(walk, want);
}

// --- specific monsters ---------------------------------------------------------------

/// A cold start generates the specific monster of a room whose monster
/// is not alive, even with no room for monsters (the room's own monster is exempt),
/// and it does not count as a generated monster.
#[test]
fn a_cold_start_generates_each_specific_monster() {
    let w = load(snapshot());
    let b = bosses(&w);
    assert_eq!(b.len(), 1);
    assert_eq!(
        (b[0].map, b[0].room, b[0].genroom),
        (MapId(1), RoomId(SPEC), RoomId(SPEC))
    );
    let st = &w.room_state[&key(SPEC)];
    assert!(st.specalive);
    assert_eq!(st.nummonsters, 0);
    assert_eq!(here(&w, SPEC), vec![b[0].number]);
}

/// The active monsters in the cache after the start-up, and those not in it, by the room
/// that generated them.
fn cached_by_genroom(w: &World) -> (Vec<u32>, Vec<u32>) {
    let resident: Vec<_> = w
        .active_monsters
        .cache()
        .resident()
        .map(|(_, n)| n)
        .collect();
    let (mut yes, mut no) = (Vec::new(), Vec::new());
    for m in w.active_monsters.values() {
        if resident.contains(&m.number) {
            yes.push(m.genroom.0);
        } else {
            no.push(m.genroom.0);
        }
    }
    yes.sort_unstable();
    yes.dedup();
    no.sort_unstable();
    no.dedup();
    (yes, no)
}

/// The snapshot with map 1 dense up to `upto` and a Lair with room for 2 at each of
/// `lairs`.
fn with_lairs(upto: u32, lairs: &[u32]) -> WorldSnapshot {
    let mut s = snapshot();
    common::dense_map1(&mut s, upto);
    for &n in lairs {
        let r = s.rooms.iter_mut().find(|r| r.key == key(n)).unwrap();
        *r = room(n, RoomType::Lair, 2);
    }
    s
}

/// WCCMMUD.MSG:369 (`initrate` 195): started
/// with no active monster, `maxactive` is looked up again at the start of each slice
/// while it is 0, and a slice ends when its count of room numbers tried reaches 195 (the
/// room number that ends it is counted again by the next). Rooms 1..=194 are the first
/// slice: their monsters are numbered up to the value the second slice finds and stay
/// in the cache; room 195's come after it and are not read in (a new monster is not
/// cached).
#[test]
fn a_cold_start_caches_the_monsters_of_the_first_slice_only() {
    let w = load(with_lairs(200, &[194, 195]));
    let (yes, no) = cached_by_genroom(&w);
    assert_eq!(yes, vec![SPEC, LAIR, SHOP, SMALL_LAIR, 194]);
    assert_eq!(no, vec![195]);
}

/// The same with no monster in the first slice: the second (room numbers 195..=388)
/// makes the first, the third slice's look-up fixes `maxactive`, and room 389's
/// monsters are left out.
#[test]
fn a_cold_start_fixes_maxactive_at_the_first_slice_that_finds_a_monster() {
    let mut s = with_lairs(400, &[195, 388, 389]);
    for n in [SPEC, LAIR, SHOP, NORMAL, SMALL_LAIR] {
        let r = s.rooms.iter_mut().find(|r| r.key == key(n)).unwrap();
        *r = room(n, RoomType::Normal, 0);
    }
    let w = load(s);
    let (yes, no) = cached_by_genroom(&w);
    assert_eq!(yes, vec![195, 388]);
    assert_eq!(no, vec![389]);
}

/// `game.buffers.monsbuf` (MONSBUF) sizes the active-monster cache the start-up fills:
/// 21 slots at 20, MBBSEmu's size, and the INIT_ACTIVE walk
/// leaves at most that many monsters in it.
#[test]
fn the_cache_has_the_configured_size() {
    let mut cfg = test_config();
    cfg.buffers.monsbuf = 20;
    let w = World::load(
        cfg,
        8,
        with_lairs(200, &[11, 12, 13, 14, 15, 16, 17, 18, 19, 20]),
    )
    .expect("world loads");
    let cache = w.active_monsters.cache();
    assert_eq!(cache.monsbuf(), 20);
    assert!(w.active_monsters.len() > 21);
    assert!(cache.resident().count() <= 21);
}

/// An alive specific monster is looked for among the active monsters
/// (that known monster, generated in this room of this map), wherever it now is; found,
/// no other copy is made.
#[test]
fn an_alive_specific_monster_that_wandered_off_is_kept() {
    let mut s = snapshot();
    set_specalive(&mut s, SPEC);
    alive_boss(&mut s, 4, SPEC, 2);
    let w = load(s);
    assert_eq!(bosses(&w).len(), 1);
    assert!(w.room_state[&key(SPEC)].specalive);
    assert!(here(&w, SPEC).is_empty());
}

/// RF_SPECALIVE with no copy generated in this room
/// (here one generated elsewhere) is cleared and a new one made.
#[test]
fn a_stale_alive_flag_is_cleared_and_the_monster_made_again() {
    let mut s = snapshot();
    set_specalive(&mut s, SPEC);
    alive_boss(&mut s, 4, 2, 2);
    let w = load(s);
    let b = bosses(&w);
    assert_eq!(b.len(), 2);
    assert_eq!(b.iter().filter(|m| m.genroom == RoomId(SPEC)).count(), 1);
    assert!(w.room_state[&key(SPEC)].specalive);
}

/// (sic) The search cuts the active monster number to 16 bits, so a copy
/// numbered above 32767 is not found and a second one is made.
#[test]
fn a_specific_monster_numbered_above_32767_is_not_found() {
    let mut s = snapshot();
    set_specalive(&mut s, SPEC);
    alive_boss(&mut s, 32_768, SPEC, SPEC);
    let w = load(s);
    assert_eq!(bosses(&w).len(), 2);
}

/// A room with no specific monster cannot have it alive.
#[test]
fn the_alive_flag_without_a_specific_monster_is_cleared() {
    let mut s = snapshot();
    set_specalive(&mut s, 2);
    let w = load(s);
    assert!(!w.room_state[&key(2)].specalive);
}

// --- lairs and shops -----------------------------------------------------------------

/// A Lair or a Shop is filled from its own group and level range until
/// generation fails or 16 are made; other rooms are not. A room lists at most NROOMMONS
/// (15) monsters, so the fill always stops at a full list first.
#[test]
fn lairs_and_shops_are_filled_until_the_room_is_full() {
    let w = load(snapshot());
    assert_eq!(here(&w, LAIR).len(), 15);
    assert_eq!(w.room_state[&key(LAIR)].nummonsters, 15);
    assert_eq!(here(&w, SHOP).len(), 2);
    assert_eq!(here(&w, SMALL_LAIR).len(), 3);
    assert!(here(&w, NORMAL).is_empty());
    for n in here(&w, LAIR) {
        let m = w.active_monsters.by_number(n).unwrap();
        assert!((1..=3).contains(&m.knownnum.0), "from the room's pool");
        assert_eq!(m.genroom, RoomId(LAIR));
    }
    // nothing anywhere else
    let total = w.active_monsters.values().count();
    assert_eq!(total, 15 + 2 + 3 + 1);
}

/// The lair fill takes the room's monster when it names one (`genmonster`).
#[test]
fn a_lair_with_a_named_monster_is_filled_with_it() {
    let mut s = snapshot();
    let r = s
        .rooms
        .iter_mut()
        .find(|r| r.key == key(SMALL_LAIR))
        .unwrap();
    r.genmonster = MonsterNo(2);
    let w = load(s);
    let got: Vec<MonsterNo> = here(&w, SMALL_LAIR)
        .into_iter()
        .map(|n| w.active_monsters.by_number(n).unwrap().knownnum)
        .collect();
    assert_eq!(got, vec![MonsterNo(2); 3]);
}

// --- reinit --------------------------------------------------------------------------

/// The room reset is for a cold start only: a SYSOP restart of the cycle makes the specific
/// monsters and fills the lairs and shops again, and leaves the room reset (recent kill,
/// found hidden items, placed items, exits) alone.
#[test]
fn a_reinit_fills_again_and_does_not_reset_the_rooms() {
    let mut s = snapshot();
    let r = s.rooms.iter_mut().find(|r| r.key == key(NORMAL)).unwrap();
    r.placed[0] = ItemNo(1);
    let mut w = load(s);
    // the cold start put the placed item down
    assert!(
        w.room_state[&key(NORMAL)]
            .items
            .iter()
            .chain(w.room_state[&key(NORMAL)].hiditems.iter())
            .any(|i| i.item == ItemNo(1))
    );
    // play happens: the boss and the lair are killed, a kill and a search in room 7, the
    // placed item is taken
    for n in here(&w, LAIR).into_iter().chain(here(&w, SPEC)) {
        remove_active_monster(&mut w, n, false);
    }
    w.room_state.get_mut(&key(SPEC)).unwrap().specalive = false;
    {
        let st = w.room_state.get_mut(&key(NORMAL)).unwrap();
        st.lastkill = 99;
        st.foundmask = 1;
        st.searchers = 1;
        for i in st.items.iter_mut().chain(st.hiditems.iter_mut()) {
            if i.item == ItemNo(1) {
                *i = RoomItem {
                    item: ItemNo(0),
                    uses: 0,
                    count: 0,
                };
            }
        }
    }
    preload_cycle(&mut w, StartUp::Reinit);
    assert_eq!(here(&w, LAIR).len(), 15);
    assert_eq!(bosses(&w).len(), 1);
    let st = &w.room_state[&key(NORMAL)];
    assert_eq!((st.lastkill, st.foundmask, st.searchers), (99, 1, 1));
    assert!(
        !st.items
            .iter()
            .chain(st.hiditems.iter())
            .any(|i| i.item == ItemNo(1)),
        "a reinit does not put placed items back"
    );
    // a cold start does all of it
    preload_cycle(&mut w, StartUp::Cold);
    let st = &w.room_state[&key(NORMAL)];
    assert_eq!((st.lastkill, st.foundmask, st.searchers), (0, 0, 0));
}

// --- rng order -----------------------------------------------------------------------

/// The rng draws come room by room in walk order, and within a room the specific monster
/// before the lair fill (map 1 before map 2): the cycle
/// draws exactly what these `generate_monster` calls in that order draw.
#[test]
fn the_draws_come_room_by_room_specific_monster_first() {
    // a room with both a specific monster and a lair fill, a lair on map 2, and the rooms
    // of the plain snapshot
    let base = || {
        let mut s = snapshot();
        let r = s.rooms.iter_mut().find(|r| r.key == key(SPEC)).unwrap();
        r.room_type = RoomType::Lair;
        r.maxmonsters = 2;
        let mut m2 = room(1, RoomType::Lair, 4);
        m2.key = RoomKey::new(2, 1);
        s.rooms.push(m2);
        s
    };
    // loaded with every room quiet, then the content set and the pass run on a seed
    let quiet = || {
        let mut s = base();
        for r in &mut s.rooms {
            r.specmon = MonsterNo(0);
            r.room_type = RoomType::Normal;
        }
        load(s)
    };
    let content = base();
    let restore = |w: &mut World| {
        for r in &content.rooms {
            w.rooms.insert(r.key, r.clone());
        }
        w.rng = SeededRng::new(76);
    };
    let mut got = quiet();
    restore(&mut got);
    preload_cycle(&mut got, StartUp::Reinit);

    let mut want = quiet();
    restore(&mut want);
    let pool = |w: &World, k: RoomKey| Request::for_room(w, k).unwrap();
    let spec = Request {
        areatype: 0,
        monnum: BOSS,
        minlevel: 0,
        maxlevel: 0x7FFF,
        owner: None,
        ally: false,
        force: false,
    };
    // 1:3 the boss, then its fill (room for 2), 1:5 fifteen (a full list), 1:6 two, 1:8
    // three, 2:1 four; each fill ends with the call that fails
    assert!(generate_monster(&mut want, key(SPEC), spec).is_some());
    let fills = [
        (key(SPEC), 2),
        (key(LAIR), 15),
        (key(SHOP), 2),
        (key(SMALL_LAIR), 3),
        (RoomKey::new(2, 1), 4),
    ];
    for (k, n) in fills {
        for _ in 0..n {
            let req = pool(&want, k);
            assert!(generate_monster(&mut want, k, req).is_some());
        }
        let req = pool(&want, k);
        assert!(generate_monster(&mut want, k, req).is_none());
    }
    let all = |w: &World| {
        let mut v: Vec<ActiveMonster> = w.active_monsters.values().cloned().collect();
        v.sort_by_key(|m| m.number);
        v
    };
    assert_eq!(all(&got), all(&want));
    assert_eq!(got.rng, want.rng);
}
