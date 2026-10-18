//! `dismiss_users_angels`, called by `cleanup_when_user_leaves`: a Villain or FIEND who
//! leaves loses every non-friendly type-37
//! (`MTYPE_ANGEL`) monster following him by first name. In the stock data type 37 is the
//! guardsmen, the bounty hunter, mercenaries and demon imps, and they go all the same.
//! Removing them prints nothing.

mod common;

use common::{enter, test_config, tiny_snapshot};
use silvermere_engine::monsters::cache::MonsterCache;
use silvermere_engine::session::{cleanup_when_user_leaves, on_logoff};
use silvermere_engine::world::{World, blank};
use silvermere_engine::{AccountRef, SessionId};
use silvermere_rules::SeededRng;
use silvermere_types::enums::{Alignment, GroupRole, MessageLevel, MonsterType};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings};
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, ItemNo, MapId, MonsterNo, RoomId, RoomKey,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins};

const ARENA: u32 = 2150;
const HUNTER: MonsterNo = MonsterNo(10);
const RAT: MonsterNo = MonsterNo(11);

fn world() -> World {
    let mut s = tiny_snapshot();
    let mut arena = blank::clear_room(RoomKey::new(1, ARENA));
    arena.name = "Newhaven Arena".into();
    s.rooms.push(arena);
    for (n, name, ty) in [
        (HUNTER, "bounty hunter", MonsterType::Angel),
        (RAT, "giant rat", MonsterType::Lair),
    ] {
        let mut k = blank::clear_known_monster(n);
        k.name = name.into();
        k.level = 1;
        k.maxhp = 20;
        k.energy = 1000;
        k.alignment = Alignment::Evil;
        k.montype = ty;
        k.curalive = 8;
        s.monsters.push(k);
    }
    let mut w = World::load(test_config(), 8, s).expect("world loads");
    w.rng = SeededRng::new(7);
    w
}

fn player_in(w: &mut World, name: &str, evil: i16) -> SessionId {
    let id = enter(w, name, ARENA, MessageLevel::High);
    let s = w.session_mut(id).unwrap();
    s.account = Some(AccountRef {
        id: AccountId(i64::from(id.0) + 1),
        username: name.to_ascii_uppercase(),
        sysop: false,
        gameop: false,
        sex: b'M',
    });
    s.created = true;
    let c = s.character.as_mut().unwrap();
    c.settings = PlayerSettings::VALIDATED;
    c.level = 5;
    (c.str, c.intel, c.wil, c.hea, c.agi, c.cha) = (55, 30, 45, 50, 30, 30);
    (c.hp, c.maxhp) = (48, 48);
    c.evil = evil;
    id
}

/// A monster of `known`, number `n`, in the arena, following `following`.
fn spawn(
    w: &mut World,
    n: i32,
    known: MonsterNo,
    following: &str,
    friendly: u8,
) -> ActiveMonsterId {
    let k = w.monsters.get(&known).unwrap().clone();
    let number = ActiveMonsterId(n);
    w.active_monsters.insert(ActiveMonster {
        number,
        knownnum: k.number,
        level: k.level,
        map: MapId(1),
        room: RoomId(ARENA),
        poison: 0,
        energy: k.energy,
        hp: k.maxhp,
        following: following.into(),
        trail: std::array::from_fn(|_| RoomId::default()),
        hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
        hunting: ActiveMonsterId::default(),
        name: k.name.clone(),
        wornitem: ItemNo::default(),
        weapon: k.weapon,
        items: std::array::from_fn(|_| (ItemNo::default(), 0)),
        cash: Coins::default(),
        maxhp: k.maxhp,
        alignment: k.alignment,
        followpct: k.followpct,
        dr: k.dr,
        ac: k.ac,
        exp: k.exp,
        maxenergy: k.energy as i16,
        friendly,
        genroom: RoomId(ARENA),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: k.montype,
        target: -1,
        hpregen: k.hpregen,
        lastdir: 0,
        lastupd: 0,
        grouprole: GroupRole::Solo,
        spells: std::array::from_fn(|_| ActiveSpell {
            spell: Default::default(),
            value: 0,
            rounds: 0,
        }),
        downrounds: 0,
        created: 0,
    });
    // resident in the cache, as a monster in play is
    w.active_monsters.touch(number);
    let rs = w.room_state.get_mut(&RoomKey::new(1, ARENA)).unwrap();
    *rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap() = number;
    number
}

fn alive(w: &World, n: ActiveMonsterId) -> bool {
    w.active_monsters.peek(n).is_some()
}

fn in_room(w: &World, n: ActiveMonsterId) -> bool {
    w.room_state[&RoomKey::new(1, ARENA)].monsters.contains(&n)
}

fn curalive(w: &World, k: MonsterNo) -> i32 {
    w.monsters[&k].curalive.into()
}

/// Everything the player has been sent so far, then cleared.
fn out(w: &mut World, id: SessionId) -> Vec<u8> {
    let prompt = silvermere_engine::tell::prompt_bytes(w, id);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    b
}

/// The monsters one Villain leaves among: each of these but the first stays.
struct Crowd {
    gone: ActiveMonsterId,
    friend: ActiveMonsterId,
    other_owner: ActiveMonsterId,
    other_type: ActiveMonsterId,
}

