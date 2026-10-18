//! Abilities: how much of an ability a character or monster has, counting every source,
//! and the ability-driven ("dynamic") bonus fields of a character.
//!
//! Characters, races, classes, spells, items and monsters carry ability slots: an ability
//! number ([`Ability`]) with a value beside it. This module answers two questions over those
//! slots exactly as the game does:
//!
//! - [`AbilityLookup::value`]: the total of one ability over the character's own slots, the
//!   spells cast on it, its race, its class, its worn items, its weapon and the pack items
//!   that are neither weapons nor wearable, with the game's stacking rule (most abilities add
//!   up; the seven in [`BEST_VALUE`] keep the highest single value) and its negation rule (a
//!   spell carrying "Negate ability" for the ability makes the total 0).
//! - [`dynamic_stats`]: the bonus fields the game keeps in the player record (`avbonus`,
//!   `acbonus`, `dmgbonus`, ...) and the status flags abilities set, rebuilt from the same
//!   sources in the same order.
//!
//! The rules crate owns no world: [`CharacterAbilities`] and [`MonsterAbilities`] borrow the
//! records and two closures that resolve spell and item numbers, which the engine passes over
//! its maps. Combat (T-031), the secondary stats (T-023), `can_use` (T-030), regen (T-037) and
//! spells (T-049) read ability totals through the [`AbilityLookup`] trait and never walk the
//! slots themselves.
//!
//! A best-value ability no source carries has no value: [`AbilityLookup::best`] returns
//! `None` for it, and [`AbilityLookup::value`] returns the 0 the game shows for it.

pub mod dynamic;
pub mod lookup;
pub mod table;

#[cfg(test)]
mod tests;

pub use dynamic::{DynamicStats, dynamic_stats};
pub use lookup::{
    AbilityLookup, CharacterAbilities, MonsterAbilities, item_ability_value, item_has_ability,
    spell_ability_value, spell_has_ability,
};
pub use silvermere_types::enums::Ability;
pub use table::{AbilityName, BEST_VALUE, MONSTER_BEST_VALUE, NABILNAMES, ability_name};
