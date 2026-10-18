//! A character's pack: [`add_item_to_inventory`], [`remove_item_from_inventory`],
//! [`deduct_item_charge`], [`user_is_wearing`], the item search [`find_item_in_inventory`]
//! with its name matcher [`matches_any_word`], and the weights: [`max_weight`],
//! [`update_encumbrance`].
//!
//! The pack is the record's fixed slots: `items[NINVITEMS]`/`itemuses[]` (0 = empty),
//! `keys[NKEYS]`/`keyuses[]`, `worn[NWORN]`/`wornuses[]`, `weapon` and `readied` (the
//! inventory slot of the lit light, -1 none). A worn or wielded item stays in its inventory
//! slot; the worn slots and `weapon` hold its item number. Duplicates are allowed
//! everywhere.

use silvermere_rules::abilities::{Ability, AbilityLookup, CharacterAbilities, item_ability_value};
use silvermere_rules::economy::{coin_weight, encumbrance_pct};
use silvermere_text::Msg;
use silvermere_types::enums::ItemType;
use silvermere_types::ids::{ItemNo, RoomKey, SessionId};
use silvermere_types::records::{Character, Item};
use silvermere_types::sizes::{NINVITEMS, NKEYS, NWORN};

use super::equip;
use super::limited::{self, Holder};
use crate::commands::{eq_ignore_case, starts_with_ignore_case};
use crate::cycles::medium::printf_s;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Most passes of the word-dropping search.
pub const MAXPASSES: usize = 50;

/// Does `typed` abbreviate a word of `name`? Each word
/// of the name is tried in turn with `starts_with_ignore_case` (case not counting), so "dag", "sil"
/// and "silver" all match "silver dagger". An empty `typed` matches, as `starts_with_ignore_case`
/// does.
pub fn matches_any_word(typed: &str, name: &str) -> bool {
    // The whole name first (this covers its first word)
    if starts_with_ignore_case(typed, name) {
        return true;
    }
    // Then each later word, after the blanks that end the one before
    let mut rest = name;
    loop {
        let Some(end) = rest.find(' ') else {
            return false;
        };
        rest = rest[end..].trim_start_matches(' ');
        if rest.is_empty() {
            return false;
        }
        if starts_with_ignore_case(typed, rest) {
            return true;
        }
    }
}

/// The `SRCH_` flags of [`find_item_in_inventory`] that narrow a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Search {
    /// `SRCH_INVENTORY`: anything carried.
    Any,
    /// `SRCH_UNWORN`: not worn, not the readied light, not the wielded weapon.
    Unworn,
    /// `SRCH_WORN`: only items in use.
    Worn,
    /// `SRCH_INVENTORY|SRCH_LIGHT`: light sources.
    Light,
    /// `SRCH_INVENTORY|SRCH_FOOD`.
    Food,
    /// `SRCH_INVENTORY|SRCH_DRINK`.
    Drink,
}

/// What `find_item_in_inventory` found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// The item chosen (the first candidate, or the exact name), if any.
    pub item: Option<ItemNo>,
    /// Its uses left (`*charges`).
    pub charges: i16,
    /// Its inventory slot (`*slot`, -1 as `None`). A key leaves it as the inventory pass
    /// left it.
    pub slot: Option<usize>,
    /// Words of the input that named it (`*words`), when the result was unique.
    pub words: usize,
    /// Candidates counted (`*count`); 0 after an exact name.
    pub count: usize,
    /// The "Please be more specific" list, in order.
    pub options: Vec<String>,
}

impl Found {
    /// More than one candidate: the caller shows [`Found::options`] and stops.
    pub fn ambiguous(&self) -> bool {
        self.count > 1
    }

    /// The slot as an `i8`, -1 for none: what the callers compare with
    /// `player.readied` (a key found on the key ring leaves -1, which equals an unlit
    /// `readied`).
    pub fn slot_code(&self) -> i8 {
        self.slot.map_or(-1, |s| s as i8)
    }
}

/// The start of the last word of `s`, 0 for a single word.
fn last_word_start(s: &str) -> usize {
    s.trim_end_matches(' ').rfind(' ').map_or(0, |i| i + 1)
}

/// The word count: one more than the blanks after the first character.
fn count_words(s: &str) -> usize {
    1 + s.bytes().skip(1).filter(u8::is_ascii_whitespace).count()
}

