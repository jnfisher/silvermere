//! The active monsters: the slot map, the monster numbers with their free list,
//! `remove_active_monster` and the write-behind of `active_monsters`.
//!
//! Active monsters are numbered: numbers freed by dead or removed monsters go on a free
//! list and a new monster gets the oldest of them, else one above the last number issued.
//! The records live in a [`slotmap::SlotMap`] with an index by number, and the free list
//! is a [`FreeList`] rebuilt at load. Clients never see the
//! numbers, but the database rows do, so they are kept.
//!
//! The `MonsterCache` sweeps and `dirty` flags become [`PersistRequest`]s: an upsert when
//! a monster is generated, a delete when it is removed, and the 60 s sweep
//! ([`save_active_monsters`]) upserts every monster changed since its last write.

use std::cell::{Ref, RefCell, RefMut};
use std::collections::{HashMap, VecDeque};

use silvermere_types::ids::{ActiveMonsterId, MonsterNo, RoomKey};
use silvermere_types::records::ActiveMonster;

use super::cache::{Lookup, MonsterCache};
use crate::event_loop::{EngineOutput, PersistRequest};
use crate::world::{ActiveSlot, World};

/// `NOACTNUM`: the number of a monster without a WCCACMSR record.
pub const NOACTNUM: ActiveMonsterId = ActiveMonsterId(65535);

/// The free list of active-monster numbers: freed numbers go in at the head, the oldest
/// is handed out from the tail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreeList {
    /// Head (newest) at the front, tail (oldest) at the back.
    free: VecDeque<ActiveMonsterId>,
    /// The last number issued.
    last: i32,
}

impl FreeList {
    /// The free list rebuilt at load, the last number issued being the highest in use:
    /// every number from 1 below the highest that has no record, lowest at the tail
    /// (reused first). 65535 is never listed.
    pub fn build<'a>(in_use: impl Iterator<Item = &'a ActiveMonsterId>) -> Self {
        let used: std::collections::HashSet<i32> = in_use.map(|n| n.0).collect();
        let last = used.iter().copied().max().unwrap_or(0).max(0);
        let mut free = VecDeque::new();
        for n in 1..last {
            if !used.contains(&n) && n != NOACTNUM.0 {
                free.push_front(ActiveMonsterId(n));
            }
        }
        Self { free, last }
    }

    /// A new monster's number: the oldest freed number, else one above the last issued.
    pub fn take(&mut self) -> ActiveMonsterId {
        if let Some(n) = self.free.pop_back() {
            return n;
        }
        self.last += 1;
        if self.last == NOACTNUM.0 {
            self.last += 1;
        }
        ActiveMonsterId(self.last)
    }

    /// Puts a freed number at the head (as [`remove_active_monster`] does).
    pub fn give_back(&mut self, n: ActiveMonsterId) {
        if n.0 > 0 && n != NOACTNUM && !self.free.contains(&n) {
            self.free.push_front(n);
        }
    }

    /// The numbers waiting, head first.
    pub fn waiting(&self) -> impl Iterator<Item = ActiveMonsterId> + '_ {
        self.free.iter().copied()
    }
}

/// The active monsters: the records, an index by number, the free list and the copy of
/// each record as last written (what the save sweep compares against).
#[derive(Debug, Clone, Default)]
pub struct ActiveMonsters {
    slots: slotmap::SlotMap<ActiveSlot, ActiveMonster>,
    by_number: HashMap<ActiveMonsterId, ActiveSlot>,
    /// The number allocator.
    pub numbers: FreeList,
    saved: HashMap<ActiveMonsterId, ActiveMonster>,
    /// Which monsters the monster cache holds: what the background passes walk
    /// (T-086). A look-up by number ([`Self::by_number`]) changes it, so it sits behind a
    /// `RefCell`.
    cache: RefCell<MonsterCache>,
}

impl std::ops::Deref for ActiveMonsters {
    type Target = slotmap::SlotMap<ActiveSlot, ActiveMonster>;

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl std::ops::IndexMut<ActiveSlot> for ActiveMonsters {
    fn index_mut(&mut self, slot: ActiveSlot) -> &mut ActiveMonster {
        &mut self.slots[slot]
    }
}

impl std::ops::Index<ActiveSlot> for ActiveMonsters {
    type Output = ActiveMonster;

    fn index(&self, slot: ActiveSlot) -> &ActiveMonster {
        &self.slots[slot]
    }
}

impl ActiveMonsters {
    /// The monsters of a snapshot: all of them as last written, the free list built.
    pub fn load(monsters: &[ActiveMonster]) -> Self {
        let mut out = Self::default();
        for m in monsters {
            out.insert(m.clone());
            out.saved.insert(m.number, m.clone());
        }
        out.numbers = FreeList::build(out.by_number.keys());
        out
    }

