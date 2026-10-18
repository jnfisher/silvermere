//! The timer queue, keyed by the fast tick.
//!
//! The fast tick comes every second: it counts the tick, runs the timers that are due and
//! then the per-tick player and monster updates. Every other cycle (energy, medium, slow,
//! monster creation, save, inactivity, timed exits) is a timer on that queue, armed with a
//! delay and re-armed by the routine itself at the end of its run. [`Scheduler`] is that
//! queue; [`on_fast_tick`] is the fast routine.
//!
//! Only the shape and the order are here. Each routine body is a `World` method that
//! later cards fill (T-035, T-037, T-026, T-056); here they only record that they ran.
//! The daily cleanup is the one routine armed by wall-clock time ([`Routine::DailyCleanup`]).
//!
//! Time comes in through the [`Clock`] trait and randomness through [`Rng`], both
//! `silvermere-rules`' (T-016): production passes [`SystemClock`]
//! and [`SeededRng`], tests the testkit's `TestClock` and a fixed seed.

use std::collections::VecDeque;

pub use silvermere_rules::{Clock, Rng, SeededRng, SystemClock};
use silvermere_types::config::GameConfig;
use silvermere_types::ids::RoomKey;

use crate::world::World;

/// `MAXKICKRUN`: most timers run in one fast tick; the rest wait.
pub const MAX_KICK_RUN: usize = 1000;

/// `RESTOCKSLOW`: shops restock once the slow count passes this, so on every 21st slow
/// cycle.
pub const RESTOCK_SLOW: u32 = 20;

/// A timer routine: what a queued timer runs when it is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Routine {
    /// The energy cycle: new energy, then a combat round.
    Energy,
    /// The medium cycle: spell rounds, room spells, wandering.
    Medium,
    /// The slow cycle: regeneration, poison, light, restock.
    Slow,
    /// Monster creation: monsters near the players.
    Spawn,
    /// The round-robin character save.
    Save,
    /// The idle check and hang-up.
    Inactivity,
    /// The daily cleanup: armed for the next
    /// `game.cleanup.at` local wall-clock time, and re-armed the same way by its run
    /// ([`crate::cleanup::on_due`]). The original's BBS ran it at its cleanup hour, offline.
    DailyCleanup,
    /// A timed exit reverts.
    ExitRevert {
        /// The room whose exit changes back.
        room: RoomKey,
        /// The direction, `0..NDIRS`.
        dir: u8,
    },
}

/// The periods, in fast ticks (seconds), from the game options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Periods {
    /// The energy cycle, 5 by default.
    pub energy: u64,
    /// The medium cycle, 3 by default.
    pub medium: u64,
    /// The slow cycle, 30 by default.
    pub slow: u64,
    /// Monster creation, 5 by default.
    pub spawn: u64,
    /// The character save, 60 by default, rounded down to whole minutes and at least 60.
    pub save: u64,
    /// The inactivity check, 3600 by default.
    pub inactivity: u64,
}

impl Periods {
    /// The periods of the module defaults.
    pub fn defaults() -> Self {
        Self::from_config(&GameConfig::default())
    }

    /// The periods a configuration asks for.
    pub fn from_config(cfg: &GameConfig) -> Self {
        // Rounded down to whole minutes, at least one minute
        let save = ((u64::from(cfg.sec_oth) / 60) * 60).max(60);
        Self {
            energy: u64::from(cfg.sec_energy).max(1),
            medium: u64::from(cfg.sec_med).max(1),
            slow: u64::from(cfg.sec_slow).max(1),
            spawn: u64::from(cfg.sec_create).max(1),
            save,
            inactivity: u64::from(cfg.inactive_secs).max(1),
        }
    }

