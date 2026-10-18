//! Timed exit changes: [`add_exit_to_list`] and their reverting ([`revert_exit`]) on the
//! scheduler.
//!
//! A step lasts 300 fast ticks, about five minutes. Under the original, several pending
//! changes share the steps, so a change can revert later than its own delay (sic). Here
//! each change is a [`Routine::ExitRevert`] due `300 * steps` ticks from now, so a change
//! reverts after its own delay and that quirk is not carried. What reverts, by
//! exit type, is as the original has it.

use silvermere_text::Msg;
use silvermere_types::enums::{Direction, ExitType};
use silvermere_types::flags::ExitFlags;
use silvermere_types::ids::{MessageNo, RoomKey};

use super::dirname;
use super::exits::{
    Arg, LOCK_LOCKED, LOCK_OPEN, fmt_msg, live_exit, set_par1, set_par2, set_par3, tell_room_all,
    trap,
};
use crate::scheduler::Routine;
use crate::world::World;

/// Fast ticks for one step of a change's delay.
pub const EXIT_DELAY: u64 = 300;

/// A Timed exit's open phase in `exitpar3s`.
pub const EXOPEN: i32 = 0;
/// A Timed exit's closed phase.
pub const EXLOCKED: i32 = 2;

/// The exit reverts after `steps` steps of [`EXIT_DELAY`] fast ticks. A step count of 0 or
/// less is taken as one step, as the callers would have it.
pub fn add_exit_to_list(world: &mut World, room: RoomKey, dir: Direction, steps: i32) {
    let steps = u64::try_from(steps.max(1)).unwrap_or(1);
    world.scheduler.arm(
        EXIT_DELAY * steps,
        Routine::ExitRevert {
            room,
            dir: dir.code() as u8,
        },
    );
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
    match exit.kind {
        ExitType::Trap | ExitType::SpellTrap => match exit.par2 {
            trap::HURTOFF => set_par2(world, room, dir, trap::HURT),
            trap::THROWOFF => set_par2(world, room, dir, trap::THROW),
            _ => {}
        },
        ExitType::Timed => {
            let msg = world
                .messages
                .get(&MessageNo(exit.par1 as i32))
                .map(|m| m.msgs.clone());
            let opening = exit.par3 != EXOPEN;
            set_par3(world, room, dir, if opening { EXOPEN } else { EXLOCKED });
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
            add_exit_to_list(world, room, dir, i32::from(exit.par2));
        }
        ExitType::Hidden => {
            let flags = ExitFlags::from_bits_retain(exit.par1);
            if flags.contains(ExitFlags::VISSEARCHED) {
                set_par1(world, room, dir, ExitFlags::NEEDSEARCH.bits());
            } else if flags.contains(ExitFlags::VISBLOCKED) {
                let n = u32::from(exit.par2.unsigned_abs());
                if (1..=10).contains(&n) {
                    set_par1(world, room, dir, blocked_upto(n));
                }
            }
        }
        ExitType::Key => {
            if i64::from(exit.par2) != i64::from(LOCK_LOCKED) {
                set_par2(world, room, dir, LOCK_LOCKED as i16);
                let line = Msg::DoorRelocked {
                    dir: dirname(world, dir),
                };
                tell_room_all(world, room, [None, None], line);
            }
        }
        ExitType::Door | ExitType::Gate => {
            if exit.par2 > 0 || exit.par1 == LOCK_LOCKED {
                return;
            }
            set_par1(world, room, dir, LOCK_LOCKED);
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
    let _ = LOCK_OPEN;
}
