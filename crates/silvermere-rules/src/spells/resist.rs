//! Resistance: item negation, the magic-resistance rolls, Spell Immunity, element
//! resistance and the magic-resistance damage multiplier.
//!
//! The rules for a worn item negating a spell, the player roll of `count_valid_targets` and
//! `cast_user_target`, the monster rolls of `cast_no_target` (never resisting) and
//! `cast_monster_target`, the element resistance, the Spell Immunity gate and the
//! "Inflict Damage on Armour" multiplier.

use silvermere_types::enums::{Ability, Element, ResistMode, SpellType};
use silvermere_types::ids::SpellNo;
use silvermere_types::records::{Item, Spell};

use super::{MAXCHANCE, MR_NEUTRAL, MRCUT_ANTIMAG, MRCUT_MAX, is_attack_type};
use crate::abilities::AbilityLookup;
use crate::rng::Rng;

/// Does something the target wears negate the spell? Every worn item carries up to ten
/// spell numbers it negates (`negates`). `worn` is the worn items that resolve to a record;
/// a missing player record counts as resisting, which the caller expresses by not calling
/// this at all.
pub fn item_negates<'a>(worn: impl IntoIterator<Item = &'a Item>, spell: SpellNo) -> bool {
    worn.into_iter().any(|item| item.negates.contains(&spell))
}

/// Whether the spell's resist mode lets this target roll at all: always, or only when the
/// target has Anti Magic. Never (and an unknown mode) never rolls.
pub fn may_resist(spell: &Spell, anti_magic: bool) -> bool {
    match spell.resmode {
        ResistMode::Always => true,
        ResistMode::AntiMagic => anti_magic,
        _ => false,
    }
}

/// The chance, in percent, a magic-resistance roll must stay at or under to resist: half
/// the resistance, held to `cap` (`MAXCHANCE` 98 in the player engines, 97 in the monster
/// and room casters' rolls on players).
pub fn resist_chance(mr: i32, cap: i32) -> i32 {
    (mr / 2).min(cap)
}

/// The player's magic-resistance roll of `count_valid_targets` and
/// `cast_user_target`: resisted when a worn item negates the spell, or when the
/// spell may be resisted and `genrdn(1,100)` is at most `min(mr/2, 98)`. A roll of 99 always
/// beats the cap. `mr` is the player's magic resistance (`(intel + 3*wil)/4 + Magic
/// Resistance abilities`).
pub fn player_resists(
    spell: &Spell,
    mr: i32,
    anti_magic: bool,
    item_negated: bool,
    rng: &mut dyn Rng,
) -> bool {
    if item_negated {
        return true;
    }
    may_resist(spell, anti_magic) && rng.range(1, 100) <= resist_chance(mr, MAXCHANCE)
}

/// A monster's magic resistance as the engines build it: its "Magic Resistance" ability
/// total plus the known monster's `magicres`, at least 1. With "Enslave Creature" in the
/// spell the known monster's `charmres` stands
/// in instead, unless it is 0: pass it as `charmres`.
pub fn monster_magic_resistance(
    magicres_ability: i32,
    known_magicres: i16,
    charmres: Option<i16>,
) -> i32 {
    match charmres {
        Some(c) if c != 0 => i32::from(c),
        _ => (magicres_ability + i32::from(known_magicres)).max(1),
    }
}

/// The single-target monster roll of `cast_monster_target`:
/// resisted when the spell may be resisted (Anti Magic read from the monster) and
/// `genrdn(1,100)` is at most `min(mr/2, 98)`. Item negation does not exist for monsters.
pub fn monster_resists_single(spell: &Spell, mr: i32, anti_magic: bool, rng: &mut dyn Rng) -> bool {
    may_resist(spell, anti_magic) && rng.range(1, 100) <= resist_chance(mr, MAXCHANCE)
}

/// The area roll of `cast_no_target` on a monster: the roll and the chance are worked out,
/// but only a roll above 999 would resist, which `genrdn(1,100)` never gives, so a monster
/// never resists an area spell (sic, kept). The roll is still drawn so a seeded sequence
/// stays aligned with the game's draws.
pub fn monster_resists_area(spell: &Spell, mr: i32, rng: &mut dyn Rng) -> bool {
    let _ = spell;
    let resroll = rng.range(1, 100);
    let _reschance = resist_chance(mr, MAXCHANCE);
    resroll > 999
}

/// "Spell Immunity": a monster whose immunity value is above the spell's level is not
/// affected by a hand cast. Autocombat casts
/// and "Non Magical Spell" spells skip the check.
pub fn spell_immune(
    immunity: i32,
    spell_level: u16,
    nonmagical: bool,
    in_autocombat: bool,
) -> bool {
    !in_autocombat && !nonmagical && immunity > i32::from(spell_level)
}

