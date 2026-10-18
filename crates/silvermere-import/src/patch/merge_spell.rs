//! The database update's spell merge.

use crate::decode::{Record, SpellRecord};

/// Merges a spell update: when any byte of the 253-byte record differs, the whole record is
/// copied. Returns whether it was.
pub fn merge_spell(old: &mut SpellRecord, new: &SpellRecord) -> bool {
    if old.encode() == new.encode() {
        return false;
    }
    *old = new.clone();
    true
}
