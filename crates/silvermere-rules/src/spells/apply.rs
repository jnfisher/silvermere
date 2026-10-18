//! Applying one spell effect to one target, and the per-round and end-of-spell effects.
//!
//! What each ability does in the player-cast engines (`cast_no_target`, `cast_user_target`,
//! `cast_monster_target`, `cast_item_target`) and in the room-on-player engine
//! (`room_cast_on_user`), per `census.md` (T-049a). The monster-cast engines and
//! `room_cast_on_monster` are T-049b.
//!
//! The engine rolls the strength ([`super::roll_amount`]), divides it
//! ([`super::per_target_share`]), decides who is a valid target and who resisted
//! ([`super::resist`]), and then calls [`apply_spell`] (or [`apply_effect`] per slot) for
//! each target with a [`Cast`] describing which engine path it is on. Every number the
//! game changes on the target record is changed here; everything that needs the world
//! (a monster to generate, an item to erase, a door to force, a text block to show, a
//! teleport) comes back as an [`Outcome`] for the engine to perform. No message is built
//! here.

use silvermere_types::enums::{Ability, Alignment, Element, MonsterType};
use silvermere_types::flags::ActiveMonsterFlags;
use silvermere_types::ids::{ItemNo, SpellNo};
use silvermere_types::records::{ActiveMonster, Character, Item, KnownMonster, Spell};
use silvermere_types::sizes::NSPELLCAST;

use super::effect::{EffectSlot, SpellEffect, SpellEffects, Stat, TeleportPart};
use super::resist::{apply_element_resistance, monster_mr_damage, mr_damage, room_mr_damage};
use super::{
    CasterLevel, CharacterSlots, SlotOutcome, TargetKind, TargetKindExt, add_spell_damage_bonus,
    add_spell_to_character, add_spell_to_monster, duration, scale_spell_amount,
};
use crate::abilities::{item_ability_value, item_has_ability, spell_has_ability};
use crate::rng::Rng;

/// What the effect code needs of a player caster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasterStats<'a> {
    /// The caster's first name: what a summoned or enslaved monster follows.
    pub firstname: &'a str,
    /// The caster's level.
    pub level: u16,
    /// The caster's "Alter spell damage" total.
    pub spelldamage_pct: i32,
    /// The caster's "Alter spell duration" total.
    pub spellduration_pct: i32,
    /// The caster's spellcasting, for "Open Door".
    pub spellcast: i16,
}

impl CasterStats<'_> {
    fn level_for_duration(&self) -> CasterLevel {
        CasterLevel {
            level: self.level,
            spellduration_pct: self.spellduration_pct,
        }
    }
}

/// Which engine path a cast is on; the arithmetic differs between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastPath {
    /// `cast_no_target` on the caster himself (Self, Self or Single, Any).
    OnSelf,
    /// `cast_no_target` on one target of an area; `power` is that target's share,
    /// `undivided` the whole roll, which Poison and Cure Poison use on monsters.
    Area {
        /// The undivided strength of the cast.
        undivided: i32,
    },
    /// `cast_user_target`, `cast_monster_target` or `cast_item_target`: one named target;
    /// `power` is the roll after the target's element resistance.
    Single,
    /// `room_cast_on_user`: a room's spell on a player; no caster, `power` is the
    /// roll after element resistance.
    Room,
}

/// One cast in progress, as the effect code sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cast<'a> {
    /// The spell record.
    pub spell: &'a Spell,
    /// The caster; `None` for a room.
    pub caster: Option<&'a CasterStats<'a>>,
    /// The engine path.
    pub path: CastPath,
    /// The strength for this target (see [`CastPath`]).
    pub power: i32,
}

impl Cast<'_> {
    /// The spell has a duration: it lasts and goes into a slot.
    pub fn lasting(&self) -> bool {
        self.spell.duration != 0
    }

    fn spelldamage_pct(&self) -> i32 {
        self.caster.map_or(0, |c| c.spelldamage_pct)
    }

    fn rounds(&self, rng: &mut dyn Rng) -> i32 {
        duration(
            self.spell,
            self.caster.map(CasterStats::level_for_duration),
            rng,
        )
    }
}

/// State carried across the slots and targets of one cast: whether the lasting add was
/// made, the chain cast and its chance, the teleport destination, and the values the area
/// loops of `cast_no_target` carry from one target to the next.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CastState {
    /// A lasting slot entry has been made for this cast: later slots do not add
    /// the spell again, but a stat alter still raises its stat.
    pub told: bool,
    /// "Cast on ending" of an instant cast, to cast after the slots.
    pub chain: Option<SpellNo>,
    /// Its chance in percent, default 100.
    pub chain_chance: i32,
    /// The teleport destination, both halves needed.
    pub teleport_room: Option<i32>,
    /// See `teleport_room`.
    pub teleport_map: Option<i32>,
    /// The previous area target's "Inflict Damage on Armour" result, which the player
    /// loop of `cast_no_target` does not clear (sic).
    pub area_armour_carry: i32,
    /// The running `amount` of the area "Inflict Damage on Armour" loops: the last player's
    /// element share, then each monster's scaled amount, compounding (sic).
    pub area_armour_amount: Option<i32>,
    /// A monster resisted an area "Steal Hit Points": the share is 0 for every later
    /// monster (sic).
    pub area_stealhp_zeroed: bool,
    /// An "Alter HP" slot took (the add was made, or an earlier slot told): the raise the
    /// engine makes with [`raise_hp`] after the slot's lines. The game's add shows the cast
    /// with its prompt before the maximum and current hit points rise, so that prompt shows
    /// the old figures.
    pub alter_hp: Option<i32>,
}

impl CastState {
    /// A fresh cast.
    pub fn new() -> Self {
        Self {
            chain_chance: 100,
            ..Self::default()
        }
    }

    /// The teleport destination when both halves were set.
    pub fn teleport(&self) -> Option<(i32, i32)> {
        Some((self.teleport_room?, self.teleport_map?))
    }
}

/// A player as a spell target, with the facts the engine resolves around the record.
pub struct UserTarget<'a, 'w> {
    /// The record; hit points, mana, poison, hunger, thirst, energy, stats, slots and the
    /// spellbook change here.
    pub character: &'a mut Character,
    /// The target is the caster.
    pub is_caster: bool,
    /// The target has Anti Magic ([`AbilityLookup::has`](crate::abilities::AbilityLookup::has)).
    pub anti_magic: bool,
    /// The target has Poison Immunity.
    pub poison_immune: bool,
    /// [`super::resist::element_resistance`] of this target for the spell, in percent.
    pub element_resist_pct: i32,
    /// The hit points at which a character dies (`deathhp`), for "Slay".
    pub deathhp: i16,
    /// The caster stands in a colliseum while arena combat is safe: "Shatter" spares the
    /// weapon.
    pub arena_safe: bool,
    /// Spell records by number, for the active-spell slots.
    pub spells: &'a dyn Fn(SpellNo) -> Option<&'w Spell>,
    /// Item records by number, for the worn items and the weapon.
    pub items: &'a dyn Fn(ItemNo) -> Option<&'w Item>,
}

