//! Timed exit changes: [`add_exit_to_list`] and their reverting ([`revert_exit`]) on the
//! scheduler.
//!
//! A step lasts 300 fast ticks, about five minutes. As in the original, the changes wait in
//! one queue (`Scheduler::exits`) and each exit timer works on the head, whatever change it
//! was armed for: one step comes off its delay, and while some is left the change goes to the
//! back with a new timer of `300 * delay` ticks (so a delay of `d` steps lasts
//! `300 * (d + ... + 1)` ticks). What reverts, by exit type, is as the original has it.

use silvermere_text::Msg;
use silvermere_types::enums::{Direction, ExitType};
use silvermere_types::flags::ExitFlags;
use silvermere_types::ids::RoomKey;
use silvermere_types::records::{ExitParams, HiddenExit, Lock, TimedExit, TrapState};

use super::dirname;
use super::exits::{Arg, fmt_msg, live_exit, set_exit_state, set_lock, tell_room_all};
use crate::scheduler::{ExitChange, Routine};
use crate::world::World;

/// Fast ticks for one step of a change's delay.
pub const EXIT_DELAY: u64 = 300;

/// Queues the change of `dir` at the back, waiting `steps` steps (kept as a byte), and arms
/// its timer for `300 * steps` ticks, worked out as the game's 16-bit int.
pub fn add_exit_to_list(world: &mut World, room: RoomKey, dir: Direction, steps: i32) {
    let dir = dir.code() as u8;
    world.scheduler.exits.push_back(ExitChange {
        room,
        dir,
        delay: steps as u8,
    });
    let ticks = (EXIT_DELAY as i16).wrapping_mul(steps as i16);
    world
        .scheduler
        .arm_signed(ticks, Routine::ExitRevert { room, dir });
}

/// An exit timer: one step off the head change's delay; it is queued again with what is
/// left, or, with none left, the exit reverts. Then the head goes.
pub fn update_exits(world: &mut World) {
    let Some(head) = world.scheduler.exits.front_mut() else {
        return;
    };
    head.delay = head.delay.wrapping_sub(1);
    let head = *head;
    match Direction::COMPASS.get(usize::from(head.dir)).copied() {
        Some(dir) if head.delay > 0 => {
            add_exit_to_list(world, head.room, dir, i32::from(head.delay));
        }
        _ => revert_exit(world, head.room, head.dir),
    }
    world.scheduler.exits.pop_front();
}

/// Drops the queued changes of the exit, and their timers.
pub fn drop_exit_changes(world: &mut World, room: RoomKey, dir: u8) {
    world
        .scheduler
        .exits
        .retain(|c| !(c.room == room && c.dir == dir));
    world.scheduler.disarm(Routine::ExitRevert { room, dir });
}

/// The blocked flags 1 to `n` together.
pub fn blocked_upto(n: u32) -> u32 {
    ((1u32 << n) - 1) * ExitFlags::BLOCKED1.bits()
}

/// The change of the exit whose timer ran out: a door, gate or key exit locks again (not a
/// door or gate with a positive `exitpar2s`, nor one locked already), a disarmed trap
/// re-arms, a found hidden exit hides (or a visible-but-blockable one blocks) again, a
/// Timed exit swaps phase, tells the room and queues its next swap after `exitpar2s` steps.
pub fn revert_exit(world: &mut World, room: RoomKey, dir: u8) {
    let Some(dir) = Direction::COMPASS.get(usize::from(dir)).copied() else {
        return;
    };
    let Some(exit) = live_exit(world, room, dir) else {
        return;
    };
    let p = exit.params();
    match p {
        ExitParams::Trap(_) | ExitParams::SpellTrap(_) => match p.trap_state() {
            Some(TrapState::Disarmed) => {
                set_exit_state(world, room, dir, p.with_trap_state(TrapState::Armed));
            }
            Some(TrapState::DisarmedThrows) => {
                set_exit_state(world, room, dir, p.with_trap_state(TrapState::ArmedThrows));
            }
            _ => {}
        },
        ExitParams::Timed(t) => {
            let msg = world.messages.get(&t.message).map(|m| m.msgs.clone());
            let opening = t.closed();
            let state = if opening { Lock::Open } else { Lock::Locked };
            set_exit_state(
                world,
                room,
                dir,
                ExitParams::Timed(TimedExit { state, ..t }),
            );
            let name = dirname(world, dir);
            let line = match (&msg, opening) {
                (Some(m), true) => Msg::ExitTimedText {
                    text: fmt_msg(&m[0], &[Arg::S(&name)]),
                },
                (Some(m), false) => Msg::ExitTimedText {
                    text: fmt_msg(&m[1], &[Arg::S(&name)]),
                },
                (None, true) => Msg::ExitOpened { dir: name },
                (None, false) => Msg::ExitClosed { dir: name },
            };
            tell_room_all(world, room, [None, None], line);
            add_exit_to_list(world, room, dir, i32::from(t.steps));
        }
        ExitParams::Hidden(h) => {
            let flags = if h.flags.contains(ExitFlags::VISSEARCHED) {
                Some(ExitFlags::NEEDSEARCH)
            } else if h.flags.contains(ExitFlags::VISBLOCKED) {
                let n = u32::from(h.blocks.unsigned_abs());
                (1..=10)
                    .contains(&n)
                    .then(|| ExitFlags::from_bits_retain(blocked_upto(n)))
            } else {
                None
            };
            if let Some(flags) = flags {
                set_exit_state(
                    world,
                    room,
                    dir,
                    ExitParams::Hidden(HiddenExit { flags, ..h }),
                );
            }
        }
        ExitParams::Key(k) => {
            if k.lock != Lock::Locked {
                set_lock(world, room, dir, &exit, Lock::Locked);
                let line = Msg::DoorRelocked {
                    dir: dirname(world, dir),
                };
                tell_room_all(world, room, [None, None], line);
            }
        }
        ExitParams::Door(d) | ExitParams::Gate(d) => {
            if d.bonus > 0 || d.lock == Lock::Locked {
                return;
            }
            set_lock(world, room, dir, &exit, Lock::Locked);
            let name = dirname(world, dir);
            let line = if exit.kind == ExitType::Door {
                Msg::DoorRelocked { dir: name }
            } else {
                Msg::GateRelocked { dir: name }
            };
            tell_room_all(world, room, [None, None], line);
        }
        // nothing to undo
        _ => {}
    }
}
