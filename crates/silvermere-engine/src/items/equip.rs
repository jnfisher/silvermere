//! Wearing and wielding: [`wear_armour`], [`remove_armour`], [`ready_weapon`], and the item
//! abilities that apply only while an item is on: [`equip_apply_item_abilities`] and
//! [`unequip_apply_item_abilities`]. Every change re-runs the secondary stats in mode 2
//! ([`recalc`], which first rebuilds the dynamic stats).
//!
//! Changing equipment breaks off autocombat first ([`break_off_autocombat`], also REST's
//! and MEDITATE's): `*Combat Off*` and the prompt come before the change's own lines.

use silvermere_rules::abilities::{Ability, item_has_ability};
use silvermere_rules::combat::is_weapon_too_fast_for_user;
use silvermere_text::Msg;
use silvermere_types::enums::{ItemType, WeaponType, WornLocation};
use silvermere_types::ids::{ItemNo, RoomKey, SessionId, SpellNo};
use silvermere_types::records::{Character, Item};
use silvermere_types::sizes::{NSPELLBOOK, NWORN};

use super::can_use::{UseCheck, can_use};
use super::inventory::{Found, Search, find_item_in_inventory, has_ability};
use crate::combat::autocombat::{display_autocombat_broken, is_inside_autocombat, kill_autocombat};
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// The secondary stats in mode 2: the derived stats again, keeping the current hit
/// points and mana.
pub fn recalc(world: &mut World, id: SessionId) {
    let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) else {
        return;
    };
    crate::player::recalculate(world, &mut c, crate::player::CalcMode::KeepCurrent);
    if let Some(s) = world.session_mut(id) {
        s.character = Some(c);
    }
}

/// May the character use the item (`user_can_use`) with the world's records?
pub fn user_can_use(world: &World, plr: &Character, item: &Item) -> bool {
    let spells = |n| world.spells.get(&n);
    let check = UseCheck {
        class: world.classes.get(&plr.class),
        anti_magic: has_ability(world, plr, Ability::AntiMagic),
        spells: &spells,
    };
    can_use(plr, item, &check).is_ok()
}

fn is_cursed(itm: &Item) -> bool {
    item_has_ability(itm, Ability::Cursed) || item_has_ability(itm, Ability::MajorCurse)
}

fn is_two_handed(itm: &Item) -> bool {
    matches!(
        itm.wpntype,
        WeaponType::TwoHandBlunt | WeaponType::TwoHandSharp
    )
}

fn room_of(plr: &Character) -> RoomKey {
    RoomKey {
        map: plr.map,
        room: plr.room,
    }
}

/// The room but the player, always heard, the blind too, with the prompt and no colour:
/// the form every equipment line uses.
pub(crate) fn tell_others(world: &mut World, id: SessionId, room: RoomKey, msg: &Msg) {
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        msg,
    );
}

/// A player in auto-combat is told `*Combat Off*` with the
/// prompt (and with `announce`, the room that they break off combat); the slots are freed
/// either way and energy above one round is cut.
pub(crate) fn break_off_autocombat(world: &mut World, id: SessionId, announce: bool) {
    if is_inside_autocombat(world, id) {
        display_autocombat_broken(world, id, announce);
    }
    kill_autocombat(world, id);
}

/// Give Spell writes the spell into
/// the spellbook and Alter HP raises the maximum and current hit points.
pub fn equip_apply_item_abilities(world: &mut World, id: SessionId, itm: &Item) {
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return;
    };
    for &(ability, value) in &itm.abilities {
        match ability {
            Ability::GiveSpell => {
                add_spell_to_spellbook(plr, SpellNo(value));
            }
            Ability::AlterHp => {
                plr.maxhp = plr.maxhp.wrapping_add(value);
                plr.hp = plr.hp.wrapping_add(value);
            }
            _ => {}
        }
    }
}

/// The reverse of
/// [`equip_apply_item_abilities`].
pub fn unequip_apply_item_abilities(world: &mut World, id: SessionId, itm: &Item) {
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        unequip_record(plr, itm);
    }
}

/// [`unequip_apply_item_abilities`] on the record itself, with no player in a session.
pub fn unequip_record(plr: &mut Character, itm: &Item) {
    for &(ability, value) in &itm.abilities {
        match ability {
            Ability::GiveSpell => purge_spell_from_spellbook(plr, SpellNo(value)),
            Ability::AlterHp => {
                plr.maxhp = plr.maxhp.wrapping_sub(value);
                plr.hp = plr.hp.wrapping_sub(value);
            }
            _ => {}
        }
    }
}

