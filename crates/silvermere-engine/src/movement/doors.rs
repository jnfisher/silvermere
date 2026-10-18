//! The door commands: OPEN, CLOSE, LOCK, PICKLOCK, the door branch of BASH and
//! [`open_door_by_force`].
//!
//! An exit is two one-way exits, one in each room. Every command that changes one side
//! looks for the matching exit back (the far room's exit the opposite way leads here and is
//! of the right type) and changes that side too, telling the players over there. A Door or
//! Gate keeps its lock state in `exitpar1s` and its open time in `exitpar3s`; a Key exit in
//! `exitpar2s` and `exitpar4s`.

use silvermere_text::Msg;
use silvermere_types::enums::{Direction, ExitType, RoomType};
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomKey, SessionId};
use silvermere_types::records::Exit;

use super::exits::{
    LOCK_CLOSED, LOCK_LOCKED, LOCK_OPEN, door_word, live_exit, set_par1, set_par2, tell_room_all,
};
use super::timed_exits::add_exit_to_list;
use super::{HOOKS, direction, dirname};
use crate::commands::dispatch::Handled;
use crate::commands::parse::{CommandLine, parse_command};
use crate::commands::pending::add_delay;
use crate::scheduler::Rng;
use crate::tell::tell_user;
use crate::world::World;

/// The second word as a direction.
fn direction_word(line: &CommandLine) -> Option<Direction> {
    parse_command(line.arg(1)).and_then(|c| direction(c.code() as i16))
}

/// The player's room, name, and the live exit `dir` of it (`None` when there is no exit
/// that way).
fn here(world: &World, id: SessionId, dir: Direction) -> Option<(RoomKey, String, Option<Exit>)> {
    let plr = world.session(id)?.player()?;
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let exit = live_exit(world, room, dir).filter(|e| e.target.0 != 0);
    Some((room, plr.firstname.clone(), exit))
}

/// The player stops resting and meditating.
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// A visible act: the player stops sneaking and is no longer hidden.
fn give_away(world: &mut World, id: SessionId) {
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
}

/// The room on the other side and its exit back, when that exit leads straight back here.
fn other_side(
    world: &World,
    room: RoomKey,
    dir: Direction,
    exit: &Exit,
) -> Option<(RoomKey, Direction, Exit)> {
    let rev = dir.reverse()?;
    let orm = RoomKey {
        map: room.map,
        room: exit.target,
    };
    let back = live_exit(world, orm, rev)?;
    (back.target == room.room).then_some((orm, rev, back))
}

/// The exit's open time in steps: `exitpar3s` for a Door or Gate, `exitpar4s` for a Key
/// exit, at least 1.
fn open_steps(exit: &Exit) -> i32 {
    let steps = if exit.kind == ExitType::Key {
        exit.par4
    } else {
        exit.par3
    };
    steps.max(1)
}

/// Sets the lock state of `exit` in `room` (`exitpar1s` for a Door or Gate, `exitpar2s`
/// for a Key exit).
fn set_lock(world: &mut World, room: RoomKey, dir: Direction, exit: &Exit, state: u32) {
    if exit.kind == ExitType::Key {
        set_par2(world, room, dir, state as i16);
    } else {
        set_par1(world, room, dir, state);
    }
}

/// The lock state of `exit` (`exitpar1s` or `exitpar2s`).
fn lock_state(exit: &Exit) -> u32 {
    if exit.kind == ExitType::Key {
        exit.par2 as u32
    } else {
        exit.par1
    }
}

/// `in_combat` for CLOSE and LOCK: fighting, being attacked, with attackers, or with a
/// monster here that could attack.
fn in_combat(world: &mut World, id: SessionId) -> bool {
    let attackers = world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.attackers > 0);
    HOOKS.combat.in_autocombat(world, id)
        || HOOKS.combat.being_attacked(world, id)
        || attackers
        || HOOKS.combat.monster_could_attack(world, id)
}

