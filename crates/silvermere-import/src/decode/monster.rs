//! WCCKNMSR known monsters: the `monster` record.
//!
//! `curalive` (+0x0A3) and `killdate`/`killtime` (merged into one
//! `killed_at`) are state: the writer stores them in `monster_state` ([`MonsterState`]).
//! `cash[5]` (+0x100) is
//! runic first and becomes copper-first [`silvermere_types::records::Coins`]. Not carried:
//! `dirty` (cache byte) and the `unk_*` members, which stay on the flat record for T-009.

use silvermere_db::repo::MonsterState;
use silvermere_types::enums::{Alignment, Gender, GroupRole, MonsterAttackType, MonsterType};
use silvermere_types::ids::{ItemNo, MessageNo, MonsterNo, SpellNo, TextBlockNo};
use silvermere_types::records::{KnownMonster, MonsterAttack, MonsterItem, MonsterSpell, layout};

use super::{DecodeError, LegacyClock, Raw, Text, abilities, coins_runic_first, record};

record! {
    /// The `monster` record, WCCKNMSR: every member, in record order.
    pub struct MonsterRecord = layout::MONSTER {
        number: i32,
        unk_0x004: Raw<50>,
        name: Text<30>,
        montype: i16,
        level: u32,
        genindex: i16,
        wornitem: i32,
        weapon: i32,
        dr: i16,
        ac: i16,
        group: i16,
        followpct: i16,
        magicres: i16,
        bsdefense: u8,
        exp: i32,
        maxhp: i16,
        energy: u16,
        hpregen: i16,
        abilities: [i16; 10],
        abilvals: [i16; 10],
        maxalive: u16,
        curalive: u16,
        grouprole: i16,
        maxfollow: u8,
        undead: u8,
        alignment: i16,
        unk_0x0AB: i16,
        regenhrs: u8,
        killdate: u16,
        killtime: u16,
        arrivemsg: i32,
        deathmsg: i32,
        items: [i32; 10],
        itemuses: [i16; 10],
        itempcts: [u8; 10],
        cash: [u32; 5],
        talktext: i32,
        charmlvl: u16,
        namelist: i32,
        atktypes: [u8; 5],
        atkaccs: [i16; 5],
        atkpcts: [u8; 5],
        atkmins: [i16; 5],
        atkmaxs: [i16; 5],
        hitmsgs: [i32; 5],
        defmsgs: [i32; 5],
        missmsgs: [i32; 5],
        atkenergies: [i16; 5],
        randtext: i32,
        charmres: i16,
        unk_0x192: u8,
        hitspells: [i16; 5],
        deathspell: i16,
        unk_0x19F: i16,
        unk_0x1A1: i16,
        unk_0x1A3: u8,
        unk_0x1A4: i32,
        dirty: u8,
        createspell: i16,
        spells: [i16; 5],
        spellpcts: [u8; 5],
        spelllvls: [u8; 5],
        desc: [Text<71>; 4],
        gender: u8,
    }
}

/// The schema's known monster and its initial state; `killdate`/`killtime` become one
/// `killed_at` in `clock`'s zone.
pub fn split_monster(
    r: &MonsterRecord,
    clock: &LegacyClock,
) -> Result<(KnownMonster, MonsterState), DecodeError> {
    let killed_at = clock.at(
        "monster",
        format_args!("number {}", r.number),
        "killdate",
        r.killdate,
        r.killtime,
    )?;
    let m = KnownMonster {
        number: MonsterNo(r.number),
        name: r.name.trimmed(),
        montype: MonsterType::from_code(r.montype as u16),
        level: r.level,
        genindex: r.genindex,
        wornitem: ItemNo(r.wornitem),
        weapon: ItemNo(r.weapon),
        dr: r.dr,
        ac: r.ac,
        group: r.group,
        followpct: r.followpct,
        magicres: r.magicres,
        bsdefense: r.bsdefense,
        exp: r.exp,
        maxhp: r.maxhp,
        energy: r.energy,
        hpregen: r.hpregen,
        abilities: abilities(&r.abilities, &r.abilvals),
        maxalive: r.maxalive,
        curalive: r.curalive,
        grouprole: GroupRole::from_code(r.grouprole as u16),
        maxfollow: r.maxfollow,
        undead: r.undead,
        alignment: Alignment::from_code(r.alignment as u16),
        regenhrs: r.regenhrs,
        killed_at,
        arrivemsg: MessageNo(r.arrivemsg),
        deathmsg: MessageNo(r.deathmsg),
        items: std::array::from_fn(|i| MonsterItem {
            item: ItemNo(r.items[i]),
            uses: r.itemuses[i],
            pct: r.itempcts[i],
        }),
        cash: coins_runic_first(r.cash),
        talktext: TextBlockNo(r.talktext),
        charmlvl: r.charmlvl,
        namelist: r.namelist,
        attacks: std::array::from_fn(|i| MonsterAttack {
            kind: MonsterAttackType::from_code(u16::from(r.atktypes[i])),
            accuracy: r.atkaccs[i],
            pct: r.atkpcts[i],
            min: r.atkmins[i],
            max: r.atkmaxs[i],
            hitmsg: MessageNo(r.hitmsgs[i]),
            defmsg: MessageNo(r.defmsgs[i]),
            missmsg: MessageNo(r.missmsgs[i]),
            energy: r.atkenergies[i],
            hitspell: SpellNo(r.hitspells[i]),
        }),
        randtext: TextBlockNo(r.randtext),
        charmres: r.charmres,
        deathspell: SpellNo(r.deathspell),
        createspell: SpellNo(r.createspell),
        spells: std::array::from_fn(|i| MonsterSpell {
            spell: SpellNo(r.spells[i]),
            pct: r.spellpcts[i],
            level: r.spelllvls[i],
        }),
        desc: r.desc.clone().map(|t| t.trimmed()),
        gender: Gender::from_code(u16::from(r.gender)),
    };
    let state = MonsterState::of(&m);
    Ok((m, state))
}
