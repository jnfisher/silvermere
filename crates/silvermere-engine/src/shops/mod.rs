//! Shop state: the twenty stock slots and what they mean by shop type, the restock queue,
//! the name search among a shop's goods ([`get_shop_item_from_name`]) and the shelf copies
//! of limited items.
//!
//! The content of a shop (`shops`/`shop_items`) and its live `qty[]` (`shop_stock`) are one
//! [`Shop`] record in [`World::shops`]; a change of stock goes to the persistence worker as
//! [`PersistRequest::ShopStock`].
//!
//! The restock queue is [`World::restock`]: one [`RestockNode`] per timed slot, due at a
//! fast-tick count (one tick a second). `check_initiate_restocking` runs once for shops 0
//! to [`NRESTOCK`]-1 when the world loads and [`restock_items`] on every 21st slow cycle
//! (the scheduler's count).

use silvermere_rules::Rng;
use silvermere_types::enums::{Currency, ShopType};
use silvermere_types::ids::{ItemNo, RoomKey, ShopNo};
use silvermere_types::records::{Shop, ShopSlot};

use crate::commands::eq_ignore_case;
use crate::event_loop::{EngineOutput, PersistRequest};
use crate::items::inventory::matches_any_word;
use crate::items::limited::{self, Holder};
use crate::world::World;

/// `NRESTOCK`: shops 0 to 199 have their restocks queued at start-up.
pub const NRESTOCK: i32 = 200;
/// `MAXRESTOCK`: most queue entries one [`restock_items`] looks at.
pub const MAXRESTOCK: usize = 1000;
/// `PCTMAX`: the top of a restock roll.
pub const PCTMAX: i32 = 100;
/// A gang shop uses only its first ten slots.
pub const GANG_SLOTS: usize = 10;
/// Fast ticks (seconds) in a minute (`SECPERMIN`).
const TICKS_PER_MINUTE: i64 = 60;

/// What one slot of a shop holds. A gang shop overloads the restock fields:
/// `regentime` is the asking price, `regennum` its currency and
/// `maxqty` holds the owner's user-id as a string (T-044 owns that).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// An ordinary slot: stock, ceiling and restock rule. `maxqty` 0 is a recycler slot:
    /// it only sells what players sold to it.
    Stock(ShopSlot),
    /// A gang shop listing (`SHOP_GANG`, first [`GANG_SLOTS`] slots).
    GangListing {
        /// The item.
        item: ItemNo,
        /// Copies on the shelf.
        qty: u16,
        /// The asking price (`regentime[]`).
        price: u16,
        /// Its currency (`regennum[]`).
        currency: Currency,
    },
}

/// The slot `i` of `shop` as [`SlotKind`].
pub fn slot_kind(shop: &Shop, i: usize) -> SlotKind {
    let s = shop.stock[i];
    if shop.shop_type == ShopType::GangShop {
        SlotKind::GangListing {
            item: s.item,
            qty: s.qty,
            price: s.regentime,
            currency: Currency::from_code(s.regennum as u16),
        }
    } else {
        SlotKind::Stock(s)
    }
}

/// One of the shop's slots lists the item.
pub fn shop_sells_item(shop: &Shop, item: ItemNo) -> bool {
    shop.stock.iter().any(|s| s.item == item)
}

/// A slot lists the item with copies left.
pub fn item_is_in_stock(shop: &Shop, item: ItemNo) -> bool {
    shop.stock.iter().any(|s| s.item == item && s.qty > 0)
}

/// The shop of the player's room, when it is a shop room.
/// The shop record itself may be missing.
pub fn shop_room(world: &World, room: RoomKey) -> Option<ShopNo> {
    let rm = world.room(room)?;
    (rm.room_type == silvermere_types::enums::RoomType::Shop).then_some(rm.shopnum)
}

/// Queues the shop's current stock for the persistence worker.
pub fn persist_stock(world: &mut World, shop: ShopNo) {
    let Some(shp) = world.shops.get(&shop) else {
        return;
    };
    let qty = std::array::from_fn(|i| shp.stock[i].qty);
    world
        .outbox
        .push(EngineOutput::Persist(PersistRequest::ShopStock {
            shop,
            qty,
        }));
}

/// What [`get_shop_item_from_name`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShopMatch {
    /// The item, when the name picked one.
    pub item: Option<ItemNo>,
    /// Candidates counted (`*count`); set only when the search ran to its end.
    pub count: usize,
    /// The "Please be more specific" names, in order.
    pub options: Vec<String>,
    /// A later exact name threw the list away by clearing the output buffer, which also empties
    /// what the command had queued (the dispatcher's `ANSI_NORMAL`).
    pub cleared: bool,
}

