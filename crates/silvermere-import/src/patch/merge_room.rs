//! The database update's room merge.
//!
//! A room is never copied whole: it holds run-time state (monsters, floor items and coins,
//! door states, found hidden items). Only the designer's fields are compared and copied,
//! one by one. Kept from the old record whatever the update says: `monsters[]`,
//! `nummonsters`, floor `items`/`itemuses`/`itemcounts`, the floor and hidden coins,
//! `owner`, `lastkill`, `foundmask`, `areacount`, `searchers`, `hidcounts`, the flags other
//! than Protected, Patrol, Owned and Gang House, and the `unk_0x008`, `unk_0x5F0`,
//! `unk_0x5F5` spans.

use crate::decode::{ItemRecord, RoomRecord};

/// `RF_PROTECTED`.
const RF_PROTECTED: u32 = 0x01;
/// `RF_PATROL`.
const RF_PATROL: u32 = 0x02;
/// `RF_OWNED`.
const RF_OWNED: u32 = 0x04;
/// `RF_GANGHOUSE`.
const RF_GANGHOUSE: u32 = 0x40;

/// Exit types the merge treats specially.
const EXIT_SPELL: i16 = 1;
const EXIT_KEY: i16 = 2;
const EXIT_HIDDEN: i16 = 6;
const EXIT_DOOR: i16 = 7;
const EXIT_TRAP: i16 = 9;
const EXIT_GATE: i16 = 11;
const EXIT_TIMED: i16 = 16;
const EXIT_USERCOUNT: i16 = 18;
const EXIT_SPELLTRAP: i16 = 24;

/// `EXLOCKED`: the locked state of a key exit's `exitpar2s` and a door's or gate's
/// `exitpar1s`.
pub const EXLOCKED: i32 = 2;

