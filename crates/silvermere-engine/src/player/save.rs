//! Saving a character: [`save_player`], [`delete_player`] and the round-robin sweep
//! [`save_characters`].
//!
//! A save is a [`PersistRequest::Character`] on the persistence channel, keyed by account
//! (`characters_per_account` is 1); the persistence worker upserts it. Every account may
//! save: the "unpaid accounts may not save" rule (`PS_PAYING`) is not carried.
//!
//! # Save points
//!
//! A character is saved at these 30 places. Each is a [`SavePoint`]; the card that
//! carries the caller calls [`save_at`] there.
//!
//! | Save point | When |
//! |---|---|
//! | `AddonLife` | an add-on gave a life |
//! | `AddonWealth` | an add-on changed the coins |
//! | `OutOfCreditsSlow` | out of credits, leaving (slow cycle) |
//! | `OutOfCreditsExempt` | out of credits, leaving (exempt-credit deduction) |
//! | `QuitMeditation` | the `QUIT` meditation ended |
//! | `Sweep` | the round-robin sweep ([`save_characters`]) |
//! | `GangLeave` | left a gang (`LEAVE`) |
//! | `GangJoinCommand` | joined a gang (`JOIN`) |
//! | `TrainStats` | `TRAIN STATS`, before the editor opens |
//! | `GiveCoinsGiver`, `GiveCoinsReceiver` | gave coins (giver, then receiver) |
//! | `GiveItemGiver`, `GiveItemReceiver` | gave an item (giver, then receiver) |
//! | `PageToggle` | `PAGE` toggled |
//! | `NameValidated` | the new name was validated |
//! | `EnterRealm` | entered the realm |
//! | `MainMenu` | back at the main menu |
//! | `Logoff` | logoff ([`crate::session::on_logoff`]) |
//! | `HangupFirst` | hang-up, first save |
//! | `HangupLeave` | hang-up, after leaving the realm |
//! | `GangJoin` | joined a gang |
//! | `GangLeaderPaid` | gang leader paid |
//! | `ShareReceiver`, `ShareGroup`, `ShareSharer` | shared coins (each receiver, the group, the sharer) |
//! | `SysopRefreshUsers`, `SysopRefreshNone` | the sysop's user refresh |
//! | `DeathWithLives` | died with lives left |
//! | `BankDeposit` | bank deposit |
//! | `BankWithdraw` | bank withdrawal |
//!
//! Two more points write the record without a full save: the one-time race and class
//! change update the stored record in place from the main menu. Here they are
//! [`SavePoint::RaceChange`] and [`SavePoint::ClassChange`], through the same [`save_at`].
//!
//! The deleting deaths call [`delete_player`] instead, through
//! [`super::profile::retire`].

use silvermere_types::enums::Ability;
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{AccountId, ClassNo, SessionId};
use silvermere_types::records::Character;

use crate::event_loop::{EngineOutput, PersistRequest};
use crate::world::World;

/// `buftosav`: characters saved per sweep. The option (`BUFTOSAV`) is not in
/// silvermere's config; the stock value is kept.
pub const BUFTOSAV: usize = 5;
/// `MAXSAVECHK`: sessions looked at per sweep.
pub const MAXSAVECHK: usize = 300;

/// The places a character is saved (the table in the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SavePoint {
    AddonLife,
    AddonWealth,
    OutOfCreditsSlow,
    OutOfCreditsExempt,
    QuitMeditation,
    Sweep,
    GangLeave,
    GangJoinCommand,
    TrainStats,
    GiveCoinsGiver,
    GiveCoinsReceiver,
    GiveItemGiver,
    GiveItemReceiver,
    PageToggle,
    NameValidated,
    EnterRealm,
    MainMenu,
    Logoff,
    HangupFirst,
    HangupLeave,
    GangJoin,
    GangLeaderPaid,
    ShareReceiver,
    ShareGroup,
    ShareSharer,
    SysopRefreshUsers,
    SysopRefreshNone,
    DeathWithLives,
    BankDeposit,
    BankWithdraw,
    RaceChange,
    ClassChange,
}

impl SavePoint {
    /// Every save point, in the order of the module doc's table.
    pub const ALL: [SavePoint; 32] = [
        SavePoint::AddonLife,
        SavePoint::AddonWealth,
        SavePoint::OutOfCreditsSlow,
        SavePoint::OutOfCreditsExempt,
        SavePoint::QuitMeditation,
        SavePoint::Sweep,
        SavePoint::GangLeave,
        SavePoint::GangJoinCommand,
        SavePoint::TrainStats,
        SavePoint::GiveCoinsGiver,
        SavePoint::GiveCoinsReceiver,
        SavePoint::GiveItemGiver,
        SavePoint::GiveItemReceiver,
        SavePoint::PageToggle,
        SavePoint::NameValidated,
        SavePoint::EnterRealm,
        SavePoint::MainMenu,
        SavePoint::Logoff,
        SavePoint::HangupFirst,
        SavePoint::HangupLeave,
        SavePoint::GangJoin,
        SavePoint::GangLeaderPaid,
        SavePoint::ShareReceiver,
        SavePoint::ShareGroup,
        SavePoint::ShareSharer,
        SavePoint::SysopRefreshUsers,
        SavePoint::SysopRefreshNone,
        SavePoint::DeathWithLives,
        SavePoint::BankDeposit,
        SavePoint::BankWithdraw,
        SavePoint::RaceChange,
        SavePoint::ClassChange,
    ];
}

