//! Applying spell effects when the caster is a monster, and a room's spell on a monster.
//!
//! What each ability does when a monster casts at a player (`monster_cast`), at the room
//! (`monster_cast_area`) or on itself (`cast_self`), and when a room casts on a monster
//! (`room_cast_on_monster`), per `census.md` sections C.2 and D (T-049b). The player-cast
//! engines and the room-on-player
//! engine are in [`super::apply`]; this module reuses its [`Outcome`] vocabulary, its
//! [`UserTarget`] and [`MonsterTarget`], and the amount, slot and resistance arithmetic of the
//! parent module and [`super::resist`]. It adds no resistance rule: the monster and room
//! casters' cap of 97 on the player's roll ([`MONSTER_RESIST_CAP`]) goes through
//! [`super::resist::resist_chance`].
//!
//! Each engine keeps its own shape, because their loops differ:
//!
//! - `monster_cast` applies one slot to one player ([`apply_monster_cast`]); the lasting
//!   spell goes on once per cast and a refusal ends the cast ([`MonsterCastState`]).
//! - `monster_cast_area` applies one slot either to the casting monster itself (the self
//!   kinds) or to every marked player at once (the area kinds), each slot on every player
//!   before the next slot, as in the game ([`apply_monster_cast_area`]); the "Remove spell"
//!   pre-pass is [`apply_monster_area_remove_pass`].
//! - `cast_self` is one call ([`apply_monster_self_spell`]).
//! - `room_cast_on_monster` applies the ten record slots with the game's skip back to
//!   slot 6 ([`apply_room_on_monster_spell`]), one slot at a time through
//!   [`apply_room_on_monster`].
//!
//! No message is built here. A kill is the engine's to see from the hit points (as in
//! [`super::apply`]); a summon, an item to erase, a text block and a teleport come back as
//! [`Outcome`]s. The prelude numbers the engines need (the cast level roll, the duration,
//! the resistance roll, the room's charm resistance and cap) are the free functions at the
//! top of this module.

use silvermere_types::enums::Ability;
use silvermere_types::ids::{ItemNo, SpellNo};
use silvermere_types::records::{ActiveMonster, Character, KnownMonster, Spell};
use silvermere_types::sizes::{NMONSPELLS, NRECABILS, NSPELLCAST};

use super::apply::{
    EndedSpell, MonsterTarget, Outcome, PoisonCut, SummonDisposition, UserTarget, clear_slot,
    damage_user, end_spells_where, knocks_out, terminate_user_poison_spell,
};
use super::effect::{EffectSlot, SpellEffect, SpellEffects, Stat, TeleportPart};
use super::resist::{
    apply_element_resistance, may_resist, mr_damage, resist_chance, room_mr_damage,
};
use super::{
    AmountRange, CharacterSlots, SlotOutcome, TargetKind, TargetKindExt, add_spell_to_monster,
    monster_add_spell_to_character,
};
use crate::abilities::{item_ability_value, spell_has_ability};
use crate::rng::Rng;

// ---------------------------------------------------------------------------------------
// The prelude numbers: what the engines work out before the effects.

/// Best chance, in percent, a player's resistance roll has against a monster's or a room's
/// cast: 97, where the player engines use [`super::MAXCHANCE`] (98).
pub const MONSTER_RESIST_CAP: i32 = 97;

/// The player's roll against a monster's cast (`monster_count_valid_targets`,
/// `monster_cast`): resisted when a worn item negates the
/// spell, or when the spell may be resisted and `genrdn(1,100)` is at most
/// `min(mr/2, 97)`. The item is checked first, so no roll is drawn for a negated spell.
pub fn player_resists_monster_cast(
    spell: &Spell,
    mr: i32,
    anti_magic: bool,
    item_negated: bool,
    rng: &mut dyn Rng,
) -> bool {
    if item_negated {
        return true;
    }
    may_resist(spell, anti_magic) && rng.range(1, 100) <= resist_chance(mr, MONSTER_RESIST_CAP)
}

/// The player's roll against a room's cast (`room_cast_on_user`): the
/// same chance, but the roll is drawn before the worn items are looked at, so a seeded
/// sequence stays aligned with the game's draws.
pub fn player_resists_room_cast(
    spell: &Spell,
    mr: i32,
    anti_magic: bool,
    item_negated: bool,
    rng: &mut dyn Rng,
) -> bool {
    let rolled =
        may_resist(spell, anti_magic) && rng.range(1, 100) <= resist_chance(mr, MONSTER_RESIST_CAP);
    rolled || item_negated
}

