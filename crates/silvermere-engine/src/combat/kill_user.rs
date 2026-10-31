//! A character's death ([`check_kill_user`]) and the knock-out ([`knock_out_user`]).
//!
//! Between 0 and `deathhp` hit points (the `death_hp` option: -10 in the module, -200 on
//! John's server) a character is only unconscious. At `deathhp` or below he dies: `You
//! have been killed!`, the room hears `<Name> is dead.`, his spells end, his coins and
//! belongings fall where he died (the death pile: ordinary floor items), a life is lost and
//! he wakes up with full hit points in the death room (`But, due to a miracle, ...`); in
//! a safe colliseum he is restored instead and keeps everything; with no lives left the
//! character is deleted and the player is back at the menu.

use silvermere_text::Msg;
use silvermere_types::enums::{Ability, RoomType, SessionState};
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ItemNo, MapId, RoomId, RoomKey, SessionId, SpellNo};
use silvermere_types::records::Coins;
use silvermere_types::sizes::NWORN;

use super::{cleanup_when_user_leaves, item_ability, item_has, player, player_mut, room_of};
use crate::event_loop::{EchoMode, EngineOutput};
use crate::items::limited::{self, Holder};
use crate::items::room_items::{dispose_of_item_in_room, dispose_of_item_in_trail};
use crate::player::profile::{self, Retire};
use crate::player::secondary::{CalcMode, recalculate};

use crate::state::login::show_menu;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Evil points from which a character wakes in the evil reincarnation room.
const EVIL_OUTLAW: i16 = 40;
/// The room an item goes to when there is no room for it where its owner died.
const LOST_ITEM_ROOM: (u32, u32) = (1, 164);

/// What [`check_kill_user`] found and did, for the callers (T-034, T-036, T-037).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillOutcome {
    /// Hit points above 0, no character, or no room record: nothing done.
    Alive,
    /// Between 0 and `deathhp`: unconscious; he lets go of whoever he dragged.
    Unconscious,
    /// Died in a safe colliseum: restored in the death room, nothing lost.
    Arena,
    /// Died and lost a life; awake in the death room with full hit points.
    Reborn,
    /// Died on the last life: the character is deleted.
    Deleted,
}

impl KillOutcome {
    /// Whether the character died.
    pub fn died(self) -> bool {
        matches!(self, Self::Arena | Self::Reborn | Self::Deleted)
    }
}

/// A character knocked out, as the cycles do it (`cycles::knock_out_user`): `<Name> drops
/// to the ground!` in grey on red with the bell, to the whole room.
pub fn knock_out_user(world: &mut World, id: SessionId) {
    crate::cycles::knock_out_user(world, id);
}

