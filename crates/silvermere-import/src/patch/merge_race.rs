//! The database update's race and class merges.
//!
//! Both compare a fixed list of members and, when one differs, copy the whole update
//! record over the old one; members outside the list (the `unk_*` tails, the
//! cache byte) come along with the copy but do not by themselves cause it.

use super::c_str_eq;
use crate::decode::{ClassRecord, RaceRecord};

/// Merges a race update: the ten ability slots (number and value), then the name,
/// `unk_0x05C` (compared as NUL-terminated text), the six minimum and six maximum stats,
/// `hpbonus`, `maxenergy`, `restored`, `exp` and `basecp`. Anything different: the whole
/// record is copied. Returns whether it was.
pub fn merge_race(old: &mut RaceRecord, new: &RaceRecord) -> bool {
    let changed = old.abilities != new.abilities
        || old.abilvals != new.abilvals
        || old.name != new.name
        || !c_str_eq(&old.unk_0x05C.0, &new.unk_0x05C.0)
        || old.minint != new.minint
        || old.minwil != new.minwil
        || old.minstr != new.minstr
        || old.minhea != new.minhea
        || old.minagi != new.minagi
        || old.mincha != new.mincha
        || old.maxint != new.maxint
        || old.maxwil != new.maxwil
        || old.maxstr != new.maxstr
        || old.maxhea != new.maxhea
        || old.maxagi != new.maxagi
        || old.maxcha != new.maxcha
        || old.hpbonus != new.hpbonus
        || old.maxenergy != new.maxenergy
        || old.restored != new.restored
        || old.exp != new.exp
        || old.basecp != new.basecp;
    if changed {
        *old = new.clone();
        // The merge marks the record for writing back; the import has no cache, so the
        // `modified` byte stays as the image holds it.
    }
    changed
}

/// Merges a class update: the ten ability slots, then the name, `unk_0x05E` (as
/// NUL-terminated text), `basehp`, `hplevel`, `exp`, `attmult`, `maxenergy`, `restored`,
/// `magery`, `magelvl`, `weapon`, `armour`, `combat` and `titles`. Anything different: the
/// whole record is copied.
pub fn merge_class(old: &mut ClassRecord, new: &ClassRecord) -> bool {
    let changed = old.abilities != new.abilities
        || old.abilvals != new.abilvals
        || old.name != new.name
        || !c_str_eq(&old.unk_0x05E.0, &new.unk_0x05E.0)
        || old.basehp != new.basehp
        || old.hplevel != new.hplevel
        || old.exp != new.exp
        || old.attmult != new.attmult
        || old.maxenergy != new.maxenergy
        || old.restored != new.restored
        || old.magery != new.magery
        || old.magelvl != new.magelvl
        || old.weapon != new.weapon
        || old.armour != new.armour
        || old.combat != new.combat
        || old.titles != new.titles;
    if changed {
        *old = new.clone();
    }
    changed
}
