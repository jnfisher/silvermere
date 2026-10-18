//! Items and inventory: get, drop, give, stash, wear, remove, ready, equip, light, keys,
//! wealth, inventory. T-030.
//!
//! The pieces they are built from are in [`crate::items`]. Eating, drinking and USE are
//! T-053's, in [`super::use_item`].
//!
//! 1.11p has no container items and no PUT; `TAKE` is in the command table but has no routine, so
//! the word goes on to exits, spells and speech. `STASH {item}` hides an item or coins in the
//! room's hidden lists (HIDE with a name), where a SEARCH finds them and GET picks them up.

use silvermere_rules::abilities::{Ability, item_has_ability};
use silvermere_rules::economy::{coin_weight, encumbrance_pct};
use silvermere_text::Msg;
use silvermere_types::enums::{Currency, DebugLevel, ItemType};
use silvermere_types::flags::{PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ItemNo, RoomKey, SessionId};
use silvermere_types::records::{Character, Coins};
use silvermere_types::sizes::NINVITEMS;

use super::dispatch::Handled;
use super::{Cmd, CommandLine, eq_ignore_case, starts_with_ignore_case};
use crate::items::equip::{self, remove_armour, tell_others, user_can_use};
use crate::items::inventory::{
    Found, Search, add_item_to_inventory, find_item_in_inventory, load, matches_any_word,
    max_weight, remove_item_from_inventory, show_options, update_encumbrance, user_is_wearing,
};
use crate::items::light::{possessive, use_light};
use crate::items::room_items::{add_item_to_room, mark_changed, remove_item_from_room};
use crate::output::clear_output;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// The five currencies in currency-code order.
const CURRENCIES: [Currency; 5] = [
    Currency::Copper,
    Currency::Silver,
    Currency::Gold,
    Currency::Platinum,
    Currency::Runic,
];

/// The currency's plural name, with the sysop's `USRCURNM` for the fifth.
pub(crate) fn plural(world: &World, cur: Currency) -> String {
    match cur {
        Currency::Runic => world.config.usr_cur_name.clone(),
        c => c.plural_name().unwrap_or("").to_string(),
    }
}

/// The singular for exactly one coin, else the plural (the fifth currency is the sysop's
/// name either way).
pub(crate) fn proper_currency_name(world: &World, amount: i64, cur: Currency) -> String {
    if amount == 1 && cur != Currency::Runic {
        return cur.singular_name().unwrap_or("").to_string();
    }
    plural(world, cur)
}

/// The first currency whose name the word abbreviates (`matches_any_word`).
pub(crate) fn match_currency(world: &World, word: &str) -> Option<Currency> {
    CURRENCIES
        .into_iter()
        .find(|&c| matches_any_word(word, &plural(world, c)))
}

fn coin_of(mut coins: Coins, cur: Currency) -> u32 {
    *coin_mut(&mut coins, cur)
}

pub(crate) fn coin_mut(coins: &mut Coins, cur: Currency) -> &mut u32 {
    match cur {
        Currency::Silver => &mut coins.silver,
        Currency::Gold => &mut coins.gold,
        Currency::Platinum => &mut coins.platinum,
        Currency::Runic => &mut coins.runic,
        _ => &mut coins.copper,
    }
}

/// Leading blanks, a sign, then digits up to the first non-digit (a number past 2147483647
/// wraps negative, as it shows in the original).
pub fn leading_int32(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i32 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        n = n.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
    }
    if neg { n.wrapping_neg() } else { n }
}

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.sessions.get(&id).and_then(|s| s.player())
}

fn player_mut(world: &mut World, id: SessionId) -> Option<&mut Character> {
    world.session_mut(id).and_then(|s| s.character.as_mut())
}

fn room_of(plr: &Character) -> RoomKey {
    RoomKey {
        map: plr.map,
        room: plr.room,
    }
}