/// Smash: the classes that keep it and the level from which they may. Class numbers as in
/// the stock WCCCLASS data.
const SMASH: [(u16, u16); 6] = [(1, 22), (2, 20), (3, 25), (4, 27), (11, 27), (14, 22)];
/// Perfect Stealth.
const PERFECT_STEALTH: [(u16, u16); 7] = [
    (6, 25),
    (7, 20),
    (8, 20),
    (9, 25),
    (10, 25),
    (14, 20),
    (15, 27),
];
/// Meditate.
const MEDITATE: [(u16, u16); 10] = [
    (3, 27),
    (4, 23),
    (5, 20),
    (6, 23),
    (9, 27),
    (10, 23),
    (11, 23),
    (12, 20),
    (13, 20),
    (14, 27),
];

fn entitled(table: &[(u16, u16)], class: ClassNo, level: u16) -> bool {
    table.iter().any(|&(c, l)| class.0 == c && level >= l)
}

/// The level-gated abilities, stripped as [`load_player`](super::load_player) and
/// [`save_player`] both do: a Smash, Perfect Stealth or Meditate slot the class may not
/// keep at this level is emptied. Only the ability number is cleared; the value stays.
pub fn strip_level_gated(c: &mut Character) {
    let (class, level) = (c.class, c.level);
    for slot in &mut c.abils {
        let table: &[(u16, u16)] = match slot.0 {
            Ability::Smash => &SMASH,
            Ability::PerfectStealth => &PERFECT_STEALTH,
            Ability::Meditate => &MEDITATE,
            _ => continue,
        };
        if !entitled(table, class, level) {
            slot.0 = Ability::None;
        }
    }
}

/// Saves the session's character. Nothing is written for an empty session, a character
/// not yet created, or one whose name is
/// not validated (`PS_VALIDATED`); otherwise the level-gated abilities are stripped and
/// the record goes to the persistence channel.
/// Returns whether a request was queued.
pub fn save_player(world: &mut World, id: SessionId) -> bool {
    let Some(s) = world.sessions.get_mut(&id) else {
        return false;
    };
    if !s.created {
        return false;
    }
    let Some(account) = s.account.as_ref().map(|a| a.id) else {
        return false;
    };
    // the session's `dragging` is the live one once the record has been loaded into the
    // session (it stays so at the main menu, until `clear_player`); a record saved before that
    // (character creation) keeps its own
    let dragging = s.party.loaded.then(|| s.party.record_dragging());
    let Some(c) = s.character.as_mut() else {
        return false;
    };
    if c.userid.is_empty() || !c.settings.contains(PlayerSettings::VALIDATED) {
        return false;
    }
    strip_level_gated(c);
    if let Some(d) = dragging {
        c.dragging = d;
    }
    let character = Box::new(c.clone());
    queue_character(world, account, character);
    true
}

/// A character write or deletion the engine queued and the store has not acknowledged
/// ([`World::queued_characters`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedCharacter {
    /// Its place among every character write and deletion the engine queued, from 1
    /// ([`World::character_writes`]).
    pub seq: u64,
    /// The record written, `None` for a deletion.
    pub record: Option<Box<Character>>,
}

/// Queues a write of the account's character row and keeps the record as the one a login
/// or an offline reader for the account starts from until the store acknowledges it
/// ([`World::queued_characters`], T-075, T-088).
pub fn queue_character(world: &mut World, account: AccountId, character: Box<Character>) {
    remember(world, account, Some(character.clone()));
    world
        .outbox
        .push(EngineOutput::Persist(PersistRequest::Character {
            account,
            character,
        }));
}

/// Queues the deletion of the account's character row; a login or an offline reader for
/// the account before the deletion lands sees no character ([`World::queued_characters`],
/// T-075, T-088).
pub fn queue_character_delete(world: &mut World, account: AccountId) {
    remember(world, account, None);
    world
        .outbox
        .push(EngineOutput::Persist(PersistRequest::DeleteCharacter {
            account,
        }));
}

fn remember(world: &mut World, account: AccountId, record: Option<Box<Character>>) {
    world.character_writes += 1;
    let seq = world.character_writes;
    world
        .queued_characters
        .insert(account, QueuedCharacter { seq, record });
}

