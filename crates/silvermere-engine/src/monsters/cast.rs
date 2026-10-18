//! A monster casts a spell (T-052): at a player ([`cast_at_player`]), on itself
//! ([`cast_self`]), by spell number ([`cast_by_number`]) and at random ([`cast_random`]).
//!
//! The arithmetic is T-067's `rules::spells::apply_monster`; the texts of a landed cast are
//! [`super::spell_support`]'s. This module keeps one order, so the lines come as in the
//! original: the cast roll is drawn first (before anything is looked at), the player's resistance,
//! the failure and the resist lines, the energy, then the slots one at a time with the kill test
//! after each damage slot.
//!
//! The teleport room and map, the teleport halves and `added` live for one cast
//! ([`MonsterCastState`]).

use silvermere_rules::Rng as _;
use silvermere_rules::spells::apply_monster::{
    MonsterCast, MonsterCastState, MonsterCaster, apply_monster_cast, monster_duration,
    player_resists_monster_cast, roll_monster_amount,
};
use silvermere_rules::spells::resist::{apply_element_resistance, item_negates};
use silvermere_rules::spells::{
    EffectSlot, Outcome, SpellEffects, TargetKind, UserTarget, add_spell_to_monster, is_attack_type,
};
use silvermere_text::Msg;
use silvermere_types::enums::Ability;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, MonsterNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::{KnownMonster, Spell};

use super::spell_support::monster_display_spell_success;
use crate::combat::{self, user_ability, user_has};
use crate::spells::cast::{report_ended, strip_worn, teleport, user_element_resist};
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// How a monster's cast at a player ended, with the reason kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonsterCastResult {
    /// Nothing was cast (a record is missing, or a lasting spell was refused and
    /// the energy given back). The caller goes on to a normal attack.
    Nothing,
    /// The spell took effect.
    Applied,
    /// The player resisted; half the energy (at least 1) is spent.
    Resisted,
    /// The cast roll failed; half the energy (at least 1) is spent.
    Failed,
    /// The monster had too little energy for the attack and did nothing.
    Tired,
    /// The spell killed the player.
    KilledTarget,
}

impl MonsterCastResult {
    /// The result as a number: 0 nothing cast, 2 the player killed, 1 otherwise.
    pub fn code(self) -> i32 {
        match self {
            Self::Nothing => 0,
            Self::KilledTarget => 2,
            _ => 1,
        }
    }
}

/// The area cast (T-051) as a result code: 0, 1 or 2.
fn monster_cast_area(
    world: &mut World,
    mon: ActiveMonsterId,
    slot: Option<usize>,
    spell: SpellNo,
    level: i32,
) -> i32 {
    super::area_cast::monster_area_cast(world, mon, slot, spell, level).code()
}

fn single_kind(kind: TargetKind) -> bool {
    matches!(
        kind,
        TargetKind::Single
            | TargetKind::SelfOrSingle
            | TargetKind::MonsterOrSingle
            | TargetKind::Any
    )
}

/// The energy a failed or resisted cast costs: half, at least 1, and nothing for a free
/// attack.
fn lose_half_energy(world: &mut World, mon: ActiveMonsterId, cost: i32) {
    if cost != 0
        && let Some(m) = combat::monster_mut(world, mon)
    {
        m.energy = m.energy.wrapping_sub((cost / 2).max(1) as u16);
    }
}

