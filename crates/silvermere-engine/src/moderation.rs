//! Sysop moderation in the running game (silvermere's own; the original had none of it):
//! disconnecting a player, a line to one player, timed mutes and jail, and closing the realm
//! to all but sysops.
//!
//! The sanctions themselves are stored by `bin` (`account_sanctions`) before it asks the
//! engine; the engine holds the ones in force on the session ([`Sanctions`], read at login),
//! enforces them, and ends them when their time is up ([`on_tick`]). A jail that ended while
//! its player was away is ended at the next entry to the realm. Ending a jail asks the store
//! to delete its row ([`crate::event_loop::PersistRequest::LiftSanction`]); a mute's row is
//! simply over.
//!
//! A jailed player stays in the jail room (`game.jail_room`): walking, following, being
//! dragged and every teleport out are refused ([`holds`]). When the jail ends he is moved to
//! the reincarnation room, the town the sysop's `GOTO SILVERMERE` goes to.

use silvermere_text::Msg;
use silvermere_types::ids::{AccountId, RoomKey, SessionId};

use crate::event_loop::{EngineOutput, PersistRequest};
use crate::world::World;

/// A mute or a jail as the engine holds it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sanction {
    /// When it ends, Unix seconds; `None` until lifted.
    pub until: Option<i64>,
    /// Told to the player; may be empty.
    pub reason: String,
}

impl Sanction {
    /// Whether it is in force at `now` (Unix seconds).
    pub fn active_at(&self, now: i64) -> bool {
        self.until.is_none_or(|u| u > now)
    }
}

/// The sanctions on a session's account.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sanctions {
    /// No gossip or auction.
    pub mute: Option<Sanction>,
    /// Held in the jail room. A jail that has ended is kept until the player is let out.
    pub jail: Option<Sanction>,
}

/// Which sanction an [`crate::event_loop::AdminRequest::Sanction`] sets or lifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mute,
    Jail,
}

impl Kind {
    /// The `account_sanctions.kind` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Mute => "mute",
            Kind::Jail => "jail",
        }
    }
}

/// ` until 2026-12-01 18:00 UTC`, or empty.
pub fn until_text(until: Option<i64>) -> String {
    until
        .and_then(|u| jiff::Timestamp::from_second(u).ok())
        .map(|t| format!(" until {}", t.strftime("%Y-%m-%d %H:%M UTC")))
        .unwrap_or_default()
}

/// ` Reason: ...`, or empty.
pub fn reason_text(reason: &str) -> String {
    if reason.is_empty() {
        String::new()
    } else {
        format!(" Reason: {reason}")
    }
}

/// The session logged in to `account`, if any.
pub fn session_of(world: &World, account: AccountId) -> Option<SessionId> {
    world.session_ids().into_iter().find(|&id| {
        world
            .session(id)
            .and_then(|s| s.account.as_ref())
            .is_some_and(|a| a.id == account)
    })
}

/// The name a sysop knows the session by: the character's first name, else the account's.
fn name_of(world: &World, id: SessionId) -> String {
    let Some(s) = world.session(id) else {
        return String::new();
    };
    s.character
        .as_ref()
        .filter(|_| s.created)
        .map(|c| c.firstname.clone())
        .or_else(|| s.account.as_ref().map(|a| a.username.clone()))
        .unwrap_or_default()
}

/// The jail room (`game.jail_room` on map 1).
pub fn jail_room(world: &World) -> RoomKey {
    World::map1(world.config.jail_room)
}

/// The room a released prisoner goes to: the reincarnation room.
fn town(world: &World) -> RoomKey {
    World::map1(world.config.reinc_room)
}

/// The mute message, when the player's mute is in force: GOSSIP and AUCTION refuse.
pub fn muted(world: &World, id: SessionId) -> Option<Msg> {
    let m = world.session(id)?.sanctions.mute.as_ref()?;
    m.active_at(world.wall).then(|| Msg::ModMuted {
        until: until_text(m.until),
        reason: reason_text(&m.reason),
    })
}

/// Whether the player's jail is in force: he may not leave the jail room.
pub fn jailed(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .and_then(|s| s.sanctions.jail.as_ref())
        .is_some_and(|j| j.active_at(world.wall))
}

/// Whether a jailed player is kept from `to`: any room but the jail room. Tells him why.
pub fn holds(world: &mut World, id: SessionId, to: Option<RoomKey>) -> bool {
    if !jailed(world, id) || to == Some(jail_room(world)) {
        return false;
    }
    tell_jailed(world, id, false);
    true
}

fn tell_jailed(world: &mut World, id: SessionId, now: bool) {
    let Some(j) = world.session(id).and_then(|s| s.sanctions.jail.clone()) else {
        return;
    };
    let (until, reason) = (until_text(j.until), reason_text(&j.reason));
    if let Some(s) = world.session_mut(id) {
        s.output.push(if now {
            Msg::ModJailedNow { until, reason }
        } else {
            Msg::ModJailed { until, reason }
        });
    }
}

/// Moves a player in the realm to `to` the way the sysop's GOTO does, his fights, drag and
/// party ended first, and shows him the room.
fn move_to(world: &mut World, id: SessionId, to: RoomKey) {
    crate::combat::cleanup_when_user_leaves(world, id);
    crate::movement::move_user::set_user_location(world, id, to, true);
    let verbose = world
        .session(id)
        .and_then(|s| s.player())
        .map_or(0, |p| p.verbose);
    crate::display::display_room(world, id, crate::display::RoomMode::verbose(verbose));
}

fn in_room(world: &World, id: SessionId) -> Option<RoomKey> {
    world
        .session(id)
        .filter(|_| world.is_in_game(id))
        .and_then(|s| s.room_key())
}