/// The strength range of a spell at a monster's cast level (`monster_cast_area`,
/// `monster_cast`, `cast_self`): as [`super::amount_range`] but with no level cap. The
/// level is the attack slot's maximum (`atkmaxs`, `monster_cast`) or the "spells after
/// combat" slot's level (`cast_by_number`); a cast from no slot gets no growth, which the
/// caller expresses with level 0.
pub fn monster_amount_range(spell: &Spell, level: i32) -> AmountRange {
    let grow = |inc: u8, lvls: u8| {
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

/// Rolls the strength of a monster's cast: uniform in [`monster_amount_range`]
/// (`genrdn(0, max-min+1) + min`).
pub fn roll_monster_amount(spell: &Spell, level: i32, rng: &mut dyn Rng) -> i32 {
    let range = monster_amount_range(spell, level);
    rng.range(0, range.max - range.min + 1) + range.min
}

/// The rounds a monster's lasting spell gets (`monster_cast`,
/// `monster_cast_area`): the record's base duration plus `durinc` for every
/// `durlvls` of the attack slot's `atkmaxs` (the division first), only for a cast from an
/// attack slot (`slot_level` is `None` for a cast from no slot). No re-roll to
/// `durlvl*level` and no
/// "Alter spell duration": a monster has neither. The self spell and the players a
/// room or an area cast reach get the base duration as it is.
pub fn monster_duration(spell: &Spell, slot_level: Option<i32>) -> i32 {
    let mut duration = i32::from(spell.duration);
    if let Some(level) = slot_level
        && spell.durlvls != 0
    {
        duration += level / i32::from(spell.durlvls) * i32::from(spell.durinc);
    }
    duration
}

/// The "magic resistance" `room_cast_on_monster` rolls and multiplies with
/// the known monster's charm resistance when the spell has
/// "Enslave Creature", else 0. The monster's own magic resistance is never read there,
/// and there is no fallback (the player engine's `monster_magic_resistance` has one).
pub fn room_charm_resistance(spell: &Spell, known: &KnownMonster) -> i32 {
    if spell_has_ability(spell, Ability::Enslave) {
        i32::from(known.charmres)
    } else {
        0
    }
}

/// The strength of a room's cast on a monster after the prelude: the
/// raw roll (no caster level: [`super::amount_range`] at level 0) less the monster's element
/// resistance, held to the spell's `maxval`. Each slot's own value is held to `maxval` too,
/// inside [`apply_room_on_monster`].
pub fn room_monster_power(spell: &Spell, roll: i32, element_resist_pct: i32) -> i32 {
    apply_element_resistance(roll, element_resist_pct).min(i32::from(spell.maxval))
}

// ---------------------------------------------------------------------------------------
// The casters, the cast and its state.

/// The casting monster: its active record (hit points, poison, energy and the five slots
/// change here when it casts on itself or steals hit points) and its known-monster record
/// (`maxhp` caps a stolen gain in `monster_cast`; `genindex` ranges a summon).
pub struct MonsterCaster<'a> {
    /// The active monster.
    pub monster: &'a mut ActiveMonster,
    /// Its known-monster record.
    pub known: &'a KnownMonster,
}

/// One monster cast in progress, as the effect code sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterCast<'a> {
    /// The spell record.
    pub spell: &'a Spell,
    /// `monster_cast`: the roll after the target's element cut; `monster_cast_area`: the
    /// undivided roll (`baseroll`).
    pub power: i32,
    /// `monster_cast_area` only: the per-target share of the base roll
    /// ([`super::per_target_share`] over the non-resisting players), which every
    /// per-player effect uses whatever the slot's own value (sic). Unused by
    /// `monster_cast`.
    pub share: i32,
    /// The rounds a lasting effect gets ([`monster_duration`]): `monster_cast` puts it on
    /// the player; `monster_cast_area` keeps it for the effects on the caster itself, the
    /// players getting the base duration.
    pub duration: i32,
}

impl MonsterCast<'_> {
    /// The computed rounds are not 0, which `monster_cast` and the self kinds of
    /// `monster_cast_area` test.
    pub fn lasting(&self) -> bool {
        self.duration != 0
    }

    /// The record's own duration is not 0, which "Slay" and every area effect test.
    pub fn base_lasting(&self) -> bool {
        self.spell.duration != 0
    }
}

/// State carried across the slots of one monster cast: whether the spell went on, and the
/// teleport halves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonsterCastState {
    /// The lasting spell has gone on (`monster_cast`), or an area effect has acted
    /// (`monster_cast_area`); later slots do not add it
    /// again. In the area engine an instant effect sets it too, so a lasting slot after an
    /// instant "Inflict Damage on Armour" never adds the spell (sic).
    pub added: bool,
    /// A lasting add was refused before any success: `monster_cast` gives the energy back
    /// and ends the cast, `monster_cast_area` ends the cast. The engine stops at the slot
    /// that set it.
    pub refused: bool,
    /// The teleport destination, both halves needed.
    pub teleport_room: Option<i32>,
    /// See `teleport_room`.
    pub teleport_map: Option<i32>,
    /// An "Alter HP" slot took: the raise the engine makes with
    /// [`raise_hp`](super::raise_hp) after the slot's lines, as the game's add shows the
    /// cast with its prompt first.
    pub alter_hp: Option<i32>,
}

impl MonsterCastState {
    /// A fresh cast.
    pub fn new() -> Self {
        Self::default()
    }

    /// The teleport destination when both halves were set.
    pub fn teleport(&self) -> Option<(i32, i32)> {
        Some((self.teleport_room?, self.teleport_map?))
    }
}

/// The four kinds `monster_cast` reaches a player with ("S1" in the census): Single, Self
/// or Single, Any, Monster or Single. Every other kind is handed to `monster_cast_area`
/// before any effect.
fn single_kind(kind: TargetKind) -> bool {
    matches!(
        kind,
        TargetKind::Single
            | TargetKind::SelfOrSingle
            | TargetKind::Any
            | TargetKind::MonsterOrSingle
    )
}

/// The kinds `monster_cast_area` treats as the casting monster itself.
fn self_kind(kind: TargetKind) -> bool {
    matches!(
        kind,
        TargetKind::OnSelf | TargetKind::SelfOrSingle | TargetKind::Monster | TargetKind::Any
    )
}

fn character_slots(c: &mut Character) -> CharacterSlots<'_> {
    CharacterSlots {
        spells: &mut c.spells,
        spellvals: &mut c.spellvals,
        spellrnds: &mut c.spellrnds,
    }
}

fn raise_stat(c: &mut Character, stat: Stat, v: i32) {
    let field = match stat {
        Stat::Intellect => &mut c.intel,
        Stat::Wisdom => &mut c.wil,
        Stat::Strength => &mut c.str,
        Stat::Health => &mut c.hea,
        Stat::Agility => &mut c.agi,
        Stat::Charm => &mut c.cha,
    };
    *field = (i32::from(*field) + v) as u16;
}

