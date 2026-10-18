//! The suicide password: `SET SUICIDE` and the `SuicideStep::OldPw`, `NewPw` and `Confirm`
//! prompts.
//!
//! The password is `player.suicpw`, 8 characters at most, typed with `*` echoed.
//! `SUICIDE` and `REROLL` with no password set go through without a prompt; `Confirm` and
//! `commit_suicide` are T-068's (`state::suicide`, `player::suicide`).

use silvermere_text::Msg;
use silvermere_types::enums::{SessionState, SuicideStep};
use silvermere_types::ids::SessionId;

use crate::event_loop::{EchoMode, EngineOutput};
use crate::state::newchar::words;
use crate::state::{Input, Outcome};
use crate::world::World;

/// The longest suicide password.
pub const MAX_LEN: usize = 8;

/// `SET SUICIDE`: a player with a password gives the current one first
/// (`SuicideStep::OldPw`), otherwise the new one is asked at once (`SuicideStep::NewPw`);
/// both with `*` echoed, 8 characters at most. No prompt follows. T-028's `cmd_set` calls
/// this.
pub fn start(world: &mut World, id: SessionId) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let has_pw = s.player().is_some_and(|p| !p.suicpw.is_empty());
    if has_pw {
        s.output.push(Msg::SetSuicideCurrentPassword);
        s.state = SessionState::Suicide(SuicideStep::OldPw);
    } else {
        s.state = SessionState::Suicide(SuicideStep::NewPw);
        s.output.push(Msg::SetSuicideNewPassword);
    }
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::Secure {
            ch: b'*',
            width: MAX_LEN as u8,
        },
    });
}

/// One input line in a `Suicide(..)` state.
pub fn on_line(world: &mut World, input: Input<'_>) -> Outcome {
    let Some(s) = world.session(input.id) else {
        return Outcome::Logoff;
    };
    match s.state {
        SessionState::Suicide(SuicideStep::OldPw) => on_old_pw(world, input),
        SessionState::Suicide(SuicideStep::NewPw) => on_new_pw(world, input),
        SessionState::Suicide(SuicideStep::Confirm) => {
            crate::state::suicide::on_confirm(world, input)
        }
        state => Outcome::Unimplemented(state),
    }
}

/// Typed characters are echoed again.
fn echo_on(world: &mut World, id: SessionId) {
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::On,
    });
}

/// `SuicideStep::OldPw`: nothing typed with a password set cancels; the right password (or
/// none set) asks for the new one; a wrong one is refused.
pub fn on_old_pw(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let pw = s.player().map(|p| p.suicpw.clone()).unwrap_or_default();
    if w.is_empty() && !pw.is_empty() {
        s.state = SessionState::Playing;
        s.output.push(Msg::SuicidePasswordNotChangedCaps);
        s.push_prompt();
        echo_on(world, id);
    } else if w.is_empty() || w[0].eq_ignore_ascii_case(&pw) {
        s.state = SessionState::Suicide(SuicideStep::NewPw);
        s.output.push(Msg::SuicideEnterNewPasswordClr);
    } else {
        s.state = SessionState::Playing;
        s.output.push(Msg::SuicideInvalidPasswordBang);
        s.push_prompt();
        echo_on(world, id);
    }
    Outcome::Continue
}

/// `SuicideStep::NewPw`: the new password, 8 characters at most; nothing typed leaves it
/// unchanged.
pub fn on_new_pw(world: &mut World, input: Input<'_>) -> Outcome {
    let id = input.id;
    let w = words(input.line);
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    let Some(first) = w.first() else {
        s.state = SessionState::Playing;
        s.output.push(Msg::SuicidePasswordNotChangedCaps);
        s.push_prompt();
        echo_on(world, id);
        return Outcome::Continue;
    };
    if first.chars().count() > MAX_LEN {
        s.output.push(Msg::SuicidePasswordTooLong);
        s.output.push(Msg::SuicideEnterNewPassword);
        return Outcome::Continue;
    }
    match s.character.as_mut() {
        None => {
            s.state = SessionState::Playing;
            s.output.push(Msg::SuicidePasswordNotChanged);
        }
        Some(plr) => {
            plr.suicpw = (*first).to_string();
            s.state = SessionState::Playing;
            s.output.push(Msg::SuicidePasswordChanged);
            s.push_prompt();
            echo_on(world, id);
        }
    }
    Outcome::Continue
}
