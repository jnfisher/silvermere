//! Spells: spell effects, resistance, durations and the arithmetic of a cast.
//!
//! The rules shared by the casting engines (a player casting with no target, at a player,
//! at a monster or at an item; a monster casting at one target or at the room; a room
//! spell; a room spell on a monster: `cast_no_target`, `cast_user_target`,
//! `cast_monster_target`, `cast_item_target`, `monster_cast`, `monster_cast_area`,
//! `room_cast_on_monster` in the engine). The engines differ almost only in messaging; this
//! crate keeps the arithmetic and leaves every string to the engine (T-050 to T-053):
//!
//! - [`effect`]: the ten ability slots of a spell record become a [`SpellEffect`] each
//!   ([`SpellEffect::from_slot`], [`SpellEffects::from_record`]). `census.md` beside this
//!   file lists every ability the engines handle and the variant it became.
//! - this module: the strength roll ([`roll_amount`]), its scaling ([`scale_spell_amount`],
//!   [`per_target_share`]), the duration ([`duration`]), the active-spell slot tables of a
//!   character and a monster ([`add_spell_to_character`], [`add_spell_to_monster`],
//!   [`monster_add_spell_to_character`]), who an area spell may hit ([`is_valid_target`],
//!   [`is_valid_monster_target`]), who may cast what ([`can_use_spell`],
//!   [`spell_can_hit_monster`]) and what a spell may be aimed at ([`match_kinds`]).
//! - [`resist`]: item negation, the magic-resistance rolls of players and monsters (with the
//!   game's never-resisting area path), Spell Immunity, element resistance and the
//!   magic-resistance damage multipliers.
//! - [`apply`]: one effect applied to one [`Target`] giving an [`Outcome`], plus the
//!   per-round and end-of-spell effects of a spell in a slot.
//!
//! Randomness comes through [`Rng`]; nothing here reads a clock, a map or a message.

pub mod apply;
pub mod apply_monster;
pub mod effect;
pub mod resist;

#[cfg(test)]
mod tests;

use silvermere_types::enums::{Ability, Alignment, Magery, MonsterType, SpellTarget, SpellType};
use silvermere_types::ids::SpellNo;
use silvermere_types::records::{ActiveSpell, Class, Spell};
use silvermere_types::sizes::{NMONSPELLS, NSPELLCAST};

use crate::abilities::spell_has_ability;
use crate::rng::Rng;

pub use apply::{
    CasterStats, ItemTarget, MonsterTarget, Outcome, Target, UserTarget, apply_effect, apply_round,
    apply_round_slot, apply_termination, cut_poison, knocks_out, raise_hp,
};
pub use effect::{EffectSlot, SpellEffect, SpellEffects, Stat};

/// Best chance of success a resistance or casting roll can have, in percent.
pub const MAXCHANCE: i32 = 98;
/// `spell.diff` at or above which a cast never fails.
pub const DIFF_NOFAIL: i16 = 200;
/// Magic resistance that changes nothing.
pub const MR_NEUTRAL: i32 = 50;
/// Most damage magic resistance takes off, in percent.
pub const MRCUT_MAX: i32 = 50;
/// The same with Anti Magic.
pub const MRCUT_ANTIMAG: i32 = 75;
/// Evil points from which a character is an Outlaw.
pub const EVIL_OUTLAW: i16 = 40;
/// Evil points an evil spell costs.
pub const EVIL_SPELL_PTS: i16 = 10;

/// The kind of target a spell record names: `spell.target`, 0..13, with the words the game
/// shows for them. The same enum as the record's; the engines read it through the
/// helpers of [`TargetKindExt`].
pub type TargetKind = SpellTarget;

/// What the engines ask of a [`TargetKind`].
pub trait TargetKindExt {
    /// An area spell: the "Divided ..." and "Full ..." kinds, cast without a target name
    /// ([`match_kinds`]).
    fn is_area(&self) -> bool;
    /// A "Divided ..." area: the rolled strength is split between the targets. Single (and
    /// unknown) kinds divide too; see [`per_target_share`].
    fn is_divided(&self) -> bool;
    /// An area kind that also reaches the monsters in the room (`count_valid_targets`).
    fn area_reaches_monsters(&self) -> bool;
    /// An area kind whose instant effect includes the caster.
    fn area_includes_caster(&self) -> bool;
    /// The attack areas ([`is_valid_target`]).
    fn is_attack_area(&self) -> bool;
    /// The party areas.
    fn is_party_area(&self) -> bool;
    /// The kinds `cast_no_target` treats as the caster himself.
    fn is_self(&self) -> bool;
}

