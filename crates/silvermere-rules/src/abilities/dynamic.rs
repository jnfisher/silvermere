//! The ability-driven ("dynamic") stats of a character: [`dynamic_stats`] and the two
//! per-ability steps it is built from.
//!
//! The game keeps these as fields of the player record (`avbonus` ... `manarate`) and
//! rebuilds them whenever the secondary stats are recalculated, on the slow update, on
//! input, on the attack command and on the confusion check. Here they are a value the
//! engine copies into the record (or recomputes on load: T-023's choice).

use silvermere_types::enums::Ability;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};

use super::lookup::{CharacterAbilities, Source, negate_target};

/// The bonus fields abilities feed and the status flags they set.
///
/// Every field is the player record's field of the same name, as a rebuild leaves it;
/// `status_from_abilities` and `status2_from_abilities` hold only the `FROMABILS` bits of
/// [`PlayerStatus`] and [`PlayerStatus2`], the ones abilities set, so the engine merges them
/// as `(status - FROMABILS) | status_from_abilities`.
///
/// Who reads what (not here): `avbonus` is added to the attack value and `acbonus` to the
/// defence value, a quarter of it against a backstab; `dmgbonus` raises the maximum damage,
/// `drbonus` adds to the damage resistance kept in tenths and `drpct` scales it; combat also
/// reads `energyuse`, `shield` and `defmod` (the defender's modifier added to the
/// attacker's: positive makes the defender easier to hit); regeneration reads `healrate` and
/// `manarate`; room light reads `roomlight`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicStats {
    /// Best of the three "Alter AV" abilities (Bless, Curse, third); 0 when none applies.
    pub avbonus: i32,
    /// "Alter Defence Value" total.
    pub acbonus: i32,
    /// "Alter Damage (Attacking)" total.
    pub dmgbonus: i32,
    /// "Alter Critical Hit Chance" total.
    pub critbonus: i32,
    /// "Alter Damage Resistance Value" total.
    pub drbonus: i32,
    /// "Alter DR by percent" total.
    pub drpct: i32,
    /// "Alter Energy Usage": the running average over the sources that gave it.
    pub energyuse: i32,
    /// "Protective Shield" total.
    pub shield: i32,
    /// "Defence Modifier" total.
    pub defmod: i32,
    /// "Alter healing rate" total.
    pub healrate: i32,
    /// "Alter Mana Regeneration" total.
    pub manarate: i32,
    /// "Alter Room Light" total.
    pub roomlight: i32,
    /// Confusion, Mute, Paralyze, Blind User, See Hidden: the `PlayerStatus::FROMABILS` bits.
    pub status_from_abilities: PlayerStatus,
    /// Slowness, Quickness, Hold person, Flee: the `PlayerStatus2::FROMABILS` bits.
    pub status2_from_abilities: PlayerStatus2,
}

impl Default for DynamicStats {
    /// Everything cleared, as a rebuild starts and as a character without abilities ends.
    fn default() -> Self {
        Self {
            avbonus: 0,
            acbonus: 0,
            dmgbonus: 0,
            critbonus: 0,
            drbonus: 0,
            drpct: 0,
            energyuse: 0,
            shield: 0,
            defmod: 0,
            healrate: 0,
            manarate: 0,
            roomlight: 0,
            status_from_abilities: PlayerStatus::empty(),
            status2_from_abilities: PlayerStatus2::empty(),
        }
    }
}

/// The bonus fields the three "Alter AV" abilities share, keeping the best value: `None`
/// while none has applied.
struct Building {
    stats: DynamicStats,
    avbonus: Option<i32>,
}

impl Building {
    fn new() -> Self {
        Self {
            stats: DynamicStats::default(),
            avbonus: None,
        }
    }

