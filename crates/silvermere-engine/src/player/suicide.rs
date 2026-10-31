//! [`commit_suicide`]: what `SUICIDE`, `SUICIDE NOW` and `REROLL` do once the player is
//! cleared to go ahead. `cmd_suicide` (`commands::misc`) calls it at once when there is no
//! suicide password; the `SUBST_SUICIDE` arm (`state::suicide`) after the right password.
//!
//! Everything carried goes first: the coins onto the floor, the inventory and the keys
//! into the room, or when it is full a room near it or along the trail (an item set to be
//! destroyed, or one with nowhere to go, "has returned to its rightful place"), loyal and
//! major-cursed items kept unless the character is about to be deleted; the worn items and
//! the weapon are cleared, the spells end and the poison goes. Then either the character
//! is deleted ([`profile::retire`], the deleting branch, `ST_SUICIDE` or the last life)
//! and the player is back at the menu, or one life is lost and the character wakes up in
//! the reincarnation room.
//!
//! Parts that belong to other cards and are not done here yet:
//! `unequip_apply_item_abilities` (T-030/T-032), the spells' endings (T-050: the spell
//! slots are cleared without the ending messages), `remove_from_gang` (T-044), the bank
//! books (T-042) and leaving the split-screen display (not carried). The death text runs
//! through `scripts::run_text_block` (T-054).

use silvermere_text::Msg;
use silvermere_types::enums::Ability;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ItemNo, MapId, RoomId, RoomKey, SessionId, SpellNo};
use silvermere_types::records::{Coins, Item};
use silvermere_types::sizes::NWORN;

use super::EVIL_OUTLAW;
use super::profile::{self, Retire};
use super::secondary::{CalcMode, recalculate};
use crate::event_loop::{EchoMode, EngineOutput, GameEvent};
use crate::items::limited::{self, Holder};
use crate::items::room_items::{dispose_of_item_in_room, dispose_of_item_in_trail};
use crate::state::login::show_menu;
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// `NO_CHARGES`: an empty inventory or worn slot's uses.
const NO_CHARGES: i16 = -1;
/// `KEY_EMPTY`: an empty key slot's uses.
const KEY_EMPTY: i16 = -2;

/// What [`commit_suicide`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suicide {
    /// The character is deleted and the player is at the main menu.
    Deleted,
    /// One life lost; the game goes on.
    LifeLost,
    /// No character: nothing done.
    NoCharacter,
}

impl Suicide {
    /// Whether the character is gone.
    pub fn gone(self) -> bool {
        self == Self::Deleted
    }
}

/// The value of `abl` on `itm`, 0 without it.
fn ability_value(itm: &Item, abl: Ability) -> i16 {
    itm.abilities
        .iter()
        .find(|(a, _)| *a == abl)
        .map_or(0, |(_, v)| *v)
}

/// Where a dropped item goes (`commit_suicide`'s two tries): the room or one near it
/// ([`dispose_of_item_in_room`]), else a room of the trail. `false`: it is lost. (Unlike a
/// death, there is no third try in room 164 of map 1.) The original's developer-mode error
/// report is a warning in the log here.
fn dispose(world: &mut World, id: SessionId, room: RoomKey, item: ItemNo, uses: i16) -> bool {
    let placed = dispose_of_item_in_room(world, room, item, uses)
        || dispose_of_item_in_trail(world, id, item, uses);
    if !placed {
        tracing::warn!(?id, item = item.0, room = ?room, "failed to drop an item at suicide");
    }
    placed
}

