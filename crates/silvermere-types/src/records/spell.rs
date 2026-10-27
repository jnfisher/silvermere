//! The `spell` record (WCCSPELS).

use serde::{Deserialize, Serialize};

use crate::enums::{Element, Magery, ResistMode, ResistStat, SpellTarget, SpellType};
use crate::flags::SpellFlags;
use crate::ids::{MessageNo, SpellNo};
use crate::records::AbilitySlot;
use crate::records::layout::{self, Layout};
use crate::sizes::NRECABILS;

/// A spell: the `spell` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spell {
    /// +0x000 spell number (key 0).
    pub number: SpellNo,
    /// +0x002 spell name.
    pub name: String,
    /// +0x020 description, two lines.
    pub descs: [String; 2],
    /// +0x086 "Resist Message".
    pub resist: i32,
    /// +0x09F caster level cap, 0 = none.
    pub levelcap: i16,
    /// +0x0A1 spell flags.
    pub flags: SpellFlags,
    /// +0x0D3 ability numbers with +0x0A5 their values.
    pub abilities: [AbilitySlot; NRECABILS],
    /// +0x0B9 combat energy used.
    pub energy: u16,
    /// +0x0BB minimum level to cast.
    pub level: u16,
    /// +0x0BD minimum value.
    pub minval: i16,
    /// +0x0BF maximum value.
    pub maxval: i16,
    /// +0x0C1 spell type (`type`).
    pub spell_type: SpellType,
    /// +0x0C3 resist mode.
    pub resmode: ResistMode,
    /// +0x0C4 difficulty, >=200 never fails.
    pub diff: i16,
    /// +0x0C6 duration bonus per level.
    pub durlvl: i16,
    /// +0x0C8 target.
    pub target: SpellTarget,
    /// +0x0CA base duration, rounds.
    pub duration: i16,
    /// +0x0CC attack element.
    pub element: Element,
    /// +0x0D0 stat that resists.
    pub resstat: ResistStat,
    /// +0x0D1 magic school.
    pub magery: Magery,
    /// +0x0E7 WCCMSG cast texts.
    pub msgnum: MessageNo,
    /// +0x0EB mana / kai cost.
    pub mana: i16,
    /// +0x0ED `maxval += maxinc * level / maxlvls`.
    pub maxinc: u8,
    /// +0x0EE see `maxinc`.
    pub maxlvls: u8,
    /// +0x0EF magery level required.
    pub magelvl: i16,
    /// +0x0F1 `minval += mininc * level / minlvls`.
    pub mininc: u8,
    /// +0x0F2 see `mininc`.
    pub minlvls: u8,
    /// +0x0F3 `duration += level / durlvls * durinc`.
    pub durinc: u8,
    /// +0x0F4 see `durinc`.
    pub durlvls: u8,
    /// +0x0F5 command name, e.g. "mmis".
    pub shortnm: String,
}

impl Spell {
    /// Byte layout of the `spell` record.
    pub const LAYOUT: &'static Layout = &layout::SPELL;
    /// The `spell` record length: 253 bytes.
    pub const RECORD_LEN: usize = 253;
}