/// The item search, for a search that starts with no candidates (every caller in this
/// module).
///
/// The inventory slots first: every slot whose item name fits (`matches_any_word`) and passes the
/// filter is a candidate, except further copies of the first candidate; an exact name
/// (`eq_ignore_case`) wins and ends the search. When nothing fits, the last word of the input is
/// cut off and the search runs again, until something fits, one word is left or
/// [`MAXPASSES`] passes were made. Then, unless an exact name was found, the key ring the
/// same way, never cutting more words than the inventory pass did; a key that fits with
/// fewer words cut replaces the inventory candidates. Item numbers with no record are
/// logged and skipped.
pub fn find_item_in_inventory(world: &World, plr: &Character, name: &str, flags: Search) -> Found {
    let mut f = Found::default();
    let mut lastnum = ItemNo(0);
    let mut exact = false;
    let mut cur = name.to_string();
    let mut ndropped = 0usize;
    // Step 1, the inventory slots
    let mut matched = false;
    let mut passes = 0;
    loop {
        passes += 1;
        for (i, &num) in plr.items.iter().enumerate().take(NINVITEMS) {
            if exact {
                break;
            }
            if num == ItemNo(0) {
                continue;
            }
            let Some(itm) = world.items.get(&num) else {
                tracing::error!(item = %num, "find_item: Item not found");
                continue;
            };
            if !matches_any_word(&cur, &itm.name) {
                continue;
            }
            if !passes_filter(plr, itm, i, flags) {
                continue;
            }
            if itm.number == lastnum {
                continue;
            }
            matched = true;
            f.count += 1;
            if f.count == 1 {
                lastnum = itm.number;
                f.slot = Some(i);
                f.charges = plr.itemuses.get(i).copied().unwrap_or(0);
            }
            f.options.push(itm.name.clone());
            if eq_ignore_case(&cur, &itm.name) {
                lastnum = itm.number;
                f.slot = Some(i);
                f.charges = plr.itemuses.get(i).copied().unwrap_or(0);
                exact = true;
            }
        }
        let mut at_start = false;
        if !matched {
            let start = last_word_start(&cur);
            if start > 0 {
                cur.truncate(start - 1);
                ndropped += 1;
            } else {
                at_start = true;
            }
        }
        if matched || at_start || passes >= MAXPASSES {
            break;
        }
    }
    if exact || f.count == 1 {
        f.words = count_words(&cur);
    }
    let invdropped_at_start = ndropped;
    // Step 2, the key ring, from the full input again
    if !exact {
        let mut invdropped = invdropped_at_start;
        let mut cur = name.to_string();
        let mut ndropped = 0usize;
        let mut matched = false;
        let mut passes = 0;
        let nkeys = usize::from(plr.nkeyslots).min(NKEYS);
        loop {
            passes += 1;
            for (i, &num) in plr.keys.iter().enumerate().take(nkeys) {
                if exact {
                    break;
                }
                if num == ItemNo(0) {
                    continue;
                }
                let Some(itm) = world.items.get(&num) else {
                    continue;
                };
                if !matches_any_word(&cur, &itm.name) || itm.number == lastnum {
                    continue;
                }
                let uses = plr.keyuses.get(i).copied().unwrap_or(0);
                matched = true;
                f.count += 1;
                if f.count == 1 {
                    lastnum = itm.number;
                    f.charges = uses;
                }
                // fewer words cut than the inventory match needed: the key wins
                if ndropped < invdropped {
                    invdropped = 0;
                    f.count = 1;
                    lastnum = itm.number;
                    f.charges = uses;
                    f.options.clear();
                }
                f.options.push(itm.name.clone());
                if eq_ignore_case(&cur, &itm.name) {
                    lastnum = itm.number;
                    f.charges = uses;
                    exact = true;
                }
            }
            let mut at_start = false;
            if !matched {
                if ndropped == invdropped {
                    matched = true;
                } else {
                    let start = last_word_start(&cur);
                    if start > 0 {
                        cur.truncate(start - 1);
                        ndropped += 1;
                    } else {
                        at_start = true;
                    }
                }
            }
            if matched || at_start || passes >= MAXPASSES {
                break;
            }
        }
        if exact || f.count == 1 {
            f.words = count_words(&cur);
        }
    }
    // Step 3: the record chosen; an exact match is never ambiguous
    if lastnum != ItemNo(0) && world.items.contains_key(&lastnum) {
        f.item = Some(lastnum);
        if exact && f.count != 0 {
            f.count = 0;
        }
    }
    f
}

