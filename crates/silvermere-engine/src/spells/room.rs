//! Room spells: a room casts a spell on a player ([`room_cast_on_user`]) or on a monster
//! ([`room_cast_on_monster`]), with "The room" as the caster.
//!
//! A room has no caster record: no energy, no mana, no cast roll, no level. The player's
//! resistance roll (half his magic resistance held to 97) and a worn item that negates the
//! spell still stop it; the strength is rolled between the record's minimum and maximum and
//! cut by the player's element resistance. Each slot then goes through T-049's
//! `apply_effect` on [`CastPath::Room`], and the player is told what each slot did by
//! [`tell_room_cast_message`] (the target line of the spell's WCCMSG triple, coloured by
//! spell type; nobody else hears anything). A teleport waits until after the last slot
//! ([`PendingTeleport`]).
//!
//! The callers (all wired through hooks that were waiting for this card):
//!
//! - `medium_update_character` every other medium pass for the room's `roomspell`
//!   ([`crate::cycles::CycleHooks::room_spell`]);
//! - `set_user_location` on entering a room flagged for an entry spell whose `roomspell`
//!   differs from the room left; there is no other entry cast (the medium pass is the only
//!   unconditional one);
//! - a Spell Trap exit (`exitpar1s`) and a Cast exit (`exitpar1s` on the way through,
//!   `exitpar2s` after any attempt);
//! - a failed pick of a trapped lock (`picktrap`) and a failed disarm (`exitpar1s`; T-031's
//!   command).
//!
//! These reach [`room_cast_on_user`] through [`RoomCasts`], the movement hooks' spell
//! implementation. Nothing in the module calls `room_cast_on_monster` (a monster crossing a
//! Spell Trap takes plain damage); it is kept for completeness.

use silvermere_rules::Rng as _;
use silvermere_rules::spells::apply::{Cast, CastPath, CastState, monster_gate};
use silvermere_rules::spells::apply_monster::{
    RoomMonsterState, apply_room_on_monster_spell, player_resists_room_cast, room_charm_resistance,
    room_monster_power,
};
use silvermere_rules::spells::resist::{item_negates, may_resist, resist_chance};
use silvermere_rules::spells::{
    CharacterSlots, EffectSlot, MAXCHANCE, MonsterTarget, Outcome, SpellEffect, SpellEffects,
    Target, UserTarget, add_spell_to_character, apply_effect, is_attack_type,
};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, DebugLevel};
use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::{
    ActiveMonsterId, ItemNo, MessageNo, MonsterNo, RoomKey, SessionId, SpellNo,
};
use silvermere_types::records::Spell;
use silvermere_types::sizes::{NHIDITEMS, NROOMITEMS};

use super::cast::{report_ended, strip_worn, user_element_resist};
use super::messages::coloured_line;
use crate::combat::{self, monster_ability, monster_has, room_of, user_has};
use crate::items::limited::{self, Holder};
use crate::items::room_items::mark_changed;
use crate::movement::exits::{Arg, fmt_msg};
use crate::world::World;

/// The destination a "Teleport Room" and a "Teleport Map" slot noted, for after the last
/// slot. A local of each cast here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingTeleport {
    pub room: i32,
    pub map: i32,
}

impl PendingTeleport {
    /// Both halves, when both were set and neither is 0.
    pub fn of(state: &CastState) -> Option<Self> {
        let (room, map) = state.teleport()?;
        (room != 0 && map != 0).then_some(Self { room, map })
    }

    fn key(self) -> RoomKey {
        RoomKey::new(self.map as u32, self.room as u32)
    }
}

/// The movement hooks' spell side (`movement::HOOKS.spells`) and the medium pass's room
/// spell: every `room_cast_on_user` call of the module.
pub struct RoomCasts;

impl crate::movement::SpellHooks for RoomCasts {
    fn room_cast_on_user(&self, world: &mut World, id: SessionId, spell: i32) {
        cast_spell_number(world, id, spell);
    }
}

/// [`room_cast_on_user`] with the record's spell field, of whatever width.
pub fn cast_spell_number(world: &mut World, id: SessionId, spell: i32) {
    if let Ok(n) = i16::try_from(spell) {
        room_cast_on_user(world, id, SpellNo(n));
    }
}

fn spell_of(world: &World, n: SpellNo) -> Option<Spell> {
    if n == SpellNo(0) {
        return None;
    }
    world.spells.get(&n).cloned()
}