/// A monster as a spell target.
pub struct MonsterTarget<'a> {
    /// The active record; hit points, poison, energy, following, friendly, flags and slots
    /// change here.
    pub monster: &'a mut ActiveMonster,
    /// Its known-monster record.
    pub known: &'a KnownMonster,
    /// Its magic resistance ([`super::resist::monster_magic_resistance`]).
    pub magic_resistance: i32,
    /// The monster has Anti Magic.
    pub anti_magic: bool,
    /// The monster has Poison Immunity.
    pub poison_immune: bool,
    /// Its element resistance for the spell, in percent.
    pub element_resist_pct: i32,
}

/// An item as a spell target (`cast_item_target`).
pub struct ItemTarget<'a> {
    /// The item.
    pub item: &'a Item,
}

/// Who a spell effect is applied to.
pub enum Target<'a, 'w> {
    /// A player.
    User(UserTarget<'a, 'w>),
    /// A monster.
    Monster(MonsterTarget<'a>),
    /// An item.
    Item(ItemTarget<'a>),
    /// Nobody: a room-wide effect with no record to change.
    None,
}

/// What a summoned monster does once generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonDisposition {
    /// Follows the caster, friendly and charmed.
    CharmedByCaster,
    /// Follows the target player, hostile.
    HuntsTarget,
    /// Hunts the target monster.
    HuntsMonster,
    /// Follows the target player, hostile, generated with the casting monster's generation
    /// index as the index range: the same as `HuntsTarget` unless the
    /// number is 0, when the range decides the random pick.
    HuntsTargetByCasterIndex,
    /// Nothing is set on it: it follows nobody and is no friend.
    Plain,
}

/// The result of applying one slot to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The effect happened; `amount` is what the game shows (damage dealt, points
    /// healed, ...). `knocked_out` is the player knock-out rule: hit points fell to 0 or
    /// below from above 0 (`knock_out_user`); dead is for the engine to decide from `hp`.
    Applied {
        /// The amount the message names.
        amount: i32,
        /// The target went down.
        knocked_out: bool,
    },
    /// The target lost `amount` hit points and the caster gains them, capped at the
    /// caster's maximum.
    StolenHp {
        /// The amount stolen.
        amount: i32,
    },
    /// The spell went into (or was renewed in) the target's slot table.
    Lasting(SlotOutcome),
    /// The target resisted within the effect itself (an area "Steal Hit Points" on a monster).
    Resisted,
    /// The target is immune (poison immunity, an undead-only spell on the living).
    Immune,
    /// Nothing happens for this slot on this target.
    NoEffect,
    /// Silly: the spell has no meaning for this kind of target or duration.
    Silly,
    /// Half of a teleport destination was set; see [`CastState::teleport`].
    Teleport(TeleportPart, i32),
    /// Run text block `n` for the target.
    TextBlock(i32),
    /// Erase every copy of item `n` from the caster's room.
    ClearItem(i32),
    /// Move the room's contents to room `n`.
    Scatter(i32),
    /// Generate known monster `n` in the room with this disposition.
    Summon(i32, SummonDisposition),
    /// Force the room's doors and gates with this chance (`bash_room_exits`).
    OpenDoors {
        /// `(spellcast - diff)/2`, held to 110 by the engine's door loop.
        chance: i32,
    },
    /// "Lore": whether the item's hidden descriptions are revealed.
    Lore {
        /// The item's level is at most the amount.
        revealed: bool,
    },
    /// "Detect Magic": the item's "Magical" value, which the engine bands.
    DetectMagic {
        /// The item's Magical value ([`item_ability_value`]).
        magical: i32,
    },
    /// The target's weapon broke: `weapon` was cleared; the engine removes the item from
    /// the inventory unless `destroyed` is false (a safe arena only unwields it).
    Shattered {
        /// The item that broke.
        item: ItemNo,
        /// Whether it leaves the inventory.
        destroyed: bool,
    },
    /// "Remove Curse": the worn slots to strip (`remove_armour`, forced).
    RemoveCurse {
        /// Indexes into the worn slots.
        worn_slots: Vec<usize>,
    },
    /// Spells ended on the target (Freedom, Dispel Magic, Cure Poison, Remove spell):
    /// their slots are cleared; each ending (`end_player_spell`: the ending line and prompt,
    /// the undo, the chain cast, the stats) is the engine's to run, one spell after the other
    /// in this order, as in the game. A slot whose record is missing is only cleared and is
    /// not listed.
    Ended {
        /// The spells to end, in slot order.
        spells: Vec<EndedSpell>,
        /// "Cure Poison": the cut in the poison level, made after the endings (see
        /// [`PoisonCut`]).
        poison: Option<PoisonCut>,
    },
    /// The lasting spell is already active and may not be renewed: the cast fails.
    AlreadyActive,
}

/// Whether a hit of `dmg` took a player down: hit points at 0 or below, and above 0 before
/// (the area loops pass the undivided share).
pub fn knocks_out(hp_after: i32, dmg: i32) -> bool {
    hp_after <= 0 && hp_after + dmg > 0
}

/// Applies every slot of a spell to one target, in record order, keeping the cast state
/// across the slots. The engine calls it once per target with the same `state` so the
/// values the game carries between area targets carry here too.
pub fn apply_spell(
    effects: &SpellEffects,
    cast: &Cast<'_>,
    target: &mut Target<'_, '_>,
    state: &mut CastState,
    rng: &mut dyn Rng,
) -> Vec<Outcome> {
    effects
        .iter()
        .map(|slot| {
            let out = apply_effect(slot, cast, target, state, rng);
            if let Target::User(user) = &mut *target {
                if let Some(v) = state.alter_hp.take() {
                    raise_hp(user.character, v);
                }
                if let Outcome::Ended {
                    poison: Some(cut), ..
                } = out
                {
                    cut_poison(user.character, cut);
                }
            }
            out
        })
        .collect()
}

/// The "Alter HP" raise: the maximum and the current hit points rise by `amount`.
pub fn raise_hp(character: &mut Character, amount: i32) {
    character.maxhp = (i32::from(character.maxhp) + amount) as i16;
    character.hp = (i32::from(character.hp) + amount) as i16;
}

/// Applies one slot to one target.
pub fn apply_effect(
    slot: &EffectSlot,
    cast: &Cast<'_>,
    target: &mut Target<'_, '_>,
    state: &mut CastState,
    rng: &mut dyn Rng,
) -> Outcome {
    let amount = slot.amount(cast.power);
    match slot.effect {
        // The slots every engine handles the same way, whatever the target.
        SpellEffect::Teleport(part) => {
            match part {
                TeleportPart::Room => state.teleport_room = Some(amount),
                TeleportPart::Map => state.teleport_map = Some(amount),
            }
            Outcome::Teleport(part, amount)
        }
        SpellEffect::EndCast => {
            if !cast.lasting() {
                state.chain = Some(SpellNo(amount as i16));
            }
            Outcome::NoEffect
        }
        SpellEffect::EndCastChance => {
            state.chain_chance = amount;
            Outcome::NoEffect
        }
        SpellEffect::ClearItem => Outcome::ClearItem(amount),
        SpellEffect::Marker(_) | SpellEffect::Component | SpellEffect::Unknown(_) => {
            Outcome::NoEffect
        }
        _ => match target {
            Target::User(user) => apply_to_user(slot, amount, cast, user, state, rng),
            Target::Monster(monster) => apply_to_monster(slot, amount, cast, monster, state, rng),
            Target::Item(item) => apply_to_item(slot, amount, cast, item),
            Target::None => apply_to_nobody(slot, amount, cast),
        },
    }
}