impl TargetKindExt for TargetKind {
    fn is_area(&self) -> bool {
        matches!(
            self,
            Self::DivArea
                | Self::DivAreaIncl
                | Self::DivAttack
                | Self::DivParty
                | Self::FullArea
                | Self::FullAttack
                | Self::FullParty
        )
    }

    fn is_divided(&self) -> bool {
        !matches!(self, Self::FullArea | Self::FullAttack | Self::FullParty)
    }

    fn area_reaches_monsters(&self) -> bool {
        matches!(
            self,
            Self::FullAttack | Self::DivAttack | Self::FullArea | Self::DivAreaIncl | Self::DivArea
        )
    }

    fn area_includes_caster(&self) -> bool {
        matches!(self, Self::DivAreaIncl | Self::FullParty | Self::DivParty)
    }

    fn is_attack_area(&self) -> bool {
        matches!(self, Self::DivAttack | Self::FullAttack)
    }

    fn is_party_area(&self) -> bool {
        matches!(self, Self::DivParty | Self::FullParty)
    }

    fn is_self(&self) -> bool {
        matches!(self, Self::OnSelf | Self::SelfOrSingle | Self::Any)
    }
}

/// Whether a spell type is an attack: Auto-Combat, Poison and "Change me" have an element
/// and roll for it; "General" (and unknown) spells do not.
pub fn is_attack_type(spell_type: SpellType) -> bool {
    matches!(
        spell_type,
        SpellType::Autocombat | SpellType::Poison | SpellType::ChangeMe
    )
}

/// The caster's level as the spell counts it: at most `levelcap` when the cap is set
/// (for the strength and for the duration).
pub fn capped_level(spell: &Spell, caster_level: u16) -> i32 {
    let level = i32::from(caster_level);
    if spell.levelcap > 0 && level > i32::from(spell.levelcap) {
        i32::from(spell.levelcap)
    } else {
        level
    }
}

/// The strength range of a spell at a caster level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmountRange {
    /// `minval + mininc*level/minlvls`, held to at most `max`.
    pub min: i32,
    /// `maxval + maxinc*level/maxlvls`.
    pub max: i32,
}

/// The level-adjusted minimum and maximum of a spell:
/// `maxinc` more for every `maxlvls` levels and `mininc` for every `minlvls`, no growth when
/// the divisor is 0, and a minimum above the maximum is brought down to it.
pub fn amount_range(spell: &Spell, caster_level: u16) -> AmountRange {
    let level = capped_level(spell, caster_level);
    let grow = |inc: u8, lvls: u8| {
        // The record holds signed bytes; the stock data never goes
        // negative, so the growth is read as the bytes say.
        if lvls > 0 {
            i32::from(inc) * level / i32::from(lvls)
        } else {
            0
        }
    };
    let min = i32::from(spell.minval) + grow(spell.mininc, spell.minlvls);
    let max = i32::from(spell.maxval) + grow(spell.maxinc, spell.maxlvls);
    AmountRange {
        min: min.min(max),
        max,
    }
}

/// Rolls the strength of a cast: uniform in `[min, max]` of [`amount_range`]
/// (`genrdn(0, max-min+1) + min`).
pub fn roll_amount(spell: &Spell, caster_level: u16, rng: &mut dyn Rng) -> i32 {
    let range = amount_range(spell, caster_level);
    rng.range(0, range.max - range.min + 1) + range.min
}