    /// The period a periodic routine re-arms with; `None` for one-shot routines.
    pub fn of(&self, routine: Routine) -> Option<u64> {
        Some(match routine {
            Routine::Energy => self.energy,
            Routine::Medium => self.medium,
            Routine::Slow => self.slow,
            Routine::Spawn => self.spawn,
            Routine::Save => self.save,
            Routine::Inactivity => self.inactivity,
            Routine::DailyCleanup | Routine::ExitRevert { .. } => return None,
        })
    }
}

/// One queued timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kick {
    /// The fast tick it is due at.
    pub when: u64,
    /// What runs.
    pub routine: Routine,
    /// Identity, so a running node can be unlinked after routines armed others.
    seq: u64,
}

/// What ran during one fast tick, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    /// A due timer routine.
    Routine(Routine),
    /// `restock_items` inside the slow cycle.
    Restock,
    /// `fast_update_characters`.
    CharacterUpdate,
    /// `fast_update_monsters`.
    MonsterUpdate,
}

/// The record of one fast tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickReport {
    /// The tick that was counted (after the increment).
    pub tick: u64,
    /// Everything that ran, in order.
    pub ran: Vec<Ran>,
}

/// The module timer queue: the tick count, the queue and the slow cycle's `restockcnt`.
#[derive(Debug, Clone)]
pub struct Scheduler {
    /// Fast ticks since start.
    tick: u64,
    /// Pending timers, soonest first.
    queue: VecDeque<Kick>,
    next_seq: u64,
    /// `restockcnt`: slow cycles since the last restock.
    restockcnt: u32,
    periods: Periods,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new(Periods::defaults())
    }
}

impl Scheduler {
    /// An empty queue at tick 0. Nothing is armed until [`Scheduler::start`].
    pub fn new(periods: Periods) -> Self {
        Self {
            tick: 0,
            queue: VecDeque::new(),
            next_seq: 0,
            restockcnt: 0,
            periods,
        }
    }

    /// The fast tick counter.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The periods in force.
    pub fn periods(&self) -> Periods {
        self.periods
    }

    /// The pending timers, soonest first.
    pub fn pending(&self) -> impl Iterator<Item = &Kick> {
        self.queue.iter()
    }

    /// Arms the periodic routines in this order (it decides which runs first on a
    /// shared tick): monster creation, save, slow, medium, energy, inactivity. The fast tick
    /// itself is the caller's (`EngineInput::Tick`).
    pub fn start(&mut self) {
        let p = self.periods;
        self.arm(p.spawn, Routine::Spawn);
        self.arm(p.save, Routine::Save);
        self.arm(p.slow, Routine::Slow);
        self.arm(p.medium, Routine::Medium);
        self.arm(p.energy, Routine::Energy);
        self.arm(p.inactivity, Routine::Inactivity);
    }

