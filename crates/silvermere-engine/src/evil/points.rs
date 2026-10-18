//! [`add_evil_points`]: the refusals and the charge of an evil act, and
//! [`update_allowed_worn_items`] for a legal level that changed.

use silvermere_rules::evil::{
    self, Attacker, Charge, Kind, LAWFUL_FORCED, Refusal, Victim, legal_level,
};
use silvermere_text::Msg;
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{ItemNo, SessionId};
use silvermere_types::records::Character;

use super::timers::{
    Give, TimerKind, add_evil_timer, already_evil, remove_single_timer, should_give_evil,
};
use crate::items::equip::{unequip_apply_item_abilities, user_can_use};
use crate::tell::tell_user;
use crate::world::World;

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

/// The character is forced lawful (`LAWFUL_FORCED`) and the sysop's `enablawf` is on.
fn is_lawful(world: &World, c: &Character) -> bool {
    c.lawful == LAWFUL_FORCED && world.config.enable_lawful
}

/// Charges an evil act: `true` when the act is refused (the attacker has been told why,
/// and the caller must not go through with it), `false` when it may go ahead.
///
/// Nothing in tournament play. Against a player victim an attack is free when
/// `should_give_evil` says so (retaliation, or a fight already charged), a robbery always
/// replaces the pair's timer (keeping the star of a seen robbery); the cost, the
/// refusals and the scaling are `rules::evil::cost`; then the timer. With
/// no victim (a monster) the same refusals and charge, no scaling and no timer. A changed
/// legal level takes off the items the new one forbids.
pub fn add_evil_points(
    world: &mut World,
    id: SessionId,
    victim: Option<SessionId>,
    points: i16,
    ticks: i8,
    kind: Kind,
) -> bool {
    let Some(plr) = player(world, id) else {
        return false;
    };
    if world.config.tournament_play {
        return false;
    }
    let oldlevel = legal_level(plr.evil);
    let attacker = Attacker {
        evil: plr.evil,
        lawful: is_lawful(world, plr),
        warnings: plr.settings.contains(PlayerSettings::EVILWARN),
    };
    let charge = match victim {
        Some(v) => {
            let Some(tplr) = player(world, v) else {
                return false;
            };
            let target = Victim {
                evil: tplr.evil,
                lawful: is_lawful(world, tplr),
            };
            let mut kind = kind;
            if kind == Kind::Attack {
                match should_give_evil(world, id, v) {
                    Give::None => return false,
                    Give::Renew | Give::New => {}
                }
            } else if already_evil(world, id, v) == Some(TimerKind::Robbery) {
                kind = Kind::Robbery;
            }
            let Some(charge) = settle(world, id, points, attacker, Some(target)) else {
                return true;
            };
            // A new timer for the pair. With `Give::New` there is no old one, so removing it
            // first changes nothing; it keeps the pair to one timer
            remove_single_timer(world, id, v);
            add_evil_timer(world, id, v, ticks, charge.timer_points, kind);
            charge
        }
        None => {
            let Some(charge) = settle(world, id, points, attacker, None) else {
                return true;
            };
            charge
        }
    };
    if legal_level(charge.evil) != oldlevel {
        update_allowed_worn_items(world, id);
    }
    false
}

/// The refusal or the charge: the message, the points and
/// `PS_EVILADDED`. `None` when refused.
fn settle(
    world: &mut World,
    id: SessionId,
    points: i16,
    attacker: Attacker,
    victim: Option<Victim>,
) -> Option<Charge> {
    match evil::cost(points, attacker, victim) {
        Err(r) => {
            let msg = match r {
                Refusal::Warnings => Msg::EvilWarning,
                Refusal::TooEvil => Msg::EvilTooEvil,
                Refusal::Lawful => Msg::EvilLawfulBlock,
            };
            tell_user(world, id, msg);
            None
        }
        Ok(c) => {
            if c.charged {
                tell_user(world, id, Msg::EvilGained);
                if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                    p.evil = c.evil;
                    p.settings.insert(PlayerSettings::EVILADDED);
                }
            }
            Some(c)
        }
    }
}

/// Every worn item,
/// then the weapon, the character may no longer use is taken off with "Your <item> has
/// been removed." (its abilities first for a worn slot, after the message for the
/// weapon); a slot whose item no longer exists is emptied silently. The item list itself
/// is not touched.
pub fn update_allowed_worn_items(world: &mut World, id: SessionId) {
    let Some(plr) = player(world, id).cloned() else {
        return;
    };
    for (i, &n) in plr.worn.iter().enumerate() {
        if n == ItemNo::default() {
            continue;
        }
        if let Some(itm) = world.items.get(&n).cloned() {
            let Some(now) = player(world, id) else {
                return;
            };
            if user_can_use(world, now, &itm) {
                continue;
            }
            unequip_apply_item_abilities(world, id, &itm);
            tell_user(
                world,
                id,
                Msg::ItemRemoved {
                    item: itm.name.clone(),
                },
            );
        }
        if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            p.worn[i] = ItemNo::default();
        }
    }
    let Some(now) = player(world, id) else {
        return;
    };
    let weapon = now.weapon;
    if weapon == ItemNo::default() {
        return;
    }
    if let Some(itm) = world.items.get(&weapon).cloned() {
        if user_can_use(world, now, &itm) {
            return;
        }
        tell_user(
            world,
            id,
            Msg::ItemRemoved {
                item: itm.name.clone(),
            },
        );
        unequip_apply_item_abilities(world, id, &itm);
    }
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.weapon = ItemNo::default();
    }
}
