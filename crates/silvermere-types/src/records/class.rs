//! The `class` record (WCCCLASS).

use serde::Serialize;

use crate::enums::{ArmourType, Magery, WeaponClass};
use crate::ids::{ClassNo, TextBlockNo};
use crate::records::AbilitySlot;
use crate::records::layout::{self, Layout};
use crate::sizes::NRECABILS;

/// A class: the `class` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Class {
    /// +0x000 class number (key 0).
    pub number: ClassNo,
    /// +0x002 class name.
    pub name: String,
    /// +0x020 max HP += level * basehp.
    pub basehp: i16,
    /// +0x022 random HP per level.
    pub hplevel: i16,
    /// +0x024 experience percentage.
    pub exp: i16,
    /// +0x026 "Attack Mult" (report only).
    pub attmult: i16,
    /// +0x028 report only.
    pub maxenergy: i16,
    /// +0x02A report only.
    pub restored: i16,
    /// +0x02C ability numbers with +0x04A their values.
    pub abilities: [AbilitySlot; NRECABILS],
    /// +0x040 magery type.
    pub magery: Magery,
    /// +0x042 magery level.
    pub magelvl: i16,
    /// +0x044 max weapon type.
    pub weapon: WeaponClass,
    /// +0x046 max armour type.
    pub armour: ArmourType,
    /// +0x048 combat value.
    pub combat: i16,
    /// +0x064 WCCTEXT block: level titles.
    pub titles: TextBlockNo,
}

impl Class {
    /// Byte layout of the `class` record.
    pub const LAYOUT: &'static Layout = &layout::CLASS;
    /// The `class` record length: 153 bytes.
    pub const RECORD_LEN: usize = 153;
}