/// The `SRCH_` filters.
fn passes_filter(plr: &Character, itm: &Item, slot: usize, flags: Search) -> bool {
    let in_use =
        || plr.worn.contains(&itm.number) || plr.readied == slot as i8 || itm.number == plr.weapon;
    match flags {
        Search::Any => true,
        Search::Food => itm.item_type == ItemType::Food,
        Search::Drink => itm.item_type == ItemType::Drink,
        Search::Light => itm.item_type == ItemType::Light,
        Search::Unworn => !in_use(),
        Search::Worn => in_use(),
    }
}

/// The ambiguity list sent to the player.
pub fn show_options(world: &mut World, id: SessionId, options: &[String]) {
    tell_user(world, id, Msg::Ambiguous);
    for name in options {
        tell_user(world, id, Msg::AmbiguousItem { name: name.clone() });
    }
}

/// A worn slot holds `item`.
pub fn user_is_wearing(plr: &Character, item: ItemNo) -> bool {
    plr.worn.iter().take(NWORN).any(|&w| w == item)
}

/// The total of an ability over the character's sources.
pub fn ability_value(world: &World, plr: &Character, ability: Ability) -> i32 {
    let spells = |n| world.spells.get(&n);
    let items = |n| world.items.get(&n);
    CharacterAbilities {
        character: plr,
        race: world.races.get(&plr.race),
        class: world.classes.get(&plr.class),
        spells: &spells,
        items: &items,
    }
    .value(ability)
}

/// Whether any source gives the ability.
pub fn has_ability(world: &World, plr: &Character, ability: Ability) -> bool {
    let spells = |n| world.spells.get(&n);
    let items = |n| world.items.get(&n);
    CharacterAbilities {
        character: plr,
        race: world.races.get(&plr.race),
        class: world.classes.get(&plr.class),
        spells: &spells,
        items: &items,
    }
    .has(ability)
}

/// The record's `maxwgt` (zero-extended) raised or lowered by the "Alter Encumbrance"
/// percentage; never 0.
pub fn max_weight(world: &World, plr: &Character) -> i32 {
    let pct = ability_value(world, plr, Ability::Encumbrance);
    let mut maxw = i32::from(plr.maxwgt as u16);
    if pct != 0 {
        maxw = maxw * (pct + 100) / 100;
    }
    if maxw != 0 { maxw } else { 1 }
}

/// Items plus coins as a
/// percentage of [`max_weight`], stored in `encumpct` and returned.
pub fn update_encumbrance(world: &mut World, id: SessionId) -> i16 {
    let Some(pct) = world
        .sessions
        .get(&id)
        .and_then(|s| s.player())
        .map(|plr| encumbrance_pct(plr.wgt, plr.coins, max_weight(world, plr)))
    else {
        return 100;
    };
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.encumpct = pct;
    }
    pct
}

/// The load the inventory shows: carried weight plus coin weight.
pub fn load(plr: &Character) -> i64 {
    i64::from(plr.wgt) + i64::from(coin_weight(plr.coins) as u16)
}

