//! Leaving the game: logoff ([`on_logoff`]), hang-up ([`on_disconnect`]),
//! [`cleanup_when_user_leaves`] and account deletion ([`on_account_deleted`]). T-056.
//!
//! The hang-up's punishment itself (the safe room, `PUNHUP`, the hit points and items, the
//! hang-up room, `just disconnected!!!`, `check_kill_user`) is T-021's
//! `state::login::leave::hangup`, on `silvermere_rules::hangup`. A hang-up then leaves as a
//! logoff does: `cleanup_when_user_leaves`, the save, the channel freed.

use silvermere_rules::evil::{Legal, legal_level};
use silvermere_types::enums::{InGame, MonsterType};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{AccountId, ActiveMonsterId, ItemNo, SessionId};
use silvermere_types::records::Character;
use silvermere_types::sizes::{NINVITEMS, NKEYS};

use std::sync::mpsc::Sender;

use crate::commands::parse::eq_ignore_case;
use crate::event_loop::{EngineOutput, PersistRequest};
use crate::monsters::active::remove_active_monster;
use crate::monsters::cache::MONFREE;
use crate::world::World;

/// The strength of a character still being created.
const STARTSTAT: u16 = 5;

/// For a fully created character (a user-id, strength no longer `STARTSTAT`, the name
/// validated), in this order: whoever fights him stops, he stops dragging, his followers
/// stop following, his gang invitations go, his angels are dismissed, his pending commands
/// are thrown away and his own combat ends.
pub fn cleanup_when_user_leaves(world: &mut World, id: SessionId) {
    let created = world
        .session(id)
        .and_then(|s| s.character.as_ref())
        .is_some_and(|p| {
            !p.userid.is_empty()
                && p.str != STARTSTAT
                && p.settings.contains(PlayerSettings::VALIDATED)
        });
    if !created {
        return;
    }
    // Whoever fights him stops
    crate::combat::autocombat::kill_autocombat_against(world, id);
    // Whoever drags him lets go
    crate::party::stop_dragging_user(world, id);
    // Every follower is told, and he leaves his party
    crate::party::stop_following_all(world, id);
    // His gang invitations go: the session id is reused, so its invitations must not
    // outlive it
    crate::gangs::clear_gang_invitations(world, id, None);
    dismiss_users_angels(world, id);
    // His pending commands are thrown away
    if let Some(s) = world.session_mut(id) {
        s.ust.pending.clear();
    }
    // His own combat ends
    crate::combat::autocombat::kill_autocombat(world, id);
}

/// For a Villain or a FIEND, every monster in the active-monster cache (slots `0..monsbuf`
/// in slot order; one not resident is not seen) that is not `friendly`, has `montype` 37
/// (`MTYPE_ANGEL`) and follows him by first name (full, case not counting) leaves the game
/// without a kill (`remove_active_monster` with `settime`, which prints nothing). The type
/// is named "Angel" in the type table, but in the stock data it is the guardsmen, the
/// bounty hunter, mercenaries and demon imps; we keep the original's rule as it is. The
/// slot test skips an empty slot (number 0) and `MONFREE` (0xFFFF).
fn dismiss_users_angels(world: &mut World, id: SessionId) {
    let Some((evil, firstname)) = world
        .session(id)
        .and_then(|s| s.character.as_ref())
        .map(|c| (c.evil, c.firstname.clone()))
    else {
        return;
    };
    if !matches!(legal_level(evil), Legal::Villain | Legal::Fiend) {
        return;
    }
    // the cache's borrow ends before any look-up or removal
    let resident: Vec<ActiveMonsterId> = {
        let cache = world.active_monsters.cache();
        (0..cache.monsbuf())
            .map(|slot| cache.number_at(slot))
            .filter(|&n| n != 0 && n != MONFREE)
            .map(|n| ActiveMonsterId(n as i32))
            .collect()
    };
    for n in resident {
        let angel = world.active_monsters.peek(n).is_some_and(|m| {
            m.friendly == 0
                && m.montype == MonsterType::Angel
                && eq_ignore_case(&m.following, &firstname)
        });
        if angel {
            remove_active_monster(world, n, true);
        }
    }
}

/// Forget list cleared, `cleanup_when_user_leaves`, the character saved and cleared, the
/// player marked outside the game. The session is removed and its id freed. A handler asks
/// for it with `World::logoff`, which keeps the session until its last write and `Close`
/// have gone out; the loop calls this at the end of the flush. Nothing is punished.
pub fn on_logoff(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.ust.forgets.clear();
    }
    cleanup_when_user_leaves(world, id);
    let saved = crate::player::save_at(world, id, crate::player::SavePoint::Logoff);
    // the id may be handed out again: nobody may go on fighting it, whatever its record,
    // and its auto-combat entry starts afresh
    crate::combat::autocombat::kill_autocombat_against(world, id);
    crate::combat::autocombat::forget(world, id);
    let account = world
        .session(id)
        .and_then(|s| s.account.as_ref())
        .map(|a| a.id);
    // a daily cleanup waiting for the store uses the record just saved, not the stored
    // row, which may not have caught up with it
    let record = saved
        .then(|| world.session(id).and_then(|s| s.character.clone()))
        .flatten();
    crate::cleanup::note_departure(world, account, record);
    if let Some(mut s) = world.sessions.remove(&id) {
        s.character = None;
        s.ingame = InGame::Outside;
        world.ids.free(id);
    }
    world.touched.retain(|t| *t != id);
    // T-069: an account being deleted is read back after this save
    if let Some(account) = account
        && world.account_deletes.contains_key(&account)
    {
        world
            .outbox
            .push(EngineOutput::AccountDeleteScan { account });
    }
}

