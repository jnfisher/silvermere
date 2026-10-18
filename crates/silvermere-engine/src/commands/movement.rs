//! Movement commands: the ten directions ([`cmd_move`]), OPEN, CLOSE, LOCK, DRAG and the
//! entry points the fast tick and the typed-exit stage call (`move_user`, `queued_move`). The
//! bodies live in [`crate::movement`]; this file is the dispatch table's view of them.

use silvermere_text::Msg;
use silvermere_types::enums::MessageLevel;
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomKey, SessionId};

use super::dispatch::Handled;
use super::parse::{CommandLine, parse_command, starts_with_ignore_case};
use super::pending::{add_delay, delay_done};
use super::{Cmd, CommandLine as Line};
use crate::movement::doors::{self, no_exits_msg, safe_colliseum};
use crate::movement::exits::{exit_is_available, live_exit, prompt};
use crate::movement::move_user::{get_encumbrance_percent, move_user_with};
use crate::movement::{direction, dirname};
use crate::output::clear_output;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// N, S, E, W, NE, NW, SE, SW, U, D. The move is not made here: with an exit that way the direction
/// goes into `movedir` and the player gets a delay (one tick, two while dragging, one or two more
/// over 66% load, doubled slowed, halved quickened, never below one, one more after two moves in a
/// row); the fast tick carries it out ([`queued_move`]). Moving ends hiding, resting and meditating
/// and leaves the party followed. Held, or over capacity, it is refused; with no exit the player
/// walks into the wall, which the room sees and which ends sneaking. Returns `Wait` (no prompt)
/// when the move was queued.
pub fn cmd_move(world: &mut World, id: SessionId, cmd: Cmd, _line: &CommandLine) -> Handled {
    let Some(dir) = direction(cmd.code() as i16) else {
        return Handled::NotCommand;
    };
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Handled::NotCommand;
    };
    plr.hidden = 0;
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    let name = plr.firstname.clone();
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    if plr.status2.contains(PlayerStatus2::HELD) {
        tell_user(world, id, Msg::CantMoveHeld);
        return Handled::Done;
    }
    if !exit_is_available(world, room, dir) {
        // no exit: into the wall
        tell_user(world, id, Msg::NoExitClr);
        let wall = Msg::RanIntoWall {
            who: name,
            dir: dirname(world, dir),
        };
        tell_room(
            world,
            room,
            [Some(id), None],
            Noise::Medium,
            TellBlind::Sight,
            true,
            None,
            &wall,
        );
        let sneaking = world
            .session(id)
            .and_then(|s| s.player())
            .is_some_and(|p| p.status.contains(PlayerStatus::SNEAKING));
        if sneaking {
            tell_user(world, id, Msg::NoLongerSneaking);
            if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                p.status.remove(PlayerStatus::SNEAKING);
            }
        }
        return Handled::Done;
    }
    // walking away leaves the party being followed
    if world.session(id).is_some_and(|s| s.party.leader.is_some()) {
        crate::party::remove_from_group(world, id);
        if let Some(s) = world.session_mut(id) {
            s.party.leader = None;
        }
    }
    let sneaking = world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.status.contains(PlayerStatus::SNEAKING));
    if sneaking {
        tell_user(world, id, Msg::MoveSneaking);
    }
    // over capacity
    let encumpct = get_encumbrance_percent(world, id);
    if encumpct > 100 {
        tell_user(world, id, Msg::MoveTooHeavyAnywhere);
        return Handled::Done;
    }
    // the move is queued with its delay
    let debug = world
        .session(id)
        .is_some_and(|s| s.flags.debug != silvermere_types::enums::DebugLevel::Off);
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Handled::NotCommand;
    };
    plr.moves = plr.moves.wrapping_add(1);
    let dragging = plr.status.contains(PlayerStatus::DRAGGING);
    let mut delay: i32 = if dragging { 2 } else { 1 };
    if encumpct > 66 {
        delay += if dragging { 2 } else { 1 };
    }
    if plr.status2.contains(PlayerStatus2::SLOWED) {
        delay *= 2;
    }
    if plr.status2.contains(PlayerStatus2::QUICKENED) {
        delay /= 2;
    }
    if delay < 1 {
        delay = 1;
    }
    if plr.moves > 2 {
        delay += 1;
    }
    plr.movedir = dir.code() as i16;
    if debug {
        tell_user(
            world,
            id,
            Msg::MoveDelayDebug {
                delay: i64::from(delay),
            },
        );
    }
    add_delay(world, id, u8::try_from(delay).unwrap_or(u8::MAX));
    Handled::Wait
}

/// OPEN: [`doors::cmd_open`].
pub fn cmd_open(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    doors::cmd_open(world, id, line)
}

/// CLOSE: [`doors::cmd_close`].
pub fn cmd_close(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    doors::cmd_close(world, id, line)
}

/// LOCK: [`doors::cmd_lock`].
pub fn cmd_lock(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    doors::cmd_lock(world, id, line)
}