/// Merges a room update. `item` looks an item up in the item table as patched so far.
/// Returns whether the room changed (an exit, the placed items, a hidden item or a flag to
/// update, or a design field differing).
pub fn merge_room<'i>(
    old: &mut RoomRecord,
    new: &RoomRecord,
    item: impl Fn(i32) -> Option<&'i ItemRecord>,
) -> bool {
    let mut changed = false;

    // Exits: destination and type, then the parameters for that type leaving out the
    // run-time one; the loop stops at the first change.
    for i in 0..old.exits.len() {
        if changed {
            break;
        }
        if new.exits[i] != old.exits[i] || new.exittypes[i] != old.exittypes[i] {
            changed = true;
            continue;
        }
        let p1 = new.exitpar1s[i] != old.exitpar1s[i];
        let p2 = new.exitpar2s[i] != old.exitpar2s[i];
        let p3 = new.exitpar3s[i] != old.exitpar3s[i];
        let p4 = new.exitpar4s[i] != old.exitpar4s[i];
        changed = match new.exittypes[i] {
            // Leave out exitpar2s (lock, trap state)
            EXIT_SPELL | EXIT_KEY | EXIT_TRAP | EXIT_USERCOUNT | EXIT_SPELLTRAP => p1 || p3 || p4,
            // Leave out exitpar3s
            EXIT_TIMED => p1 || p2 || p4,
            // Leave out exitpar1s (lock, found flags)
            EXIT_HIDDEN | EXIT_DOOR | EXIT_GATE => p2 || p3 || p4,
            // Every other type: all four
            _ => p1 || p2 || p3 || p4,
        };
    }

    // Placed items.
    if !changed && new.placed != old.placed {
        changed = true;
    }

    // Any hidden item in the update means work to do.
    if !changed && new.hiditems.iter().any(|&h| h != 0) {
        changed = true;
    }

    // Protected, Patrol and Owned follow the update both ways; Gang House is only cleared.
    // These change the old record at once.
    for bit in [RF_PROTECTED, RF_PATROL, RF_OWNED] {
        if new.flags & bit != old.flags & bit {
            old.flags = (old.flags & !bit) | (new.flags & bit);
            changed = true;
        }
    }
    if old.flags & RF_GANGHOUSE != 0 && new.flags & RF_GANGHOUSE == 0 {
        old.flags &= !RF_GANGHOUSE;
        changed = true;
    }

    // The remaining design fields.
    let design_differs = old.name != new.name
        || old.desc != new.desc
        || old.mapfile != new.mapfile
        || old.type_ != new.type_
        || old.shopnum != new.shopnum
        || old.minlevel != new.minlevel
        || old.maxlevel != new.maxlevel
        || old.genmonster != new.genmonster
        || old.light != new.light
        || old.ganghouse != new.ganghouse
        || old.maxmonsters != new.maxmonsters
        || old.unk_0x555 != new.unk_0x555
        || old.mongroup != new.mongroup
        || old.deathroom != new.deathroom
        || old.huproom != new.huproom
        || old.cmdtext != new.cmdtext
        || old.unk_0x5AF != new.unk_0x5AF
        || old.roomspell != new.roomspell
        || old.picktrap != new.picktrap
        || old.regendelay != new.regendelay
        || old.areamax != new.areamax
        || old.ctlroom != new.ctlroom
        || old.protmsg != new.protmsg
        || old.specmon != new.specmon;
    if !(changed || design_differs) {
        return false;
    }

    // Exits: the destination always; a changed type takes all four parameters; the same
    // type relocks keys, doors and gates, keeps exitpar2s of spell and user-count
    // exits, and takes all four otherwise.
    for i in 0..old.exits.len() {
        old.exits[i] = new.exits[i];
        let same_type = old.exittypes[i] == new.exittypes[i];
        old.exittypes[i] = new.exittypes[i];
        old.exitpar1s[i] = new.exitpar1s[i];
        old.exitpar3s[i] = new.exitpar3s[i];
        old.exitpar4s[i] = new.exitpar4s[i];
        if !same_type {
            old.exitpar2s[i] = new.exitpar2s[i];
            continue;
        }
        match new.exittypes[i] {
            // Key exit: locked again
            EXIT_KEY => old.exitpar2s[i] = EXLOCKED as i16,
            // Keep exitpar2s
            EXIT_SPELL | EXIT_USERCOUNT => {}
            // Door or gate: locked again
            EXIT_DOOR | EXIT_GATE => {
                old.exitpar1s[i] = EXLOCKED as u32;
                old.exitpar2s[i] = new.exitpar2s[i];
            }
            // All four from the update
            _ => old.exitpar2s[i] = new.exitpar2s[i],
        }
    }

    // Placed items.
    old.placed = new.placed;

    // Hidden items: each non-gettable hidden item of the update must be in the room;
    // looking for it empties slots whose item is gone or is gettable without a copy limit;
    // if it is not there it goes in the first free slot.
    for i in 0..new.hiditems.len() {
        let wanted = new.hiditems[i];
        if wanted == 0 {
            continue;
        }
        match item(wanted) {
            Some(it) if it.gettable == 0 => {}
            _ => continue,
        }
        let mut found = false;
        for j in 0..old.hiditems.len() {
            if found {
                break;
            }
            let have = old.hiditems[j];
            if have == 0 {
                continue;
            }
            if have == wanted {
                found = true;
            } else if item(have).is_none_or(|it| it.gettable != 0 && it.limit == 0) {
                // Gone, or unlimited loot: empty the slot (-1 uses = none)
                old.hiditems[j] = 0;
                old.hiduses[j] = -1;
            }
        }
        if !found && let Some(j) = old.hiditems.iter().position(|&h| h == 0) {
            // The first free hidden slot
            old.hiditems[j] = wanted;
            old.hiduses[j] = new.hiduses[i];
        }
    }

    // Name, description lines, map file and the other design fields.
    old.name = new.name.clone();
    old.desc = new.desc.clone();
    old.mapfile = new.mapfile.clone();
    old.type_ = new.type_;
    old.shopnum = new.shopnum;
    old.minlevel = new.minlevel;
    old.maxlevel = new.maxlevel;
    old.genmonster = new.genmonster;
    old.light = new.light;
    old.ganghouse = new.ganghouse;
    old.maxmonsters = new.maxmonsters;
    old.unk_0x555 = new.unk_0x555;
    old.mongroup = new.mongroup;
    old.deathroom = new.deathroom;
    old.huproom = new.huproom;
    old.cmdtext = new.cmdtext;
    old.unk_0x5AF = new.unk_0x5AF;
    old.regendelay = new.regendelay;
    old.areamax = new.areamax;
    old.ctlroom = new.ctlroom;
    old.specmon = new.specmon;
    old.protmsg = new.protmsg;
    old.roomspell = new.roomspell;
    old.picktrap = new.picktrap;
    // The merge also marks the room for writing back (`changed`); not carried.
    true
}