    /// Adds a record (its number already chosen); a record with the same number is
    /// replaced.
    pub fn insert(&mut self, m: ActiveMonster) -> ActiveSlot {
        if let Some(old) = self.by_number.get(&m.number).copied() {
            self.slots.remove(old);
        }
        let n = m.number;
        let slot = self.slots.insert(m);
        self.by_number.insert(n, slot);
        slot
    }

    /// Takes a record out by slot.
    pub fn remove(&mut self, slot: ActiveSlot) -> Option<ActiveMonster> {
        let m = self.slots.remove(slot)?;
        if self.by_number.get(&m.number) == Some(&slot) {
            self.by_number.remove(&m.number);
        }
        Some(m)
    }

    /// The record, mutable, by slot.
    pub fn get_mut(&mut self, slot: ActiveSlot) -> Option<&mut ActiveMonster> {
        self.slots.get_mut(slot)
    }

    /// Every record, mutable.
    pub fn values_mut(&mut self) -> slotmap::basic::ValuesMut<'_, ActiveSlot, ActiveMonster> {
        self.slots.values_mut()
    }

    /// Every record with its slot, mutable.
    pub fn iter_mut(&mut self) -> slotmap::basic::IterMut<'_, ActiveSlot, ActiveMonster> {
        self.slots.iter_mut()
    }

    /// The slot of active monster `n`.
    pub fn slot_of(&self, n: ActiveMonsterId) -> Option<ActiveSlot> {
        let slot = *self.by_number.get(&n)?;
        // a record whose number was changed in place is found by a scan
        if self.slots.get(slot).is_some_and(|m| m.number == n) {
            return Some(slot);
        }
        self.slots
            .iter()
            .find(|(_, m)| m.number == n)
            .map(|(k, _)| k)
    }

    /// A look-up by number: the record, with the monster read into the cache
    /// ([`Self::touch`]). Code that works on a record it already has uses
    /// [`Self::peek`].
    pub fn by_number(&self, n: ActiveMonsterId) -> Option<&ActiveMonster> {
        self.touch(n);
        self.peek(n)
    }

    /// A look-up by number, mutable.
    pub fn by_number_mut(&mut self, n: ActiveMonsterId) -> Option<&mut ActiveMonster> {
        self.touch(n);
        self.peek_mut(n)
    }

    /// The record, leaving the cache as it is (a monster already in the cache, or a
    /// record in hand).
    pub fn peek(&self, n: ActiveMonsterId) -> Option<&ActiveMonster> {
        self.slots.get(self.slot_of(n)?)
    }

    /// The record, mutable, leaving the cache as it is.
    pub fn peek_mut(&mut self, n: ActiveMonsterId) -> Option<&mut ActiveMonster> {
        let slot = self.slot_of(n)?;
        self.slots.get_mut(slot)
    }

    /// The cache walk of a look-up by number: see [`MonsterCache::get`].
    pub fn touch(&self, n: ActiveMonsterId) -> Lookup {
        self.touch_raw(n.0 as u32)
    }

    /// [`Self::touch`] with a raw 32-bit number, for callers that
    /// pass a number cut to 16 bits and sign-extended.
    pub fn touch_raw(&self, monnum: u32) -> Lookup {
        let exists = i32::try_from(monnum)
            .ok()
            .is_some_and(|n| self.slot_of(ActiveMonsterId(n)).is_some());
        self.cache.borrow_mut().get(monnum, exists)
    }

    /// The cache.
    pub fn cache(&self) -> Ref<'_, MonsterCache> {
        self.cache.borrow()
    }

    /// The cache, to change.
    pub fn cache_mut(&self) -> RefMut<'_, MonsterCache> {
        self.cache.borrow_mut()
    }

    /// Has active monster `n` changed since it was last written?
    pub fn is_dirty(&self, n: ActiveMonsterId) -> bool {
        self.peek(n) != self.saved.get(&n)
    }

    /// The numbers in use, in number order.
    pub fn numbers_in_use(&self) -> Vec<ActiveMonsterId> {
        let mut v: Vec<ActiveMonsterId> = self.slots.values().map(|m| m.number).collect();
        v.sort_unstable();
        v
    }
}

/// Queues a persistence request.
pub(crate) fn persist(world: &mut World, req: PersistRequest) {
    world.outbox.push(EngineOutput::Persist(req));
}