/// A monster's cast at a player.
/// `slot` is the attack slot that casts (its accuracy the spell, its minimum the cast
/// chance, its energy the cost, its maximum the level); `None` (no slot) casts
/// `spellnum` at 100% for no energy at no level (a hit spell, a spell after the round).
pub fn cast_at_player(
    world: &mut World,
    mon: ActiveMonsterId,
    slot: Option<usize>,
    id: SessionId,
    spellnum: i32,
) -> MonsterCastResult {
    use MonsterCastResult as R;
    // the cast roll comes first, whatever follows
    let roll = world.rng.range(0, 100);
    let Some(m) = combat::monster(world, mon).cloned() else {
        return R::Nothing;
    };
    let Some(known) = world.monsters.get(&m.knownnum).cloned() else {
        return R::Nothing;
    };
    let Some(plr) = combat::player(world, id).cloned() else {
        return R::Nothing;
    };
    let room = RoomKey {
        map: m.map,
        room: m.room,
    };
    if world.room(room).is_none() {
        return R::Nothing;
    }
    let attack = slot.and_then(|s| known.attacks.get(s)).copied();
    let number = attack.map_or(spellnum as i16, |a| a.accuracy);
    let Some(spl) = world.spells.get(&SpellNo(number)).cloned() else {
        return R::Nothing;
    };
    let chance = attack.map_or(100, |a| i32::from(a.min));
    // area spells are the room's. (sic) A cast from no slot reads the level from the word
    // in front of the attack maximums, which is the last attack's minimum.
    if !single_kind(spl.target) {
        let level = match attack {
            Some(a) => i32::from(a.max),
            None => known.attacks.last().map_or(0, |a| i32::from(a.min)),
        };
        return match monster_cast_area(world, mon, slot, spl.number, level) {
            0 => R::Nothing,
            2 => R::KilledTarget,
            _ => R::Applied,
        };
    }
    // being attacked wakes the player
    if let Some(p) = combat::player_mut(world, id) {
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
    let cost = attack.map_or(0, |a| i32::from(a.energy));
    // an attack cancels an exit typed
    crate::state::login::leave::stop_users_exit(world, id);
    // Spell Immunity: the level compares as a signed int
    let mut resisted = user_ability(world, &plr, Ability::SpellImmune) > i32::from(spl.level);
    // the same test for every spell type
    let castok = roll < chance;
    // too little energy: nothing happens, and it still counts as handled
    if u32::from(m.energy) < cost as u32 {
        crate::items::equip::recalc(world, id);
        return R::Tired;
    }
    // an item negating the spell resists it; else the magic-resistance roll,
    // made only for a cast that succeeded (T-067 review, silvermere#32)
    let negated = {
        let worn: Vec<_> = plr.worn.iter().filter_map(|n| world.items.get(n)).collect();
        item_negates(worn, spl.number)
    };
    if negated {
        resisted = true;
    } else if castok {
        let anti = user_has(world, &plr, Ability::AntiMagic);
        if player_resists_monster_cast(&spl, i32::from(plr.mr), anti, false, &mut world.rng) {
            resisted = true;
        }
    }
    let firstname = plr.firstname.clone();
    if resisted {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::MonsterCastYouResisted {
                caster: m.name.clone(),
                spell: spl.name.clone(),
            });
            s.push_prompt();
        }
        tell_room(
            world,
            room,
            [None, Some(id)],
            Noise::Medium,
            TellBlind::All,
            true,
            None,
            &Msg::MonsterCastOtherResisted {
                who: firstname,
                caster: m.name.clone(),
                spell: spl.name.clone(),
            },
        );
        lose_half_energy(world, mon, cost);
        crate::items::equip::recalc(world, id);
        return R::Resisted;
    }
    if !castok {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::MonsterCastFailedYou {
                caster: m.name.clone(),
                spell: spl.name.clone(),
            });
            s.push_prompt();
        }
        tell_room(
            world,
            room,
            [None, Some(id)],
            Noise::Medium,
            TellBlind::All,
            true,
            None,
            &Msg::MonsterCastFailedRoom {
                caster: m.name.clone(),
                spell: spl.name.clone(),
                target: firstname,
            },
        );
        lose_half_energy(world, mon, cost);
        crate::items::equip::recalc(world, id);
        return R::Failed;
    }
    // the player's resistance to the element of an attack spell
    let resist = if is_attack_type(spl.spell_type) {
        user_element_resist(world, &plr, &spl)
    } else {
        0
    };
    // the attack's full energy is spent
    if let Some(mm) = combat::monster_mut(world, mon) {
        mm.energy = mm.energy.wrapping_sub(cost as u16);
    }
    // the strength, with the level only for a cast from a slot
    let level = attack.map_or(0, |a| i32::from(a.max));
    let rolled = roll_monster_amount(&spl, level, &mut world.rng);
    let power = apply_element_resistance(rolled, resist);
    let duration = monster_duration(&spl, attack.map(|a| i32::from(a.max)));
    let cast = MonsterCast {
        spell: &spl,
        power,
        share: 0,
        duration,
    };
    let effects = SpellEffects::from_record(&spl);
    let mut state = MonsterCastState::new();
    for s in effects.iter() {
        let was_added = state.added;
        let out = apply_slot(world, mon, &known, id, &cast, s, &mut state);
        let value = s.amount(power);
        match after_slot(world, mon, &known, id, &spl, s, out, value, was_added) {
            Flow::Go => {}
            Flow::Killed => return R::KilledTarget,
        }
        // the add has shown the cast with its prompts; then the hit points rise
        if let Some(v) = state.alter_hp.take() {
            if let Some(p) = combat::player_mut(world, id) {
                silvermere_rules::spells::raise_hp(p, v);
            }
            crate::items::equip::recalc(world, id);
        }
        if state.refused {
            // refused: the energy is given back and the cast ends with 0
            if let Some(mm) = combat::monster_mut(world, mon) {
                mm.energy = mm.energy.wrapping_add(cost as u16);
            }
            return R::Nothing;
        }
    }
    // both halves of a teleport, to a map that is loaded
    if let Some((troom, tmap)) = state.teleport() {
        let dest = RoomKey {
            map: silvermere_types::ids::MapId(tmap as _),
            room: silvermere_types::ids::RoomId(troom as _),
        };
        teleport(world, id, dest, false);
    }
    // the spell may have changed his stats
    crate::items::equip::recalc(world, id);
    R::Applied
}

