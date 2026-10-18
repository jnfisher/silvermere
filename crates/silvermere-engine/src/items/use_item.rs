//! Item use (T-053): an item's abilities applied when it is used by hand (USE,
//! READ, EAT, DRINK, OPEN {item}; the commands are [`crate::commands::use_item`]) or when
//! a weapon with "Auto Use Abilities" lands a blow in combat ([`auto_use_on_hit`], called
//! from both blow paths).
//!
//! The five target kinds are [`use_no_target`],
//! [`use_player_target`], [`use_monster_target`], [`use_item_target`] and
//! [`use_spell_target`]; [`use_item`] picks one by the [`UseTarget`]. Each walks the
//! item's twenty ability slots in record order. A One Time Cast goes through T-050's
//! casting engine with [`CastMode::Item`] (no success roll, the magery test of its own;
//! the energy, mana and level are still needed and spent).
//!
//! The lines a use prints are gathered and sent together: a refused One Time Cast sends
//! none of them. The equip and unequip abilities (Give
//! Spell, Alter HP; [`super::equip::equip_apply_item_abilities`] and
//! [`super::equip::unequip_apply_item_abilities`]) came with wearing and wielding, in
//! [`super::equip`]; the charge count is [`super::inventory::deduct_item_charge`].

use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Direction, ExitType, ItemType, Magery};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::{Exit, Item, Spell};

use crate::combat::{self, Combatant};
use crate::items::equip::add_spell_to_spellbook;
use crate::movement::exits::{LOCK_CLOSED, LOCK_LOCKED, door_word, live_exit, set_par1, set_par2};
use crate::movement::timed_exits::add_exit_to_list;
use crate::scheduler::Rng;
use crate::spells::cast::{CastMode, CastOpts, CastTarget, Caster, apply_engage, cast};
use crate::spells::spellbook::{find_spell_in_spellbook, user_can_use_spell};
use crate::tell::tell_user;
use crate::world::World;

/// What an item is used on (the kinds of target USE finds, and a key's direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseTarget {
    /// Nothing named (`use_no_target`).
    None,
    /// A player in the room (`use_player_target`).
    Player(SessionId),
    /// An active monster in the room (`use_monster_target`).
    Monster(ActiveMonsterId),
    /// An item carried or on the floor (`use_item_target`).
    Item(ItemNo),
    /// A spell of the user's spellbook (`use_spell_target`).
    Spell(SpellNo),
    /// A key's exit (`use_no_target` with the direction).
    Direction(Direction),
}

/// How the `use_*_target` functions were reached: by hand (the command number as `dir`)
/// or by a landed blow (`DIR_AUTOUSE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Hand,
    AutoUse,
}

/// Uses item `item` (carried by `id`) on `target` by hand. The answer is 0 when it was not used,
/// non-zero when the item was really used (the caller then takes a charge), 2 when a cast
/// killed its target.
pub fn use_item(world: &mut World, id: SessionId, item: ItemNo, target: UseTarget) -> i32 {
    let Some(itm) = world.items.get(&item).cloned() else {
        return 0;
    };
    match target {
        UseTarget::None => i32::from(use_no_target(world, id, &itm, None)),
        UseTarget::Direction(dir) => i32::from(use_no_target(world, id, &itm, Some(dir))),
        UseTarget::Player(t) => use_player_target(world, id, &itm, t, Via::Hand),
        UseTarget::Monster(m) => use_monster_target(world, id, &itm, m, Via::Hand),
        UseTarget::Item(t) => use_item_target(world, id, &itm, t),
        UseTarget::Spell(s) => use_spell_target(world, id, &itm, s),
    }
}

/// The landed-blow hook of both blow paths: the wielded weapon `item`, which has "Auto Use
/// Abilities", fires at the victim of a landed blow that did not kill (`use_player_target`
/// / `use_monster_target` with `DIR_AUTOUSE`). Returns the last cast's result (2: the
/// victim player died).
pub fn auto_use_on_hit(world: &mut World, id: SessionId, item: ItemNo, victim: Combatant) -> i32 {
    let Some(itm) = world.items.get(&item).cloned() else {
        return 0;
    };
    match victim {
        Combatant::Player(t) => use_player_target(world, id, &itm, t, Via::AutoUse),
        Combatant::Monster(m) => use_monster_target(world, id, &itm, m, Via::AutoUse),
    }
}

/// [`user_can_use_spell`] for the user; `Err(true)` when the refusal is the level
/// (the original has then printed `This spell is too powerful for you - `).
fn can_cast(world: &World, id: SessionId, spl: &Spell) -> Result<(), bool> {
    let Some(plr) = combat::player(world, id) else {
        return Err(false);
    };
    user_can_use_spell(world, plr, spl)
}