    /// Queues `routine` to run `delay` fast
    /// ticks from now. The node goes in time order, after the last node due earlier: so
    /// ahead of nodes due at the same tick, unless the head itself is due then, in which
    /// case right after the head. The quirk is kept because it
    /// decides the order of two cycles that fall due together.
    pub fn arm(&mut self, delay: u64, routine: Routine) {
        let node = Kick {
            when: self.tick + delay,
            routine,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        let Some(head) = self.queue.front() else {
            self.queue.push_back(node);
            return;
        };
        if head.when > node.when {
            self.queue.push_front(node);
            return;
        }
        // cur/prev start at the head; walk while the node at cur is due earlier.
        let mut prev = 0;
        let mut cur = 0;
        while cur < self.queue.len() && self.queue[cur].when < node.when {
            prev = cur;
            cur += 1;
        }
        self.queue.insert(prev + 1, node);
    }

    /// Removes every pending instance of `routine`.
    pub fn disarm(&mut self, routine: Routine) {
        self.queue.retain(|k| k.routine != routine);
    }

    fn remove(&mut self, seq: u64) {
        if let Some(pos) = self.queue.iter().position(|k| k.seq == seq) {
            self.queue.remove(pos);
        }
    }
}

/// One fast tick. The tick count goes up, the due timers run in queue order (at most
/// [`MAX_KICK_RUN`]), each re-arming itself at the end of its body, then the per-tick
/// character and monster updates run, and the polls until the next tick (the monster
/// passes, T-086).
/// Every draw, the cycles' and the command handlers', comes from `world.rng`, the
/// engine's one random source.
pub fn on_fast_tick(world: &mut World) -> TickReport {
    world.scheduler.tick += 1;
    let tick = world.scheduler.tick;
    let mut ran = Vec::new();
    let mut cnt = 0;
    // The node stays at the head while its routine runs, so a timer
    // the routine arms is linked in behind it; then it is unlinked and freed.
    while let Some(head) = world.scheduler.queue.front()
        && head.when <= tick
        && cnt < MAX_KICK_RUN
    {
        cnt += 1;
        let (seq, routine) = (head.seq, head.routine);
        run_routine(world, routine, &mut ran);
        world.scheduler.remove(seq);
    }
    world.fast_update_characters();
    ran.push(Ran::CharacterUpdate);
    world.fast_update_monsters();
    ran.push(Ran::MonsterUpdate);
    world.poll_monster_passes();
    TickReport { tick, ran }
}

/// One routine body: the work, then the re-arm.
fn run_routine(world: &mut World, routine: Routine, ran: &mut Vec<Ran>) {
    ran.push(Ran::Routine(routine));
    match routine {
        Routine::Energy => world.energy_cycle(),
        Routine::Medium => world.medium_cycle(),
        Routine::Slow => {
            world.slow_cycle();
            // Count slow cycles; restock once the count passes RESTOCKSLOW.
            let s = &mut world.scheduler;
            s.restockcnt += 1;
            if s.restockcnt > RESTOCK_SLOW {
                s.restockcnt = 0;
                world.restock_items();
                ran.push(Ran::Restock);
            }
            world.slow_cycle_end();
        }
        Routine::Spawn => world.spawn_cycle(),
        Routine::Save => world.save_cycle(),
        Routine::Inactivity => world.inactivity_check(),
        Routine::DailyCleanup => crate::cleanup::on_due(world),
        Routine::ExitRevert { room, dir } => world.revert_exit(room, dir),
    }
    if let Some(period) = world.scheduler.periods.of(routine) {
        world.scheduler.arm(period, routine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_order_is_the_originals() {
        let mut s = Scheduler::new(Periods::defaults());
        s.arm(5, Routine::Spawn);
        s.arm(60, Routine::Save);
        s.arm(30, Routine::Slow);
        s.arm(3, Routine::Medium);
        s.arm(5, Routine::Energy);
        s.arm(3600, Routine::Inactivity);
        let order: Vec<Routine> = s.pending().map(|k| k.routine).collect();
        assert_eq!(
            order,
            [
                Routine::Medium,
                Routine::Energy,
                Routine::Spawn,
                Routine::Slow,
                Routine::Save,
                Routine::Inactivity
            ]
        );
        // A node due at the head's tick goes right after the head.
        s.arm(
            3,
            Routine::ExitRevert {
                room: RoomKey::new(1, 1),
                dir: 0,
            },
        );
        assert_eq!(s.pending().nth(1).unwrap().when, 3);
        assert_eq!(
            s.pending().nth(1).unwrap().routine,
            Routine::ExitRevert {
                room: RoomKey::new(1, 1),
                dir: 0,
            }
        );
    }

    #[test]
    fn save_period_rounds_down_to_minutes_with_a_floor_of_60() {
        let cfg = GameConfig {
            sec_oth: 150,
            ..GameConfig::default()
        };
        assert_eq!(Periods::from_config(&cfg).save, 120);
        let cfg = GameConfig {
            sec_oth: 7,
            ..GameConfig::default()
        };
        assert_eq!(Periods::from_config(&cfg).save, 60);
    }
}