enum Flow {
    Go,
    Killed,
}

/// One slot on the player (T-067's `apply_monster_cast`) with the character taken out of its
/// session for the call.
fn apply_slot(
    world: &mut World,
    mon: ActiveMonsterId,
    known: &KnownMonster,
    id: SessionId,
    cast: &MonsterCast<'_>,
    slot: &EffectSlot,
    state: &mut MonsterCastState,
) -> Outcome {
    let Some(mut caster_rec) = combat::monster(world, mon).cloned() else {
        return Outcome::NoEffect;
    };
    let Some(p) = combat::player(world, id) else {
        return Outcome::NoEffect;
    };
    let anti_magic = user_has(world, p, Ability::AntiMagic);
    let poison_immune = user_has(world, p, Ability::PoisonImmune);
    let element_resist_pct = user_element_resist(world, p, cast.spell);
    let deathhp = world
        .config
        .death_hp
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let World {
        sessions,
        spells,
        items,
        ..
    } = world;
    let Some(character) = sessions.get_mut(&id).and_then(|s| s.character.as_mut()) else {
        return Outcome::NoEffect;
    };
    let spells_f = |n| spells.get(&n);
    let items_f = |n| items.get(&n);
    let mut user = UserTarget {
        character,
        is_caster: false,
        anti_magic,
        poison_immune,
        element_resist_pct,
        deathhp,
        arena_safe: false,
        spells: &spells_f,
        items: &items_f,
    };
    let mut caster = MonsterCaster {
        monster: &mut caster_rec,
        known,
    };
    let out = apply_monster_cast(slot, cast, &mut caster, &mut user, state);
    // a steal of hit points is the only change to the caster's record
    if let Some(m) = combat::monster_mut(world, mon) {
        m.hp = caster_rec.hp;
    }
    out
}

