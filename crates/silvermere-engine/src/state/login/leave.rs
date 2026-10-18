//! Leaving: `x` in the realm and its meditation, `X` at the menu, the hang-up and the
//! inactivity hang-up.

use std::sync::OnceLock;

use silvermere_rules::hangup::{HangupSituation, penalty, punishes};
use silvermere_text::catalogue::{self, Catalogue, WCCMMUD_TOML};
use silvermere_text::{Msg, cp437, finish};
use silvermere_types::enums::{Ability, InGame, RoomType, SessionState};
use silvermere_types::flags::{PlayerFlags, PlayerStatus, RoomFlags, SessionFlags};
use silvermere_types::ids::{ItemNo, RoomId, SessionId};
use silvermere_types::records::{Character, RoomItem};

use crate::state::{Outcome, in_realm};
use crate::tell::{Noise, TellBlind, tell_game, tell_room};
use crate::world::{RoomState, World};

/// The delay `cmd_quit` adds: the fast cycles of meditation before the exit.
pub const EXIT_DELAY: u8 = 6;

/// A `WCCMMUD.MSG` text block (T-011's `wccmmud.toml`) as the player receives it.
fn wccmmud(key: &str) -> Vec<u8> {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    let cat = CATALOGUE
        .get_or_init(|| catalogue::parse(WCCMMUD_TOML).expect("wccmmud.toml is a catalogue"));
    let text = &cat.messages.get(key).expect("a WCCMMUD.MSG block").text;
    finish(&cp437::encode(text), None)
}

/// T-023's [`crate::player::save_player`] for the session's player, with its guards (a
/// created, validated character) and the level-gated ability stripping.
pub fn save_player(world: &mut World, id: SessionId) {
    crate::player::save_player(world, id);
}

/// `x`, reached through `execute_input`. Returns `true` when the prompt follows (only
/// after the refusal).
///
/// Not carried: the add-on text variables. The input echo that `add_delay` turns off is
/// turned straight back on when no line is queued, which is always the case here until
/// T-024 brings the queue: the echo is left alone.
pub fn cmd_quit(world: &mut World, id: SessionId) -> bool {
    let Some(s) = world.session_mut(id) else {
        return false;
    };
    let Some(plr) = s.character.as_mut() else {
        return false;
    };
    if plr.hp <= 0 {
        s.output.push(Msg::ExitWounded);
        return true;
    }
    // "*Combat Off*" and a prompt when queued, then out of the queue either way.
    if crate::combat::is_inside_autocombat(world, id) {
        crate::combat::display_autocombat_broken(world, id);
    }
    crate::combat::kill_autocombat(world, id);
    let Some(s) = world.session_mut(id) else {
        return false;
    };
    s.autocmb = Default::default();
    let Some(plr) = s.character.as_mut() else {
        return false;
    };
    // (sic: the `.` after the line end)
    s.output.push(Msg::ExitMeditate);
    plr.delay = plr.delay.saturating_add(EXIT_DELAY);
    plr.status.insert(PlayerStatus::EXITING);
    plr.hidden = 0;
    plr.status.remove(PlayerStatus::SNEAKING);
    let who = plr.firstname.clone();
    let room = s.room_key();
    // the player's gang invitations are cleared
    world.gang_invites.retain(|g| g.session != id);
    if let Some(room) = room {
        tell_room(
            world,
            room,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::ExitOthers { who },
        );
    }
    false
}

/// Stage 1 of `execute_input`: a line typed while waiting to exit. `BREAK` cancels the
/// exit; anything else is refused.
pub fn exiting_line(world: &mut World, id: SessionId, line: &str) {
    let word = line.split_whitespace().next().unwrap_or("");
    if word.eq_ignore_ascii_case("break") {
        stop_users_exit(world, id);
    } else if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::ExitWaiting);
    }
}

/// Whether the session's player is waiting to exit (`PlayerStatus::EXITING`).
pub fn is_exiting(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .and_then(|s| s.player())
        .is_some_and(|p| p.status.contains(PlayerStatus::EXITING))
}

/// An attack or a spell (or `BREAK`) ends the meditation. The combat paths that call it
/// are T-033's and T-034's.
pub fn stop_users_exit(world: &mut World, id: SessionId) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if !plr.status.contains(PlayerStatus::EXITING) {
        return;
    }
    plr.status.remove(PlayerStatus::EXITING);
    plr.delay = 0;
    s.output.push(Msg::ExitInterrupted);
    s.push_prompt();
}

/// The exit part of `fast_update_character`, once per fast cycle: while `EXITING`, the
/// command delay counts down with a `...` each cycle; at zero the exit completes
/// ([`finish_exit`]). The rest of the delay handling (moves, queued commands, the
/// re-prompt) is T-037's and T-024's.
pub fn exit_countdown(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if !plr.status.contains(PlayerStatus::EXITING) || plr.delay == 0 {
        return;
    }
    plr.delay -= 1;
    if plr.delay == 0 {
        finish_exit(world, id);
    } else {
        world.touch(id);
        if let Some(s) = world.sessions.get_mut(&id) {
            s.output.push(Msg::ExitDots);
        }
    }
}

