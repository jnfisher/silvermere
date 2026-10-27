//! The `item` record (WCCITEMS).

use serde::{Deserialize, Serialize};

use crate::enums::{ArmourType, Currency, ItemType, WeaponType, WornLocation};
use crate::ids::{ClassNo, ItemNo, MessageNo, RaceNo, SpellNo, TextBlockNo};
use crate::records::layout::{self, Layout};
use crate::records::{AbilitySlot, Coins};
use crate::sizes::NITEMABILS;

/// An item: the `item` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// +0x000 item number (key 0).
    pub number: ItemNo,
    /// +0x004 detect power to show `hiddesc`.
    pub level: u16,
    /// +0x006 copies allowed, 0 unlimited.
    pub limit: u16,
    /// +0x0AD item name.
    pub name: String,
    /// +0x0CB description, six lines.
    pub desc: [String; 6],
    /// +0x239 hidden description, three lines.
    pub hiddesc: [String; 3],
    /// +0x2F0 copies in play (limited item).
    pub active: u16,
    /// +0x2F2 encumbrance.
    pub weight: i16,
    /// +0x2F4 item type (`type`).
    pub item_type: ItemType,
    /// +0x2F6 ability numbers with +0x3DC their values.
    pub abilities: [AbilitySlot; NITEMABILS],
    /// +0x31E uses of a new copy, -1 none.
    pub charges: i16,
    /// +0x320 energy to use.
    pub energy: i16,
    /// +0x322 price in `currency` units.
    pub cost: u16,
    /// +0x324 classes allowed, all 0 = any.
    pub classes: [ClassNo; 10],
    /// +0x33E weapon damage, minimum.
    pub mindmg: i16,
    /// +0x340 weapon damage, maximum.
    pub maxdmg: i16,
    /// +0x342 armour class.
    pub ac: i16,
    /// +0x344 races allowed, all 0 = any.
    pub races: [RaceNo; 10],
    /// +0x36C spells negated while worn.
    pub negates: [SpellNo; 10],
    /// +0x394 weapon type.
    pub wpntype: WeaponType,
    /// +0x396 armour type.
    pub armtype: ArmourType,
    /// +0x398 wear location.
    pub worn: WornLocation,
    /// +0x399 accuracy bonus.
    pub accuracy: i16,
    /// +0x39B damage resistance.
    pub dr: i16,
    /// +0x39D may be picked up.
    pub gettable: i16,
    /// +0x39F strength required.
    pub strreq: i16,
    /// +0x3AB containers: coins found on OPEN (`cash[5]`, runic first in the record).
    pub cash: Coins,
    /// +0x3D9 weapon speed.
    pub speed: i16,
    /// +0x404 WCCMSG: hit verbs.
    pub hitmsg: MessageNo,
    /// +0x408 WCCMSG: miss verbs.
    pub missmsg: MessageNo,
    /// +0x40C WCCTEXT after the description.
    pub textnum: TextBlockNo,
    /// +0x410 WCCMSG when charges run out.
    pub destroymsg: MessageNo,
    /// +0x420 may not be dropped/given.
    pub nodrop: u8,
    /// +0x421 cost currency.
    pub currency: Currency,
    /// +0x422 kept when charges run out.
    pub retain: u8,
    /// +0x423 may be stolen.
    pub robable: u8,
    /// +0x424 destroyed on death.
    pub destroy: u8,
}

impl Item {
    /// Byte layout of the `item` record.
    pub const LAYOUT: &'static Layout = &layout::ITEM;
    /// The `item` record length: 1061 bytes.
    pub const RECORD_LEN: usize = 1061;
}
