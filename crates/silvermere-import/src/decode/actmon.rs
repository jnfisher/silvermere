//! WCCACMSR active monsters: the `actmon` record, state (empty in the
//! virgin file). `cash[5]` is runic first like the known monster's.
//! Not carried: `dirty` (cache byte) and the `unk_*` members.

use silvermere_types::enums::{Alignment, GroupRole, MonsterType};
use silvermere_types::flags::ActiveMonsterFlags;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, MonsterNo, RoomId, SpellNo};
use silvermere_types::records::{ActiveMonster, ActiveSpell, layout};

use super::{DecodeError, LegacyClock, Raw, Text, coins_runic_first, record};

record! {
    /// The `actmon` record, WCCACMSR: every member, in record order.
    pub struct ActmonRecord = layout::ACTMON {
        number: i32,
        knownnum: i32,
        level: u32,
        map: i32,
        room: i32,
        poison: i16,
        energy: u16,
        hp: i16,
        following: Text<30>,
        trail: [i32; 10],
        hunters: [i32; 10],
        hunting: i32,
        unk_0x08C: i16,
        name: Text<30>,
        wornitem: i32,
        weapon: i32,
        items: [i32; 10],
        itemuses: [i16; 10],
        cash: [u32; 5],
        maxhp: i16,
        alignment: i16,
        followpct: i16,
        dr: i16,
        ac: i16,
        exp: i32,
        maxenergy: i16,
        friendly: u8,
        unk_0x115: Raw<6>,
        genroom: i32,
        inactive: u8,
        flags: u16,
        unk_0x122: Raw<2>,
        montype: i16,
        target: i16,
        hpregen: i16,
        lastdir: i16,
        unk_0x12C: i16,
        unk_0x12E: i16,
        unk_0x130: u8,
        unk_0x131: i32,
        dirty: u8,
        lastupd: i32,
        grouprole: i16,
        spells: [i16; 5],
        spellvals: [i16; 5],
        spelldurs: [i16; 5],
        downrounds: i16,
        created: i16,
        unk_0x15E: Raw<36>,
    }
}

/// The schema's active monster. `created` holds only a `now()` time (set when the
/// monster is generated), so it is read on the import day in `clock`'s zone.
pub fn to_active_monster(
    r: &ActmonRecord,
    clock: &LegacyClock,
) -> Result<ActiveMonster, DecodeError> {
    let created = clock.at(
        "actmon",
        format_args!("number {}", r.number),
        "created",
        clock.today,
        r.created as u16,
    )?;
    Ok(ActiveMonster {
        number: ActiveMonsterId(r.number),
        knownnum: MonsterNo(r.knownnum),
        level: r.level,
        map: MapId(r.map as u32),
        room: RoomId(r.room as u32),
        poison: r.poison,
        energy: r.energy,
        hp: r.hp,
        following: r.following.trimmed(),
        trail: r.trail.map(|t| RoomId(t as u32)),
        hunters: r.hunters.map(ActiveMonsterId),
        hunting: ActiveMonsterId(r.hunting),
        name: r.name.trimmed(),
        wornitem: ItemNo(r.wornitem),
        weapon: ItemNo(r.weapon),
        items: std::array::from_fn(|i| (ItemNo(r.items[i]), r.itemuses[i])),
        cash: coins_runic_first(r.cash),
        maxhp: r.maxhp,
        alignment: Alignment::from_code(r.alignment as u16),
        followpct: r.followpct,
        dr: r.dr,
        ac: r.ac,
        exp: r.exp,
        maxenergy: r.maxenergy,
        friendly: r.friendly,
        genroom: RoomId(r.genroom as u32),
        inactive: r.inactive,
        flags: ActiveMonsterFlags::from_bits_retain(r.flags),
        montype: MonsterType::from_code(r.montype as u16),
        target: r.target,
        hpregen: r.hpregen,
        lastdir: r.lastdir,
        lastupd: r.lastupd,
        grouprole: GroupRole::from_code(r.grouprole as u16),
        spells: std::array::from_fn(|i| ActiveSpell {
            spell: SpellNo(r.spells[i]),
            value: r.spellvals[i],
            rounds: r.spelldurs[i],
        }),
        downrounds: r.downrounds,
        created,
    })
}
