//! The database update's message, social action and text block merges.

use silvermere_types::records::LayoutSet;

use crate::decode::{ActionRecord, MudmsgRecord, Record, TextblkRecord};

/// `AT_KEEP`: an action the database update does not replace.
pub const AT_KEEP: i16 = 0x40;

/// Merges a message update: when one of the three texts differs all three are copied. Kept: `number`, `modified`,
/// `unk_0x0FB`. Returns whether they were.
pub fn merge_message(old: &mut MudmsgRecord, new: &MudmsgRecord) -> bool {
    if old.msgs == new.msgs {
        return false;
    }
    old.msgs = new.msgs.clone();
    true
}

/// Merges a social action update: the update record replaces the action unless the
/// existing one carries `AT_KEEP` in `targets`.
/// Returns whether it was replaced.
pub fn merge_action(old: &mut ActionRecord, new: &ActionRecord) -> bool {
    if old.targets & AT_KEEP != 0 {
        return false;
    }
    *old = new.clone();
    true
}

/// Merges a text block update, with one deliberate deviation [I]. The original compares the
/// 2022-byte images and writes the update record over the block (with the host's
/// `dupdbtv`). Here only the first `carried` bytes of the image are the update's: the stock
/// records carry 2010 of the 2022, and how many bytes `dupdbtv` writes on the
/// variable-length `WCCTEXT` is not settled. So the carried bytes are compared and copied and the rest of the stored block is kept, which
/// leaves the 110 long stock blocks unshortened, as T-006's export holds them (decided
/// as option (b) of silvermere#25). Returns whether the block changed.
pub fn merge_text(old: &mut TextblkRecord, new: &TextblkRecord, carried: usize) -> bool {
    merge_text_as(old, new, carried, LayoutSet::Dos)
}

/// [`merge_text`] with `carried` counting bytes of a `set`-layout image (a Worldgroup 3.x
/// text block's text starts at +0x018).
pub fn merge_text_as(
    old: &mut TextblkRecord,
    new: &TextblkRecord,
    carried: usize,
    set: LayoutSet,
) -> bool {
    let mut bytes = old.encode_as(set);
    let n = carried.min(bytes.len());
    bytes[..n].copy_from_slice(&new.encode_as(set)[..n]);
    let merged = TextblkRecord::decode_as(&bytes, set).expect("a whole textblk image");
    if merged == *old {
        return false;
    }
    *old = merged;
    true
}