fn texts_of(world: &World, n: i32) -> Option<[String; 3]> {
    if n == 0 {
        return None;
    }
    world.messages.get(&MessageNo(n)).map(|m| m.msgs.clone())
}

/// The fallback target line when the spell has no WCCMSG record. The `\r` is the
/// original's: the line gets a second one (sic, harmless).
pub const DEFAULT_ROOM_TEXT: &str = "%s cast %s on you for %d damage.\r";

/// The text of [`tell_room_cast_message`]: the spell's WCCMSG target line (or
/// [`DEFAULT_ROOM_TEXT`]) with the amount only for an alternate-message spell, else with
/// `"The room"`, the spell's name and the amount; then the third line of the "Descriptive
/// Message" record, if any. `None` when there is nothing to show but the clear-line and
/// colour codes.
pub fn room_cast_text(world: &World, spl: &Spell, amount: i32) -> String {
    let fmt = texts_of(world, spl.msgnum.0)
        .map(|m| m[1].clone())
        .unwrap_or_else(|| DEFAULT_ROOM_TEXT.to_owned());
    let desc = texts_of(
        world,
        silvermere_rules::abilities::spell_ability_value(spl, Ability::DescMsg),
    );
    let mut text = String::new();
    if !fmt.is_empty() {
        let line = format!("{fmt}\r");
        let amount = Arg::D(i64::from(amount));
        if spl.flags.contains(SpellFlags::ALTMSG) {
            text.push_str(&fmt_msg(&line, &[amount]));
        } else {
            text.push_str(&fmt_msg(
                &line,
                &[Arg::S("The room"), Arg::S(&spl.name), amount],
            ));
        }
    }
    if let Some(d) = desc
        && !d[2].is_empty()
    {
        text.push_str(&d[2]);
        text.push('\r');
    }
    text
}

/// The line to the player in the spell type's colour (`COL_SPELLGEN` for General,
/// `COL_SPELLOTH` else), and his prompt.
pub fn tell_room_cast_message(world: &mut World, id: SessionId, spl: &Spell, amount: i32) {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return;
    }
    let text = room_cast_text(world, spl, amount);
    if let Some(s) = world.session_mut(id) {
        s.output.push(coloured_line(spl, text));
        s.push_prompt();
    }
}

/// A room casts the spell on the player.
pub fn room_cast_on_user(world: &mut World, id: SessionId, spell: SpellNo) {
    let Some(p) = combat::player(world, id) else {
        return;
    };
    let Some(spl) = spell_of(world, spell) else {
        return;
    };
    // the resistance roll (drawn first), then a worn item
    let anti_magic = user_has(world, p, Ability::AntiMagic);
    let negated = item_negates(p.worn.iter().filter_map(|n| world.items.get(n)), spl.number);
    let mr = i32::from(p.mr);
    let modifier = user_element_resist(world, p, &spl);
    if player_resists_room_cast(&spl, mr, anti_magic, negated, &mut world.rng) {
        return;
    }
    // the strength, less the element resistance
    let maxval = i32::from(spl.maxval);
    let minval = i32::from(spl.minval).min(maxval);
    let roll = world.rng.range(0, maxval - minval + 1) + minval;
    let roll = (i64::from(roll) * (100 - i64::from(modifier)) / 100) as i32;
    // the spell-added flag and the teleport destination start clear
    let mut state = CastState::new();
    let effects = SpellEffects::from_record(&spl);
    for slot in effects.iter() {
        let amount = slot.amount(roll);
        // SYSOP DEBUG: the ability's name and amount (numbers up to 190, sic)
        if world
            .session(id)
            .is_some_and(|s| s.flags.debug == DebugLevel::On)
            && slot.ability.code() <= 190
        {
            let ability = slot.ability.name().unwrap_or_default().to_owned();
            crate::tell::tell_user(
                world,
                id,
                Msg::RoomCastDebug {
                    ability,
                    amount: i64::from(amount),
                },
            );
        }
        room_slot(world, id, &spl, slot, roll, &mut state);
    }
    // the teleport, last; the map must be there (loaded)
    if let Some(t) = PendingTeleport::of(&state)
        && world.room(t.key()).is_some()
    {
        teleport(world, id, t.key());
    }
}