/// A cast from an item (`manual`/`mode` 1). The auto-combat flag is set only on the combat path. The
/// engage a by-hand cast asks for is handed on as the engines do; on the combat path the
/// round's own `engage_autocombat` comes after it and decides, so it is not applied.
fn item_cast(
    world: &mut World,
    id: SessionId,
    spell: SpellNo,
    target: CastTarget,
    via: Via,
) -> i32 {
    let opts = CastOpts {
        mode: CastMode::Item,
        autocombat: via == Via::AutoUse,
        announce: true,
    };
    let r = cast(world, Caster::Player(id), spell, target, opts);
    if via == Via::Hand {
        apply_engage(world, id, &r);
    }
    r.outcome.code()
}

/// Sends the gathered lines to `id`.
fn flush(world: &mut World, id: SessionId, out: Vec<Msg>) {
    for m in out {
        tell_user(world, id, m);
    }
}

/// The item's abilities on the user. "Auto Use Abilities" ends the list (the combat code
/// never comes here). Light and its duration add to the user's; a scroll ("Link to Spell")
/// teaches its spell; a One Time Cast casts with no target and returns that cast's result
/// at once; Heal and Heal Mana restore up to the maximum. Then a key is tried on the exit
/// `dir`. Returns whether something happened.
///
/// `dir` is `None` for every caller but USE {key} {direction}. (sic, not carried) Under the
/// original a key used with no direction acts on an unpredictable exit; here a key with no
/// direction does nothing.
pub fn use_no_target(world: &mut World, id: SessionId, itm: &Item, dir: Option<Direction>) -> bool {
    if combat::player(world, id).is_none() {
        return false;
    }
    let mut out = Vec::new();
    let mut rc = false;
    for &(ability, value) in &itm.abilities {
        match ability {
            Ability::AutoUse => break,
            Ability::GenLight | Ability::GenLightTime => {
                if let Some(p) = combat::player_mut(world, id) {
                    if ability == Ability::GenLight {
                        p.light = p.light.wrapping_add(value);
                    } else {
                        p.lighttime = p.lighttime.wrapping_add(value);
                    }
                }
                rc = true;
            }
            Ability::LinkSpell => rc |= read_scroll(world, id, itm, SpellNo(value), &mut out),
            Ability::OneTimeCast => {
                let Some(spl) = world.spells.get(&SpellNo(value)).cloned() else {
                    continue;
                };
                if spl.magery == Magery::None || can_cast(world, id, &spl).is_ok() {
                    flush(world, id, out);
                    return item_cast(world, id, spl.number, CastTarget::None, Via::Hand) != 0;
                }
                // The lines so far are thrown away
                return false;
            }
            Ability::Heal => {
                if let Some(p) = combat::player_mut(world, id) {
                    p.hp = p.hp.wrapping_add(value).min(p.maxhp);
                }
                out.push(Msg::ReceiveHp { n: value.into() });
                rc = true;
            }
            Ability::HealMana => {
                if let Some(p) = combat::player_mut(world, id) {
                    p.mana = p.mana.wrapping_add(value).max(0);
                    if i32::from(p.mana) > i32::from(p.maxmana) {
                        p.mana = p.maxmana as i16;
                    }
                }
                out.push(Msg::ReceiveMana { n: value.into() });
                rc = true;
            }
            _ => {}
        }
    }
    if itm.item_type == ItemType::Key
        && let Some(dir) = dir
    {
        rc |= use_key(world, id, itm, dir, &mut out);
    }
    flush(world, id, out);
    rc
}

/// A scroll's "Link to Spell" slot: exactly one of `You don't know what
/// to do with this!` (not allowed, or the spellbook would not take it), `You realize that
/// you already know this scroll!` (the spell's name finds that spell in the spellbook),
/// `You read ... and learn the spell ...`. Each follows an ANSI_CLRLINE.
fn read_scroll(
    world: &mut World,
    id: SessionId,
    itm: &Item,
    spell: SpellNo,
    out: &mut Vec<Msg>,
) -> bool {
    let Some(spl) = world.spells.get(&spell).cloned() else {
        return false;
    };
    out.push(Msg::Clrline);
    if let Err(too_powerful) = can_cast(world, id, &spl) {
        if too_powerful {
            out.push(Msg::CastTooPowerfulPrefix);
        }
        out.push(Msg::UseUnknown);
        return false;
    }
    // The spell's name looked up in the spellbook
    let Some(mut plr) = combat::player(world, id).cloned() else {
        return false;
    };
    let found = find_spell_in_spellbook(world, &mut plr, &spl.name, true);
    let Some(p) = combat::player_mut(world, id) else {
        return false;
    };
    // the search clears the slots of spells that no longer exist
    p.spellbook = plr.spellbook;
    // The output buffer is emptied: what is queued goes, the ANSI_CLRLINE above and the
    // dispatcher's ANSI_NORMAL with it
    out.clear();
    crate::output::clear_output(world, id);
    let Some(p) = combat::player_mut(world, id) else {
        return false;
    };
    if found.spell == Some(spl.number) {
        out.push(Msg::AlreadyKnownScroll);
        return false;
    }
    if add_spell_to_spellbook(p, spl.number) {
        out.push(Msg::LearnScroll {
            item: itm.name.clone(),
            spell: spl.name.clone(),
        });
        return true;
    }
    out.push(Msg::UseUnknown);
    false
}