/// The messages and consequences of one slot's outcome. `value` is the slot's value; `was_added`
/// the state's `added` before the slot.
#[allow(clippy::too_many_arguments)]
fn after_slot(
    world: &mut World,
    mon: ActiveMonsterId,
    known: &KnownMonster,
    id: SessionId,
    spl: &Spell,
    slot: &EffectSlot,
    out: Outcome,
    value: i32,
    was_added: bool,
) -> Flow {
    let name = combat::player(world, id).map_or(String::new(), |p| p.firstname.clone());
    let room = combat::monster(world, mon).map(|m| RoomKey {
        map: m.map,
        room: m.room,
    });
    let hurts = matches!(
        slot.ability,
        Ability::DamageNoArmour | Ability::Slay | Ability::DamageArmour | Ability::StealHp
    );
    match out {
        Outcome::Applied {
            amount,
            knocked_out,
        } => match slot.ability {
            _ if hurts => {
                monster_display_spell_success(world, Some(id), spl, mon, &name, amount);
                return after_damage(world, id, room, knocked_out);
            }
            // these are shown; "Alter Hunger" and "Alter Thirst" are silent
            Ability::Energy | Ability::Heal | Ability::HealMana | Ability::Poison => {
                monster_display_spell_success(world, Some(id), spl, mon, &name, amount);
            }
            _ => {}
        },
        Outcome::StolenHp { amount } => {
            monster_display_spell_success(world, Some(id), spl, mon, &name, amount);
            return after_damage(world, id, room, false);
        }
        // each add that took is shown with the value as its amount
        Outcome::Lasting(so) if so.is_success() => {
            monster_display_spell_success(world, Some(id), spl, mon, &name, value);
            if matches!(
                slot.ability,
                Ability::Intellect
                    | Ability::Wisdom
                    | Ability::Strength
                    | Ability::Health
                    | Ability::Agility
                    | Ability::Charm
            ) {
                crate::items::equip::recalc(world, id);
            }
        }
        Outcome::Ended { spells, poison } => {
            if slot.ability == Ability::CurePoison {
                // the poison spell ends first, then the cast is shown
                report_ended(world, id, &spells, poison);
                monster_display_spell_success(world, Some(id), spl, mon, &name, value);
            } else {
                // shown first, then each spell ends
                monster_display_spell_success(world, Some(id), spl, mon, &name, value);
                report_ended(world, id, &spells, poison);
            }
        }
        Outcome::Summon(n, _) => {
            // shown to the room for the first effect of the cast only
            if !was_added {
                monster_display_spell_success(world, None, spl, mon, "everyone", value);
            }
            if let Some(room) = room {
                summon(world, room, n, i32::from(known.genindex), Some(&name));
            }
        }
        Outcome::TextBlock(n) => {
            crate::scripts::special::run_text_block(
                world,
                id,
                silvermere_types::ids::TextBlockNo(n),
            );
        }
        Outcome::ClearItem(n) => {
            if let Some(room) = room {
                crate::scripts::actions::erase_item_from_room(
                    world,
                    room,
                    silvermere_types::ids::ItemNo(n),
                );
            }
        }
        Outcome::RemoveCurse { worn_slots } => strip_worn(world, id, &worn_slots),
        _ => {}
    }
    Flow::Go
}

/// The kill test of a damage slot (repeated for each damage kind): a death is shared
/// out as a kill by nobody of no monster, worth nothing (sic, `exp` is never set), and ends
/// the cast with 2; a player just dropped is knocked out.
fn after_damage(
    world: &mut World,
    id: SessionId,
    room: Option<RoomKey>,
    knocked_out: bool,
) -> Flow {
    if combat::check_kill_user(world, id).died() {
        if let Some(room) = room {
            combat::kill_monster::distribute_experience(world, None, 0, Some(id), None, room);
        }
        return Flow::Killed;
    }
    if knocked_out {
        combat::knock_out_user(world, id);
    }
    Flow::Go
}

/// The summoned monster is generated forced, owned by nobody, in the level range given. A
/// player named in `follow` is the one the new monster goes after, not as a friend.
fn summon(world: &mut World, room: RoomKey, n: i32, genindex: i32, follow: Option<&str>) {
    let req = super::generate::Request {
        areatype: -1,
        monnum: MonsterNo(n as _),
        minlevel: genindex as i16,
        maxlevel: genindex as i16,
        owner: None,
        ally: false,
        force: true,
    };
    if let Some(new) = super::generate::generate_monster(world, room, req)
        && let Some(who) = follow
        && let Some(m) = combat::monster_mut(world, new)
    {
        m.following = who.to_owned();
        m.friendly = 0;
    }
}

