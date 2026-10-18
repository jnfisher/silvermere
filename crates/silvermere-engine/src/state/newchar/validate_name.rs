//! The new-name check, and the `NewCharStep::Validating` and `NewCharStep::Name` states.
//!
//! In the original the check takes about 30 seconds under MBBSEmu. Here the characters
//! online are compared at once, the stored characters through one indexed query
//! (`EngineOutput::NameCheck`, answered with [`on_checked`] from an
//! `EngineInput::Custom`), and the known monsters from the world's table. The messages
//! and the asynchronous completion stay.

use silvermere_text::Msg;
use silvermere_types::enums::{InGame, NewCharStep, SessionState};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::SessionId;

use crate::event_loop::EngineOutput;
use crate::state::newchar::{ENTRY_MASK, capitalise, tell, words};
use crate::state::{Input, Outcome};
use crate::tell::tell_game;
use crate::world::World;

/// Is the upper-cased word a game command? T-024's `commands::parse_command` over the
/// whole table.
pub fn parse_command(word: &str) -> bool {
    crate::commands::parse_command(word).is_some()
}

/// Starts the check for the session's `firstname`: the characters online first, then the
/// stored characters
/// through `EngineOutput::NameCheck`. The session is at [`NewCharStep::Validating`].
pub fn begin(world: &mut World, id: SessionId) {
    let Some(name) = world
        .sessions
        .get(&id)
        .and_then(|s| s.player())
        .map(|p| p.firstname.clone())
    else {
        return;
    };
    let online = world
        .session_ids()
        .into_iter()
        .filter(|&other| other != id)
        .filter_map(|other| world.sessions.get(&other).and_then(|s| s.player()))
        .any(|p| p.firstname.eq_ignore_ascii_case(&name));
    if online {
        on_checked(world, id, true);
        return;
    }
    world.outbox.push(EngineOutput::NameCheck { id, name });
}

/// The answer to `EngineOutput::NameCheck`: `taken` when a stored character has the name.
/// Then the known monsters are compared and the character enters the realm or the player
/// is asked for another name. Ignored when the session is no longer waiting.
pub fn on_checked(world: &mut World, id: SessionId, taken: bool) {
    let Some(s) = world.sessions.get(&id) else {
        return;
    };
    if s.state != SessionState::NewChar(NewCharStep::Validating) {
        return;
    }
    let Some(plr) = s.player() else {
        // no character on the channel
        return;
    };
    let name = plr.firstname.clone();
    let taken = taken
        || world
            .monsters
            .values()
            .any(|m| m.name.eq_ignore_ascii_case(&name));
    if taken {
        // the name is taken (by a character or a monster)
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::NameTaken { name });
            s.output.push(Msg::EnterNewName);
            s.state = SessionState::NewChar(NewCharStep::Name);
        }
        return;
    }
    // count the players in the game, mark the character checked, put it in play and save
    // it
    let max_in_game = usize::from(world.ids.max());
    let cnt = world
        .session_ids()
        .into_iter()
        .filter(|&i| world.is_in_game(i))
        .count();
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    plr.settings.insert(PlayerSettings::VALIDATED);
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    // The character is complete: `save_player` may write it from now on.
    s.created = true;
    // the first save of the record
    if !crate::player::save_at(world, id, crate::player::SavePoint::NameValidated) {
        tracing::warn!(?id, "new character not saved (no account bound)");
    }
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let full = cnt >= max_in_game;
    if full {
        // the game is full (maxingame is max_sessions here)
        s.output.push(Msg::GameFull);
        s.output.push(Msg::MudMenuPromptNormal);
        s.state = SessionState::MainMenu;
        s.ingame = InGame::Module;
    }
    if full {
        return;
    }
    // announce the arrival (invisible sysops are not carried)
    let entered = Msg::PlayerEntered { who: name };
    tell_game(world, Some(id), ENTRY_MASK, true, None, &entered);
    // set up the screen and show the prompt
    // TODO T-025: show_status (the room display) between the backspace and the prompt;
    // TODO T-027: rebuild the user list.
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::Backspace);
        s.push_prompt();
    }
}

/// `NewCharStep::Validating`: input is ignored while the check runs.
pub fn on_validating(world: &mut World, input: Input<'_>) -> Outcome {
    tell(world, input.id, Msg::ValidatingWait);
    Outcome::Continue
}

/// `NewCharStep::Name`: the check found the name taken and the player types another one,
/// or `X` to give up.
pub fn on_new_name(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let Some(first) = w.first() else {
        tell(world, id, Msg::NewNamePrompt);
        return Outcome::Continue;
    };
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    if first.eq_ignore_ascii_case("x") {
        // back to the main menu with no character
        s.output.push(Msg::MudMenuPromptCr);
        s.character = None;
        s.state = SessionState::MainMenu;
        return Outcome::Continue;
    }
    if first.chars().count() > 10 {
        s.output.push(Msg::NameTooLong10);
        s.output.push(Msg::TryAnotherName);
        return Outcome::Continue;
    }
    if parse_command(first) {
        s.output.push(Msg::NameNotAllowed);
        s.output.push(Msg::TryAnotherName);
        return Outcome::Continue;
    }
    // letters only (blanks are stripped first)
    let name = first.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) {
        s.output.push(Msg::NameInvalidNew);
        s.output.push(Msg::TryAnotherName);
        return Outcome::Continue;
    }
    // store it and start the check again
    let Some(plr) = s.character.as_mut() else {
        // (no record: back to the main menu); the Rust session always has the record
        // here, so this is the `X` exit without the prompt
        s.character = None;
        s.state = SessionState::MainMenu;
        return Outcome::Continue;
    };
    plr.firstname = capitalise(name);
    s.output.push(Msg::NameValidatingPlain);
    s.state = SessionState::NewChar(NewCharStep::Validating);
    begin(world, id);
    Outcome::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    // the first name may not be a game command
    #[test]
    fn command_words() {
        assert!(parse_command("SYSOP"));
        assert!(parse_command("sysop"));
        assert!(parse_command("NORTH"));
        assert!(parse_command("NE"));
        // the whole table now: ATTACK's `A` and the open `VERB*` too
        assert!(parse_command("At"));
        assert!(parse_command("Verbena"));
        assert!(!parse_command("KASIMIR"));
    }
}