/// Doing anything ends resting and meditating.
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(plr) = player_mut(world, id) {
        plr.status.remove(PlayerStatus::RESTING);
        plr.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// The encumbrance and the secondary stats brought up to date after the load changed.
fn reweigh(world: &mut World, id: SessionId) {
    update_encumbrance(world, id);
    equip::recalc(world, id);
}

fn debug_on(world: &World, id: SessionId) -> bool {
    world
        .sessions
        .get(&id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off)
}

/// A cursed item the player wears cannot be got rid of unless a second copy is carried
/// (DROP, STASH and GIVE alike).
fn cursed_and_worn(world: &World, plr: &Character, item: ItemNo) -> bool {
    let Some(itm) = world.items.get(&item) else {
        return false;
    };
    (item_has_ability(itm, Ability::Cursed) || item_has_ability(itm, Ability::MajorCurse))
        && user_is_wearing(plr, item)
        && plr
            .items
            .iter()
            .take(NINVITEMS)
            .filter(|&&n| n == item)
            .count()
            < 2
}

fn tell_room_high(world: &mut World, id: SessionId, room: RoomKey, msg: &Msg) {
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::High,
        TellBlind::All,
        true,
        None,
        msg,
    );
}

/// LIGHT alone tells the light level here; LIGHT {item} lights a carried light source.
pub fn cmd_light(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    let _ = cmd;
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        // how bright is it here?
        let (level, desc) = crate::display::light::light_at(world, id)
            .map_or((0, ""), |(level, desc)| (level, desc.text()));
        tell_user(
            world,
            id,
            Msg::LightLevel {
                desc: desc.to_string(),
            },
        );
        if debug_on(world, id) {
            tell_user(world, id, Msg::LightDebug { level });
        }
        return Handled::Done;
    }
    let name = line.rest_from(1);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let f = find_item_in_inventory(world, plr, name, Search::Light);
    if f.ambiguous() {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the dispatcher's colour reset is thrown away
    clear_output(world, id);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() else {
        return Handled::NotCommand;
    };
    if f.charges != 0 {
        if !user_can_use(world, plr, &itm) {
            // an item the player may not use
            tell_user(world, id, Msg::LightForbidden);
            return Handled::Done;
        }
        if itm.item_type != ItemType::Light {
            // not a light source
            tell_user(world, id, Msg::CannotLight { item: itm.name });
        } else if plr.readied != -1 {
            // one light at a time
            tell_user(world, id, Msg::AlreadyLit);
        } else if use_light(world, id, &itm) {
            // the slot is now the readied light
            let Some(plr) = player_mut(world, id) else {
                return Handled::Done;
            };
            plr.readied = f.slot_code();
            let who = plr.firstname.clone();
            let pron = possessive(plr.sex);
            let room = room_of(plr);
            tell_user(
                world,
                id,
                Msg::LightOk {
                    item: itm.name.clone(),
                },
            );
            tell_others(
                world,
                id,
                room,
                &Msg::OtherLights {
                    who,
                    pron: pron.to_string(),
                    item: itm.name,
                },
            );
        }
    } else if itm.item_type == ItemType::Light {
        // burnt out
        tell_user(world, id, Msg::LightRecharge);
    } else {
        return Handled::NotCommand;
    }
    Handled::Done
}

/// Moves up to `amount` coins of `cur` from the pile `from` into the purse, then puts
/// coins back one at a time while the load is too heavy (at most 500). Returns the amount kept.
fn take_coins(plr: &mut Character, pile: &mut u32, cur: Currency, amount: i32, maxwgt: i32) -> i32 {
    let mut amount = amount;
    *coin_mut(&mut plr.coins, cur) = coin_mut(&mut plr.coins, cur).wrapping_add(amount as u32);
    *pile = pile.wrapping_sub(amount as u32);
    let mut cnt = 0;
    while i32::from(plr.wgt) + coin_weight(plr.coins) > maxwgt
        && *coin_mut(&mut plr.coins, cur) > 0
        && cnt < 500
        && amount > 0
    {
        cnt += 1;
        *coin_mut(&mut plr.coins, cur) -= 1;
        *pile += 1;
        amount -= 1;
    }
    amount
}