/// He lets go of whoever he drags, every follower of his stops following him, and he is
/// moved there with the move recorded (`party::stop_following_all`, which also clears his
/// own list).
fn teleport(world: &mut World, id: SessionId, dest: RoomKey) {
    crate::movement::move_user::stop_dragging(world, id);
    crate::party::stop_following_all(world, id);
    crate::movement::move_user::set_user_location(world, id, dest, false);
}

/// One slot of a room spell on a player: the effect through T-049's room path, what it asks
/// of the world, and the message rule. The spell-added flag is `state.told`: a lasting slot
/// adds the spell and tells the player once per cast; an instant effect tells him unless
/// the spell was added earlier in the cast; the lasting stat alters add the spell (once)
/// and say nothing.
fn room_slot(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    roll: i32,
    state: &mut CastState,
) {
    let added_before = state.told;
    let out = if room_default(slot.effect) {
        default_lasting(world, id, spl, slot.amount(roll), state)
    } else {
        apply_to_user(world, id, spl, slot, roll, state)
    };
    let amount = slot.amount(roll);
    let key = combat::player(world, id).map(room_of);
    let mut changed = false;
    let mut shown = amount;
    match &out {
        // the room-wide slots
        Outcome::TextBlock(n) => {
            crate::scripts::special::run_text_block(
                world,
                id,
                silvermere_types::ids::TextBlockNo(*n),
            );
        }
        Outcome::Scatter(n) => {
            if let Some(key) = key {
                move_room_contents(
                    world,
                    key,
                    RoomKey {
                        map: key.map,
                        room: silvermere_types::ids::RoomId(*n as u32),
                    },
                );
            }
        }
        Outcome::ClearItem(n) => {
            if let Some(key) = key {
                crate::scripts::actions::erase_item_from_room(world, key, ItemNo(*n));
            }
        }
        // the weapon breaks: "Your %s has been shattered!" and no cast message
        Outcome::Shattered { item, .. } => shattered(world, id, *item),
        Outcome::RemoveCurse { worn_slots } => {
            changed = true;
            strip_worn(world, id, worn_slots);
        }
        // Freedom, Dispel magic; Cure Poison
        Outcome::Ended { spells, poison } => {
            changed = true;
            report_ended(world, id, spells, *poison);
        }
        Outcome::Summon(n, _) => {
            changed = true;
            summon(world, id, *n);
        }
        // Hunger and Thirst say nothing
        Outcome::Applied {
            amount: a,
            knocked_out,
        } => {
            changed = !matches!(slot.effect, SpellEffect::Hunger | SpellEffect::Thirst);
            shown = *a;
            if *knocked_out {
                combat::knock_out_user(world, id);
            }
        }
        _ => {}
    }
    // the lasting add tells him, once (the stat alters' own add says nothing)
    let lasting = matches!(out, Outcome::Lasting(_))
        && !matches!(
            slot.effect,
            SpellEffect::StatAlter(_) | SpellEffect::AlterHp
        );
    if lasting && !added_before {
        tell_room_cast_message(world, id, spl, amount);
    }
    if changed && !state.told {
        tell_room_cast_message(world, id, spl, shown);
    }
    // the hit points rise after the add
    if let Some(v) = state.alter_hp.take()
        && let Some(p) = combat::player_mut(world, id)
    {
        silvermere_rules::spells::raise_hp(p, v);
    }
    if matches!(
        slot.effect,
        SpellEffect::StatAlter(_) | SpellEffect::AlterHp | SpellEffect::Shatter
    ) {
        crate::items::equip::recalc(world, id);
    }
}

/// The abilities T-049 handles before its target arms (a chain, a component, a guard, an
/// unknown number) that have no room effect of their own: as any unlisted ability
/// they make the spell a lasting one when it has a duration.
fn room_default(effect: SpellEffect) -> bool {
    matches!(
        effect,
        SpellEffect::EndCast
            | SpellEffect::EndCastChance
            | SpellEffect::Component
            | SpellEffect::Guarded
            | SpellEffect::Unknown(_)
    )
}

/// The lasting add of [`room_default`]: once per cast, the record's duration as it is,
/// renewing.
fn default_lasting(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    amount: i32,
    state: &mut CastState,
) -> Outcome {
    if spl.duration == 0 || state.told {
        return Outcome::NoEffect;
    }
    let Some(c) = combat::player_mut(world, id) else {
        return Outcome::NoEffect;
    };
    let out = add_spell_to_character(
        CharacterSlots {
            spells: &mut c.spells,
            spellvals: &mut c.spellvals,
            spellrnds: &mut c.spellrnds,
        },
        spl.number,
        i32::from(spl.duration),
        amount,
        true,
    );
    state.told = true;
    Outcome::Lasting(out)
}

