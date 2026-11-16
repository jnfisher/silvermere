//! The engine's lifecycle entry points: the BBS host's module callbacks
//! as functions on the world.
//!
//! | Callback | Here | Notes |
//! |---|---|---|
//! | `lonrou` | [`on_login`] | the channel starts with a clean game state |
//! | `sttrou` | [`on_line`] | one `match` arm per state |
//! | `stsrou` 240 | [`on_cycle`] | the per-user cycle; T-037 |
//! | `huprou` | [`on_disconnect`] | the hang-up rules (T-021) |
//! | `lofrou` | [`on_logoff`] | save, clear, leave |
//! | `mcurou` | [`on_daily_cleanup`] | T-056 |
//! | `finrou` | [`on_shutdown`] | T-059 |
//!
//! | `dlarou` | [`on_account_deleted`] | T-056 |
//!
//! `injrou` is not carried (T-013). The leaving callbacks are in
//! [`hangup`].

pub mod hangup;

use std::time::Instant;

use silvermere_types::enums::{InGame, NewCharStep, SessionState};
use silvermere_types::ids::SessionId;

use crate::player_session::{Caps, PlayerSession};
use crate::state::newchar::form;
use crate::state::{self, Input, Outcome};
use crate::world::World;

pub use hangup::{
    AccountDeletion, AccountScan, cleanup_when_user_leaves, delete_account, on_account_deleted,
    on_account_scan, on_disconnect, on_logoff,
};

/// A connection arrived: allocates a dense id and the session. `Err` when every slot is
/// in use (the caller sends the full message and closes).
pub fn on_connect(
    world: &mut World,
    caps: Caps,
    now: Instant,
) -> Result<SessionId, crate::ids::ServerFull> {
    let id = world.ids.alloc()?;
    let caps = Caps {
        utf8: caps.utf8 || world.compat.utf8,
        ..caps
    };
    let mut session = PlayerSession::new(id, caps, now);
    session.ingame = InGame::Module;
    world.sessions.insert(id, session);
    on_login(world, id);
    Ok(id)
}

/// The login callback minus the BBS login line: the channel
/// starts with a clean game state (`clear_player`, no pending lines, no inviter, no
/// forget list, no editor session) at [`SessionState::Start`]. The login dialogue itself
/// is `silvermere-auth`'s (T-021).
pub fn on_login(world: &mut World, id: SessionId) {
    if let Some(s) = world.sessions.get_mut(&id) {
        s.reset_for_logon();
        s.state = SessionState::Start;
    }
}

/// The state dispatch of the input callback: the line goes to the module that owns the
/// session's state. The global-command pass (the host's `globs[]`) is not carried: there
/// are no other modules.
///
/// `rng` and `today` (the UTC midnight, [`crate::event_loop::day_start`]) come from the
/// engine for the handlers that need them: entering the realm runs `load_player`.
pub fn on_line(world: &mut World, id: SessionId, line: &str, today: i64) -> Outcome {
    let Some(s) = world.sessions.get_mut(&id) else {
        return Outcome::Logoff;
    };
    // Typing clears the inactivity mark (as do running a command and entering the realm).
    s.ust
        .flags
        .remove(silvermere_types::flags::SessionFlags::INACTWARN);
    let state = s.state;
    let input = Input { id, line };
    match state {
        SessionState::Start | SessionState::Enter => state::login::on_line(world, input),
        SessionState::MainMenu | SessionState::Help => state::menu::on_line(world, input, today),
        SessionState::NewChar(_) | SessionState::RaceChange(_) => {
            state::newchar::on_line(world, input)
        }
        SessionState::SysopMenu(_) => state::sysop_menu::on_line(world, input),
        SessionState::Playing => state::play::on_line(world, input),
        SessionState::Listing | SessionState::UserReport | SessionState::Disband => {
            state::listing::on_line(world, input)
        }
        SessionState::Suicide(_) => state::suicide::on_line(world, input),
    }
}

/// The empty lines the attack interrupt handed out ([`World::injected`]), read as typed:
/// the outcomes, by session.
pub fn read_injected(world: &mut World, today: i64) -> Vec<(SessionId, Outcome)> {
    let mut outcomes = Vec::new();
    while !world.injected.is_empty() {
        for id in std::mem::take(&mut world.injected) {
            outcomes.push((id, on_line(world, id, "", today)));
        }
    }
    outcomes
}

/// One keystroke in character mode (the host's status `INBLK` in the status callback).
/// The line editor is the transport's; the paged
/// lists end on any key (`commands::input::on_key`, T-024); the stat editor's form takes
/// its keys while it is open (T-063).
pub fn on_key(world: &mut World, id: SessionId, byte: u8) -> Outcome {
    let form = world.session(id).is_some_and(|s| {
        s.state == SessionState::NewChar(NewCharStep::Stats)
            && s.editor.as_ref().is_some_and(|e| e.form.is_some())
    });
    if form {
        return form::on_key(world, id, byte);
    }
    crate::commands::input::on_key(world, id, byte)
}

/// Keys arriving together in character mode, as [`on_key`] for each, but the stat
/// editor's form does their work once, after the last.
pub fn on_keys(world: &mut World, id: SessionId, bytes: &[u8]) -> Outcome {
    let form = world.session(id).is_some_and(|s| {
        s.state == SessionState::NewChar(NewCharStep::Stats)
            && s.editor.as_ref().is_some_and(|e| e.form.is_some())
    });
    if form {
        return form::on_keys(world, id, bytes);
    }
    let mut outcome = Outcome::Continue;
    for &b in bytes {
        outcome = crate::commands::input::on_key(world, id, b);
    }
    outcome
}

/// The per-user cycle (status 240 in the status callback). T-037 fills it.
pub fn on_cycle(_world: &mut World, _id: SessionId) {}

/// The daily cleanup callback on demand (`EngineInput::DailyCleanup`): the same
/// run [`crate::scheduler::Routine::DailyCleanup`] starts at `game.cleanup.at`, without
/// moving the next scheduled one. See [`crate::cleanup`].
pub fn on_daily_cleanup(world: &mut World) {
    crate::cleanup::begin(world);
}

/// `finrou`: the shutdown hook. T-059 saves every character and flushes writes.
pub fn on_shutdown(_world: &mut World) {}