/// The slots of `cast_no_target` and `cast_user_target` that act with no target record:
/// "Open Door" and "Text Block"; everything else is nothing without a target.
fn apply_to_nobody(slot: &EffectSlot, amount: i32, cast: &Cast<'_>) -> Outcome {
    match slot.effect {
        SpellEffect::TextBlock => Outcome::TextBlock(amount),
        SpellEffect::Scatter if cast.path == CastPath::Room => Outcome::Scatter(amount),
        SpellEffect::OpenDoor if matches!(cast.path, CastPath::OnSelf | CastPath::Area { .. }) => {
            open_door(cast)
        }
        _ => Outcome::NoEffect,
    }
}

/// "Open Door" (`cast_no_target`): instant only, with chance
/// `(spellcast - diff)/2` for `bash_room_exits`.
fn open_door(cast: &Cast<'_>) -> Outcome {
    if cast.lasting() {
        return Outcome::Silly;
    }
    let spellcast = cast.caster.map_or(0, |c| i32::from(c.spellcast));
    Outcome::OpenDoors {
        chance: (spellcast - i32::from(cast.spell.diff)) / 2,
    }
}

/// The lasting-spell add on a player, once per cast: [`super::duration`] and
/// [`super::add_spell_to_character`] (a room's add has no caster).
fn add_lasting_to_user(
    cast: &Cast<'_>,
    user: &mut UserTarget<'_, '_>,
    power: i32,
    renew: bool,
    state: &mut CastState,
    rng: &mut dyn Rng,
) -> Outcome {
    if state.told {
        return Outcome::NoEffect;
    }
    let rounds = cast.rounds(rng);
    let outcome = add_spell_to_character(
        CharacterSlots {
            spells: &mut user.character.spells,
            spellvals: &mut user.character.spellvals,
            spellrnds: &mut user.character.spellrnds,
        },
        cast.spell.number,
        rounds,
        power,
        renew,
    );
    state.told = true;
    Outcome::Lasting(outcome)
}

/// A lasting dose for an area target: the share less the target's element resistance
/// (`add_duration_spell_to_room`).
fn area_dose(cast: &Cast<'_>, power: i32, element_resist_pct: i32) -> i32 {
    match cast.path {
        CastPath::Area { .. } => apply_element_resistance(power, element_resist_pct),
        _ => power,
    }
}