/// Applies one slot to the player with T-049's `apply_effect` on the room path.
fn apply_to_user(
    world: &mut World,
    id: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    roll: i32,
    state: &mut CastState,
) -> Outcome {
    let Some(t) = combat::player(world, id) else {
        return Outcome::NoEffect;
    };
    let anti_magic = user_has(world, t, Ability::AntiMagic);
    let poison_immune = user_has(world, t, Ability::PoisonImmune);
    let element_resist_pct = user_element_resist(world, t, spl);
    let deathhp = world
        .config
        .death_hp
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let cast = Cast {
        spell: spl,
        caster: None,
        path: CastPath::Room,
        power: roll,
    };
    let World {
        sessions,
        spells,
        items,
        rng,
        ..
    } = world;
    let Some(character) = sessions.get_mut(&id).and_then(|s| s.character.as_mut()) else {
        return Outcome::NoEffect;
    };
    let spells_f = |n| spells.get(&n);
    let items_f = |n| items.get(&n);
    let mut target = Target::User(UserTarget {
        character,
        is_caster: false,
        anti_magic,
        poison_immune,
        element_resist_pct,
        deathhp,
        arena_safe: false,
        spells: &spells_f,
        items: &items_f,
    });
    apply_effect(slot, &cast, &mut target, state, rng)
}

/// "Shatter" by a room: the weapon's abilities come off, it leaves the inventory and, when
/// it was there, `Your %s has been shattered!` with his prompt; then his stats again (the
/// caller's recalc).
fn shattered(world: &mut World, id: SessionId, item: ItemNo) {
    let Some(itm) = world.items.get(&item).cloned() else {
        return;
    };
    crate::items::equip::unequip_apply_item_abilities(world, id, &itm);
    if crate::items::inventory::remove_item_from_inventory(world, id, item).is_some()
        && let Some(s) = world.session_mut(id)
    {
        s.output.push(Msg::CastYourShattered { item: itm.name });
        s.push_prompt();
    }
}

/// "Summon Creature" by a room: known monster `amount` in the room, level range 0 to 65000,
/// forced, and the new monster follows the player by name, no friend. (The level range is
/// held to the field's 0x7FFF; a named monster ignores it.)
fn summon(world: &mut World, id: SessionId, n: i32) {
    let Some(p) = combat::player(world, id) else {
        return;
    };
    let (key, name) = (room_of(p), p.firstname.clone());
    let req = crate::monsters::generate::Request {
        areatype: -1,
        monnum: MonsterNo(n),
        minlevel: 0,
        maxlevel: 0x7FFF,
        owner: None,
        ally: false,
        force: true,
    };
    if let Some(mon) = crate::monsters::generate::generate_monster(world, key, req)
        && let Some(m) = combat::monster_mut(world, mon)
    {
        m.following = name;
        m.friendly = 0;
    }
}

/// Every coin on the floor of `from`, visible and hidden, goes to `to` (same map), then
/// every item that can be picked up, visible and hidden, with all its copies: each copy
/// leaves the limited records of `from` and is put down in `to` (`dispose_of_item_in_room`,
/// which may carry it on through an exit). Items that cannot be taken stay. `false` when
/// either room does not exist.
pub fn move_room_contents(world: &mut World, from: RoomKey, to: RoomKey) -> bool {
    if world.room(from).is_none()
        || world.room(to).is_none()
        || !world.room_state.contains_key(&from)
        || !world.room_state.contains_key(&to)
    {
        return false;
    }
    let (coins, hidcoins) = {
        let fs = world.room_state.get_mut(&from).expect("checked");
        (
            std::mem::take(&mut fs.coins),
            std::mem::take(&mut fs.hidcoins),
        )
    };
    if let Some(ts) = world.room_state.get_mut(&to) {
        add_coins(&mut ts.coins, coins);
        add_coins(&mut ts.hidcoins, hidcoins);
    }
    for hidden in [false, true] {
        let n = if hidden { NHIDITEMS } else { NROOMITEMS };
        for i in 0..n {
            let Some(slot) = world
                .room_state
                .get(&from)
                .map(|s| if hidden { s.hiditems[i] } else { s.items[i] })
            else {
                continue;
            };
            if slot.item.0 == 0 {
                continue;
            }
            let gettable = world
                .items
                .get(&slot.item)
                .is_some_and(|itm| itm.gettable != 0);
            if !gettable {
                continue;
            }
            for _ in 0..i32::from(slot.count) + 1 {
                limited::release(world, slot.item, Holder::Room(from));
                crate::items::room_items::dispose_of_item_in_room(world, to, slot.item, slot.uses);
            }
            if let Some(s) = world.room_state.get_mut(&from) {
                let cell = if hidden {
                    &mut s.hiditems[i]
                } else {
                    &mut s.items[i]
                };
                cell.item = ItemNo(0);
                cell.uses = -1;
                cell.count = 0;
            }
            mark_changed(world, from);
        }
    }
    true
}