/// The connection is gone. A player in the realm is dealt with by
/// `state::login::leave::hangup` (the safe room, the `PUNHUP` punishment, the hang-up room,
/// `just disconnected!!!`, `check_kill_user`, the save); then, for a fully created
/// character, `cleanup_when_user_leaves` and the save, which is
/// [`on_logoff`].
pub fn on_disconnect(world: &mut World, id: SessionId) {
    crate::state::login::leave::hangup(world, id);
    on_logoff(world, id);
}

/// An account was deleted, and its character (`stored`, not in a session) goes with it.
/// `DELUSER` is not an option in silvermere: accounts own their characters, so this is
/// always the original's `YES` (`docs/config-defaults.md`). The evil points, bank books and
/// the character row go with the account row (`ON DELETE CASCADE`); here the engine's
/// copies go, every carried item is handed back (a limited copy is released), the gang
/// counts one member fewer and is flagged disbanded if he led it, and the character row is
/// deleted.
pub fn on_account_deleted(world: &mut World, account: AccountId, stored: Option<Character>) {
    tracing::info!(target: "mmlog", account = account.0, "Deleting Mud user");
    // The evil points and the bankbooks
    world.evil_points.remove(&account);
    world.bank_books.retain(|(a, _), _| *a != account);
    let Some(mut c) = stored else {
        return;
    };
    // Every inventory item, then every key
    for i in 0..NINVITEMS {
        let item = c.items.get(i).copied().unwrap_or(ItemNo(0));
        if item != ItemNo(0) {
            crate::items::inventory::remove_item_from_record(world, &mut c, item);
        }
    }
    for i in 0..NKEYS {
        let item = c.keys.get(i).copied().unwrap_or(ItemNo(0));
        if item != ItemNo(0) {
            crate::items::inventory::remove_item_from_record(world, &mut c, item);
        }
    }
    crate::gangs::on_character_deleted(world, &c);
    // The WCCUSERS record
    crate::player::queue_character_delete(world, account);
}

/// The answer to `AdminRequest::DeleteAccount` (T-069).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountDeletion {
    /// The engine has run [`on_account_deleted`] and queued the row deletion;
    /// `character` is the first name of the character that went with it.
    Queued { character: Option<String> },
    /// No account has that id.
    NotFound,
    /// The store could not read the account.
    Failed(String),
}

/// The store's read of an account being deleted, after a flush (T-069).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountScan {
    /// No account row has that id.
    NoAccount,
    /// The account exists; its stored character, if it has one.
    Found(Option<Box<Character>>),
    /// The flush or the read failed.
    Failed(String),
}

/// The sessions playing on `account`.
fn sessions_of(world: &World, account: AccountId) -> Vec<SessionId> {
    world
        .session_ids()
        .into_iter()
        .filter(|&s| {
            world
                .session(s)
                .and_then(|s| s.account.as_ref())
                .is_some_and(|a| a.id == account)
        })
        .collect()
}

/// `DELETE /v1/accounts/{id}` (T-069): deletes `account` through the engine, so that
/// nothing the engine holds for it is written back after the rows are gone. A session on
/// the account is logged off by the normal path ([`on_logoff`] saves it); then, or at
/// once when nobody plays on it, the store is asked to read the account and its stored
/// character after a flush ([`EngineOutput::AccountDeleteScan`]), and
/// [`on_account_scan`] finishes the deletion. A second request for an account already
/// being deleted waits for the same answer.
pub fn delete_account(world: &mut World, account: AccountId, reply: Sender<AccountDeletion>) {
    let waiting = world.account_deletes.entry(account).or_default();
    waiting.push(reply);
    if waiting.len() > 1 {
        return;
    }
    let online = sessions_of(world, account);
    if online.is_empty() {
        world
            .outbox
            .push(EngineOutput::AccountDeleteScan { account });
    }
    for id in online {
        world.logoff(id);
    }
}

/// The store's answer to [`EngineOutput::AccountDeleteScan`]. If the account logged in
/// again while the store was reading, that session is logged off and the store asked
/// again. Otherwise [`on_account_deleted`] runs with the stored character, the account
/// row's deletion is queued behind its writes ([`PersistRequest::DeleteAccount`]) and
/// the requests are answered.
pub fn on_account_scan(world: &mut World, account: AccountId, scan: AccountScan) {
    let Some(replies) = world.account_deletes.remove(&account) else {
        tracing::warn!(account = account.0, "account scan with no deletion waiting");
        return;
    };
    let answer = match scan {
        AccountScan::NoAccount => AccountDeletion::NotFound,
        AccountScan::Failed(e) => {
            tracing::error!(account = account.0, %e, "account deletion: the store read failed");
            AccountDeletion::Failed(e)
        }
        AccountScan::Found(stored) => {
            // T-088: a record queued since the store read stands for the row
            let stored =
                crate::player::queued_or_stored(world, account, stored.map(|c| *c)).map(Box::new);
            let online = sessions_of(world, account);
            if !online.is_empty() {
                world.account_deletes.insert(account, replies);
                for id in online {
                    world.logoff(id);
                }
                return;
            }
            // a daily cleanup waiting for the store must not bring the record back
            if let Some(departed) = world.cleanup.pending.as_mut() {
                departed.remove(&account);
            }
            // T-072: a login already past the store's read must not bring it back
            world.deleted_accounts.insert(account);
            let character = stored.as_ref().map(|c| c.firstname.clone());
            on_account_deleted(world, account, stored.map(|c| *c));
            world
                .outbox
                .push(EngineOutput::Persist(PersistRequest::DeleteAccount(
                    account,
                )));
            AccountDeletion::Queued { character }
        }
    };
    for reply in replies {
        let _ = reply.send(answer.clone());
    }
}