fn apply_to_user(
    slot: &EffectSlot,
    amount: i32,
    cast: &Cast<'_>,
    user: &mut UserTarget<'_, '_>,
    state: &mut CastState,
    rng: &mut dyn Rng,
) -> Outcome {
    use CastPath as P;
    let path = cast.path;
    let lasting = cast.lasting();
    let kind = cast.spell.target;
    let area = matches!(path, P::Area { .. });
    // What the engines do for an ability they do not list: silly when instant, a slot entry
    // when lasting; the room engine only records the lasting one.
    let default_branch =
        |user: &mut UserTarget<'_, '_>, state: &mut CastState, rng: &mut dyn Rng| {
            if lasting {
                let dose = area_dose(cast, amount, user.element_resist_pct);
                add_lasting_to_user(cast, user, dose, true, state, rng)
            } else if path == P::Room {
                Outcome::NoEffect
            } else {
                Outcome::Silly
            }
        };
    if path == P::Room && !room_reaches(slot.effect, kind, lasting) {
        return Outcome::NoEffect;
    }
    // The element share of an area target.
    let share = |pct: i32| apply_element_resistance(amount, pct);
    let hp = i32::from(user.character.hp);
    let alive = hp > 0;

    match slot.effect {
        SpellEffect::TextBlock => Outcome::TextBlock(amount),
        SpellEffect::Scatter if path == P::Room => Outcome::Scatter(amount),
        SpellEffect::OpenDoor if matches!(path, P::OnSelf | P::Area { .. }) => open_door(cast),

        SpellEffect::Damage {
            ignore_armour: true,
        } => match path {
            P::OnSelf | P::Single => {
                if lasting {
                    // the unscaled value goes into the slot.
                    return add_lasting_to_user(cast, user, amount, true, state, rng);
                }
                // scaled by the caster's spell damage bonus.
                let dmg = scale_spell_amount(amount, cast.spelldamage_pct());
                damage_user(user, dmg, dmg)
            }
            P::Area { .. } => {
                // the caster is hit only by the inclusive kinds.
                if user.is_caster && !kind.area_includes_caster() {
                    return Outcome::NoEffect;
                }
                if lasting {
                    let dose = area_dose(cast, amount, user.element_resist_pct);
                    return add_lasting_to_user(cast, user, dose, true, state, rng);
                }
                // the element share, no spell damage bonus, and the
                // knock-out test with the undivided share (sic).
                let dmg = share(user.element_resist_pct);
                damage_user(user, dmg, amount)
            }
            P::Room => {
                if lasting {
                    return add_lasting_to_user(cast, user, amount, true, state, rng);
                }
                // no kill check, knocked out from alive only.
                room_damage_user(user, amount, alive)
            }
        },

        SpellEffect::Damage {
            ignore_armour: false,
        } => match path {
            P::OnSelf | P::Single => {
                // no duration test anywhere.
                let scaled = add_spell_damage_bonus(amount, cast.spelldamage_pct());
                let dmg = mr_damage(scaled, i32::from(user.character.mr), user.anti_magic, 0);
                damage_user(user, dmg, dmg)
            }
            P::Area { .. } => {
                // element share, the carried cut (sic), knock-out
                // with the undivided share (sic); the share feeds the monster loop.
                let scaled = share(user.element_resist_pct);
                state.area_armour_amount = Some(scaled);
                let dmg = mr_damage(
                    scaled,
                    i32::from(user.character.mr),
                    user.anti_magic,
                    state.area_armour_carry,
                );
                state.area_armour_carry = dmg;
                damage_user(user, dmg, amount)
            }
            P::Room => {
                let dmg = room_mr_damage(amount, i32::from(user.character.mr));
                room_damage_user(user, dmg, alive)
            }
        },

        SpellEffect::Slay => match path {
            P::Single | P::Room => {
                if lasting {
                    // silly by a caster; a room's acts only without a duration.
                    return if path == P::Room {
                        Outcome::NoEffect
                    } else {
                        Outcome::Silly
                    };
                }
                // to one below death, or to death from below 0.
                let deathhp = i32::from(user.deathhp);
                let value = if hp >= 0 { hp - deathhp + 1 } else { -deathhp };
                if path == P::Room {
                    room_damage_user(user, value, alive)
                } else {
                    damage_user(user, value, value)
                }
            }
            _ => default_branch(user, state, rng),
        },

        SpellEffect::StealHp => match path {
            P::OnSelf => Outcome::Silly,
            P::Single => {
                if lasting {
                    return Outcome::Silly;
                }
                let out = damage_user(user, amount, amount);
                stolen(out, amount)
            }
            P::Area { .. } => {
                if lasting {
                    return Outcome::Silly;
                }
                // the element share; knock-out with the undivided one.
                let dmg = share(user.element_resist_pct);
                let out = damage_user(user, dmg, amount);
                stolen(out, dmg)
            }
            // plain damage, nobody gains.
            P::Room => room_damage_user(user, amount, alive),
        },

        SpellEffect::Heal => {
            if lasting {
                let dose = area_dose(cast, amount, 0);
                return add_lasting_to_user(cast, user, dose, true, state, rng);
            }
            // up to the maximum: on oneself or a single target the gain may be negative, an
            // area's is floored at 0.
            let maxhp = i32::from(user.character.maxhp);
            let mut gain = if hp + amount > maxhp {
                maxhp - hp
            } else {
                amount
            };
            if area && gain < 0 {
                gain = 0;
            }
            user.character.hp = (hp + gain) as i16;
            Outcome::Applied {
                amount: gain,
                knocked_out: false,
            }
        }

        SpellEffect::HealMana => {
            if lasting {
                return add_lasting_to_user(cast, user, amount, true, state, rng);
            }
            // up to the maximum, then kept between 0 and it (an area's is not clamped).
            let mana = i32::from(user.character.mana);
            let maxmana = i32::from(user.character.maxmana);
            let gain = if mana + amount > maxmana {
                maxmana - mana
            } else {
                amount
            };
            let mut after = mana + gain;
            if !area && after < 0 {
                after = 0;
            }
            user.character.mana = after as i16;
            Outcome::Applied {
                amount: gain,
                knocked_out: false,
            }
        }

        SpellEffect::Poison => {
            // the immune are skipped.
            if user.poison_immune {
                return Outcome::Immune;
            }
            // raised to the amount if lower;
            // the lasting form raises it and adds the spell.
            let raised = i32::from(user.character.poison) < amount;
            if raised {
                user.character.poison = amount as i16;
            }
            if lasting {
                return add_lasting_to_user(cast, user, amount, true, state, rng);
            }
            Outcome::Applied {
                amount,
                knocked_out: false,
            }
        }

        SpellEffect::CurePoison => {
            if lasting {
                return add_lasting_to_user(cast, user, amount, true, state, rng);
            }
            // the first poison spell ends, then the level falls, never below 0; the area loop
            // has no floor (sic).
            Outcome::Ended {
                spells: terminate_user_poison_spell(user).into_iter().collect(),
                poison: Some(PoisonCut {
                    amount,
                    floor: !area,
                }),
            }
        }

        SpellEffect::Energy => {
            if lasting {
                let dose = area_dose(cast, amount, user.element_resist_pct);
                return add_lasting_to_user(cast, user, dose, true, state, rng);
            }
            // the amount, or an area target's element share.
            let gain = if area {
                share(user.element_resist_pct)
            } else {
                amount
            };
            user.character.energy = (i32::from(user.character.energy) + gain) as u16;
            Outcome::Applied {
                amount: gain,
                knocked_out: false,
            }
        }

        SpellEffect::Hunger | SpellEffect::Thirst => {
            if lasting {
                return add_lasting_to_user(cast, user, amount, true, state, rng);
            }
            if area {
                // the instant area form does nothing.
                return Outcome::NoEffect;
            }
            let field = if slot.effect == SpellEffect::Hunger {
                &mut user.character.hunger
            } else {
                &mut user.character.thirst
            };
            *field = (i32::from(*field) + amount) as i16;
            Outcome::Applied {
                amount,
                knocked_out: false,
            }
        }

        SpellEffect::Summon => match path {
            P::OnSelf => {
                if lasting {
                    return Outcome::Silly;
                }
                Outcome::Summon(amount, SummonDisposition::CharmedByCaster)
            }
            P::Single | P::Room => {
                if lasting && path == P::Single {
                    return Outcome::Silly;
                }
                Outcome::Summon(amount, SummonDisposition::HuntsTarget)
            }
            P::Area { .. } => Outcome::Silly,
        },

        SpellEffect::Shatter => match path {
            // a room's always destroys.
            P::Single | P::Room => shatter(user, amount, path == P::Room || !user.arena_safe),
            _ => default_branch(user, state, rng),
        },

        SpellEffect::RemoveCurse => {
            // on oneself (areas silly); on a named target only for Self or Single and Any
            // (areas silly; Single and Monster or Single do nothing); by a room for the self
            // kinds, Cursed items only.
            let self_kind = match path {
                P::OnSelf => true,
                P::Area { .. } => return Outcome::Silly,
                P::Single => {
                    if kind.is_area() {
                        return Outcome::Silly;
                    }
                    matches!(kind, TargetKind::SelfOrSingle | TargetKind::Any)
                }
                P::Room => kind.is_self(),
            };
            if !self_kind {
                return Outcome::NoEffect;
            }
            if lasting {
                return if path == P::Room {
                    Outcome::NoEffect
                } else {
                    Outcome::Silly
                };
            }
            let caster_level = if path == P::Room {
                None
            } else {
                cast.caster.map(|c| i32::from(c.level))
            };
            remove_curse(user, caster_level)
        }

        SpellEffect::Freedom => {
            if area {
                return Outcome::Silly;
            }
            if lasting {
                return match path {
                    P::OnSelf | P::Room => {
                        add_lasting_to_user(cast, user, amount, true, state, rng)
                    }
                    _ => Outcome::Silly,
                };
            }
            end_spells_where(user, |spell| {
                spell_has_ability(spell, Ability::HoldPerson)
                    || spell_has_ability(spell, Ability::Paralyze)
            })
        }

        SpellEffect::DispelMagic => {
            // the self kinds and Full Party Area; other areas are silly. The amount is an
            // ability number, -1 for all.
            if area && kind != TargetKind::FullParty {
                return Outcome::Silly;
            }
            if lasting {
                return if path == P::Room {
                    Outcome::NoEffect
                } else {
                    Outcome::Silly
                };
            }
            end_spells_where(user, |spell| {
                amount == -1 || spell_has_ability(spell, Ability::from_code(amount as u16))
            })
        }

        SpellEffect::RemoveSpell { chain } => {
            // The first pass of `cast_no_target` and
            // `cast_user_target`: spell number `amount` ends on the target.
            // The room engine has no such pass.
            if path == P::Room || slot.value == 0 {
                return Outcome::NoEffect;
            }
            let number = SpellNo(amount as i16);
            let Some(i) = user.character.spells.iter().position(|&s| s == number) else {
                return Outcome::NoEffect;
            };
            let power = i32::from(user.character.spellvals[i]);
            clear_slot(user.character, i);
            let spells = (user.spells)(number)
                .map(|_| EndedSpell {
                    number,
                    power,
                    chain,
                })
                .into_iter()
                .collect();
            Outcome::Ended {
                spells,
                poison: None,
            }
        }

        SpellEffect::StatAlter(stat) => stat_alter(cast, user, amount, state, rng, |c, v| {
            let field = match stat {
                Stat::Intellect => &mut c.intel,
                Stat::Wisdom => &mut c.wil,
                Stat::Strength => &mut c.str,
                Stat::Health => &mut c.hea,
                Stat::Agility => &mut c.agi,
                Stat::Charm => &mut c.cha,
            };
            *field = (i32::from(*field) + v) as u16;
        }),

        SpellEffect::AlterHp => {
            let mut raised = false;
            let out = stat_alter(cast, user, amount, state, rng, |_, _| raised = true);
            if raised {
                state.alter_hp = Some(amount);
            }
            out
        }

        SpellEffect::GiveSpell => match path {
            // lasting, single kinds only; the spell joins the
            // spellbook when the slot entry is made (or was already made this cast).
            P::Single => {
                if kind.is_area() {
                    return Outcome::Silly;
                }
                if !lasting {
                    return Outcome::Silly;
                }
                let out = add_lasting_to_user(cast, user, amount, false, state, rng);
                if matches!(
                    out,
                    Outcome::NoEffect | Outcome::Lasting(SlotOutcome::Added(_))
                ) || matches!(out, Outcome::Lasting(SlotOutcome::Renewed(_)))
                {
                    let given = SpellNo(amount as i16);
                    if !user.character.spellbook.contains(&given) {
                        user.character.spellbook.push(given);
                    }
                    out
                } else {
                    Outcome::AlreadyActive
                }
            }
            _ => default_branch(user, state, rng),
        },

        // Meaningless on a player; nothing by a room.
        SpellEffect::Enslave => {
            if path == P::Room {
                Outcome::NoEffect
            } else {
                Outcome::Silly
            }
        }

        // Markers on a living target, item effects on a player, and the monster-only gate.
        SpellEffect::DetectMagic | SpellEffect::KillDead | SpellEffect::Guarded => {
            Outcome::NoEffect
        }
        SpellEffect::Lore | SpellEffect::Scatter | SpellEffect::OpenDoor => {
            default_branch(user, state, rng)
        }
        SpellEffect::Lasting(_) => default_branch(user, state, rng),

        SpellEffect::Teleport(_)
        | SpellEffect::EndCast
        | SpellEffect::EndCastChance
        | SpellEffect::ClearItem
        | SpellEffect::Marker(_)
        | SpellEffect::Component
        | SpellEffect::Unknown(_) => Outcome::NoEffect,
    }
}

