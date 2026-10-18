//! The evil timers.
//!
//! A timer says the attacker attacked or robbed the victim lately: for its rounds the
//! victim may strike back for free (`should_give_evil`), both see the evil star
//! (`display_evil_star`), and the attacker may not slip into a protected room or GOTO
//! away while it carries points (`is_in_retaliation`). The list is oldest first; one timer
//! per pair at most.

use silvermere_rules::evil::{Kind, OUTLAW_POINTS};
use silvermere_types::ids::SessionId;

use crate::world::World;

/// The loop guard of `decrement_evil_timers`.
const MAX_TIMER_VISITS: usize = 1000;

/// One evil timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvilTimer {
    /// `attacker`: the session who earned the points.
    pub attacker: SessionId,
    /// `victim`: the session attacked or robbed.
    pub victim: SessionId,
    /// `rounds`: slow ticks left (a signed byte).
    pub rounds: i8,
    /// `points`: the evil points the act charged (0 against an evil victim).
    pub points: i16,
    /// `flags & ET_ROBBED`: a robbery, not an attack.
    pub robbed: bool,
    /// `flags & ET_NOSTAR`: the victim is not shown the evil star.
    pub nostar: bool,
}

/// The kind of the timer [`already_evil`] finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    /// `EV_ATTACK`.
    Attack,
    /// `EV_ROBBERY`.
    Robbery,
    /// `EV_ROBNOSTAR`.
    RobNoStar,
}

/// What [`should_give_evil`] decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Give {
    /// 0: retaliation, or a fight already charged.
    None,
    /// 1: an attack replaces the attacker's robbery timer.
    Renew,
    /// 2: no timer either way; a new one starts.
    New,
}

/// The kind of the timer the
/// attacker holds on the victim.
pub fn already_evil(world: &World, attacker: SessionId, victim: SessionId) -> Option<TimerKind> {
    world
        .evil_timers
        .iter()
        .find(|t| t.victim == victim && t.attacker == attacker)
        .map(|t| match (t.robbed, t.nostar) {
            (false, _) => TimerKind::Attack,
            (true, true) => TimerKind::RobNoStar,
            (true, false) => TimerKind::Robbery,
        })
}

/// Whether `attacker`'s act on `victim` is charged, and how its timer goes: see [`Give`].
pub fn should_give_evil(world: &World, attacker: SessionId, victim: SessionId) -> Give {
    if already_evil(world, victim, attacker).is_some() {
        return Give::None;
    }
    match already_evil(world, attacker, victim) {
        Some(TimerKind::Attack) => Give::None,
        Some(_) => Give::Renew,
        None => Give::New,
    }
}

/// Whether `attacker` shows the evil star to `victim`: `false` only when the
/// attacker robbed this viewer unseen (a `KIND_NOSTAR` timer).
pub fn display_evil_star(world: &World, attacker: SessionId, viewer: SessionId) -> bool {
    !world
        .evil_timers
        .iter()
        .any(|t| t.victim == viewer && t.attacker == attacker && t.nostar)
}

/// The star after `other`'s name in `viewer`'s `Also here:`: a timer either way or 40 or
/// more evil points, unless `other` robbed `viewer` unseen.
pub fn shows_evil_star(
    world: &World,
    viewer: SessionId,
    other: SessionId,
    other_evil: i16,
) -> bool {
    (already_evil(world, viewer, other).is_some()
        || already_evil(world, other, viewer).is_some()
        || other_evil >= OUTLAW_POINTS)
        && display_evil_star(world, other, viewer)
}

/// Appends a timer unless the pair has one.
pub fn add_evil_timer(
    world: &mut World,
    attacker: SessionId,
    victim: SessionId,
    ticks: i8,
    points: i16,
    kind: Kind,
) {
    if already_evil(world, attacker, victim).is_some() {
        return;
    }
    world.evil_timers.push(EvilTimer {
        attacker,
        victim,
        rounds: ticks,
        points,
        robbed: kind != Kind::Attack,
        nostar: kind == Kind::NoStar,
    });
}

/// Drops the pair's timer.
pub fn remove_single_timer(world: &mut World, attacker: SessionId, victim: SessionId) {
    world
        .evil_timers
        .retain(|t| !(t.victim == victim && t.attacker == attacker));
}

/// Drops the expired timers. (sic) The first timer is only
/// dropped when it is the only one; until then it keeps counting down, and its signed
/// byte wraps from -128 to 127.
fn remove_expired_evil_timers(world: &mut World) {
    let timers = &mut world.evil_timers;
    match timers.len() {
        0 => {}
        1 => {
            if timers[0].rounds <= 0 {
                timers.clear();
            }
        }
        _ => {
            let mut i = 1;
            while i < timers.len() {
                if timers[i].rounds <= 0 {
                    timers.remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Called for each player by `slow_update_character`: one round off every timer the
/// attacker holds (at most 1000 visited), then the expired ones go.
pub fn decrement_evil_timers(world: &mut World, attacker: SessionId) {
    if world.evil_timers.is_empty() {
        return;
    }
    for t in world.evil_timers.iter_mut().take(MAX_TIMER_VISITS) {
        if t.attacker == attacker {
            t.rounds = t.rounds.wrapping_sub(1);
        }
    }
    remove_expired_evil_timers(world);
}

/// The player holds a timer that charged points. Used by `user_allowed_in_room` and
/// SYSOP GOTO.
pub fn is_in_retaliation(world: &World, id: SessionId) -> bool {
    world
        .evil_timers
        .iter()
        .any(|t| t.attacker == id && t.points != 0)
}
