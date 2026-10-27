//! Typed ids: one newtype per kind of number the game stores and refers to.
//!
//! The inner type is the width and signedness of the stored field (32-bit signed is `i32`,
//! 32-bit unsigned is `u32`, 16-bit signed is `i16`, 16-bit unsigned is `u16`); account and
//! character ids are database keys.

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$meta:meta])* $name:ident($inner:ty)) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $inner);

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id!(
    /// Map number (32-bit unsigned in the room record).
    MapId(u32)
);
id!(
    /// Room number within a map (32-bit unsigned in the room record); 0 means "no room".
    RoomId(u32)
);
id!(
    /// Item number (32-bit signed in the item record); 0 means "no item".
    ItemNo(i32)
);
id!(
    /// Known-monster number (32-bit signed in the known-monster record).
    MonsterNo(i32)
);
id!(
    /// Active monster number (32-bit signed in the active-monster record).
    ActiveMonsterId(i32)
);
id!(
    /// Spell number (16-bit signed in the spell record).
    SpellNo(i16)
);
id!(
    /// Class number (16-bit unsigned in the class record).
    ClassNo(u16)
);
id!(
    /// Race number (16-bit unsigned in the race record).
    RaceNo(u16)
);
id!(
    /// Shop number (32-bit signed in the shop record).
    ShopNo(i32)
);
id!(
    /// Message number (32-bit signed in the WCCMSG record).
    MessageNo(i32)
);
id!(
    /// Text block number (32-bit signed in the WCCTEXT record).
    TextBlockNo(i32)
);
id!(
    /// A connected session (in place of the BBS channel number).
    SessionId(u16)
);
id!(
    /// Account primary key.
    AccountId(i64)
);
id!(
    /// Character primary key.
    CharacterId(i64)
);

/// A room's key: map and room number (the room file's key).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct RoomKey {
    /// Map number.
    pub map: MapId,
    /// Room number within the map.
    pub room: RoomId,
}

impl RoomKey {
    /// The key of room `room` on map `map`.
    pub const fn new(map: u32, room: u32) -> Self {
        Self {
            map: MapId(map),
            room: RoomId(room),
        }
    }
}