/// Whether a room spell on a player (`room_cast_on_user`) lets an effect act for a spell of
/// this kind and duration: the instant player effects and their lasting forms only for the
/// kinds that name one player (Single, Self or Single, Any, Monster or Single; the others do
/// nothing, not even a lasting add); "Summon", "Steal Hit Points" and "Freedom" only without
/// a duration; the lasting stat alters for those kinds and Self. Every other effect is
/// decided in its arm.
fn room_reaches(effect: SpellEffect, kind: TargetKind, lasting: bool) -> bool {
    let one = matches!(
        kind,
        TargetKind::Single
            | TargetKind::SelfOrSingle
            | TargetKind::Any
            | TargetKind::MonsterOrSingle
    );
    match effect {
        SpellEffect::Damage { .. }
        | SpellEffect::Slay
        | SpellEffect::Energy
        | SpellEffect::Hunger
        | SpellEffect::Thirst
        | SpellEffect::Heal
        | SpellEffect::HealMana
        | SpellEffect::Poison
        | SpellEffect::CurePoison
        | SpellEffect::DispelMagic => one,
        SpellEffect::StealHp | SpellEffect::Summon | SpellEffect::Freedom => one && !lasting,
        SpellEffect::StatAlter(_) | SpellEffect::AlterHp => one || kind == TargetKind::OnSelf,
        _ => true,
    }
}

/// `hp -= dmg` with the player knock-out rule tested on `ko_amount` (the engines pass the
/// damage for self and single targets and the undivided share for area targets).
pub(super) fn damage_user(user: &mut UserTarget<'_, '_>, dmg: i32, ko_amount: i32) -> Outcome {
    let after = i32::from(user.character.hp) - dmg;
    user.character.hp = after as i16;
    Outcome::Applied {
        amount: dmg,
        knocked_out: knocks_out(after, ko_amount),
    }
}

