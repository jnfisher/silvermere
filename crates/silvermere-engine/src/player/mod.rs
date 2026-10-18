//! The player record: creating, loading, saving and deriving a character.
//!
//! The design is `docs/engine/player-record.md`; the card is T-023. The pieces:
//!
//! - [`create`]: `create_player`, `roll_stats`, `class_title`.
//! - [`load`]: `load_player`'s repairs, notices and derived values for a stored row.
//! - [`save`]: `save_player` and its 30 save points, `delete_player`, the 60 s sweep.
//! - [`secondary`]: the secondary stats, pure and as the engine wrapper.
//! - [`profile`]: the permanent info and kept evil points an account keeps, and the
//!   bookkeeping of a character's permanent end.
//! - [`suicide`]: `commit_suicide`, the end of `SUICIDE` and `REROLL` (T-068).
//!
//! The card's `touches` named `player/*.rs` files; the merged engine (T-015) had no
//! `player` module yet, so this one is new and registered once in `lib.rs`. Session-only
//! state (`leader`, `group[]`, `dragging`, `movedir`'s channel use, `attackers`, `delay`,
//! `moves`, `resttick`, `medtick`) stays out of persistence: the party lives in
//! [`crate::player_session::Party`], and the rest are reset on load.

pub mod create;
pub mod load;
pub mod profile;
pub mod save;
pub mod secondary;
pub mod suicide;

pub use create::{class_title, create_player, roll_stats};
pub use load::{LoadError, load_player};
pub use profile::Retire;
pub use save::{
    QueuedCharacter, SavePoint, delete_player, on_characters_written, queue_character,
    queue_character_delete, queued_or_stored, save_at, save_characters, save_player,
    settle_characters_written, with_queued,
};
pub use secondary::{CalcMode, Secondary, recalculate, secondary};

/// `PLAYER_VERSION`: the record version of 1.11p, the only one loaded.
pub const PLAYER_VERSION: i16 = 22;
/// `STARTSTAT`: every stat of a new character before the roll.
pub const STARTSTAT: u16 = 5;
/// `RANK_MIDDLE`: middle of the party; also no party.
pub const RANK_MIDDLE: u8 = 0;
/// `MAXLIVES`: the lives a character starts with.
pub const MAXLIVES: i16 = 9;
/// `EVIL_OUTLAW`: from 40 evil points the evil reincarnation room is used.
pub const EVIL_OUTLAW: i16 = 40;
