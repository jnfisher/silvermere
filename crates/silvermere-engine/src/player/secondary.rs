//! The secondary stats.
//!
//! [`secondary`] is the pure part: every derived figure from the character's stats, level,
//! class, race and ability totals, read through T-032's [`AbilityLookup`]. [`recalculate`]
//! is the engine wrapper: it refreshes the ability-driven ("dynamic") fields first, merges the two
//! status words those abilities set, then writes the figures back under the [`CalcMode`] rules.
//!
//! The figures come out as the original shows them, wrapping included: every result is
//! cut to 16 bits (`as i16` here), so a figure above 32767 shows negative, and division
//! truncates toward zero ((42-50)/3 is -2). The one product that can leave 16 bits before its
//! division, (base Health - 50) * level / 16, is wrapped to 16 bits before dividing.
//!
//! Callers, by mode:
//!
//! | Mode | Callers |
//! |---|---|
//! | 0 [`CalcMode::Refill`] | `roll_stats`; entering the realm with no experience |
//! | 1 [`CalcMode::KeepMaxHp`] | the sysop's character edit |
//! | 2 [`CalcMode::KeepCurrent`] | everything else: `load_player`, death, suicide, spells, worn items and rooms, training and commands, entering with experience |

use silvermere_rules::abilities::{Ability, AbilityLookup, CharacterAbilities, dynamic_stats};
use silvermere_types::enums::Magery;
use silvermere_types::flags::{PlayerFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::records::{Character, Class, Race};

use crate::world::World;

/// The mode of the secondary-stats calculation: which current figures it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcMode {
    /// 0: recompute everything and fill hit points and mana to the new maximums.
    Refill = 0,
    /// 1: keep the maximum hit points; raise `maxmana`/`maxmr` to cover the current values
    /// Nothing is refilled.
    KeepMaxHp = 1,
    /// 2: recompute the maximums, leave current hit points and mana alone.
    KeepCurrent = 2,
}

/// Where a character's stealth comes from (`PF_CLASSSTEALTH`/`PF_RACESTEALTH`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StealthSource {
    None,
    Race,
    Class,
    Both,
}

/// The derived figures, before truncation to the record's field widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Secondary {
    pub maxwgt: i32,
    pub perception: i32,
    pub stealth: i32,
    pub stealth_source: StealthSource,
    pub thievery: i32,
    pub traps: i32,
    pub disarm: i32,
    pub picklocks: i32,
    pub tracking: i32,
    /// Already floored at 0: a maximum mana above 32767 counts as negative, so it shows
    /// as 0.
    pub maxmana: i32,
    pub spellcast: i32,
    pub maxmr: i32,
    /// The full formula; [`CalcMode::KeepMaxHp`] ignores it.
    pub maxhp: i32,
    pub mabase: i32,
    pub martial: i32,
}

/// The level figure of thievery, traps, picklocks and tracking: each level up to 15 counts
/// fully, each level past it half.
fn skill_level(level: i32) -> i32 {
    if level > 15 {
        (level - 15) / 2 + 15
    } else {
        level
    }
}

