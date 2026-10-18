//! Robbing: ROB ([`cmd_rob`]), [`rob_user`], [`rob_monster`] and the monsters' rob attack
//! ([`monster_rob_user`]).

use silvermere_rules::Rng;
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, Currency, RoomType};
use silvermere_types::flags::{PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, SessionId};
use silvermere_types::records::Character;

use super::attack::pvp_level_range_ok;
use super::{
    EvilKind, add_evil_points, debug_on, item_has, object_pronoun, player, player_mut, room_of,
    subject_pronoun,
};
use crate::commands::combat::{Found, find_action_target};
use crate::commands::dispatch::Handled;
use crate::commands::pending::add_delay;
use crate::commands::{Cmd, CommandLine};
use crate::items::inventory::{add_item_to_inventory, remove_item_from_inventory, show_options};
use crate::output::clear_output;
use crate::tell::tell_user;
use crate::world::World;

/// `player.lawful` of a character forced lawful.
const LAWFUL_FORCED: i8 = -10;

/// ROB {user/monster}.
pub fn cmd_rob(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player_mut(world, id) else {
        return Handled::NotCommand;
    };
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    if line.argc() == 1 {
        tell_user(world, id, Msg::RobSyntax);
        return Handled::Done;
    }
    let found = find_action_target(world, id, line.rest_from(1), true);
    // the pending output is cleared unless the name was ambiguous
    if !matches!(found, Found::Multiple(_)) {
        clear_output(world, id);
    }
    match found {
        Found::Multiple(options) => show_options(world, id, &options),
        Found::User(target) => {
            reveal(world, id);
            let Some(tplr) = player(world, target) else {
                return Handled::Done;
            };
            let sees =
                player(world, id).is_some_and(|p| p.status.contains(PlayerStatus::SEEHIDDEN));
            if tplr.hidden != 0 && !sees {
                tell_user(world, id, Msg::RobNotSeen);
                return Handled::Done;
            }
            add_delay(world, id, 1);
            rob_user(world, id, target, None);
        }
        Found::Monster(monnum) => {
            reveal(world, id);
            add_delay(world, id, 1);
            rob_monster(world, id, monnum, None);
        }
        Found::None => tell_user(world, id, Msg::RobNotSeen),
    }
    Handled::Done
}

/// Trying a robbery gives the thief away.
fn reveal(world: &mut World, id: SessionId) {
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::SNEAKING);
        p.hidden = 0;
    }
}

/// The coins of one currency (`CUR_COPPER` .. `CUR_RUNIC`) a character carries.
fn purse(c: &mut Character, cur: i32) -> Option<(&mut u32, Currency)> {
    Some(match cur {
        0 => (&mut c.coins.copper, Currency::Copper),
        1 => (&mut c.coins.silver, Currency::Silver),
        2 => (&mut c.coins.gold, Currency::Gold),
        3 => (&mut c.coins.platinum, Currency::Platinum),
        4 => (&mut c.coins.runic, Currency::Runic),
        _ => return None,
    })
}

