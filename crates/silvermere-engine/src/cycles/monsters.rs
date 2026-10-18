//! The monster passes of the background cycles, over the monster
//! cache (T-086).
//!
//! A pass is not a loop over every monster: [`slow_update_monsters`],
//! [`medium_update_monsters`] and [`fast_update_monsters`] only mark a pass pending and
//! take one step; each step ([`slow_update_next_monster`] and its siblings) handles the
//! monster in the next cache slot, if there is one, and the pass ends after slot
//! `monsbuf - 1` (slot `monsbuf` is never visited). The steps after the first come from
//! the per-user polling routine, slow then fast then medium, every second poll of a
//! channel in the game. So only monsters resident in the cache ([`crate::monsters::cache`])
//! are updated, in slot order, and with nobody in the game a pass moves on one slot per
//! cycle.
//!
//! The polls are many to the second (MBBSEmu with `Timer.Hertz` 1000),
//! far more than the slots of a pass, so while anyone is in the game
//! [`poll_monster_passes`] runs at the end of each tick and steps until no pass is
//! pending: the passes of one tick end within it. A pass started while one is pending does
//! not start again: it only marks a step pending when none is.
//!
//! The slow body (poison and regeneration) is T-037's; the medium body is T-035's,
//! reached through `CycleHooks::medium_update_monster`, and the fast body is T-035's
//! `monsters::following::fast_update_monster`.

use silvermere_types::ids::ActiveMonsterId;

use crate::monsters::cache::MONFREE;
use crate::world::World;

/// The `actmon.number` of a free cache slot.
pub const FREEMON: ActiveMonsterId = ActiveMonsterId(MONFREE as i32);

/// One monster pass's cursor: `slowmonslot`/`slowmonpend` and their medium and fast
/// siblings, kept from one pass to the next.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassCursor {
    /// The next cache slot.
    pub slot: usize,
    /// Passes pending (0 or 1).
    pub pend: u8,
}

/// The three monster pass cursors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonsterPasses {
    /// `slowmonslot`, `slowmonpend`.
    pub slow: PassCursor,
    /// `mediummonslot`, `mediummonpend`.
    pub medium: PassCursor,
    /// `fastmonslot`, `fastmonpend`.
    pub fast: PassCursor,
}

impl MonsterPasses {
    /// Is any pass under way?
    pub fn pending(&self) -> bool {
        self.slow.pend > 0 || self.medium.pend > 0 || self.fast.pend > 0
    }
}

/// Which pass a step is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    Slow,
    Medium,
    Fast,
}

/// The live monster in the pass's next slot, and the cursor moved on. `None` when no pass
/// is pending.
fn step(world: &mut World, pass: Pass) -> Option<Option<ActiveMonsterId>> {
    let monsbuf = world.active_monsters.cache().monsbuf();
    let passes = &mut world.cycles.monster_passes;
    let cursor = match pass {
        Pass::Slow => &mut passes.slow,
        Pass::Medium => &mut passes.medium,
        Pass::Fast => &mut passes.fast,
    };
    if cursor.pend == 0 {
        return None;
    }
    let slot = cursor.slot;
    cursor.slot += 1;
    if cursor.slot >= monsbuf {
        cursor.slot = 0;
        cursor.pend -= 1;
    }
    let n = world.active_monsters.cache().number_at(slot);
    // Number 0 or 65535 marks a free cache slot
    Some((n != 0 && n != MONFREE).then_some(ActiveMonsterId(n as i32)))
}

/// Starts a slow monster pass: a new pass number, a pass pending, and its first step.
pub fn slow_update_monsters(world: &mut World) {
    world.cycles.slowmoncycle = world.cycles.slowmoncycle.wrapping_add(1);
    let c = &mut world.cycles.monster_passes.slow;
    if c.pend < 1 {
        c.pend += 1;
    }
    slow_update_next_monster(world);
}

/// The slow update of the monster in the next cache slot.
pub fn slow_update_next_monster(world: &mut World) {
    if let Some(Some(n)) = step(world, Pass::Slow) {
        let cycle = world.cycles.slowmoncycle;
        if let Some(mon) = world.active_monsters.peek_mut(n) {
            slow_update_monster(mon, cycle);
        }
    }
}

/// One monster's slow update: once per pass, the last move direction is forgotten, poison
/// takes its toll and a wounded monster regenerates `hpregen`, up to its maximum. A monster
/// poisoned below 0 dies in the medium pass (T-035). The unused read of the last `WCCACMSR`
/// record is not carried.
pub fn slow_update_monster(mon: &mut silvermere_types::records::ActiveMonster, cycle: i32) {
    if mon.lastupd == cycle {
        return;
    }
    mon.lastupd = cycle;
    mon.lastdir = -1;
    if mon.poison > 0 {
        mon.hp = mon.hp.wrapping_sub(mon.poison);
    }
    if mon.hp < mon.maxhp {
        mon.hp = mon.hp.wrapping_add(mon.hpregen);
        if mon.hp > mon.maxhp {
            mon.hp = mon.maxhp;
        }
    }
}

/// Starts a medium monster pass: a pass pending, nobody has wandered off in it yet
/// (`mediummoved`, MAXMOVERS a pass), and its first step.
pub fn medium_update_monsters(world: &mut World) {
    let c = &mut world.cycles.monster_passes.medium;
    if c.pend < 1 {
        c.pend += 1;
    }
    world.spawn.mediummoved = 0;
    medium_update_next_monster(world);
}

/// T-035's `medium_update_monster` for the monster in the next cache slot.
pub fn medium_update_next_monster(world: &mut World) {
    if let Some(Some(n)) = step(world, Pass::Medium) {
        (world.cycles.hooks.medium_update_monster)(world, n);
    }
}

/// Starts a fast monster pass: a pass pending and its first step.
pub fn fast_update_monsters(world: &mut World) {
    let c = &mut world.cycles.monster_passes.fast;
    if c.pend < 1 {
        c.pend += 1;
    }
    fast_update_next_monster(world);
}

/// The fast update of the monster in the next cache slot, only when it follows a player.
/// (sic) So a
/// knocked-down monster that follows nobody does not count its down rounds here.
pub fn fast_update_next_monster(world: &mut World) {
    if let Some(Some(n)) = step(world, Pass::Fast)
        && world
            .active_monsters
            .peek(n)
            .is_some_and(|m| !m.following.is_empty())
    {
        crate::monsters::following::fast_update_monster(world, n);
    }
}

/// The monster steps of one poll: slow, fast, medium.
pub fn poll_step(world: &mut World) {
    slow_update_next_monster(world);
    fast_update_next_monster(world);
    medium_update_next_monster(world);
}

/// The polls between this tick and the next: while a player is in the game (only their
/// channels poll), steps until no pass is pending. See the module notes.
pub fn poll_monster_passes(world: &mut World) {
    let polling = world
        .session_ids()
        .into_iter()
        .any(|id| world.is_in_game(id));
    if !polling {
        return;
    }
    run_pending_passes(world);
}

/// Steps until no pass is pending, whoever is in the game: a whole pass for a test.
pub fn run_pending_passes(world: &mut World) {
    while world.cycles.monster_passes.pending() {
        poll_step(world);
    }
}