/// DRAG alone lets go; DRAG {name} takes hold of a player in the room who is down (hit points at or
/// below 0); DRAG {name} {direction} takes hold and queues the move for the fast tick. A player
/// already dragged, or hidden from a dragger without See Hidden, cannot be taken; nobody is dragged
/// out of a colliseum under the arena-safe option; a healthy target, the room and (at message level
/// High) the target hear of the attempt.
///
/// The name is matched by prefix against the players in the room (the full action target
/// search is T-027's; its "which one?" list for several matches is not here, the first match is
/// taken).
pub fn cmd_drag(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Handled::NotCommand;
    };
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    let name = plr.firstname.clone();
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let dragging_flag = plr.status.contains(PlayerStatus::DRAGGING);
    let see_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    if line.argc() == 1 {
        if dragging_flag {
            // let go (the dragging link itself is left as it is, sic)
            let victim = world.session(id).and_then(|s| s.party.dragging);
            let who = victim
                .and_then(|v| world.session(v))
                .and_then(|s| s.player())
                .map_or(String::new(), |p| p.firstname.clone());
            tell_user(world, id, Msg::DragRelease { who });
            if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                p.status.remove(PlayerStatus::DRAGGING);
            }
        }
        return Handled::Done;
    }
    // the second word names another player in the room
    let word = line.arg(1);
    let target = world.session_ids().into_iter().find(|&o| {
        o != id
            && world.is_in_game(o)
            && world.session(o).and_then(|s| s.player()).is_some_and(|p| {
                p.map == room.map
                    && p.room == room.room
                    && starts_with_ignore_case(word, &p.firstname)
            })
    });
    // the match lookup ends, dropping the colour reset (this search carries no list)
    clear_output(world, id);
    let Some(tgt) = target else {
        tell_user(world, id, Msg::DragSyntax);
        return Handled::Done;
    };
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
    let Some(tplr) = world.session(tgt).and_then(|s| s.player()).cloned() else {
        return Handled::Done;
    };
    // is the target dragged by someone already?
    let being_dragged = tplr.hp <= 0
        && world.session_ids().into_iter().any(|o| {
            world.is_in_game(o)
                && world.session(o).is_some_and(|s| {
                    s.party.dragging == Some(tgt)
                        && s.player()
                            .is_some_and(|p| p.status.contains(PlayerStatus::DRAGGING))
                })
        });
    if being_dragged {
        tell_user(
            world,
            id,
            Msg::DragAlready {
                who: tplr.firstname.clone(),
            },
        );
        return Handled::Done;
    }
    if tplr.hidden != 0 && !see_hidden {
        tell_user(world, id, Msg::DragSyntaxHidden);
        return Handled::Done;
    }
    if tplr.hp > 0 {
        // a healthy target
        tell_user(
            world,
            id,
            Msg::DragTooHealthy {
                who: tplr.firstname.clone(),
            },
        );
        if tplr.msglevel == MessageLevel::High {
            tell_user(world, tgt, Msg::DragTriedYou { who: name.clone() });
            prompt(world, tgt);
        }
        let line = Msg::DragAttempted {
            who: name,
            target: tplr.firstname.clone(),
        };
        tell_room(
            world,
            room,
            [Some(id), Some(tgt)],
            Noise::High,
            TellBlind::All,
            true,
            None,
            &line,
        );
        return Handled::Done;
    }
    // down and helpless; a direction after the name?
    let dir_word = (line.argc() > 2)
        .then(|| parse_command(line.arg(2)))
        .flatten();
    match dir_word {
        Some(cmd) => {
            let Some(dir) = direction(cmd.code() as i16) else {
                // a word parse_command knows that is no direction: nothing
                return Handled::Done;
            };
            if safe_colliseum(world, room) {
                tell_user(world, id, Msg::DragColliseum);
                return Handled::Done;
            }
            if live_exit(world, room, dir).is_some_and(|e| e.target.0 != 0) {
                if let Some(s) = world.session_mut(id) {
                    s.party.dragging = Some(tgt);
                    if let Some(p) = s.character.as_mut() {
                        p.status |= PlayerStatus::DRAGGING;
                    }
                }
                if delay_done(world, id) {
                    add_delay(world, id, 1);
                }
                if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                    p.movedir = dir.code() as i16;
                }
                Handled::Wait
            } else {
                tell_user(world, id, no_exits_msg(world, dir));
                Handled::Done
            }
        }
        None => {
            if safe_colliseum(world, room) {
                tell_user(world, id, Msg::DragColliseum);
            } else {
                if let Some(s) = world.session_mut(id) {
                    s.party.dragging = Some(tgt);
                    if let Some(p) = s.character.as_mut() {
                        p.status |= PlayerStatus::DRAGGING;
                    }
                }
                tell_user(
                    world,
                    id,
                    Msg::NowDragging {
                        who: tplr.firstname,
                    },
                );
            }
            Handled::Done
        }
    }
}

/// The move itself, a plain call. Returns whether the player ended up in another room.
pub fn move_user(world: &mut World, id: SessionId, dir: i16, how: i32) -> bool {
    crate::movement::move_user(world, id, dir, how)
}

/// The queued move of `fast_update_character`: a drag when the player drags someone who
/// is still in the game, else a sneak for a sneaking player, else a plain move; the drag
/// ends when its victim is gone.
pub fn queued_move(world: &mut World, id: SessionId, dir: i16) -> bool {
    let Some(s) = world.session(id) else {
        return false;
    };
    let Some(plr) = s.player() else {
        return false;
    };
    let victim = s.party.dragging.filter(|v| world.is_in_game(*v));
    if plr.status.contains(PlayerStatus::DRAGGING) && victim.is_some() {
        // a drag
        return move_user_with(world, id, dir, 2, false, victim);
    }
    let sneaking = plr.status.contains(PlayerStatus::SNEAKING);
    if let Some(s) = world.session_mut(id) {
        s.party.dragging = None;
        if let Some(p) = s.character.as_mut() {
            p.status.remove(PlayerStatus::DRAGGING);
        }
    }
    if sneaking {
        // a sneak
        return move_user_with(world, id, dir, 1, true, None);
    }
    move_user_with(world, id, dir, 0, false, None)
}

/// The followers on lower session ids move after their leader (move type 3); one who is down or
/// cannot follow leaves the party ([`crate::party::follow_step`]).
pub fn followers_move(world: &mut World, leader: SessionId, dir: i16) {
    crate::party::follow_step(world, leader, dir);
}

#[allow(dead_code)]
type LineAlias = Line;
