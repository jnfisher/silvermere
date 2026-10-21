//! Record structs: the normalised shape of the game's data records, not their byte image.
//!
//! Text fields are `String` (decoded with [`crate::fixed::FixedStr`] at the import boundary),
//! parallel arrays become arrays of small structs (a room's exit, exit type and four exit
//! parameter arrays are one [`Exit`] each), codes are the [`crate::enums`] types and flag words the
//! [`crate::flags`] types. The `unk_*` members and the cache "dirty"/"modified" bytes are not
//! carried. Each type's `LAYOUT` is the byte layout of its record ([`layout`]), which
//! `silvermere-import` decodes with; field names otherwise follow the record member names.
//! DOS packed dates and times are not kept: they are Unix seconds UTC (`i64`, 0 = never),
//! converted by the importer, and the monster's `killdate`/`killtime` pair is one `killed_at`.

use serde::Serialize;

use crate::enums::Ability;

pub mod action;
pub mod bankbook;
pub mod character;
pub mod class;
pub mod gang;
pub mod item;
pub mod item_ownership;
pub mod layout;
pub mod message;
pub mod monster;
pub mod race;
pub mod room;
pub mod runtime;
pub mod shop;
pub mod spell;
pub mod text_block;
pub mod update;

pub use action::Action;
pub use bankbook::BankBook;
pub use character::{Character, EvilRecord, PermanentRecord};
pub use class::Class;
pub use gang::Gang;
pub use item::Item;
pub use item_ownership::ItemOwnership;
pub use layout::{Field, Layout, LayoutSet};
pub use message::Message;
pub use monster::{
    ActiveMonster, ActiveSpell, KnownMonster, MonsterAttack, MonsterItem, MonsterSpell,
};
pub use race::Race;
pub use room::{Exit, Room, RoomItem};
pub use runtime::{BigExp, ForgetEntry, PendingCommand, TopEntry, UserFlags, UserState};
pub use shop::{Shop, ShopSlot};
pub use spell::Spell;
pub use text_block::TextBlock;
pub use update::UpdateRecord;

/// Coins of the five currencies, copper first like [`crate::enums::Currency`].
///
/// The records order coins two ways: the player and room records hold them as five named
/// fields `runic..copper`, and the `cash[5]` arrays of items, known monsters and active
/// monsters are runic first (`cash[0]` is runic). The decoder maps both to this struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Coins {
    /// Copper farthings (`CUR_COPPER`).
    pub copper: u32,
    /// Silver nobles (`CUR_SILVER`).
    pub silver: u32,
    /// Gold crowns (`CUR_GOLD`).
    pub gold: u32,
    /// Platinum pieces (`CUR_PLATINUM`).
    pub platinum: u32,
    /// The sysop-named fifth currency (`CUR_RUNIC`).
    pub runic: u32,
}

impl Coins {
    /// From a `cash[5]` array in the records' runic-first order.
    pub const fn from_runic_first(cash: [u32; 5]) -> Self {
        Self {
            runic: cash[0],
            platinum: cash[1],
            gold: cash[2],
            silver: cash[3],
            copper: cash[4],
        }
    }

    /// To a `cash[5]` array in the records' runic-first order.
    pub const fn to_runic_first(self) -> [u32; 5] {
        [
            self.runic,
            self.platinum,
            self.gold,
            self.silver,
            self.copper,
        ]
    }
}

/// An ability slot: the ability number and the value beside it (two parallel arrays in the records).
pub type AbilitySlot = (Ability, i16);
