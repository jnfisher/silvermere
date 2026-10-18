//! `You notice ... here.`: the items in a room ([`items_here`], [`search_room`]) and the
//! players a search spots ([`users_hidden_in_room`]).

use silvermere_rules::Rng;
use silvermere_text::Msg;
use silvermere_types::enums::Currency;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ItemNo, RoomKey, SessionId};
use silvermere_types::records::{Coins, RoomItem};
use silvermere_types::sizes::NHIDITEMS;

use crate::world::World;

/// The coin words (singular for one, else plural) with the runic coins under the sysop's
/// name.
pub(crate) fn coin_words(world: &World, coins: &Coins) -> Vec<String> {
    let mut out = Vec::new();
    // Runic, whatever the amount
    if coins.runic > 0 {
        out.push(format!("{} {}", coins.runic, world.config.usr_cur_name));
    }
    // Platinum, gold, silver, copper
    for (amount, cur) in [
        (coins.platinum, Currency::Platinum),
        (coins.gold, Currency::Gold),
        (coins.silver, Currency::Silver),
        (coins.copper, Currency::Copper),
    ] {
        if amount > 0 {
            let name = if amount == 1 {
                cur.singular_name()
            } else {
                cur.plural_name()
            };
            out.push(format!("{amount} {}", name.unwrap_or_default()));
        }
    }
    out
}

/// The item tally: one entry per distinct item, counting `1 + slot.count` for each slot
/// that holds it.
///
/// A new item goes to the *last* free entry (the free slot is overwritten for every empty
/// entry the search passes), and the list is read from entry 0 up, so distinct items come
/// out in the reverse of the order their first slot has (`large sign, newbie manual` for
/// room 1/2140, whose slots hold 1098 then 11). Slots whose item no longer exists are
/// emptied.
fn tally<const N: usize>(world: &World, slots: &mut [RoomItem; N]) -> Vec<(ItemNo, i32)> {
    let mut list: [(ItemNo, i32); N] = [(ItemNo(0), 0); N];
    for s in slots.iter_mut() {
        if s.item.0 == 0 {
            continue;
        }
        if !world.items.contains_key(&s.item) {
            *s = RoomItem {
                item: ItemNo(0),
                uses: s.uses,
                count: 0,
            };
            continue;
        }
        add(&mut list, s.item, s.count);
    }
    list.into_iter().filter(|(n, _)| n.0 != 0).collect()
}

/// One item into the tally.
fn add<const N: usize>(list: &mut [(ItemNo, i32); N], item: ItemNo, extra: i16) {
    let mut found = false;
    let mut slot = 0;
    for (j, e) in list.iter_mut().enumerate() {
        if e.0 == item {
            e.1 += 1 + i32::from(extra);
            found = true;
        } else if e.0.0 == 0 {
            slot = j;
        }
    }
    if !found {
        list[slot] = (item, 1 + i32::from(extra));
    }
}

/// `"2 rope, torch"`: the count before an item with more than one copy.
fn item_words(world: &World, list: &[(ItemNo, i32)]) -> Vec<String> {
    list.iter()
        .map(|(n, count)| {
            let name = world.items.get(n).map_or("", |i| i.name.as_str());
            if *count > 1 {
                format!("{count} {name}")
            } else {
                name.to_owned()
            }
        })
        .collect()
}

/// The text after `You notice ` for the coins and items lying in the open, `None` when
/// there is nothing. Items that no longer exist are taken
/// off the room on the way.
pub fn items_here(world: &mut World, key: RoomKey) -> Option<String> {
    let mut state = world.room_state.get(&key)?.clone();
    let mut words = coin_words(world, &state.coins);
    let list = tally(world, &mut state.items);
    words.extend(item_words(world, &list));
    world.room_state.insert(key, state);
    (!words.is_empty()).then(|| words.join(", "))
}

/// The `You notice` line of the room display, or nothing.
pub fn items_msg(world: &mut World, key: RoomKey) -> Option<Msg> {
    items_here(world, key).map(|items| Msg::RoomItems { items })
}