/// The room engine's damage: no kill check, knocked out only when alive before.
fn room_damage_user(user: &mut UserTarget<'_, '_>, dmg: i32, alive: bool) -> Outcome {
    let after = i32::from(user.character.hp) - dmg;
    user.character.hp = after as i16;
    Outcome::Applied {
        amount: dmg,
        knocked_out: after <= 0 && alive,
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

/// The lasting stat alters: lasting only (instant and areas are silly, nothing by a room);
/// the slot entry is made without renewal, and the stat rises when it was made or an
/// earlier slot already made it.
fn stat_alter(
    cast: &Cast<'_>,
    user: &mut UserTarget<'_, '_>,
    amount: i32,
    state: &mut CastState,
    rng: &mut dyn Rng,
    raise: impl FnOnce(&mut Character, i32),
) -> Outcome {
    let silly = if cast.path == CastPath::Room {
        Outcome::NoEffect
    } else {
        Outcome::Silly
    };
    if matches!(cast.path, CastPath::Area { .. }) && cast.spell.target.is_area() {
        return silly;
    }
    if !cast.lasting() {
        return silly;
    }
    let out = add_lasting_to_user(cast, user, amount, false, state, rng);
    match out {
        Outcome::NoEffect | Outcome::Lasting(SlotOutcome::Added(_)) => {
            raise(user.character, amount);
            out
        }
        Outcome::Lasting(_) => Outcome::AlreadyActive,
        other => other,
    }
}

/// "Shatter": the wielded weapon breaks when the amount is above its Quality; a safe arena
/// only unwields it.
fn shatter(user: &mut UserTarget<'_, '_>, amount: i32, destroyed: bool) -> Outcome {
    let weapon = user.character.weapon;
    if weapon == ItemNo(0) {
        return Outcome::NoEffect;
    }
    let Some(item) = (user.items)(weapon) else {
        return Outcome::NoEffect;
    };
    if amount > item_ability_value(item, Ability::Quality) {
        user.character.weapon = ItemNo(0);
        Outcome::Shattered {
            item: weapon,
            destroyed,
        }
    } else {
        Outcome::NoEffect
    }
}

/// "Remove Curse": every worn item that is Cursed, or whose Major Curse level is below the
/// caster's level, comes off. The game's extra pass over the wielded weapon is broken
/// (sic) and is not reproduced.
fn remove_curse(user: &mut UserTarget<'_, '_>, caster_level: Option<i32>) -> Outcome {
    let worn_slots = user
        .character
        .worn
        .iter()
        .enumerate()
        .filter(|(_, n)| **n != ItemNo(0))
        .filter_map(|(i, &n)| (user.items)(n).map(|item| (i, item)))
        .filter(|(_, item)| {
            let curselvl = item_ability_value(item, Ability::MajorCurse);
            item_has_ability(item, Ability::Cursed)
                || caster_level.is_some_and(|level| curselvl > 0 && level > curselvl)
        })
        .map(|(i, _)| i)
        .collect();
    Outcome::RemoveCurse { worn_slots }
}

pub(super) fn clear_slot(character: &mut Character, i: usize) {
    character.spells[i] = SpellNo(0);
    character.spellvals[i] = 0;
    character.spellrnds[i] = 0;
}

/// Ends every active spell matching `pick` (Freedom, Dispel Magic): the slot is cleared and
/// the spell is listed for its ending with `chain` set; a slot whose record is missing is
/// just cleared.
pub(super) fn end_spells_where(
    user: &mut UserTarget<'_, '_>,
    pick: impl Fn(&Spell) -> bool,
) -> Outcome {
    let mut spells = Vec::new();
    for i in 0..NSPELLCAST {
        let number = user.character.spells[i];
        if number == SpellNo(0) {
            continue;
        }
        match (user.spells)(number) {
            Some(spell) if pick(spell) => {
                let power = i32::from(user.character.spellvals[i]);
                clear_slot(user.character, i);
                spells.push(EndedSpell {
                    number,
                    power,
                    chain: true,
                });
            }
            Some(_) => {}
            None => clear_slot(user.character, i),
        }
    }
    Outcome::Ended {
        spells,
        poison: None,
    }
}

/// Ends the first poison spell on a player: the first slot whose spell has the Poison
/// element is cleared and returned for its ending with `chain` set (the engine runs it); a
/// slot whose record is missing is cleared on the way.
pub fn terminate_user_poison_spell(user: &mut UserTarget<'_, '_>) -> Option<EndedSpell> {
    for i in 0..NSPELLCAST {
        let number = user.character.spells[i];
        if number == SpellNo(0) {
            continue;
        }
        match (user.spells)(number) {
            Some(spell) if spell.element == Element::Poison => {
                let power = i32::from(user.character.spellvals[i]);
                clear_slot(user.character, i);
                return Some(EndedSpell {
                    number,
                    power,
                    chain: true,
                });
            }
            Some(_) => {}
            None => clear_slot(user.character, i),
        }
    }
    None
}

fn apply_to_monster(
    slot: &EffectSlot,
    amount: i32,
    cast: &Cast<'_>,
    target: &mut MonsterTarget<'_>,
    state: &mut CastState,
    rng: &mut dyn Rng,
) -> Outcome {
    use CastPath as P;
    let lasting = cast.lasting();
    let path = cast.path;
    let undivided = match path {
        P::Area { undivided } => undivided,
        _ => amount,
    };
    // What `cast_monster_target` does for an ability it does not list: silly when instant,
    // else the slot entry; the area loops add through `add_duration_spell_to_room` with the
    // share less the monster's element defence.
    let add_lasting =
        |target: &mut MonsterTarget<'_>, state: &mut CastState, rng: &mut dyn Rng, power: i32| {
            if state.told {
                return Outcome::NoEffect;
            }
            let rounds = cast.rounds(rng);
            let out =
                add_spell_to_monster(&mut target.monster.spells, cast.spell.number, rounds, power);
            state.told = true;
            Outcome::Lasting(out)
        };
    let default_branch =
        |target: &mut MonsterTarget<'_>, state: &mut CastState, rng: &mut dyn Rng| {
            if lasting {
                let dose = area_dose(cast, amount, target.element_resist_pct);
                add_lasting(target, state, rng, dose)
            } else {
                Outcome::Silly
            }
        };
    let caster_name = cast.caster.map_or("", |c| c.firstname);

    match slot.effect {
        SpellEffect::Damage {
            ignore_armour: true,
        } => {
            if lasting && matches!(path, P::Area { .. }) {
                let dose = area_dose(cast, amount, target.element_resist_pct);
                return add_lasting(target, state, rng, dose);
            }
            // scaled, no duration test (a lasting spell still deals
            // the damage and is never added); in an area: the share as it is.
            let dmg = if path == P::Single {
                scale_spell_amount(amount, cast.spelldamage_pct())
            } else {
                amount
            };
            damage_monster(target, dmg, caster_name, rng)
        }
        SpellEffect::Damage {
            ignore_armour: false,
        } => {
            // a single target's amount; an area's compounds from target to target (sic).
            let base = match path {
                P::Area { .. } => state.area_armour_amount.unwrap_or(undivided),
                _ => amount,
            };
            let scaled = add_spell_damage_bonus(base, cast.spelldamage_pct());
            if matches!(path, P::Area { .. }) {
                state.area_armour_amount = Some(scaled);
            }
            let dmg = monster_mr_damage(scaled, target.magic_resistance, target.anti_magic);
            damage_monster(target, dmg, caster_name, rng)
        }
        SpellEffect::Slay => {
            if lasting || path != P::Single {
                return Outcome::Silly;
            }
            // to -2 hit points.
            let dmg = i32::from(target.monster.hp) + 2;
            damage_monster(target, dmg, caster_name, rng)
        }
        SpellEffect::Enslave => {
            if path != P::Single {
                return Outcome::Silly;
            }
            // the caster's level must reach the charm level.
            let Some(caster) = cast.caster else {
                return Outcome::NoEffect;
            };
            if caster.level < target.known.charmlvl {
                return Outcome::NoEffect;
            }
            let out = if lasting {
                add_lasting(target, state, rng, amount)
            } else {
                Outcome::Applied {
                    amount,
                    knocked_out: false,
                }
            };
            target.monster.following = caster.firstname.to_owned();
            target.monster.friendly = 1;
            target.monster.flags |= ActiveMonsterFlags::CHARMED;
            out
        }
        SpellEffect::StealHp => match path {
            P::Single => {
                if lasting {
                    return add_lasting(target, state, rng, amount);
                }
                let out = damage_monster(target, amount, caster_name, rng);
                monster_stolen(out, amount)
            }
            P::Area { .. } => {
                if lasting {
                    return Outcome::Silly;
                }
                // the one area case where a monster resists, and a
                // resist zeroes the share for every later monster (sic).
                if state.area_stealhp_zeroed {
                    let out = damage_monster(target, 0, caster_name, rng);
                    return monster_stolen(out, 0);
                }
                let chance =
                    super::resist::resist_chance(target.magic_resistance, super::MAXCHANCE);
                if rng.range(1, 100) <= chance {
                    state.area_stealhp_zeroed = true;
                    return Outcome::Resisted;
                }
                let out = damage_monster(target, amount, caster_name, rng);
                monster_stolen(out, amount)
            }
            _ => Outcome::Silly,
        },
        SpellEffect::Energy => {
            if lasting {
                let dose = area_dose(cast, amount, target.element_resist_pct);
                return add_lasting(target, state, rng, dose);
            }
            target.monster.energy = (i32::from(target.monster.energy) + amount) as u16;
            Outcome::Applied {
                amount,
                knocked_out: false,
            }
        }
        SpellEffect::Summon => {
            if lasting || path != P::Single {
                return Outcome::Silly;
            }
            Outcome::Summon(amount, SummonDisposition::HuntsMonster)
        }
        SpellEffect::Heal => {
            if lasting {
                return add_lasting(target, state, rng, amount);
            }
            let hp = i32::from(target.monster.hp);
            // a single target caps at the known monster's maximum; an area's at the active
            // record's.
            let maxhp = i32::from(if path == P::Single {
                target.known.maxhp
            } else {
                target.monster.maxhp
            });
            let heal = if hp + amount > maxhp {
                maxhp - hp
            } else {
                amount
            };
            if heal != 0 {
                target.monster.hp = (hp + heal) as i16;
                // the befriending roll only when something was healed
                if path == P::Single {
                    befriend(target, caster_name, rng);
                }
            }
            Outcome::Applied {
                amount: heal,
                knocked_out: false,
            }
        }
        SpellEffect::Poison => {
            if target.poison_immune {
                return Outcome::Immune;
            }
            // a single target's amount; an area's undivided amount (sic).
            let level = if path == P::Single { amount } else { undivided };
            if i32::from(target.monster.poison) < level {
                target.monster.poison = level as i16;
            }
            if path == P::Single {
                aggro(target, caster_name, rng);
            }
            if lasting {
                return add_lasting(target, state, rng, amount);
            }
            Outcome::Applied {
                amount: level,
                knocked_out: false,
            }
        }
        SpellEffect::CurePoison => {
            if lasting {
                return add_lasting(target, state, rng, amount);
            }
            let poison = i32::from(target.monster.poison);
            match path {
                P::Single => {
                    target.monster.poison = (poison - amount).max(0) as i16;
                    befriend(target, caster_name, rng);
                }
                _ => {
                    // the test is inverted (sic): a level below the
                    // undivided amount falls to 0, a level at or above it stays.
                    if poison < undivided {
                        target.monster.poison = (poison - undivided).max(0) as i16;
                    }
                }
            }
            Outcome::Applied {
                amount,
                knocked_out: false,
            }
        }
        // No effect on a monster: the lights, hunger, thirst,
        // mana and text blocks; the gates were applied in the prelude.
        SpellEffect::Hunger
        | SpellEffect::Thirst
        | SpellEffect::HealMana
        | SpellEffect::TextBlock
        | SpellEffect::KillDead
        | SpellEffect::DetectMagic
        | SpellEffect::RemoveSpell { .. }
        | SpellEffect::Guarded => Outcome::NoEffect,
        SpellEffect::Lasting(Ability::UserLight | Ability::RoomLight | Ability::BlindingLight) => {
            Outcome::NoEffect
        }
        SpellEffect::Lasting(_)
        | SpellEffect::Scatter
        | SpellEffect::Shatter
        | SpellEffect::RemoveCurse
        | SpellEffect::Freedom
        | SpellEffect::DispelMagic
        | SpellEffect::StatAlter(_)
        | SpellEffect::AlterHp
        | SpellEffect::GiveSpell
        | SpellEffect::OpenDoor
        | SpellEffect::Lore => default_branch(target, state, rng),
        SpellEffect::Teleport(_)
        | SpellEffect::EndCast
        | SpellEffect::EndCastChance
        | SpellEffect::ClearItem
        | SpellEffect::Marker(_)
        | SpellEffect::Component
        | SpellEffect::Unknown(_) => Outcome::NoEffect,
    }
}

