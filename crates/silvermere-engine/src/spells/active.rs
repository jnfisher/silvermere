//! Lasting spells once cast: their rounds on players (in `medium_update_character`) and on
//! monsters (in `medium_update_monster`), their end, and the confusion checks that cost a
//! player or a monster the action ([`check_confusion`], [`check_monster_confusion`]).
//!
//! The arithmetic of a round and of an ending is T-049's (`rules::spells::apply_round_slot`,
//! `apply_monster_round`, `apply_termination`, `apply_monster_termination`); this module
//! owns the order, the messages and what the engine does with the outcomes (the flee move,
//! the knock-out, the chain cast, the recomputed stats).

use silvermere_rules::Rng as _;
use silvermere_rules::abilities::spell_ability_value;
use silvermere_rules::spells::apply::{apply_monster_round, apply_monster_termination};
use silvermere_rules::spells::{SpellEffects, apply_round_slot, apply_termination};
use silvermere_text::Msg;
use silvermere_types::enums::Ability;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ActiveMonsterId, MessageNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::Spell;
use silvermere_types::sizes::{NMONSPELLS, NSPELLCAST};

use super::cast::{CastMode, CastOpts, cast_no_target};
use crate::combat::{self, room_of, user_ability, user_has};
use crate::monsters::move_monster::{move_monster, pick_valid_random_direction};
use crate::movement::exits::{Arg, fmt_msg};
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Who has the lasting spells being ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Player(SessionId),
    Monster(ActiveMonsterId),
}

/// One round of the lasting spells of a player or a monster. For a player, `true` when a
/// spell killed him (the medium pass stops there); a monster's death is its medium pass's
/// own test (T-035).
pub fn tick_active_spells(world: &mut World, who: Who) -> bool {
    match who {
        Who::Player(id) => tick_player_spells(world, id),
        Who::Monster(m) => {
            tick_monster_spells(world, m);
            false
        }
    }
}

/// The spell rounds of `medium_update_character`: for each slot with a spell whose record
/// exists, one round less, the round's effect (also on the last round), and at exactly 0
/// the slot is cleared and the spell ends ([`end_player_spell`] with the chain allowed).
/// `check_kill_user` follows every active slot; a death ends the pass and returns `true`. A
/// slot whose record is missing is left alone.
pub fn tick_player_spells(world: &mut World, id: SessionId) -> bool {
    for i in 0..NSPELLCAST {
        let Some(plr) = combat::player(world, id) else {
            return false;
        };
        let n = plr.spells[i];
        if n == SpellNo(0) {
            continue;
        }
        let Some(spl) = world.spells.get(&n).cloned() else {
            continue;
        };
        let power = i32::from(plr.spellvals[i]);
        // one round less
        let Some(plr) = combat::player_mut(world, id) else {
            return false;
        };
        plr.spellrnds[i] -= 1;
        // the round's effect
        routine_player(world, id, &spl, power);
        let Some(plr) = combat::player_mut(world, id) else {
            return false;
        };
        if plr.spellrnds[i] == 0 {
            // cleared before the ending runs
            plr.spells[i] = SpellNo(0);
            plr.spellvals[i] = 0;
            end_player_spell(world, id, &spl, power, true);
        }
        if combat::check_kill_user(world, id).died() {
            return true;
        }
    }
    false
}

/// One round's effect: T-049's [`apply_round_slot`] one slot at a time, each followed by
/// its prompt (damage ignoring armour, Steal Hit Points, Heal,
/// Heal Mana), the knock-out a damage slot caused, or the move a Flee slot rolled. So each
/// prompt shows the values after its own slot (T-100).
fn routine_player(world: &mut World, id: SessionId, spl: &Spell, power: i32) {
    let effects = SpellEffects::from_record(spl);
    for slot in effects.iter() {
        let out = {
            let World { sessions, rng, .. } = world;
            let Some(plr) = sessions.get_mut(&id).and_then(|s| s.character.as_mut()) else {
                return;
            };
            apply_round_slot(plr, slot, power, rng)
        };
        if out.prompt
            && let Some(s) = world.session_mut(id)
        {
            s.push_prompt();
        }
        // the knock-out
        if out.knocked_out {
            combat::knock_out_user(world, id);
        }
        // the Flee move in a random direction
        if out.flee
            && let Some(plr) = combat::player(world, id)
        {
            let key = room_of(plr);
            if let Some(dir) = pick_valid_random_direction(world, key) {
                crate::movement::move_user::move_user(
                    world,
                    id,
                    dir.code() as i16,
                    crate::movement::move_user::MoveType::Fear.code(),
                );
            }
        }
    }
}

/// A player's spell ends: the ending line (the spell's Descriptive Message record,
/// `msgs[0]` with the spell name, in brown on a cleared line, then the prompt; with no text
/// the colour codes still go out ahead of the next line), T-049's undo
/// ([`apply_termination`]), the "Cast on ending" spell as a chained cast when `chain`, and
/// the stats again.
pub fn end_player_spell(world: &mut World, id: SessionId, spl: &Spell, power: i32, chain: bool) {
    let text = termination_text(world, spl);
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::SpellEnding {
            text: text.clone().unwrap_or_default(),
        });
        if text.is_some() {
            s.push_prompt();
        }
    }
    let end = {
        let World { sessions, rng, .. } = world;
        let Some(plr) = sessions.get_mut(&id).and_then(|s| s.character.as_mut()) else {
            return;
        };
        apply_termination(plr, spl, power, chain, rng)
    };
    if let Some(next) = end.chain {
        cast_no_target(
            world,
            id,
            next,
            CastOpts {
                mode: CastMode::Chain,
                ..CastOpts::default()
            },
        );
    }
    crate::items::equip::recalc(world, id);
}

