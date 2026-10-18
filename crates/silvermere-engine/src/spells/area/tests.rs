//! `spells::area`: the one target iteration.

use proptest::prelude::*;
use silvermere_rules::Rng as _;
use silvermere_rules::spells::resist::{item_negates, player_resists};
use silvermere_types::enums::{Alignment, MonsterType, ResistMode, SpellTarget, SpellType};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, SessionId, SpellNo};

use super::*;
use crate::spells::test_fixture::{self as fx, OTHER, RAT, ROOM, SHAMAN, enter, plr, spawn};

/// An area spell of `kind` that everybody may resist (`SRES_ALWAYS`).
fn area_spell(kind: SpellTarget) -> Spell {
    let mut s = fx::spell(
        950,
        "test area",
        SpellType::Autocombat,
        kind,
        (20, 20),
        0,
        0,
        &[(Ability::DamageNoArmour, 0)],
    );
    s.resmode = ResistMode::Always;
    s
}

fn follow(world: &mut World, leader: SessionId, follower: SessionId) {
    world.session_mut(follower).unwrap().party.leader = Some(leader);
    crate::party::add_to_group(world, leader, follower, false);
}

fn marked(world: &World, id: SessionId) -> bool {
    combat::player(world, id)
        .unwrap()
        .status2
        .contains(PlayerStatus2::AREATARGET)
}

/// Players and monsters reached, and the count's evil and only-hidden flags.
type T050 = (
    Vec<(SessionId, bool)>,
    Vec<(ActiveMonsterId, bool)>,
    bool,
    bool,
);

/// T-050's inline loop (`spells/cast.rs` before this card): the valid players who did not
/// resist, then the valid monsters; the same draws in the same order.
fn t050_loop(world: &mut World, id: SessionId, spl: &Spell) -> T050 {
    let mut players = Vec::new();
    let (mut evil, mut only_hidden) = (false, true);
    let c = combat::player(world, id).unwrap().clone();
    let sees_hidden = c.status.contains(PlayerStatus::SEEHIDDEN);
    let mut ids = world.session_ids();
    ids.sort();
    for tid in ids {
        if !world.is_in_game(tid) {
            continue;
        }
        let Some(v) = valid_player(world, id, tid, spl) else {
            continue;
        };
        let tp = combat::player(world, tid).unwrap();
        let worn: Vec<_> = tp.worn.iter().filter_map(|n| world.items.get(n)).collect();
        let negated = item_negates(worn, spl.number);
        let anti = user_has(world, tp, Ability::AntiMagic);
        let (mr, hidden) = (i32::from(tp.mr), tp.hidden);
        if !player_resists(spl, mr, anti, negated, &mut world.rng) {
            if v.evil {
                evil = true;
                if hidden == 0 || sees_hidden {
                    only_hidden = false;
                }
            }
            players.push((tid, v.evil));
        }
    }
    let mut monsters = Vec::new();
    if spl.target.area_reaches_monsters() {
        for &n in world.room_state[&room_of(&c)].monsters.iter() {
            if n.0 == 0 {
                continue;
            }
            if let Some(v) = valid_monster(world, id, n, spl) {
                if v.evil {
                    evil = true;
                    only_hidden = false;
                }
                monsters.push((n, v.evil));
            }
        }
    }
    (players, monsters, evil, only_hidden)
}

#[derive(Debug, Clone)]
struct Who {
    level: u16,
    evil: i16,
    mr: u16,
    hidden: bool,
    leader: Option<usize>,
}

#[derive(Debug, Clone)]
struct Mon {
    alignment: u16,
    montype: u16,
    follows: bool,
    charmed: bool,
}

