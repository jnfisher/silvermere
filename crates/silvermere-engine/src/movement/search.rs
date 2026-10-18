//! SEARCH: `cmd_search` and `search_for_hidden_exits`. The room search itself (the room's
//! items shown in search mode) is T-025's `display::room_items::search_room`.

use silvermere_text::Msg;
use silvermere_types::enums::{Direction, ExitType};
use silvermere_types::flags::{ExitFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomKey, SessionId};

use super::exits::{live_exit, set_par1, tell_room_all};
use super::timed_exits::add_exit_to_list;
use super::{HOOKS, direction, dirname};
use crate::commands::dispatch::Handled;
use crate::commands::parse::{CommandLine, parse_command};
use crate::commands::pending::add_delay;
use crate::display::can_see;
use crate::display::room_items::search_room;
use crate::scheduler::Rng;
use crate::tell::tell_user;
use crate::world::World;

/// SEARCH: searching ends resting, meditating, sneaking and hiding; in the dark or blinded
/// the try still costs a tick; refused while attacking; alone it searches the room for
/// hidden things and tells the room; with a direction it looks for a hidden exit that way;
/// more words are not this command. Every search costs one tick of delay and ends without a
/// prompt.
pub fn cmd_search(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Handled::NotCommand;
    };
    plr.status
        .remove(PlayerStatus::RESTING | PlayerStatus::SNEAKING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    plr.hidden = 0;
    let name = plr.firstname.clone();
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    if can_see(world, id).is_none() {
        add_delay(world, id, 1);
        return Handled::Done;
    }
    if HOOKS.combat.in_autocombat(world, id) {
        tell_user(world, id, Msg::SearchNoAttack);
        add_delay(world, id, 1);
        return Handled::Done;
    }
    match line.argc() {
        1 => {
            search_room(world, id);
            tell_room_all(
                world,
                room,
                [Some(id), None],
                Msg::SearchingArea { who: name },
            );
            add_delay(world, id, 1);
            Handled::Wait
        }
        2 => {
            match parse_command(line.arg(1)).and_then(|c| direction(c.code() as i16)) {
                Some(dir) => search_for_hidden_exits(world, id, dir),
                None => tell_user(world, id, Msg::SearchWhat),
            }
            add_delay(world, id, 1);
            Handled::Wait
        }
        _ => Handled::NotCommand,
    }
}

/// A search for a hidden exit `dir`: the room sees the search; a Hidden exit that needs
/// searching is found when a 0-100 roll is under perception less 15 (at least 3), marked
/// found and queued to hide again after one step; a Trap exit is found (reported only) when
/// the roll comes under the traps skill; otherwise `You notice nothing different ...`.
pub fn search_for_hidden_exits(world: &mut World, id: SessionId, dir: Direction) {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    let (name, perception, traps) = (plr.firstname.clone(), plr.perception, plr.traps);
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let Some(exit) = live_exit(world, room, dir) else {
        return;
    };
    tell_room_all(
        world,
        room,
        [Some(id), None],
        Msg::SearchingExits { who: name },
    );
    let found = match exit.kind {
        ExitType::Hidden
            if ExitFlags::from_bits_retain(exit.par1).contains(ExitFlags::NEEDSEARCH) =>
        {
            let threshold = i32::from(perception - 15).max(3);
            if world.rng.range(0, 100) < threshold {
                set_par1(world, room, dir, ExitFlags::VISSEARCHED.bits());
                tell_user(
                    world,
                    id,
                    match dir {
                        Direction::Up => Msg::SearchFoundUp,
                        Direction::Down => Msg::SearchFoundDown,
                        _ => Msg::SearchFoundDir {
                            dir: dirname(world, dir),
                        },
                    },
                );
                add_exit_to_list(world, room, dir, 1);
                true
            } else {
                false
            }
        }
        ExitType::Trap if world.rng.range(0, 100) < i32::from(traps) => {
            tell_user(
                world,
                id,
                match dir {
                    Direction::Up => Msg::SearchTrapAbove,
                    Direction::Down => Msg::SearchTrapBelow,
                    _ => Msg::SearchTrapDir {
                        dir: dirname(world, dir),
                    },
                },
            );
            true
        }
        _ => false,
    };
    if !found {
        tell_user(
            world,
            id,
            match dir {
                Direction::Up => Msg::SearchNothingAbove,
                Direction::Down => Msg::SearchNothingBelow,
                _ => Msg::SearchNothingDir {
                    dir: dirname(world, dir),
                },
            },
        );
    }
}