/// The shop's goods whose name
/// the typed text abbreviates (`matches_any_word`). An exact name among the candidates wins at
/// once; one candidate is the item; several give the list (the second candidate first,
/// then the first, then the others in slot order) and no item.
pub fn get_shop_item_from_name(world: &World, shop: &Shop, name: &str) -> ShopMatch {
    let mut m = ShopMatch::default();
    let mut first: Option<(ItemNo, &str)> = None;
    let mut cnt = 0usize;
    for slot in &shop.stock {
        if slot.item == ItemNo(0) {
            continue;
        }
        let Some(itm) = world.items.get(&slot.item) else {
            continue;
        };
        if !matches_any_word(name, &itm.name) {
            continue;
        }
        cnt += 1;
        let exact = eq_ignore_case(&itm.name, name);
        match first {
            None => {
                if exact {
                    m.item = Some(itm.number);
                    return m;
                }
                first = Some((slot.item, &itm.name));
            }
            Some((_, first_name)) => {
                if cnt == 2 {
                    if exact {
                        m.item = Some(itm.number);
                        return m;
                    }
                    m.options.push(itm.name.clone());
                    m.options.push(first_name.to_string());
                } else if exact {
                    // A later exact match drops the list (`clear_output`)
                    m.options.clear();
                    m.cleared = true;
                    m.item = Some(itm.number);
                    return m;
                } else {
                    m.options.push(itm.name.clone());
                }
            }
        }
    }
    m.count = cnt;
    if cnt == 1 {
        m.item = first.map(|(n, _)| n);
    }
    m
}

/// What LOOK at a name in a shop room comes to ([`display_item_in_shop`]), for the LOOK
/// command (T-027).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopLook {
    /// Not a shop room, no shop, no such goods or sold out: LOOK goes on as if nothing matched.
    NotHandled,
    /// Several goods fit: the list is shown; LOOK is handled.
    Ambiguous(Vec<String>),
    /// An item in stock: LOOK shows its description (`display_item_desc`); LOOK is handled.
    Describe(ItemNo),
}

/// What LOOK at `name` in the shop room `room` decides.
pub fn display_item_in_shop(world: &World, room: RoomKey, name: &str) -> ShopLook {
    let Some(shop) = shop_room(world, room).and_then(|n| world.shops.get(&n)) else {
        return ShopLook::NotHandled;
    };
    let m = get_shop_item_from_name(world, shop, name);
    match m.item {
        None if m.count > 0 => ShopLook::Ambiguous(m.options),
        None => ShopLook::NotHandled,
        Some(item) if item_is_in_stock(shop, item) => ShopLook::Describe(item),
        Some(_) => ShopLook::NotHandled,
    }
}

/// What a purchase took off the ledger for a limited item (`buy_item`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShelfCopy {
    /// Not a limited item: nothing is counted.
    Unlimited,
    /// A copy recorded on this shop's shelf (one a player sold here) left the ledger; the
    /// buyer's record takes its place, so the count in play nets out unchanged. It goes
    /// back with [`return_shelf_copy`] when the sale falls through.
    FromShelf,
    /// No copy is recorded on the shelf (stock from the shop data or a restock): fewer than
    /// `limit` copies are in play, and the buyer's own record is the one that counts.
    Unrecorded,
}

/// The shelf copy taken off the buyer's room in `buy_item`: `None` when the
/// item is limited, the shelf holds no record of it and every allowed copy is in play
/// (reason 1).
///
/// The shelf's copies are `OWN_ROOM` records of the shop's room, marked
/// [`limited::SHELF_MARK`] so that a copy dropped on the shop's floor is never taken for one.
/// A recorded shelf copy is released; one that was never recorded (stock from the shop data
/// or a restock, which records none) is the only kind that needs a free copy under the limit.
pub fn take_shelf_copy(world: &mut World, item: ItemNo, room: RoomKey) -> Option<ShelfCopy> {
    let itm = world.items.get(&item)?;
    if itm.limit == 0 {
        return Some(ShelfCopy::Unlimited);
    }
    let (limit, active) = (itm.limit, itm.active);
    let shelf = Holder::Shelf(room);
    let recorded = limited::holders_of(world, item)
        .iter()
        .any(|own| shelf.holds(own));
    if recorded {
        limited::release(world, item, shelf);
        Some(ShelfCopy::FromShelf)
    } else if active < limit {
        Some(ShelfCopy::Unrecorded)
    } else {
        None
    }
}

/// When the buyer cannot pay, the shelf record [`take_shelf_copy`] took goes back.
pub fn return_shelf_copy(world: &mut World, item: ItemNo, room: RoomKey, copy: ShelfCopy) {
    if copy == ShelfCopy::FromShelf {
        limited::acquire(world, item, Holder::Shelf(room));
    }
}

/// One pending restock: a timed slot and the fast tick it is due at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestockNode {
    /// The shop (`shopnum`).
    pub shop: ShopNo,
    /// The slot (`slot`).
    pub slot: usize,
    /// Due at this fast tick.
    pub due: u64,
}