/// A monster's cast on itself: the room is told the monster cast it on "itself"; a lasting
/// spell goes on the monster and each Poison slot adds to its poison; an instant spell
/// summons for each Summon slot.
pub fn cast_self(world: &mut World, mon: ActiveMonsterId, spl: &Spell, level: i32) {
    let value = roll_monster_amount(spl, level, &mut world.rng);
    monster_display_spell_success(world, None, spl, mon, "itself", value);
    let effects = SpellEffects::from_record(spl);
    if spl.duration != 0 {
        if let Some(m) = combat::monster_mut(world, mon) {
            add_spell_to_monster(&mut m.spells, spl.number, i32::from(spl.duration), value);
        }
        for s in effects.iter() {
            if s.ability == Ability::Poison
                && let Some(m) = combat::monster_mut(world, mon)
            {
                m.poison = m.poison.wrapping_add(s.amount(value) as i16);
            }
        }
    } else {
        let Some(m) = combat::monster(world, mon) else {
            return;
        };
        let room = RoomKey {
            map: m.map,
            room: m.room,
        };
        for s in effects.iter() {
            if s.ability == Ability::Summon {
                // any montype, any level, nobody's, forced
                let req = super::generate::Request {
                    areatype: -1,
                    monnum: MonsterNo(s.amount(value) as _),
                    minlevel: 0,
                    maxlevel: 0x7FFF,
                    owner: None,
                    ally: false,
                    force: true,
                };
                super::generate::generate_monster(world, room, req);
            }
        }
    }
}

/// A monster's cast by spell number: the spell's target kind decides: the monster itself,
/// the player it fights (when there is one), or the room. `level` counts as a signed byte, so a
/// level above 127 is negative.
pub fn cast_by_number(
    world: &mut World,
    mon: ActiveMonsterId,
    spellnum: SpellNo,
    level: u8,
    target: Option<SessionId>,
) {
    let Some(spl) = world.spells.get(&spellnum).cloned() else {
        return;
    };
    let level = i32::from(level as i8);
    match spl.target {
        TargetKind::OnSelf | TargetKind::SelfOrSingle | TargetKind::Monster => {
            cast_self(world, mon, &spl, level);
        }
        TargetKind::Single | TargetKind::MonsterOrSingle => {
            if let Some(id) = target {
                cast_at_player(world, mon, None, id, i32::from(spellnum.0));
            }
        }
        TargetKind::DivArea
        | TargetKind::DivAreaIncl
        | TargetKind::Any
        | TargetKind::Item
        | TargetKind::DivAttack
        | TargetKind::DivParty
        | TargetKind::FullArea
        | TargetKind::FullAttack
        | TargetKind::FullParty => {
            monster_cast_area(world, mon, None, spellnum, level);
        }
        // target kinds above TGT_FULLPARTY do nothing
        _ => {}
    }
}

/// A monster's random cast: when the monster has spells, one roll of 0-99 picks the first
/// slot whose cumulative percent is above it, cast at that slot's level; a roll above
/// every slot casts nothing. No roll is drawn for a monster without spells.
pub fn cast_random(world: &mut World, mon: ActiveMonsterId, target: SessionId) {
    let Some(m) = combat::monster(world, mon) else {
        return;
    };
    let Some(known) = world.monsters.get(&m.knownnum) else {
        return;
    };
    if combat::player(world, target).is_none() || known.spells[0].spell == SpellNo(0) {
        return;
    }
    let spells = known.spells;
    let roll = world.rng.range(0, 100);
    if let Some(s) = spells.iter().find(|s| i32::from(s.pct) > roll) {
        cast_by_number(world, mon, s.spell, s.level, Some(target));
    }
}