/// A suicide or reroll once cleared. `today` is the day as the database stores it
/// (`deathdate`, the kept evil points).
pub fn commit_suicide(world: &mut World, id: SessionId, today: i64) -> Suicide {
    // `cleanup_when_user_leaves`: whoever fights him stops, whoever drags him lets go, his
    // followers are told and he leaves his party, his gang invitations and pending commands
    // go and his own combat ends. Run first: the steps before it only read `evil`, which it
    // does not touch.
    crate::session::cleanup_when_user_leaves(world, id);
    let World {
        sessions,
        rooms,
        room_state,
        config,
        ..
    } = world;
    let Some(session) = sessions.get_mut(&id) else {
        return Suicide::NoCharacter;
    };
    let party = &mut session.party;
    let Some(plr) = session.character.as_mut() else {
        return Suicide::NoCharacter;
    };
    // map 1, the evil or the normal reincarnation room
    let mut newmap = MapId(1);
    let mut newroom = RoomId(if plr.evil >= EVIL_OUTLAW {
        config.evil_reinc_room
    } else {
        config.reinc_room
    });
    plr.status.remove(PlayerStatus::DRAGGING);
    // dragging nobody (`dragging` is session state here, `Party::dragging`)
    party.dragging = None;
    plr.readied = -1;
    plr.light = 0;
    plr.lighttime = 0;
    // the coins fall onto the floor; a room may name its own death room
    let key = RoomKey::new(plr.map.0, plr.room.0);
    if let Some(rm) = room_state.get_mut(&key) {
        let c = &mut rm.coins;
        c.runic = c.runic.wrapping_add(plr.coins.runic);
        c.platinum = c.platinum.wrapping_add(plr.coins.platinum);
        c.gold = c.gold.wrapping_add(plr.coins.gold);
        c.silver = c.silver.wrapping_add(plr.coins.silver);
        c.copper = c.copper.wrapping_add(plr.coins.copper);
        if let Some(room) = rooms.get(&key)
            && room.deathroom.0 != 0
        {
            newroom = room.deathroom;
            newmap = room.key.map;
        }
    }
    // he no longer counts as having searched this room (only with a room record)
    if room_state.contains_key(&key) {
        crate::movement::move_user::clear_search_flags_of(plr, room_state.get_mut(&key));
    }
    plr.coins = Coins::default();

    let deleting = plr.status.contains(PlayerStatus::SUICIDE) || plr.lives <= 1;
    let (lives, suicide) = (plr.lives, plr.status.contains(PlayerStatus::SUICIDE));
    let userid = plr.userid.clone();
    let (slots, uses) = (plr.items.clone(), plr.itemuses.clone());
    let (keys, keyuses) = (plr.keys.clone(), plr.keyuses.clone());
    let mut returned: Vec<String> = Vec::new();

    // the inventory: in the room, else near it, else along the trail, else lost
    let mut deathtext = 0;
    for (i, &itemno) in slots.iter().enumerate() {
        if itemno.0 == 0 {
            continue;
        }
        match world.items.get(&itemno).cloned() {
            Some(itm) => {
                // (sic) every item overwrites it: only the last one counts
                deathtext = ability_value(&itm, Ability::DeathText);
                // loyal and major-cursed items stay
                if lives != 1
                    && !suicide
                    && itm
                        .abilities
                        .iter()
                        .any(|(a, _)| matches!(a, Ability::LoyalItem | Ability::MajorCurse))
                {
                    continue;
                }
                limited::release(world, itemno, Holder::User(&userid));
                if itm.destroy != 0 || !dispose(world, id, key, itemno, uses[i]) {
                    returned.push(itm.name);
                }
            }
            None => {
                tracing::error!(?id, item = itemno.0, "Sui:Item not found");
            }
        }
        if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            plr.items[i] = ItemNo(0);
            plr.itemuses[i] = NO_CHARGES;
        }
    }
    // the keys, all of them, the same way
    for (i, &itemno) in keys.iter().enumerate() {
        if itemno.0 == 0 {
            continue;
        }
        match world.items.get(&itemno).cloned() {
            Some(itm) => {
                limited::release(world, itemno, Holder::User(&userid));
                if itm.destroy != 0 || !dispose(world, id, key, itemno, keyuses[i]) {
                    returned.push(itm.name);
                }
            }
            None => tracing::error!(?id, key = itemno.0, "Sui:Key not found"),
        }
        if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            plr.keys[i] = ItemNo(0);
            plr.keyuses[i] = KEY_EMPTY;
        }
    }
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Suicide::NoCharacter;
    };
    // the worn items and the weapon (their abilities: TODO(T-030, T-032))
    plr.worn = [ItemNo(0); NWORN];
    plr.wornuses = [NO_CHARGES; NWORN];
    plr.weapon = ItemNo(0);
    // nothing is carried any more
    plr.wgt = 0;
    // the spells end (each one's ending runs below, once the record is free)
    let ended: Vec<(SpellNo, i16)> = plr
        .spells
        .iter()
        .zip(plr.spellvals.iter())
        .filter(|(s, _)| s.0 != 0)
        .map(|(s, v)| (*s, *v))
        .collect();
    plr.spells = Default::default();
    plr.spellvals = Default::default();
    plr.spellrnds = Default::default();
    plr.poison = 0;
    let name = plr.firstname.clone();

    let out = &mut world.sessions.get_mut(&id).expect("looked up above").output;
    for item in returned {
        out.push(Msg::SuicideItemReturned { item });
    }
    // each spell ends, with no chain cast
    for (n, power) in ended {
        if let Some(spl) = world.spells.get(&n).cloned() {
            crate::spells::active::end_player_spell(world, id, &spl, i32::from(power), false);
        }
    }
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.poison = 0;
    }

    if deleting {
        // the gang, taken first: retire ends with delete_player and
        // clear_player, and remove_from_gang only changes memory
        let gang = world
            .session(id)
            .and_then(|s| s.player())
            .map(|p| p.gang.clone())
            .unwrap_or_default();
        if !gang.is_empty() {
            crate::gangs::remove_from_gang(world, id, &gang);
        }
        // nobody follows him any more and he leaves any group (before retire's
        // clear_player, so his followers still read his name)
        crate::party::stop_following_all(world, id);
        // evil points, deathdate, the profile, the top list, delete_player
        profile::retire(world, id, today, Retire::Suicide);
        // the bank books (T-042): TODO.
        // the original's line here shows a blank name (sic); this one carries it
        if world.config.sys_log_deaths {
            tracing::info!(target: "audit", ?id, name = %name, "User {name} has rerolled.");
        }
        world
            .outbox
            .push(EngineOutput::Event(GameEvent::Rerolled { id }));
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::SuicideDeleted);
        }
        // the main menu, in normal colour
        show_menu(world, id);
        return Suicide::Deleted;
    }

    // recount the load, lose the life
    let lives = match world.session_mut(id).and_then(|s| s.character.as_mut()) {
        Some(plr) => {
            plr.lives -= 1;
            plr.lives
        }
        None => return Suicide::LifeLost,
    };
    // echo back on
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::On,
    });
    if world.config.sys_log_deaths {
        tracing::info!(target: "audit", ?id, name = %name, "User {name} has committed suicide.");
    }
    world
        .outbox
        .push(EngineOutput::Event(GameEvent::Suicided { id }));
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::SuicideLifeLost {
            lives: i32::from(lives),
        });
    }
    // the room hears it, with their prompts
    tell_room(
        world,
        key,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &Msg::SuicideRoom { who: name },
    );
    // full strength in the reincarnation room, the derived stats again
    if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
        c.hp = c.maxhp;
        c.mana = c.maxmana as i16;
        c.map = newmap;
        c.room = newroom;
        // the carried weight: the loyal and cursed items kept still weigh
        c.wgt = c
            .items
            .iter()
            .filter_map(|n| world.items.get(n))
            .map(|itm| itm.weight.max(0) as u16)
            .fold(0u16, u16::saturating_add);
        recalculate(world, &mut c, CalcMode::KeepCurrent);
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
    }
    // the death text block, run as a whole
    if deathtext != 0 {
        crate::scripts::run_text_block(
            world,
            id,
            silvermere_types::ids::TextBlockNo(i32::from(deathtext)),
        );
    }
    Suicide::LifeLost
}