/// The target's resistance to the spell's element, in percent (the same table for players
/// and monsters): only attack types have
/// an element; Cold reads "Alter Cold Attack Damage (Defence)", Hot "Alter Hot Attack Damage
/// (Defence)", Stone "Resist Stone", Poison "Poison Immunity", Water "Resist Water",
/// Lightning "Resist Lightning"; Normal and unknown elements, and General spells, give 0.
/// Nothing here is random.
pub fn element_resistance(spell: &Spell, target: &dyn AbilityLookup) -> i32 {
    if !is_attack_type(spell.spell_type) {
        return 0;
    }
    let ability = match spell.element {
        Element::Cold => Ability::ColdDefence,
        Element::Hot => Ability::HotDefence,
        Element::Stone => Ability::ResistStone,
        Element::Poison => Ability::PoisonImmune,
        Element::Water => Ability::ResistWater,
        Element::Lightning => Ability::ResistLightning,
        _ => return 0,
    };
    target.value(ability)
}

/// Takes an element resistance off an amount: `amount*(100-pct)/100` in 32 bits (per area
/// target, or once on the roll for a single target).
pub fn apply_element_resistance(amount: i32, resist_pct: i32) -> i32 {
    let v = i64::from(amount) * (100 - i64::from(resist_pct)) / 100;
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The percentage a player's magic resistance takes off "Inflict Damage on Armour": with
/// Anti Magic half the resistance, held to 0..=75 (`MRCUT_ANTIMAG`); otherwise half of
/// `mr - 50` (`MR_NEUTRAL`), at most 50 (`MRCUT_MAX`), when that is above 0. A player's
/// magic resistance is unsigned, so the test is a 16-bit unsigned one (sic): below 50 it
/// wraps round to a huge number and the cut is the full 50, and only mr 50 (no cut) and 51
/// (half of 1 is 0) are left with none. `mr` is taken as the 16-bit field. `carried` reproduces
/// the area loop of `cast_no_target`, which does not clear the cut between
/// players: a target without Anti Magic and with exactly 50 keeps the previous target's cut;
/// pass 0 everywhere else.
pub fn mr_cut_pct(mr: i32, anti_magic: bool, carried: i32) -> i32 {
    let mr = mr as u16;
    if anti_magic {
        i32::from((mr / 2) as i16).clamp(0, MRCUT_ANTIMAG)
    } else {
        let mut cut = carried;
        let above = mr.wrapping_sub(MR_NEUTRAL as u16);
        if above > 0 {
            cut = i32::from((above / 2) as i16);
        }
        cut.min(MRCUT_MAX)
    }
}

/// "Inflict Damage on Armour" on a player after magic resistance (from a player's or a
/// monster's cast): with a cut, `amount - amount*cut/100`; with none, an Anti Magic target
/// takes the full amount and any other `amount*(50-mr)/100 + amount`, where `50-mr` is
/// worked unsigned and kept signed 16-bit: 0 at mr 50 and -1 at mr 51, the only two values
/// that reach it (see
/// [`mr_cut_pct`]). `amount` is the strength after the caster's "Alter spell damage" bonus.
pub fn mr_damage(amount: i32, mr: i32, anti_magic: bool, carried_cut: i32) -> i32 {
    let cut = mr_cut_pct(mr, anti_magic, carried_cut);
    if cut == 0 {
        if anti_magic {
            amount
        } else {
            let extra = i32::from((MR_NEUTRAL as u16).wrapping_sub(mr as u16) as i16);
            amount.wrapping_mul(extra) / 100 + amount
        }
    } else {
        amount - amount.wrapping_mul(cut) / 100
    }
}

/// The percentage a monster's magic resistance takes off "Inflict Damage on Armour": the
/// same rule as [`mr_cut_pct`] but signed 16-bit, so a monster below 50 gets no cut and
/// takes extra damage instead. The cut is cleared for every target.
pub fn monster_mr_cut_pct(mr: i32, anti_magic: bool) -> i32 {
    if anti_magic {
        (mr / 2).clamp(0, MRCUT_ANTIMAG)
    } else if mr - MR_NEUTRAL > 0 {
        ((mr - MR_NEUTRAL) / 2).min(MRCUT_MAX)
    } else {
        0
    }
}

/// "Inflict Damage on Armour" on a monster after magic resistance: as [`mr_damage`] with
/// the signed cut of [`monster_mr_cut_pct`], so with no
/// cut and no Anti Magic the monster takes `amount*(50-mr)/100 + amount`.
pub fn monster_mr_damage(amount: i32, mr: i32, anti_magic: bool) -> i32 {
    let cut = monster_mr_cut_pct(mr, anti_magic);
    if cut == 0 {
        if anti_magic {
            amount
        } else {
            amount.wrapping_mul(MR_NEUTRAL - mr) / 100 + amount
        }
    } else {
        amount - amount.wrapping_mul(cut) / 100
    }
}

/// The room engine's multiplier on a player (a room spell):
/// `amount * max(150 - mr, 25) / 100`, with no Anti Magic branch and no caps.
pub fn room_mr_damage(amount: i32, mr: i32) -> i32 {
    let pct = (150 - mr).max(25);
    amount.wrapping_mul(pct) / 100
}

/// Whether a spell type can be resisted by an element at all (the attack types); kept for
/// callers that only have the type.
pub fn has_element(spell_type: SpellType) -> bool {
    is_attack_type(spell_type)
}
