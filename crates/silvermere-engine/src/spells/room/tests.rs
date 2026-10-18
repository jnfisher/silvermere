//! `spells::room`: a room's spell on a player and on a monster.

use silvermere_types::enums::{Ability, ResistMode, SpellTarget, SpellType};
use silvermere_types::flags::ActiveMonsterFlags;
use silvermere_types::ids::{ItemNo, MonsterNo, RoomId};

use super::*;
use crate::spells::test_fixture::{
    self as fx, FREEZE, MAGMA, OTHER, RAT, ROOM, SMITE, enter, lines, plr, spawn,
};

fn add_spell(world: &mut World, s: Spell) -> SpellNo {
    let n = s.number;
    world.spells.insert(n, s);
    n
}

/// A room spell of `kind` with fixed strength `v` and these slots, no WCCMSG record.
fn room_spell(n: i16, kind: SpellTarget, v: i16, duration: i16, abl: &[(Ability, i16)]) -> Spell {
    fx::spell(
        n,
        &format!("trap {n}"),
        SpellType::General,
        kind,
        (v, v),
        duration,
        0,
        abl,
    )
}

fn hp(world: &mut World, id: SessionId) -> i16 {
    plr(world, id).hp
}

// The cast line, the shatter line and the dazed line: the lines as the player sees them.
#[test]
fn room_cast_lines() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    let mut out = Vec::new();
    // a General spell, WCCMSG 7: "The room" as the caster (stock smite, the Cast exit of
    // The Silver River), the descriptive line after it
    room_cast_on_user(&mut w, a, SMITE);
    out.push(format!("smite (General): {:?}", lines(&mut w, a)));
    // an attack spell with alternate messaging: the amount only (stock magma heat)
    room_cast_on_user(&mut w, a, MAGMA);
    out.push(format!(
        "magma heat (Auto-Combat, ALTMSG): {:?}",
        lines(&mut w, a)
    ));
    // a General spell with alternate messaging (stock freezing water)
    room_cast_on_user(&mut w, a, FREEZE);
    out.push(format!(
        "freezing water (General, ALTMSG): {:?}",
        lines(&mut w, a)
    ));
    // no WCCMSG record: the built-in text, "The room" again, an attack spell
    let mut s = room_spell(
        951,
        SpellTarget::Single,
        7,
        0,
        &[(Ability::DamageNoArmour, 0)],
    );
    s.name = "spikes".into();
    s.spell_type = SpellType::Autocombat;
    let n = add_spell(&mut w, s);
    room_cast_on_user(&mut w, a, n);
    out.push(format!(
        "spikes (Auto-Combat, no WCCMSG): {:?}",
        lines(&mut w, a)
    ));
    // nobody else hears a room's cast
    out.push(format!("Bea: {:?}", lines(&mut w, b)));
    // Shatter: the weapon breaks
    let itm = w.items.get_mut(&ItemNo(1)).unwrap();
    itm.name = "long sword".into();
    {
        let p = plr(&mut w, a);
        p.items[0] = ItemNo(1);
        p.itemuses[0] = -1;
        p.weapon = ItemNo(1);
    }
    let n = add_spell(
        &mut w,
        room_spell(952, SpellTarget::Single, 5, 0, &[(Ability::Shatter, 0)]),
    );
    room_cast_on_user(&mut w, a, n);
    out.push(format!("shatter: {:?}", lines(&mut w, a)));
    assert_eq!(plr(&mut w, a).weapon, ItemNo(0));
    // a monster knocked down rises after its rounds
    let rat = spawn(&mut w, RAT, ROOM);
    {
        let m = combat::monster_mut(&mut w, rat).unwrap();
        m.flags.insert(ActiveMonsterFlags::DOWN);
        m.downrounds = 1;
    }
    lines(&mut w, b);
    crate::monsters::following::fast_update_monster(&mut w, rat);
    out.push(format!("dazed: {:?}", lines(&mut w, b)));
    insta::assert_snapshot!(out.join("\n"));
    // the colour rows: General and the others
    lines(&mut w, a);
    room_cast_on_user(&mut w, a, SMITE);
    room_cast_on_user(&mut w, a, MAGMA);
    let msgs: Vec<_> = w
        .session_mut(a)
        .unwrap()
        .output
        .take()
        .0
        .into_iter()
        .filter_map(|i| match i {
            crate::output::Item::Msg { msg, .. } => Some(msg.key()),
            _ => None,
        })
        .collect();
    assert_eq!(msgs, ["spell_line_gen", "spell_line_oth"]);
}