/// Scales an amount by the caster's "Alter spell damage" percentage: `amount*(pct+100)/100`,
/// worked in 32 bits.
pub fn scale_spell_amount(amount: i32, spelldamage_pct: i32) -> i32 {
    let scaled = i64::from(amount) * (i64::from(spelldamage_pct) + 100) / 100;
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The inline form the "Inflict Damage on Armour" cases use instead of
/// [`scale_spell_amount`]: `pct*amount/100 + amount` in 16 bits. It rounds the bonus toward
/// zero before adding, so for a negative percentage it can differ from the 32-bit form by
/// one.
pub fn add_spell_damage_bonus(amount: i32, spelldamage_pct: i32) -> i32 {
    spelldamage_pct.wrapping_mul(amount) / 100 + amount
}

/// Each target's share of the rolled strength: a "Divided" area, a single target and any
/// unknown kind give `amount/ntargets`, a "Full" area gives every target all of it. The game
/// never divides by zero (the caster counts himself, or the cast is refused earlier); here
/// `ntargets` below 1 is read as 1.
pub fn per_target_share(amount: i32, target: TargetKind, ntargets: i32) -> i32 {
    if target.is_divided() {
        amount / ntargets.max(1)
    } else {
        amount
    }
}

/// What the duration arithmetic needs from the caster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CasterLevel {
    /// The caster's level (the spell's level cap applies).
    pub level: u16,
    /// The caster's "Alter spell duration" total, a percentage added to 100.
    pub spellduration_pct: i32,
}

/// The rounds a lasting spell gets, on a character or on a monster: the record's base
/// duration grows by
/// `durinc` for every `durlvls` levels of the (capped) caster level, is re-rolled between
/// itself and `durlvl*level` (both ends included) when it is shorter than that, and is then
/// scaled by the caster's "Alter spell duration" percentage. With no caster (a room spell)
/// the base duration is used as it is.
pub fn duration(spell: &Spell, caster: Option<CasterLevel>, rng: &mut dyn Rng) -> i32 {
    let mut duration = i32::from(spell.duration);
    let Some(caster) = caster else {
        return duration;
    };
    let level = capped_level(spell, caster.level);
    if spell.durlvls != 0 {
        duration += i32::from(spell.durinc) * (level / i32::from(spell.durlvls));
    }
    let maxdur = i32::from(spell.durlvl) * level;
    if duration < maxdur {
        duration = rng.range(duration, maxdur + 1);
    }
    let scaled = i64::from(duration) * (i64::from(caster.spellduration_pct) + 100) / 100;
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// What putting a spell into a slot table did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOutcome {
    /// Put into the free slot at this index.
    Added(usize),
    /// Already active and renewed in place at this index: new value and rounds.
    Renewed(usize),
    /// Already active and not renewable: the cast fails ("You attempt to cast ..., but
    /// fail.").
    AlreadyActive,
    /// No free slot: the cast fails.
    Full,
}

impl SlotOutcome {
    /// Whether the spell is in a slot after the call.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Added(_) | Self::Renewed(_))
    }
}

/// The active-spell slots of a character: `spells`, `spellvals` and `spellrnds`, ten of
/// each.
#[derive(Debug, PartialEq, Eq)]
pub struct CharacterSlots<'a> {
    /// The spell numbers, 0 for a free slot.
    pub spells: &'a mut [SpellNo; NSPELLCAST],
    /// The strength each was cast with.
    pub spellvals: &'a mut [i16; NSPELLCAST],
    /// The rounds left of each.
    pub spellrnds: &'a mut [i16; NSPELLCAST],
}

/// Puts a lasting spell on a character: a
/// spell already active is renewed (value and rounds replaced) only when `renew` is set,
/// otherwise the cast fails; a new spell takes the first free slot; a full table fails.
/// `duration` and `power` are stored as the record's 16-bit fields hold them.
pub fn add_spell_to_character(
    slots: CharacterSlots<'_>,
    spell: SpellNo,
    duration: i32,
    power: i32,
    renew: bool,
) -> SlotOutcome {
    if let Some(i) = slots.spells.iter().position(|&s| s == spell) {
        if renew {
            slots.spellvals[i] = power as i16;
            slots.spellrnds[i] = duration as i16;
            return SlotOutcome::Renewed(i);
        }
        return SlotOutcome::AlreadyActive;
    }
    match slots.spells.iter().position(|&s| s == SpellNo(0)) {
        Some(i) => {
            slots.spells[i] = spell;
            slots.spellrnds[i] = duration as i16;
            slots.spellvals[i] = power as i16;
            SlotOutcome::Added(i)
        }
        None => SlotOutcome::Full,
    }
}

