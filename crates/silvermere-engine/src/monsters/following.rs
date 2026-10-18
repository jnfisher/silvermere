//! The monster bodies of the medium and fast cycles: [`medium_update_monster`] (death and
//! wandering), [`fast_update_monster`] (getting up, following a player, giving up) and
//! [`check_monster_confusion`]. T-037's cycles call them for the monster in each cache slot
//! the passes step through (`cycles::monsters`, T-086). They work on the record in hand
//! (`ActiveMonsters::peek`), leaving the cache as it is.
//!
//! The spell rounds of the medium update are T-050's `spells::active::tick_monster_spells`;
//! the death is T-033's, reached through `CycleHooks::check_kill_monster`.

use silvermere_rules::spawn::{GIVEUPRNDS, MAXMOVERS, wander_chance};
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Direction, MonsterType};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus};
use silvermere_types::ids::{ActiveMonsterId, RoomKey, SessionId};
use silvermere_types::sizes::NTRAIL;

use super::active::remove_active_monster;
use super::monster_has_ability;
use super::move_monster::{
    Moved, dir_monster_travelling_coord, move_monster, pick_valid_random_direction,
};
use crate::movement::exits::tell_room_all;
use crate::scheduler::Rng;
use crate::world::World;

/// The medium cycle's monster body without the spell rounds (T-052): a monster below 0 hit
/// points dies; one that follows nobody heads for the monster it hunts or wanders by type
/// (Lair and NPC never; a Guard when changed this round or a mover place is left; others
/// with `(100-followpct)/2` percent while a place is left; at most [`MAXMOVERS`] a pass;
/// not after fighting, not charmed, not confused, never back along `lastdir`); then the
/// "fought" mark is cleared.
pub fn medium_update_monster(world: &mut World, n: ActiveMonsterId) {
    // the spell rounds (T-050), before the death test
    crate::spells::active::tick_monster_spells(world, n);
    let Some(mon) = world.active_monsters.peek(n).cloned() else {
        return;
    };
    // dead: the experience goes to nobody (no killer), every fight against it
    // ends
    if mon.hp < 0 && (world.cycles.hooks.check_kill_monster)(world, n) {
        crate::combat::kill_autocombat_against_monster(world, n);
        return;
    }
    if mon.following.is_empty() {
        let here = RoomKey {
            map: mon.map,
            room: mon.room,
        };
        if mon.hunting.0 != 0 {
            // head for the monster it hunts
            if let Some(dir) = dir_monster_travelling_coord(world, mon.hunting, here)
                && !check_monster_confusion(world, n)
            {
                move_monster(world, n, dir, None, false);
            }
        } else {
            match mon.montype {
                MonsterType::Lair | MonsterType::Npc => {}
                MonsterType::Guard => {
                    // a guard takes a mover place before the confusion test.
                    // `dirty` (changed since the last write) is the record differing
                    // from its last saved copy.
                    let dirty = world.active_monsters.is_dirty(n);
                    if (dirty || world.spawn.mediummoved < MAXMOVERS)
                        && !mon.flags.contains(ActiveMonsterFlags::FOUGHT)
                        && !mon.flags.contains(ActiveMonsterFlags::CHARMED)
                    {
                        world.spawn.mediummoved += 1;
                        if !check_monster_confusion(world, n) {
                            wander(world, n, here, mon.lastdir);
                        }
                    }
                }
                _ => {
                    if world.spawn.mediummoved < MAXMOVERS
                        && !mon.flags.contains(ActiveMonsterFlags::FOUGHT)
                        && !mon.flags.contains(ActiveMonsterFlags::CHARMED)
                        && world.rng.range(0, 100) < wander_chance(mon.followpct)
                        && !check_monster_confusion(world, n)
                    {
                        world.spawn.mediummoved += 1;
                        wander(world, n, here, mon.lastdir);
                    }
                }
            }
        }
    }
    // the round is over
    if let Some(m) = world.active_monsters.peek_mut(n) {
        m.flags.remove(ActiveMonsterFlags::FOUGHT);
    }
}

/// A random usable exit, unless it is the one the monster came by last. No usable exit (-1)
/// never equals a `lastdir` of a real direction, and `move_monster` with no exit does
/// nothing.
fn wander(world: &mut World, n: ActiveMonsterId, here: RoomKey, lastdir: i16) {
    let Some(dir) = pick_valid_random_direction(world, here) else {
        return;
    };
    if lastdir != dir.code() as i16 {
        move_monster(world, n, dir, None, false);
    }
}