/// Drops every save in `outbox[from..]` that a later save of the same row in that range
/// overwrites (T-113). A room state, an active monster, a known monster's live part, a
/// shop's stock and an item's copies in play are written whole, so the last save of a
/// row leaves what all of them in order leave; the others keep their places. What the
/// store holds once the outbox is written does not change, only the number of writes.
pub fn coalesce_saves(outbox: &mut Vec<EngineOutput>, from: usize) {
    #[derive(PartialEq, Eq, Hash)]
    enum Row {
        Room(RoomKey),
        Active(ActiveMonsterId),
        Known(MonsterNo),
        Shop(silvermere_types::ids::ShopNo),
        Item(silvermere_types::ids::ItemNo),
    }
    let row = |o: &EngineOutput| match o {
        EngineOutput::Persist(PersistRequest::RoomState(s)) => Some(Row::Room(s.key)),
        EngineOutput::Persist(PersistRequest::ActiveMonster(m)) => Some(Row::Active(m.number)),
        EngineOutput::Persist(PersistRequest::MonsterState { number, .. }) => {
            Some(Row::Known(*number))
        }
        EngineOutput::Persist(PersistRequest::ShopStock { shop, .. }) => Some(Row::Shop(*shop)),
        EngineOutput::Persist(PersistRequest::ItemActive { item, .. }) => Some(Row::Item(*item)),
        _ => None,
    };
    let from = from.min(outbox.len());
    let mut seen = std::collections::HashSet::new();
    let mut keep = vec![true; outbox.len() - from];
    for (i, o) in outbox[from..].iter().enumerate().rev() {
        if let Some(r) = row(o)
            && !seen.insert(r)
        {
            keep[i] = false;
        }
    }
    let mut i = 0;
    outbox.retain(|_| {
        let k = i < from || keep[i - from];
        i += 1;
        k
    });
}

/// Writes active monster `n` now (insert or update) and notes it as saved.
pub fn save_active_monster(world: &mut World, n: ActiveMonsterId) {
    let Some(m) = world.active_monsters.peek(n).cloned() else {
        return;
    };
    world.active_monsters.saved.insert(n, m.clone());
    persist(world, PersistRequest::ActiveMonster(Box::new(m)));
}

/// The monsters the cache pushed out since the last call are written back when they
/// changed, which leaves them unchanged since their last write.
pub fn write_back_evicted(world: &mut World) {
    let evicted = world.active_monsters.cache_mut().take_evicted();
    for n in evicted {
        if world.active_monsters.peek(n).is_some() && world.active_monsters.is_dirty(n) {
            save_active_monster(world, n);
        }
    }
}

/// The 60 s sweep: every active monster that
/// changed since it was last written is upserted.
pub fn save_active_monsters(world: &mut World) {
    let changed: Vec<ActiveMonsterId> = world
        .active_monsters
        .values()
        .filter(|m| world.active_monsters.saved.get(&m.number) != Some(*m))
        .map(|m| m.number)
        .collect();
    for n in changed {
        save_active_monster(world, n);
    }
}

/// Writes a room's live state.
pub fn save_room(world: &mut World, room: RoomKey) {
    if let Some(s) = world.room_state.get(&room) {
        let req = PersistRequest::RoomState(Box::new(s.clone()));
        persist(world, req);
    }
}

/// Writes a known monster's live part (`curalive`, `killed_at`).
pub fn save_known_monster(world: &mut World, n: MonsterNo) {
    if let Some(k) = world.monsters.get(&n) {
        let req = PersistRequest::MonsterState {
            number: n,
            curalive: k.curalive,
            killed_at: k.killed_at,
        };
        persist(world, req);
    }
}

/// Puts a monster in the first free slot of the room's list; `None` when the room is
/// unknown or full.
pub fn add_monster_to_room(world: &mut World, room: RoomKey, n: ActiveMonsterId) -> Option<usize> {
    let s = world.room_state.get_mut(&room)?;
    let i = s.monsters.iter().position(|m| m.0 == 0)?;
    s.monsters[i] = n;
    Some(i)
}

/// Takes a monster out of the room's list: the slot it was in.
pub fn take_monster_from_room(
    world: &mut World,
    room: RoomKey,
    n: ActiveMonsterId,
) -> Option<usize> {
    let s = world.room_state.get_mut(&room)?;
    let i = s.monsters.iter().position(|m| *m == n)?;
    s.monsters[i] = ActiveMonsterId(0);
    Some(i)
}