/// `hp -= dmg` on a monster, then the survivor's turn-on-the-caster roll: a monster that
/// is not an angel follows the caster,
/// unfriendly, with `followpct` percent chance; an angel already following him stops being
/// friendly. A dead monster (hit points at or below 0) is the engine's to remove.
fn damage_monster(
    target: &mut MonsterTarget<'_>,
    dmg: i32,
    caster_name: &str,
    rng: &mut dyn Rng,
) -> Outcome {
    let after = i32::from(target.monster.hp) - dmg;
    target.monster.hp = after as i16;
    if after > 0 {
        aggro(target, caster_name, rng);
    }
    Outcome::Applied {
        amount: dmg,
        knocked_out: false,
    }
}

fn aggro(target: &mut MonsterTarget<'_>, caster_name: &str, rng: &mut dyn Rng) {
    if target.monster.montype != MonsterType::Angel {
        if rng.range(1, 100) < i32::from(target.known.followpct) {
            target.monster.following = caster_name.to_owned();
            target.monster.friendly = 0;
        }
    } else if target.monster.following.eq_ignore_ascii_case(caster_name) {
        target.monster.friendly = 0;
    }
}

/// The befriending roll after a heal or cure: a monster that is not an angel follows the
/// caster as a friend when `genrdn(1,100)` is above its
/// `followpct` (the inverse sense of the attack roll).
fn befriend(target: &mut MonsterTarget<'_>, caster_name: &str, rng: &mut dyn Rng) {
    if target.monster.montype != MonsterType::Angel
        && rng.range(1, 100) > i32::from(target.known.followpct)
    {
        target.monster.following = caster_name.to_owned();
        target.monster.friendly = 1;
    }
}

fn monster_stolen(out: Outcome, amount: i32) -> Outcome {
    match out {
        Outcome::Applied { .. } => Outcome::StolenHp { amount },
        other => other,
    }
}

/// `cast_item_target`: only Clear Item, Detect Magic and Lore act; the damage, defence and
/// theft abilities are refused as silly; the rest do nothing.
fn apply_to_item(
    slot: &EffectSlot,
    amount: i32,
    cast: &Cast<'_>,
    target: &ItemTarget<'_>,
) -> Outcome {
    match slot.effect {
        SpellEffect::DetectMagic => {
            if cast.lasting() {
                return Outcome::Silly;
            }
            Outcome::DetectMagic {
                magical: item_ability_value(target.item, Ability::Magical),
            }
        }
        SpellEffect::Lore => {
            if cast.lasting() {
                return Outcome::Silly;
            }
            Outcome::Lore {
                revealed: i32::from(target.item.level) <= amount,
            }
        }
        SpellEffect::Damage { .. } | SpellEffect::StealHp => Outcome::Silly,
        SpellEffect::Lasting(
            Ability::Defence
            | Ability::ColdDefence
            | Ability::DamageResist
            | Ability::ShadowStealth
            | Ability::Shield
            | Ability::Shadowform,
        ) => Outcome::Silly,
        _ => Outcome::NoEffect,
    }
}

/// What ending a spell on a player asked of the engine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Termination {
    /// The "Cast on ending" spell to cast with the player as caster (a chain cast).
    pub chain: Option<SpellNo>,
    /// The "Give Spell" spell that left the spellbook.
    pub purged: Option<SpellNo>,
}

/// Ends one spell on a player (for the engine's `end_player_spell`): each slot is undone
/// with the slot's value or the cast strength. Give Spell leaves the
/// spellbook; Poison wears off (never below 0); Alter HP and the stat alters go again; Cast
/// on ending (with its chance, default 100) is rolled when `chain` is set. The ending
/// message and the secondary stats are the engine's.
pub fn apply_termination(
    character: &mut Character,
    spell: &Spell,
    power: i32,
    chain: bool,
    rng: &mut dyn Rng,
) -> Termination {
    let effects = SpellEffects::from_record(spell);
    let mut end = Termination::default();
    let mut endspell = 0;
    let mut chance = 100;
    for slot in effects.iter() {
        let val = slot.amount(power);
        match slot.effect {
            SpellEffect::GiveSpell => {
                let given = SpellNo(val as i16);
                character.spellbook.retain(|&s| s != given);
                end.purged = Some(given);
            }
            SpellEffect::Poison => {
                character.poison = (i32::from(character.poison) - val).max(0) as i16;
            }
            SpellEffect::AlterHp => {
                character.maxhp = (i32::from(character.maxhp) - val) as i16;
                character.hp = (i32::from(character.hp) - val) as i16;
            }
            SpellEffect::StatAlter(stat) => {
                let field = match stat {
                    Stat::Intellect => &mut character.intel,
                    Stat::Wisdom => &mut character.wil,
                    Stat::Strength => &mut character.str,
                    Stat::Health => &mut character.hea,
                    Stat::Agility => &mut character.agi,
                    Stat::Charm => &mut character.cha,
                };
                *field = (i32::from(*field) - val) as u16;
            }
            SpellEffect::EndCast => endspell = val,
            SpellEffect::EndCastChance => chance = val,
            _ => {}
        }
    }
    if chain && endspell != 0 && rng.range(0, 100) < chance {
        end.chain = Some(SpellNo(endspell as i16));
    }
    end
}

/// A spell taken off a player whose ending the engine runs (see [`Outcome::Ended`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndedSpell {
    /// The spell.
    pub number: SpellNo,
    /// The strength it was cast with (`spellvals`).
    pub power: i32,
    /// The ending may cast its "Cast on ending" spell.
    pub chain: bool,
}