/// The store's acknowledgement (T-088): the first `written` character writes and
/// deletions the engine queued are committed, so a read of the store now sees them. Each
/// account whose last queued write is among them no longer needs its queued record; one
/// with a newer write queued keeps it.
///
/// A login's character is read by the gate before the engine sees the login, so a read
/// made before the commit can still be on its way. Every such login belongs to a session
/// in the login dialogue now (it connected before the read, and so before this
/// acknowledgement): the records are dropped once each of those sessions has logged in
/// (taking its record, [`crate::state::login::logged_in`]) or gone
/// ([`settle_characters_written`]).
///
/// This rests on one ordering: a connection's `Connected` reaches the engine (net and
/// bridge to engine) before the store's acknowledgement of a commit that the gate's read
/// for it preceded (store to engine). The connection is opened before its login dialogue
/// starts, the read comes after the dialogue, and the acknowledgement after the commit.
pub fn on_characters_written(world: &mut World, written: u64) {
    let waiting: Vec<SessionId> = world
        .sessions
        .iter()
        .filter(|(_, s)| in_login_dialogue(s))
        .map(|(id, _)| *id)
        .collect();
    world.characters_written.push((written, waiting));
    settle_characters_written(world);
}

/// A connection the gate is still logging in: no account bound yet, at the state a new
/// or relogging session waits in.
fn in_login_dialogue(s: &crate::player_session::PlayerSession) -> bool {
    s.account.is_none() && s.state == silvermere_types::enums::SessionState::Start
}

/// Applies the latest acknowledgement whose logins in progress are all done (each fast
/// tick and each acknowledgement): the queued records it covers are dropped, and the
/// acknowledgements before it are settled with it.
pub fn settle_characters_written(world: &mut World) {
    let in_dialogue = |id: &SessionId| world.sessions.get(id).is_some_and(in_login_dialogue);
    let Some(i) = world
        .characters_written
        .iter()
        .rposition(|(_, waiting)| !waiting.iter().any(in_dialogue))
    else {
        return;
    };
    let written = world.characters_written[i].0;
    world.characters_written.drain(..=i);
    world.queued_characters.retain(|_, q| q.seq > written);
}

/// What an offline reader takes for the account's character (T-088), so it sees every
/// save already made: the record queued for the account if
/// the store has not acknowledged it, else the stored row it read. `None` when the account
/// has no character, or its deletion is queued.
pub fn queued_or_stored(
    world: &World,
    account: AccountId,
    stored: Option<Character>,
) -> Option<Character> {
    match world.queued_characters.get(&account) {
        Some(q) => q.record.as_deref().cloned(),
        None => stored,
    }
}

/// [`queued_or_stored`] over a store's read of every character with its account: queued
/// records replace their rows, queued deletions drop them, and a queued record whose row
/// the read did not find (a first save still on its way) is added, in account order.
pub fn with_queued(
    world: &World,
    rows: Vec<(AccountId, Character)>,
) -> Vec<(AccountId, Character)> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(AccountId, Character)> = rows
        .into_iter()
        .filter_map(|(account, c)| {
            seen.insert(account);
            queued_or_stored(world, account, Some(c)).map(|c| (account, c))
        })
        .collect();
    let mut added: Vec<(AccountId, Character)> = world
        .queued_characters
        .iter()
        .filter(|(a, _)| !seen.contains(*a))
        .filter_map(|(a, q)| q.record.as_deref().map(|c| (*a, c.clone())))
        .collect();
    added.sort_by_key(|(a, _)| *a);
    out.extend(added);
    out
}

/// [`save_player`] at one of the save points; the site is logged.
pub fn save_at(world: &mut World, id: SessionId, point: SavePoint) -> bool {
    let saved = save_player(world, id);
    tracing::trace!(?id, ?point, saved, "save_player");
    saved
}

/// Deletes the character and clears the session's record: the account's character row is deleted
/// and the session holds no character.
pub fn delete_player(world: &mut World, id: SessionId) {
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let account = s.account.as_ref().map(|a| a.id);
    s.character = None;
    s.created = false;
    // `clear_player` zeroes the whole record, `dragging` with it: whoever is made next on
    // this channel starts from 0, not from this session's value
    s.party = crate::party::Party::default();
    if let Some(account) = account {
        queue_character_delete(world, account);
    }
}

/// The round-robin sweep: from where the last sweep stopped, look at up to [`MAXSAVECHK`]
/// session slots and save up to [`BUFTOSAV`] characters in the game; past the last slot,
/// the next sweep starts again at slot 0. A sweep never wraps within itself.
pub fn save_characters(world: &mut World) {
    let nterms = world.ids.max();
    let (mut saved, mut looked) = (0, 0);
    while looked < MAXSAVECHK && saved < BUFTOSAV && world.save_next < nterms {
        looked += 1;
        let id = SessionId(world.save_next);
        if world.is_in_game(id) {
            save_at(world, id, SavePoint::Sweep);
            saved += 1;
        }
        world.save_next += 1;
    }
    if world.save_next >= nterms {
        world.save_next = 0;
    }
}