/// OPEN, the direction form; a word that is no direction names a container (`OPEN <item>`,
/// T-053's [`crate::commands::use_item::open_item`]).
pub fn cmd_open(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        tell_user(world, id, Msg::OpenSyntax);
        return Handled::Done;
    }
    if HOOKS.combat.in_autocombat(world, id) {
        HOOKS.combat.display_autocombat_broken(world, id);
    }
    HOOKS.combat.kill_autocombat(world, id);
    let Some(dir) = direction_word(line) else {
        // OPEN {item}
        return crate::commands::use_item::open_item(world, id, line);
    };
    let Some((room, name, exit)) = here(world, id, dir) else {
        return Handled::NotCommand;
    };
    let Some(exit) = exit else {
        tell_user(world, id, Msg::NotADoor);
        return Handled::Done;
    };
    give_away(world, id);
    if !matches!(exit.kind, ExitType::Door | ExitType::Gate | ExitType::Key) {
        tell_user(world, id, Msg::NotADoor);
        return Handled::Done;
    }
    let what = door_word(exit.kind).to_owned();
    match lock_state(&exit) {
        LOCK_OPEN => tell_user(world, id, Msg::DoorWasOpen { what }),
        LOCK_CLOSED => {
            set_lock(world, room, dir, &exit, LOCK_OPEN);
            add_delay(world, id, 1);
            add_exit_to_list(world, room, dir, open_steps(&exit));
            // the other side: a door or gate leading straight back (a key exit too, for
            // a key exit here)
            if let Some((orm, rev, back)) = other_side(world, room, dir, &exit) {
                let opens = match back.kind {
                    ExitType::Door | ExitType::Gate => true,
                    ExitType::Key => exit.kind == ExitType::Key,
                    _ => false,
                };
                if opens {
                    set_lock(world, orm, rev, &back, LOCK_OPEN);
                    add_exit_to_list(world, orm, rev, open_steps(&back));
                    let line = Msg::DoorRemoteOpened {
                        what: if back.kind == ExitType::Key {
                            "door".to_owned()
                        } else {
                            what.clone()
                        },
                        dir: dirname(world, rev),
                    };
                    tell_room_all(world, orm, [Some(id), None], line);
                }
            }
            let seen = Msg::DoorSeenOpen {
                who: name,
                what: what.clone(),
                dir: dirname(world, dir),
            };
            tell_room_all(world, room, [Some(id), None], seen);
            tell_user(world, id, Msg::DoorNowOpen { what });
        }
        _ => tell_user(world, id, Msg::DoorIsLocked { what }),
    }
    Handled::Done
}

/// CLOSE.
pub fn cmd_close(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if in_combat(world, id) {
        tell_user(world, id, Msg::CloseNoCombat);
        return Handled::Done;
    }
    if line.argc() == 1 {
        tell_user(world, id, Msg::CloseSyntax);
        return Handled::Done;
    }
    let Some(dir) = direction_word(line) else {
        tell_user(world, id, Msg::CloseSyntax);
        return Handled::Done;
    };
    let Some((room, name, exit)) = here(world, id, dir) else {
        return Handled::NotCommand;
    };
    let Some(exit) = exit else {
        tell_user(world, id, Msg::NotADoor);
        return Handled::Done;
    };
    give_away(world, id);
    if !matches!(exit.kind, ExitType::Door | ExitType::Gate | ExitType::Key) {
        tell_user(world, id, Msg::NotADoor);
        return Handled::Done;
    }
    let what = door_word(exit.kind).to_owned();
    if lock_state(&exit) != LOCK_OPEN {
        // for a Key exit the ternary always says "gate" (sic)
        let what = if exit.kind == ExitType::Key {
            "gate".to_owned()
        } else {
            what
        };
        tell_user(world, id, Msg::DoorNotOpen { what });
        return Handled::Done;
    }
    set_lock(world, room, dir, &exit, LOCK_CLOSED);
    add_delay(world, id, 1);
    if let Some((orm, rev, back)) = other_side(world, room, dir, &exit) {
        let closes = match back.kind {
            ExitType::Door | ExitType::Gate => exit.kind != ExitType::Key,
            ExitType::Key => exit.kind == ExitType::Key,
            _ => false,
        };
        if closes {
            set_lock(world, orm, rev, &back, LOCK_CLOSED);
            let line = Msg::DoorRemoteClosed {
                what: what.clone(),
                dir: dirname(world, rev),
            };
            tell_room_all(world, orm, [Some(id), None], line);
        }
    }
    let seen = Msg::DoorSeenClose {
        who: name,
        what: what.clone(),
        dir: dirname(world, dir),
    };
    tell_room_all(world, room, [Some(id), None], seen);
    tell_user(world, id, Msg::DoorNowClosed { what });
    Handled::Done
}

