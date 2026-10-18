//! Casting (T-050): the CAST/INVOKE path from the spellbook to the effects and messages,
//! lasting spells and their rounds, the spellbook and the Kai powers.
//!
//! - [`cast`]: the one casting path ([`cast::cast`]) with its four target arms
//!   (`cast_no_target`, `cast_user_target`, `cast_monster_target`, `cast_item_target`).
//! - [`messages`]: `display_spell_success`, the three-view [`messages::SpellMsg`].
//! - [`active`]: the spell rounds and endings on players and monsters, and confusion.
//! - [`spellbook`]: finding a spell by name, who may cast what, PURGE, the Kai powers.
//!
//! The arithmetic is T-049's `silvermere_rules::spells`; the commands are
//! `commands::cast`.

pub mod active;
pub mod area;
pub mod cast;
pub mod messages;
#[cfg(test)]
mod per_step_tests;
pub mod room;
pub mod spellbook;
#[cfg(test)]
pub(crate) mod test_fixture;

pub use cast::{
    CastMode, CastOpts, CastOutcome, CastResult, CastTarget, Caster, Engage, autocombat_cast, cast,
};
pub use messages::{SpellMsg, Told};