/// The fast cycle's monster body: a knocked-down monster counts its down rounds and gets
/// up; a monster following a player goes after the player who left its room (seen, not just
/// arrived, on its map, charmed or with `followpct` percent), and every round it fails to
/// keep up counts; past [`GIVEUPRNDS`] it gives up (an Angel leaves the game).
pub fn fast_update_monster(world: &mut World, n: ActiveMonsterId) {
    let Some(mon) = world.active_monsters.peek(n).cloned() else {
        return;
    };
    if mon.flags.contains(ActiveMonsterFlags::DOWN) {
        let mut rose = false;
        if let Some(m) = world.active_monsters.peek_mut(n) {
            m.downrounds -= 1;
            if m.downrounds <= 0 {
                m.flags.remove(ActiveMonsterFlags::DOWN);
                rose = true;
            }
        }
        if rose {
            let msg = Msg::StunRiseOther {
                who: format!("the {}", mon.name),
            };
            let room = RoomKey {
                map: mon.map,
                room: mon.room,
            };
            tell_room_all(world, room, [None, None], msg);
        }
    }
    if mon.following.is_empty() {
        return;
    }
    let mut lost = false;
    match find_player(world, &mon.following) {
        Some(unum) => {
            let plr = world.session(unum).and_then(|s| s.player()).cloned();
            if let Some(plr) = plr {
                let unseen = plr.hidden != 0 || plr.status.contains(PlayerStatus::SNEAKING);
                let follows = plr.room != mon.room
                    && (!unseen || monster_has_ability(world, &mon, Ability::SeeHidden))
                    && !plr.status.contains(PlayerStatus::JUSTARRIVED)
                    && plr.map == mon.map
                    && (mon.flags.contains(ActiveMonsterFlags::CHARMED)
                        || world.rng.range(0, 100) < i32::from(mon.followpct));
                if follows {
                    // a confused monster stays without losing the round
                    let room = RoomKey {
                        map: mon.map,
                        room: mon.room,
                    };
                    match dir_player_travelling_coord(world, unum, room) {
                        None => lost = true,
                        Some(dir) => {
                            if !check_monster_confusion(world, n)
                                && move_monster(world, n, dir, None, false) == Moved::No
                            {
                                lost = true;
                            }
                        }
                    }
                } else if plr.room != mon.room {
                    lost = true;
                }
            }
        }
        None => lost = true,
    }
    let Some(m) = world.active_monsters.peek_mut(n) else {
        return;
    };
    if lost {
        m.inactive = m.inactive.saturating_add(1);
    }
    // it gives up
    if m.inactive > GIVEUPRNDS {
        if m.montype != MonsterType::Angel {
            m.inactive = 0;
            m.following.clear();
            if m.flags.contains(ActiveMonsterFlags::CHARMED) {
                m.flags.remove(ActiveMonsterFlags::CHARMED);
                // every spell carrying Enslave Creature ends (its end-of-spell effects
                // are T-052's); a slot whose spell is gone is cleared
                let spells = m.spells;
                for (i, s) in spells.iter().enumerate() {
                    if s.spell.0 == 0 {
                        continue;
                    }
                    let clear = world.spells.get(&s.spell).is_none_or(|spl| {
                        spl.abilities.iter().any(|(a, _)| *a == Ability::Enslave)
                    });
                    if clear && let Some(m) = world.active_monsters.peek_mut(n) {
                        m.spells[i] = silvermere_types::records::ActiveSpell {
                            spell: silvermere_types::ids::SpellNo(0),
                            value: 0,
                            rounds: 0,
                        };
                    }
                }
            }
        } else {
            remove_active_monster(world, n, true);
        }
    }
}

/// The player in the realm with that first name.
fn find_player(world: &World, name: &str) -> Option<SessionId> {
    world.session_ids().into_iter().find(|id| {
        world.is_in_game(*id)
            && world
                .session(*id)
                .and_then(|s| s.player())
                .is_some_and(|p| p.firstname.eq_ignore_ascii_case(name))
    })
}

/// The way the player left `room`, from the player's trail: the exit of `room` that leads
/// to the room after it in the trail.
pub fn dir_player_travelling_coord(
    world: &World,
    unum: SessionId,
    room: RoomKey,
) -> Option<Direction> {
    let plr = world.session(unum)?.player()?;
    if plr.room == room.room {
        return None;
    }
    for i in 1..NTRAIL {
        if plr.trailrooms[i] == room.room
            && plr.trailmaps[i] == room.map
            && let Some(rm) = world.room(room)
        {
            for (j, e) in rm.exits.iter().enumerate() {
                if e.target.0 != 0 && e.target == plr.trailrooms[i - 1] {
                    return Some(Direction::COMPASS[j]);
                }
            }
        }
    }
    None
}

/// Whether the monster is too confused to act: T-050's
/// [`crate::spells::active::check_monster_confusion`].
pub fn check_monster_confusion(world: &mut World, n: ActiveMonsterId) -> bool {
    crate::spells::active::check_monster_confusion(world, n)
}
