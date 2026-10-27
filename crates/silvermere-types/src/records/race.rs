//! The `race` record (WCCRACE).

use serde::{Deserialize, Serialize};

use crate::ids::RaceNo;
use crate::records::AbilitySlot;
use crate::records::layout::{self, Layout};
use crate::sizes::NRECABILS;

/// A race: the `race` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Race {
    /// +0x000 race number (key 0).
    pub number: RaceNo,
    /// +0x002 race name.
    pub name: String,
    /// +0x020 minimum intellect.
    pub minint: i16,
    /// +0x022 minimum willpower.
    pub minwil: i16,
    /// +0x024 minimum strength.
    pub minstr: i16,
    /// +0x026 minimum health.
    pub minhea: i16,
    /// +0x028 minimum agility.
    pub minagi: i16,
    /// +0x02A minimum charm.
    pub mincha: i16,
    /// +0x02C max HP += level * hpbonus.
    pub hpbonus: i16,
    /// +0x02E report only.
    pub maxenergy: i16,
    /// +0x030 report only.
    pub restored: i16,
    /// +0x032 ability numbers with +0x048 their values.
    pub abilities: [AbilitySlot; NRECABILS],
    /// +0x046 starting character points.
    pub basecp: i16,
    /// +0x062 experience percentage.
    pub exp: i16,
    /// +0x066 maximum intellect.
    pub maxint: i16,
    /// +0x068 maximum willpower.
    pub maxwil: i16,
    /// +0x06A maximum strength.
    pub maxstr: i16,
    /// +0x06C maximum health.
    pub maxhea: i16,
    /// +0x06E maximum agility.
    pub maxagi: i16,
    /// +0x070 maximum charm.
    pub maxcha: i16,
}

impl Race {
    /// Byte layout of the `race` record.
    pub const LAYOUT: &'static Layout = &layout::RACE;
    /// The `race` record length: 126 bytes.
    pub const RECORD_LEN: usize = 126;
}