fn stat_of(ability: Ability) -> Option<Stat> {
    match SpellEffect::from_slot(ability, 0) {
        SpellEffect::StatAlter(stat) => Some(stat),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// monster_cast: one slot on one player.

/// Applies one slot of a monster's single-target cast to the player (`monster_cast`).
/// `cast.power` is the roll after the element cut; the slot's own value replaces it. The
/// lasting spell goes on through [`monster_add_spell_to_character`] with the
/// computed rounds, once per cast; a refusal sets [`MonsterCastState::refused`] and the
/// engine ends the cast. The differences from `cast_user_target` are in the arms: no caster
/// bonus on the damage, "Dispel magic" ending every spell, "Remove Curse" taking only Cursed
/// items, "Cure Poison" with no floor, "Summon" ranged by the caster's generation index.
pub fn apply_monster_cast(
    slot: &EffectSlot,
    cast: &MonsterCast<'_>,
    caster: &mut MonsterCaster<'_>,
    user: &mut UserTarget<'_, '_>,
    state: &mut MonsterCastState,
) -> Outcome {
    let value = slot.amount(cast.power);
    let kind = cast.spell.target;
    let single = single_kind(kind);
    let lasting = cast.lasting();
    let number = cast.spell.number;
    let hp = i32::from(user.character.hp);

    // Once per cast, every lasting effect alike: the add through
    // `monster_add_cast_spell_to_user`; a refusal gives the energy back and ends the cast.
    let add_once = |user: &mut UserTarget<'_, '_>, state: &mut MonsterCastState, extend: bool| {
        if state.added {
            return Outcome::NoEffect;
        }
        let out = monster_add_spell_to_character(
            character_slots(user.character),
            number,
            cast.duration,
            value,
            extend,
        );
        if out.is_success() {
            state.added = true;
        } else {
            state.refused = true;
        }
        Outcome::Lasting(out)
    };

    match slot.ability {
        // the same whatever the kind.
        Ability::TextBlock => Outcome::TextBlock(value),
        Ability::ClearItem => Outcome::ClearItem(value),
        Ability::TeleportRoom => {
            state.teleport_room = Some(value);
            Outcome::Teleport(TeleportPart::Room, value)
        }
        Ability::TeleportMap => {
            state.teleport_map = Some(value);
            Outcome::Teleport(TeleportPart::Map, value)
        }

        // instant, on the self kinds only (Single and Monster or Single
        // fall out), every worn item whose Cursed value is set; no Major Curse clause.
        Ability::RemoveCurse => {
            if lasting
                || !matches!(
                    kind,
                    TargetKind::OnSelf | TargetKind::SelfOrSingle | TargetKind::Any
                )
            {
                return Outcome::NoEffect;
            }
            let worn_slots = user
                .character
                .worn
                .iter()
                .enumerate()
                .filter(|(_, n)| **n != ItemNo(0))
                .filter_map(|(i, &n)| (user.items)(n).map(|item| (i, item)))
                .filter(|(_, item)| item_ability_value(item, Ability::Cursed) != 0)
                .map(|(i, _)| i)
                .collect();
            Outcome::RemoveCurse { worn_slots }
        }

        // as the player engine.
        Ability::Freedom => {
            if lasting || !single {
                return Outcome::NoEffect;
            }
            end_spells_where(user, |spell| {
                spell_has_ability(spell, Ability::HoldPerson)
                    || spell_has_ability(spell, Ability::Paralyze)
            })
        }

        // every active spell ends; the value is ignored.
        Ability::DispelMagic => {
            if lasting || !single {
                return Outcome::NoEffect;
            }
            end_spells_where(user, |_| true)
        }

        // the value as it is, no caster bonus; knocked out when the
        // hit points fall to 0 or below from above.
        Ability::DamageNoArmour => {
            if !single {
                return Outcome::NoEffect;
            }
            if lasting {
                return add_once(user, state, true);
            }
            damage_user(user, value, value)
        }

        // the record's own duration decides; to one above death.
        Ability::Slay => {
            if !single || cast.base_lasting() {
                return Outcome::NoEffect;
            }
            let deathhp = i32::from(user.deathhp);
            let v = if hp >= 0 { hp - deathhp + 1 } else { -deathhp };
            damage_user(user, v, v)
        }

        // the magic-resistance multiplier with no caster bonus, whatever
        // the duration, never added as lasting; the message shows the unsoftened value
        // the knock-out test uses the softened one.
        Ability::DamageArmour => {
            if !single {
                return Outcome::NoEffect;
            }
            let dmg = mr_damage(value, i32::from(user.character.mr), user.anti_magic, 0);
            let after = hp - dmg;
            user.character.hp = after as i16;
            Outcome::Applied {
                amount: value,
                knocked_out: knocks_out(after, dmg),
            }
        }

        // instant only; the monster gains, capped at its known
        // monster's maximum (not the active record's).
        Ability::StealHp => {
            if !single || lasting {
                return Outcome::NoEffect;
            }
            let out = damage_user(user, value, value);
            let gained = (i32::from(caster.monster.hp) + value).min(i32::from(caster.known.maxhp));
            caster.monster.hp = gained as i16;
            stolen(out, value)
        }

        Ability::Energy => {
            if !single {
                return Outcome::NoEffect;
            }
            if lasting {
                return add_once(user, state, true);
            }
            user.character.energy = (i32::from(user.character.energy) + value) as u16;
            Outcome::Applied {
                amount: value,
                knocked_out: false,
            }
        }

        // instant only; the caster's generation index as the range.
        Ability::Summon => {
            if !single || lasting {
                return Outcome::NoEffect;
            }
            Outcome::Summon(value, SummonDisposition::HuntsTargetByCasterIndex)
        }

        Ability::Hunger | Ability::Thirst => {
            if !single {
                return Outcome::NoEffect;
            }
            if lasting {
                return add_once(user, state, true);
            }
            let field = if slot.ability == Ability::Hunger {
                &mut user.character.hunger
            } else {
                &mut user.character.thirst
            };
            *field = (i32::from(*field) + value) as i16;
            Outcome::Applied {
                amount: value,
                knocked_out: false,
            }
        }

        // up to the maximum.
        Ability::Heal => {
            if !single {
                return Outcome::NoEffect;
            }
            if lasting {
                return add_once(user, state, true);
            }
            let maxhp = i32::from(user.character.maxhp);
            let amount = if hp + value > maxhp {
                maxhp - hp
            } else {
                value
            };
            user.character.hp = (hp + amount) as i16;
            Outcome::Applied {
                amount,
                knocked_out: false,
            }
        }

        // up to the maximum, never below 0.
        Ability::HealMana => {
            if !single {
                return Outcome::NoEffect;
            }
            if lasting {
                return add_once(user, state, true);
            }
            let amount = mana_gain(user.character, value);
            user.character.mana = (i32::from(user.character.mana) + amount) as i16;
            Outcome::Applied {
                amount,
                knocked_out: false,
            }
        }

        // the immune are skipped; raised to the value if lower; the
        // lasting form adds the spell first and raises only when that was not refused.
        Ability::Poison => {
            if !single {
                return Outcome::NoEffect;
            }
            if user.poison_immune {
                return Outcome::Immune;
            }
            let raise = |user: &mut UserTarget<'_, '_>| {
                if i32::from(user.character.poison) < value {
                    user.character.poison = value as i16;
                }
            };
            if lasting {
                let out = add_once(user, state, true);
                if state.refused {
                    return out;
                }
                raise(user);
                return out;
            }
            raise(user);
            Outcome::Applied {
                amount: value,
                knocked_out: false,
            }
        }

        // the poison spell ends and the level falls, with no floor.
        Ability::CurePoison => {
            if !single {
                return Outcome::NoEffect;
            }
            if lasting {
                return add_once(user, state, true);
            }
            Outcome::Ended {
                spells: terminate_user_poison_spell(user).into_iter().collect(),
                poison: Some(PoisonCut {
                    amount: value,
                    floor: false,
                }),
            }
        }

        // nothing.
        Ability::KillDead
        | Ability::DetectMagic
        | Ability::None
        | Ability::Enslave
        | Ability::EvilInCombat => Outcome::NoEffect,

        // lasting only, on the single kinds and Self; the add is made
        // without extending (an active instance is a refusal that ends the cast), and the
        // stat rises when it was made or an earlier slot already added the spell.
        Ability::AlterHp
        | Ability::Intellect
        | Ability::Wisdom
        | Ability::Strength
        | Ability::Health
        | Ability::Agility
        | Ability::Charm => {
            if !(single || kind == TargetKind::OnSelf) || !lasting {
                return Outcome::NoEffect;
            }
            let out = add_once(user, state, false);
            if state.refused {
                return out;
            }
            if slot.ability == Ability::AlterHp {
                state.alter_hp = Some(value);
            } else if let Some(stat) = stat_of(slot.ability) {
                raise_stat(user.character, stat, value);
            }
            out
        }

        // every other ability is a lasting effect on the single kinds,
        // nothing without a duration (never silly here).
        _ => {
            if !single || !lasting {
                return Outcome::NoEffect;
            }
            add_once(user, state, true)
        }
    }
}

/// Applies every slot of a monster's single-target cast, in record order, stopping where
/// the game stops: a refused lasting add ([`MonsterCastState::refused`]) or a death (the
/// player's hit points at or below `deathhp` after a damage slot, `check_kill_user`).
/// Returns the outcomes of the slots applied.
pub fn apply_monster_cast_spell(
    effects: &SpellEffects,
    cast: &MonsterCast<'_>,
    caster: &mut MonsterCaster<'_>,
    user: &mut UserTarget<'_, '_>,
    state: &mut MonsterCastState,
) -> Vec<Outcome> {
    let mut outs = Vec::new();
    for slot in effects.iter() {
        let out = apply_monster_cast(slot, cast, caster, user, state);
        if let Some(v) = state.alter_hp.take() {
            super::raise_hp(user.character, v);
        }
        if let Outcome::Ended {
            poison: Some(cut), ..
        } = out
        {
            super::apply::cut_poison(user.character, cut);
        }
        let hurt = matches!(
            slot.ability,
            Ability::DamageNoArmour | Ability::Slay | Ability::DamageArmour | Ability::StealHp
        ) && matches!(out, Outcome::Applied { .. } | Outcome::StolenHp { .. });
        outs.push(out);
        if state.refused || (hurt && user.character.hp <= user.deathhp) {
            break;
        }
    }
    outs
}

/// "Heal Mana" on a player as the monster and area engines work it: a positive value up to
/// the maximum, a negative one never below 0.
fn mana_gain(c: &Character, value: i32) -> i32 {
    let mana = i32::from(c.mana);
    let maxmana = i32::from(c.maxmana);
    if value > 0 {
        if mana + value > maxmana {
            maxmana - mana
        } else {
            value
        }
    } else if mana + value < 0 {
        -mana
    } else {
        value
    }
}

fn stolen(out: Outcome, amount: i32) -> Outcome {
    match out {
        Outcome::Applied {
            knocked_out: true, ..
        } => out,
        _ => Outcome::StolenHp { amount },
    }
}

// ---------------------------------------------------------------------------------------
// monster_cast_area: one slot on the caster or on every marked player.

/// What one slot of a monster's area cast did: on the casting monster or its room, and on
/// each marked player (empty when the slot does not reach the players).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaOutcome {
    /// The effect on the caster itself (a self kind) or on the room (an item erased, a
    /// summon): `NoEffect` when the slot acts on the players.
    pub caster: Outcome,
    /// One outcome per player of the slice, in order; empty when the slot does not reach
    /// them. After a refused lasting add the list stops at the player who refused.
    pub players: Vec<Outcome>,
}

impl AreaOutcome {
    fn none() -> Self {
        Self {
            caster: Outcome::NoEffect,
            players: Vec::new(),
        }
    }
}

/// The "Remove spell" and "Terminate Spell" pre-pass of `monster_cast_area` before the
/// slots: for every such slot with a value, on an area kind, every marked player's active
/// spell of that number is cleared and ended (`end_player_spell`, with `chain` set for
/// "Remove spell"), and a player
/// whose spell was ended loses his mark, so the slots that follow leave him alone.
/// Returns one [`Outcome::Ended`] per player and the indexes of the players unmarked; the
/// engine takes those out of the slice it passes to [`apply_monster_cast_area`].
pub fn apply_monster_area_remove_pass(
    effects: &SpellEffects,
    spell: &Spell,
    players: &mut [UserTarget<'_, '_>],
) -> RemovePassOutcome {
    let mut out = RemovePassOutcome {
        players: players
            .iter()
            .map(|_| Outcome::Ended {
                spells: Vec::new(),
                poison: None,
            })
            .collect(),
        unmarked: Vec::new(),
    };
    if !spell.target.is_area() {
        return out;
    }
    for slot in effects.iter() {
        let chain = match slot.ability {
            Ability::RemoveSpell => true,
            Ability::TermSpell => false,
            _ => continue,
        };
        if slot.value == 0 {
            continue;
        }
        let rmvnum = SpellNo(slot.value);
        for (p, user) in players.iter_mut().enumerate() {
            if out.unmarked.contains(&p) {
                continue;
            }
            for k in 0..NSPELLCAST {
                if user.character.spells[k] != rmvnum {
                    continue;
                }
                let power = i32::from(user.character.spellvals[k]);
                clear_slot(user.character, k);
                if (user.spells)(rmvnum).is_none() {
                    continue;
                }
                if let Outcome::Ended { spells, .. } = &mut out.players[p] {
                    spells.push(EndedSpell {
                        number: rmvnum,
                        power,
                        chain,
                    });
                }
                if !out.unmarked.contains(&p) {
                    out.unmarked.push(p);
                }
            }
        }
    }
    out
}

/// The result of [`apply_monster_area_remove_pass`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemovePassOutcome {
    /// One [`Outcome::Ended`] per player, the spells ended on him and the chain casts asked.
    pub players: Vec<Outcome>,
    /// The players (indexes into the slice) whose mark was taken off.
    pub unmarked: Vec<usize>,
}

