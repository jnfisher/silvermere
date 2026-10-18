//! [`move_monster`]: a monster through an exit, with the gates of its own movement (held,
//! slowed, knocked down, group discipline, the room-group rule, the exit kinds and traps),
//! the departure and arrival lines, the noise next door and a group leader's followers;
//! [`pick_valid_random_direction`] and [`dir_monster_travelling_coord`] for the wanderers
//! and hunters.
//!
//! Called by the medium and fast monster passes ([`super::following`]), by
//! `handle_following_monsters` when a player walks out (through
//! `movement::MonsterHooks`), and by T-036's hunting, with the arguments (monster,
//! direction, causing player, follow).

use silvermere_rules::spawn;
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Direction, ExitType, GroupRole, MonsterType, RoomType};
use silvermere_types::flags::ActiveMonsterFlags;
use silvermere_types::ids::{ActiveMonsterId, RoomId, RoomKey, SessionId};
use silvermere_types::sizes::{NAMTRAIL, NDIRS, NROOMMONS};

use super::active::{add_monster_to_room, save_room, take_monster_from_room};
use super::monster_has_ability;
use crate::display::movement_noise::display_entry_movement;
use crate::movement::dirname;
use crate::movement::exits::{live_exit, tell_room_all};
use crate::scheduler::Rng;
use crate::tell::tell_user;
use crate::world::World;

/// What [`move_monster`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// 0: it stayed.
    No,
    /// 1: it moved.
    Yes,
    /// 2: no such active monster.
    NoSuch,
}

