//! A kill's experience shared out.
//!
//! The pot is divided (the integer part, at least 1) among the killer, everyone in
//! auto-combat on the same victim and everyone in auto-combat with no target at all (a
//! caster of a spell that needs none) who stands in the room of the kill. The killer
//! counts even when not queued. The killer and the fighters leave auto-combat with
//! `*Combat Off*`; the casters keep going. A monster victim and a player victim share
//! one function here over [`AutoTarget`].

use silvermere_types::ids::{RoomKey, SessionId};

use super::autocombat::{display_autocombat_broken, is_inside_autocombat, kill_autocombat};
use super::{AutoTarget, player, room_of};
use crate::world::World;

/// One share: `exp / cnt`, and at least 1.
pub fn share(exp: u32, cnt: u32) -> u32 {
    (exp / cnt.max(1)).max(1)
}

/// Whether `id` is in auto-combat with no target (neither a monster nor a player, queued) in
/// `room`.
fn caster_here(world: &World, id: SessionId, room: RoomKey) -> bool {
    world.autocombat.entry(id).target.is_none()
        && player(world, id).is_some_and(|p| room_of(p) == room)
}

/// The share-out of `exp` among those fighting the target in `room`. `killer` is `None` for
/// a kill no player made (the count of sharers still starts at 1). Nothing happens
/// without experience. Each share goes through `add_experience`, verbose (T-039: `You gain
/// N experience.`); the killer and the fighters then stop with `*Combat Off*`. Returns the
/// shares handed out, in channel order.
pub fn distribute_experience(
    world: &mut World,
    killer: Option<SessionId>,
    exp: u32,
    victim: AutoTarget,
    room: RoomKey,
) -> Vec<(SessionId, u32)> {
    if exp == 0 {
        return Vec::new();
    }
    let ids = world.session_ids();
    let mut cnt: u32 = 1;
    for &i in &ids {
        if Some(i) == killer || !is_inside_autocombat(world, i) {
            continue;
        }
        let target = world.autocombat.entry(i).target;
        if target == Some(victim) || caster_here(world, i, room) {
            cnt += 1;
        }
    }
    let each = share(exp, cnt);
    let mut shares = Vec::new();
    for &i in &ids {
        let target = world.autocombat.entry(i).target;
        let queued = is_inside_autocombat(world, i);
        let is_killer = Some(i) == killer;
        if target == Some(victim) || is_killer {
            if queued || is_killer {
                crate::progression::add_experience(
                    world,
                    i,
                    each.min(i32::MAX as u32) as i32,
                    true,
                );
                kill_autocombat(world, i);
                display_autocombat_broken(world, i, false);
                shares.push((i, each));
            }
        } else if target.is_none() && queued && caster_here(world, i, room) {
            crate::progression::add_experience(world, i, each.min(i32::MAX as u32) as i32, true);
            shares.push((i, each));
        }
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::share;

    // killer + n fighters + m casters share alike, the remainder lost, at least 1 each
    #[test]
    fn the_split_table() {
        let table: &[(u32, u32, u32, u32)] = &[
            // (exp, fighters besides the killer, casters, share)
            (15, 0, 0, 15),
            (15, 1, 0, 7),
            (15, 0, 1, 7),
            (15, 1, 1, 5),
            (15, 2, 1, 3),
            (9, 3, 0, 2),
            (3, 2, 2, 1),
            (1, 5, 5, 1),
            (100, 4, 2, 14),
        ];
        for &(exp, n, m, want) in table {
            assert_eq!(share(exp, 1 + n + m), want, "{exp} among 1+{n}+{m}");
        }
    }
}
