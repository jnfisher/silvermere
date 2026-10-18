//! The active-monster cache: which monsters the background passes see.
//!
//! Under MBBSEmu only some active monsters act: the slow, medium and fast monster passes
//! walk a cache of `monsbuf + 1` slots (slots 0..=monsbuf) slot by slot, so only monsters
//! resident in the cache move, regenerate or follow. MONSBUF is 300 in the shipped
//! WCCMMUD.MSG:170, which comes to 20 under MBBSEmu; here it is `game.buffers.monsbuf`.
//! Every record stays in memory ([`super::ActiveMonsters`]); this struct keeps only which
//! monster number each cache slot holds, and changes it at each look-up by number and
//! each delete, so the same monsters act.
//!
//! A monster comes into the cache when something asks for it by number: the room
//! display's "Also here" list, combat, a moving monster and the monsters of the rooms it
//! passes, spells, and so on. A newly generated monster is written straight to the file
//! and is not cached until then, unless its creation spell asks for it.

use silvermere_types::ids::ActiveMonsterId;

/// MONSBUF as shipped (WCCMMUD.MSG:170, `{Number of active monsters to buffer? 300}`).
pub const MONSBUF: usize = 300;

/// `MONFREE`: the number of a freed cache slot.
pub const MONFREE: u32 = 65535;

/// The active-monster cache: the number held by each of the `monsbuf + 1` slots (0 =
/// empty, never used; [`MONFREE`] = freed), and `last_used_slot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterCache {
    /// `monsbuf`: the hash modulus, and the passes' slot count (they never reach slot
    /// `monsbuf`).
    monsbuf: usize,
    /// Slots 0..=monsbuf: the active monster number each holds.
    slots: Vec<u32>,
    /// The slot loaded or found last, -1 at first.
    last_used_slot: i32,
    /// Live monsters pushed out of the cache since the owner last asked; the owner writes
    /// back those that changed.
    evicted: Vec<ActiveMonsterId>,
    /// The slots holding a live monster.
    used: u32,
    /// Look-ups that found the monster cached.
    hits: u32,
    /// Look-ups that went to the file.
    misses: u32,
}

impl Default for MonsterCache {
    fn default() -> Self {
        Self::new(MONSBUF)
    }
}

/// What [`MonsterCache::get`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// The monster was in the slot already.
    Hit(usize),
    /// The monster was read into the slot.
    Loaded(usize),
    /// No such monster.
    Missing,
}

impl MonsterCache {
    /// An empty cache of `monsbuf + 1` slots.
    pub fn new(monsbuf: usize) -> Self {
        let monsbuf = monsbuf.max(1);
        Self {
            monsbuf,
            slots: vec![0; monsbuf + 1],
            last_used_slot: -1,
            evicted: Vec::new(),
            used: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// The slots holding a live monster.
    pub fn used(&self) -> u32 {
        self.used
    }

    /// Look-ups that found the monster cached.
    pub fn hits(&self) -> u32 {
        self.hits
    }

    /// Look-ups that went to the file.
    pub fn misses(&self) -> u32 {
        self.misses
    }

    /// The cache's part of clearing the statistics: the hit and miss counters go to zero;
    /// `used` is a state, not a statistic, and stays.
    pub fn clear_statistics(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// `monsbuf`.
    pub fn monsbuf(&self) -> usize {
        self.monsbuf
    }

    /// The number in slot `slot`; 0 past the end.
    pub fn number_at(&self, slot: usize) -> u32 {
        self.slots.get(slot).copied().unwrap_or(0)
    }

    /// The slot holding a live monster `n`, without touching the cache.
    pub fn slot_of(&self, n: ActiveMonsterId) -> Option<usize> {
        let n = n.0 as u32;
        if n == 0 || n == MONFREE {
            return None;
        }
        self.slots.iter().position(|&s| s == n)
    }

    /// The live monsters in slot order (slots 0..=monsbuf).
    ///
    /// Through [`super::ActiveMonsters::cache`] the iterator holds the cache's `RefCell`
    /// borrow: collect it before any look-up by number (`by_number`, `touch`), which
    /// borrows the cache mutably and would panic.
    pub fn resident(&self) -> impl Iterator<Item = (usize, ActiveMonsterId)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, n)| **n != 0 && **n != MONFREE)
            .map(|(i, n)| (i, ActiveMonsterId(*n as i32)))
    }

