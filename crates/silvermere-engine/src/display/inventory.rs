//! INVENTORY, KEYS and WEALTH: [`display_inventory_items`], [`display_users_keys`] and
//! [`display_user_money`].

use silvermere_rules::economy::{EncumbranceLevel, to_copper};
use silvermere_text::{Msg, ansi};
use silvermere_types::enums::{Currency, ItemType, WeaponType};
use silvermere_types::ids::{ItemNo, SessionId};
use silvermere_types::records::Character;

use super::room_items::coin_words;
use crate::items::inventory::{load, max_weight, update_encumbrance};
use crate::world::World;

/// The copper coin's name, the unit of the `Wealth:` line.
fn copper_name() -> String {
    Currency::Copper
        .plural_name()
        .unwrap_or_default()
        .to_owned()
}

/// `ANSI_*` as text, for the colour arguments.
fn ansi_str(a: &'static [u8]) -> &'static str {
    std::str::from_utf8(a).expect("ANSI strings are ASCII")
}

/// The words after `You are carrying `, or `None` for `Nothing!`.
///
/// The tally keeps one entry per distinct item number in the order of its first slot (the
/// search stops at the first free entry). Each worn item is named with its worn-location
/// name and taken off its count; then the readied item with its uses divided by 10, then
/// the weapon (`Two handed` for 2-handed blunt and sharp weapons, else `Weapon Hand`), each
/// only while its count is above 0; then the rest, with the count in front of duplicates.
pub fn carried_words(world: &World, plr: &Character) -> Option<String> {
    let mut words = coin_words(world, &plr.coins);
    let mut tally: Vec<(ItemNo, i32)> = Vec::new();
    for &n in plr.items.iter().filter(|n| n.0 != 0) {
        match tally.iter_mut().find(|(t, _)| *t == n) {
            Some(e) => e.1 += 1,
            None => tally.push((n, 1)),
        }
    }
    let take = |tally: &mut Vec<(ItemNo, i32)>, n: ItemNo| {
        for e in tally.iter_mut().filter(|(t, _)| *t == n) {
            e.1 -= 1;
        }
    };
    for &w in plr.worn.iter().filter(|w| w.0 != 0) {
        if let Some(itm) = world.items.get(&w) {
            words.push(format!(
                "{} ({})",
                itm.name,
                itm.worn.name().unwrap_or_default()
            ));
            take(&mut tally, w);
        }
    }
    if plr.readied != -1
        && let Some(&slot_item) = usize::try_from(plr.readied)
            .ok()
            .and_then(|i| plr.items.get(i))
        && tally
            .iter()
            .any(|&(t, c)| t.0 != 0 && t == slot_item && c > 0)
        && let Some(itm) = world.items.get(&slot_item)
    {
        let uses = plr.itemuses.get(plr.readied as usize).copied().unwrap_or(0);
        words.push(format!("{} (Readied/{})", itm.name, uses / 10));
        take(&mut tally, slot_item);
    }
    if plr.weapon.0 != 0
        && tally.iter().any(|&(t, c)| t == plr.weapon && c > 0)
        && let Some(itm) = world.items.get(&plr.weapon)
    {
        let two = itm.item_type == ItemType::Weapon
            && matches!(
                itm.wpntype,
                WeaponType::TwoHandBlunt | WeaponType::TwoHandSharp
            );
        words.push(format!(
            "{} ({})",
            itm.name,
            if two { "Two handed" } else { "Weapon Hand" }
        ));
        take(&mut tally, plr.weapon);
    }
    for (n, c) in tally {
        if c <= 0 {
            continue;
        }
        if let Some(itm) = world.items.get(&n) {
            words.push(if c > 1 {
                format!("{c} {}", itm.name)
            } else {
                itm.name.clone()
            });
        }
    }
    (!words.is_empty()).then(|| words.join(", "))
}

/// The key list of [`display_users_keys`]: one entry per kind, a new kind in front, ` name`
/// or ` count names` (sic: an `s` whatever the name), `,` between. Keys whose item record
/// is missing are skipped. `None` for no keys.
pub fn key_words(world: &World, plr: &Character) -> Option<String> {
    let mut list: Vec<(ItemNo, &str, u32)> = Vec::new();
    for &k in plr.keys.iter().filter(|k| k.0 != 0) {
        let Some(itm) = world.items.get(&k) else {
            continue;
        };
        match list.iter_mut().find(|(n, _, _)| *n == k) {
            Some(e) => e.2 += 1,
            None => list.insert(0, (k, itm.name.as_str(), 1)),
        }
    }
    if list.is_empty() {
        return None;
    }
    let parts: Vec<String> = list
        .iter()
        .map(|&(_, name, count)| {
            if count > 1 {
                format!(" {count} {name}s")
            } else {
                format!(" {name}")
            }
        })
        .collect();
    Some(parts.join(","))
}

/// The keys message: inside the wrapped inventory display, or alone.
fn keys_msg(world: &World, plr: &Character, wrapped: bool) -> Msg {
    match key_words(world, plr) {
        Some(keys) if wrapped => Msg::InvKeysList { keys },
        Some(keys) => Msg::KeysList { keys },
        None => Msg::KeysNone,
    }
}

/// The carried line, the keys, `Wealth:` and `Encumbrance:`, for the character on `shown`,
/// sent to `to`.
pub fn display_inventory_items(world: &mut World, shown: SessionId, to: SessionId) {
    if world.session(shown).and_then(|s| s.player()).is_none() {
        return;
    }
    // The encumbrance percentage, which is also stored in `encumpct` for combat, the medium
    // cycle and movement
    let pct = update_encumbrance(world, shown);
    let Some(plr) = world.session(shown).and_then(|s| s.player()) else {
        return;
    };
    let mut msgs = Vec::new();
    msgs.push(match carried_words(world, plr) {
        Some(items) => Msg::InvCarrying { items },
        None => Msg::InvCarryingNothing,
    });
    msgs.push(keys_msg(world, plr, true));
    msgs.push(Msg::InvWealth {
        copper: to_copper(plr.coins),
        name: copper_name(),
    });
    // The items' weight plus the coins' (16-bit) over the maximum weight printed as an
    // `int`
    let max = max_weight(world, plr);
    let level = EncumbranceLevel::from_pct(pct);
    let colour = match level {
        EncumbranceLevel::Heavy => ansi::ANSI_FGRED,
        EncumbranceLevel::Medium => ansi::ANSI_FGBROWN,
        EncumbranceLevel::Light => ansi::ANSI_FGGREEN,
        EncumbranceLevel::None => ansi::ANSI_FGGREY,
    };
    msgs.push(Msg::InvEncumbrance {
        cur: load(plr).to_string(),
        max: (max as i16).to_string(),
        colour: ansi_str(colour).to_owned(),
        level: level.name().to_owned(),
        pct: i32::from(pct),
    });
    if let Some(s) = world.session_mut(to) {
        for m in msgs {
            s.output.push(m);
        }
    }
}

/// The KEYS command.
pub fn display_users_keys(world: &mut World, id: SessionId) {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    let msg = keys_msg(world, plr, false);
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// The WEALTH command: `You have` and the coins, then the total in copper, or
/// `You have No money!`. Returns `false` for no character.
pub fn display_user_money(world: &mut World, id: SessionId) -> bool {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return false;
    };
    let words = coin_words(world, &plr.coins);
    let mut msgs = Vec::new();
    if words.is_empty() {
        msgs.push(Msg::WealthNone);
    } else {
        msgs.push(Msg::WealthList {
            coins: words.join(", "),
        });
        msgs.push(Msg::InvWealth {
            copper: to_copper(plr.coins),
            name: copper_name(),
        });
    }
    if let Some(s) = world.session_mut(id) {
        for m in msgs {
            s.output.push(m);
        }
    }
    true
}