/// The SEARCH command's listing. The hidden coins are always found; each hidden item that
/// can be picked up is found on a roll of 1 to 100 below `perception - 10`, which marks its
/// slot in `foundmask`, flags the player `ST_SEARCHED` and counts a searcher. Then the
/// hidden players are looked for, and `Your search revealed nothing.` when nothing at all
/// turned up. Returns whether anything was found.
pub fn search_room(world: &mut World, id: SessionId) -> bool {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return false;
    };
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let perception = i32::from(plr.perception);
    let Some(mut state) = world.room_state.get(&key).cloned() else {
        return false;
    };
    let mut words = coin_words(world, &state.hidcoins);
    let mut list = [(ItemNo(0), 0); NHIDITEMS];
    let mut searched = false;
    for i in 0..NHIDITEMS {
        let s = state.hiditems[i];
        if s.item.0 == 0 {
            continue;
        }
        match world.items.get(&s.item) {
            Some(itm) => {
                if itm.gettable != 0 && world.rng.range(1, 100) < perception - 10 {
                    // Bits 0..14 only
                    if i < 15 {
                        state.foundmask |= 1 << i;
                    }
                    searched = true;
                    state.searchers = state.searchers.saturating_add(1);
                    add(&mut list, s.item, s.count);
                }
            }
            None => {
                state.hiditems[i].item = ItemNo(0);
                state.hiditems[i].count = 0;
            }
        }
    }
    let list: Vec<(ItemNo, i32)> = list.into_iter().filter(|(n, _)| n.0 != 0).collect();
    words.extend(item_words(world, &list));
    world.room_state.insert(key, state);
    if searched && let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.status |= PlayerStatus::SEARCHED;
    }
    if !words.is_empty() {
        let items = words.join(", ");
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::RoomItems { items });
        }
        users_hidden_in_room(world, id);
        return true;
    }
    // `clear_output`: the "You notice" start goes, and the command's ANSI_NORMAL with it
    crate::output::clear_output(world, id);
    if users_hidden_in_room(world, id) > 0 {
        return true;
    }
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::SearchNothing);
    }
    false
}

/// Every other player in the game hiding in the searcher's room is spotted with See Hidden
/// or on a roll of 0 to 100 below `perception`, comes out of hiding and sneaking and is
/// marked `ST_SPOTTED`; the names go out in one `You see ... hiding in the shadows.` line.
/// Returns how many were spotted.
pub fn users_hidden_in_room(world: &mut World, id: SessionId) -> usize {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return 0;
    };
    let (room, map) = (plr.room, plr.map);
    let see_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    let perception = i32::from(plr.perception);
    let mut names = Vec::new();
    for other in world.session_ids() {
        if other == id || !world.is_in_game(other) {
            continue;
        }
        let Some((vroom, vmap, vhidden)) = world
            .session_mut(other)
            .and_then(|s| s.character.as_ref())
            .map(|v| (v.room, v.map, v.hidden))
        else {
            continue;
        };
        if vroom != room || vmap != map || vhidden == 0 {
            continue;
        }
        // The roll is made only without See Hidden
        if (see_hidden || world.rng.range(0, 100) < perception)
            && let Some(v) = world.session_mut(other).and_then(|s| s.character.as_mut())
        {
            v.status |= PlayerStatus::SPOTTED;
            v.status.remove(PlayerStatus::SNEAKING);
            v.hidden = 0;
            names.push(v.firstname.clone());
        }
    }
    if !names.is_empty()
        && let Some(s) = world.session_mut(id)
    {
        s.output.push(Msg::HidingSeen {
            who: names.join(", "),
        });
    }
    names.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The last free entry takes a new item
    #[test]
    fn tally_fills_from_the_end() {
        let mut list = [(ItemNo(0), 0); 4];
        add(&mut list, ItemNo(1098), 0);
        add(&mut list, ItemNo(11), 0);
        add(&mut list, ItemNo(1098), 2);
        assert_eq!(
            list,
            [
                (ItemNo(0), 0),
                (ItemNo(0), 0),
                (ItemNo(11), 1),
                (ItemNo(1098), 4)
            ]
        );
    }
}