fn add_coins(to: &mut silvermere_types::records::Coins, from: silvermere_types::records::Coins) {
    to.copper = to.copper.wrapping_add(from.copper);
    to.silver = to.silver.wrapping_add(from.silver);
    to.gold = to.gold.wrapping_add(from.gold);
    to.platinum = to.platinum.wrapping_add(from.platinum);
    to.runic = to.runic.wrapping_add(from.runic);
}

/// A room casts the spell on a monster: the filters (Kill Dead needs the undead, Affects
/// Animal an animal, Affects Living the living), Spell Immunity above the spell's level,
/// the element resistance of an attack spell, a magic resistance roll against the charm
/// resistance of an "Enslave Creature" spell only (0 otherwise, so never resisted), the
/// strength held to `maxval`, then the slots through T-067's `apply_room_on_monster_spell`
/// until a damage slot kills it.
///
/// Each cast starts clear: after the first effect no later slot adds another lasting
/// spell.
pub fn room_cast_on_monster(world: &mut World, mon: ActiveMonsterId, spell: SpellNo) {
    let Some(spl) = spell_of(world, spell) else {
        return;
    };
    let Some(m) = combat::monster(world, mon).cloned() else {
        return;
    };
    let Some(known) = world.monsters.get(&m.knownnum).cloned() else {
        return;
    };
    let effects = SpellEffects::from_record(&spl);
    let undead = known.undead != 0;
    let animal = monster_has(world, &m, Ability::Animal);
    let non_living = monster_has(world, &m, Ability::NonLiving);
    if monster_gate(&effects, undead, animal, non_living).is_some() {
        return;
    }
    // (unsigned compare)
    let immunity = monster_ability(world, &m, Ability::SpellImmune);
    if u32::from(spl.level) < immunity as u32 {
        return;
    }
    // an attack spell's element; a General spell reads `resist` uninitialised (sic), taken
    // as 0 here
    let resist = if is_attack_type(spl.spell_type) {
        super::cast::monster_element_resist(world, &m, &spl)
    } else {
        0
    };
    let charmres = room_charm_resistance(&spl, &known);
    let anti_magic = monster_has(world, &m, Ability::AntiMagic);
    if may_resist(&spl, anti_magic) && world.rng.range(1, 100) <= resist_chance(charmres, MAXCHANCE)
    {
        return;
    }
    let maxval = i32::from(spl.maxval);
    let minval = i32::from(spl.minval).min(maxval);
    let roll = world.rng.range(0, maxval - minval + 1) + minval;
    let power = room_monster_power(&spl, roll, resist);
    let poison_immune = monster_has(world, &m, Ability::PoisonImmune);
    let mut state = RoomMonsterState::default();
    // the monster into the cache (T-086)
    world.active_monsters.touch(mon);
    {
        let World {
            active_monsters, ..
        } = world;
        let Some(monster) = active_monsters.values_mut().find(|a| a.number == mon) else {
            return;
        };
        let mut target = MonsterTarget {
            monster,
            known: &known,
            magic_resistance: charmres,
            anti_magic,
            poison_immune,
            element_resist_pct: resist,
        };
        apply_room_on_monster_spell(&spl, power, &mut target, &mut state);
    }
    // check_kill_monster(mon, 0) (sic: channel 0 is named as the killer; -1 was probably
    // meant and is used here) and everyone fighting it stops
    if combat::monster(world, mon).is_some_and(|m| m.hp <= 0)
        && combat::check_kill_monster(world, mon, None)
    {
        combat::kill_autocombat_against_monster(world, mon);
    }
}

#[cfg(test)]
mod tests;
