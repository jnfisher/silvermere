//! T-100: spell effects applied one step at a time, in the original's order, so each
//! prompt shows the figures of its own moment (T-099's queue-time prompts): the rounds of
//! a lasting spell, a lasting "Alter HP", and spells ended by another.

use silvermere_types::enums::{Ability, SpellTarget, SpellType};
use silvermere_types::ids::{MessageNo, SessionId, SpellNo};
use silvermere_types::records::{Message, Spell};

use super::cast::{CastMode, CastOpts, cast_no_target};
use super::test_fixture::{self as fx, ROOM, enter, plr};
use crate::world::World;

fn add(world: &mut World, s: Spell) -> SpellNo {
    let n = s.number;
    world.spells.insert(n, s);
    n
}

const CL: &str = "\x1b[79D\x1b[K";

/// The prompt of a class with no magery: `[HP=n]:`, the figure in its `get_hp_colour`
/// colour.
fn prompt(hp: &str) -> String {
    format!("{CL}\x1b[0;37m[HP={hp}\x1b[0;37m]:")
}

/// What `id` has queued, prompts as captured, ANSI kept (the flush's rendering).
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

// A lasting spell's round: each slot changes the hit points and sends its prompt there and
// then (damage and heal alike).
#[test]
fn a_spell_round_prompts_after_each_slot() {
    let mut w = fx::world();
    let n = add(
        &mut w,
        fx::spell(
            900,
            "ebb and flow",
            SpellType::General,
            SpellTarget::OnSelf,
            (0, 0),
            10,
            0,
            &[(Ability::Heal, 3), (Ability::DamageNoArmour, 5)],
        ),
    );
    let id = enter(&mut w, "Alia", ROOM);
    let p = plr(&mut w, id);
    p.hp = 50;
    p.spells[0] = n;
    p.spellrnds[0] = 5;
    raw(&mut w, id);
    super::active::tick_player_spells(&mut w, id);
    assert_eq!(plr(&mut w, id).hp, 48);
    // healed to 53, then hurt to 48 (below half: bright white)
    assert_eq!(
        raw(&mut w, id),
        [prompt("53"), prompt("\x1b[1;37m48")].concat()
    );
}

// A lasting Alter HP on the caster takes hold as a lasting add, which shows the cast with
// its prompt; only then do the maximum and current hit points rise.
#[test]
fn alter_hp_shows_the_cast_before_the_raise() {
    let mut w = fx::world();
    let n = add(
        &mut w,
        fx::spell(
            901,
            "vigour",
            SpellType::General,
            SpellTarget::OnSelf,
            (20, 20),
            10,
            7,
            &[(Ability::AlterHp, 0)],
        ),
    );
    let id = enter(&mut w, "Alia", ROOM);
    plr(&mut w, id).hp = 60;
    raw(&mut w, id);
    cast_no_target(
        &mut w,
        id,
        n,
        CastOpts {
            mode: CastMode::Chain,
            ..CastOpts::default()
        },
    );
    // the cast is shown with the 60 hit points of before the raise
    assert_eq!(
        raw(&mut w, id),
        [
            format!("{CL}\x1b[1;34mYou cast vigour on Alia!\r\n"),
            prompt("60"),
            "\x1b[79D\x1b[K\x1b[1;34m".to_owned(),
        ]
        .concat()
    );
    assert_eq!(plr(&mut w, id).hp, 80);
}

// Freedom shows the cast, then ends each holding spell one after the other: its ending line
// and prompt, then its undo (the Alter HP bonus goes), so the second ending's prompt shows
// the first undo and not the second.
#[test]
fn freedom_ends_each_spell_before_the_next() {
    let mut w = fx::world();
    w.messages.insert(
        MessageNo(950),
        Message {
            number: MessageNo(950),
            msgs: ["The %s lets go.".into(), String::new(), String::new()],
        },
    );
    for (n, bonus) in [(910, 10), (911, 5)] {
        add(
            &mut w,
            fx::spell(
                n,
                &format!("bind {n}"),
                SpellType::General,
                SpellTarget::Single,
                (0, 0),
                10,
                0,
                &[
                    (Ability::HoldPerson, 1),
                    (Ability::AlterHp, bonus),
                    (Ability::DescMsg, 950),
                ],
            ),
        );
    }
    let freedom = add(
        &mut w,
        fx::spell(
            920,
            "freedom",
            SpellType::General,
            SpellTarget::OnSelf,
            (1, 1),
            0,
            7,
            &[(Ability::Freedom, 0)],
        ),
    );
    let id = enter(&mut w, "Alia", ROOM);
    let p = plr(&mut w, id);
    (p.hp, p.maxhp) = (80, 115);
    p.spells[0] = SpellNo(910);
    p.spells[1] = SpellNo(911);
    raw(&mut w, id);
    cast_no_target(
        &mut w,
        id,
        freedom,
        CastOpts {
            mode: CastMode::Chain,
            ..CastOpts::default()
        },
    );
    let ending = |n: i16| format!("{CL}\x1b[0;33mThe bind {n} lets go.\r\n");
    assert_eq!(
        raw(&mut w, id),
        [
            format!("{CL}\x1b[1;34mYou cast freedom on Alia!\r\n"),
            prompt("80"),
            format!("{CL}\x1b[1;34m"),
            // the first ending, before its 10 go
            ending(910),
            prompt("80"),
            // the second, after the first's undo and before its own 5
            ending(911),
            prompt("70"),
        ]
        .concat()
    );
    assert_eq!(plr(&mut w, id).hp, 65);
}

// An instant Cure Poison on the caster ends his poison spell first (its ending line and
// prompt), then lowers the level (not below 0), then shows the cast.
#[test]
fn cure_poison_ends_the_poison_spell_before_the_cast_is_shown() {
    let mut w = fx::world();
    w.messages.insert(
        MessageNo(951),
        Message {
            number: MessageNo(951),
            msgs: ["The %s fades.".into(), String::new(), String::new()],
        },
    );
    let mut venom = fx::spell(
        912,
        "venom",
        SpellType::General,
        SpellTarget::Single,
        (0, 0),
        10,
        0,
        &[(Ability::Poison, 4), (Ability::DescMsg, 951)],
    );
    venom.element = silvermere_types::enums::Element::Poison;
    add(&mut w, venom);
    let cure = add(
        &mut w,
        fx::spell(
            921,
            "cure",
            SpellType::General,
            SpellTarget::OnSelf,
            (3, 3),
            0,
            7,
            &[(Ability::CurePoison, 0)],
        ),
    );
    let id = enter(&mut w, "Alia", ROOM);
    let p = plr(&mut w, id);
    p.poison = 6;
    p.spells[0] = SpellNo(912);
    raw(&mut w, id);
    cast_no_target(
        &mut w,
        id,
        cure,
        CastOpts {
            mode: CastMode::Chain,
            ..CastOpts::default()
        },
    );
    assert_eq!(
        raw(&mut w, id),
        [
            format!("{CL}\x1b[0;33mThe venom fades.\r\n"),
            prompt("100"),
            format!("{CL}\x1b[1;34mYou cast cure on Alia!\r\n"),
            prompt("100"),
        ]
        .concat()
    );
    // 6, less the ending's 4, less the cure's 3, not below 0
    assert_eq!(plr(&mut w, id).poison, 0);
}