/// Applies one slot of a monster's area cast (`monster_cast_area`):
/// to the casting monster itself for the self kinds (Self, Self or Single, Monster, Any),
/// to every marked player of `players` for the area kinds, and to nobody for Single, Item
/// and Monster or Single. The slot's own value replaces `cast.power` for the effects on the
/// caster and the room; the per-player effects use `cast.share` whatever the slot holds
/// (sic). The lasting spell goes on the players once per cast with the base duration and
/// each player's element share; a refusal before any success sets
/// [`MonsterCastState::refused`] and the engine ends the cast.
pub fn apply_monster_cast_area(
    slot: &EffectSlot,
    cast: &MonsterCast<'_>,
    caster: &mut MonsterCaster<'_>,
    players: &mut [UserTarget<'_, '_>],
    state: &mut MonsterCastState,
) -> AreaOutcome {
    let roll = slot.amount(cast.power);
    let value = cast.share;
    let kind = cast.spell.target;
    let on_self = self_kind(kind);
    let area = kind.is_area();
    let base_lasting = cast.base_lasting();
    let number = cast.spell.number;
    let mut out = AreaOutcome::none();

    // a lasting effect on the caster itself: into its slots with the computed rounds.
    let self_add = |caster: &mut MonsterCaster<'_>| {
        if cast.duration != 0 {
            Outcome::Lasting(add_spell_to_monster(
                &mut caster.monster.spells,
                number,
                cast.duration,
                roll,
            ))
        } else {
            Outcome::NoEffect
        }
    };
    // once per cast: each marked player gets the base duration and his element share, with
    // renewal only when stronger; the first refusal before any success ends the cast.
    let area_add = |players: &mut [UserTarget<'_, '_>], state: &mut MonsterCastState| {
        if state.added {
            return players.iter().map(|_| Outcome::NoEffect).collect();
        }
        let mut outs = Vec::with_capacity(players.len());
        let mut any = false;
        for user in players.iter_mut() {
            let dose = apply_element_resistance(value, user.element_resist_pct);
            let o = monster_add_spell_to_character(
                character_slots(user.character),
                number,
                i32::from(cast.spell.duration),
                dose,
                true,
            );
            let refused = !o.is_success();
            outs.push(Outcome::Lasting(o));
            if refused && !any {
                state.refused = true;
                return outs;
            }
            any |= !refused;
        }
        if !any {
            state.refused = true;
        }
        outs
    };
    // The per-player share less the player's element resistance.
    let resisted =
        |user: &UserTarget<'_, '_>| apply_element_resistance(value, user.element_resist_pct);

    match slot.ability {
        // the undivided roll, from the monster's room.
        Ability::ClearItem => out.caster = Outcome::ClearItem(roll),

        // the self kinds get a lasting effect only; an instant area
        // cast hurts the marked players only for Divided Area Incl. You, Full Party Area and
        // Divided Party Area (sic), each by his element share, with no
        // magic resistance and no caster bonus.
        Ability::DamageNoArmour => {
            if on_self {
                out.caster = self_add(caster);
            } else if area {
                if !base_lasting {
                    let hits = matches!(
                        kind,
                        TargetKind::DivAreaIncl | TargetKind::FullParty | TargetKind::DivParty
                    );
                    out.players = players
                        .iter_mut()
                        .map(|user| {
                            if !hits {
                                return Outcome::NoEffect;
                            }
                            let amount = resisted(user);
                            damage_user(user, amount, amount)
                        })
                        .collect();
                } else {
                    out.players = area_add(players, state);
                }
                state.added = true;
            }
        }

        // the self kinds get a lasting effect only; the area kinds take
        // the damage at once whatever the duration (never added), each player's element
        // share through the magic-resistance multiplier with the cut cleared per player
        Ability::DamageArmour => {
            if on_self {
                out.caster = self_add(caster);
            } else if area {
                out.players = players
                    .iter_mut()
                    .map(|user| {
                        let amount = resisted(user);
                        let dmg =
                            mr_damage(amount, i32::from(user.character.mr), user.anti_magic, 0);
                        damage_user(user, dmg, dmg)
                    })
                    .collect();
                state.added = true;
            }
        }

        // instant area only; the monster gains each player's share,
        // capped at the active record's maximum; a lasting area spell does nothing.
        Ability::StealHp => {
            if area && !base_lasting {
                out.players = players
                    .iter_mut()
                    .map(|user| {
                        let amount = resisted(user);
                        let o = damage_user(user, amount, amount);
                        let gained = (i32::from(caster.monster.hp) + amount)
                            .min(i32::from(caster.monster.maxhp));
                        caster.monster.hp = gained as i16;
                        stolen(o, amount)
                    })
                    .collect();
                state.added = true;
            }
        }

        // area only; the share less the element resistance.
        Ability::Energy => {
            if area {
                if !base_lasting {
                    out.players = players
                        .iter_mut()
                        .map(|user| {
                            let amount = resisted(user);
                            user.character.energy =
                                (i32::from(user.character.energy) + amount) as u16;
                            Outcome::Applied {
                                amount,
                                knocked_out: false,
                            }
                        })
                        .collect();
                } else {
                    out.players = area_add(players, state);
                }
                state.added = true;
            }
        }

        // Self, Self or Single and Any without a duration; nothing is
        // set on the summon; the area kinds do nothing.
        Ability::Summon => {
            if matches!(
                kind,
                TargetKind::OnSelf | TargetKind::SelfOrSingle | TargetKind::Any
            ) && !base_lasting
            {
                out.caster = Outcome::Summon(roll, SummonDisposition::Plain);
            }
        }

        // the caster heals by the undivided roll up to the active
        // record's maximum; the players by the share up to theirs (no floor).
        Ability::Heal => {
            if on_self {
                if cast.duration != 0 {
                    out.caster = self_add(caster);
                } else {
                    let hp = i32::from(caster.monster.hp);
                    let maxhp = i32::from(caster.monster.maxhp);
                    let heal = if hp + roll > maxhp { maxhp - hp } else { roll };
                    caster.monster.hp = (hp + heal) as i16;
                    out.caster = Outcome::Applied {
                        amount: heal,
                        knocked_out: false,
                    };
                    state.added = true;
                }
            } else if area {
                if !base_lasting {
                    out.players = players
                        .iter_mut()
                        .map(|user| {
                            let hp = i32::from(user.character.hp);
                            let maxhp = i32::from(user.character.maxhp);
                            let heal = if hp + value > maxhp {
                                maxhp - hp
                            } else {
                                value
                            };
                            user.character.hp = (hp + heal) as i16;
                            Outcome::Applied {
                                amount: heal,
                                knocked_out: false,
                            }
                        })
                        .collect();
                } else {
                    out.players = area_add(players, state);
                }
                state.added = true;
            }
        }

        // area only (a monster has no mana).
        Ability::HealMana => {
            if area {
                if !base_lasting {
                    out.players = players
                        .iter_mut()
                        .map(|user| {
                            let heal = mana_gain(user.character, value);
                            user.character.mana = (i32::from(user.character.mana) + heal) as i16;
                            Outcome::Applied {
                                amount: heal,
                                knocked_out: false,
                            }
                        })
                        .collect();
                } else {
                    out.players = area_add(players, state);
                }
                state.added = true;
            }
        }

        // the self kinds get a lasting effect only (no poison set);
        // every marked player without Poison Immunity is raised to the share, and the
        // lasting form then adds the spell to everyone marked, immune or not.
        Ability::Poison => {
            if on_self {
                out.caster = self_add(caster);
            } else if area {
                let raise = |user: &mut UserTarget<'_, '_>| {
                    if user.poison_immune {
                        return false;
                    }
                    if i32::from(user.character.poison) < value {
                        user.character.poison = value as i16;
                    }
                    true
                };
                if !base_lasting {
                    out.players = players
                        .iter_mut()
                        .map(|user| {
                            if raise(user) {
                                Outcome::Applied {
                                    amount: value,
                                    knocked_out: false,
                                }
                            } else {
                                Outcome::Immune
                            }
                        })
                        .collect();
                } else {
                    for user in players.iter_mut() {
                        raise(user);
                    }
                    out.players = area_add(players, state);
                }
                state.added = true;
            }
        }

        // Self, Self or Single and Any (not Monster), whatever the
        // duration: the caster's poison falls by the undivided roll, never below 0, and the
        // message names the share (sic). The area kinds skip the immune (sic) and have no
        // floor.
        Ability::CurePoison => {
            if matches!(
                kind,
                TargetKind::OnSelf | TargetKind::SelfOrSingle | TargetKind::Any
            ) {
                caster.monster.poison = (i32::from(caster.monster.poison) - roll).max(0) as i16;
                out.caster = Outcome::Applied {
                    amount: value,
                    knocked_out: false,
                };
                state.added = true;
            } else if area {
                if !base_lasting {
                    out.players = players
                        .iter_mut()
                        .map(|user| {
                            if user.poison_immune {
                                return Outcome::Immune;
                            }
                            user.character.poison =
                                (i32::from(user.character.poison) - value) as i16;
                            Outcome::Applied {
                                amount: value,
                                knocked_out: false,
                            }
                        })
                        .collect();
                } else {
                    out.players = area_add(players, state);
                }
                state.added = true;
            }
        }

        // no effect when a monster casts them on the room.
        ability if area_no_effect(ability) => {}

        // every other ability is a lasting effect on the caster (the
        // self kinds) or on the marked players (the area kinds); Item and Monster or Single
        // do nothing.
        _ => {
            if on_self {
                out.caster = self_add(caster);
            } else if area && base_lasting {
                out.players = area_add(players, state);
                state.added = true;
            }
        }
    }
    out
}