/// A monster moves through an exit. `usrnum` is the player who caused the move (`None`:
/// none); `follow` is set for a member moving after its group leader.
pub fn move_monster(
    world: &mut World,
    monnum: ActiveMonsterId,
    dir: Direction,
    usrnum: Option<SessionId>,
    follow: bool,
) -> Moved {
    let Some(mon) = world.active_monsters.by_number(monnum).cloned() else {
        return Moved::NoSuch;
    };
    let Some(kmon) = world.monsters.get(&mon.knownnum).cloned() else {
        return Moved::No;
    };
    // held: never; slowed: half the time
    if monster_has_ability(world, &mon, Ability::HoldPerson) {
        return Moved::No;
    }
    if monster_has_ability(world, &mon, Ability::Slowness) && world.rng.range(0, 100) < 50 {
        return Moved::No;
    }
    // knocked down: getting up takes the turn
    if mon.flags.contains(ActiveMonsterFlags::DOWN) {
        if let Some(m) = world.active_monsters.by_number_mut(monnum) {
            m.downrounds -= 1;
            if m.downrounds <= 0 {
                m.flags.remove(ActiveMonsterFlags::DOWN);
            }
        }
        return Moved::No;
    }
    let start = RoomKey {
        map: mon.map,
        room: mon.room,
    };
    // a group member on its own waits for a leader of its group here; a
    // leader waits for a higher-level leader of its group
    if !follow && matches!(mon.grouprole, GroupRole::Follower | GroupRole::Leader) {
        let isleader = mon.grouprole == GroupRole::Leader;
        let Some(slots) = world.room_state.get(&start).map(|s| s.monsters) else {
            return Moved::No;
        };
        for other in slots.into_iter().take(NROOMMONS) {
            if other.0 == 0 || other == monnum {
                continue;
            }
            let Some(o) = world.active_monsters.by_number(other) else {
                continue;
            };
            if o.grouprole == GroupRole::Leader
                && (!isleader || o.level > kmon.level)
                && world
                    .monsters
                    .get(&o.knownnum)
                    .is_some_and(|k| k.group == kmon.group)
            {
                return Moved::No;
            }
        }
    }
    // stationary monsters never move
    if mon.grouprole == GroupRole::Stationary {
        return Moved::No;
    }
    let Some(exit) = live_exit(world, start, dir) else {
        return Moved::No;
    };
    if exit.target.0 == 0 {
        return Moved::No;
    }
    let to = RoomKey {
        map: mon.map,
        room: exit.target,
    };
    let Some(torm) = world.room(to).cloned() else {
        return Moved::No;
    };
    // monsters keep to rooms of their own group. An Angel may go into another group's
    // room only when it has no flags at all and is no Colliseum (sic, as the original
    // moves Angels), and that is kept. A Quest monster goes anywhere; a Guard only into
    // RF_PATROL rooms and never through a Block Guard exit.
    if torm.mongroup != mon.montype.code() as i16 {
        let sic_noflags = torm.flags.is_empty();
        let angel_may = mon.montype == MonsterType::Angel
            && sic_noflags
            && torm.room_type != RoomType::Colliseum;
        if !angel_may && mon.montype != MonsterType::Quest {
            if mon.montype != MonsterType::Guard {
                return Moved::No;
            }
            if !torm
                .flags
                .contains(silvermere_types::flags::RoomFlags::PATROL)
            {
                return Moved::No;
            }
            // the Block Guard rule: only here, for a guard leaving its group; other
            // monsters take the exit as a plain one
            if exit.kind == ExitType::BlockGuard {
                return Moved::No;
            }
        }
    }
    // the kind of exit
    let trap_damage = |rng: &mut crate::scheduler::SeededRng| -> i16 {
        let p1 = i32::try_from(exit.par1).unwrap_or(i32::MAX);
        rng.range(p1 / 2, p1.saturating_add(1)) as i16
    };
    match exit.kind {
        ExitType::Trap if matches!(exit.par2, 0 | 2 | 3) => {
            let d = trap_damage(&mut world.rng);
            if let Some(m) = world.active_monsters.by_number_mut(monnum) {
                m.hp = m.hp.wrapping_sub(d);
            }
        }
        ExitType::SpellTrap if matches!(exit.par2, 0 | 2 | 3) => {
            let d = trap_damage(&mut world.rng);
            if let Some(m) = world.active_monsters.by_number_mut(monnum) {
                m.hp = m.hp.wrapping_sub(d).max(0);
            }
        }
        ExitType::Key if exit.par2 != 0 && mon.montype != MonsterType::Quest => {
            return Moved::No;
        }
        ExitType::Gate | ExitType::Door
            if exit.par1 != 0
                && !matches!(mon.montype, MonsterType::Guard | MonsterType::Quest) =>
        {
            return Moved::No;
        }
        ExitType::Spell
        | ExitType::Item
        | ExitType::Toll
        | ExitType::Hidden
        | ExitType::MapChange
        | ExitType::RemoteAction => return Moved::No,
        _ => {}
    }
    // the move
    if let Some(m) = world.active_monsters.by_number_mut(monnum) {
        m.lastdir = dir.code() as i16;
    }
    let mut moved = Moved::No;
    if take_monster_from_room(world, start, monnum).is_some() {
        if add_monster_to_room(world, to, monnum).is_some() {
            moved = Moved::Yes;
            announce(world, &mon.name, start, to, dir, usrnum);
            if let Some(m) = world.active_monsters.by_number_mut(monnum) {
                m.room = to.room;
                m.trail.copy_within(0..NAMTRAIL - 1, 1);
                m.trail[0] = to.room;
            }
            save_room(world, to);
        } else {
            add_monster_to_room(world, start, monnum);
        }
        save_room(world, start);
    }
    // a leader that moved on its own takes its group along
    if moved == Moved::Yes && !follow && mon.grouprole == GroupRole::Leader {
        take_followers(world, start, dir, usrnum, &kmon);
    }
    moved
}