/// Into the first free of the first
/// `nspellslots` slots, unless the spell is known already.
pub fn add_spell_to_spellbook(plr: &mut Character, spell: SpellNo) -> bool {
    if usize::from(plr.nspellslots) > NSPELLBOOK {
        plr.nspellslots = NSPELLBOOK as u8;
    }
    let n = usize::from(plr.nspellslots).min(plr.spellbook.len());
    if plr.spellbook[..n].contains(&spell) {
        return false;
    }
    match plr.spellbook[..n].iter().position(|&s| s == SpellNo(0)) {
        Some(i) => {
            plr.spellbook[i] = spell;
            true
        }
        None => false,
    }
}

/// Clears every slot holding the spell.
pub fn purge_spell_from_spellbook(plr: &mut Character, spell: SpellNo) {
    for s in plr.spellbook.iter_mut().take(NSPELLBOOK) {
        if *s == spell {
            *s = SpellNo(0);
        }
    }
}

/// Takes the worn item in slot `i` off with its abilities, tells the room and the player
/// (the swap lines of [`wear_armour`]).
fn take_off_in_the_way(world: &mut World, id: SessionId, i: usize, witm: &Item) {
    unequip_apply_item_abilities(world, id, witm);
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return;
    };
    plr.worn[i] = ItemNo(0);
    plr.wornuses[i] = 0;
    let who = plr.firstname.clone();
    let room = room_of(plr);
    tell_others(
        world,
        id,
        room,
        &Msg::OtherRemovesBang {
            who,
            item: witm.name.clone(),
        },
    );
    tell_user(
        world,
        id,
        Msg::Removed {
            item: witm.name.clone(),
        },
    );
}

/// Puts on `item` (from EQUIP, which has found it
/// with `charges` uses), or the unworn carried item `name` names. Returns whether it was
/// put on; `true` also when there is no character.
pub fn wear_armour(
    world: &mut World,
    id: SessionId,
    name: &str,
    item: Option<(ItemNo, i16)>,
) -> bool {
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
        return true;
    };
    // A free worn slot first
    let Some(freeslot) = plr.worn.iter().take(NWORN).position(|&w| w == ItemNo(0)) else {
        tell_user(world, id, Msg::WearNoRoom);
        return false;
    };
    let found = match item {
        Some(pair) => Some(pair),
        None => {
            let f: Found = find_item_in_inventory(world, plr, name, Search::Unworn);
            f.item.map(|n| (n, f.charges))
        }
    };
    let mut worn = false;
    'done: {
        let Some((itemno, charges)) = found else {
            tell_user(
                world,
                id,
                Msg::WearNotHaveAny {
                    name: name.to_string(),
                },
            );
            break 'done;
        };
        let Some(itm) = world.items.get(&itemno).cloned() else {
            break 'done;
        };
        let loc = itm.worn;
        if loc == WornLocation::Nowhere {
            tell_user(world, id, Msg::NotWearable { item: itm.name });
            break 'done;
        }
        let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
            return true;
        };
        if !user_can_use(world, plr, &itm) {
            tell_user(world, id, Msg::WearForbidden);
            break 'done;
        }
        // Take off whatever is in the way
        let ring = |l: WornLocation| matches!(l, WornLocation::Finger | WornLocation::Finger2);
        let wrist = |l: WornLocation| matches!(l, WornLocation::Wrist | WornLocation::Wrist2);
        let (mut rings, mut bracelets) = (0u8, 0u8);
        for i in 0..NWORN {
            let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
                return true;
            };
            let w = plr.worn[i];
            if w == ItemNo(0) {
                continue;
            }
            let Some(witm) = world.items.get(&w).cloned() else {
                continue;
            };
            let clash = witm.worn == loc
                || witm.worn == WornLocation::Worn
                || (ring(loc) && ring(witm.worn))
                || (wrist(loc) && wrist(witm.worn));
            if !clash {
                continue;
            }
            if ring(witm.worn) || wrist(witm.worn) {
                // (sic) both counters go up for either kind
                rings = rings.wrapping_add(1);
                bracelets = bracelets.wrapping_add(1);
                if is_cursed(&witm) {
                    // A cursed one stays on: give up, silently
                    return false;
                }
                if (rings >= 2 && ring(witm.worn)) || (bracelets >= 2 && wrist(witm.worn)) {
                    // The pair is full: take it off
                    take_off_in_the_way(world, id, i, &witm);
                    rings = rings.wrapping_sub(1);
                    recalc(world, id);
                } else if witm.number == itemno {
                    // Another copy of the same: take it off
                    take_off_in_the_way(world, id, i, &witm);
                    recalc(world, id);
                }
            } else {
                if is_cursed(&witm) {
                    tell_user(world, id, Msg::WearCursedInWay { item: witm.name });
                    return false;
                }
                take_off_in_the_way(world, id, i, &witm);
            }
        }
        // No shield with a two-handed weapon
        let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
            return true;
        };
        if loc == WornLocation::OffHand
            && plr.weapon != ItemNo(0)
            && world.items.get(&plr.weapon).is_some_and(is_two_handed)
        {
            tell_user(world, id, Msg::OffhandWith2h);
            return false;
        }
        // Changing equipment breaks off autocombat
        break_off_autocombat(world, id, true);
        // Put it on, with its charges. (sic, not carried) EQUIP under the original can give
        // the item a stray charge count; here the charges come from EQUIP's own lookup.
        let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
            return true;
        };
        plr.worn[freeslot] = itemno;
        plr.wornuses[freeslot] = charges;
        let who = plr.firstname.clone();
        let room = room_of(plr);
        worn = true;
        // Its abilities, then the room and the player
        equip_apply_item_abilities(world, id, &itm);
        tell_others(
            world,
            id,
            room,
            &Msg::OtherWears {
                who,
                item: itm.name.clone(),
            },
        );
        tell_user(world, id, Msg::WearOk { item: itm.name });
    }
    recalc(world, id);
    worn
}