/// A restock node due `minutes` from now. (sic) A `regentime` over 32767 is a negative
/// number of minutes, as in the original; here a negative interval is due at once, and the
/// interval is counted in fast ticks from now, not on the wall clock to the minute.
fn due_in(now: u64, minutes: i16) -> u64 {
    now.saturating_add_signed(i64::from(minutes) * TICKS_PER_MINUTE)
        .max(now)
}

/// A slot with no interval is topped up once now (by `regennum` when a 1-99 roll, the top
/// of 100 excluded, is below `regenpct`, never past `maxqty`) or cut back to `maxqty`; a
/// timed slot is cut back to `maxqty` and queued, its first restock due a random
/// 1..max(2,`regentime`) minutes away. Gang shops and unknown shops are skipped.
pub fn check_initiate_restocking(world: &mut World, shop: ShopNo, rng: &mut dyn Rng) {
    let now = world.scheduler.tick();
    let Some(shp) = world.shops.get_mut(&shop) else {
        return;
    };
    if shp.shop_type == ShopType::GangShop {
        return;
    }
    let mut dirty = false;
    let mut nodes = Vec::new();
    for (i, s) in shp.stock.iter_mut().enumerate() {
        if s.item == ItemNo(0) {
            continue;
        }
        if s.regentime == 0 {
            if s.qty < s.maxqty {
                // Roll 1-99 (the top, PCTMAX, excluded), below regenpct wins
                if rng.range(1, PCTMAX) < i32::from(s.regenpct) {
                    s.qty = s.qty.wrapping_add_signed(s.regennum);
                    if s.qty > s.maxqty {
                        s.qty = s.maxqty;
                    }
                    dirty = true;
                }
            } else if s.qty > s.maxqty {
                s.qty = s.maxqty;
                dirty = true;
            }
        } else {
            if s.qty > s.maxqty {
                s.qty = s.maxqty;
                dirty = true;
            }
            // 1 up to max(2, `regentime`), the top excluded, `regentime` read as signed
            let top = (s.regentime as i16).max(2);
            let minutes = rng.range(1, i32::from(top)) as i16;
            nodes.push(RestockNode {
                shop,
                slot: i,
                due: due_in(now, minutes),
            });
        }
    }
    // Each node goes on the head of the list: the queue's last entry is its head
    world.restock.extend(nodes);
    if dirty {
        persist_stock(world, shop);
    }
}

/// [`check_initiate_restocking`] for every shop in `0..NRESTOCK`.
pub fn initiate_all(world: &mut World, rng: &mut dyn Rng) {
    for k in 0..NRESTOCK {
        check_initiate_restocking(world, ShopNo(k), rng);
    }
}

/// Every due entry among the first
/// [`MAXRESTOCK`] from the head (sic: later ones wait for ever) whose shop still exists and
/// is not a gang shop: below `maxqty`, a 1-99 roll below `regenpct` adds `regennum`
/// copies (never past `maxqty`); then the next restock is due one full `regentime`. An
/// entry whose shop is gone or became a gang shop stays due and is passed over.
pub fn restock_items(world: &mut World, rng: &mut dyn Rng) {
    let now = world.scheduler.tick();
    let n = world.restock.len();
    let mut changed: Vec<ShopNo> = Vec::new();
    for k in (n.saturating_sub(MAXRESTOCK)..n).rev() {
        let node = world.restock[k];
        if node.due > now {
            continue;
        }
        let Some(shp) = world.shops.get_mut(&node.shop) else {
            continue;
        };
        if shp.shop_type == ShopType::GangShop {
            continue;
        }
        let s = &mut shp.stock[node.slot];
        if s.qty < s.maxqty && rng.range(1, PCTMAX) < i32::from(s.regenpct) {
            // regennum is signed: a negative value takes stock away (inferred)
            s.qty = s.qty.wrapping_add_signed(s.regennum);
            if s.qty > s.maxqty {
                s.qty = s.maxqty;
            }
            if !changed.contains(&node.shop) {
                changed.push(node.shop);
            }
        }
        let minutes = s.regentime as i16;
        world.restock[k].due = due_in(now, minutes);
    }
    for shop in changed {
        persist_stock(world, shop);
    }
}

/// The restock queue for the sysop listing (`display_all_restocking_required`, T-055):
/// the entries of `shop` (every shop for `None`), head first.
pub fn pending_restocks(world: &World, shop: Option<ShopNo>) -> Vec<RestockNode> {
    world
        .restock
        .iter()
        .rev()
        .filter(|n| shop.is_none_or(|s| n.shop == s))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_counts_minutes_in_ticks() {
        assert_eq!(due_in(100, 2), 220);
        assert_eq!(due_in(100, 0), 100);
        // a regentime over 32767 is negative as an int: due at once here
        assert_eq!(due_in(100, 40000u16 as i16), 100);
    }
}
