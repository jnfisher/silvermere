//! The `itemown` record (WCCITOWN).

use serde::Serialize;

use crate::enums::OwnerKind;
use crate::ids::{ActiveMonsterId, ItemNo, MapId, RoomId};
use crate::records::layout::{self, Layout};

/// Where one copy of a limited item is: the `itemown` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemOwnership {
    /// +0x00 record id (key 0).
    pub id: i32,
    /// +0x04 item number (key 1).
    pub itemnum: ItemNo,
    /// +0x08 kind of owner (key 1).
    pub owntype: OwnerKind,
    /// +0x09 `OWN_ROOM`: map.
    pub mapnum: MapId,
    /// +0x0D `OWN_ROOM`: room.
    pub roomnum: RoomId,
    /// +0x11 `OWN_USER`: user-id.
    pub userid: String,
    /// +0x2F `OWN_MONSTER`: active monster.
    pub monnum: ActiveMonsterId,
}

impl ItemOwnership {
    /// Byte layout of the `itemown` record.
    pub const LAYOUT: &'static Layout = &layout::ITEMOWN;
    /// The `itemown` record length: 65 bytes.
    pub const RECORD_LEN: usize = 65;
}
