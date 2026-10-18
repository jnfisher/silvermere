//! The `monster` record (WCCKNMSR, known monsters) and the `actmon` record (WCCACMSR, active ones).

use serde::Serialize;

use crate::enums::{Alignment, Gender, GroupRole, MonsterAttackType, MonsterType};
use crate::flags::ActiveMonsterFlags;
use crate::ids::{
    ActiveMonsterId, ItemNo, MapId, MessageNo, MonsterNo, RoomId, SpellNo, TextBlockNo,
};
use crate::records::layout::{self, Layout};
use crate::records::{AbilitySlot, Coins};
use crate::sizes::{NAMTRAIL, NMONATTACKS, NMONITEMS, NMONSPELLS, NRECABILS};

/// An item a known monster may carry: `items[i]`, `itemuses[i]`, `itempcts[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MonsterItem {
    /// +0x0BA item carried.
    pub item: ItemNo,
    /// +0x0E2 uses of it.
    pub uses: i16,
    /// +0x0F6 percent chance it is carried.
    pub pct: u8,
}

/// One attack slot of a known monster: the `atk*`, `*msgs` and `hitspells` arrays at one index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MonsterAttack {
    /// +0x11E attack type.
    pub kind: MonsterAttackType,
    /// +0x123 accuracy; the spell number for a spell attack.
    pub accuracy: i16,
    /// +0x12D cumulative percent chance.
    pub pct: u8,
    /// +0x132 minimum damage; cast percent for a spell attack.
    pub min: i16,
    /// +0x13C maximum damage; cast level for a spell attack.
    pub max: i16,
    /// +0x146 WCCMSG: hit.
    pub hitmsg: MessageNo,
    /// +0x15A WCCMSG: deflect / dodge.
    pub defmsg: MessageNo,
    /// +0x16E WCCMSG: miss.
    pub missmsg: MessageNo,
    /// +0x182 energy of the attack.
    pub energy: i16,
    /// +0x193 spell cast on a hit.
    pub hitspell: SpellNo,
}

/// A spell a known monster casts after combat rounds: `spells`, `spellpcts`, `spelllvls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MonsterSpell {
    /// +0x1AB spell number.
    pub spell: SpellNo,
    /// +0x1B5 cumulative percent chance.
    pub pct: u8,
    /// +0x1BA cast level.
    pub level: u8,
}

/// A known monster: the `monster` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnownMonster {
    /// +0x000 known-monster number (key 0).
    pub number: MonsterNo,
    /// +0x036 name.
    pub name: String,
    /// +0x054 generation type.
    pub montype: MonsterType,
    /// +0x056 level; experience awarded is `exp * level`.
    pub level: u32,
    /// +0x05A generation index.
    pub genindex: i16,
    /// +0x05C item worn (abilities only).
    pub wornitem: ItemNo,
    /// +0x060 weapon item.
    pub weapon: ItemNo,
    /// +0x064 damage resistance.
    pub dr: i16,
    /// +0x066 armour class.
    pub ac: i16,
    /// +0x068 group number.
    pub group: i16,
    /// +0x06A percent chance to follow a user.
    pub followpct: i16,
    /// +0x06C magic resistance.
    pub magicres: i16,
    /// +0x06E defence against backstab.
    pub bsdefense: u8,
    /// +0x06F experience.
    pub exp: i32,
    /// +0x073 hit points.
    pub maxhp: i16,
    /// +0x075 energy per round.
    pub energy: u16,
    /// +0x077 hit points per slow tick.
    pub hpregen: i16,
    /// +0x079 ability numbers with +0x08D their values.
    pub abilities: [AbilitySlot; NRECABILS],
    /// +0x0A1 max alive, 0 = no limit.
    pub maxalive: u16,
    /// +0x0A3 alive now.
    pub curalive: u16,
    /// +0x0A5 group role.
    pub grouprole: GroupRole,
    /// +0x0A7 members a leader takes along.
    pub maxfollow: u8,
    /// +0x0A8 affected by "Kill Dead".
    pub undead: u8,
    /// +0x0A9 alignment.
    pub alignment: Alignment,
    /// +0x0AD regeneration time, hours.
    pub regenhrs: u8,
    /// +0x0AE/+0x0B0 when last killed (`killdate` DOS date and `killtime` DOS time
    /// merged), Unix seconds UTC; 0 = never.
    pub killed_at: i64,
    /// +0x0B2 WCCMSG: arrival.
    pub arrivemsg: MessageNo,
    /// +0x0B6 WCCMSG: death.
    pub deathmsg: MessageNo,
    /// +0x0BA items carried.
    pub items: [MonsterItem; NMONITEMS],
    /// +0x100 most coins carried (`cash[5]`, runic first in the record).
    pub cash: Coins,
    /// +0x114 text block for ASK.
    pub talktext: TextBlockNo,
    /// +0x118 user level needed to charm.
    pub charmlvl: u16,
    /// +0x11A random-name list.
    pub namelist: i32,
    /// The five attack slots.
    pub attacks: [MonsterAttack; NMONATTACKS],
    /// +0x18C text block: random event.
    pub randtext: TextBlockNo,
    /// +0x190 resistance to enslaving.
    pub charmres: i16,
    /// +0x19D spell cast when killed.
    pub deathspell: SpellNo,
    /// +0x1A9 spell cast when generated.
    pub createspell: SpellNo,
    /// The spells cast after combat rounds.
    pub spells: [MonsterSpell; NMONSPELLS],
    /// +0x1BF description, four lines.
    pub desc: [String; 4],
    /// +0x2DB 0 It, 1 He, 2 She.
    pub gender: Gender,
}

