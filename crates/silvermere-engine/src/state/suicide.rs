//! `Suicide(..)`: SET SUICIDE and SUICIDE password prompts.
//!
//! The two `SET SUICIDE` steps live in `newchar::suicide_pw` (T-022: the suicide password
//! is set right after creation); `SuicideStep::Confirm`, the password typed for `SUICIDE`
//! and `REROLL`, is here (T-068) and ends in `player::suicide::commit_suicide`.

use silvermere_types::enums::{SessionState, SuicideStep};

use crate::event_loop::{EchoMode, EngineOutput};
use crate::player::suicide::commit_suicide;
use crate::state::newchar::{suicide_pw, words};
use crate::state::{Input, Outcome};
use crate::world::World;
use silvermere_text::Msg;

/// One input line in a state this module owns.
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    match world.session(input.id).map(|s| s.state) {
        Some(SessionState::Suicide(SuicideStep::Confirm)) => on_confirm(world, input),
        _ => suicide_pw::on_line(world, input),
    }
}

/// `SuicideStep::Confirm`: the password typed for `SUICIDE`/`REROLL`. No password set and
/// none typed, or a match (case not counting), carries the suicide out; anything else is
/// `Invalid password specified.` and back to the realm.
pub fn on_confirm(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let pw = s.player().map(|p| p.suicpw.clone()).unwrap_or_default();
    let wrong = (w.is_empty() && !pw.is_empty())
        || (!w.is_empty() && !w[0].eq_ignore_ascii_case(&pw))
        || (!w.is_empty() && pw.is_empty());
    // typed characters are echoed again either way
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::On,
    });
    let today = world.today;
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    if wrong {
        s.state = SessionState::Playing;
        s.output.push(Msg::SuicideInvalidPassword);
        s.push_prompt();
        return Outcome::Continue;
    }
    let gone = commit_suicide(world, id, today).gone();
    if let Some(s) = world.session_mut(id) {
        if gone {
            s.state = SessionState::MainMenu;
        } else {
            s.push_prompt();
            s.state = SessionState::Playing;
        }
    }
    Outcome::Continue
}
