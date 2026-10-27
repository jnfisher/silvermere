//! The `room` record (WCCMP001) and the `roomkey` record.

use serde::{Deserialize, Serialize};

use crate::enums::{ExitType, GangHouse, RoomType};
use crate::flags::RoomFlags;
use crate::ids::{ActiveMonsterId, ItemNo, MonsterNo, RoomId, RoomKey, ShopNo, TextBlockNo};
use crate::records::Coins;
use crate::records::layout::{self, Layout};
use crate::sizes::{NDIRS, NHIDITEMS, NPLACED, NROOMITEMS, NROOMMONS};

/// One exit of a room: the destination, exit type and four parameters at one direction index.
///
/// The parameters mean different things per [`ExitType`]; a Hidden exit keeps
/// [`crate::flags::ExitFlags`] in `par1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    /// +0x338 destination room on the same map, 0 = no exit.
    pub target: RoomId,
    /// +0x360 exit type.
    pub kind: ExitType,
    /// +0x374 first parameter (32-bit unsigned).
    pub par1: u32,
    /// +0x39C second parameter (16-bit signed).
    pub par2: i16,
    /// +0x3B0 third parameter (32-bit signed).
    pub par3: i32,
    /// +0x3D8 fourth parameter (32-bit signed).
    pub par4: i32,
}

/// An item slot of a room: `items[i]`, `itemuses[i]`, `itemcounts[i]` (or the hidden ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomItem {
    /// The item number, 0 = empty.
    pub item: ItemNo,
    /// Uses left, -1 none.
    pub uses: i16,
    /// Extra copies in this slot.
    pub count: i16,
}

/// A room: the `room` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    /// +0x000 map number and +0x004 room number (key 0).
    pub key: RoomKey,
    /// +0x105 room name.
    pub name: String,
    /// +0x13A long description, seven lines.
    pub desc: [String; 7],
    /// +0x32B ANSI map file.
    pub mapfile: String,
    /// The ten exits, by [`crate::enums::Direction`].
    pub exits: [Exit; NDIRS],
    /// +0x400 active monsters here.
    pub monsters: [ActiveMonsterId; NROOMMONS],
    /// +0x43C room type (`type`).
    pub room_type: RoomType,
    /// +0x43E shop/bank/trainer, 0 = none.
    pub shopnum: ShopNo,
    /// +0x442 owner's user-id (`RF_OWNED`).
    pub owner: String,
    /// +0x460 generation: min level.
    pub minlevel: i16,
    /// +0x462 generation: max level (arena: player max).
    pub maxlevel: i16,
    /// +0x464 generation monster number.
    pub genmonster: MonsterNo,
    /// +0x468 light level, <0 dark.
    pub light: i16,
    /// +0x46A gang house number 0..10.
    pub ganghouse: GangHouse,
    /// +0x46C visible items with +0x4B0 their uses and +0x56B their extra copies.
    pub items: [RoomItem; NROOMITEMS],
    /// +0x4D2 hidden items with +0x50E their uses and +0x58D their extra copies.
    pub hiditems: [RoomItem; NHIDITEMS],
    /// +0x52C..+0x53C coins lying here.
    pub coins: Coins,
    /// +0x540..+0x550 hidden coins.
    pub hidcoins: Coins,
    /// +0x554 most monsters generated here.
    pub maxmonsters: u8,
    /// +0x557 monster group.
    pub mongroup: i16,
    /// +0x559 minute of last kill (run time).
    pub lastkill: u16,
    /// +0x55B room flags.
    pub flags: RoomFlags,
    /// +0x55F hidden slots found (run time).
    pub foundmask: u32,
    /// +0x563 reincarnation room, 0 default.
    pub deathroom: RoomId,
    /// +0x567 room for a hang-up here.
    pub huproom: RoomId,
    /// +0x5AB WCCTEXT block: room command.
    pub cmdtext: TextBlockNo,
    /// +0x5B3 minutes between regenerations.
    pub regendelay: i16,
    /// +0x5B5 area monster maximum.
    pub areamax: i16,
    /// +0x5B7 area monsters now (run time).
    pub areacount: i16,
    /// +0x5B9 controlling room, 0 = none.
    pub ctlroom: RoomId,
    /// +0x5BD specific monster (known number).
    pub specmon: MonsterNo,
    /// +0x5C1 placed items.
    pub placed: [ItemNo; NPLACED],
    /// +0x5E9 WCCTEXT: protected reason.
    pub protmsg: TextBlockNo,
    /// +0x5EE spell on a failed pick-lock.
    pub picktrap: i16,
    /// +0x5F1 occupant spell, medium cycle.
    pub roomspell: i32,
    /// +0x5F6 players who found hidden items.
    pub searchers: u8,
    /// +0x5F7 monsters generated here now.
    pub nummonsters: u8,
}

impl Room {
    /// Byte layout of the `room` record.
    pub const LAYOUT: &'static Layout = &layout::ROOM;
    /// The `room` record length: 1528 bytes.
    pub const RECORD_LEN: usize = 1528;
}

impl RoomKey {
    /// Byte layout of the `roomkey` record.
    pub const LAYOUT: &'static Layout = &layout::ROOMKEY;
    /// The `roomkey` record length: 8 bytes.
    pub const RECORD_LEN: usize = 8;
}
