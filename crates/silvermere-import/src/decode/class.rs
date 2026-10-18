//! WCCCLASS classes: the `class` record. Not carried: `modified` (cache
//! byte) and the `unk_*` members (`unk_0x05E` stays on the flat record for T-009).

use silvermere_types::enums::{ArmourType, Magery, WeaponClass};
use silvermere_types::ids::{ClassNo, TextBlockNo};
use silvermere_types::records::{Class, layout};

use super::{Raw, Text, abilities, record};

record! {
    /// The `class` record, WCCCLASS: every member, in record order.
    pub struct ClassRecord = layout::CLASS {
        number: u16,
        name: Text<30>,
        basehp: i16,
        hplevel: i16,
        exp: i16,
        attmult: i16,
        maxenergy: i16,
        restored: i16,
        abilities: [i16; 10],
        magery: i16,
        magelvl: i16,
        weapon: i16,
        armour: i16,
        combat: i16,
        abilvals: [i16; 10],
        unk_0x05E: Raw<6>,
        titles: i32,
        modified: u8,
        unk_0x069: Raw<48>,
    }
}

/// The schema's class.
pub fn to_class(r: &ClassRecord) -> Class {
    Class {
        number: ClassNo(r.number),
        name: r.name.trimmed(),
        basehp: r.basehp,
        hplevel: r.hplevel,
        exp: r.exp,
        attmult: r.attmult,
        maxenergy: r.maxenergy,
        restored: r.restored,
        abilities: abilities(&r.abilities, &r.abilvals),
        magery: Magery::from_code(r.magery as u16),
        magelvl: r.magelvl,
        weapon: WeaponClass::from_code(r.weapon as u16),
        armour: ArmourType::from_code(r.armour as u16),
        combat: r.combat,
        titles: TextBlockNo(r.titles),
    }
}