/// One player robbing another: the refusals (a way of life, yourself, the levels, a
/// protected room or safe arena), the skill roll against `thievery` (a bump the victim
/// feels, or a quiet failure), the evil hook (T-038), then coins (half the time: a random
/// amount of copper, silver, gold or platinum) or an item. `what` names the currency or
/// item; ROB passes `None`.
///
/// (sic) An item is taken only when the one picked carries "Loyal Item"; and the random
/// currency never picks runic.
pub fn rob_user(world: &mut World, id: SessionId, target: SessionId, what: Option<&str>) {
    let (Some(plr), Some(tplr)) = (player(world, id).cloned(), player(world, target).cloned())
    else {
        return;
    };
    if (plr.lawful == LAWFUL_FORCED && world.config.enable_lawful)
        || plr.settings.contains(PlayerSettings::EVILWARN)
    {
        tell_user(world, id, Msg::RobWayOfLife);
        return;
    }
    if id == target {
        tell_user(world, id, Msg::RobYourself);
        return;
    }
    let key = room_of(&plr);
    let Some(rm) = world.room(key).cloned() else {
        return;
    };
    if !pvp_level_range_ok(world, id, target) && !debug_on(world, id) {
        tell_user(world, id, Msg::RobUnbalanced);
        return;
    }
    if rm
        .flags
        .contains(silvermere_types::flags::RoomFlags::PROTECTED)
        || (rm.room_type == RoomType::Colliseum && world.display.arena_safe)
    {
        tell_user(world, id, Msg::RobGuilt);
        return;
    }
    // the skill roll
    let i = world.rng.range(1, 100);
    if i32::from(plr.thievery) + 10 < i {
        if add_evil_points(world, id, Some(target), 1, 11, EvilKind::Robbery) {
            return;
        }
        let tref = player(world, target);
        let him = object_pronoun(tref).to_owned();
        tell_user(
            world,
            id,
            Msg::RobBump {
                who: tplr.firstname.clone(),
                him,
            },
        );
        let he = subject_pronoun(player(world, id)).to_owned();
        if let Some(s) = world.session_mut(target) {
            s.output.push(Msg::RobBumped {
                who: plr.firstname.clone(),
                he,
            });
            s.push_prompt();
        }
        return;
    }
    let fail = |world: &mut World| {
        tell_user(
            world,
            id,
            Msg::RobFail {
                who: tplr.firstname.clone(),
            },
        );
    };
    if i32::from(plr.thievery) < i {
        if add_evil_points(world, id, Some(target), 1, 11, EvilKind::NoStar) {
            return;
        }
        fail(world);
        return;
    }
    if add_evil_points(world, id, Some(target), 1, 11, EvilKind::NoStar) {
        return;
    }
    // what to steal
    let mut cash: Option<i32> = None;
    let mut item = None;
    match what {
        Some(w) => {
            let names = [
                "copper",
                "silver",
                "gold",
                "platinum",
                world.config.usr_cur_name.as_str(),
            ];
            if let Some(c) = names
                .iter()
                .position(|n| crate::commands::parse::starts_with_ignore_case(w, n))
            {
                cash = Some(c as i32);
            } else {
                let found = crate::items::inventory::find_item_in_inventory(
                    world,
                    &tplr,
                    w,
                    crate::items::inventory::Search::Unworn,
                );
                item = found.item.and_then(|n| world.items.get(&n).cloned());
            }
        }
        None => {
            if world.rng.range(1, 100) < 50 {
                cash = Some(world.rng.range(0, 4));
            } else {
                let mut found = false;
                for &n in &tplr.items {
                    if n.0 != 0 && world.rng.range(1, 100) < 50 {
                        let itm = world.items.get(&n).cloned();
                        found = itm
                            .as_ref()
                            .is_some_and(|i| item_has(i, Ability::LoyalItem));
                        item = itm;
                    }
                }
                if !found {
                    item = None;
                    for &n in &tplr.keys {
                        if n.0 != 0 && world.rng.range(1, 100) < 50 {
                            item = world.items.get(&n).cloned();
                        }
                    }
                }
            }
        }
    }
    // coins
    if let Some(cur) = cash {
        let have = player_mut(world, target)
            .and_then(|t| purse(t, cur).map(|(p, _)| *p))
            .unwrap_or(0);
        // the roll takes the low word, a negative one counting as 0
        let low = have as u16 as i16;
        let amount = world.rng.range(0, i32::from(low.max(0))) as u32;
        if amount == 0 {
            fail(world);
            return;
        }
        let Some(curr) = player_mut(world, target).and_then(|t| {
            purse(t, cur).map(|(p, c)| {
                *p -= amount;
                c
            })
        }) else {
            fail(world);
            return;
        };
        if let Some(p) = player_mut(world, id).and_then(|p| purse(p, cur)) {
            *p.0 = p.0.wrapping_add(amount);
        }
        let coin = crate::movement::exits::currency_name(world, i64::from(amount), curr);
        tell_user(
            world,
            id,
            Msg::RobStoleCoins {
                n: amount,
                coin,
                who: tplr.firstname.clone(),
            },
        );
        return;
    }
    // an item
    if let Some(itm) = item.filter(|i| i.robable != 0) {
        let n = itm.number;
        let copies = tplr
            .items
            .iter()
            .chain(&tplr.keys)
            .filter(|&&x| x == n)
            .count();
        let in_use = tplr.worn.contains(&n) || tplr.weapon == n;
        if !(copies == 1 && in_use)
            && let Some(charges) = remove_item_from_inventory(world, target, n)
        {
            if add_item_to_inventory(world, id, n, charges) {
                tell_user(
                    world,
                    id,
                    Msg::RobStoleItem {
                        item: itm.name.clone(),
                        who: tplr.firstname.clone(),
                    },
                );
                return;
            }
            add_item_to_inventory(world, target, n, charges);
        }
    }
    fail(world);
}

/// Robbing a monster, which the game does not offer. Only a character forced lawful
/// who named something is told his way of life prevents it; ROB names nothing, so it does
/// nothing at all.
pub fn rob_monster(world: &mut World, id: SessionId, _monnum: ActiveMonsterId, what: Option<&str>) {
    let Some(plr) = player(world, id) else {
        return;
    };
    if what.is_some() && plr.lawful == LAWFUL_FORCED && world.config.enable_lawful {
        tell_user(world, id, Msg::RobWayOfLife);
    }
}

/// A monster's rob attack: a stub in 1.11p that never steals; `false` lets the monster
/// strike with its first slot instead.
pub fn monster_rob_user(
    _world: &mut World,
    _monnum: ActiveMonsterId,
    _slot: usize,
    _id: SessionId,
) -> bool {
    false
}