/// Takes off the worn item `name` names, puts out
/// the readied light, or unwields the weapon. A cursed item stays on unless `force`.
pub fn remove_armour(world: &mut World, id: SessionId, name: &str, force: bool) {
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
        return;
    };
    let f = find_item_in_inventory(world, plr, name, Search::Worn);
    // The buffer is emptied (for REMOVE, the
    // dispatcher's ANSI_NORMAL; DROP, HIDE and GIVE have cleared it already)
    crate::output::clear_output(world, id);
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
        return;
    };
    let Some(itm) = f.item.and_then(|n| world.items.get(&n)).cloned() else {
        tell_user(
            world,
            id,
            Msg::NotWearing {
                name: name.to_string(),
            },
        );
        recalc(world, id);
        return;
    };
    if !force && is_cursed(&itm) {
        // No recalculation on this path
        tell_user(world, id, Msg::CantRemove { item: itm.name });
        return;
    }
    let readied = plr.readied;
    let weapon = plr.weapon;
    let who = plr.firstname.clone();
    let room = room_of(plr);
    if itm.item_type == ItemType::Light {
        // Only the readied one can be put out
        if f.slot_code() == readied {
            let genlight = silvermere_rules::abilities::item_ability_value(&itm, Ability::GenLight);
            if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                plr.readied = -1;
                plr.light = plr.light.wrapping_sub(genlight as i16);
            }
            tell_others(
                world,
                id,
                room,
                &Msg::OtherRemovesBang {
                    who,
                    item: itm.name.clone(),
                },
            );
            tell_user(world, id, Msg::RemovedExtinguished { item: itm.name });
        } else {
            tell_user(
                world,
                id,
                Msg::NotLit {
                    name: name.to_string(),
                },
            );
        }
    } else if itm.number == weapon {
        ready_weapon(world, id, None);
    } else {
        // Every worn copy. Each "You have removed" waits in the output buffer
        // until the end, so a second copy's room line carries the first one's text to the
        // room instead (the buffer is shared, `tell_room` sends and clears it).
        let mut pending: Option<Msg> = None;
        for i in 0..NWORN {
            let w = world
                .sessions
                .get(&id)
                .and_then(|s| s.player())
                .map_or(ItemNo(0), |p| p.worn[i]);
            if w != itm.number {
                continue;
            }
            unequip_apply_item_abilities(world, id, &itm);
            if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
                plr.worn[i] = ItemNo(0);
                plr.wornuses[i] = 0;
            }
            if let Some(earlier) = pending.take() {
                tell_others(world, id, room, &earlier);
            }
            tell_others(
                world,
                id,
                room,
                &Msg::OtherRemovesBang {
                    who: who.clone(),
                    item: itm.name.clone(),
                },
            );
            pending = Some(Msg::Removed {
                item: itm.name.clone(),
            });
            // The waiting "You have removed" goes out with the break-off's
            // flush, ahead of `*Combat Off*`
            if is_inside_autocombat(world, id)
                && let Some(m) = pending.take()
            {
                tell_user(world, id, m);
            }
            break_off_autocombat(world, id, true);
        }
        if let Some(m) = pending {
            tell_user(world, id, m);
        }
    }
    recalc(world, id);
}

