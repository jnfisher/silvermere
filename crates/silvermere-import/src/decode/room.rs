//! WCCMP001 / NEWMP001 rooms: the `room` record.
//!
//! The record holds content and run-time state together; [`split_room`] maps it to the
//! schema's [`Room`] (content and state fields, as `silvermere-types` keeps them) and the
//! [`RoomState`] the writer stores beside it: floor items, hidden items, coins, owner,
//! `RF_SPECALIVE`, `monsters[15]`, `lastkill`, `foundmask`, `areacount`, `searchers`,
//! `nummonsters`. Not carried: `unk_0x008` (no known use), `changed` (cache byte), `unk_0x5F0`/`unk_0x5F5`; `unk_0x555` and `unk_0x5AF`
//! stay on the flat record for T-009.

use silvermere_db::repo::RoomState;
use silvermere_types::enums::{ExitType, GangHouse, RoomType};
use silvermere_types::flags::RoomFlags;
use silvermere_types::ids::{
    ActiveMonsterId, ItemNo, MonsterNo, RoomId, RoomKey, ShopNo, TextBlockNo,
};
use silvermere_types::records::{Exit, Room, RoomItem, layout};

use super::{Raw, Text, coins_named, record};

record! {
    /// The `room` record, WCCMP001: every member, in record order.
    pub struct RoomRecord = layout::ROOM {
        mapnum: u32,
        roomnum: u32,
        unk_0x008: Raw<253>,
        name: Text<53>,
        desc: [Text<71>; 7],
        mapfile: Text<13>,
        exits: [u32; 10],
        exittypes: [i16; 10],
        exitpar1s: [u32; 10],
        exitpar2s: [i16; 10],
        exitpar3s: [i32; 10],
        exitpar4s: [i32; 10],
        monsters: [u32; 15],
        type_ = "type": i16,
        shopnum: i32,
        owner: Text<30>,
        minlevel: i16,
        maxlevel: i16,
        genmonster: i32,
        light: i16,
        ganghouse: u16,
        items: [i32; 17],
        itemuses: [i16; 17],
        hiditems: [i32; 15],
        hiduses: [i16; 15],
        runic: u32,
        platinum: u32,
        gold: u32,
        silver: u32,
        copper: u32,
        hidrunic: u32,
        hidplatinum: u32,
        hidgold: u32,
        hidsilver: u32,
        hidcopper: u32,
        maxmonsters: u8,
        unk_0x555: i16,
        mongroup: i16,
        lastkill: u16,
        flags: u32,
        foundmask: u32,
        deathroom: u32,
        huproom: u32,
        itemcounts: [i16; 17],
        hidcounts: [i16; 15],
        cmdtext: i32,
        unk_0x5AF: i32,
        regendelay: i16,
        areamax: i16,
        areacount: i16,
        ctlroom: u32,
        specmon: i32,
        placed: [i32; 10],
        protmsg: i32,
        changed: u8,
        picktrap: i16,
        unk_0x5F0: u8,
        roomspell: i32,
        unk_0x5F5: u8,
        searchers: u8,
        nummonsters: u8,
    }
}

impl RoomRecord {
    /// Key 0: `(mapnum, roomnum)` from the record itself, never from the file name.
    pub fn key(&self) -> RoomKey {
        RoomKey::new(self.mapnum, self.roomnum)
    }
}

/// The schema's room and its initial state (the `.VIR` is the virgin state).
pub fn split_room(r: &RoomRecord) -> (Room, RoomState) {
    let room = Room {
        key: r.key(),
        name: r.name.trimmed(),
        desc: r.desc.clone().map(|t| t.trimmed()),
        mapfile: r.mapfile.trimmed(),
        exits: std::array::from_fn(|d| Exit {
            target: RoomId(r.exits[d]),
            kind: ExitType::from_code(r.exittypes[d] as u16),
            par1: r.exitpar1s[d],
            par2: r.exitpar2s[d],
            par3: r.exitpar3s[d],
            par4: r.exitpar4s[d],
        }),
        monsters: r.monsters.map(|m| ActiveMonsterId(m as i32)),
        room_type: RoomType::from_code(r.type_ as u16),
        shopnum: ShopNo(r.shopnum),
        owner: r.owner.trimmed(),
        minlevel: r.minlevel,
        maxlevel: r.maxlevel,
        genmonster: MonsterNo(r.genmonster),
        light: r.light,
        ganghouse: GangHouse::from_code(r.ganghouse),
        items: std::array::from_fn(|i| RoomItem {
            item: ItemNo(r.items[i]),
            uses: r.itemuses[i],
            count: r.itemcounts[i],
        }),
        hiditems: std::array::from_fn(|i| RoomItem {
            item: ItemNo(r.hiditems[i]),
            uses: r.hiduses[i],
            count: r.hidcounts[i],
        }),
        coins: coins_named(r.runic, r.platinum, r.gold, r.silver, r.copper),
        hidcoins: coins_named(
            r.hidrunic,
            r.hidplatinum,
            r.hidgold,
            r.hidsilver,
            r.hidcopper,
        ),
        maxmonsters: r.maxmonsters,
        mongroup: r.mongroup,
        lastkill: r.lastkill,
        flags: RoomFlags::from_bits_retain(r.flags),
        foundmask: r.foundmask,
        deathroom: RoomId(r.deathroom),
        huproom: RoomId(r.huproom),
        cmdtext: TextBlockNo(r.cmdtext),
        regendelay: r.regendelay,
        areamax: r.areamax,
        areacount: r.areacount,
        ctlroom: RoomId(r.ctlroom),
        specmon: MonsterNo(r.specmon),
        placed: r.placed.map(ItemNo),
        protmsg: TextBlockNo(r.protmsg),
        picktrap: r.picktrap,
        roomspell: r.roomspell,
        searchers: r.searchers,
        nummonsters: r.nummonsters,
    };
    let state = RoomState::of(&room);
    (room, state)
}