fn build(
    kind: SpellTarget,
    who: &[Who],
    mons: &[Mon],
    seed: u64,
) -> (World, Vec<SessionId>, Spell) {
    let mut w = fx::world();
    let spl = area_spell(kind);
    w.spells.insert(spl.number, spl.clone());
    let ids: Vec<SessionId> = (0..who.len())
        .map(|i| enter(&mut w, &format!("P{i}"), ROOM))
        .collect();
    for (i, x) in who.iter().enumerate() {
        let p = plr(&mut w, ids[i]);
        p.level = x.level;
        p.evil = x.evil;
        p.mr = x.mr;
        p.hidden = u8::from(x.hidden);
    }
    for (i, x) in who.iter().enumerate() {
        if let Some(l) = x.leader
            && l != i
            && l < who.len()
        {
            follow(&mut w, ids[l], ids[i]);
        }
    }
    for m in mons {
        let n = spawn(&mut w, RAT, ROOM);
        let a = combat::monster_mut(&mut w, n).unwrap();
        a.alignment = Alignment::from_code(m.alignment);
        a.montype = MonsterType::from_code(m.montype);
        if m.follows {
            a.following = "P0".into();
        }
        if m.charmed {
            a.flags.insert(ActiveMonsterFlags::CHARMED);
        }
    }
    w.rng = silvermere_rules::SeededRng::new(seed);
    (w, ids, spl)
}

fn kinds() -> impl Strategy<Value = SpellTarget> {
    prop::sample::select(vec![3u16, 5, 9, 10, 11, 12, 13]).prop_map(SpellTarget::from_code)
}

fn whos() -> impl Strategy<Value = Vec<Who>> {
    prop::collection::vec(
        (
            1u16..20,
            0i16..80,
            0u16..250,
            any::<bool>(),
            prop::option::of(0usize..5),
        )
            .prop_map(|(level, evil, mr, hidden, leader)| Who {
                level,
                evil,
                mr,
                hidden,
                leader,
            }),
        1..6,
    )
}

