//! The monsters' half of the combat round: [`do_monster_combat`], run by the energy round
//! (`combat::autocombat::background_energy`)
//! through `RoundHooks::monster_combat`, before or after the players' auto-combat on the
//! 60/40 roll.
//!
//! For every player in the game, in the shuffled session order (reshuffled by each slow
//! cycle, `shuffle_mappings`), each monster slot of that
//! player's room in order gets its turn:
//!
//! - a follower of a player: a friendly guard attacks the first player it sees fighting
//!   its master, a charmed friend fights in the master's auto-combat
//!   (`do_charmed_monster_combat`), any other friend that is not Lawful Good, Good or
//!   Neutral attacks the first player it sees other than the master; an unfriendly
//!   follower attacks the master if it sees him;
//! - a hunter of a monster walks one step along the hunted monster's trail, or attacks
//!   it when the trail has no step from here;
//! - anything else picks its target by [`silvermere_rules::monster_ai::choose`].
//!
//! (sic) A monster is visited once per player standing in its room: with two players
//! in a room each monster gets two turns a round. A monster that attacked on its first
//! turn has spent energy, and `attack_monster_user` returns at once below `maxenergy`,
//! but the target's `attackers` count is raised before that call
//! all the same; a guard or Lawful Evil monster that picked nobody rolls again. A hunter
//! that steps into a room whose player comes later in the order steps again there.
//!
//! Each attack adds one to the target's `attackers` count first. The count is zeroed by
//! the medium cycle (`medium_update_character` in `cycles::medium`) every 3 s, so at
//! least once between two energy rounds (5 s): it holds this round's picks plus any free
//! attacks (`give_monsters_a_free_attack`) since the last medium pass.
//! The attack itself, its texts and its blows are T-033's `combat::attack_monster_user`.
//!
//! Not carried: the `stop` flag (never set), the room-cache reload (the slots are read
//! afresh each time), the `GUARD`/`DEFEND`/`HUNT` command stubs.

use silvermere_rules::monster_ai::{Candidate, choose, starts_fights};
use silvermere_types::enums::MonsterType;
use silvermere_types::flags::ActiveMonsterFlags;
use silvermere_types::ids::{ActiveMonsterId, RoomKey, SessionId};
use silvermere_types::sizes::NROOMMONS;

use super::following::check_monster_confusion;
use super::move_monster::{dir_monster_travelling_coord, move_monster};
use super::visibility::monster_can_see_user;
use crate::combat::autocombat::{
    do_charmed_monster_combat, is_combatting_monster, is_combatting_user, is_inside_autocombat,
};
use crate::combat::{attack_monster_monster, attack_monster_user, monster, player, player_mut};
use crate::comms::{UserMatch, get_user_number};
use crate::world::World;

/// The monsters' half of the combat round.
pub fn do_monster_combat(world: &mut World) {
    let order = world.shuffled_sessions();
    for &unum in &order {
        if !world.is_in_game(unum) {
            continue;
        }
        let Some(plr) = player(world, unum) else {
            continue;
        };
        // this player's room; the candidates below are tested against it
        let room = RoomKey {
            map: plr.map,
            room: plr.room,
        };
        for m in 0..NROOMMONS {
            let Some(n) = world.room_state.get(&room).map(|s| s.monsters[m]) else {
                break;
            };
            if n.0 == 0 {
                continue;
            }
            monster_turn(world, &order, room, n);
        }
    }
}

/// One monster's turn in `room`.
fn monster_turn(world: &mut World, order: &[SessionId], room: RoomKey, n: ActiveMonsterId) {
    let Some(mon) = monster(world, n).cloned() else {
        return;
    };
    if !mon.following.is_empty() {
        // the master, by exact name; offline: nothing
        let UserMatch::One(fnum) = get_user_number(world, &mon.following, true) else {
            return;
        };
        if mon.friendly != 0 {
            if mon.montype == MonsterType::Guard {
                // the first player seen fighting the master
                let found = order.iter().copied().find(|&v| {
                    v != fnum
                        && world.is_in_game(v)
                        && monster_can_see_user(world, player(world, v), room, &mon)
                        && is_inside_autocombat(world, v)
                        && is_combatting_user(world, v, fnum)
                });
                if let Some(v) = found {
                    strike(world, n, v);
                }
            } else if mon.flags.contains(ActiveMonsterFlags::CHARMED) {
                if is_inside_autocombat(world, fnum) {
                    do_charmed_monster_combat(world, fnum, n);
                }
            } else if starts_fights(mon.alignment) {
                // the first player seen other than the master
                let found = order.iter().copied().find(|&v| {
                    v != fnum
                        && world.is_in_game(v)
                        && monster_can_see_user(world, player(world, v), room, &mon)
                });
                if let Some(v) = found {
                    strike(world, n, v);
                }
            }
        } else if monster_can_see_user(world, player(world, fnum), room, &mon) {
            // an unfriendly follower goes for its master
            strike(world, n, fnum);
        }
    } else if mon.hunting.0 != 0 {
        // one step along the hunted monster's trail. (sic) With no step from
        // here (the hunted monster is in this room, or this room is not on its trail) the
        // attack is made wherever the hunted monster is.
        let here = RoomKey {
            map: mon.map,
            room: mon.room,
        };
        match dir_monster_travelling_coord(world, mon.hunting, here) {
            Some(dir) => {
                if !check_monster_confusion(world, n) {
                    move_monster(world, n, dir, None, false);
                }
            }
            None => {
                if mon.friendly == 0 {
                    attack_monster_monster(world, n, mon.hunting);
                }
            }
        }
    } else {
        // by type
        let mut seen = Vec::new();
        let mut cands = Vec::new();
        for &v in order {
            if !world.is_in_game(v) {
                continue;
            }
            let Some(p) = player(world, v) else {
                continue;
            };
            if !monster_can_see_user(world, Some(p), room, &mon) {
                continue;
            }
            cands.push(Candidate {
                evil: p.evil,
                attackers: p.attackers,
                fighting_me: is_combatting_monster(world, v, n),
            });
            seen.push(v);
        }
        if let Some(i) = choose(mon.montype, mon.alignment, &cands, &mut world.rng) {
            strike(world, n, seen[i]);
        }
    }
}

/// The target's `attackers` count goes up by one, then the monster attacks him
/// (`attack_monster_user`).
fn strike(world: &mut World, n: ActiveMonsterId, target: SessionId) {
    if let Some(p) = player_mut(world, target) {
        p.attackers = p.attackers.wrapping_add(1);
    }
    attack_monster_user(world, n, target);
}
