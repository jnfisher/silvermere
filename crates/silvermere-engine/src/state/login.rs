//! `Start` and `Enter`: the session from the moment its user is logged in until it reaches
//! the `[MAJORMUD]:` menu, and the BBS menu shim before it.
//!
//! The Worldgroup login and signup dialogue is `silvermere-auth`'s; `bin` runs it and
//! sends [`EngineInput::LoggedIn`](crate::EngineInput::LoggedIn), which lands in
//! [`logged_in`]. From there:
//!
//! - with `compat.bbs_menu` on (the default), the session sits in [`SessionState::Start`]
//!   at MBBSEmu's BBS main menu (`Make your selection (X to exit):`): `M` selects the
//!   module, `X` asks `Are you sure (Y/N, or R to re-logon)?`. `Start` is where a
//!   player of the original is before first selecting the module: on the BBS, outside the
//!   module;
//! - selecting the module runs the module entry ([`enter_module`]), which prints the menu
//!   and moves to `MainMenu`. With the shim off, [`logged_in`] runs it at once.
//!
//! [`enter`] is the realm entry (`P`/`E`), [`leave`] everything that takes a player out:
//! `X` in the realm, `X` at the menu, the hang-up and the inactivity hang-up.

pub mod enter;
pub mod leave;

use silvermere_text::{MenuInput, Msg, render_login_greeting, render_menu};
use silvermere_types::enums::SessionState;
use silvermere_types::ids::SessionId;
use silvermere_types::records::Character;

use crate::player_session::{AccountRef, AutoCombat};
use crate::state::{Input, Outcome};
use crate::world::World;

/// What `bin` hands the engine once the login dialogue has authenticated the user: the
/// account, its character as stored (none before character creation, T-022) and the
/// permissions the menu entry reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub account: AccountRef,
    /// The account's character, as `load_player` would read it (T-023 owns the load).
    pub character: Option<Box<Character>>,
    /// The gossip key: `Perm::Gossip` (the account's `no_gossip` flag unset).
    pub may_gossip: bool,
    /// The auction key: `Perm::Auction`.
    pub may_auction: bool,
    /// The `SYS_*` INI keys and the developer gate the account holds (T-055).
    pub sysop_keys: crate::commands::sysop::Keys,
}

/// The user behind `id` has logged in. The channel's game state is reset (`clear_player`,
/// pending lines, inviter, forget list, editor state), then, as the original prints them after
/// a BBS logon, `USRLOGIN` with the short version and the tournament notice;
/// then the BBS menu shim, or the module menu when `compat.bbs_menu` is off. A login for an
/// account deleted in this run (`World::deleted_accounts`, T-072) is closed instead.
pub fn logged_in(world: &mut World, id: SessionId, login: Login) {
    // T-072: the account was deleted in this run, after the gate read its character
    if world.deleted_accounts.contains(&login.account.id) {
        tracing::info!(
            account = login.account.id.0,
            "login for a deleted account; closing"
        );
        if let Some(s) = world.session_mut(id) {
            s.output
                .push_raw(&b"\r\nThis account has been deleted.\r\n"[..]);
        }
        world.logoff(id);
        return;
    }
    // T-075: the record the engine queued last for the account wins over the gate's
    // read, which may have come before the store wrote it
    let character = match world.queued_characters.remove(&login.account.id) {
        Some(queued) => queued.record,
        None => login.character,
    };
    let short_version = world.compat.short_version.clone();
    let tournament = world.config.tournament_play;
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.reset_for_logon();
    s.flags.sysop = login.account.sysop;
    s.flags.gameop = login.account.gameop;
    s.sysop_keys = login.sysop_keys;
    s.account = Some(login.account);
    s.character = character.map(|c| *c);
    // A stored row is a finished character (T-023: `save_player` writes only those).
    s.created = s.character.is_some();
    s.ust
        .flags
        .remove(silvermere_types::flags::SessionFlags::INACTWARN);
    s.state = SessionState::Start;
    s.bbs_confirm = false;
    // Sysops and game operators have their own rights, so the gossip and auction flags are
    // set only for the others.
    let staff = s.flags.sysop || s.flags.gameop;
    s.flags.hasgoss = login.may_gossip && !staff;
    s.flags.hasauct = login.may_auction && !staff;
    s.output.push_raw(render_login_greeting(&short_version));
    if tournament {
        s.output.push(Msg::TournamentNotice);
    }
    if world.compat.bbs_menu {
        bbs_menu(world, id);
    } else {
        enter_module(world, id);
    }
}