/// The completed exit: to the room's hang-up room when it has one, saved, back at the
/// main menu with `USREXIT` and the hard-coded `[MAJORMUD]:` prompt, and
/// `"%s just left the Realm."` to the game.
///
/// Not carried: the host's polling and screen-mode teardown, the add-on text variables.
/// `cleanup_when_user_leaves` ends the fights, the drag and the party; the session keeps
/// the character after `clear_player` so `E` can enter again without a database read (see
/// `enter_module`).
pub fn finish_exit(world: &mut World, id: SessionId) {
    let Some(key) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    let huproom = world.room(key).map_or(RoomId(0), |r| r.huproom);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    plr.status.remove(PlayerStatus::EXITING);
    crate::movement::move_user::clear_search_flags(world, id, key);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if huproom.0 != 0 {
        plr.room = huproom;
    }
    let who = plr.firstname.clone();
    crate::combat::cleanup_when_user_leaves(world, id);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.state = SessionState::MainMenu;
    s.ingame = InGame::Module;
    s.flags.ingame = InGame::Module;
    s.output.push_raw(wccmmud("usrexit"));
    s.output.push(Msg::MudMenuPromptCr);
    save_player(world, id);
    // The BBS invisible flag is not carried.
    tell_game(
        world,
        Some(id),
        PlayerFlags::NOENTRYMSG,
        true,
        None,
        &Msg::PlayerLeftRealm { who },
    );
}

/// `X` at the main menu: out of the module. `cleanup_when_user_leaves` and `clear_player`
/// free the channel's game state; `LEAVEMNU` is the sub-board hook text, which neither
/// MBBSEmu nor silvermere shows (the `entry-existing` capture goes straight from the echo
/// to the BBS menu). Then the BBS menu shim, or the logoff when it is off.
///
/// The save here is an addition: the character was already saved when the player left the
/// realm. The extra persistence request is kept so a character changed at the menu (a race
/// change, T-047) is never lost.
pub fn leave_module(world: &mut World, id: SessionId) -> Outcome {
    save_player(world, id);
    if world.compat.bbs_menu {
        super::bbs_menu(world, id);
        Outcome::Continue
    } else {
        Outcome::Logoff
    }
}

/// The hang-up of a session whose connection is gone. For a player in the realm: in a
/// safe room only the hang-up room; otherwise, when `PUNHUP` says so, the hit-point loss
/// ([`penalty`]) and up to `MAXHUPIT` items dropped, then the hang-up room and the
/// hung-up status for the next entry's notice; `"%s just disconnected!!!"` to the game.
/// The character is saved once, after the punishment.
///
/// The situation is read from the live combat state before anything is changed:
/// `is_inside_autocombat` (the player is queued to
/// fight), `is_being_attacked` (another player in the game fights him; a monster's attack
/// does not count) and `is_in_pvp_combat`; the arena's safe mode is the sysop's
/// `arenasafe`.
pub fn hangup(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    s.ust.forgets.clear();
    let playing = s.character.is_some() && in_realm(s.state);
    let Some(key) = s.room_key().filter(|_| playing) else {
        save_player(world, id);
        return;
    };
    let situation = HangupSituation {
        in_autocombat: crate::combat::autocombat::is_inside_autocombat(world, id),
        being_attacked: crate::combat::autocombat::is_being_attacked(world, id),
        in_pvp: crate::combat::autocombat::is_in_pvp_combat(world, id),
    };
    let arena_safe = world.display.arena_safe;
    let fighting = situation.in_autocombat || situation.being_attacked;
    crate::movement::move_user::clear_search_flags(world, id, key);
    let (safe, huproom) = match world.room(key) {
        Some(rm) => (
            (rm.flags.contains(RoomFlags::PROTECTED) && !fighting)
                || (rm.room_type == RoomType::Colliseum && arena_safe),
            rm.huproom,
        ),
        None => (false, RoomId(0)),
    };
    let config = world.config.clone();
    let World {
        sessions,
        items,
        room_state,
        ..
    } = world;
    let Some(plr) = sessions.get_mut(&id).and_then(|s| s.character.as_mut()) else {
        return;
    };
    if !safe {
        if punishes(config.pun_hup, situation) {
            if plr.hp > 0 {
                let p = penalty(i32::from(plr.maxhp), &config, &mut world.rng);
                plr.hp = (i32::from(plr.hp) - p.hp_loss) as i16;
            } else {
                plr.hp = (i32::from(plr.hp) + config.death_hp) as i16;
            }
            drop_items(plr, items, room_state.get_mut(&key), config.max_hup_items);
        }
        plr.status.insert(PlayerStatus::HUNGUP);
    }
    if huproom.0 != 0 {
        plr.room = huproom;
    }
    let who = plr.firstname.clone();
    tell_game(
        world,
        Some(id),
        PlayerFlags::NOENTRYMSG,
        true,
        None,
        &Msg::PlayerDisconnected { who },
    );
    crate::combat::check_kill_user(world, id);
    save_player(world, id);
}