/// Puts a jailed player in the realm into the jail room, if he is not there.
fn hold(world: &mut World, id: SessionId) {
    let jail = jail_room(world);
    if in_room(world, id).is_some_and(|r| r != jail) && world.room(jail).is_some() {
        move_to(world, id, jail);
    }
}

/// Ends a player's jail: with `lift_row` the store's row goes (the sysop's own lift has
/// deleted it already), and a player in the jail room is moved to town.
fn release(world: &mut World, id: SessionId, lift_row: bool) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(jail) = s.sanctions.jail.take() else {
        return;
    };
    let account = s.account.as_ref().map(|a| a.id);
    if lift_row && let Some(account) = account {
        world
            .outbox
            .push(EngineOutput::Persist(PersistRequest::LiftSanction {
                account,
                kind: Kind::Jail.as_str(),
                until: jail.until,
            }));
    }
    if world.is_in_game(id) {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::ModReleased);
        }
        if in_room(world, id) == Some(jail_room(world)) && world.room(town(world)).is_some() {
            move_to(world, id, town(world));
        }
        if let Some(s) = world.session_mut(id) {
            s.push_prompt();
        }
    }
}

/// Entering the realm: a jail in force puts the player in the jail room and tells him; one
/// that ended while he was away lets him out.
pub fn on_enter_realm(world: &mut World, id: SessionId) {
    let Some(jail) = world.session(id).and_then(|s| s.sanctions.jail.clone()) else {
        return;
    };
    if jail.active_at(world.wall) {
        hold(world, id);
        tell_jailed(world, id, false);
        if let Some(s) = world.session_mut(id) {
            s.push_prompt();
        }
    } else {
        release(world, id, true);
    }
}

/// Each tick: the mutes and jails whose time is up end.
pub fn on_tick(world: &mut World) {
    let now = world.wall;
    for id in world.session_ids() {
        let Some(s) = world.session(id) else { continue };
        let mute_over = s.sanctions.mute.as_ref().is_some_and(|m| !m.active_at(now));
        let jail_over = s.sanctions.jail.as_ref().is_some_and(|j| !j.active_at(now));
        if mute_over {
            let in_game = world.is_in_game(id);
            if let Some(s) = world.session_mut(id) {
                s.sanctions.mute = None;
                if in_game {
                    s.output.push(Msg::ModUnmuted);
                    s.push_prompt();
                }
            }
        }
        // a jailed player at the menu is let out when he enters the realm
        if jail_over && world.is_in_game(id) {
            release(world, id, true);
        }
    }
}

/// Disconnects the player on `account`, saved as a logoff saves (no hang-up penalty), and
/// tells him why. His name, or `None` when he is not connected.
pub fn kick(world: &mut World, account: AccountId, reason: &str) -> Option<String> {
    let id = session_of(world, account)?;
    let name = name_of(world, id);
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::ModKicked {
            reason: reason_text(reason),
        });
    }
    world.touch(id);
    world.logoff(id);
    Some(name)
}

/// A line from the sysop to the player on `account`, in the game or at the menus. His
/// name, or `None` when he is not connected.
pub fn tell(world: &mut World, account: AccountId, text: &str) -> Option<String> {
    let id = session_of(world, account)?;
    let name = name_of(world, id);
    let in_game = world.is_in_game(id);
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::ModTold {
            text: text.to_string(),
        });
        if in_game {
            s.push_prompt();
        }
    }
    world.touch(id);
    Some(name)
}

/// Sets (`Some`) or lifts (`None`) the mute or jail of the player on `account`, already
/// stored by the caller. His name, or `None` when he is not connected (the stored row
/// takes effect at his next login).
pub fn set(
    world: &mut World,
    account: AccountId,
    kind: Kind,
    sanction: Option<Sanction>,
) -> Option<String> {
    let id = session_of(world, account)?;
    let name = name_of(world, id);
    let in_game = world.is_in_game(id);
    world.touch(id);
    match (kind, sanction) {
        (Kind::Mute, Some(m)) => {
            let msg = Msg::ModMutedNow {
                until: until_text(m.until),
                reason: reason_text(&m.reason),
            };
            if let Some(s) = world.session_mut(id) {
                s.sanctions.mute = Some(m);
                if in_game {
                    s.output.push(msg);
                    s.push_prompt();
                }
            }
        }
        (Kind::Mute, None) => {
            if let Some(s) = world.session_mut(id)
                && s.sanctions.mute.take().is_some()
                && in_game
            {
                s.output.push(Msg::ModUnmuted);
                s.push_prompt();
            }
        }
        (Kind::Jail, Some(j)) => {
            if let Some(s) = world.session_mut(id) {
                s.sanctions.jail = Some(j);
            }
            if in_game {
                tell_jailed(world, id, true);
                hold(world, id);
                if let Some(s) = world.session_mut(id) {
                    s.push_prompt();
                }
            }
        }
        // the caller has deleted the row
        (Kind::Jail, None) => release(world, id, false),
    }
    Some(name)
}

/// Closes the realm: every player but the sysops is told why, saved and disconnected. The
/// login refuses them until it opens (`bin` closes the login first). The number
/// disconnected.
pub fn close_realm(world: &mut World, reason: &str) -> usize {
    let reason = if reason.is_empty() {
        String::new()
    } else {
        format!(": {reason}")
    };
    let mut closed = 0;
    for id in world.session_ids() {
        let Some(s) = world.session(id) else { continue };
        if s.account.as_ref().is_none_or(|a| a.sysop) {
            continue;
        }
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::ModRealmClosed {
                reason: reason.clone(),
            });
        }
        world.touch(id);
        world.logoff(id);
        closed += 1;
    }
    closed
}
