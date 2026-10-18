//! `You hear movement ...`: [`display_entry_movement`].
//!
//! The room description has no movement line for the exit a player came through: the
//! original never shows one (sic), so it is not carried.

use silvermere_text::Msg;
use silvermere_types::enums::{Direction, ExitType};
use silvermere_types::ids::RoomKey;

use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// The exit kinds sound passes through.
pub fn carries_sound(kind: ExitType) -> bool {
    matches!(
        kind,
        ExitType::Normal
            | ExitType::Item
            | ExitType::Toll
            | ExitType::Door
            | ExitType::Trap
            | ExitType::Gate
    )
}

/// The line the listeners beyond exit `dir` hear: from the direction of their own way back
/// (the reverse of `dir`).
pub fn noise_msg(world: &World, dir: Direction) -> Option<Msg> {
    Some(match dir.reverse()? {
        Direction::Up => Msg::NoiseAbove,
        Direction::Down => Msg::NoiseBelow,
        back => Msg::Noise {
            dir: world.display.dirnames.get(usize::from(back.code())).into(),
        },
    })
}

/// Someone entered `room` through `from` (`None`: not through an exit). Everyone in the
/// rooms at the other end of its other sound-carrying exits, on the same map, hears it with
/// their prompt, blinded players included. Called by the movement and monster cards (T-026,
/// T-035).
pub fn display_entry_movement(world: &mut World, from: Option<Direction>, room: RoomKey) {
    let Some(rm) = world.room(room) else {
        return;
    };
    let exits = rm.exits;
    for (i, exit) in exits.iter().enumerate() {
        let dir = Direction::COMPASS[i];
        if Some(dir) == from || exit.target.0 == 0 || !carries_sound(exit.kind) {
            continue;
        }
        let Some(msg) = noise_msg(world, dir) else {
            continue;
        };
        // Nobody excepted, blind players too, with the prompt, without colour.
        tell_room(
            world,
            RoomKey {
                map: room.map,
                room: exit.target,
            },
            [None, None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &msg,
        );
    }
}