/// A key on the exit `dir` of the user's room. A Key exit holds its
/// key in `exitpar1s` and its state in `exitpar2s`; a Door or Gate its key in `exitpar4s`
/// and its state in `exitpar1s`. A locked exit that the key fits is unlocked on both sides
/// and queued to lock again after `exitpar4s` (Key) or `exitpar3s` (Door, Gate) steps, at
/// least 1 (the low word only). A key that fits counts as used whether or not the exit was
/// locked; one that does not fit prints its line and does not.
fn use_key(
    world: &mut World,
    id: SessionId,
    itm: &Item,
    dir: Direction,
    out: &mut Vec<Msg>,
) -> bool {
    let Some(room) = combat::player(world, id).map(combat::room_of) else {
        return false;
    };
    let Some(exit) = live_exit(world, room, dir).filter(|e| e.target.0 != 0) else {
        return false;
    };
    let key = itm.number.0;
    let fits_key_exit = exit.kind == ExitType::Key && exit.par1 as i32 == key;
    let fits_door = matches!(exit.kind, ExitType::Door | ExitType::Gate) && exit.par4 == key;
    if !fits_key_exit && !fits_door {
        out.push(Msg::KeyNoFit {
            item: itm.name.clone(),
        });
        return false;
    }
    let what = if fits_key_exit {
        "door"
    } else {
        door_word(exit.kind)
    };
    let state = if fits_key_exit {
        exit.par2 as u32
    } else {
        exit.par1
    };
    if state != LOCK_LOCKED {
        out.push(Msg::KeyNotLocked { what: what.into() });
        return true;
    }
    unlock(world, room, dir, &exit);
    // the same exit seen from the other room, if it leads straight back here and is of
    // the same family (Key for Key; Door or Gate for Door or Gate)
    if let Some(rev) = dir.reverse() {
        let orm = RoomKey {
            map: room.map,
            room: exit.target,
        };
        if let Some(back) = live_exit(world, orm, rev)
            && back.target == room.room
            && (back.kind == ExitType::Key) == fits_key_exit
            && matches!(back.kind, ExitType::Key | ExitType::Door | ExitType::Gate)
        {
            unlock(world, orm, rev, &back);
        }
    }
    out.push(Msg::Unlocked { what: what.into() });
    true
}

/// One side of an exit unlocked and queued to lock again (`add_exit_to_list`).
fn unlock(world: &mut World, room: RoomKey, dir: Direction, exit: &Exit) {
    let steps = if exit.kind == ExitType::Key {
        set_par2(world, room, dir, LOCK_CLOSED as i16);
        exit.par4
    } else {
        set_par1(world, room, dir, LOCK_CLOSED);
        exit.par3
    };
    add_exit_to_list(world, room, dir, i32::from(steps as i16).max(1));
}