/// The BBS main menu of the shim (MBBSEmu's, byte for byte as `entry-existing` shows it).
pub fn bbs_menu(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.bbs_confirm = false;
        s.state = SessionState::Start;
        s.output.push(Msg::BbsMenu);
    }
}

/// One input line in a state this module owns.
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    let Some(s) = world.session(input.id) else {
        return Outcome::Logoff;
    };
    match s.state {
        // `Enter` is never waited in: it runs in the call that set it.
        SessionState::Enter => {
            enter_module(world, input.id);
            Outcome::Continue
        }
        _ if s.account.is_none() => Outcome::Continue,
        _ if s.bbs_confirm => bbs_confirm_line(world, input),
        _ => bbs_menu_line(world, input),
    }
}

/// The first character of the first word, upper case.
pub(crate) fn key(line: &str) -> Option<char> {
    line.split_whitespace()
        .next()
        .and_then(|w| w.chars().next())
        .map(|c| c.to_ascii_uppercase())
}

fn bbs_menu_line(world: &mut World, input: Input<'_>) -> Outcome {
    match key(input.line) {
        Some('M') => {
            if let Some(s) = world.session_mut(input.id) {
                s.output.push(Msg::BbsEnterModule);
            }
            enter_module(world, input.id);
        }
        Some('X') => {
            if let Some(s) = world.session_mut(input.id) {
                s.output.push(Msg::BbsLogoffWarning);
                s.bbs_confirm = true;
            }
        }
        _ => bbs_menu(world, input.id),
    }
    Outcome::Continue
}

fn bbs_confirm_line(world: &mut World, input: Input<'_>) -> Outcome {
    match key(input.line) {
        Some('Y') => {
            if let Some(s) = world.session_mut(input.id) {
                s.output.push(Msg::BbsGoodbye);
            }
            leave::save_player(world, input.id);
            Outcome::Logoff
        }
        Some('R') => {
            log_on_again(world, input.id);
            Outcome::Continue
        }
        Some('N') => {
            bbs_menu(world, input.id);
            Outcome::Continue
        }
        _ => {
            if let Some(s) = world.session_mut(input.id) {
                s.output.push(Msg::BbsLogoffWarning);
            }
            Outcome::Continue
        }
    }
}

/// `R` at the logoff question: out of the account without hanging up. The character is
/// saved, the session forgets its account, and the transport side is told to run the
/// login dialogue again ([`EngineOutput::ReturnToLogin`](crate::EngineOutput::ReturnToLogin)).
pub fn log_on_again(world: &mut World, id: SessionId) {
    leave::save_player(world, id);
    if let Some(s) = world.session_mut(id) {
        s.reset_for_logon();
        s.account = None;
        s.flags.sysop = false;
        s.flags.gameop = false;
        s.state = SessionState::Start;
        s.bbs_confirm = false;
    }
    world
        .outbox
        .push(crate::event_loop::EngineOutput::ReturnToLogin { id });
}

/// The `[MAJORMUD]:` menu as the original draws it for this session.
pub fn menu_bytes(world: &World, id: SessionId) -> Vec<u8> {
    let sysop = world.session(id).is_some_and(|s| s.flags.sysop);
    let race_change = crate::state::newchar::race_change::change_open(world, id).map(|m| {
        silvermere_text::RaceChange {
            class_too: world.config.enable_class_change,
            minutes_left: m,
        }
    });
    render_menu(MenuInput {
        version: &world.compat.mud_version,
        race_change,
        sysop,
        tournament: world.config.tournament_play,
    })
}

/// The `[MAJORMUD]:` menu followed by an ANSI reset.
pub fn show_menu(world: &mut World, id: SessionId) {
    let menu = menu_bytes(world, id);
    if let Some(s) = world.session_mut(id) {
        s.output.push_raw(menu);
        s.output.push(Msg::AnsiNormal);
    }
}

/// `Start`/`Enter`: the player has selected MajorMUD. The channel's auto-combat state is
/// cleared, the cached key tests are made at [`logged_in`], the menu is printed between two
/// ANSI resets and the next line is a menu choice.
///
/// Not carried: the developer-mode `MUDCHARINFO` request and its client-info state, the
/// host's terminal setup, the credit rate and no-pay key, the `save_player`/`clear_player`
/// pair (the session keeps the character it was given at login, so `P`/`E` need no database
/// read; T-023 decides the reload).
pub fn enter_module(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.autocmb = AutoCombat::default();
        s.ust
            .flags
            .remove(silvermere_types::flags::SessionFlags::CHARMODE);
        s.ust.echocol = 0;
        s.output.push(Msg::AnsiNormal);
    }
    show_menu(world, id);
    if let Some(s) = world.session_mut(id) {
        s.state = SessionState::MainMenu;
    }
}
