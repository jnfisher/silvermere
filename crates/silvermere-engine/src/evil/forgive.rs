//! [`attempt_to_forgive`]: the victim forgives the
//! attacker; FORGIVE (`commands::evil`).

use silvermere_text::Msg;
use silvermere_types::ids::SessionId;

use super::points::update_allowed_worn_items;
use crate::tell::tell_user;
use crate::world::World;

fn name(world: &World, id: SessionId) -> String {
    world
        .session(id)
        .and_then(|s| s.player())
        .map(|p| p.firstname.clone())
        .unwrap_or_default()
}

/// The victim forgives the attacker: the attacker's
/// timer on the victim goes and the points it charged come off the attacker again (worn
/// items are checked again). `true` when a timer was forgiven. With sysop debug on, the
/// victim (the current user) sees the search.
///
/// (sic, not carried) Under the original a forgiveness can upset the other timers; here
/// exactly the forgiven timer is removed.
pub fn attempt_to_forgive(world: &mut World, attacker: SessionId, victim: SessionId) -> bool {
    if world.session(attacker).and_then(|s| s.player()).is_none() {
        return false;
    }
    if world.evil_timers.is_empty() {
        return false;
    }
    let debug = crate::combat::debug_on(world, victim);
    if debug {
        let msg = Msg::ForgiveDebug {
            vnum: i64::from(victim.0),
            victim: name(world, victim),
            anum: i64::from(attacker.0),
            attacker: name(world, attacker),
        };
        tell_user(world, victim, msg);
    }
    let mut found = None;
    for (i, t) in world.evil_timers.clone().iter().enumerate() {
        // The trace covers the timers after the first
        if i > 0 && debug {
            let msg = Msg::ForgiveDebugList {
                anum: i64::from(t.attacker.0),
                attacker: name(world, t.attacker),
                vnum: i64::from(t.victim.0),
                victim: name(world, t.victim),
            };
            tell_user(world, victim, msg);
        }
        if t.victim == victim && t.attacker == attacker {
            found = Some(i);
            break;
        }
    }
    let Some(i) = found else {
        return false;
    };
    let t = world.evil_timers.remove(i);
    if let Some(p) = world
        .session_mut(attacker)
        .and_then(|s| s.character.as_mut())
    {
        p.evil = p.evil.wrapping_sub(t.points);
    }
    update_allowed_worn_items(world, attacker);
    true
}