    /// `last_used_slot`.
    pub fn last_used_slot(&self) -> i32 {
        self.last_used_slot
    }

    /// The monsters pushed out since the last call, oldest first.
    pub fn take_evicted(&mut self) -> Vec<ActiveMonsterId> {
        std::mem::take(&mut self.evicted)
    }

    /// A look-up by number, on the slot numbers:
    /// probe from `monnum % monsbuf` up to slot `monsbuf` (no wrap) until the monster or
    /// an empty slot turns up; on a miss the first freed slot on the way is used, else the
    /// first empty one, else the hashed slot; never `last_used_slot` (then the next slot, or
    /// the hashed one past the end). `exists` says whether WCCACMSR has the monster: when
    /// it does not, the slot keeps a live monster and an empty or freed one is marked
    /// freed.
    ///
    /// `monnum` is a raw 32-bit number: callers that cut a number to 16 bits
    /// and sign-extend it pass the result.
    pub fn get(&mut self, monnum: u32, exists: bool) -> Lookup {
        let hash = (monnum % self.monsbuf as u32) as usize;
        let mut slot: Option<usize> = None;
        let mut freed: Option<usize> = None;
        let mut i = hash;
        while i <= self.monsbuf && slot.is_none() {
            let n = self.slots[i];
            if n == monnum {
                self.last_used_slot = i as i32;
                self.hits += 1;
                return Lookup::Hit(i);
            }
            if freed.is_none() && n == MONFREE {
                freed = Some(i);
            }
            if slot.is_none() && n == 0 {
                slot = Some(i);
            }
            i += 1;
        }
        self.misses += 1;
        let mut slot = freed.or(slot).unwrap_or(hash);
        if slot as i32 == self.last_used_slot {
            slot += 1;
            if slot > self.monsbuf {
                slot = hash;
            }
        }
        let old = self.slots[slot];
        let live = old != 0 && old != MONFREE;
        // the old monster stops counting, and a load or a kept live monster counts again
        if live {
            self.used = self.used.saturating_sub(1);
        }
        if exists || live {
            self.used += 1;
        }
        if exists {
            if live {
                self.evicted.push(ActiveMonsterId(old as i32));
            }
            self.slots[slot] = monnum;
            self.last_used_slot = slot as i32;
            Lookup::Loaded(slot)
        } else {
            if !live {
                self.slots[slot] = MONFREE;
            }
            Lookup::Missing
        }
    }

