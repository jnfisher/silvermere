//! The ability name table and the sets of abilities with special stacking.
//!
//! The ability names are held by `silvermere-types` as `names::ABILITY_NAMES`, indexed by
//! the ability number. This file gives them the shape the game prints them in and names the
//! abilities whose totals keep the best value instead of adding up.

use core::fmt;

use silvermere_types::enums::Ability;

/// The size of the game's ability name table: 188 names (0-187) and an end marker. The Rust
/// table holds the 188 names and no end marker.
pub const NABILNAMES: usize = 189;

/// The abilities whose character total keeps the highest single value instead of adding
/// up: Alter AV (Bless), Confusion, Damaging shield, Alter Energy Usage, Confuse Message,
/// Alter AV (Curse) and the third attack value.
pub const BEST_VALUE: [Ability; 7] = [
    Ability::AvBless,
    Ability::Confusion,
    Ability::DamageShield,
    Ability::EnergyUse,
    Ability::ConfuseMsg,
    Ability::AvCurse,
    Ability::AvThird,
];

/// The abilities whose monster total keeps the best value: the seven of [`BEST_VALUE`] plus
/// the elemental defences and
/// resistances (Alter Cold and Hot Attack Damage (Defence), Resist Stone, Resist Lightning).
pub const MONSTER_BEST_VALUE: [Ability; 11] = [
    Ability::ColdDefence,
    Ability::HotDefence,
    Ability::AvBless,
    Ability::ResistStone,
    Ability::ResistLightning,
    Ability::Confusion,
    Ability::DamageShield,
    Ability::EnergyUse,
    Ability::ConfuseMsg,
    Ability::AvCurse,
    Ability::AvThird,
];

/// The name text for an ability. An ability number the table does not cover (an `Unknown`
/// code past 187) reads as an empty string.
pub fn ability_name(ability: Ability) -> &'static str {
    ability.name().unwrap_or("")
}

/// An ability printed the way the sysop's monster description and the spell casting code
/// print it: the name then the number in brackets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityName(pub Ability);

impl fmt::Display for AbilityName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", ability_name(self.0), self.0.code())
    }
}