fn mons() -> impl Strategy<Value = Vec<Mon>> {
    prop::collection::vec(
        (
            0u16..7,
            prop::sample::select(vec![0u16, 1, 2, 5, 37]),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(alignment, montype, follows, charmed)| Mon {
                alignment,
                montype,
                follows,
                charmed,
            }),
        0..4,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    // Card §4: with no mark left over from an earlier cast, the player caster's targets are
    // exactly T-050's inline loop's, with the same draws.
    #[test]
    fn player_targets_equal_t050_loop(kind in kinds(), who in whos(), mons in mons(), seed in any::<u64>()) {
        let (mut a, ids, spl) = build(kind, &who, &mons, seed);
        let (mut b, _, _) = build(kind, &who, &mons, seed);
        let new = count_valid_targets(&mut a, ids[0], &spl);
        let (players, monsters, evil, only_hidden) = t050_loop(&mut b, ids[0], &spl);
        prop_assert_eq!(&new.players, &players);
        prop_assert_eq!(new.nplayers, players.len() as i32);
        prop_assert_eq!(&new.monsters, &monsters);
        prop_assert_eq!((new.evil, new.only_hidden), (evil, only_hidden));
        prop_assert_eq!(a.rng.range(0, 1_000_000), b.rng.range(0, 1_000_000));
        // and `targets` reads the same off the marks
        let t = targets(&a, fx::key(ROOM), &Caster::Player(ids[0]), &spl);
        let expect: Vec<AreaTarget> = players
            .iter()
            .map(|p| AreaTarget::Player(p.0))
            .chain(monsters.iter().map(|m| AreaTarget::Monster(m.0)))
            .collect();
        prop_assert_eq!(t, expect);
    }
}

// The attack and party areas, and the pvp range: a party of two and a stranger in range.
#[test]
fn attack_area_spares_the_party_and_hits_the_stranger_and_monsters() {
    let (mut w, _, _) = build(SpellTarget::FullAttack, &[], &[], 1);
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let c = enter(&mut w, "Cyd", ROOM);
    let far = enter(&mut w, "Dan", OTHER);
    follow(&mut w, a, b);
    let rat = spawn(&mut w, RAT, ROOM);
    let spl = area_spell(SpellTarget::FullAttack);
    let t = count_valid_targets(&mut w, a, &spl);
    assert_eq!(t.players, vec![(c, true)]);
    assert_eq!(t.monsters, vec![(rat, false)]);
    assert!(marked(&w, c) && !marked(&w, a) && !marked(&w, b) && !marked(&w, far));
    // the party areas reach exactly the party, the caster too
    let spl = area_spell(SpellTarget::FullParty);
    // (hitting anyone who has no evil timer on the caster is evil, the party too)
    let t = count_valid_targets(&mut w, a, &spl);
    assert_eq!(t.players, vec![(a, true), (b, true)]);
    assert!(t.monsters.is_empty());
    // the follower casting: his leader is the party
    let t = count_valid_targets(&mut w, b, &spl);
    assert_eq!(t.players, vec![(a, true), (b, true)]);
    // out of the pvp range (level 9 apart is the most): the stranger is spared
    plr(&mut w, c).level = 20;
    plr(&mut w, a).level = 10;
    let spl = area_spell(SpellTarget::FullAttack);
    let t = count_valid_targets(&mut w, a, &spl);
    assert!(t.players.is_empty());
    assert!(!marked(&w, c), "an invalid player loses his mark");
}

// Guards and angels only for a caster of 40 evil points or more; never a Lawful Good
// monster; a Good one is hit and is an evil act.
#[test]
fn guards_angels_and_lawful_good() {
    let (mut w, _, _) = build(SpellTarget::FullAttack, &[], &[], 1);
    let a = enter(&mut w, "Alf", ROOM);
    let guard = spawn(&mut w, RAT, ROOM);
    let angel = spawn(&mut w, RAT, ROOM);
    let lg = spawn(&mut w, RAT, ROOM);
    let good = spawn(&mut w, RAT, ROOM);
    combat::monster_mut(&mut w, guard).unwrap().montype = MonsterType::Guard;
    combat::monster_mut(&mut w, angel).unwrap().montype = MonsterType::Angel;
    combat::monster_mut(&mut w, lg).unwrap().alignment = Alignment::LawfulGood;
    combat::monster_mut(&mut w, good).unwrap().alignment = Alignment::Good;
    let spl = area_spell(SpellTarget::FullAttack);
    let t = count_valid_targets(&mut w, a, &spl);
    assert_eq!(t.monsters, vec![(good, true)]);
    assert!(t.evil && !t.only_hidden);
    plr(&mut w, a).evil = 40;
    let t = count_valid_targets(&mut w, a, &spl);
    assert_eq!(
        t.monsters,
        vec![(guard, false), (angel, false), (good, true)]
    );
    // a full area takes every monster
    let t = count_valid_targets(&mut w, a, &area_spell(SpellTarget::FullArea));
    assert_eq!(t.monsters.len(), 4);
}

// A valid target who resists keeps a mark an earlier count gave him (sic), so the cast
// still reaches him; he is not counted for the division.
#[test]
fn a_stale_mark_outlives_a_resist() {
    let (mut w, _, _) = build(SpellTarget::FullArea, &[], &[], 1);
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let spl = area_spell(SpellTarget::FullArea);
    let t = count_valid_targets(&mut w, a, &spl);
    assert_eq!(t.nplayers, 2);
    assert!(marked(&w, b));
    // now Bea wears an item that negates it: she resists every time
    w.items
        .get_mut(&silvermere_types::ids::ItemNo(1))
        .unwrap()
        .negates[0] = spl.number;
    plr(&mut w, b).worn[0] = silvermere_types::ids::ItemNo(1);
    fx::lines(&mut w, a);
    let t = count_valid_targets(&mut w, a, &spl);
    assert_eq!(fx::lines(&mut w, a), vec!["Bea resists your spell!"]);
    assert_eq!(t.nplayers, 1, "not counted");
    assert_eq!(t.players, vec![(a, true), (b, true)], "still reached");
    assert!(marked(&w, b));
}

// A monster's count takes everyone in its room, party or level notwithstanding, and clears
// no mark; the room's reach is every player in the room.
#[test]
fn monster_and_room_casters() {
    let (mut w, _, _) = build(SpellTarget::FullAttack, &[], &[], 1);
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let far = enter(&mut w, "Dan", OTHER);
    follow(&mut w, a, b);
    plr(&mut w, far).status2.insert(PlayerStatus2::AREATARGET);
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    let quake = w.spells[&SpellNo(192)].clone();
    assert_eq!(monster_count_valid_targets(&mut w, shaman, &quake), 2);
    assert_eq!(
        targets(&w, fx::key(ROOM), &Caster::Monster(shaman), &quake),
        vec![AreaTarget::Player(a), AreaTarget::Player(b)]
    );
    assert!(marked(&w, far), "nobody's mark is cleared");
    assert_eq!(
        targets(&w, fx::key(OTHER), &Caster::Room(fx::key(OTHER)), &quake),
        vec![AreaTarget::Player(far)]
    );
}