/// An instant "Cure Poison"'s cut in the poison level: `poison -= amount`, kept at 0 or
/// above when `floor`. It follows the poison spell's ending (whose own undo is floored), as
/// [`terminate_user_poison_spell`] runs first (every engine floors it but the area loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoisonCut {
    /// The cut.
    pub amount: i32,
    /// Never below 0.
    pub floor: bool,
}

/// Makes a [`PoisonCut`].
pub fn cut_poison(character: &mut Character, cut: PoisonCut) {
    let mut poison = i32::from(character.poison) - cut.amount;
    if cut.floor && poison < 0 {
        poison = 0;
    }
    character.poison = poison as i16;
}

/// What a round of a spell on a player did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoundOutcome {
    /// "Flee" fired: the engine moves the holder through a random usable exit.
    pub flee: bool,
    /// Damage over time took the holder to 0 or below from above.
    pub knocked_out: bool,
    /// The slot sends the prompt: damage ignoring armour, Steal Hit Points, Heal and Heal
    /// Mana.
    pub prompt: bool,
}

/// The per-round effects of a spell on a player, each slot with its value or the cast
/// strength: Flee with
/// `strength`-in-100 chance; damage ignoring armour and Steal Hit Points drain hit points
/// (with the knock-out rule); Energy rises to at most the maximum; Hunger and Thirst
/// change; Heal rises to at most the maximum; Heal Mana stays within 0 and the maximum;
/// Cure Poison lowers the poison (a negative value is ignored), never below 0. The outcome
/// is every slot's, or-ed; the engine runs [`apply_round_slot`] one slot at a time, as the
/// game moves, prompts and knocks out slot by slot.
pub fn apply_round(
    character: &mut Character,
    effects: &SpellEffects,
    power: i32,
    rng: &mut dyn Rng,
) -> RoundOutcome {
    let mut out = RoundOutcome::default();
    for slot in effects.iter() {
        let o = apply_round_slot(character, slot, power, rng);
        out.flee |= o.flee;
        out.knocked_out |= o.knocked_out;
        out.prompt |= o.prompt;
    }
    out
}

/// One slot of [`apply_round`].
pub fn apply_round_slot(
    character: &mut Character,
    slot: &EffectSlot,
    power: i32,
    rng: &mut dyn Rng,
) -> RoundOutcome {
    let mut out = RoundOutcome::default();
    let val = slot.amount(power);
    match slot.effect {
        SpellEffect::Lasting(Ability::Flee) => {
            if rng.range(0, 100) < val {
                out.flee = true;
            }
        }
        SpellEffect::Damage {
            ignore_armour: true,
        }
        | SpellEffect::StealHp => {
            let after = i32::from(character.hp) - val;
            character.hp = after as i16;
            out.prompt = true;
            if knocks_out(after, val) {
                out.knocked_out = true;
            }
        }
        SpellEffect::Energy => {
            let energy = (i32::from(character.energy) + val).min(i32::from(character.maxenergy));
            character.energy = energy as u16;
        }
        SpellEffect::Hunger => character.hunger = (i32::from(character.hunger) + val) as i16,
        SpellEffect::Thirst => character.thirst = (i32::from(character.thirst) + val) as i16,
        SpellEffect::Heal => {
            let hp = (i32::from(character.hp) + val).min(i32::from(character.maxhp));
            character.hp = hp as i16;
            out.prompt = true;
        }
        SpellEffect::HealMana => {
            let mana = (i32::from(character.mana) + val).clamp(0, i32::from(character.maxmana));
            character.mana = mana as i16;
            out.prompt = true;
        }
        SpellEffect::CurePoison => {
            let poison = i32::from(character.poison);
            if poison - val < poison {
                character.poison = (poison - val).max(0) as i16;
            }
        }
        _ => {}
    }
    out
}

/// The per-round effects of a spell on a monster: Flee; damage ignoring armour; Energy to
/// at most the maximum;
/// Heal with no cap; Cure Poison as for a player. Steal Hit Points, Poison and Heal Mana do
/// nothing here (unlike the player's round).
pub fn apply_monster_round(
    monster: &mut ActiveMonster,
    effects: &SpellEffects,
    power: i32,
    rng: &mut dyn Rng,
) -> RoundOutcome {
    let mut out = RoundOutcome::default();
    for slot in effects.iter() {
        let val = slot.amount(power);
        match slot.effect {
            SpellEffect::Lasting(Ability::Flee) => {
                if rng.range(0, 100) < val {
                    out.flee = true;
                }
            }
            SpellEffect::Damage {
                ignore_armour: true,
            } => {
                monster.hp = (i32::from(monster.hp) - val) as i16;
            }
            SpellEffect::Energy => {
                let energy = i32::from(monster.energy) + val;
                let max = i32::from(monster.maxenergy);
                // compared unsigned.
                monster.energy = if (energy as u16) > (max as u16) {
                    max as u16
                } else {
                    energy as u16
                };
            }
            SpellEffect::Heal => monster.hp = (i32::from(monster.hp) + val) as i16,
            SpellEffect::CurePoison => {
                let poison = i32::from(monster.poison);
                if poison - val < poison {
                    monster.poison = (poison - val).max(0) as i16;
                }
            }
            _ => {}
        }
    }
    out
}

/// Ends one spell on a monster:
/// Enslave frees it (it forgets whom it followed, is no friend, loses the charmed flag);
/// Poison wears off, never below 0. Nothing else is undone.
pub fn apply_monster_termination(monster: &mut ActiveMonster, spell: &Spell, power: i32) {
    for slot in SpellEffects::from_record(spell).iter() {
        let val = slot.amount(power);
        match slot.effect {
            SpellEffect::Enslave => {
                monster.following.clear();
                monster.friendly = 0;
                monster.flags.remove(ActiveMonsterFlags::CHARMED);
            }
            SpellEffect::Poison => {
                monster.poison = (i32::from(monster.poison) - val).max(0) as i16;
            }
            _ => {}
        }
    }
}

/// The prelude gates of `cast_monster_target` and `room_cast_on_monster`: Kill Dead needs
/// an undead target;
/// Affects Animal needs an "Animal" target; Affects Living refuses a "Non Living" one.
/// `None` when the spell may go on.
pub fn monster_gate(
    effects: &SpellEffects,
    undead: bool,
    animal: bool,
    non_living: bool,
) -> Option<Outcome> {
    for slot in effects.iter() {
        let blocked = match slot.ability {
            Ability::KillDead => !undead,
            Ability::AffectsAnimal => !animal,
            Ability::AffectsLiving => non_living,
            _ => false,
        };
        if blocked {
            return Some(Outcome::Immune);
        }
    }
    None
}

/// The caster's gain from a "Steal Hit Points" outcome, capped at his maximum.
pub fn gain_stolen_hp(caster: &mut Character, amount: i32) {
    let hp = (i32::from(caster.hp) + amount).min(i32::from(caster.maxhp));
    caster.hp = hp as i16;
}

/// Whether a monster's alignment makes hitting it an evil act when it does not follow
/// the caster.
pub fn is_good_aligned(alignment: Alignment) -> bool {
    matches!(alignment, Alignment::Good | Alignment::LawfulGood)
}