/// The no-effect list of `monster_cast_area`: the markers, and the
/// abilities a monster's area cast drops (Hunger, Thirst, the stat alters, Dispel magic,
/// Freedom, Remove Curse, Text Block, Remove spell and Terminate Spell).
pub fn area_no_effect(ability: Ability) -> bool {
    matches!(
        ability,
        Ability::None
            | Ability::Enslave
            | Ability::Hunger
            | Ability::Thirst
            | Ability::KillDead
            | Ability::DetectMagic
            | Ability::Intellect
            | Ability::Wisdom
            | Ability::Strength
            | Ability::Health
            | Ability::Agility
            | Ability::Charm
            | Ability::EvilInCombat
            | Ability::DispelMagic
            | Ability::AffectsAnimal
            | Ability::Freedom
            | Ability::RemoveCurse
            | Ability::Quality
            | Ability::Good
            | Ability::Evil
            | Ability::ConfuseMsg
            | Ability::AffectsLiving
            | Ability::NonLiving
            | Ability::NotGood
            | Ability::NotEvil
            | Ability::Neutral
            | Ability::NotNeutral
            | Ability::AutoUse
            | Ability::DescMsg
            | Ability::StartMsg
            | Ability::RemoveSpell
            | Ability::TextBlock
            | Ability::TermSpell
    )
}

