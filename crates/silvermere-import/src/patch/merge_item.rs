//! The database update's item merge.

use crate::decode::ItemRecord;

/// Merges an item update. The ability, class, race and negated-spell arrays, the name and
/// the six plus three description lines, then every other definition member are compared;
/// when one differs all of them are copied. Kept: `number` (the key), `active` (copies in
/// play), `unk_0x008`, `unk_0x3bf` and `dirty`. Returns whether the item was copied.
pub fn merge_item(old: &mut ItemRecord, new: &ItemRecord) -> bool {
    let changed = old.abilities != new.abilities
        || old.abilvals != new.abilvals
        || old.classes != new.classes
        || old.races != new.races
        || old.negates != new.negates
        || old.name != new.name
        || old.desc != new.desc
        || old.hiddesc != new.hiddesc
        || old.level != new.level
        || old.limit != new.limit
        || old.weight != new.weight
        || old.type_ != new.type_
        || old.charges != new.charges
        || old.energy != new.energy
        || old.cost != new.cost
        || old.currency != new.currency
        || old.retain != new.retain
        || old.robable != new.robable
        || old.destroy != new.destroy
        || old.unk_0x338 != new.unk_0x338
        || old.unk_0x33a != new.unk_0x33a
        || old.unk_0x33c != new.unk_0x33c
        || old.mindmg != new.mindmg
        || old.maxdmg != new.maxdmg
        || old.ac != new.ac
        || old.wpntype != new.wpntype
        || old.armtype != new.armtype
        || old.worn != new.worn
        || old.accuracy != new.accuracy
        || old.dr != new.dr
        || old.gettable != new.gettable
        || old.strreq != new.strreq
        || old.unk_0x3a1 != new.unk_0x3a1
        || old.unk_0x3a5 != new.unk_0x3a5
        || old.unk_0x3a9 != new.unk_0x3a9
        || old.cash != new.cash
        || old.unk_0x3d7 != new.unk_0x3d7
        || old.speed != new.speed
        || old.hitmsg != new.hitmsg
        || old.missmsg != new.missmsg
        || old.textnum != new.textnum
        || old.unk_0x414 != new.unk_0x414
        || old.unk_0x418 != new.unk_0x418
        || old.unk_0x41c != new.unk_0x41c
        || old.nodrop != new.nodrop
        || old.destroymsg != new.destroymsg;
    if !changed {
        return false;
    }
    let keep = (
        old.number,
        old.active,
        old.unk_0x008.clone(),
        old.unk_0x3bf.clone(),
        old.dirty,
    );
    *old = new.clone();
    (
        old.number,
        old.active,
        old.unk_0x008,
        old.unk_0x3bf,
        old.dirty,
    ) = keep;
    true
}