// The resistance roll and a worn item stop the cast; the strength is cut by the element
// resistance.
#[test]
fn resisted_negated_and_element_cut() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let mut s = room_spell(
        953,
        SpellTarget::Single,
        40,
        0,
        &[(Ability::DamageNoArmour, 0)],
    );
    s.spell_type = SpellType::Autocombat;
    s.element = silvermere_types::enums::Element::from_code(1);
    s.resmode = ResistMode::Always;
    let n = add_spell(&mut w, s);
    room_cast_on_user(&mut w, a, n);
    assert_eq!(hp(&mut w, a), 60, "40 damage, no magic resistance");
    // Hot defence 50: half
    plr(&mut w, a).abils[0] = (Ability::HotDefence, 50);
    room_cast_on_user(&mut w, a, n);
    assert_eq!(hp(&mut w, a), 40);
    // a worn item negating it
    w.items.get_mut(&ItemNo(1)).unwrap().negates[0] = n;
    plr(&mut w, a).worn[0] = ItemNo(1);
    room_cast_on_user(&mut w, a, n);
    assert_eq!(hp(&mut w, a), 40);
    // magic resistance 250 (held to 97): almost always resisted
    plr(&mut w, a).worn[0] = ItemNo(0);
    plr(&mut w, a).mr = 250;
    let mut resisted = 0;
    for _ in 0..100 {
        let before = hp(&mut w, a);
        plr(&mut w, a).hp = 100;
        room_cast_on_user(&mut w, a, n);
        resisted += i32::from(hp(&mut w, a) == 100);
        plr(&mut w, a).hp = before;
    }
    assert!(resisted > 85, "{resisted}");
}