/// Applies every slot of a monster's area cast in record order, stopping at a refused
/// lasting add ([`MonsterCastState::refused`]), where the game ends the cast. The pre-pass
/// ([`apply_monster_area_remove_pass`]) is the engine's to run first, with the unmarked
/// players taken out of `players`.
pub fn apply_monster_cast_area_spell(
    effects: &SpellEffects,
    cast: &MonsterCast<'_>,
    caster: &mut MonsterCaster<'_>,
    players: &mut [UserTarget<'_, '_>],
    state: &mut MonsterCastState,
) -> Vec<AreaOutcome> {
    let mut outs = Vec::new();
    for slot in effects.iter() {
        outs.push(apply_monster_cast_area(slot, cast, caster, players, state));
        if state.refused {
            break;
        }
    }
    outs
}

// ---------------------------------------------------------------------------------------
// cast_self: a monster's spell on itself.

/// A monster's spell on itself (`cast_self`): no cast
/// roll, no resistance, no energy. `value` is the roll ([`roll_monster_amount`] at the
/// "spells after combat" slot's level). A lasting spell goes into the monster's slots for
/// the base duration and then every "Poison" slot adds its value to the poison (no maximum,
/// no immunity check); an instant spell summons the known monster the slot's value names
/// for every
/// "Summon" slot, with nothing set on it. Nothing else happens.
pub fn apply_monster_self_spell(
    spell: &Spell,
    value: i32,
    caster: &mut MonsterCaster<'_>,
) -> Vec<Outcome> {
    let effects = SpellEffects::from_record(spell);
    let mut outs = Vec::new();
    if spell.duration != 0 {
        outs.push(Outcome::Lasting(add_spell_to_monster(
            &mut caster.monster.spells,
            spell.number,
            i32::from(spell.duration),
            value,
        )));
        for slot in effects.iter() {
            if slot.ability == Ability::Poison {
                let abval = slot.amount(value);
                caster.monster.poison = (i32::from(caster.monster.poison) + abval) as i16;
                outs.push(Outcome::Applied {
                    amount: abval,
                    knocked_out: false,
                });
            }
        }
    } else {
        for slot in effects.iter() {
            if slot.ability == Ability::Summon {
                outs.push(Outcome::Summon(
                    slot.amount(value),
                    SummonDisposition::Plain,
                ));
            }
        }
    }
    outs
}