/// The figures of the secondary-stats calculation.
///
/// `c.critbonus` must already be the dynamic value ([`recalculate`] refreshes it first),
/// because martial arts adds it. `abilities` totals over the character's sources (the
/// ability value and has-ability look-ups).
pub fn secondary(
    c: &Character,
    class: &Class,
    race: &Race,
    abilities: &dyn AbilityLookup,
) -> Secondary {
    let intel = i32::from(c.intel);
    let wil = i32::from(c.wil);
    let str_ = i32::from(c.str);
    let agi = i32::from(c.agi);
    let cha = i32::from(c.cha);
    let level = i32::from(c.level);

    // 48 per point of strength, 36 more per point above 100.
    let mut maxwgt = str_ * 48;
    if str_ > 100 {
        maxwgt += (str_ - 100) * 36;
    }

    // one pass totals Perception and, into `extra`, Stealth.
    let (perc_abil, stealth_abil) = abilities.pair(Ability::Perception, Ability::Stealth);
    let perception = (intel * 5 + wil * 2 + cha) / 8 + perc_abil;

    let cls_stealth = abilities.has(Ability::ClassStealth);
    let rac_stealth = abilities.has(Ability::RaceStealth);
    let (stealth, stealth_source) = if cls_stealth || rac_stealth {
        // `lvl` is 2; 30 + (level-15)*2/2 past 15, else level*2.
        let lvl = 2;
        let tmp = if level > 15 {
            (level - 15) * lvl / 2 + lvl * 15
        } else {
            level * lvl
        };
        let mut s = agi / 4 + tmp + cha / 6 + intel / 8 + 20;
        let source = if rac_stealth && !cls_stealth {
            s -= 15;
            StealthSource::Race
        } else if cls_stealth && !rac_stealth {
            StealthSource::Class
        } else {
            s += 10;
            StealthSource::Both
        };
        // (the short field, then the floor).
        let s = i32::from((s + stealth_abil) as i16).max(0);
        (s, source)
    } else {
        (0, StealthSource::None)
    };

    let thievery = if abilities.has(Ability::Thievery) {
        let lvl = skill_level(level);
        let t = (agi + intel + cha + lvl * 24) / 6 + abilities.value(Ability::Thievery);
        i32::from(t as i16).max(0)
    } else {
        0
    };

    let (traps, disarm) = if abilities.has(Ability::FindTraps) {
        let lvl = skill_level(level);
        let base = (intel + agi + cha * 2 + lvl * 28) / 7;
        let (find, disarm_abil) = abilities.pair(Ability::FindTraps, Ability::DisarmTraps);
        let traps = base + find + abilities.value(Ability::FindTrapsVal);
        (
            i32::from(traps as i16).max(0),
            i32::from((base + disarm_abil) as i16).max(0),
        )
    } else {
        (0, 0)
    };

    let picklocks = if abilities.has(Ability::Picklocks) {
        let lvl = skill_level(level);
        let (p, val) = abilities.pair(Ability::Picklocks, Ability::PicklocksVal);
        let t = (agi + intel + lvl * 10) * 2 / 7 + p + val;
        i32::from(t as i16).max(0)
    } else {
        0
    };

    let tracking = if abilities.has(Ability::Tracking) {
        let lvl = skill_level(level);
        let t = (intel * 2 + wil + cha + lvl * 40) / 8 + abilities.value(Ability::Tracking);
        i32::from(t as i16).max(0)
    } else {
        0
    };

    // maximum mana and the spellcasting base by magery.
    let magelvl = i32::from(class.magelvl);
    // `manabonus`: the long field cut to the 16-bit int.
    let manabonus = i32::from(c.manabonus as i16);
    let caster_mana = || abilities.value(Ability::Mana) + (magelvl * level + 3) * 2 + manabonus;
    let (maxmana, castbase) = match class.magery {
        Magery::Mage => (caster_mana(), (intel * 3 + wil) / 6),
        Magery::Priest => (caster_mana(), (wil * 3 + intel) / 6),
        Magery::Druid => (caster_mana(), (intel + wil) / 3),
        Magery::Bard => (caster_mana(), (cha * 3 + wil) / 6),
        Magery::Kai => (level - 1, 500),
        _ => (0, -150), // no magery
    };
    // unsigned field, signed test.
    let maxmana = i32::from(maxmana as i16).max(0);

    // one pass totals Spellcasting and, into `extra`, Magic Resistance.
    let (cast_abil, mr_abil) = abilities.pair(Ability::Spellcasting, Ability::MagicResist);
    let spellcast = level * 2 + castbase + magelvl * 5 + cast_abil;
    let maxmr = (intel + wil * 3) / 4 + mr_abil;

    // base Health, not the buffed value.
    let bashea = i32::from(c.bashea);
    let tmp = i32::from(((bashea - 50) * level) as i16) / 16; // 16-bit product
    let maxhp = bashea / 2
        + level * i32::from(class.basehp)
        + level * i32::from(race.hpbonus)
        + tmp
        + i32::from(c.randomhp)
        + abilities.value(Ability::AlterHp)
        + i32::from(c.hpbonus as i16);

    // the martial-arts base, kept in 1..=75.
    let mabase = (level / 10 + (intel - 50) / 10 + (agi - 50) / 20 + (cha - 50) / 30).clamp(1, 75);
    let mut martial = mabase
        + i32::from(c.critbonus)
        + abilities.value(Ability::Dodge)
        + cha / 10
        + agi / 5
        + level / 5;
    if abilities.has(Ability::Jumpkick) {
        martial = (martial + level) * 2;
    }

    Secondary {
        maxwgt,
        perception,
        stealth,
        stealth_source,
        thievery,
        traps,
        disarm,
        picklocks,
        tracking,
        maxmana,
        spellcast,
        maxmr,
        maxhp,
        mabase,
        martial,
    }
}