/// A character at 0 hit points or below: knocked out or killed.
///
/// Not carried: the developer-mode error reports (logged instead) and the split-screen
/// mode's ending. Belonging to other cards and reached through their stand-ins:
/// `cleanup_when_user_leaves` (T-034, T-043), the spells' ending messages (T-050), the item
/// abilities taken back (`unequip_apply_item_abilities`: `recalculate` rebuilds them),
/// `remove_from_gang` (T-044) and the bankbooks' removal (T-042).
pub fn check_kill_user(world: &mut World, id: SessionId) -> KillOutcome {
    let Some(plr) = player(world, id).cloned() else {
        return KillOutcome::Alive;
    };
    if plr.hp > 0 {
        return KillOutcome::Alive;
    }
    // where he will wake up
    let mut deathmap = MapId(1);
    let mut deathroom = RoomId(if plr.evil >= EVIL_OUTLAW {
        world.config.evil_reinc_room
    } else {
        world.config.reinc_room
    });
    // his followers stop following him, and he leaves his party
    crate::party::stop_following_all(world, id);
    // no room record: nothing more
    let key = room_of(&plr);
    let Some(rm) = world.room(key).cloned() else {
        return KillOutcome::Alive;
    };
    crate::movement::move_user::clear_search_flags(world, id, key);
    // above deathhp he is only unconscious; a safe colliseum has no such state
    let arena = world.display.arena_safe && rm.room_type == RoomType::Colliseum;
    if !arena && i32::from(plr.hp) > world.config.death_hp {
        if let Some(s) = world.session_mut(id) {
            s.party.dragging = None;
        }
        return KillOutcome::Unconscious;
    }
    tell_user(world, id, Msg::YouKilled);
    if world.config.sys_log_deaths {
        tracing::info!(target: "audit", ?id, name = %plr.firstname, "User {} has been killed.", plr.firstname);
    }
    crate::metrics::death();
    cleanup_when_user_leaves(world, id);
    if let Some(s) = world.session_mut(id) {
        s.party.dragging = None;
    }
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::DRAGGING);
        p.readied = -1;
        p.light = 0;
        p.lighttime = 0;
        p.movedir = -1;
        p.delay = 0;
    }
    // the room hears it
    tell_room(
        world,
        key,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &Msg::PlayerIsDead {
            who: plr.firstname.clone(),
        },
    );
    // his spells end, each with its ending (no chain), then the poison
    for i in 0..silvermere_types::sizes::NSPELLCAST {
        let Some(p) = player_mut(world, id) else {
            break;
        };
        let (n, power) = (p.spells[i], p.spellvals[i]);
        if n == SpellNo(0) {
            continue;
        }
        p.spells[i] = SpellNo(0);
        p.spellvals[i] = 0;
        p.spellrnds[i] = 0;
        if let Some(spl) = world.spells.get(&n).cloned() {
            crate::spells::active::end_player_spell(world, id, &spl, i32::from(power), false);
        }
    }
    if let Some(p) = player_mut(world, id) {
        p.poison = 0;
    }
    // the room's own death room
    if rm.deathroom.0 != 0 {
        deathroom = rm.deathroom;
        deathmap = rm.key.map;
    }
    // a safe colliseum: restored, nothing lost
    if arena {
        let lives = if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
            c.hp = c.arenahp;
            c.mana = c.maxmana as i16;
            c.map = deathmap;
            c.room = deathroom;
            recalculate(world, &mut c, CalcMode::KeepCurrent);
            let lives = c.lives;
            if let Some(s) = world.session_mut(id) {
                s.character = Some(c);
            }
            lives
        } else {
            0
        };
        tell_user(world, id, Msg::SavedColliseum);
        tell_user(
            world,
            id,
            Msg::LivesLeft {
                lives: i32::from(lives),
            },
        );
        appeared(
            world,
            id,
            &plr.firstname,
            RoomKey {
                map: deathmap,
                room: deathroom,
            },
        );
        echo_on(world, id);
        return KillOutcome::Arena;
    }
    // a real death: the coins stay in the room
    if let Some(rs) = world.room_state.get_mut(&key) {
        let c = &mut rs.coins;
        c.runic = c.runic.wrapping_add(plr.coins.runic);
        c.platinum = c.platinum.wrapping_add(plr.coins.platinum);
        c.gold = c.gold.wrapping_add(plr.coins.gold);
        c.silver = c.silver.wrapping_add(plr.coins.silver);
        c.copper = c.copper.wrapping_add(plr.coins.copper);
    }
    crate::items::room_items::mark_changed(world, key);
    if let Some(p) = player_mut(world, id) {
        p.coins = Coins::default();
    }
    // the inventory falls; loyal and major-cursed items stay unless this was the last life
    let mut deathtext = 0;
    let mut returned: Vec<String> = Vec::new();
    for i in 0..plr.items.len() {
        let itemno = plr.items[i];
        if itemno.0 == 0 {
            continue;
        }
        match world.items.get(&itemno).cloned() {
            Some(itm) => {
                // (sic) every item overwrites it: only the last one counts
                deathtext = item_ability(&itm, Ability::DeathText);
                if plr.lives != 1
                    && (item_has(&itm, Ability::LoyalItem) || item_has(&itm, Ability::MajorCurse))
                {
                    continue;
                }
                limited::release(world, itemno, Holder::User(&plr.userid));
                if itm.destroy == 0 {
                    let uses = plr.itemuses[i];
                    let lost = RoomKey::new(LOST_ITEM_ROOM.0, LOST_ITEM_ROOM.1);
                    if !dispose_of_item_in_room(world, key, itemno, uses)
                        && !dispose_of_item_in_trail(world, id, itemno, uses)
                        && !dispose_of_item_in_room(world, lost, itemno, uses)
                    {
                        returned.push(itm.name.clone());
                        tracing::warn!(
                            ?id,
                            item = itemno.0,
                            "failed to drop {} at death",
                            itm.name
                        );
                    }
                } else {
                    returned.push(itm.name.clone());
                }
            }
            None => tracing::error!(?id, item = itemno.0, "Kil:Item not found"),
        }
        if let Some(p) = player_mut(world, id) {
            p.items[i] = ItemNo(0);
            p.itemuses[i] = -1;
        }
        // one "returned" line per item, in slot order
        for item in returned.drain(..) {
            tell_user(world, id, Msg::SuicideItemReturned { item });
        }
    }
    // the keys, all of them
    for i in 0..plr.keys.len() {
        let itemno = plr.keys[i];
        if itemno.0 == 0 {
            continue;
        }
        match world.items.get(&itemno).cloned() {
            Some(itm) => {
                limited::release(world, itemno, Holder::User(&plr.userid));
                let uses = plr.keyuses[i];
                if itm.destroy != 0
                    || (!dispose_of_item_in_room(world, key, itemno, uses)
                        && !dispose_of_item_in_trail(world, id, itemno, uses))
                {
                    tell_user(
                        world,
                        id,
                        Msg::SuicideItemReturned {
                            item: itm.name.clone(),
                        },
                    );
                }
            }
            None => tracing::error!(?id, key = itemno.0, "Kil:Key not found"),
        }
        if let Some(p) = player_mut(world, id) {
            p.keys[i] = ItemNo(0);
            p.keyuses[i] = -2;
        }
    }
    // the worn items and the weapon come off
    if let Some(p) = player_mut(world, id) {
        p.worn = [ItemNo(0); NWORN];
        p.wornuses = [-1; NWORN];
        p.weapon = ItemNo(0);
    }
    // a life is lost
    let lives = player_mut(world, id).map_or(0, |p| {
        p.lives -= 1;
        p.lives
    });
    if lives > 0 {
        // awake in the death room, full strength
        if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
            c.wgt = c
                .items
                .iter()
                .filter_map(|n| world.items.get(n))
                .map(|itm| itm.weight.max(0) as u16)
                .fold(0u16, u16::saturating_add);
            c.hp = c.maxhp;
            c.mana = c.maxmana as i16;
            c.map = deathmap;
            c.room = deathroom;
            recalculate(world, &mut c, CalcMode::KeepCurrent);
            if let Some(s) = world.session_mut(id) {
                s.character = Some(c);
            }
        }
        tell_user(world, id, Msg::SavedMiracle);
        tell_user(
            world,
            id,
            Msg::LivesLeft {
                lives: i32::from(lives),
            },
        );
        echo_on(world, id);
        if let Some(s) = world.session_mut(id) {
            s.push_prompt();
        }
        appeared(
            world,
            id,
            &plr.firstname,
            RoomKey {
                map: deathmap,
                room: deathroom,
            },
        );
        // save_player: the periodic save picks the record up (T-023)
        crate::player::save::save_at(world, id, crate::player::save::SavePoint::DeathWithLives);
        // the death text block, run as a whole
        if deathtext != 0 {
            crate::scripts::run_text_block(
                world,
                id,
                silvermere_types::ids::TextBlockNo(deathtext),
            );
        }
        return KillOutcome::Reborn;
    }
    // no lives left: the character is gone
    let today = world.today;
    profile::retire(world, id, today, Retire::LastLife);
    // the gang (T-044) and the bank books (T-042)
    tell_user(world, id, Msg::NoLivesRemaining);
    show_menu(world, id);
    // back to the main menu
    if let Some(s) = world.session_mut(id) {
        s.state = SessionState::MainMenu;
    }
    KillOutcome::Deleted
}

/// `<Name> appeared on the floor in the middle of the room.` to the room he wakes in.
fn appeared(world: &mut World, id: SessionId, name: &str, room: RoomKey) {
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &Msg::AppearedOnFloor {
            who: name.to_owned(),
        },
    );
}

/// Echo back on for the player.
fn echo_on(world: &mut World, id: SessionId) {
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::On,
    });
}