// ---------------------------------------------------------------------------------------
// room_cast_on_monster: a room's spell on a monster.

/// The one flag `room_cast_on_monster` keeps across casts: `spelladded` is never cleared
/// between monster casts in the game (sic); only `room_cast_on_user` resets it. The engine
/// owns it and decides whether to reproduce that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoomMonsterState {
    /// An effect has been applied this cast (or an earlier one, see above): the lasting
    /// blocks and the "Poison" add are skipped.
    pub spelladded: bool,
}

/// The three kinds `room_cast_on_monster` reaches a monster with ("MON3" in the census):
/// Monster, Any, Monster or Single.
fn monster_kind(kind: TargetKind) -> bool {
    matches!(
        kind,
        TargetKind::Monster | TargetKind::Any | TargetKind::MonsterOrSingle
    )
}

/// The no-effect list of `room_cast_on_monster`.
pub fn room_monster_no_effect(ability: Ability) -> bool {
    matches!(
        ability,
        Ability::None
            | Ability::UserLight
            | Ability::RoomLight
            | Ability::Hunger
            | Ability::Thirst
            | Ability::KillDead
            | Ability::DetectMagic
            | Ability::EvilInCombat
            | Ability::BlindingLight
            | Ability::AffectsAnimal
            | Ability::Quality
            | Ability::Good
            | Ability::Evil
            | Ability::ConfuseMsg
            | Ability::AffectsLiving
            | Ability::NonLiving
            | Ability::NotGood
            | Ability::NotEvil
            | Ability::Neutral
            | Ability::NotNeutral
            | Ability::AutoUse
            | Ability::DescMsg
            | Ability::StartMsg
            | Ability::RemoveSpell
            | Ability::NonMagical
            | Ability::TextBlock
            | Ability::HealMana
            | Ability::TermSpell
    )
}

/// What one slot did besides its outcome, for the spell-level loop.
struct RoomSlot {
    out: Outcome,
    /// `check_kill_monster` ran and the monster is at or below 0 hit points.
    killed: bool,
    /// The lasting block ran: the slot loop then goes on at slot 6 (sic).
    clobbered: bool,
}

/// Applies one slot of a room's spell to a monster (`room_cast_on_monster`). `power` is
/// [`room_monster_power`]; the slot's own value replaces it, and either is held to the
/// spell's `maxval`. `charmres` is [`room_charm_resistance`]. Only the Monster, Any and
/// Monster or Single kinds reach the monster. The lasting block (every ability the engine
/// does not list, and "Heal", which never heals
/// at once) renews the spell where it is and writes it into every free slot (sic) with the
/// base duration, once while `spelladded` is clear; every applied effect sets `spelladded`.
pub fn apply_room_on_monster(
    slot: &EffectSlot,
    spell: &Spell,
    power: i32,
    target: &mut MonsterTarget<'_>,
    charmres: i32,
    state: &mut RoomMonsterState,
) -> Outcome {
    room_slot(slot, spell, power, target, charmres, state).out
}

