//! Limited items: the copies in play of an item whose `limit` is not 0, and who holds each.
//!
//! Each copy in play has one ownership record, and the item record counts the copies in
//! play (`item.active`). The records are [`World::item_ownership`] keyed by record id, the
//! count stays in the item record, and both go to the persistence worker write-behind
//! ([`PersistRequest::ItemOwnershipInsert`],
//! [`PersistRequest::ItemOwnershipDelete`], [`PersistRequest::ItemActive`]).
//!
//! `TRACK_LIMITED` (`tracklim`) is [`silvermere_types::config::GameConfig::track_limited`];
//! its activity-log lines go to `tracing`.

use silvermere_types::enums::OwnerKind;
use silvermere_types::ids::{ActiveMonsterId, ItemNo, RoomKey};
use silvermere_types::records::ItemOwnership;

use crate::event_loop::{EngineOutput, PersistRequest};
use crate::world::World;

/// Most records one release scan examines.
pub const MAXOWNSCAN: usize = 1000;

/// The `userid` of an `OWN_ROOM` record that stands for a copy on a shop's shelf rather
/// than on the room's floor. A copy dropped in a shop and a copy sold to it are both
/// `OWN_ROOM` records of the shop's room; the mark keeps them apart. Floor
/// records have an empty `userid`.
pub const SHELF_MARK: &str = "#SHELF";

/// Who holds a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder<'a> {
    /// `OWN_USER`, by user-id.
    User(&'a str),
    /// `OWN_ROOM`.
    Room(RoomKey),
    /// `OWN_MONSTER`, by active monster number.
    Monster(ActiveMonsterId),
    /// A shop's shelf: `OWN_ROOM` of the shop's room, marked [`SHELF_MARK`].
    Shelf(RoomKey),
}

impl Holder<'_> {
    /// Does `own` record a copy this holder has?
    pub fn holds(&self, own: &ItemOwnership) -> bool {
        match *self {
            Holder::User(userid) => {
                own.owntype == OwnerKind::User && own.userid.eq_ignore_ascii_case(userid)
            }
            Holder::Room(key) => {
                own.owntype == OwnerKind::Room
                    && own.userid != SHELF_MARK
                    && own.mapnum == key.map
                    && own.roomnum == key.room
            }
            Holder::Monster(m) => own.owntype == OwnerKind::Monster && own.monnum == m,
            Holder::Shelf(key) => {
                own.owntype == OwnerKind::Room
                    && own.userid == SHELF_MARK
                    && own.mapnum == key.map
                    && own.roomnum == key.room
            }
        }
    }
}

fn persist(world: &mut World, req: PersistRequest) {
    world.outbox.push(EngineOutput::Persist(req));
}

fn note_active(world: &mut World, item: ItemNo) {
    if let Some(itm) = world.items.get(&item) {
        let active = itm.active;
        persist(world, PersistRequest::ItemActive { item, active });
    }
}

/// Records a new copy for `holder` while fewer than `limit` are in play. `true` when the
/// item is not limited or the copy was recorded; `false` when every allowed copy is already
/// out (the caller must not hand it over) or the item is unknown.
pub fn acquire(world: &mut World, item: ItemNo, holder: Holder<'_>) -> bool {
    let Some(itm) = world.items.get(&item) else {
        return false;
    };
    if itm.limit == 0 {
        return true;
    }
    if itm.active >= itm.limit {
        return false;
    }
    // the new record's id follows the highest one, 1 for an empty file
    let id = world.item_ownership.keys().max().map_or(1, |&m| m + 1);
    let mut own = ItemOwnership {
        id,
        itemnum: item,
        owntype: OwnerKind::User,
        mapnum: Default::default(),
        roomnum: Default::default(),
        userid: String::new(),
        monnum: ActiveMonsterId(0),
    };
    match holder {
        Holder::User(userid) => userid.clone_into(&mut own.userid),
        Holder::Room(key) => {
            own.owntype = OwnerKind::Room;
            own.mapnum = key.map;
            own.roomnum = key.room;
        }
        Holder::Monster(m) => {
            own.owntype = OwnerKind::Monster;
            own.monnum = m;
        }
        Holder::Shelf(key) => {
            own.owntype = OwnerKind::Room;
            own.mapnum = key.map;
            own.roomnum = key.room;
            SHELF_MARK.clone_into(&mut own.userid);
        }
    }
    world.item_ownership.insert(id, own.clone());
    persist(world, PersistRequest::ItemOwnershipInsert(own));
    let name = match world.items.get_mut(&item) {
        Some(itm) => {
            itm.active += 1;
            itm.name.clone()
        }
        None => String::new(),
    };
    note_active(world, item);
    if world.config.track_limited {
        tracing::info!(target: "mmlog", "Item {name} added to {holder:?}");
    }
    true
}

/// Deletes the first record of `item` held by `holder` among the first [`MAXOWNSCAN`]
/// records (by id), and count one copy fewer in play. The count goes down whether or not a
/// record was found (sic); a count already at 0 is logged. `true` unless the item is unknown.
pub fn release(world: &mut World, item: ItemNo, holder: Holder<'_>) -> bool {
    let Some(itm) = world.items.get(&item) else {
        return false;
    };
    if itm.limit == 0 {
        return true;
    }
    let mut ids: Vec<i32> = world.item_ownership.keys().copied().collect();
    ids.sort_unstable();
    let found = ids.into_iter().take(MAXOWNSCAN).find(|id| {
        world
            .item_ownership
            .get(id)
            .is_some_and(|own| own.itemnum == item && holder.holds(own))
    });
    if let Some(id) = found {
        world.item_ownership.remove(&id);
        persist(world, PersistRequest::ItemOwnershipDelete(id));
    }
    let name = match world.items.get_mut(&item) {
        Some(itm) if itm.active > 0 => {
            itm.active -= 1;
            itm.name.clone()
        }
        Some(itm) => {
            // A count already at 0 is logged
            tracing::error!(item = %item, "remlogical? {}: no copies in play", itm.name);
            itm.name.clone()
        }
        None => String::new(),
    };
    note_active(world, item);
    if world.config.track_limited {
        tracing::info!(target: "mmlog", "Item {name} removed from {holder:?}");
    }
    true
}

/// The records of `item`'s copies in play (`holders_of`), lowest id first.
pub fn holders_of(world: &World, item: ItemNo) -> Vec<&ItemOwnership> {
    let mut rows: Vec<&ItemOwnership> = world
        .item_ownership
        .values()
        .filter(|own| own.itemnum == item)
        .collect();
    rows.sort_by_key(|own| own.id);
    rows
}