/// LOCK: a closed door, gate (both only while `exitpar2s <= 0`) or key exit locks, on both
/// sides; locking costs a second tick.
pub fn cmd_lock(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if in_combat(world, id) {
        tell_user(world, id, Msg::LockNoCombat);
        return Handled::Done;
    }
    if line.argc() == 1 {
        tell_user(world, id, Msg::LockSyntax);
        return Handled::Done;
    }
    let Some(dir) = direction_word(line) else {
        tell_user(world, id, Msg::LockSyntaxLower);
        return Handled::Done;
    };
    let Some((room, _, exit)) = here(world, id, dir) else {
        return Handled::NotCommand;
    };
    let Some(exit) = exit else {
        tell_user(world, id, no_exits_msg(world, dir));
        return Handled::Done;
    };
    give_away(world, id);
    add_delay(world, id, 1);
    let lockable = match exit.kind {
        ExitType::Door | ExitType::Gate => exit.par2 <= 0,
        ExitType::Key => true,
        _ => false,
    };
    if !lockable {
        tell_user(world, id, Msg::LockNoBenefit);
        return Handled::Done;
    }
    let what = door_word(exit.kind).to_owned();
    if lock_state(&exit) != LOCK_CLOSED {
        tell_user(world, id, Msg::LockNeedClose { what });
        return Handled::Done;
    }
    set_lock(world, room, dir, &exit, LOCK_LOCKED);
    if let Some((orm, rev, back)) = other_side(world, room, dir, &exit)
        && back.kind == exit.kind
    {
        set_lock(world, orm, rev, &back, LOCK_LOCKED);
    }
    tell_user(world, id, Msg::DoorNowLocked { what });
    add_delay(world, id, 1);
    Handled::Done
}

/// `There are no exits upwards/downwards/to the %s!`.
pub fn no_exits_msg(world: &World, dir: Direction) -> Msg {
    match dir {
        Direction::Up => Msg::NoExitsUp,
        Direction::Down => Msg::NoExitsDown,
        _ => Msg::NoExitsDir {
            dir: dirname(world, dir),
        },
    }
}

/// PICKLOCK: a try costs two ticks and gives the player away; a Key exit's lock opens when
/// a 0-100 roll is under `exitpar3s` plus picklocks, a locked door's or gate's under
/// `exitpar2s` plus picklocks (with picklocks above 0), and relocks after its time; a
/// failure costs two more ticks, and a trapped door casts the room's `picktrap` spell
/// instead of a message.
pub fn cmd_picklock(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        return Handled::NotCommand;
    }
    if HOOKS.combat.in_autocombat(world, id) {
        HOOKS.combat.display_autocombat_broken(world, id);
    }
    HOOKS.combat.kill_autocombat(world, id);
    let Some(dir) = direction_word(line) else {
        tell_user(world, id, Msg::PicklockSyntax);
        return Handled::Done;
    };
    let Some((room, name, exit)) = here(world, id, dir) else {
        return Handled::NotCommand;
    };
    let Some(exit) = exit else {
        tell_user(world, id, Msg::PicklockFail);
        return Handled::Done;
    };
    give_away(world, id);
    add_delay(world, id, 2);
    let picklocks = world
        .session(id)
        .and_then(|s| s.player())
        .map_or(0, |p| i32::from(p.picklocks));
    let what = door_word(exit.kind).to_owned();
    let (lockable, bonus) = match exit.kind {
        ExitType::Key => (true, exit.par3),
        ExitType::Door | ExitType::Gate => (exit.par1 == LOCK_LOCKED, i32::from(exit.par2)),
        _ => (false, 0),
    };
    if !lockable {
        tell_user(world, id, Msg::PicklockFail);
        return Handled::Done;
    }
    let roll = world.rng.range(0, 100);
    if picklocks > 0 && roll < bonus.wrapping_add(picklocks) {
        set_lock(world, room, dir, &exit, LOCK_CLOSED);
        add_exit_to_list(world, room, dir, open_steps(&exit));
        if let Some((orm, rev, back)) = other_side(world, room, dir, &exit) {
            // a Key exit over there is unlocked only from a Key exit
            let unlocks = match back.kind {
                ExitType::Door | ExitType::Gate => true,
                ExitType::Key => exit.kind == ExitType::Key,
                _ => false,
            };
            if unlocks {
                set_lock(world, orm, rev, &back, LOCK_CLOSED);
                add_exit_to_list(world, orm, rev, open_steps(&back));
            }
        }
        let seen = match dir {
            Direction::Up => Msg::PicklockSeenUp {
                who: name,
                what: what.clone(),
            },
            Direction::Down => Msg::PicklockSeenDown {
                who: name,
                what: what.clone(),
            },
            _ => Msg::PicklockSeenDir {
                who: name,
                what: what.clone(),
                dir: dirname(world, dir),
            },
        };
        tell_room_all(world, room, [Some(id), None], seen);
        tell_user(world, id, Msg::Unlocked { what });
    } else {
        add_delay(world, id, 2);
        if exit.kind != ExitType::Key {
            let picktrap = world.room(room).map_or(0, |r| r.picktrap);
            if picktrap != 0 {
                HOOKS
                    .spells
                    .room_cast_on_user(world, id, i32::from(picktrap));
                return Handled::Done;
            }
        }
        tell_user(world, id, Msg::PicklockFail);
    }
    Handled::Done
}