/// Puts a copy of `item` with `charges` uses (-2: a new copy, the item's own `charges`)
/// into the first free key slot (a key) or inventory slot. A limited item must have a copy
/// free (`A strange force stops you ...`); the item, the load and the coins must not weigh
/// more than the character can carry, and a slot must be free (`You cannot carry that
/// much!`), else the copy is released again. The refusal goes to `id`'s output. `true` when
/// stored.
pub fn add_item_to_inventory(world: &mut World, id: SessionId, item: ItemNo, charges: i16) -> bool {
    let Some(itm) = world.items.get(&item).cloned() else {
        return false;
    };
    let Some(userid) = world
        .sessions
        .get(&id)
        .and_then(|s| s.player())
        .map(|p| p.userid.clone())
    else {
        return false;
    };
    // A limited item must still have a copy free
    if !limited::acquire(world, item, Holder::User(&userid)) {
        tell_user(world, id, Msg::LimitedRefused);
        return false;
    }
    // The item, what he carries and his coins against what he can carry
    let too_heavy = {
        let plr = world.sessions.get(&id).and_then(|s| s.player());
        plr.is_some_and(|plr| {
            i32::from(itm.weight) + i32::from(plr.wgt) + coin_weight(plr.coins)
                > max_weight(world, plr)
        })
    };
    if too_heavy {
        tell_user(world, id, Msg::CarryTooMuch);
        limited::release(world, item, Holder::User(&userid));
        return false;
    }
    let found = world
        .session_mut(id)
        .and_then(|s| s.character.as_mut())
        .is_some_and(|plr| {
            let uses = if charges == -2 { itm.charges } else { charges };
            let stored = if itm.item_type == ItemType::Key {
                // The first empty key slot (never more than NKEYS)
                if usize::from(plr.nkeyslots) > NKEYS {
                    plr.nkeyslots = NKEYS as u8;
                }
                let n = usize::from(plr.nkeyslots);
                match plr.keys.iter().take(n).position(|&k| k == ItemNo(0)) {
                    Some(i) => {
                        plr.keys[i] = item;
                        plr.keyuses[i] = uses;
                        true
                    }
                    None => false,
                }
            } else {
                // The first empty inventory slot
                match plr
                    .items
                    .iter()
                    .take(NINVITEMS)
                    .position(|&k| k == ItemNo(0))
                {
                    Some(i) => {
                        plr.items[i] = item;
                        plr.itemuses[i] = uses;
                        true
                    }
                    None => false,
                }
            };
            if stored {
                plr.wgt = plr.wgt.wrapping_add(itm.weight as u16);
            }
            stored
        });
    if !found {
        // No free slot: give the copy back and say why
        limited::release(world, item, Holder::User(&userid));
        tell_user(world, id, Msg::CarryTooMuch);
    }
    found
}

/// For an online character: takes the first copy of `item` out of the inventory slots, else
/// out of the key slots. The readied light going out loses its light; a wielded or worn
/// item with no other copy left is taken off with its abilities. The weight goes down and a
/// limited copy is released. Returns the copy's uses, `None` when no copy was found.
pub fn remove_item_from_inventory(world: &mut World, id: SessionId, item: ItemNo) -> Option<i16> {
    let mut plr = world.session_mut(id)?.character.take()?;
    let charges = remove_item_from_record(world, &mut plr, item);
    if let Some(s) = world.session_mut(id) {
        s.character = Some(plr);
    }
    charges
}

/// [`remove_item_from_inventory`] on a record that is not in a session, with no player:
/// the midnight cleanup and the offline user deletion use it on stored records. The
/// abilities of a wielded or worn item are taken off the record itself
/// ([`super::equip::unequip_record`]).
pub fn remove_item_from_record(
    world: &mut World,
    plr: &mut Character,
    item: ItemNo,
) -> Option<i16> {
    let itm = world.items.get(&item).cloned()?;
    let genlight = item_ability_value(&itm, Ability::GenLight);
    let mut unequip = 0usize;
    let mut charges = None;
    if let Some(i) = plr.items.iter().take(NINVITEMS).position(|&n| n == item) {
        if itm.item_type == ItemType::Light && plr.readied == i as i8 {
            plr.readied = -1;
            plr.light = plr.light.wrapping_sub(genlight as i16);
        }
        charges = Some(plr.itemuses[i]);
        plr.wgt = plr.wgt.wrapping_sub(itm.weight as u16);
        plr.items[i] = ItemNo(0);
        plr.itemuses[i] = -1;
        let stillhave = plr.items.contains(&item);
        if plr.weapon == item && !stillhave {
            unequip += 1;
            plr.weapon = ItemNo(0);
        }
        for j in 0..NWORN {
            if plr.worn[j] == item && !stillhave {
                unequip += 1;
                plr.worn[j] = ItemNo(0);
                plr.wornuses[j] = -1;
            }
        }
    } else {
        // The key slots
        if usize::from(plr.nkeyslots) > NKEYS {
            plr.nkeyslots = NKEYS as u8;
        }
        let n = usize::from(plr.nkeyslots);
        if let Some(i) = plr.keys.iter().take(n).position(|&k| k == item) {
            charges = Some(plr.keyuses[i]);
            plr.wgt = plr.wgt.wrapping_sub(itm.weight as u16);
            plr.keys[i] = ItemNo(0);
            plr.keyuses[i] = -2;
        }
    }
    let charges = charges?;
    for _ in 0..unequip {
        equip::unequip_record(plr, &itm);
    }
    let userid = plr.userid.clone();
    limited::release(world, item, Holder::User(&userid));
    Some(charges)
}