/// Drops up to `max` items into the room: every second inventory slot (sic), then every
/// second key slot, Loyal Items kept, items marked `destroy` removed without being dropped.
/// An inventory item counts only when it left the inventory; a key counts whenever its
/// record exists, Loyal or not (sic). An item's 20 abilities are read.
fn drop_items(
    plr: &mut Character,
    items: &std::collections::HashMap<ItemNo, silvermere_types::records::Item>,
    mut room: Option<&mut RoomState>,
    max: u32,
) {
    let loyal = |itm: &silvermere_types::records::Item| {
        itm.abilities.iter().any(|(a, _)| *a == Ability::LoyalItem)
    };
    let mut cnt = 0;
    let mut i = 0;
    while cnt < max && i < plr.items.len() {
        let itemno = plr.items[i];
        if itemno.0 != 0
            && let Some(itm) = items.get(&itemno)
            && !loyal(itm)
        {
            let uses = remove_item_from_inventory(plr, itemno, itm.weight).unwrap_or(0);
            let dropped = itm.destroy != 0 || add_item_to_room(room.as_deref_mut(), itemno, uses);
            if dropped {
                cnt += 1;
            }
        }
        i += 2;
    }
    let mut i = 0;
    while cnt < max && i < plr.keys.len() {
        let itemno = plr.keys[i];
        if itemno.0 != 0
            && let Some(itm) = items.get(&itemno)
        {
            if !loyal(itm) {
                let uses = remove_item_from_inventory(plr, itemno, itm.weight).unwrap_or(0);
                if itm.destroy == 0 {
                    add_item_to_room(room.as_deref_mut(), itemno, uses);
                }
            }
            cnt += 1;
        }
        i += 2;
    }
}

/// The part of item removal the hang-up needs: the first inventory slot holding the item,
/// else the first key slot, is emptied and the weight taken off; returns the copy's uses.
/// TODO(T-030): the readied light, the wielded and worn copies, the limited-item
/// registration.
fn remove_item_from_inventory(plr: &mut Character, itemno: ItemNo, weight: i16) -> Option<i16> {
    if let Some(i) = plr.items.iter().position(|&x| x == itemno) {
        let uses = plr.itemuses[i];
        plr.items[i] = ItemNo(0);
        plr.itemuses[i] = -1;
        plr.wgt = plr.wgt.saturating_sub(weight.max(0) as u16);
        return Some(uses);
    }
    let nkeys = usize::from(plr.nkeyslots).min(plr.keys.len());
    if let Some(i) = plr.keys[..nkeys].iter().position(|&x| x == itemno) {
        let uses = plr.keyuses[i];
        plr.keys[i] = ItemNo(0);
        plr.keyuses[i] = -2;
        plr.wgt = plr.wgt.saturating_sub(weight.max(0) as u16);
        return Some(uses);
    }
    None
}

/// Adding an item to a room, reduced to what the hang-up needs: the first empty visible
/// slot. `false` when the room is full or unknown. TODO(T-030): stacking equal items as the
/// original does.
fn add_item_to_room(room: Option<&mut RoomState>, item: ItemNo, uses: i16) -> bool {
    let Some(room) = room else {
        return false;
    };
    match room.items.iter_mut().find(|slot| slot.item.0 == 0) {
        Some(slot) => {
            *slot = RoomItem {
                item,
                uses,
                count: 0,
            };
            true
        }
        None => false,
    }
}

/// The inactivity check, run every `game.inactive_secs` by `Routine::Inactivity`: a
/// logged-in session still carrying the mark of the last run is hung up with `INACHUP`,
/// after its state is set to the main menu so that the realm is left without the hang-up
/// punishment; every other session is marked. Typing anything clears the mark
/// (`session::on_line`). Sysops keep the mark and stay.
pub fn inactivity_check(world: &mut World) {
    for id in world.session_ids() {
        let Some(s) = world.sessions.get_mut(&id) else {
            continue;
        };
        // in the module: here, logged in
        if s.account.is_none() {
            continue;
        }
        if s.ust.flags.contains(SessionFlags::INACTWARN) && !s.flags.sysop {
            s.state = SessionState::MainMenu;
            s.output.push_raw(wccmmud("inachup"));
            world.touch(id);
            save_player(world, id);
            world.logoff(id);
        } else {
            s.ust.flags.insert(SessionFlags::INACTWARN);
        }
    }
}