fn crowd(w: &mut World) -> Crowd {
    Crowd {
        // the name is matched ignoring case (`eq_ignore_case`)
        gone: spawn(w, 1, HUNTER, "KASIMIR", 0),
        friend: spawn(w, 2, HUNTER, "Kasimir", 1),
        other_owner: spawn(w, 3, HUNTER, "Brigit", 0),
        other_type: spawn(w, 4, RAT, "Kasimir", 0),
    }
}

// A Villain who leaves loses his hunters
#[test]
fn a_villain_leaving_loses_the_hunter_on_his_trail() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir", 150);
    let watcher = player_in(&mut w, "Brigit", 0);
    let c = crowd(&mut w);
    out(&mut w, k);
    out(&mut w, watcher);
    cleanup_when_user_leaves(&mut w, k);
    assert!(!alive(&w, c.gone));
    assert!(!in_room(&w, c.gone));
    // removed as killed: one fewer alive, killed now
    assert_eq!(curalive(&w, HUNTER), 7);
    // the rest of the cache is left as it was
    assert!(alive(&w, c.friend), "a friend stays");
    assert!(alive(&w, c.other_owner), "one following someone else stays");
    assert!(alive(&w, c.other_type), "another type stays");
    assert_eq!(curalive(&w, RAT), 8);
    // no room text and nothing to the room
    assert!(out(&mut w, k).is_empty());
    assert!(out(&mut w, watcher).is_empty());
}

// A FIEND loses them too
#[test]
fn a_fiend_loses_it_too() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir", 210);
    let c = crowd(&mut w);
    cleanup_when_user_leaves(&mut w, k);
    assert!(!alive(&w, c.gone));
    assert!(alive(&w, c.friend) && alive(&w, c.other_owner) && alive(&w, c.other_type));
}

// The legal level: 120 is the first Villain, 209 the last
#[test]
fn only_a_villain_or_a_fiend_loses_it() {
    for (evil, loses) in [
        (-300, false),
        (0, false),
        (79, false),
        (119, false),
        (120, true),
        (209, true),
        (210, true),
    ] {
        let mut w = world();
        let k = player_in(&mut w, "Kasimir", evil);
        let c = crowd(&mut w);
        cleanup_when_user_leaves(&mut w, k);
        assert_eq!(alive(&w, c.gone), !loses, "evil {evil}");
        assert!(alive(&w, c.friend), "evil {evil}");
        assert_eq!(
            curalive(&w, HUNTER),
            if loses { 7 } else { 8 },
            "evil {evil}"
        );
    }
}

// The logoff, which the hang-up and the exit wait end with
#[test]
fn the_logoff_dismisses_them() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir", 150);
    let c = crowd(&mut w);
    on_logoff(&mut w, k);
    assert!(!alive(&w, c.gone));
    assert!(!in_room(&w, c.gone));
    assert!(alive(&w, c.other_owner));
}

// The walk is over the cache slots only, so
// a hunter that is not resident is not seen and stays (here the cache is replaced by an
// empty one, so only the monster looked up afterwards is in it).
#[test]
fn a_hunter_not_in_the_cache_is_not_seen() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir", 150);
    *w.active_monsters.cache_mut() = MonsterCache::new(4);
    let seen = spawn(&mut w, 1, HUNTER, "Kasimir", 0);
    let unseen = spawn(&mut w, 2, HUNTER, "Kasimir", 0);
    // 2 is no longer resident, its record stays
    *w.active_monsters.cache_mut() = MonsterCache::new(4);
    w.active_monsters.touch(seen);
    assert_eq!(w.active_monsters.cache().slot_of(unseen), None);
    cleanup_when_user_leaves(&mut w, k);
    assert!(!alive(&w, seen));
    assert!(alive(&w, unseen), "not resident, not seen");
    assert!(in_room(&w, unseen));
    assert_eq!(curalive(&w, HUNTER), 7);
}

// The hunters go in cache-slot order, and each
// number goes to the head of the free list. MONSBUF 4: 5 hashes to slot 1 and 2 to slot 2,
// so 5 goes first though 2 is the lower number; the list is then [2, 5] and the next two
// numbers generated are 5, then 2.
#[test]
fn the_hunters_go_in_slot_order_and_their_numbers_are_reused_in_that_order() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir", 150);
    *w.active_monsters.cache_mut() = MonsterCache::new(4);
    // the spawn order puts 5 in slot 1 (5 % 4) and 2 in slot 2
    let five = spawn(&mut w, 5, HUNTER, "Kasimir", 0);
    let two = spawn(&mut w, 2, HUNTER, "Kasimir", 0);
    let cache = w.active_monsters.cache();
    assert_eq!(
        (cache.slot_of(five), cache.slot_of(two)),
        (Some(1), Some(2))
    );
    drop(cache);
    cleanup_when_user_leaves(&mut w, k);
    assert!(!alive(&w, five) && !alive(&w, two));
    assert_eq!(
        w.active_monsters.numbers.waiting().collect::<Vec<_>>(),
        [ActiveMonsterId(2), ActiveMonsterId(5)]
    );
    assert_eq!(w.active_monsters.numbers.take(), ActiveMonsterId(5));
    assert_eq!(w.active_monsters.numbers.take(), ActiveMonsterId(2));
}
