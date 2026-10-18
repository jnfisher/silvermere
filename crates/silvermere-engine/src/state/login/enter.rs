//! Entering the realm: `P` or `E` at the `[MAJORMUD]:` menu.

use silvermere_text::{Msg, render_entry_banner};
use silvermere_types::enums::{InGame, SessionState};
use silvermere_types::flags::{PlayerFlags, PlayerStatus2};
use silvermere_types::ids::SessionId;

use crate::display::{RoomMode, display_room};
use crate::player::load_player;
use crate::tell::tell_game;
use crate::world::World;

/// `P`/`E` at the main menu, in the original's order:
///
/// 1. the player cap: `We're sorry, but the game is currently full.` and
///    the bare `[MAJORMUD]:` prompt when [`World::max_in_game`] players (`maxingame`)
///    are in the realm already;
/// 2. no character yet: character creation (`newchar::begin`);
/// 3. T-023's [`crate::player::load_player`] on the session's held row: every repair,
///    notice and derived value of entering the game (the hang-up notice among them).
///    It runs on every `P`/`E`, so the notices come on each entry as in the original; the
///    held row is the one saved at the last return to the menu. A row that will not load
///    (a record version other than 22) is kept, logged, and the menu shown again;
/// 4. a character whose stats were reset (`STATRESET`) opens the stat editor instead
///    (`edit_character_stats`, T-047);
/// 5. `USRFOLED`, into the realm (`Playing`);
/// 6. the tournament notice;
/// 7. the room (T-025) and the prompt;
/// 8. `"%s just entered the Realm."` to the game, filtered by `PF_NOENTRYMSG`, with the
///    listeners' prompts.
///
/// The hang-up notice comes before the banner because `load_player` runs before
/// `USRFOLED` is shown; the `disconnect-punish` capture shows the same.
///
/// Not carried: the database-problem text, the play key, the IP authorisation, the BBS
/// charge, the paying-account and level gates.
/// The player cap keeps the first line of its message only.
pub fn enter_realm(world: &mut World, id: SessionId, today: i64) {
    let in_game = world
        .session_ids()
        .into_iter()
        .filter(|&i| world.is_in_game(i))
        .count();
    if in_game >= usize::from(world.max_in_game) {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::GameFull);
            s.output.push(Msg::MudMenuPromptNormal);
        }
        return;
    }
    let tournament = world.config.tournament_play;
    let Some(s) = world.session_mut(id) else {
        return;
    };
    // No character, so `create_player` and the race list (T-022). A row that is not
    // `created` (one creation left at `X` or `QUIT`) counts as none: as in the original,
    // `E` then starts character creation again.
    let Some(plr) = s.character.as_mut().filter(|_| s.created) else {
        crate::state::newchar::begin(world, id, today);
        return;
    };
    let row = plr.clone();
    if let Err(e) = load_player(world, id, row, today) {
        tracing::warn!(?id, %e, "character not loaded");
        super::show_menu(world, id);
        return;
    }
    let Some(s) = world.session_mut(id) else {
        return;
    };
    // A character whose stats were reset goes through the editor (first name locked)
    // before it enters; finishing the editor brings it into the realm.
    if s.character
        .as_ref()
        .is_some_and(|c| c.status2.contains(PlayerStatus2::STATRESET))
    {
        crate::state::newchar::edit_character_stats(world, id, false);
        return;
    }
    let name = s
        .character
        .as_ref()
        .map(|c| c.firstname.clone())
        .unwrap_or_default();
    s.output.push_raw(render_entry_banner());
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    s.flags.ingame = InGame::Realm;
    // TODO(T-047): the tournament start room for a player with no time in the realm since
    // the last cleanup (no session time recorded).
    if tournament {
        s.output.push(Msg::TournamentNotice);
    }
    // the room, in the player's verbose mode
    let verbose = world
        .session(id)
        .and_then(|s| s.player())
        .map_or(0, |p| p.verbose);
    display_room(world, id, RoomMode::verbose(verbose));
    if let Some(s) = world.session_mut(id) {
        s.push_prompt();
    }
    // The BBS invisible flag is not carried.
    tell_game(
        world,
        Some(id),
        PlayerFlags::NOENTRYMSG,
        true,
        None,
        &Msg::PlayerEntered { who: name },
    );
    crate::combat::check_kill_user(world, id);
    crate::metrics::login();
}
