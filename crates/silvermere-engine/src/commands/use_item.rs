//! USE, READ, EAT, DRINK and OPEN {item} (T-053): [`cmd_use`], [`cmd_eat`], [`cmd_drink`]
//! and the item branch of OPEN. What the item does is [`crate::items::use_item`]; LIGHT is T-030's
//! [`super::inventory::cmd_light`], which USE hands a light source to.

use silvermere_rules::Rng as _;
use silvermere_text::Msg;
use silvermere_types::enums::{ItemType, WornLocation};
use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;
use silvermere_types::sizes::NWORN;

use super::cast::{ANYTHING, Found as Target, find_action_target};
use super::dispatch::Handled;
use super::parse::parse_command;
use super::{Cmd, CommandLine};
use crate::combat;
use crate::items::equip::{tell_others, user_can_use};
use crate::items::inventory::{Search, deduct_item_charge, find_item_in_inventory, show_options};
use crate::items::use_item::{UseTarget, use_item};
use crate::movement::direction;
use crate::output::clear_output;
use crate::tell::tell_user;
use crate::world::World;

fn player(world: &World, id: SessionId) -> Option<&Character> {
    world.session(id).and_then(|s| s.player())
}

/// Ends resting and meditating.
fn stop_resting(world: &mut World, id: SessionId) {
    if let Some(p) = combat::player_mut(world, id) {
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
}

/// USE and READ. The rest of the line names a carried item (several fits: the list). It must have
/// uses left, the user must be allowed it (`user_can_use`), a weapon must be the one wielded and an
/// item with a wear location must be worn. With nothing after its name a light source is lit
/// (LIGHT) and anything else is used on nothing; a key takes the next word as a direction; any
/// other item is used on what the rest of the line names (a player, a monster, an item, a spell;
/// nothing found is nothing). A use that did something takes a charge. With no such item READ looks
/// instead, and USE offers the line to the room's command text before `You don't have ...`.
pub fn cmd_use(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    // the word alone
    if line.argc() == 1 {
        let msg = if cmd == Cmd::Read {
            Msg::ReadSyntax
        } else {
            Msg::UseSyntax
        };
        tell_user(world, id, msg);
        return Handled::Done;
    }
    // the carried item the rest of the line names
    let name = line.rest_from(1);
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
    let itm = f.item.and_then(|n| world.items.get(&n)).cloned();
    let Some(itm) = itm else {
        if cmd == Cmd::Read {
            // READ looks at the thing instead
            stop_resting(world, id);
            return super::info::cmd_look(world, id, cmd, line);
        }
        // the room's command text, else the refusal
        if crate::scripts::room_command(world, id, &line.raw) {
            return Handled::Done;
        }
        tell_user(
            world,
            id,
            Msg::DontHave {
                item: name.to_owned(),
            },
        );
        return Handled::Done;
    };
    if f.charges == 0 {
        // no uses left
        tell_user(world, id, Msg::NoUses { item: itm.name });
        return Handled::Done;
    }
    // may the user use it, wielded or worn as it must be
    if !user_can_use(world, plr, &itm) {
        tell_user(world, id, Msg::UseForbidden);
        return Handled::Done;
    }
    if itm.worn == WornLocation::Nowhere {
        if itm.item_type == ItemType::Weapon && itm.number != plr.weapon {
            tell_user(world, id, Msg::UseMustReady);
            return Handled::Done;
        }
    } else if !plr.worn[..NWORN].contains(&itm.number) {
        tell_user(world, id, Msg::UseMustWear);
        return Handled::Done;
    }
    let light = |world: &mut World| {
        stop_resting(world, id);
        super::inventory::cmd_light(world, id, cmd, line)
    };
    // the use, on what the rest of the line names
    let used = if line.argc() == f.words + 1 {
        if itm.item_type == ItemType::Light {
            return light(world);
        }
        use_item(world, id, itm.number, UseTarget::None)
    } else if itm.item_type == ItemType::Key {
        // the next word as a direction (NORTH .. DOWN)
        if let Some(p) = combat::player_mut(world, id) {
            p.status.remove(PlayerStatus::RESTING);
        }
        match parse_command(line.arg(f.words + 1)).and_then(|c| direction(c.code() as i16)) {
            Some(dir) => use_item(world, id, itm.number, UseTarget::Direction(dir)),
            None => 0,
        }
    } else {
        let target = find_action_target(world, id, line.rest_from(f.words + 1), ANYTHING);
        if let Target::Multiple(_, options) = &target {
            show_options(world, id, options);
            return Handled::Done;
        }
        if combat::debug_on(world, id) {
            tracing::debug!(target = ?target, "Target type");
        }
        let target = match target {
            Target::User(t) => UseTarget::Player(t),
            Target::Monster(m) => UseTarget::Monster(m),
            Target::Inventory(n) | Target::Floor(n) => UseTarget::Item(n),
            Target::Spell(s) => UseTarget::Spell(s),
            Target::None | Target::Multiple(..) => {
                if itm.item_type == ItemType::Light {
                    return light(world);
                }
                UseTarget::None
            }
        };
        use_item(world, id, itm.number, target)
    };
    // a use that did something takes a charge
    if used != 0 {
        stop_resting(world, id);
        deduct_item_charge(world, id, itm.number, f.charges);
    }
    Handled::Done
}

/// EAT and DRINK: a carried food (drink) the user may use, with a use left. The user and, unless
/// hidden, the room are told; then the item's use effects run (`use_no_target`): when they did
/// something the item loses a use, when not the food (drink) level goes up by one. Every refusal is
/// silent and not handled, except DRINK of an empty drink (`There is nothing more that ...`).
fn consume(world: &mut World, id: SessionId, line: &CommandLine, drink: bool) -> Handled {
    if player(world, id).is_none() {
        return Handled::NotCommand;
    }
    stop_resting(world, id);
    if line.argc() == 1 {
        return Handled::NotCommand;
    }
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let search = if drink { Search::Drink } else { Search::Food };
    let f = find_item_in_inventory(world, plr, line.rest_from(1), search);
    if f.ambiguous() {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the colour reset is dropped (EAT and DRINK alike)
    clear_output(world, id);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() else {
        return Handled::NotCommand;
    };
    let kind = if drink {
        ItemType::Drink
    } else {
        ItemType::Food
    };
    if !user_can_use(world, plr, &itm) || itm.item_type != kind {
        return Handled::NotCommand;
    }
    if f.charges == 0 {
        if !drink {
            return Handled::NotCommand;
        }
        tell_user(world, id, Msg::ItemNothingMore { item: itm.name });
        return Handled::Done;
    }
    let who = plr.firstname.clone();
    let hidden = plr.hidden != 0;
    let room = combat::room_of(plr);
    let (mine, theirs) = if drink {
        (
            Msg::DrinkItem {
                item: itm.name.clone(),
            },
            Msg::OtherDrinks {
                who,
                item: itm.name.clone(),
            },
        )
    } else {
        (
            Msg::EatItem {
                item: itm.name.clone(),
            },
            Msg::OtherEats {
                who,
                item: itm.name.clone(),
            },
        )
    };
    tell_user(world, id, mine);
    if !hidden {
        tell_others(world, id, room, &theirs);
    }
    if use_item(world, id, itm.number, UseTarget::None) != 0 {
        deduct_item_charge(world, id, itm.number, f.charges);
    } else if let Some(p) = combat::player_mut(world, id) {
        if drink {
            p.thirst = p.thirst.wrapping_add(1);
        } else {
            p.hunger = p.hunger.wrapping_add(1);
        }
    }
    Handled::Done
}

/// EAT {food}.
pub fn cmd_eat(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    consume(world, id, line, false)
}

/// DRINK {drink}.
pub fn cmd_drink(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    consume(world, id, line, true)
}

/// OPEN {item}, when the word after OPEN is no direction: a carried container the user may use.
/// With a charge left it is used (`use_no_target`) and, if that did something, each of its five
/// coin maxima (`item.cash`) is rolled from 0 up and added to the purse and it loses a charge; with
/// none left `There is nothing more that ...`. Nothing by that name: the syntax line. Not usable or
/// not a container: not handled.
pub fn open_item(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let f = find_item_in_inventory(world, plr, line.rest_from(1), Search::Any);
    if f.ambiguous() {
        show_options(world, id, &f.options);
        return Handled::Done;
    }
    // the match lookup ends: the colour reset is dropped
    clear_output(world, id);
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() else {
        tell_user(world, id, Msg::OpenSyntax);
        return Handled::Done;
    };
    if !user_can_use(world, plr, &itm) || itm.item_type != ItemType::Container {
        return Handled::NotCommand;
    }
    if f.charges == 0 {
        tell_user(world, id, Msg::ItemNothingMore { item: itm.name });
        return Handled::Done;
    }
    if use_item(world, id, itm.number, UseTarget::None) != 0 {
        // runic, platinum, gold, silver, copper, each rolled from 0 to its maximum
        let c = itm.cash;
        let mut roll = |max: u32| {
            if max == 0 {
                0
            } else {
                world.rng.long(0, i64::from(max)) as u32
            }
        };
        let found = [
            roll(c.runic),
            roll(c.platinum),
            roll(c.gold),
            roll(c.silver),
            roll(c.copper),
        ];
        if let Some(p) = combat::player_mut(world, id) {
            p.coins.runic = p.coins.runic.wrapping_add(found[0]);
            p.coins.platinum = p.coins.platinum.wrapping_add(found[1]);
            p.coins.gold = p.coins.gold.wrapping_add(found[2]);
            p.coins.silver = p.coins.silver.wrapping_add(found[3]);
            p.coins.copper = p.coins.copper.wrapping_add(found[4]);
        }
        deduct_item_charge(world, id, itm.number, f.charges);
    }
    Handled::Done
}
