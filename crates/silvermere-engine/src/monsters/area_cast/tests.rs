//! `monsters::area_cast`: a monster's spell on the room.

use silvermere_types::enums::{ResistMode, SpellTarget, SpellType};
use silvermere_types::records::Spell;

use super::*;
use crate::spells::test_fixture::{
    self as fx, OTHER, QUAKE, ROOM, SHAMAN, enter, lines, plr, spawn,
};

fn add_spell(world: &mut World, s: Spell) -> SpellNo {
    let n = s.number;
    world.spells.insert(n, s);
    n
}

fn area(n: i16, kind: SpellTarget, v: i16, duration: i16, abl: &[(Ability, i16)]) -> Spell {
    let mut s = fx::spell(
        n,
        &format!("blast {n}"),
        SpellType::Autocombat,
        kind,
        (v, v),
        duration,
        0,
        abl,
    );
    s.resmode = ResistMode::Never;
    s
}

fn hp(world: &mut World, id: SessionId) -> i16 {
    plr(world, id).hp
}

fn follow(world: &mut World, leader: SessionId, follower: SessionId) {
    world.session_mut(follower).unwrap().party.leader = Some(leader);
    crate::party::add_to_group(world, leader, follower, false);
}

// The nomadic shaman's quake (its attack slot
// 1, Full Attack Area, Inflict Damage on Armour) hits both members of a party and nobody in
// the next room; each is shown the cast (with the Starting Message) while the first slot
// acts; the room hears an empty light-red line for each.
#[test]
fn shaman_quake_hits_both_party_members() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let far = enter(&mut w, "Dan", OTHER);
    follow(&mut w, a, b);
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    for id in [a, b, far] {
        lines(&mut w, id);
    }
    let r = monster_area_cast(&mut w, shaman, Some(1), QUAKE, 10);
    assert_eq!(r, AreaCastResult::Cast);
    // 10-20, no element cut, no magic resistance: a resistance that would go below 0
    // still cuts the full 50% (sic)
    for id in [a, b] {
        let lost = 100 - hp(&mut w, id);
        assert!((5..=10).contains(&lost), "{lost}");
        let out = lines(&mut w, id);
        assert_eq!(out[0], "nomadic shaman raises his staff in the air!");
        assert_eq!(out[1], format!("A quake rocks the room for {lost} damage!"));
    }
    assert_eq!(hp(&mut w, far), 100);
    assert!(lines(&mut w, far).is_empty());
    assert_eq!(
        combat::monster(&w, shaman).unwrap().energy,
        0,
        "the slot's 1000"
    );
    // no energy left: nothing is cast, the stats pass still runs (1)
    let before = hp(&mut w, a);
    assert_eq!(
        monster_area_cast(&mut w, shaman, Some(1), QUAKE, 10),
        AreaCastResult::Cast
    );
    assert_eq!(hp(&mut w, a), before);
}

