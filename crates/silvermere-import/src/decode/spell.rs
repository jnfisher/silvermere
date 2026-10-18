//! WCCSPELS spells: the `spell` record. Not carried: `modified` (cache
//! byte) and the `unk_*` members.

use silvermere_types::enums::{Element, Magery, ResistMode, ResistStat, SpellTarget, SpellType};
use silvermere_types::flags::SpellFlags;
use silvermere_types::ids::{MessageNo, SpellNo};
use silvermere_types::records::{Spell, layout};

use super::{Raw, Text, abilities, record};

record! {
    /// The `spell` record, WCCSPELS: every member, in record order.
    pub struct SpellRecord = layout::SPELL {
        number: i16,
        name: Text<30>,
        descs: [Text<51>; 2],
        resist: i32,
        unk_0x08A: Raw<21>,
        levelcap: i16,
        flags: u8,
        unk_0x0A2: Raw<3>,
        abilvals: [i16; 10],
        energy: u16,
        level: u16,
        minval: i16,
        maxval: i16,
        type_ = "type": i16,
        resmode: u8,
        diff: i16,
        durlvl: i16,
        target: i16,
        duration: i16,
        element: i16,
        unk_0x0CE: Raw<2>,
        resstat: u8,
        magery: i16,
        abilities: [i16; 10],
        msgnum: i32,
        mana: i16,
        maxinc: u8,
        maxlvls: u8,
        magelvl: i16,
        mininc: u8,
        minlvls: u8,
        durinc: u8,
        durlvls: u8,
        shortnm: Text<6>,
        modified: u8,
        unk_0x0FC: u8,
    }
}

/// The schema's spell.
pub fn to_spell(r: &SpellRecord) -> Spell {
    Spell {
        number: SpellNo(r.number),
        name: r.name.trimmed(),
        descs: r.descs.clone().map(|t| t.trimmed()),
        resist: r.resist,
        levelcap: r.levelcap,
        flags: SpellFlags::from_bits_retain(r.flags),
        abilities: abilities(&r.abilities, &r.abilvals),
        energy: r.energy,
        level: r.level,
        minval: r.minval,
        maxval: r.maxval,
        spell_type: SpellType::from_code(r.type_ as u16),
        resmode: ResistMode::from_code(u16::from(r.resmode)),
        diff: r.diff,
        durlvl: r.durlvl,
        target: SpellTarget::from_code(r.target as u16),
        duration: r.duration,
        element: Element::from_code(r.element as u16),
        resstat: ResistStat::from_code(u16::from(r.resstat)),
        magery: Magery::from_code(r.magery as u16),
        msgnum: MessageNo(r.msgnum),
        mana: r.mana,
        maxinc: r.maxinc,
        maxlvls: r.maxlvls,
        magelvl: r.magelvl,
        mininc: r.mininc,
        minlvls: r.minlvls,
        durinc: r.durinc,
        durlvls: r.durlvls,
        shortnm: r.shortnm.trimmed(),
    }
}