    /// One ability slot's value applied to the stats.
    fn apply(&mut self, ability: Ability, value: i32) {
        let s = &mut self.stats;
        match ability {
            // The three attack value abilities: the best one counts.
            Ability::AvBless | Ability::AvCurse | Ability::AvThird => {
                if self.avbonus.is_none_or(|best| best < value) {
                    self.avbonus = Some(value);
                }
            }
            Ability::Defence => s.acbonus += value,
            Ability::Damage => s.dmgbonus += value,
            Ability::Critical => s.critbonus += value,
            Ability::DamageResist => s.drbonus += value,
            Ability::DrPercent => s.drpct += value,
            // Energy use is averaged with what earlier sources gave, so with
            // several sources the result depends on their order.
            Ability::EnergyUse => {
                s.energyuse = if s.energyuse != 0 {
                    (s.energyuse + value) / 2
                } else {
                    value
                };
            }
            Ability::Shield => s.shield += value,
            Ability::DefenceMod => s.defmod += value,
            Ability::HealRate => s.healrate += value,
            Ability::ManaRate => s.manarate += value,
            // The status abilities only set their flag; the value does not matter.
            Ability::Confusion => s.status_from_abilities |= PlayerStatus::CONFUSED,
            Ability::Mute => s.status_from_abilities |= PlayerStatus::MUTE,
            Ability::Paralyze => s.status_from_abilities |= PlayerStatus::PARALYSED,
            Ability::BlindUser => s.status_from_abilities |= PlayerStatus::BLIND,
            Ability::SeeHidden => s.status_from_abilities |= PlayerStatus::SEEHIDDEN,
            Ability::Slowness => s.status2_from_abilities |= PlayerStatus2::SLOWED,
            Ability::Quickness => s.status2_from_abilities |= PlayerStatus2::QUICKENED,
            Ability::HoldPerson => s.status2_from_abilities |= PlayerStatus2::HELD,
            Ability::Flee => s.status2_from_abilities |= PlayerStatus2::AFRAID,
            Ability::RoomLight => s.roomlight += value,
            // Every other ability, "Negate ability" included, changes nothing.
            _ => {}
        }
    }

    /// One ability negated: the whole field goes back to 0, whatever source gave it, or the
    /// flag is cleared.
    fn negate(&mut self, ability: Ability) {
        let s = &mut self.stats;
        match ability {
            Ability::AvBless | Ability::AvCurse | Ability::AvThird => self.avbonus = Some(0),
            Ability::Defence => s.acbonus = 0,
            Ability::Damage => s.dmgbonus = 0,
            Ability::Critical => s.critbonus = 0,
            Ability::DamageResist => s.drbonus = 0,
            Ability::DrPercent => s.drpct = 0,
            Ability::EnergyUse => s.energyuse = 0,
            Ability::Shield => s.shield = 0,
            Ability::DefenceMod => s.defmod = 0,
            Ability::Confusion => s.status_from_abilities -= PlayerStatus::CONFUSED,
            Ability::Mute => s.status_from_abilities -= PlayerStatus::MUTE,
            Ability::Paralyze => s.status_from_abilities -= PlayerStatus::PARALYSED,
            Ability::BlindUser => s.status_from_abilities -= PlayerStatus::BLIND,
            Ability::SeeHidden => s.status_from_abilities -= PlayerStatus::SEEHIDDEN,
            Ability::Slowness => s.status2_from_abilities -= PlayerStatus2::SLOWED,
            Ability::Quickness => s.status2_from_abilities -= PlayerStatus2::QUICKENED,
            Ability::HoldPerson => s.status2_from_abilities -= PlayerStatus2::HELD,
            Ability::Flee => s.status2_from_abilities -= PlayerStatus2::AFRAID,
            Ability::HealRate => s.healrate = 0,
            Ability::ManaRate => s.manarate = 0,
            Ability::RoomLight => s.roomlight = 0,
            // Other abilities are left alone.
            _ => {}
        }
    }

    /// No attack value ability at all: no bonus.
    fn finish(self) -> DynamicStats {
        DynamicStats {
            avbonus: self.avbonus.unwrap_or(0),
            ..self.stats
        }
    }
}

/// The stats rebuilt from scratch.
///
/// Clears everything abilities set, then applies every slot of every source in the order of
/// the totals: own slots, spells cast (a slot without a fixed value using the rolled one,
/// "Negate ability" only noted), race, class, worn items, the weapon, carried items that are
/// neither weapons nor wearable; then each "Negate ability" slot of the spells cancels the
/// ability its value names. Only spells negate: the ability on a race, class or item is
/// ignored.
pub fn dynamic_stats(character: &CharacterAbilities<'_>) -> DynamicStats {
    let mut building = Building::new();
    let mut negate = false;
    for source in character.sources() {
        match source {
            Source::Slots(slots) => {
                for &(ability, value) in slots {
                    if ability != Ability::None {
                        building.apply(ability, i32::from(value));
                    }
                }
            }
            Source::Spell { spell, rolled } => {
                for &(ability, value) in &spell.abilities {
                    if ability == Ability::Negate {
                        negate = true;
                    } else if ability != Ability::None {
                        let value = if value != 0 { value } else { rolled };
                        building.apply(ability, i32::from(value));
                    }
                }
            }
            Source::Item(Some(item)) => {
                for &(ability, value) in &item.abilities {
                    if ability != Ability::None {
                        building.apply(ability, i32::from(value));
                    }
                }
            }
            Source::Item(None) => {}
        }
    }
    if negate {
        for (spell, _) in character.spells_cast() {
            for &(ability, value) in &spell.abilities {
                if ability == Ability::Negate
                    && let Some(target) = negate_target(value)
                {
                    building.negate(target);
                }
            }
        }
    }
    building.finish()
}
