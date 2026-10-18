//! WCCITEMS items: the `item` record.
//!
//! `active` (+0x2F0, copies in play) is state the schema keeps on `items` beside the
//! content (`Items::set_active`); it is imported as found (0 in the virgin file). `cash[5]`
//! (+0x3AB) is runic first like the monster's. Not carried: `dirty`
//! (cache byte) and the `unk_*` members, which stay on the flat record for T-009.

use silvermere_types::enums::{ArmourType, Currency, ItemType, WeaponType, WornLocation};
use silvermere_types::ids::{ClassNo, ItemNo, MessageNo, RaceNo, SpellNo, TextBlockNo};
use silvermere_types::records::{Item, layout};

use super::{Raw, Text, abilities, coins_runic_first, record};

record! {
    /// The `item` record, WCCITEMS: every member, in record order.
    pub struct ItemRecord = layout::ITEM {
        number: i32,
        level: u16,
        limit: u16,
        unk_0x008: Raw<165>,
        name: Text<30>,
        desc: [Text<61>; 6],
        hiddesc: [Text<61>; 3],
        active: u16,
        weight: i16,
        type_ = "type": i16,
        abilities: [i16; 20],
        charges: i16,
        energy: i16,
        cost: u16,
        classes: [i16; 10],
        unk_0x338: i16,
        unk_0x33a: i16,
        unk_0x33c: i16,
        mindmg: i16,
        maxdmg: i16,
        ac: i16,
        races: [i32; 10],
        negates: [i32; 10],
        wpntype: i16,
        armtype: i16,
        worn: u8,
        accuracy: i16,
        dr: i16,
        gettable: i16,
        strreq: i16,
        unk_0x3a1: i32,
        unk_0x3a5: i32,
        unk_0x3a9: i16,
        cash: [i32; 5],
        unk_0x3bf: Raw<24>,
        unk_0x3d7: i16,
        speed: i16,
        dirty: u8,
        abilvals: [i16; 20],
        hitmsg: i32,
        missmsg: i32,
        textnum: i32,
        destroymsg: i32,
        unk_0x414: i32,
        unk_0x418: i32,
        unk_0x41c: i32,
        nodrop: u8,
        currency: u8,
        retain: u8,
        robable: u8,
        destroy: u8,
    }
}

/// The schema's item.
pub fn to_item(r: &ItemRecord) -> Item {
    Item {
        number: ItemNo(r.number),
        level: r.level,
        limit: r.limit,
        name: r.name.trimmed(),
        desc: r.desc.clone().map(|t| t.trimmed()),
        hiddesc: r.hiddesc.clone().map(|t| t.trimmed()),
        active: r.active,
        weight: r.weight,
        item_type: ItemType::from_code(r.type_ as u16),
        abilities: abilities(&r.abilities, &r.abilvals),
        charges: r.charges,
        energy: r.energy,
        cost: r.cost,
        classes: r.classes.map(|c| ClassNo(c as u16)),
        mindmg: r.mindmg,
        maxdmg: r.maxdmg,
        ac: r.ac,
        races: r.races.map(|c| RaceNo(c as u16)),
        negates: r.negates.map(|s| SpellNo(s as i16)),
        wpntype: WeaponType::from_code(r.wpntype as u16),
        armtype: ArmourType::from_code(r.armtype as u16),
        worn: WornLocation::from_code(u16::from(r.worn)),
        accuracy: r.accuracy,
        dr: r.dr,
        gettable: r.gettable,
        strreq: r.strreq,
        cash: coins_runic_first(r.cash.map(|c| c as u32)),
        speed: r.speed,
        hitmsg: MessageNo(r.hitmsg),
        missmsg: MessageNo(r.missmsg),
        textnum: TextBlockNo(r.textnum),
        destroymsg: MessageNo(r.destroymsg),
        nodrop: r.nodrop,
        currency: Currency::from_code(u16::from(r.currency)),
        retain: r.retain,
        robable: r.robable,
        destroy: r.destroy,
    }
}