impl KnownMonster {
    /// Byte layout of the `monster` record.
    pub const LAYOUT: &'static Layout = &layout::MONSTER;
    /// The `monster` record length: 732 bytes.
    pub const RECORD_LEN: usize = 732;
}

/// A spell in effect on an active monster: `spells`, `spellvals`, `spelldurs` at one index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActiveSpell {
    /// +0x13C spell number.
    pub spell: SpellNo,
    /// +0x146 its value.
    pub value: i16,
    /// +0x150 rounds left.
    pub rounds: i16,
}

/// An active monster: the `actmon` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActiveMonster {
    /// +0x000 active number (key 0).
    pub number: ActiveMonsterId,
    /// +0x004 known-monster number.
    pub knownnum: MonsterNo,
    /// +0x008 copy of `monster.level`.
    pub level: u32,
    /// +0x00C map of the current room.
    pub map: MapId,
    /// +0x010 current room.
    pub room: RoomId,
    /// +0x014 poison per slow tick.
    pub poison: i16,
    /// +0x016 energy left this round.
    pub energy: u16,
    /// +0x018 hit points.
    pub hp: i16,
    /// +0x01A name of the user followed.
    pub following: String,
    /// +0x038 rooms passed, `[0]` latest.
    pub trail: [RoomId; NAMTRAIL],
    /// +0x060 active monsters tracking it.
    pub hunters: [ActiveMonsterId; 10],
    /// +0x088 active monster it tracks.
    pub hunting: ActiveMonsterId,
    /// +0x08E name.
    pub name: String,
    /// +0x0AC item worn.
    pub wornitem: ItemNo,
    /// +0x0B0 weapon item.
    pub weapon: ItemNo,
    /// +0x0B4 items carried with +0x0DC their uses.
    pub items: [(ItemNo, i16); NMONITEMS],
    /// +0x0F0 coins carried (`cash[5]`, runic first in the record).
    pub cash: Coins,
    /// +0x104 maximum hit points.
    pub maxhp: i16,
    /// +0x106 alignment.
    pub alignment: Alignment,
    /// +0x108 percent chance to follow.
    pub followpct: i16,
    /// +0x10A damage resistance.
    pub dr: i16,
    /// +0x10C armour class.
    pub ac: i16,
    /// +0x10E experience.
    pub exp: i32,
    /// +0x112 energy per round.
    pub maxenergy: i16,
    /// +0x114 "Friend" of the followed user.
    pub friendly: u8,
    /// +0x11B room where generated.
    pub genroom: RoomId,
    /// +0x11F rounds inactive.
    pub inactive: u8,
    /// +0x120 active monster flags.
    pub flags: ActiveMonsterFlags,
    /// +0x124 generation type.
    pub montype: MonsterType,
    /// +0x126 user being fought, -1 none.
    pub target: i16,
    /// +0x128 hit points per slow tick.
    pub hpregen: i16,
    /// +0x12A direction of last move, -1.
    pub lastdir: i16,
    /// +0x136 slow-update cycle last applied.
    pub lastupd: i32,
    /// +0x13A group role.
    pub grouprole: GroupRole,
    /// The spells in effect.
    pub spells: [ActiveSpell; NMONSPELLS],
    /// +0x15A rounds left knocked down.
    pub downrounds: i16,
    /// +0x15C when generated (a DOS time in the record), Unix seconds UTC.
    pub created: i64,
}

impl ActiveMonster {
    /// Byte layout of the `actmon` record.
    pub const LAYOUT: &'static Layout = &layout::ACTMON;
    /// The `actmon` record length: 386 bytes.
    pub const RECORD_LEN: usize = 386;
}
