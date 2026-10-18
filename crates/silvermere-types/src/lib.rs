//! Record types, enums, ids, fixed-width string helpers (no I/O).
//!
//! The vocabulary every other crate shares: typed ids, the room, item, monster and shop
//! enums, the flag sets as bitflags, and the record structs of the game's data files. This
//! crate sits at the bottom of the dependency graph and performs no I/O.
//!
//! - [`ids`]: typed numbers (`RoomKey`, `ItemNo`, ...).
//! - [`enums`]: one enum per family of stored codes, discriminants equal to the stored values.
//! - [`flags`]: the flag words as `bitflags`.
//! - [`records`]: the record structs in their normalised shape, plus the byte layouts the
//!   importer decodes with.
//! - [`names`]: the game's fixed name tables.
//! - [`fixed`]: NUL-terminated CP437 fields to `String` and back.
//!
//! See `docs/types.md` for each family.

#![forbid(unsafe_code)]

pub mod config;
pub mod enums;
pub mod fixed;
pub mod flags;
pub mod ids;
pub mod names;
pub mod records;

/// Array sizes of the records.
pub mod sizes {
    /// `NSPELLCAST`: spells cast on a character.
    pub const NSPELLCAST: usize = 10;
    /// `NINVITEMS`: inventory slots.
    pub const NINVITEMS: usize = 100;
    /// `NKEYS`: key slots.
    pub const NKEYS: usize = 50;
    /// `NSPELLBOOK`: spellbook slots.
    pub const NSPELLBOOK: usize = 100;
    /// `NTRAIL`: rooms a character remembers (track).
    pub const NTRAIL: usize = 20;
    /// `NWORN`: worn-item slots.
    pub const NWORN: usize = 20;
    /// `NGROUP`: other members of a group.
    pub const NGROUP: usize = 5;
    /// `NABILITY`: character abilities.
    pub const NABILITY: usize = 30;
    /// `NDIRS`: directions.
    pub const NDIRS: usize = 10;
    /// `NROOMITEMS`: visible items in a room.
    pub const NROOMITEMS: usize = 17;
    /// `NHIDITEMS`: hidden items in a room.
    pub const NHIDITEMS: usize = 15;
    /// `NROOMMONS`: active monsters in a room.
    pub const NROOMMONS: usize = 15;
    /// `NPLACED`: placed items of a room.
    pub const NPLACED: usize = 10;
    /// `NCURRENCY`: currencies.
    pub const NCURRENCY: usize = 5;
    /// `NITEMABILS`: abilities of an item.
    pub const NITEMABILS: usize = 20;
    /// `NSHOPITEMS`: item slots of a shop.
    pub const NSHOPITEMS: usize = 20;
    /// `NRECABILS`: ability slots of a spell, class, race or known monster.
    pub const NRECABILS: usize = 10;
    /// `NMONITEMS`: items a monster carries.
    pub const NMONITEMS: usize = 10;
    /// `NMONATTACKS`: attack slots of a monster.
    pub const NMONATTACKS: usize = 5;
    /// `NMONSPELLS`: spell slots of a monster.
    pub const NMONSPELLS: usize = 5;
    /// `NAMTRAIL`: rooms an active monster remembers.
    pub const NAMTRAIL: usize = 10;
    /// `NTOPLIST`: entries of the top list.
    pub const NTOPLIST: usize = 100;
    /// `NACTTEXTS`: message texts of a social action.
    pub const NACTTEXTS: usize = 11;
    /// `TEXTSIZE`: text bytes of one WCCTEXT record.
    pub const TEXTSIZE: usize = 2000;
}

pub use sizes::*;