/// Puts a lasting spell on an active monster: a spell already active is always renewed; a
/// new one takes the first
/// free of the five slots; a full table fails.
pub fn add_spell_to_monster(
    slots: &mut [ActiveSpell; NMONSPELLS],
    spell: SpellNo,
    duration: i32,
    power: i32,
) -> SlotOutcome {
    if let Some(i) = slots.iter().position(|s| s.spell == spell) {
        slots[i].rounds = duration as i16;
        slots[i].value = power as i16;
        return SlotOutcome::Renewed(i);
    }
    match slots.iter().position(|s| s.spell == SpellNo(0)) {
        Some(i) => {
            slots[i] = ActiveSpell {
                spell,
                value: power as i16,
                rounds: duration as i16,
            };
            SlotOutcome::Added(i)
        }
        None => SlotOutcome::Full,
    }
}

/// Puts a monster's lasting spell on a character (`monster_add_cast_spell_to_user`). Unlike
/// the player engines' [`add_spell_to_character`]: no duration
/// scaling (the caller passes the rounds), a spell already active is renewed only when
/// `extend` is set *and* the new value is stronger than the stored one, a weaker or
/// equal cast is ignored, and a new spell takes the *last* free slot.
pub fn monster_add_spell_to_character(
    slots: CharacterSlots<'_>,
    spell: SpellNo,
    duration: i32,
    effect: i32,
    extend: bool,
) -> SlotOutcome {
    let mut free = None;
    for i in 0..NSPELLCAST {
        if slots.spells[i] == spell {
            if extend && i32::from(slots.spellvals[i]) < effect {
                slots.spellvals[i] = effect as i16;
                slots.spellrnds[i] = duration as i16;
                return SlotOutcome::Renewed(i);
            }
            return SlotOutcome::AlreadyActive;
        }
        if slots.spells[i] == SpellNo(0) {
            free = Some(i);
        }
    }
    match free {
        Some(i) => {
            slots.spells[i] = spell;
            slots.spellrnds[i] = duration as i16;
            slots.spellvals[i] = effect as i16;
            SlotOutcome::Added(i)
        }
        None => SlotOutcome::Full,
    }
}

/// Everything `is_valid_target` asks about a candidate player that is not
/// in the spell record: the engine fills it from its session and room state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerCandidate {
    /// The candidate is the caster himself.
    pub is_caster: bool,
    /// Both stand in the same room of the same map.
    pub same_room: bool,
    /// The candidate leads the caster.
    pub leads_caster: bool,
    /// The caster leads the candidate.
    pub led_by_caster: bool,
    /// The candidate is in the caster's group.
    pub in_casters_group: bool,
    /// The candidate's evil points.
    pub evil: i16,
    /// `should_give_evil`: attacking him would earn evil.
    pub attack_gives_evil: bool,
    /// `pvp_level_range_ok`.
    pub pvp_range_ok: bool,
    /// The caster stands in a colliseum while arena combat is safe.
    pub arena_safe: bool,
    /// The caster is a sysop in debug mode.
    pub sysop_debug: bool,
}

/// Why a spell may hit a target, and what it costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidTarget {
    /// Hitting this target is an evil act.
    pub evil: bool,
}