/// GET {item}, GET {amount} {currency}, GET {currency}.
pub fn cmd_get(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let room = room_of(plr);
    stop_resting(world, id);
    if line.argc() == 1 {
        tell_user(world, id, Msg::GetSyntax);
        reweigh(world, id);
        return Handled::Done;
    }
    let name = line.rest_from(1);
    // an item of that name lying here, hidden ones included
    let debug = debug_on(world, id);
    if let Some(t) = remove_item_from_room(world, room, name, debug) {
        if add_item_to_inventory(world, id, t.item, t.charges) {
            let item = world
                .items
                .get(&t.item)
                .map(|i| i.name.clone())
                .unwrap_or_default();
            let who = player(world, id)
                .map(|p| p.firstname.clone())
                .unwrap_or_default();
            tell_user(world, id, Msg::GetTook { item: item.clone() });
            tell_room_high(world, id, room, &Msg::OtherPicksUp { who, item });
        } else {
            // back where it was; the refusal is already out
            add_item_to_room(world, room, t.item, t.hidden, t.charges);
        }
        reweigh(world, id);
        return Handled::Done;
    }
    // three coins per unit of carrying capacity left
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let maxwgt = max_weight(world, plr);
    let maxcoins = ((maxwgt - i32::from(plr.wgt) - coin_weight(plr.coins)) * 3).max(0);
    if line.argc() >= 3 {
        let mut amount = leading_int32(line.rest_from(1));
        if amount > 0 && amount < 99999 {
            amount = amount.min(maxcoins);
            match match_currency(world, line.rest_from(2)) {
                Some(cur) => {
                    let got = {
                        let World {
                            sessions,
                            room_state,
                            ..
                        } = world;
                        let (Some(plr), Some(rm)) = (
                            sessions.get_mut(&id).and_then(|s| s.character.as_mut()),
                            room_state.get_mut(&room),
                        ) else {
                            return Handled::NotCommand;
                        };
                        // the pile in plain sight, else the hidden one
                        if i64::from(*coin_mut(&mut rm.coins, cur)) >= i64::from(amount) {
                            Some(take_coins(
                                plr,
                                coin_mut(&mut rm.coins, cur),
                                cur,
                                amount,
                                maxwgt,
                            ))
                        } else if i64::from(*coin_mut(&mut rm.hidcoins, cur)) >= i64::from(amount) {
                            Some(take_coins(
                                plr,
                                coin_mut(&mut rm.hidcoins, cur),
                                cur,
                                amount,
                                maxwgt,
                            ))
                        } else {
                            None
                        }
                    };
                    got_coins(world, id, room, cur, got, amount);
                }
                None => tell_user(
                    world,
                    id,
                    Msg::GetSyntaxCurrency {
                        amount: amount.to_string(),
                    },
                ),
            }
        } else {
            tell_user(world, id, Msg::GetSyntaxAmount);
        }
    } else {
        // GET {currency}: as many as he can carry, from plain sight only
        match match_currency(world, name) {
            Some(cur) => {
                let got = {
                    let World {
                        sessions,
                        room_state,
                        ..
                    } = world;
                    let (Some(plr), Some(rm)) = (
                        sessions.get_mut(&id).and_then(|s| s.character.as_mut()),
                        room_state.get_mut(&room),
                    ) else {
                        return Handled::NotCommand;
                    };
                    let amount = (*coin_mut(&mut rm.coins, cur) as i32).min(maxcoins);
                    (amount > 0)
                        .then(|| take_coins(plr, coin_mut(&mut rm.coins, cur), cur, amount, maxwgt))
                };
                match got {
                    Some(n) => got_coins(world, id, room, cur, Some(n), n),
                    None => {
                        let coin = plural(world, cur);
                        tell_user(world, id, Msg::GetNoAny { coin });
                    }
                }
            }
            None => tell_user(
                world,
                id,
                Msg::GetNotHere {
                    name: name.to_string(),
                },
            ),
        }
    }
    reweigh(world, id);
    Handled::Done
}

/// The end of a coin GET: the coins were there or not.
fn got_coins(
    world: &mut World,
    id: SessionId,
    room: RoomKey,
    cur: Currency,
    got: Option<i32>,
    asked: i32,
) {
    match got {
        Some(n) => {
            mark_changed(world, room);
            let coin = proper_currency_name(world, i64::from(n), cur);
            let who = player(world, id)
                .map(|p| p.firstname.clone())
                .unwrap_or_default();
            tell_user(
                world,
                id,
                Msg::PickedUpCoins {
                    amount: n.to_string(),
                    coin,
                },
            );
            let coin = plural(world, cur);
            tell_room_high(world, id, room, &Msg::OtherPicksUpCoins { who, coin });
        }
        None => {
            let coin = proper_currency_name(world, i64::from(asked), cur);
            tell_user(
                world,
                id,
                Msg::GetNoCoins {
                    amount: asked.to_string(),
                    coin,
                },
            );
        }
    }
}