/// Wields `item` (EQUIP's find) or the unworn
/// carried weapon `name` names; `name` `None` puts the weapon away (UNREADY). A cursed
/// weapon in hand stops both. Returns whether a weapon was readied or put away.
pub fn ready_weapon(
    world: &mut World,
    id: SessionId,
    name: Option<(&str, Option<ItemNo>)>,
) -> bool {
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
        return false;
    };
    // A cursed weapon sticks to the hand
    if plr.weapon != ItemNo(0)
        && let Some(witm) = world.items.get(&plr.weapon)
        && is_cursed(witm)
    {
        let item = witm.name.clone();
        tell_user(world, id, Msg::CantRemove { item });
        return false;
    }
    let who = plr.firstname.clone();
    let room = room_of(plr);
    let Some((name, item)) = name else {
        // UNREADY
        let weapon = plr.weapon;
        if weapon != ItemNo(0)
            && let Some(witm) = world.items.get(&weapon).cloned()
        {
            tell_user(world, id, Msg::Unwielded);
            tell_others(
                world,
                id,
                room,
                &Msg::OtherRemovesDot {
                    who,
                    item: witm.name.clone(),
                },
            );
            // Changing weapons ends automatic combat
            break_off_autocombat(world, id, true);
            unequip_apply_item_abilities(world, id, &witm);
        }
        if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            plr.weapon = ItemNo(0);
        }
        recalc(world, id);
        return true;
    };
    let item = match item {
        Some(n) => Some(n),
        None => find_item_in_inventory(world, plr, name, Search::Unworn).item,
    };
    let mut rc = false;
    'done: {
        let Some(itm) = item.and_then(|n| world.items.get(&n)).cloned() else {
            tell_user(
                world,
                id,
                Msg::NotHaveA {
                    name: name.to_string(),
                },
            );
            break 'done;
        };
        if itm.item_type != ItemType::Weapon {
            tell_user(world, id, Msg::NotAWeapon { item: itm.name });
            break 'done;
        }
        let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
            return false;
        };
        if !user_can_use(world, plr, &itm) {
            tell_user(world, id, Msg::WeaponForbidden);
            break 'done;
        }
        // No two-handed weapon with something in the off hand
        if is_two_handed(&itm) {
            for &w in plr.worn.iter().take(NWORN) {
                if w == ItemNo(0) {
                    continue;
                }
                if let Some(witm) = world.items.get(&w)
                    && witm.worn == WornLocation::OffHand
                {
                    let item = witm.name.clone();
                    tell_user(world, id, Msg::Offhand2hReady { item });
                    return false;
                }
            }
        }
        // Changing weapons ends automatic combat
        let old = plr.weapon;
        break_off_autocombat(world, id, true);
        // The old weapon is put away first
        if old != ItemNo(0)
            && let Some(oitm) = world.items.get(&old).cloned()
        {
            tell_others(
                world,
                id,
                room,
                &Msg::OtherRemovesDot {
                    who: who.clone(),
                    item: oitm.name.clone(),
                },
            );
            unequip_apply_item_abilities(world, id, &oitm);
        }
        // Wield the new one
        let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
            return false;
        };
        plr.weapon = itm.number;
        rc = true;
        tell_others(
            world,
            id,
            room,
            &Msg::OtherWields {
                who,
                item: itm.name.clone(),
            },
        );
        tell_user(
            world,
            id,
            Msg::WieldOk {
                item: itm.name.clone(),
            },
        );
        let Some(plr) = world.sessions.get(&id).and_then(|s| s.player()) else {
            return false;
        };
        // Too weak for it (compared unsigned: a negative requirement is always too heavy)
        let heavy = itm.strreq as u16 > plr.str;
        // Quick with it
        let quick = world
            .classes
            .get(&plr.class)
            .is_some_and(|cls| is_weapon_too_fast_for_user(plr, cls, Some(&itm)))
            && !heavy;
        if heavy {
            tell_user(world, id, Msg::WeaponHeavy);
        }
        if quick {
            tell_user(world, id, Msg::WeaponQuick);
        }
        equip_apply_item_abilities(world, id, &itm);
    }
    recalc(world, id);
    rc
}