/// May the caster aim this spell at that player?
///
/// Hitting him is evil when he has fewer than [`EVIL_OUTLAW`] evil points and
/// `should_give_evil` says so. He must be in the caster's room. The
/// player-killing rules apply to every spell but a "General" one without "Evil in Combat":
/// the pvp level range, or a Single/Self/Self-or-Single spell on oneself, or a
/// safe arena, or sysop debug. Then the kind of target decides: single and self
/// kinds take only the caster; the attack areas exclude the caster, his leader, his
/// followers and his group; "Divided Area Not You" excludes only the caster; the party areas
/// take exactly those the attack areas exclude; every other kind takes anyone.
pub fn is_valid_target(spell: &Spell, candidate: &PlayerCandidate) -> Option<ValidTarget> {
    let evil = candidate.evil < EVIL_OUTLAW && candidate.attack_gives_evil;
    if !candidate.same_room {
        return None;
    }
    let pvp_rules = !matches!(spell.spell_type, SpellType::General)
        || spell_has_ability(spell, Ability::EvilInCombat);
    if pvp_rules && !candidate.pvp_range_ok {
        let self_kind = matches!(
            spell.target,
            TargetKind::Single | TargetKind::OnSelf | TargetKind::SelfOrSingle
        );
        if self_kind && candidate.is_caster {
            return Some(ValidTarget { evil });
        }
        if !candidate.arena_safe && !candidate.sysop_debug {
            return None;
        }
    }
    let party = candidate.is_caster
        || candidate.leads_caster
        || candidate.led_by_caster
        || candidate.in_casters_group;
    let valid = match spell.target {
        TargetKind::Single | TargetKind::OnSelf | TargetKind::SelfOrSingle => candidate.is_caster,
        TargetKind::DivAttack | TargetKind::FullAttack => !party,
        TargetKind::DivArea => !candidate.is_caster,
        TargetKind::DivParty | TargetKind::FullParty => party,
        _ => true,
    };
    valid.then_some(ValidTarget { evil })
}

/// Everything [`is_valid_monster_target`] asks about a candidate monster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterCandidate {
    /// The monster's alignment.
    pub alignment: Alignment,
    /// Its generation type.
    pub montype: MonsterType,
    /// It follows the caster by name.
    pub follows_caster: bool,
    /// It is charmed (`ActiveMonsterFlags::CHARMED`).
    pub charmed: bool,
    /// It is friendly.
    pub friendly: bool,
    /// The caster's evil points.
    pub caster_evil: i16,
    /// `spell_can_hit_monster` ([`spell_can_hit_monster`]).
    pub can_hit: bool,
}

/// May the caster aim this spell at that monster?
///
/// Hitting a Good or Lawful Good monster that does not follow the caster is evil. The spell
/// must be able to hit it at all. The attack areas spare a
/// charmed or friendly follower, always take an unfriendly one, spare guards and angels
/// while the caster has fewer than [`EVIL_OUTLAW`] evil points, and never take a Lawful Good
/// monster; the party areas take only a charmed follower; single,
/// self and item kinds never take a monster; every other kind takes any.
pub fn is_valid_monster_target(spell: &Spell, candidate: &MonsterCandidate) -> Option<ValidTarget> {
    let evil = matches!(candidate.alignment, Alignment::Good | Alignment::LawfulGood)
        && !candidate.follows_caster;
    if !candidate.can_hit {
        return None;
    }
    let valid = match spell.target {
        TargetKind::DivAttack | TargetKind::FullAttack => {
            if candidate.charmed || candidate.friendly {
                if candidate.follows_caster {
                    return None;
                }
            } else if candidate.follows_caster {
                return Some(ValidTarget { evil });
            }
            let outlaw = candidate.caster_evil >= EVIL_OUTLAW;
            if matches!(candidate.montype, MonsterType::Guard | MonsterType::Angel) && !outlaw {
                return None;
            }
            !matches!(candidate.alignment, Alignment::LawfulGood)
        }
        TargetKind::DivParty | TargetKind::FullParty => {
            candidate.charmed && candidate.follows_caster
        }
        TargetKind::Single | TargetKind::OnSelf | TargetKind::SelfOrSingle | TargetKind::Item => {
            false
        }
        _ => true,
    };
    valid.then_some(ValidTarget { evil })
}

/// Can this spell affect a monster with "Required to hit"? A monster without the ability
/// can always be affected; otherwise
/// the spell's own "Required to hit" value must equal the monster's.
pub fn spell_can_hit_monster(monster_reqtohit: i32, spell: &Spell) -> bool {
    if monster_reqtohit != 0 {
        return crate::abilities::spell_ability_value(spell, Ability::ReqToHit) == monster_reqtohit;
    }
    true
}

/// The three bands of the evil law that `user_can_use_spell` distinguishes (by the legal
/// levels, [`crate::evil::legal_level`]): Good and Saint; Neutral and Seedy; Outlaw,
/// Criminal, Villain and Fiend. The evil module (T-038) maps evil points to the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalBand {
    /// -51 evil points and below (Good, Saint).
    Good,
    /// -50 to 39 (Neutral, Seedy).
    Neutral,
    /// 40 and above (Outlaw, Criminal, Villain, Fiend).
    Evil,
}

