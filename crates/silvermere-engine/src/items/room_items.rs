//! Items lying in a room: [`add_item_to_room`] and [`remove_item_from_room`]. The visible
//! list holds [`NROOMITEMS`] slots and the hidden list (STASH, HIDE) [`NHIDITEMS`]; a slot
//! holds one item number, its uses and a count of extra copies. Every change marks the room
//! changed, which here queues the room's live state for the persistence worker
//! ([`mark_changed`]).

use silvermere_rules::abilities::item_has_ability;
use silvermere_types::enums::Ability;
use silvermere_types::ids::{ItemNo, RoomKey};
use silvermere_types::records::{Item, RoomItem};
use silvermere_types::sizes::{NHIDITEMS, NROOMITEMS};

use super::inventory::matches_any_word;
use super::limited::{self, Holder};
use crate::event_loop::{EngineOutput, PersistRequest};
use crate::world::World;

/// Marks the room changed: its live state goes to the persistence worker (write-behind; the
/// worker coalesces the writes of one room).
pub fn mark_changed(world: &mut World, room: RoomKey) {
    if let Some(state) = world.room_state.get(&room) {
        let req = PersistRequest::RoomState(Box::new(state.clone()));
        world.outbox.push(EngineOutput::Persist(req));
    }
}

/// Puts a copy of `item` with `charges` uses into `room`, hidden or visible. A limited item
/// is first recorded as lying there ([`super::limited::acquire`]). An uncharged copy
/// (`charges` -1) is stacked on a slot that already holds the item; otherwise the first
/// empty slot takes it. Returns the slot, or `None` when the room or item does not exist,
/// no copy is free, or every slot is taken (the room's record of the copy is then released
/// again).
pub fn add_item_to_room(
    world: &mut World,
    room: RoomKey,
    item: ItemNo,
    hidden: bool,
    charges: i16,
) -> Option<usize> {
    if !world.room_state.contains_key(&room) || !world.items.contains_key(&item) {
        return None;
    }
    if !limited::acquire(world, item, Holder::Room(room)) {
        return None;
    }
    let state = world.room_state.get_mut(&room)?;
    let slots: &mut [RoomItem] = if hidden {
        &mut state.hiditems[..NHIDITEMS]
    } else {
        &mut state.items[..NROOMITEMS]
    };
    // Hidden or visible: stack an uncharged copy
    let mut slot = slots.iter().position(|s| s.item == item && charges == -1);
    if let Some(i) = slot {
        slots[i].count += 1;
    } else {
        // The first empty slot
        slot = slots.iter().position(|s| s.item == ItemNo(0));
        if let Some(i) = slot {
            slots[i] = RoomItem {
                item,
                uses: charges,
                count: 0,
            };
        }
    }
    match slot {
        Some(i) => {
            mark_changed(world, room);
            Some(i)
        }
        None => {
            // No room for it: the copy is not in the room after all
            limited::release(world, item, Holder::Room(room));
            None
        }
    }
}

/// What [`remove_item_from_room`] took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The item.
    pub item: ItemNo,
    /// Its uses left.
    pub charges: i16,
    /// It came from the hidden list.
    pub hidden: bool,
}

/// Takes one copy of the first item whose name fits `name` (`matches_any_word`) and may be picked
/// up (`gettable`, or the taker's sysop `debug` is on), visible items first. A hidden item
/// counts only once a search has found its slot (`foundmask` bit). (sic) A Gang House Deed
/// in an earlier slot is taken whatever name was typed; kept.
pub fn remove_item_from_room(
    world: &mut World,
    room: RoomKey,
    name: &str,
    debug: bool,
) -> Option<Taken> {
    let state = world.room_state.get(&room)?;
    let fits = |itm: Option<&Item>| match itm {
        Some(itm) => {
            (matches_any_word(name, &itm.name) && (itm.gettable != 0 || debug))
                || item_has_ability(itm, Ability::GangDeed)
        }
        None => false,
    };
    let mut taken = None;
    // The visible items
    for (i, s) in state.items.iter().enumerate().take(NROOMITEMS) {
        if s.item != ItemNo(0) && fits(world.items.get(&s.item)) {
            taken = Some((i, false));
            break;
        }
    }
    // The hidden items the player's search found
    if taken.is_none() {
        for (i, s) in state.hiditems.iter().enumerate().take(NHIDITEMS) {
            if s.item == ItemNo(0) {
                continue;
            }
            let Some(itm) = world.items.get(&s.item) else {
                continue;
            };
            if matches_any_word(name, &itm.name) && (itm.gettable != 0 || debug) {
                // only a slot the player's search has found; an unfound one is
                // passed over
                if state.foundmask & (1u32 << i) != 0 {
                    taken = Some((i, true));
                    break;
                }
            }
        }
    }
    let (i, hidden) = taken?;
    let state = world.room_state.get_mut(&room)?;
    let slot = if hidden {
        &mut state.hiditems[i]
    } else {
        &mut state.items[i]
    };
    let out = Taken {
        item: slot.item,
        charges: slot.uses,
        hidden,
    };
    if slot.count > 0 {
        slot.count -= 1;
    } else {
        *slot = RoomItem {
            item: ItemNo(0),
            uses: -1,
            count: 0,
        };
        if hidden {
            state.foundmask &= !(1u32 << i);
        }
    }
    mark_changed(world, room);
    limited::release(world, out.item, Holder::Room(room));
    Some(out)
}