/// The ending text of a spell: `msgs[0]` of its Descriptive Message record, formatted with
/// the spell name; `None` with no record or an empty text.
pub fn termination_text(world: &World, spl: &Spell) -> Option<String> {
    let n = spell_ability_value(spl, Ability::DescMsg);
    if n == 0 {
        return None;
    }
    let msg = world.messages.get(&MessageNo(n))?;
    if msg.msgs[0].is_empty() {
        return None;
    }
    Some(format!("{}\r", fmt_msg(&msg.msgs[0], &[Arg::S(&spl.name)])))
}

/// The spell rounds of `medium_update_monster`: each slot with a spell loses a round (even
/// when its record is missing), the round's effect runs, and at exactly 0 the slot is
/// cleared (its value kept, sic) and the spell ends. Nothing is printed but a Flee's move.
/// The death test (hit points below 0) is the medium pass's
/// (`monsters::following::medium_update_monster`, which calls this first).
pub fn tick_monster_spells(world: &mut World, mon: ActiveMonsterId) {
    for i in 0..NMONSPELLS {
        let Some(m) = combat::monster(world, mon) else {
            return;
        };
        let n = m.spells[i].spell;
        if n == SpellNo(0) {
            continue;
        }
        let spl = world.spells.get(&n).cloned();
        let Some(m) = world.active_monsters.peek_mut(mon) else {
            return;
        };
        m.spells[i].rounds -= 1;
        let Some(spl) = spl else {
            continue;
        };
        let power = i32::from(m.spells[i].value);
        // the round's effect, one ability at a time: a Flee moves the monster on the spot,
        // before the later abilities and before a spell that ran out is cleared (so
        // `move_monster` still sees this spell's own Hold or Slowness)
        for slot in SpellEffects::from_record(&spl).slots {
            let one = SpellEffects { slots: vec![slot] };
            let flee = {
                let World {
                    active_monsters,
                    rng,
                    ..
                } = world;
                let Some(m) = active_monsters.peek_mut(mon) else {
                    return;
                };
                apply_monster_round(m, &one, power, rng).flee
            };
            if flee
                && let Some(m) = combat::monster(world, mon)
                && let Some(dir) = pick_valid_random_direction(
                    world,
                    RoomKey {
                        map: m.map,
                        room: m.room,
                    },
                )
            {
                move_monster(world, mon, dir, None, false);
            }
        }
        // run out: clear the slot, then end the spell
        let Some(m) = world.active_monsters.peek_mut(mon) else {
            return;
        };
        if m.spells[i].rounds == 0 {
            m.spells[i].spell = SpellNo(0);
            apply_monster_termination(m, &spl, power);
        }
    }
}

/// A confused player (the flag as it stood before the stats are refreshed) loses the action
/// with the chance of his Confusion total; he and the room are told (his Confusion Message
/// record's texts, or the stock ones) and one delay is added. `true` when the action is
/// lost.
pub fn check_confusion(world: &mut World, id: SessionId) -> bool {
    let Some(plr) = combat::player(world, id) else {
        return false;
    };
    if !plr.status.contains(PlayerStatus::CONFUSED) {
        return false;
    }
    if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
        crate::player::secondary::apply_dynamic(world, &mut c);
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
    }
    let Some(plr) = combat::player(world, id) else {
        return false;
    };
    let chance = user_ability(world, plr, Ability::Confusion);
    // the message number is read as the Confusion Message total, from the same sources as
    // the Confusion value.
    let msgnum = if user_has(world, plr, Ability::ConfuseMsg) {
        user_ability(world, plr, Ability::ConfuseMsg)
    } else {
        0
    };
    let who = plr.firstname.clone();
    let room = room_of(plr);
    if !(chance > 0 && world.rng.range(0, 100) < chance) {
        return false;
    }
    let texts = (msgnum != 0)
        .then(|| {
            world
                .messages
                .get(&MessageNo(msgnum))
                .map(|m| m.msgs.clone())
        })
        .flatten();
    let room_msg = match texts {
        Some(m) => {
            tell_user(world, id, Msg::ConfuseMsgLine { text: m[0].clone() });
            Msg::ConfuseMsgLine {
                text: fmt_msg(&m[1], &[Arg::S(&who)]),
            }
        }
        None => {
            tell_user(world, id, Msg::ConfusedYou);
            Msg::ConfusedRoom { who }
        }
    };
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &room_msg,
    );
    crate::commands::pending::add_delay(world, id, 1);
    true
}

/// The monster's Confusion total is its chance to lose the action; the room is told (the
/// Confusion Message record's room text with its name, or the stock line). `true` when it
/// is lost.
pub fn check_monster_confusion(world: &mut World, mon: ActiveMonsterId) -> bool {
    let Some(m) = combat::monster(world, mon) else {
        return false;
    };
    let chance = combat::monster_ability(world, m, Ability::Confusion);
    if !(chance > 0 && world.rng.range(0, 100) < chance) {
        return false;
    }
    let Some(m) = combat::monster(world, mon) else {
        return false;
    };
    let msgnum = combat::monster_ability(world, m, Ability::ConfuseMsg);
    let name = m.name.clone();
    let room = RoomKey {
        map: m.map,
        room: m.room,
    };
    let msg = match (msgnum != 0)
        .then(|| world.messages.get(&MessageNo(msgnum)))
        .flatten()
    {
        Some(r) => Msg::ConfuseMsgLine {
            text: fmt_msg(&r.msgs[1], &[Arg::S(&name)]),
        },
        None => Msg::ConfusedRoom { who: name },
    };
    tell_room(
        world,
        room,
        [None, None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &msg,
    );
    true
}