fn room_slot(
    slot: &EffectSlot,
    spell: &Spell,
    power: i32,
    target: &mut MonsterTarget<'_>,
    charmres: i32,
    state: &mut RoomMonsterState,
) -> RoomSlot {
    let maxval = i32::from(spell.maxval);
    let amount = slot.amount(power).min(maxval);
    let mon3 = monster_kind(spell.target);
    let mut slot_out = RoomSlot {
        out: Outcome::NoEffect,
        killed: false,
        clobbered: false,
    };
    if !mon3 {
        return slot_out;
    }
    let hp = i32::from(target.monster.hp);

    slot_out.out = match slot.ability {
        // straight off the hit points, no caster bonus.
        Ability::DamageNoArmour => room_hit(target, state, &mut slot_out, amount),

        // to -2, whatever the duration.
        Ability::Slay => room_hit(target, state, &mut slot_out, hp + 2),

        // the linear `150 - charmres` percent, at least 25, held to
        // the spell's maximum.
        Ability::DamageArmour => {
            let dmg = room_mr_damage(amount, charmres).min(maxval);
            room_hit(target, state, &mut slot_out, dmg)
        }

        // instant only; a lasting Energy is dropped.
        Ability::Energy => {
            state.spelladded = true;
            if spell.duration == 0 {
                target.monster.energy = (i32::from(target.monster.energy) + amount) as u16;
                Outcome::Applied {
                    amount,
                    knocked_out: false,
                }
            } else {
                Outcome::NoEffect
            }
        }

        // nothing but the flag.
        Ability::Summon | Ability::CurePoison => {
            state.spelladded = true;
            Outcome::NoEffect
        }

        // the immune are skipped (and the flag stays); raised to the
        // amount; a lasting spell also hits the hit points at once with no kill check, and
        // the spell is recorded through [`add_spell_to_monster`] (first free slot) for
        // the base duration, lasting or not, once.
        Ability::Poison => {
            if target.poison_immune {
                Outcome::Immune
            } else {
                if i32::from(target.monster.poison) < amount {
                    target.monster.poison = amount as i16;
                }
                if spell.duration != 0 {
                    target.monster.hp = (hp - amount) as i16;
                }
                let out = if state.spelladded {
                    Outcome::Applied {
                        amount,
                        knocked_out: false,
                    }
                } else {
                    Outcome::Lasting(add_spell_to_monster(
                        &mut target.monster.spells,
                        spell.number,
                        i32::from(spell.duration),
                        amount,
                    ))
                };
                state.spelladded = true;
                out
            }
        }

        ability if room_monster_no_effect(ability) => Outcome::NoEffect,

        // every unlisted ability and "Heal": the lasting block.
        _ => {
            if state.spelladded {
                Outcome::NoEffect
            } else {
                slot_out.clobbered = true;
                state.spelladded = true;
                Outcome::Lasting(write_every_free_slot(target.monster, spell, amount))
            }
        }
    };
    slot_out
}

/// `hp -= dmg` on the monster: it is killed when its hit points are at or below 0
/// afterwards (`check_kill_monster`).
fn room_hit(
    target: &mut MonsterTarget<'_>,
    state: &mut RoomMonsterState,
    slot_out: &mut RoomSlot,
    dmg: i32,
) -> Outcome {
    let after = i32::from(target.monster.hp) - dmg;
    target.monster.hp = after as i16;
    slot_out.killed = after <= 0;
    state.spelladded = true;
    Outcome::Applied {
        amount: dmg,
        knocked_out: false,
    }
}

/// The lasting block of `room_cast_on_monster`: the slot already holding the spell is
/// renewed with the base duration and the amount, and then every free
/// slot is written with them (sic). Reports the renewed slot, else the first slot written,
/// else `Full`.
fn write_every_free_slot(monster: &mut ActiveMonster, spell: &Spell, amount: i32) -> SlotOutcome {
    let mut renewed = None;
    let mut added = None;
    for (i, s) in monster.spells.iter_mut().enumerate() {
        if s.spell == spell.number {
            s.rounds = spell.duration;
            s.value = amount as i16;
            renewed.get_or_insert(i);
        }
    }
    for (i, s) in monster.spells.iter_mut().enumerate() {
        if s.spell == SpellNo(0) {
            s.spell = spell.number;
            s.rounds = spell.duration;
            s.value = amount as i16;
            added.get_or_insert(i);
        }
    }
    match (renewed, added) {
        (Some(i), _) => SlotOutcome::Renewed(i),
        (None, Some(i)) => SlotOutcome::Added(i),
        (None, None) => SlotOutcome::Full,
    }
}

/// Applies a room's spell to a monster slot by slot (`room_cast_on_monster`), over the ten
/// record slots (empty ones do nothing) until the monster is killed by a damage slot, with
/// the game's skip: after the lasting block the next slot is slot 6, whatever slot it was
/// on, so the slots between
/// are skipped or, from slot 6 on, applied a second time (sic). `power` is
/// [`room_monster_power`]. Returns `(slot index, outcome)` for every slot applied, in order.
pub fn apply_room_on_monster_spell(
    spell: &Spell,
    power: i32,
    target: &mut MonsterTarget<'_>,
    state: &mut RoomMonsterState,
) -> Vec<(usize, Outcome)> {
    let charmres = room_charm_resistance(spell, target.known);
    let mut outs = Vec::new();
    let mut i = 0;
    while i < NRECABILS {
        let (ability, value) = spell.abilities[i];
        let slot = EffectSlot {
            ability,
            effect: SpellEffect::from_slot(ability, value),
            value,
        };
        let r = room_slot(&slot, spell, power, target, charmres, state);
        outs.push((i, r.out));
        if r.killed {
            break;
        }
        if r.clobbered {
            i = NMONSPELLS;
        }
        i += 1;
    }
    outs
}