/// The departure line for the room left and the arrival line for the room entered, then
/// the noise next door.
fn announce(
    world: &mut World,
    name: &str,
    from: RoomKey,
    to: RoomKey,
    dir: Direction,
    usrnum: Option<SessionId>,
) {
    let mon = name.to_string();
    let left = match dir {
        Direction::Up => Msg::MonsterLeftUp { mon: mon.clone() },
        Direction::Down => Msg::MonsterLeftDown { mon: mon.clone() },
        _ => Msg::MonsterLeftDir {
            mon: mon.clone(),
            dir: dirname(world, dir),
        },
    };
    tell_room_all(world, from, [usrnum, None], left);
    let rev = dir.reverse().unwrap_or(Direction::North);
    let arrive = |world: &World, sic: bool| match rev {
        Direction::Up => Msg::MonsterArrivesAbove { mon: mon.clone() },
        Direction::Down => Msg::MonsterArrivesBelow { mon: mon.clone() },
        _ if sic => Msg::MonsterArrivesDirSic {
            mon: mon.clone(),
            dir: dirname(world, rev),
        },
        _ => Msg::MonsterArrivesDir {
            mon: mon.clone(),
            dir: dirname(world, rev),
        },
    };
    match usrnum {
        Some(u) => {
            // the player who caused it gets the line; the room gets the second
            // copy, which leaves out "room" (sic)
            let m = arrive(world, false);
            tell_user(world, u, m);
            let m = arrive(world, true);
            tell_room_all(world, to, [Some(u), None], m);
        }
        None => {
            let m = arrive(world, false);
            tell_room_all(world, to, [None, None], m);
        }
    }
    display_entry_movement(world, Some(rev), to);
}

/// Members of the leader's group in the room it left, and leaders of the group with a lower
/// level, follow, up to `maxfollow` of them. (sic) The known monster is reused for each
/// follower looked at, so from the first on the limit is that monster's `maxfollow`, not
/// the leader's.
fn take_followers(
    world: &mut World,
    left: RoomKey,
    dir: Direction,
    usrnum: Option<SessionId>,
    leader: &silvermere_types::records::KnownMonster,
) {
    let group = leader.group;
    let level = leader.level;
    let mut maxfollow = leader.maxfollow;
    let mut cnt = 0u8;
    for i in 0..NROOMMONS {
        if cnt >= maxfollow {
            break;
        }
        let Some(n) = world.room_state.get(&left).map(|s| s.monsters[i]) else {
            return;
        };
        if n.0 == 0 {
            continue;
        }
        let Some(m) = world.active_monsters.by_number(n).cloned() else {
            continue;
        };
        if m.grouprole == GroupRole::Follower
            || (m.grouprole == GroupRole::Leader && m.level < level)
        {
            let Some(k) = world.monsters.get(&m.knownnum).cloned() else {
                continue;
            };
            maxfollow = k.maxfollow;
            if k.group == group {
                cnt += 1;
                move_monster(world, m.number, dir, usrnum, true);
            }
        }
    }
}

/// The exit kinds a random move may take.
pub fn walkable(kind: ExitType) -> bool {
    matches!(
        kind,
        ExitType::Normal
            | ExitType::Key
            | ExitType::Action
            | ExitType::Door
            | ExitType::Gate
            | ExitType::BlockGuard
            | ExitType::SpellTrap
    )
}

/// A random direction among the room's exits a random move may take.
pub fn pick_valid_random_direction(world: &mut World, room: RoomKey) -> Option<Direction> {
    let exits = world.room(room)?.exits;
    let usable: [bool; NDIRS] =
        std::array::from_fn(|i| exits[i].target.0 != 0 && walkable(exits[i].kind));
    spawn::random_direction(usable, &mut world.rng).map(|i| Direction::COMPASS[i])
}

/// The way the monster left `room`, from its trail (room numbers only): the exit of `room`
/// that leads to the room after it in the trail.
pub fn dir_monster_travelling_coord(
    world: &World,
    monster: ActiveMonsterId,
    room: RoomKey,
) -> Option<Direction> {
    let mon = world.active_monsters.by_number(monster)?;
    if mon.room == room.room {
        return None;
    }
    for i in 1..NAMTRAIL {
        if mon.trail[i] == room.room
            && let Some(rm) = world.room(room)
        {
            let next: RoomId = mon.trail[i - 1];
            for (j, e) in rm.exits.iter().enumerate() {
                if e.target.0 != 0 && e.target == next {
                    return Some(Direction::COMPASS[j]);
                }
            }
        }
    }
    None
}