// (sic) An instant "Inflict Damage Ignoring Armour" hurts nobody unless the kind is
// Divided Area Incl. You, Full Party Area or Divided Party Area; the divided share is the
// roll over the players hit.
#[test]
fn damage_ignoring_armour_by_kind_and_division() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    let full = add_spell(
        &mut w,
        area(
            970,
            SpellTarget::FullAttack,
            20,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    assert_eq!(
        monster_area_cast(&mut w, shaman, None, full, 0),
        AreaCastResult::Cast
    );
    assert_eq!((hp(&mut w, a), hp(&mut w, b)), (100, 100));
    lines(&mut w, a);
    let div = add_spell(
        &mut w,
        area(
            971,
            SpellTarget::DivAreaIncl,
            20,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    monster_area_cast(&mut w, shaman, None, div, 0);
    assert_eq!((hp(&mut w, a), hp(&mut w, b)), (90, 90));
    // his own line, then the room's line for Bea (to all but her)
    assert_eq!(
        lines(&mut w, a),
        vec![
            "Nomadic shaman cast blast 971 on you.",
            "Nomadic shaman cast blast 971 on Bea."
        ]
    );
}

// A failed cast roll costs half the slot's energy (at least 1); with
// nobody in the room nothing is cast at all (0).
#[test]
fn failed_cast_and_empty_room() {
    let mut w = fx::world();
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    assert_eq!(
        monster_area_cast(&mut w, shaman, Some(1), QUAKE, 10),
        AreaCastResult::Nothing
    );
    let a = enter(&mut w, "Alf", ROOM);
    w.monsters.get_mut(&SHAMAN).unwrap().attacks[1].min = 0;
    assert_eq!(
        monster_area_cast(&mut w, shaman, Some(1), QUAKE, 10),
        AreaCastResult::Cast
    );
    assert_eq!(combat::monster(&w, shaman).unwrap().energy, 500);
    assert_eq!(hp(&mut w, a), 100);
}

// A lasting area spell goes on every marked player with the record's
// duration and each one is shown it; when nobody takes it the cast ends with 0.
#[test]
fn lasting_area_spell_and_refusal() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    let n = add_spell(
        &mut w,
        // (the stat alters do nothing in a monster's area cast; "Alter damage"
        // takes the default, lasting branch)
        area(972, SpellTarget::FullArea, 4, 7, &[(Ability::Damage, 0)]),
    );
    lines(&mut w, a);
    assert_eq!(
        monster_area_cast(&mut w, shaman, None, n, 0),
        AreaCastResult::Cast
    );
    for id in [a, b] {
        let p = plr(&mut w, id);
        let k = p.spells.iter().position(|&s| s == n).expect("added");
        assert_eq!((p.spellvals[k], p.spellrnds[k]), (4, 7));
    }
    // Alf sees his own line and Bea's room line
    assert_eq!(
        lines(&mut w, a),
        vec![
            "Nomadic shaman cast blast 972 on you.",
            "Nomadic shaman cast blast 972 on Bea."
        ]
    );
    // again with the same strength: refused by everyone, 0 (renewal needs more)
    assert_eq!(
        monster_area_cast(&mut w, shaman, None, n, 0),
        AreaCastResult::Nothing
    );
}

// A death (at DEATHHP, -200 by default) is a kill by nobody worth
// nothing (2).
#[test]
fn a_kill_returns_two() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    plr(&mut w, a).hp = 5;
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    let n = add_spell(
        &mut w,
        area(
            973,
            SpellTarget::FullParty,
            300,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    assert_eq!(
        monster_area_cast(&mut w, shaman, None, n, 0),
        AreaCastResult::Killed
    );
}

// "Remove spell" takes the named spell off each marked player and
// his mark with it, so the slots after it leave him alone.
#[test]
fn remove_spell_unmarks() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    let bless = add_spell(&mut w, area(974, SpellTarget::OnSelf, 1, 10, &[]));
    {
        let p = plr(&mut w, a);
        p.spells[0] = bless;
        p.spellvals[0] = 1;
        p.spellrnds[0] = 5;
    }
    let n = add_spell(
        &mut w,
        area(
            975,
            SpellTarget::FullArea,
            10,
            0,
            &[(Ability::RemoveSpell, bless.0), (Ability::Heal, 0)],
        ),
    );
    plr(&mut w, a).hp = 50;
    plr(&mut w, b).hp = 50;
    monster_area_cast(&mut w, shaman, None, n, 0);
    assert_eq!(plr(&mut w, a).spells[0], SpellNo(0));
    assert_eq!((hp(&mut w, a), hp(&mut w, b)), (50, 60));
}

/// What `id` has queued, prompts as captured, ANSI kept (T-099's flush rendering).
fn raw(world: &mut World, id: SessionId) -> String {
    let palette = crate::tell::palette_of(world, id);
    let b = world
        .session(id)
        .unwrap()
        .output
        .render_in(world, id, palette);
    world.session_mut(id).unwrap().output.clear();
    String::from_utf8_lossy(&b).into_owned()
}

// An instant area effect works one player at a time (his damage,
// his display, his kill or knock-out, then the next), so the line the room is told about
// the first player carries the second player's prompt from before his own damage (T-100).
#[test]
fn an_area_blast_shows_each_player_his_figures_of_the_moment() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    follow(&mut w, a, b);
    let n = add_spell(
        &mut w,
        area(
            930,
            SpellTarget::FullParty,
            10,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    let shaman = spawn(&mut w, SHAMAN, ROOM);
    for id in [a, b] {
        raw(&mut w, id);
    }
    monster_area_cast(&mut w, shaman, None, n, 0);
    assert_eq!((hp(&mut w, a), hp(&mut w, b)), (90, 90));
    const CL: &str = "\x1b[79D\x1b[K";
    let line = |who: &str| format!("{CL}\x1b[1;31mNomadic shaman cast blast 930 on {who}.\r\n");
    let prompt = |hp: u32| format!("{CL}\x1b[0;37m[HP={hp}\x1b[0;37m]:");
    assert_eq!(
        raw(&mut w, a),
        [line("you"), prompt(90), line("Bea"), prompt(90)].concat()
    );
    // Bea hears of Alf's blast with her 100 still whole
    assert_eq!(
        raw(&mut w, b),
        [line("Alf"), prompt(100), line("you"), prompt(90)].concat()
    );
}