// A lasting slot adds the spell and tells the player once per cast; an instant slot after
// it says nothing; the lasting stat alters add it silently; Hunger and Thirst say nothing.
#[test]
fn the_spell_is_added_once_per_cast() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let n = add_spell(
        &mut w,
        room_spell(
            954,
            SpellTarget::Single,
            3,
            10,
            &[
                (Ability::DamageNoArmour, 0),
                (Ability::Energy, 0),
                (Ability::Heal, 0),
            ],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    assert_eq!(
        lines(&mut w, a),
        vec!["The room cast trap 954 on you for 3 damage."]
    );
    let p = plr(&mut w, a);
    assert_eq!(p.spells.iter().filter(|&&s| s == n).count(), 1);
    assert_eq!(p.hp, 100, "lasting: no instant damage");
    // instant: each slot tells its own amount
    let n = add_spell(
        &mut w,
        room_spell(
            955,
            SpellTarget::Any,
            6,
            0,
            &[
                (Ability::DamageNoArmour, 0),
                (Ability::Hunger, 0),
                (Ability::Heal, 4),
            ],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    assert_eq!(
        lines(&mut w, a),
        vec![
            "The room cast trap 955 on you for 6 damage.",
            "The room cast trap 955 on you for 4 damage."
        ]
    );
    assert_eq!(hp(&mut w, a), 98);
    // the stat alters: added once, the stat raised for each slot, nothing said
    let n = add_spell(
        &mut w,
        room_spell(
            956,
            SpellTarget::OnSelf,
            2,
            10,
            &[(Ability::Strength, 0), (Ability::Agility, 0)],
        ),
    );
    let (str0, agi0) = (plr(&mut w, a).str, plr(&mut w, a).agi);
    room_cast_on_user(&mut w, a, n);
    assert!(lines(&mut w, a).is_empty());
    let p = plr(&mut w, a);
    assert_eq!((p.str, p.agi), (str0 + 2, agi0 + 2));
    // an area kind: nothing at all
    let n = add_spell(
        &mut w,
        room_spell(
            957,
            SpellTarget::FullArea,
            6,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    assert!(lines(&mut w, a).is_empty());
    assert_eq!(hp(&mut w, a), 98);
}

// The teleport waits for the last slot; the player lets go of his followers and is moved;
// the damage slot after the teleport slot still hit him here.
#[test]
fn teleport_after_the_last_slot() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let b = enter(&mut w, "Bea", ROOM);
    w.session_mut(b).unwrap().party.leader = Some(a);
    crate::party::add_to_group(&mut w, a, b, false);
    let n = add_spell(
        &mut w,
        room_spell(
            958,
            SpellTarget::Single,
            5,
            0,
            &[
                (Ability::TeleportRoom, OTHER as i16),
                (Ability::TeleportMap, 1),
                (Ability::DamageNoArmour, 0),
            ],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    let p = plr(&mut w, a);
    assert_eq!((p.room, p.hp), (RoomId(OTHER), 95));
    assert_eq!(p.trailrooms[0], RoomId(OTHER));
    assert_eq!(w.session(b).unwrap().party.leader, None);
    assert_eq!(plr(&mut w, b).room, RoomId(ROOM));
    // a map that is not there: no move
    let n = add_spell(
        &mut w,
        room_spell(
            959,
            SpellTarget::Single,
            5,
            0,
            &[
                (Ability::TeleportRoom, ROOM as i16),
                (Ability::TeleportMap, 9),
            ],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    assert_eq!(plr(&mut w, a).room, RoomId(OTHER));
}

// Scatter moves the coins and the items that can be taken; Clear Item erases one.
#[test]
fn scatter_and_clear_item() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    w.items.get_mut(&ItemNo(1)).unwrap().gettable = 1;
    {
        let rs = w.room_state.get_mut(&fx::key(ROOM)).unwrap();
        rs.coins.gold = 7;
        rs.hidcoins.copper = 3;
        rs.items[0] = silvermere_types::records::RoomItem {
            item: ItemNo(1),
            uses: -1,
            count: 1,
        };
    }
    let n = add_spell(
        &mut w,
        room_spell(
            960,
            SpellTarget::Single,
            OTHER as i16,
            0,
            &[(Ability::Scatter, 0)],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    let from = &w.room_state[&fx::key(ROOM)];
    assert_eq!((from.coins.gold, from.hidcoins.copper), (0, 0));
    assert_eq!(from.items[0].item, ItemNo(0));
    let to = &w.room_state[&fx::key(OTHER)];
    assert_eq!((to.coins.gold, to.hidcoins.copper), (7, 3));
    let copies: i32 = to
        .items
        .iter()
        .filter(|s| s.item == ItemNo(1))
        .map(|s| i32::from(s.count) + 1)
        .sum();
    assert_eq!(copies, 2);
    // Clear Item, in the room he stands in
    let n = add_spell(
        &mut w,
        room_spell(961, SpellTarget::Single, 1, 0, &[(Ability::ClearItem, 1)]),
    );
    plr(&mut w, a).room = RoomId(OTHER);
    room_cast_on_user(&mut w, a, n);
    assert!(
        w.room_state[&fx::key(OTHER)]
            .items
            .iter()
            .all(|s| s.item != ItemNo(1))
    );
}

// The summoned monster follows the player by name, no friend.
#[test]
fn summon_hunts_the_player() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    let n = add_spell(
        &mut w,
        room_spell(962, SpellTarget::Single, 1, 0, &[(Ability::Summon, 1)]),
    );
    room_cast_on_user(&mut w, a, n);
    let m = w
        .room_state
        .get(&fx::key(ROOM))
        .unwrap()
        .monsters
        .iter()
        .find(|m| m.0 != 0)
        .copied()
        .expect("a monster");
    let m = combat::monster(&w, m).unwrap();
    assert_eq!((m.following.as_str(), m.friendly), ("Alf", 0));
    // generate_monster announces it first, then the cast line
    assert_eq!(
        lines(&mut w, a),
        vec![
            "giant rat just arrived from nowhere.",
            "The room cast trap 962 on you for 1 damage."
        ]
    );
}

// SYSOP DEBUG shows each slot's ability and amount first.
#[test]
fn sysop_debug_line() {
    let mut w = fx::world();
    let a = enter(&mut w, "Alf", ROOM);
    w.session_mut(a).unwrap().flags.debug = silvermere_types::enums::DebugLevel::On;
    let n = add_spell(
        &mut w,
        room_spell(
            963,
            SpellTarget::Single,
            4,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    room_cast_on_user(&mut w, a, n);
    let out = lines(&mut w, a);
    assert_eq!(out.len(), 2);
    assert!(out[0].ends_with(" (4)"), "{out:?}");
}

// The filters, the immunity, the slots until the monster dies; the kill ends everyone's
// fight with it.
#[test]
fn room_cast_on_a_monster() {
    let mut w = fx::world();
    let rat = spawn(&mut w, RAT, ROOM);
    let n = add_spell(
        &mut w,
        room_spell(
            964,
            SpellTarget::Monster,
            30,
            0,
            &[(Ability::DamageNoArmour, 0)],
        ),
    );
    room_cast_on_monster(&mut w, rat, n);
    assert_eq!(combat::monster(&w, rat).unwrap().hp, 470);
    // Kill Dead: only the undead
    let n2 = add_spell(
        &mut w,
        room_spell(
            965,
            SpellTarget::Monster,
            30,
            0,
            &[(Ability::DamageNoArmour, 0), (Ability::KillDead, 0)],
        ),
    );
    room_cast_on_monster(&mut w, rat, n2);
    assert_eq!(combat::monster(&w, rat).unwrap().hp, 470);
    // Spell Immunity above the spell's level
    w.monsters.get_mut(&RAT).unwrap().abilities[0] = (Ability::SpellImmune, 5);
    room_cast_on_monster(&mut w, rat, n);
    assert_eq!(combat::monster(&w, rat).unwrap().hp, 470);
    w.monsters.get_mut(&RAT).unwrap().abilities[0] = (Ability::None, 0);
    // a lasting spell goes in once per cast, whatever slots follow (sic: into every free
    // slot)
    let n3 = add_spell(
        &mut w,
        room_spell(
            966,
            SpellTarget::Monster,
            2,
            5,
            &[(Ability::Strength, 0), (Ability::Agility, 0)],
        ),
    );
    room_cast_on_monster(&mut w, rat, n3);
    let m = combat::monster(&w, rat).unwrap();
    assert!(m.spells.iter().all(|s| s.spell == n3));
    // the kill
    combat::monster_mut(&mut w, rat).unwrap().hp = 10;
    room_cast_on_monster(&mut w, rat, n);
    assert!(combat::monster(&w, rat).is_none());
    let _ = MonsterNo(0);
}