/// The item's abilities on another player (by hand), or an Auto Use weapon's on the player
/// it hit (`via` [`Via::AutoUse`]). An "Auto Use Abilities" slot rolls its percent (0
/// always): by hand it ends the list; in combat a later slot works only after a successful
/// roll. Light, hit points and mana go to the target (who is told of the last two, with a
/// prompt); a One Time Cast is cast at the target
/// ([`cast_user_target`](crate::spells::cast::cast_user_target) as an item cast) when its
/// spell has no magery or the user may cast it. Returns 1 when an effect was applied, or
/// the last cast's result.
pub fn use_player_target(
    world: &mut World,
    id: SessionId,
    itm: &Item,
    target: SessionId,
    via: Via,
) -> i32 {
    if combat::player(world, target).is_none() {
        return 0;
    }
    if combat::debug_on(world, id) {
        tracing::debug!(item = %itm.name, "DBG: Using");
    }
    let auto = via == Via::AutoUse;
    let mut rc = 0;
    let mut trig = false;
    for &(ability, value) in &itm.abilities {
        if ability == Ability::AutoUse {
            if value == 0 || world.rng.range(0, 100) < i32::from(value) {
                trig = true;
            }
            if !auto {
                break;
            }
            continue;
        }
        if auto && !trig {
            continue;
        }
        match ability {
            Ability::GenLight | Ability::GenLightTime => {
                if let Some(t) = combat::player_mut(world, target) {
                    if ability == Ability::GenLight {
                        t.light = t.light.wrapping_add(value);
                    } else {
                        t.lighttime = t.lighttime.wrapping_add(value);
                    }
                }
                rc = 1;
            }
            Ability::OneTimeCast => {
                let Some(spl) = world.spells.get(&SpellNo(value)).cloned() else {
                    continue;
                };
                if spl.magery == Magery::None || can_cast(world, id, &spl).is_ok() {
                    if combat::debug_on(world, id) {
                        tracing::debug!(spell = %spl.name, "DBG: About to cast");
                    }
                    rc = item_cast(world, id, spl.number, CastTarget::User(target), via);
                }
            }
            Ability::Heal | Ability::HealMana => {
                let msg = if let Some(t) = combat::player_mut(world, target) {
                    if ability == Ability::Heal {
                        t.hp = t.hp.wrapping_add(value).min(t.maxhp);
                        Msg::ReceiveHp { n: value.into() }
                    } else {
                        t.mana = t.mana.wrapping_add(value).max(0);
                        if i32::from(t.mana) > i32::from(t.maxmana) {
                            t.mana = t.maxmana as i16;
                        }
                        Msg::ReceiveMana { n: value.into() }
                    }
                } else {
                    continue;
                };
                tell_user(world, target, msg);
                if let Some(s) = world.session_mut(target) {
                    s.push_prompt();
                }
                rc = 1;
            }
            _ => {}
        }
    }
    rc
}

/// Only a One Time Cast does anything to a monster, cast with
/// [`cast_monster_target`](crate::spells::cast::cast_monster_target) as an item cast when
/// the user may cast the spell (no exemption for a spell of no magery here). Auto Use works
/// as in [`use_player_target`]. Returns the last cast's result, or 0.
///
/// (sic) A user too low for the spell is left with `This spell is too powerful for you - `
/// in the output (the rest of the line never comes).
pub fn use_monster_target(
    world: &mut World,
    id: SessionId,
    itm: &Item,
    mon: ActiveMonsterId,
    via: Via,
) -> i32 {
    if combat::monster(world, mon).is_none() {
        return 0;
    }
    let auto = via == Via::AutoUse;
    let debug = combat::debug_on(world, id);
    let mut rc = 0;
    let mut trig = false;
    for &(ability, value) in &itm.abilities {
        if ability == Ability::None {
            continue;
        }
        if debug {
            tracing::debug!(ability = ?ability, trig, "DBG: Ability");
        }
        if ability == Ability::AutoUse {
            if value == 0 || world.rng.range(0, 100) < i32::from(value) {
                trig = true;
            }
            if !auto {
                break;
            }
            continue;
        }
        if auto && !trig {
            continue;
        }
        if ability != Ability::OneTimeCast {
            if debug {
                tracing::debug!("USE MONSTER TARGET DEFAULT.");
            }
            continue;
        }
        let Some(spl) = world.spells.get(&SpellNo(value)).cloned() else {
            continue;
        };
        match can_cast(world, id, &spl) {
            Ok(()) => {
                if debug {
                    tracing::debug!(spell = spl.number.0, "DEBUG : CASTING");
                }
                rc = item_cast(world, id, spl.number, CastTarget::Monster(mon), via);
                if rc == 0 && debug {
                    tracing::debug!("DEBUG : SPELL FAILED");
                }
            }
            Err(too_powerful) => {
                if too_powerful {
                    tell_user(world, id, Msg::CastTooPowerfulPrefix);
                }
                if debug {
                    tracing::debug!(spell = spl.number.0, "You cannot use spell");
                }
            }
        }
    }
    rc
}

/// Every One Time Cast slot is cast at the target item
/// ([`cast_item_target`](crate::spells::cast::cast_item_target) as an item cast) when the
/// user may cast it. Auto Use plays no part. Returns the last cast's result, or 0.
pub fn use_item_target(world: &mut World, id: SessionId, itm: &Item, target: ItemNo) -> i32 {
    if !world.items.contains_key(&target) {
        return 0;
    }
    let mut rc = 0;
    for &(ability, value) in &itm.abilities {
        if ability != Ability::OneTimeCast {
            continue;
        }
        let Some(spl) = world.spells.get(&SpellNo(value)).cloned() else {
            continue;
        };
        if can_cast(world, id, &spl).is_ok() {
            rc = item_cast(world, id, spl.number, CastTarget::Item(target), Via::Hand);
        }
    }
    rc
}

/// Does nothing (using an item on a spell has no effect in the game); always 0.
pub fn use_spell_target(_world: &mut World, _id: SessionId, _itm: &Item, _spell: SpellNo) -> i32 {
    0
}

#[cfg(test)]
mod tests;