/// STASH (the same as HIDE with a name): hide a carried item or coins in the room.
pub fn cmd_stash(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() == 1 {
        tell_user(world, id, Msg::StashSyntax);
        return Handled::Done;
    }
    hide_item(world, id, cmd, line)
}

/// The item and coin forms of HIDE, for STASH and for HIDE with a name (the HIDE arm is
/// `misc::cmd_hide`'s, which calls this when a word follows HIDE). Returns `NotCommand` when the
/// words name neither a carried item nor an amount of coins.
pub fn hide_item(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let room = room_of(plr);
    stop_resting(world, id);
    let name = line.rest_from(1);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let f = find_item_in_inventory(world, plr, name, Search::Any);
    if f.ambiguous() {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the dispatcher's colour reset is thrown away
    clear_output(world, id);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let mut rc = Handled::Done;
    if let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() {
        if itm.nodrop != 0 || cursed_and_worn(world, plr, itm.number) {
            tell_user(world, id, Msg::HideForbidden);
            return Handled::Done;
        }
        let readied = plr.readied;
        // hiding something needs light to see by
        if crate::display::light::can_see(world, id).is_none() {
            return Handled::Done;
        }
        if f.slot_code() == readied {
            remove_armour(world, id, name, false);
        }
        let charges = remove_item_from_inventory(world, id, itm.number).unwrap_or(0);
        if add_item_to_room(world, room, itm.number, true, charges).is_some() {
            tell_user(world, id, Msg::HidItem { item: itm.name });
        } else {
            add_item_to_inventory(world, id, itm.number, charges);
            tell_user(world, id, Msg::HideNoRoom { item: itm.name });
        }
    } else if line.argc() >= 3 {
        let amount = leading_int32(name);
        if amount > 0 {
            match match_currency(world, line.rest_from(2)) {
                Some(cur) => {
                    let hidden = {
                        let World {
                            sessions,
                            room_state,
                            ..
                        } = world;
                        match (
                            sessions.get_mut(&id).and_then(|s| s.character.as_mut()),
                            room_state.get_mut(&room),
                        ) {
                            (Some(plr), Some(rm)) if coin_of(plr.coins, cur) >= amount as u32 => {
                                *coin_mut(&mut plr.coins, cur) -= amount as u32;
                                *coin_mut(&mut rm.hidcoins, cur) += amount as u32;
                                true
                            }
                            _ => false,
                        }
                    };
                    let coin = proper_currency_name(world, i64::from(amount), cur);
                    let amount = amount.to_string();
                    if hidden {
                        mark_changed(world, room);
                        tell_user(world, id, Msg::HidCoins { amount, coin });
                    } else {
                        tell_user(world, id, Msg::HideNoCoins { amount, coin });
                    }
                }
                None => tell_user(
                    world,
                    id,
                    Msg::HideSyntaxCurrency {
                        amount: amount.to_string(),
                    },
                ),
            }
        } else {
            rc = Handled::NotCommand;
        }
    } else {
        rc = Handled::NotCommand;
    }
    // the stats again, whatever happened
    equip::recalc(world, id);
    rc
}

/// DROP {item} or DROP {amount} {currency}.
pub fn cmd_drop(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let room = room_of(plr);
    stop_resting(world, id);
    if line.argc() == 1 {
        tell_user(world, id, Msg::DropSyntax);
        reweigh(world, id);
        return Handled::Done;
    }
    let name = line.rest_from(1);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let f: Found = find_item_in_inventory(world, plr, name, Search::Any);
    if f.ambiguous() {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the dispatcher's colour reset is thrown away
    clear_output(world, id);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() {
        // no-drop, and a cursed item worn with no second copy
        if itm.nodrop != 0 || cursed_and_worn(world, plr, itm.number) {
            tell_user(world, id, Msg::DropForbidden);
            return Handled::Done;
        }
        // the lit light is put out first
        if f.slot_code() == plr.readied {
            remove_armour(world, id, name, false);
        }
        let charges = remove_item_from_inventory(world, id, itm.number).unwrap_or(0);
        if add_item_to_room(world, room, itm.number, false, charges).is_some() {
            let who = player(world, id)
                .map(|p| p.firstname.clone())
                .unwrap_or_default();
            tell_user(
                world,
                id,
                Msg::Dropped {
                    item: itm.name.clone(),
                },
            );
            tell_room_high(
                world,
                id,
                room,
                &Msg::OtherDropped {
                    who,
                    item: itm.name,
                },
            );
        } else {
            // the floor is full: it goes back
            add_item_to_inventory(world, id, itm.number, charges);
            tell_user(world, id, Msg::DropNoRoom { item: itm.name });
        }
    } else if line.argc() >= 3 {
        let amount = leading_int32(name);
        if amount > 0 {
            match match_currency(world, line.rest_from(2)) {
                Some(cur) => {
                    let dropped = {
                        let World {
                            sessions,
                            room_state,
                            ..
                        } = world;
                        match (
                            sessions.get_mut(&id).and_then(|s| s.character.as_mut()),
                            room_state.get_mut(&room),
                        ) {
                            (Some(plr), Some(rm)) if coin_of(plr.coins, cur) >= amount as u32 => {
                                *coin_mut(&mut plr.coins, cur) -= amount as u32;
                                *coin_mut(&mut rm.coins, cur) += amount as u32;
                                true
                            }
                            _ => false,
                        }
                    };
                    let coin = proper_currency_name(world, i64::from(amount), cur);
                    if dropped {
                        mark_changed(world, room);
                        let who = player(world, id)
                            .map(|p| p.firstname.clone())
                            .unwrap_or_default();
                        tell_user(
                            world,
                            id,
                            Msg::DroppedCoins {
                                amount: amount.to_string(),
                                coin,
                            },
                        );
                        let coin = plural(world, cur);
                        tell_room_high(world, id, room, &Msg::OtherDroppedCoins { who, coin });
                    } else {
                        tell_user(
                            world,
                            id,
                            Msg::DropNoCoins {
                                amount: amount.to_string(),
                                coin,
                            },
                        );
                    }
                }
                None => tell_user(
                    world,
                    id,
                    Msg::DropSyntaxCurrency {
                        amount: amount.to_string(),
                    },
                ),
            }
        } else {
            tell_user(world, id, Msg::DropSyntaxAmount);
        }
    } else {
        tell_user(
            world,
            id,
            Msg::DropNotHave {
                name: name.to_string(),
            },
        );
    }
    reweigh(world, id);
    Handled::Done
}

/// What the search for monsters and other players found for GIVE.
enum Target {
    /// Nothing, or a monster: GIVE refuses either way.
    NotAPlayer,
    /// One player.
    User(SessionId),
    /// Several candidates: the list was shown.
    Multiple,
}

/// The monster and player steps of the action target search as GIVE asks for them: active
/// monsters in the room whose name fits, then players in the room (not the giver) whose
/// first name fits the first word; a hidden player only for a giver who sees hidden things.
/// A full name ends the search at once. With more than one candidate and the last kind found a
/// player, the list is shown.
///
/// TODO(T-033): the monster step's word dropping belongs to the full action target search;
/// GIVE only needs to know a monster is not a player.
fn find_receiver(world: &mut World, id: SessionId, room: RoomKey, name: &str) -> Target {
    let mut count = 0usize;
    let mut options = Vec::new();
    let mut found: Option<SessionId> = None;
    let mut monster = false;
    // a search brings the room's monsters into the cache (T-086)
    if let Some(st) = world.room_state.get(&room) {
        for &m in st.monsters.iter().filter(|m| m.0 != 0) {
            world.active_monsters.touch(m);
        }
    }
    for m in world.active_monsters.values() {
        if m.map == room.map && m.room == room.room && matches_any_word(name, &m.name) {
            count += 1;
            monster = true;
            if eq_ignore_case(name, &m.name) {
                return Target::NotAPlayer;
            }
            options.push(m.name.clone());
        }
    }
    // only the first word can name a player
    let first = name.split(' ').next().unwrap_or("");
    let sees_hidden = player(world, id).is_some_and(|p| p.status.contains(PlayerStatus::SEEHIDDEN));
    let mut ids: Vec<SessionId> = world.session_ids();
    ids.sort();
    for other in ids {
        if other == id || !world.is_in_game(other) {
            continue;
        }
        let Some(vplr) = player(world, other) else {
            continue;
        };
        if vplr.room != room.room
            || vplr.map != room.map
            || !matches_any_word(first, &vplr.firstname)
        {
            continue;
        }
        if vplr.hidden != 0 && !sees_hidden {
            continue;
        }
        found = Some(other);
        monster = false;
        count += 1;
        if eq_ignore_case(first, &vplr.firstname) {
            return Target::User(other);
        }
        options.push(vplr.firstname.clone());
    }
    if count > 1 && !monster {
        show_options(world, id, &options);
        return Target::Multiple;
    }
    match found {
        Some(t) if !monster => Target::User(t),
        _ => Target::NotAPlayer,
    }
}

/// GIVE {amount} {currency} TO {player}, GIVE {item} TO {player}.
pub fn cmd_give(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let room = room_of(plr);
    stop_resting(world, id);
    if line.argc() < 4 {
        return Handled::NotCommand;
    }
    if let Some(plr) = player_mut(world, id) {
        plr.status.remove(PlayerStatus::SNEAKING);
        plr.hidden = 0;
    }
    let amt = leading_int32(line.arg(1)) as i16 as i32;
    if amt > 0 {
        give_coins(world, id, room, amt, line)
    } else {
        give_item(world, id, room, line)
    }
}

fn give_coins(
    world: &mut World,
    id: SessionId,
    room: RoomKey,
    amt: i32,
    line: &CommandLine,
) -> Handled {
    let Some(cur) = match_currency(world, line.arg(2)) else {
        tell_user(world, id, Msg::GiveSyntax);
        return Handled::Done;
    };
    if !starts_with_ignore_case(line.arg(3), "to") {
        tell_user(world, id, Msg::GiveSyntax);
        return Handled::Done;
    }
    let tgt = find_receiver(world, id, room, line.rest_from(4));
    // the match lookup ends (the colour reset is dropped) unless the name was ambiguous
    if !matches!(tgt, Target::Multiple) {
        clear_output(world, id);
    }
    let tgt = match tgt {
        Target::Multiple => return Handled::Done,
        Target::NotAPlayer => {
            tell_user(world, id, Msg::GiveToWhat);
            return Handled::Done;
        }
        Target::User(t) => t,
    };
    let Some(tplr) = player(world, tgt) else {
        return Handled::Done;
    };
    if tplr.settings.contains(PlayerSettings::NORECEIVE) {
        let who = tplr.firstname.clone();
        tell_user(world, id, Msg::RefusesOffer { who });
        return Handled::Done;
    }
    // three coins per unit of weight the receiver has left, in ints
    let maxwgt = max_weight(world, tplr);
    let avail = (maxwgt - (coin_weight(tplr.coins) + i32::from(tplr.wgt))).wrapping_mul(3);
    let mut amt = amt;
    if avail < 0 {
        amt = 0;
    } else if avail < amt {
        amt = avail;
    }
    let moved = {
        let (giver, receiver) = two_players(world, id, tgt);
        let (Some(plr), Some(tplr)) = (giver, receiver) else {
            return Handled::Done;
        };
        if amt as u32 <= *coin_mut(&mut plr.coins, cur) {
            *coin_mut(&mut plr.coins, cur) -= amt as u32;
            *coin_mut(&mut tplr.coins, cur) += amt as u32;
            let mut j = 0;
            while i32::from(tplr.wgt) + coin_weight(tplr.coins) > maxwgt
                && *coin_mut(&mut tplr.coins, cur) > 0
                && j < 500
                && amt > 0
            {
                j += 1;
                *coin_mut(&mut tplr.coins, cur) -= 1;
                *coin_mut(&mut plr.coins, cur) += 1;
                amt -= 1;
            }
            true
        } else {
            false
        }
    };
    // the words of the coin lines: "copper", "silver", "gold", "platinum", USRCURNM
    let word = match cur {
        Currency::Silver => "silver".to_string(),
        Currency::Gold => "gold".to_string(),
        Currency::Platinum => "platinum".to_string(),
        Currency::Runic => world.config.usr_cur_name.clone(),
        _ => "copper".to_string(),
    };
    if moved {
        let me = player(world, id)
            .map(|p| p.firstname.clone())
            .unwrap_or_default();
        let them = player(world, tgt)
            .map(|p| p.firstname.clone())
            .unwrap_or_default();
        tell_user(
            world,
            id,
            Msg::GaveCoins {
                who: them.clone(),
                amount: amt.to_string(),
                coin: word.clone(),
            },
        );
        if let Some(s) = world.session_mut(tgt) {
            s.output.push(Msg::GotCoins {
                who: me.clone(),
                amount: amt.to_string(),
                coin: word,
            });
            s.push_prompt();
        }
        tell_room(
            world,
            room,
            [Some(id), Some(tgt)],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::GaveSomeCoins {
                who: me,
                whom: them,
            },
        );
    } else if cur == Currency::Runic {
        let coin = plural(world, cur);
        tell_user(world, id, Msg::GiveNotManyRunic { coin });
    } else {
        tell_user(world, id, Msg::GiveNotMuch { coin: word });
    }
    // both records are saved
    crate::player::save_player(world, id);
    crate::player::save_player(world, tgt);
    Handled::Done
}

/// Both characters, mutably (giver, receiver).
fn two_players(
    world: &mut World,
    a: SessionId,
    b: SessionId,
) -> (Option<&mut Character>, Option<&mut Character>) {
    if a == b {
        return (None, None);
    }
    let mut first = None;
    let mut second = None;
    for (k, s) in world.sessions.iter_mut() {
        if *k == a {
            first = s.character.as_mut();
        } else if *k == b {
            second = s.character.as_mut();
        }
    }
    (first, second)
}

fn give_item(world: &mut World, id: SessionId, room: RoomKey, line: &CommandLine) -> Handled {
    let name = line.rest_from(1);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let f = find_item_in_inventory(world, plr, name, Search::Any);
    if f.ambiguous() {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the dispatcher's colour reset is thrown away
    clear_output(world, id);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() else {
        return Handled::NotCommand;
    };
    if itm.nodrop != 0 || cursed_and_worn(world, plr, itm.number) {
        tell_user(world, id, Msg::GiveForbidden);
        return Handled::Done;
    }
    // the readied item is unreadied first (by the first word again)
    if f.slot_code() == plr.readied {
        remove_armour(world, id, line.arg(1), false);
    }
    if !starts_with_ignore_case(line.arg(f.words + 1), "to") {
        return Handled::NotCommand;
    }
    let tgt = match find_receiver(world, id, room, line.rest_from(f.words + 2)) {
        Target::Multiple => return Handled::Done,
        Target::NotAPlayer => return Handled::NotCommand,
        Target::User(t) => t,
    };
    let Some(tplr) = player(world, tgt) else {
        return Handled::Done;
    };
    if tplr.settings.contains(PlayerSettings::NORECEIVE) {
        let who = tplr.firstname.clone();
        tell_user(world, id, Msg::RefusesOfferPlain { who });
        return Handled::Done;
    }
    let charges = remove_item_from_inventory(world, id, itm.number).unwrap_or(0);
    let me = player(world, id)
        .map(|p| p.firstname.clone())
        .unwrap_or_default();
    let them = player(world, tgt)
        .map(|p| p.firstname.clone())
        .unwrap_or_default();
    if add_item_to_inventory(world, tgt, itm.number, charges) {
        crate::player::save_player(world, id);
        crate::player::save_player(world, tgt);
        tell_user(
            world,
            id,
            Msg::GaveItem {
                item: itm.name.clone(),
                who: them.clone(),
            },
        );
        if let Some(s) = world.session_mut(tgt) {
            s.output.push(Msg::GotItem {
                who: me.clone(),
                item: itm.name,
            });
            s.push_prompt();
        }
        tell_room(
            world,
            room,
            [Some(id), Some(tgt)],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::GaveSomething {
                who: me,
                whom: them,
            },
        );
    } else {
        // no room: the receiver has had the refusal, the item goes back
        tell_user(world, id, Msg::CannotAccept { who: them });
        add_item_to_inventory(world, id, itm.number, charges);
    }
    Handled::Done
}

/// ARM: the syntax alone, else on to [`cmd_equip`].
pub fn cmd_arm(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    equip_case(world, id, cmd, line, Msg::ArmSyntax)
}

/// WIELD: the syntax alone, else on to [`cmd_equip`].
pub fn cmd_wield(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    equip_case(world, id, cmd, line, Msg::WieldSyntax)
}

/// WEAR: the syntax alone, else on to [`cmd_equip`].
pub fn cmd_wear(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    equip_case(world, id, cmd, line, Msg::WearSyntax)
}

/// READY: the syntax alone, else on to [`cmd_equip`].
pub fn cmd_ready(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    equip_case(world, id, cmd, line, Msg::ReadySyntax)
}

fn equip_case(
    world: &mut World,
    id: SessionId,
    cmd: Cmd,
    line: &CommandLine,
    syntax: Msg,
) -> Handled {
    if line.argc() == 1 {
        tell_user(world, id, syntax);
        return Handled::Done;
    }
    cmd_equip(world, id, cmd, line)
}

/// EQUIP alone lists the inventory; EQUIP {item} readies a weapon or wears anything else, among the
/// carried items not in use.
pub fn cmd_equip(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if line.argc() == 1 {
        display_inventory_items(world, id);
        return Handled::Done;
    }
    let name = line.rest_from(1);
    let f = find_item_in_inventory(world, plr, name, Search::Unworn);
    // the match lookup ends by clearing the output buffer: the dispatcher's colour reset never
    // reaches the user, so `You are now wearing ...` starts at `ESC[79D` (the `shop` capture, steps
    // 12 and 22-26)
    clear_output(world, id);
    let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() else {
        tell_user(
            world,
            id,
            Msg::NotLeftUnequipped {
                name: name.to_string(),
            },
        );
        return Handled::Done;
    };
    if let Some(plr) = player_mut(world, id) {
        plr.status.remove(PlayerStatus::SNEAKING);
        plr.hidden = 0;
    }
    if itm.item_type == ItemType::Weapon {
        equip::ready_weapon(world, id, Some((name, Some(itm.number))));
    } else {
        equip::wear_armour(world, id, name, Some((itm.number, f.charges)));
    }
    Handled::Done
}

/// REMOVE: taking something off ends sneaking, hiding, resting and meditating; a cursed item stays
/// on.
pub fn cmd_remove(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() == 1 {
        tell_user(world, id, Msg::RemoveSyntax);
        return Handled::Done;
    }
    if let Some(plr) = player_mut(world, id) {
        plr.status
            .remove(PlayerStatus::SNEAKING | PlayerStatus::RESTING);
        plr.hidden = 0;
        plr.status2.remove(PlayerStatus2::MEDITATING);
    }
    remove_armour(world, id, line.rest_from(1), false);
    Handled::Done
}

/// UNREADY (the status changes are in `dispatch`): put the weapon away.
pub fn cmd_unready(world: &mut World, id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    equip::ready_weapon(world, id, None);
    Handled::Done
}

/// The INVENTORY display (T-027, `display::inventory`).
pub fn display_inventory_items(world: &mut World, id: SessionId) {
    crate::display::inventory::display_inventory_items(world, id, id);
}

/// The KEYS display (T-027).
pub fn display_users_keys(world: &mut World, id: SessionId) {
    crate::display::inventory::display_users_keys(world, id);
}

/// WEALTH (T-027); not a command
/// without a character.
pub fn display_user_money(world: &mut World, id: SessionId) -> Handled {
    Handled::from_rc(crate::display::inventory::display_user_money(world, id))
}

/// The weight test the GIVE coin path and `encumbrance` share, for tests.
pub fn encumbrance_of(world: &World, plr: &Character) -> (i64, i32, i16) {
    let max = max_weight(world, plr);
    (load(plr), max, encumbrance_pct(plr.wgt, plr.coins, max))
}