    /// The cache part of deleting a monster: the monster is fetched (read in if need be)
    /// and its slot cleared and marked freed. `exists` as for [`Self::get`].
    pub fn delete(&mut self, monnum: u32, exists: bool) {
        match self.get(monnum, exists) {
            Lookup::Hit(slot) | Lookup::Loaded(slot) => {
                // the record is in the file, so it stops counting
                self.used = self.used.saturating_sub(1);
                self.slots[slot] = MONFREE;
            }
            Lookup::Missing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: i32) -> ActiveMonsterId {
        ActiveMonsterId(n)
    }

    /// The hashed slot first, then the probe.
    #[test]
    fn hashes_then_probes() {
        let mut c = MonsterCache::new(10);
        assert_eq!(c.get(13, true), Lookup::Loaded(3));
        assert_eq!(c.get(23, true), Lookup::Loaded(4));
        assert_eq!(c.get(13, true), Lookup::Hit(3));
        assert_eq!(c.get(23, true), Lookup::Hit(4));
        assert_eq!(c.last_used_slot(), 4);
    }

    /// The probe runs up to slot `monsbuf`, the one the passes never
    /// visit, and does not wrap: a full tail evicts the hashed slot.
    #[test]
    fn probe_reaches_the_last_slot_and_then_evicts_the_hashed_one() {
        let mut c = MonsterCache::new(10);
        assert_eq!(c.get(9, true), Lookup::Loaded(9));
        assert_eq!(c.get(19, true), Lookup::Loaded(10));
        // 29: slots 9 and 10 full, slot 9 is the hash but 10 is last_used_slot only if
        // loaded last; here last_used_slot is 10, the hash 9 is taken
        assert_eq!(c.get(29, true), Lookup::Loaded(9));
        assert_eq!(c.take_evicted(), vec![id(9)]);
    }

    /// The slot loaded last is never evicted at once.
    #[test]
    fn last_loaded_slot_is_spared() {
        let mut c = MonsterCache::new(10);
        assert_eq!(c.get(10, true), Lookup::Loaded(0));
        for n in [1, 2, 3, 4, 5, 6, 7, 8, 9] {
            c.get(n, true);
        }
        assert_eq!(c.get(11, true), Lookup::Loaded(10));
        // last_used_slot is 10; 20 hashes to 0 (full, no empty slot on the way): 0 is evicted
        assert_eq!(c.get(20, true), Lookup::Loaded(0));
        // 30 hashes to 0 = last_used_slot: the next slot, 1, goes instead
        assert_eq!(c.get(30, true), Lookup::Loaded(1));
        assert_eq!(c.take_evicted(), vec![id(10), id(1)]);
    }

    /// Freed slots are recycled before empty ones.
    #[test]
    fn freed_slots_come_first() {
        let mut c = MonsterCache::new(10);
        c.get(3, true);
        c.get(13, true);
        c.delete(3, true);
        assert_eq!(c.number_at(3), MONFREE);
        c.get(7, true);
        // 23: slot 3 freed, slot 4 holds 13, slot 5 empty: the freed slot is used
        assert_eq!(c.get(23, true), Lookup::Loaded(3));
        assert!(c.take_evicted().is_empty());
    }

    /// (sic) A delete finds the monster first, so its freed slot is `last_used_slot` and the
    /// next load that would recycle it takes the slot after it instead, even when that
    /// slot holds a live monster.
    #[test]
    fn a_slot_freed_last_is_skipped() {
        let mut c = MonsterCache::new(10);
        c.get(3, true);
        c.get(13, true);
        c.delete(3, true);
        assert_eq!(c.get(23, true), Lookup::Loaded(4));
        assert_eq!(c.take_evicted(), vec![id(13)]);
    }

    /// MBBSEmu's size (20): 21 slots,
    /// hashed modulo 20, the probe reaching slot 20 and not wrapping.
    #[test]
    fn a_cache_of_twenty_hashes_modulo_twenty() {
        let mut c = MonsterCache::new(20);
        assert_eq!(c.monsbuf(), 20);
        assert_eq!(c.get(45, true), Lookup::Loaded(5));
        assert_eq!(c.get(39, true), Lookup::Loaded(19));
        assert_eq!(c.get(59, true), Lookup::Loaded(20));
        // 79: slots 19 and 20 full (20 is last_used_slot), no wrap to slot 0: the hashed slot
        // 19 is evicted
        assert_eq!(c.get(79, true), Lookup::Loaded(19));
        assert_eq!(c.take_evicted(), vec![id(39)]);
        assert_eq!(c.number_at(0), 0);
        assert_eq!(c.resident().count(), 3);
    }

    /// A monster not in the file leaves a live slot alone and marks
    /// an empty one freed.
    #[test]
    fn missing_monster_marks_an_empty_slot_freed() {
        let mut c = MonsterCache::new(10);
        assert_eq!(c.get(5, false), Lookup::Missing);
        assert_eq!(c.number_at(5), MONFREE);
        c.get(9, true);
        c.get(19, true);
        // 29: no empty slot from 9 on, the hashed slot 9 holds a live monster: kept
        assert_eq!(c.get(29, false), Lookup::Missing);
        assert_eq!(c.number_at(9), 9);
        assert!(c.take_evicted().is_empty());
    }

    /// A cached find is a hit, a load is a miss, a monster
    /// missing from the file is a miss too.
    #[test]
    fn lookups_count_hits_and_misses() {
        let mut c = MonsterCache::new(10);
        c.get(13, true);
        c.get(13, true);
        c.get(13, true);
        c.get(5, false);
        assert_eq!((c.hits(), c.misses()), (2, 2));
        c.clear_statistics();
        assert_eq!((c.hits(), c.misses()), (0, 0));
    }

    /// `used` counts live slots and survives a clear.
    #[test]
    fn used_follows_loads_and_deletes() {
        let mut c = MonsterCache::new(10);
        c.get(3, true);
        c.get(4, true);
        c.get(6, false);
        assert_eq!(c.used(), 2);
        c.delete(3, true);
        assert_eq!(c.used(), 1);
        c.clear_statistics();
        assert_eq!(c.used(), 1);
    }
}