/// The monster leaves the game without a kill. Its generating room (and that room's controlling
/// room) count one fewer, it leaves its room's list, its known monster counts one fewer
/// alive (and with `settime` notes now as its kill time), its record is deleted and its
/// number goes to the head of the free list.
pub fn remove_active_monster(world: &mut World, n: ActiveMonsterId, settime: bool) {
    let Some(slot) = world.active_monsters.slot_of(n) else {
        return;
    };
    let Some(mon) = world.active_monsters.get(slot).cloned() else {
        return;
    };
    let genkey = RoomKey {
        map: mon.map,
        room: mon.genroom,
    };
    if let Some(ctl) = world.room(genkey).map(|r| r.ctlroom) {
        if let Some(s) = world.room_state.get_mut(&genkey)
            && s.nummonsters > 0
        {
            s.nummonsters -= 1;
        }
        if ctl.0 != 0 {
            // the controlling room is the room itself or another of the same map
            let c = RoomKey {
                map: mon.map,
                room: ctl,
            };
            if let Some(s) = world.room_state.get_mut(&c)
                && s.areacount > 0
            {
                s.areacount -= 1;
                if c != genkey {
                    save_room(world, c);
                }
            }
        }
        save_room(world, genkey);
    }
    let here = RoomKey {
        map: mon.map,
        room: mon.room,
    };
    if let Some(s) = world.room_state.get_mut(&here) {
        for m in &mut s.monsters {
            if *m == n {
                *m = ActiveMonsterId(0);
            }
        }
    }
    save_room(world, here);
    if let Some(k) = world.monsters.get_mut(&mon.knownnum)
        && k.curalive > 0
    {
        k.curalive -= 1;
        if settime {
            k.killed_at = world.wall;
        }
        save_known_monster(world, mon.knownnum);
    }
    delete_active_record(world, n);
}

/// The end of [`remove_active_monster`] and `check_kill_monster`: the record is deleted
/// and its number goes to the head of the free list (65535, `NOACTNUM`, has no record and
/// is not freed).
pub fn delete_active_record(world: &mut World, n: ActiveMonsterId) {
    // the number is read into the cache, then its slot is freed (T-086)
    let exists = world.active_monsters.slot_of(n).is_some();
    world.active_monsters.cache_mut().delete(n.0 as u32, exists);
    if let Some(slot) = world.active_monsters.slot_of(n) {
        world.active_monsters.remove(slot);
    }
    world.active_monsters.saved.remove(&n);
    if n == NOACTNUM {
        return;
    }
    persist(world, PersistRequest::ActiveMonsterDelete(n));
    world.active_monsters.numbers.give_back(n);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-113: the last save of each row stays where it was, everything else in order;
    /// nothing before `from` is touched.
    #[test]
    fn coalescing_keeps_the_last_save_of_each_row() {
        let known = |n: i32, curalive: u16| {
            EngineOutput::Persist(PersistRequest::MonsterState {
                number: MonsterNo(n),
                curalive,
                killed_at: 0,
            })
        };
        let active = |item: i32, active: u16| {
            EngineOutput::Persist(PersistRequest::ItemActive {
                item: silvermere_types::ids::ItemNo(item),
                active,
            })
        };
        let gone = EngineOutput::Persist(PersistRequest::ActiveMonsterDelete(ActiveMonsterId(5)));
        let mut outbox = vec![
            known(1, 9),
            known(1, 1),
            active(1, 1),
            known(2, 2),
            gone,
            known(1, 3),
            active(1, 2),
            EngineOutput::Persist(PersistRequest::ActiveMonsterDelete(ActiveMonsterId(5))),
        ];
        coalesce_saves(&mut outbox, 1);
        let got: Vec<String> = outbox.iter().map(|o| format!("{o:?}")).collect();
        let want: Vec<String> = [
            known(1, 9),
            known(2, 2),
            EngineOutput::Persist(PersistRequest::ActiveMonsterDelete(ActiveMonsterId(5))),
            known(1, 3),
            active(1, 2),
            EngineOutput::Persist(PersistRequest::ActiveMonsterDelete(ActiveMonsterId(5))),
        ]
        .iter()
        .map(|o| format!("{o:?}"))
        .collect();
        assert_eq!(got, want);
    }

    /// Freed numbers come back oldest first.
    #[test]
    fn numbers_are_reused_oldest_first() {
        let mut f = FreeList::default();
        assert_eq!(f.take(), ActiveMonsterId(1));
        assert_eq!(f.take(), ActiveMonsterId(2));
        assert_eq!(f.take(), ActiveMonsterId(3));
        f.give_back(ActiveMonsterId(2));
        f.give_back(ActiveMonsterId(1));
        assert_eq!(f.take(), ActiveMonsterId(2));
        assert_eq!(f.take(), ActiveMonsterId(1));
        assert_eq!(f.take(), ActiveMonsterId(4));
    }

    /// The gaps below the highest number, lowest reused first.
    #[test]
    fn build_lists_the_gaps() {
        let used = [ActiveMonsterId(2), ActiveMonsterId(5)];
        let mut f = FreeList::build(used.iter());
        assert_eq!(
            f.waiting().collect::<Vec<_>>(),
            vec![ActiveMonsterId(4), ActiveMonsterId(3), ActiveMonsterId(1)]
        );
        assert_eq!(f.take(), ActiveMonsterId(1));
        assert_eq!(f.take(), ActiveMonsterId(3));
        assert_eq!(f.take(), ActiveMonsterId(4));
        assert_eq!(f.take(), ActiveMonsterId(6));
    }
}