/// What [`deduct_item_charge`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    /// The item stays: `uses` left (or unlimited, or kept when empty).
    Left(i16),
    /// The last use went and the item is gone.
    Gone,
}

/// Uses one charge of the carried copy of `item` that has `charges` uses, inventory slots
/// first, then the key ring. -1 is unlimited. At 0 an item that is not kept when empty
/// (`retain`) goes: its weight, slot, wear and light are taken off, its limited copy
/// released, and the player is told with the item's destroy message (the room line too when
/// it has one) or `It's uses gone`.
pub fn deduct_item_charge(world: &mut World, id: SessionId, item: ItemNo, charges: i16) -> Charge {
    if charges == -1 {
        return Charge::Left(-1);
    }
    let Some(itm) = world.items.get(&item).cloned() else {
        return Charge::Gone;
    };
    let genlight = item_ability_value(&itm, Ability::GenLight) as i16;
    let mut unequip = 0usize;
    let (left, gone, userid, first, room) = {
        let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
            return Charge::Gone;
        };
        let room = RoomKey {
            map: plr.map,
            room: plr.room,
        };
        if let Some(i) =
            (0..NINVITEMS).find(|&i| plr.items[i] == item && plr.itemuses[i] == charges)
        {
            plr.itemuses[i] -= 1;
            let left = plr.itemuses[i];
            let gone = left == 0 && itm.retain == 0;
            if gone {
                plr.wgt = plr.wgt.wrapping_sub(itm.weight as u16);
                plr.items[i] = ItemNo(0);
                if plr.weapon == item {
                    unequip += 1;
                    plr.weapon = ItemNo(0);
                }
                for j in 0..NWORN {
                    if plr.worn[j] == item {
                        unequip += 1;
                        plr.worn[j] = ItemNo(0);
                    }
                }
                if plr.readied == i as i8 {
                    plr.readied = -1;
                    plr.light = plr.light.wrapping_sub(genlight);
                }
            }
            (left, gone, plr.userid.clone(), plr.firstname.clone(), room)
        } else if let Some(i) = (0..NKEYS.min(plr.keys.len()))
            .find(|&i| plr.keys[i] == item && plr.keyuses[i] == charges)
        {
            plr.keyuses[i] -= 1;
            let left = plr.keyuses[i];
            let gone = left == 0 && itm.retain == 0;
            if gone {
                plr.wgt = plr.wgt.wrapping_sub(itm.weight as u16);
                plr.keys[i] = ItemNo(0);
                for j in 0..NWORN {
                    if plr.worn[j] == item {
                        unequip += 1;
                        plr.worn[j] = ItemNo(0);
                    }
                }
            }
            (left, gone, plr.userid.clone(), plr.firstname.clone(), room)
        } else {
            // no carried copy matched
            return Charge::Gone;
        }
    };
    if !gone {
        return Charge::Left(left);
    }
    for _ in 0..unequip {
        equip::unequip_apply_item_abilities(world, id, &itm);
    }
    limited::release(world, item, Holder::User(&userid));
    item_destroyed(world, id, &itm, &first, room);
    Charge::Gone
}

/// The going of a used-up item: its destroy message (`msgs[0]` with
/// the item name to the player, `msgs[1]` with the player's and the item's name to the
/// room) or the standard line.
fn item_destroyed(world: &mut World, id: SessionId, itm: &Item, first: &str, room: RoomKey) {
    if itm.destroymsg.0 == 0 {
        tell_user(
            world,
            id,
            Msg::UsesGone {
                item: itm.name.clone(),
            },
        );
        return;
    }
    let Some(msg) = world.messages.get(&itm.destroymsg).cloned() else {
        return;
    };
    tell_user(
        world,
        id,
        Msg::ItemDestroyed {
            text: printf_s(&msg.msgs[0], &[&itm.name]),
        },
    );
    if !msg.msgs[1].is_empty() {
        let line = Msg::ItemDestroyed {
            text: printf_s(&msg.msgs[1], &[first, &itm.name]),
        };
        tell_room(
            world,
            room,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &line,
        );
    }
}