impl LegalBand {
    /// The band of an evil-point total, by the legal levels' thresholds.
    pub fn of_evil(evil: i16) -> Self {
        if evil <= -51 {
            Self::Good
        } else if evil < EVIL_OUTLAW {
            Self::Neutral
        } else {
            Self::Evil
        }
    }
}

/// Why a character may not cast a spell (`user_can_use_spell`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CannotUse {
    /// The spell has a magery and the class's is another.
    WrongMagery,
    /// The class's magery level is below the spell's `magelvl`.
    MageryLevel,
    /// The character's level is below the spell's: "This spell is too powerful
    /// for you".
    TooPowerful,
    /// An alignment ability of the spell forbids it at the character's legal level.
    Alignment,
}

/// May this character cast (or use through an item) this spell? A spell of a magery needs
/// the class to have that magery at the
/// spell's magery level or above; the character must have reached the spell's level; and
/// the spell's alignment abilities must suit the legal band: good characters cast no Evil,
/// Not Good or Neutral spell; neutral ones no Evil or Good spell; evil ones no Good, Not
/// Evil or Neutral spell.
pub fn can_use_spell(
    spell: &Spell,
    class: &Class,
    level: u16,
    legal: LegalBand,
) -> Result<(), CannotUse> {
    if spell.magery != Magery::None {
        if spell.magery != class.magery {
            return Err(CannotUse::WrongMagery);
        }
        if spell.magelvl > class.magelvl {
            return Err(CannotUse::MageryLevel);
        }
    }
    if spell.level > level {
        return Err(CannotUse::TooPowerful);
    }
    let has = |a| spell_has_ability(spell, a);
    let forbidden = match legal {
        LegalBand::Good => has(Ability::Evil) || has(Ability::NotGood) || has(Ability::Neutral),
        LegalBand::Neutral => has(Ability::Evil) || has(Ability::Good),
        LegalBand::Evil => has(Ability::Good) || has(Ability::NotEvil) || has(Ability::Neutral),
    };
    if forbidden {
        return Err(CannotUse::Alignment);
    }
    Ok(())
}

/// What kinds of name the CAST command may aim a spell at: what the target search looks
/// through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MatchKinds {
    /// Players in the room.
    pub users: bool,
    /// Monsters in the room.
    pub monsters: bool,
    /// Items carried.
    pub inventory: bool,
    /// Items lying in the room.
    pub floor: bool,
    /// Spells in the spellbook.
    pub spells: bool,
    /// A player may not name himself (combat spells on a single target).
    pub not_self: bool,
    /// Charmed monsters are matched last.
    pub charmed_last: bool,
}

impl MatchKinds {
    /// An area spell: no target name is wanted.
    pub fn none(self) -> bool {
        !(self.users || self.monsters || self.inventory || self.floor || self.spells)
    }
}

/// What a spell's target kind lets CAST name. Area kinds take no name; Single,
/// Self and Self or Single take a player (not the caster for a combat spell); Monster takes
/// monsters, charmed last; Item takes carried items and the floor; Monster or Single takes
/// monsters and players; Any and unknown kinds take everything.
pub fn match_kinds(spell: &Spell) -> MatchKinds {
    let everything = MatchKinds {
        users: true,
        monsters: true,
        inventory: true,
        floor: true,
        spells: true,
        not_self: false,
        charmed_last: true,
    };
    match spell.target {
        kind if kind.is_area() => MatchKinds::default(),
        TargetKind::Single | TargetKind::OnSelf | TargetKind::SelfOrSingle => MatchKinds {
            users: true,
            not_self: is_attack_type(spell.spell_type),
            ..MatchKinds::default()
        },
        TargetKind::Monster => MatchKinds {
            monsters: true,
            charmed_last: true,
            ..MatchKinds::default()
        },
        TargetKind::Item => MatchKinds {
            inventory: true,
            floor: true,
            ..MatchKinds::default()
        },
        TargetKind::MonsterOrSingle => MatchKinds {
            users: true,
            monsters: true,
            charmed_last: true,
            ..MatchKinds::default()
        },
        _ => everything,
    }
}
