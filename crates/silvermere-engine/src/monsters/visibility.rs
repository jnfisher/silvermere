//! [`monster_can_see_user`]: whether a monster sees a player, for the target choice of the
//! monsters' round ([`super::combat_ai`]).

use silvermere_types::enums::Ability;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::RoomKey;
use silvermere_types::records::{ActiveMonster, Character};

use super::monster_has_ability;
use crate::world::World;

/// Whether a monster sees a player: the player stands in `room` (the room of the player
/// whose turn it is, not the monster's) and is seen: a hidden player only by a monster with
/// See Hidden; a sneaking one only by a monster with See Hidden and not on the round he
/// arrived (`JUSTARRIVED`); anyone else always.
///
/// (sic) `background_energy` clears `JUSTARRIVED` for every player
/// (`energy_update_character`) before `do_monster_combat` runs, its only caller, so the
/// arrival round never protects anyone there.
pub fn monster_can_see_user(
    world: &World,
    plr: Option<&Character>,
    room: RoomKey,
    mon: &ActiveMonster,
) -> bool {
    let Some(plr) = plr else {
        return false;
    };
    if plr.room != room.room || plr.map != room.map {
        return false;
    }
    if plr.hidden != 0 {
        monster_has_ability(world, mon, Ability::SeeHidden)
    } else if !plr.status.contains(PlayerStatus::SNEAKING) {
        true
    } else {
        !plr.status.contains(PlayerStatus::JUSTARRIVED)
            && monster_has_ability(world, mon, Ability::SeeHidden)
    }
}
