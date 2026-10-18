//! The database update's shop merge.

use crate::decode::ShopRecord;

/// `SHOP_GANG`.
pub const SHOP_GANG: i16 = 11;

/// Merges a shop update. Each member that differs is copied on its own. A gang shop, in the
/// update or in the game, is left alone completely (the caller also skips an existing gang
/// shop). Name and description lines; `minlevel`, `maxlevel`, `markup`, `ownable`,
/// `expandable` and `trainclass` together; a new type takes the whole stock list,
/// quantities included; then per slot a different item brings its quantity with it while
/// `maxqty`, `regentime`, `regennum` and `regenpct` are copied on their own, so `qty` is
/// otherwise kept. Never copied: `number`, `dirty`, `unk_0x1dc`. Returns whether anything
/// was copied.
pub fn merge_shop(old: &mut ShopRecord, new: &ShopRecord) -> bool {
    if new.type_ == SHOP_GANG || old.type_ == SHOP_GANG {
        return false;
    }
    let mut dirty = false;
    if old.name != new.name {
        old.name = new.name.clone();
        dirty = true;
    }
    for i in 0..old.desc.len() {
        if old.desc[i] != new.desc[i] {
            old.desc[i] = new.desc[i].clone();
            dirty = true;
        }
    }
    if old.minlevel != new.minlevel
        || old.maxlevel != new.maxlevel
        || old.markup != new.markup
        || old.ownable != new.ownable
        || old.expandable != new.expandable
        || old.trainclass != new.trainclass
    {
        old.minlevel = new.minlevel;
        old.maxlevel = new.maxlevel;
        old.markup = new.markup;
        old.ownable = new.ownable;
        old.expandable = new.expandable;
        old.trainclass = new.trainclass;
        dirty = true;
    }
    if old.type_ != new.type_ {
        old.type_ = new.type_;
        old.items = new.items;
        old.maxqty = new.maxqty;
        old.qty = new.qty;
        old.regentime = new.regentime;
        old.regennum = new.regennum;
        old.regenpct = new.regenpct;
        dirty = true;
    }
    for i in 0..old.items.len() {
        if old.items[i] != new.items[i] {
            old.items[i] = new.items[i];
            old.qty[i] = new.qty[i];
            dirty = true;
        }
        if old.maxqty[i] != new.maxqty[i] {
            old.maxqty[i] = new.maxqty[i];
            dirty = true;
        }
        if old.regentime[i] != new.regentime[i] {
            old.regentime[i] = new.regentime[i];
            dirty = true;
        }
        if old.regennum[i] != new.regennum[i] {
            old.regennum[i] = new.regennum[i];
            dirty = true;
        }
        if old.regenpct[i] != new.regenpct[i] {
            old.regenpct[i] = new.regenpct[i];
            dirty = true;
        }
    }
    dirty
}