/// The door branch of BASH: `BASH <direction>` at a Door or Gate exit. `None` when the word
/// is no direction, there is no exit that way, or the exit is not a door or gate: then BASH
/// is the attack (T-033's `cmd_any_attack`).
///
/// The chance to break through is strength plus `exitpar2s` against a 0-100 roll; a try
/// costs two ticks; the room sees it either way; one time in four the basher takes 1-4
/// damage, an already open door included (sic). BASH never walks through a Key exit,
/// so that case is not carried.
pub fn cmd_bash_door(world: &mut World, id: SessionId, line: &CommandLine) -> Option<Handled> {
    let dir = direction_word(line)?;
    let (room, name, exit) = here(world, id, dir)?;
    let exit = exit?;
    // there is an exit that way: bashing is noisy
    let str = world
        .session(id)
        .and_then(|s| s.player())
        .map_or(0, |p| i32::from(p.str));
    give_away(world, id);
    let chance = str.wrapping_add(i32::from(exit.par2));
    if HOOKS.combat.in_autocombat(world, id) {
        HOOKS.combat.display_autocombat_broken(world, id);
    }
    HOOKS.combat.kill_autocombat(world, id);
    if !matches!(exit.kind, ExitType::Door | ExitType::Gate) {
        return None;
    }
    let what = door_word(exit.kind).to_owned();
    if exit.par1 == LOCK_OPEN {
        tell_user(world, id, Msg::BashAlreadyOpen { what });
    } else {
        add_delay(world, id, 2);
        if world.rng.range(0, 100) < chance {
            open_door_by_force(world, room, dir, id);
            let seen = match dir {
                Direction::Up => Msg::BashSeenUp {
                    who: name,
                    what: what.clone(),
                },
                Direction::Down => Msg::BashSeenDown {
                    who: name,
                    what: what.clone(),
                },
                _ => Msg::BashSeenDir {
                    who: name,
                    what: what.clone(),
                    dir: dirname(world, dir),
                },
            };
            tell_room_all(world, room, [Some(id), None], seen);
            tell_user(world, id, Msg::BashOk { what });
        } else {
            let seen = match dir {
                Direction::Up => Msg::BashAttemptSeenUp { who: name, what },
                Direction::Down => Msg::BashAttemptSeenDown { who: name, what },
                _ => Msg::BashAttemptSeenDir {
                    who: name,
                    what,
                    dir: dirname(world, dir),
                },
            };
            tell_room_all(world, room, [Some(id), None], seen);
            tell_user(world, id, Msg::BashFail);
        }
    }
    // 1 time in 4 the bash hurts
    if world.rng.range(0, 100) < 25 {
        let dmg = world.rng.range(1, 4);
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.hp = p.hp.wrapping_sub(dmg as i16);
        }
        tell_user(
            world,
            id,
            Msg::BashHurt {
                dmg: i64::from(dmg),
            },
        );
    }
    Some(Handled::Done)
}

/// The door or gate `dir` of `room` bursts open and is queued to lock again after its time;
/// the same door seen from the other side opens too, and the players there (but `unum`) are
/// told it just flew open.
pub fn open_door_by_force(world: &mut World, room: RoomKey, dir: Direction, unum: SessionId) {
    let Some(exit) = live_exit(world, room, dir) else {
        return;
    };
    set_par1(world, room, dir, LOCK_OPEN);
    add_exit_to_list(world, room, dir, exit.par3.max(1));
    let Some((orm, rev, back)) = other_side(world, room, dir, &exit) else {
        return;
    };
    if !matches!(back.kind, ExitType::Door | ExitType::Gate) {
        return;
    }
    add_exit_to_list(world, orm, rev, back.par3.max(1));
    set_par1(world, orm, rev, LOCK_OPEN);
    let what = door_word(exit.kind).to_owned();
    // (sic) "above" and "below" read reversed for the far room
    let line = match dir {
        Direction::Up => Msg::FlewOpenAbove { what },
        Direction::Down => Msg::FlewOpenBelow { what },
        _ => Msg::FlewOpenDir {
            what,
            dir: dirname(world, rev),
        },
    };
    tell_room_all(world, orm, [Some(unum), None], line);
}

/// Whether `room` is a colliseum under the arena-safe option: DRAG's refusal.
pub fn safe_colliseum(world: &World, room: RoomKey) -> bool {
    world.display.arena_safe
        && world
            .room(room)
            .is_some_and(|r| r.room_type == RoomType::Colliseum)
}