/// The dynamic-stats refresh the secondary-stats calculation runs first: T-032's
/// [`dynamic_stats`] copied into the record, and the two status words merged as its doc
/// comment says: the ability-driven bits cleared, then the abilities set them.
pub fn apply_dynamic(world: &World, c: &mut Character) {
    let d = {
        let spells = |n| world.spells.get(&n);
        let items = |n| world.items.get(&n);
        dynamic_stats(&CharacterAbilities {
            character: c,
            race: world.races.get(&c.race),
            class: world.classes.get(&c.class),
            spells: &spells,
            items: &items,
        })
    };
    c.avbonus = d.avbonus as i16;
    c.acbonus = d.acbonus as i16;
    c.dmgbonus = d.dmgbonus as i16;
    c.critbonus = d.critbonus as i16;
    c.drbonus = d.drbonus as i16;
    c.drpct = d.drpct as i16;
    c.energyuse = d.energyuse as i16;
    c.shield = d.shield as i16;
    c.defmod = d.defmod as i16;
    c.healrate = d.healrate as i16;
    c.manarate = d.manarate as i16;
    c.roomlight = d.roomlight as i16;
    c.status = (c.status - PlayerStatus::FROMABILS) | d.status_from_abilities;
    c.status2 = (c.status2 - PlayerStatus2::FROMABILS) | d.status2_from_abilities;
}

/// The secondary stats on a character the caller holds: nothing happens when its class or
/// race record is missing.
pub fn recalculate(world: &World, c: &mut Character, mode: CalcMode) {
    let (Some(class), Some(race)) = (world.classes.get(&c.class), world.races.get(&c.race)) else {
        return;
    };
    apply_dynamic(world, c);
    let s = {
        let spells = |n| world.spells.get(&n);
        let items = |n| world.items.get(&n);
        let lookup = CharacterAbilities {
            character: c,
            race: Some(race),
            class: Some(class),
            spells: &spells,
            items: &items,
        };
        secondary(c, class, race, &lookup)
    };
    c.maxwgt = s.maxwgt as i16;
    c.perception = s.perception as i16;
    c.stealth = s.stealth as i16;
    match s.stealth_source {
        StealthSource::None => c.flags -= PlayerFlags::CLASSSTEALTH | PlayerFlags::RACESTEALTH,
        StealthSource::Race => {
            c.flags |= PlayerFlags::RACESTEALTH;
            c.flags -= PlayerFlags::CLASSSTEALTH;
        }
        StealthSource::Class => {
            c.flags |= PlayerFlags::CLASSSTEALTH;
            c.flags -= PlayerFlags::RACESTEALTH;
        }
        StealthSource::Both => c.flags |= PlayerFlags::CLASSSTEALTH | PlayerFlags::RACESTEALTH,
    }
    c.thievery = s.thievery as i16;
    c.traps = s.traps as i16;
    c.disarm = s.disarm as i16;
    c.picklocks = s.picklocks as i16;
    c.tracking = s.tracking as i16;
    c.maxmana = s.maxmana as u16;
    c.spellcast = s.spellcast as i16;
    c.maxmr = s.maxmr as u16;
    c.mr = c.maxmr;
    if mode != CalcMode::KeepMaxHp {
        c.maxhp = s.maxhp as i16;
    }
    match mode {
        // `mana` is a short and `maxmana` unsigned, so the comparison
        // is unsigned, as here.
        CalcMode::KeepMaxHp => {
            if c.mana as u16 > c.maxmana {
                c.maxmana = c.mana as u16;
            }
            if c.mr > c.maxmr {
                c.maxmr = c.mr;
            }
        }
        CalcMode::Refill => {
            c.mana = c.maxmana as i16;
            c.hp = c.maxhp;
        }
        CalcMode::KeepCurrent => {}
    }
    c.mabase = s.mabase as u8;
    c.martial = s.martial as i16;
}
