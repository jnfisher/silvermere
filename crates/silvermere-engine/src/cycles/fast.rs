//! The per-player part of the fast cycle, every `secfast` (1) second:
//! [`fast_update_characters`] and [`fast_update_character`].

use silvermere_rules::regen;
use silvermere_text::Msg;
use silvermere_types::enums::{DebugLevel, ExitType};
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{RoomId, RoomKey, SessionId};
use silvermere_types::sizes::NTRAIL;

use super::slow::{clamp_i16, clamp_mana};
use super::{MoveKind, prompt};
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// Runs the per-player fast update for every session in the realm, in id order.
pub fn fast_update_characters(world: &mut World) {
    for id in world.session_ids() {
        if world.is_in_game(id) {
            fast_update_character(world, id);
        }
    }
}

/// One player's fast update, in the order its lines come out: follow the leader, rest, meditate,
/// the stun, then the command delay and queue.
pub fn fast_update_character(world: &mut World, id: SessionId) {
    if world.session(id).and_then(|s| s.player()).is_none() {
        return;
    }
    follow_leader(world, id);
    rest_tick(world, id);
    meditate_tick(world, id);
    stun_tick(world, id);
    (world.cycles.hooks.delay_step)(world, id);
}

/// A follower whose leader walked off this cycle walks after him (move type 3); one who is
/// down, whose leader left the game, or who cannot make the move leaves the party line.
fn follow_leader(world: &mut World, id: SessionId) {
    let Some(s) = world.session(id) else {
        return;
    };
    let Some(leader) = s.party.leader else {
        return;
    };
    let up = s.player().is_some_and(|p| p.hp > 0);
    let keep = if up && world.is_in_game(leader) {
        match direction_player_travelling(world, leader, id) {
            Some(dir) => (world.cycles.hooks.move_user)(world, id, dir, MoveKind::Follow),
            None => true,
        }
    } else {
        false
    };
    if !keep && let Some(s) = world.session_mut(id) {
        s.party.leader = None;
    }
}

/// The way `traveller` left
/// `viewer`'s room, from the traveller's trail: the exit of the viewer's room that leads
/// to the room after it in the trail, unless that exit is a remote-action one. `None`
/// when either has no character, both stand in the same room number (the map is not
/// compared, sic), or the trail does not lead out of the viewer's room.
pub fn direction_player_travelling(
    world: &World,
    traveller: SessionId,
    viewer: SessionId,
) -> Option<u8> {
    let plr = world.session(traveller)?.player()?;
    let vplr = world.session(viewer)?.player()?;
    if plr.room == vplr.room {
        return None;
    }
    for i in 1..NTRAIL {
        if plr.trailrooms[i] == vplr.room && plr.trailmaps[i] == vplr.map {
            let rm = world.room(RoomKey {
                map: vplr.map,
                room: vplr.room,
            })?;
            let next = plr.trailrooms[i - 1];
            for (j, exit) in rm.exits.iter().enumerate() {
                if exit.target != RoomId(0)
                    && exit.target == next
                    && exit.kind != ExitType::RemoteAction
                {
                    return Some(j as u8);
                }
            }
        }
    }
    None
}

/// While resting, every 21st fast tick heals the slow amount
/// tripled ([`regen::rest_hp_regen`]), never above the maximum, and the prompt is sent
/// again (also at full hit points).
fn rest_tick(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let debug = s.flags.debug != DebugLevel::Off;
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if !plr.status.contains(PlayerStatus::RESTING) || !regen::rest_tick(&mut s.ticks.resttick) {
        return;
    }
    if plr.hp != plr.maxhp {
        let r = regen::rest_hp_regen(plr.level, plr.hea, plr.healrate);
        if plr.hp < plr.maxhp {
            if debug {
                s.output.push(Msg::RestDebug { regen: r.into() });
            }
            plr.hp = clamp_i16(i32::from(plr.hp) + r);
        }
        if plr.hp > plr.maxhp {
            plr.hp = plr.maxhp;
        }
        if plr.hp > 0 {
            plr.aided = 0;
        }
    }
    prompt(world, id);
}

/// While meditating, every 15th fast tick restores the slow mana
/// amount without the `manarate` scaling ([`regen::meditate_mana_regen`]); once mana is
/// full (or already was) the meditation ends with the wake line. The prompt is sent
/// again either way.
fn meditate_tick(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let debug = s.flags.debug != DebugLevel::Off;
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if !plr.status2.contains(PlayerStatus2::MEDITATING)
        || !regen::meditate_tick(&mut s.ticks.medtick)
    {
        return;
    }
    let mut wake = true;
    if i32::from(plr.mana) < i32::from(plr.maxmana) {
        if let Some(cls) = world.classes.get(&plr.class) {
            let stat = regen::class_stat(cls.magery, plr.intel, plr.wil, plr.cha);
            let r = regen::meditate_mana_regen(cls.magery, stat, plr.level, cls.magelvl);
            plr.mana = clamp_mana(i32::from(plr.mana) + r, plr.maxmana);
            if debug {
                s.output.push(Msg::MeditateDebug { regen: r.into() });
            }
        }
        wake = i32::from(plr.mana) >= i32::from(plr.maxmana);
    }
    if wake {
        plr.mana = clamp_i16(i32::from(plr.maxmana));
        plr.status2.remove(PlayerStatus2::MEDITATING);
        s.output.push(Msg::MeditateDone);
    }
    prompt(world, id);
}

/// Knocked to the floor (`ST2_KNOCKEDDOWN`), `pvptimer` counts
/// down one a fast tick; when it runs out the player gets up, and the room is told with
/// its prompt.
fn stun_tick(world: &mut World, id: SessionId) {
    let Some(plr) = world
        .sessions
        .get_mut(&id)
        .and_then(|s| s.character.as_mut())
    else {
        return;
    };
    if !plr.status2.contains(PlayerStatus2::KNOCKEDDOWN) {
        return;
    }
    plr.pvptimer = plr.pvptimer.saturating_sub(1);
    if plr.pvptimer > 0 {
        return;
    }
    plr.status2.remove(PlayerStatus2::KNOCKEDDOWN);
    let other = Msg::StunRiseOther {
        who: plr.firstname.clone(),
    };
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    crate::tell::tell_user(world, id, Msg::StunRise);
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &other,
    );
}

/// The default [`super::CycleHooks::delay_step`]: the command delay and typed-ahead
/// queue, T-024's `commands::pending::fast_update_delay` (which hands the exit wait to
/// T-021's `exit_countdown`).
pub fn delay_step(world: &mut World, id: SessionId) {
    crate::commands::pending::fast_update_delay(world, id);
}
