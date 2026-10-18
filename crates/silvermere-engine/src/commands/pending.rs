//! The typed-ahead queue, the command delay and flood control.
//!
//! The queue ([`commands_pending`], [`execute_pending_command`], [`add_delayed_command`]), the
//! delay ([`add_delay`], [`delay_done`]) and the delay part of the fast tick
//! ([`fast_update_delay`]). The queue is the session's `ust.pending`
//! (`PlayerSession::push_pending`/`pop_pending`, 40 slots). The delay is `Character::delay`, in
//! fast ticks.
//!
//! Echo: in the original a line typed during the delay is not shown as it is typed (the queued
//! line is printed when it runs), until the queue is empty again. Here that is
//! [`EngineOutput::SetEcho`] to the transport, which echoes at receive time otherwise.

use silvermere_text::Msg;
use silvermere_types::enums::DebugLevel;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::SessionId;

use crate::commands::{input, movement};
use crate::event_loop::{EchoMode, EngineOutput};
use crate::state::login::leave;
use crate::world::World;

/// Links walked before "slow down".
pub const PENDWARN: usize = 8;
/// Links walked before the line is dropped.
pub const PENDMAX: usize = 12;
/// The most of a line that is queued (the queue node's text buffer less its terminator).
pub const PENDTEXT: usize = 254;

/// Lines wait in the queue.
pub fn commands_pending(world: &World, id: SessionId) -> bool {
    world.session(id).is_some_and(|s| !s.ust.pending.is_empty())
}

/// The line goes on the end of the queue, unshown. The walk to the end counts the links it follows:
/// at the 8th (`PENDWARN`, so nine lines already waiting) the player is told to slow down, and at
/// the 12th (`PENDMAX`, thirteen waiting) the line is dropped with a second message. Both messages
/// print when a line is dropped, because the walk passes 8 before 12.
pub fn add_delayed_command(world: &mut World, id: SessionId, line: &str) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let waiting = s.ust.pending.len();
    if waiting > 0 {
        let links = waiting - 1;
        if links >= PENDWARN {
            s.output.push(Msg::SlowDown);
        }
        if links >= PENDMAX {
            s.output.push(Msg::TooFast);
            return;
        }
    }
    // at most PENDTEXT bytes of the line are kept
    let mut text = line.to_string();
    if text.len() > PENDTEXT {
        let mut cut = PENDTEXT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    // The queue can never hold more than 13 lines (the drop above), so the 40-slot cap is never
    // reached.
    s.push_pending(text);
}

/// The oldest queued line is shown as if typed now (`Pend:` too for a sysop with debugging on) and
/// run through [`input::execute_input`] as a queued line; when the queue is then empty the input
/// echo comes back on.
pub fn execute_pending_command(world: &mut World, id: SessionId) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    if let Some(line) = s.pop_pending() {
        let debug = s.flags.debug != DebugLevel::Off;
        s.output.push(Msg::PendEcho { line: line.clone() });
        if debug {
            s.output.push(Msg::PendDebug { line: line.clone() });
        }
        input::execute_input(world, id, &line, true);
        if commands_pending(world, id) {
            return;
        }
    }
    echo_on(world, id);
}

/// The input echo back on: the player sees what he types again.
pub fn echo_on(world: &mut World, id: SessionId) {
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::On,
    });
}

/// `delay` ticks more before the next command runs, with the input echo off while the player waits.
pub fn add_delay(world: &mut World, id: SessionId, delay: u8) {
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return;
    };
    plr.delay = plr.delay.saturating_add(delay);
    world.outbox.push(EngineOutput::SetEcho {
        id,
        mode: EchoMode::Off,
    });
}

/// The delay is 0; `false` with no character.
pub fn delay_done(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .and_then(|s| s.character.as_ref())
        .is_some_and(|p| p.delay == 0)
}

/// The delay and queue part of `fast_update_character`, once per fast tick for a session in the
/// game. The delay counts down one per tick; at zero the input is let through again (echo on when
/// nothing waits), the queued move runs (`movedir`, T-026's `move_user`; followers on lower
/// session ids move in the same tick, T-043), then the next typed-ahead line, or else the prompt
/// is shown again. With no delay running, one queued line runs per tick. A player waiting to exit
/// (`ST_EXITING`) is handed to `state::login::leave::exit_countdown` before the delay branch, so
/// the queued move and the "no delay running" drain never run for an exiting player here; stage 1
/// refuses every line during the wait, so nothing observable differs from the original. T-037 owns
/// the rest of `fast_update_character`.
pub fn fast_update_delay(world: &mut World, id: SessionId) {
    let Some(plr) = world.session(id).and_then(|s| s.character.as_ref()) else {
        return;
    };
    if plr.status.contains(PlayerStatus::EXITING) {
        leave::exit_countdown(world, id);
        return;
    }
    if plr.delay == 0 {
        // no delay running: the next typed-ahead command runs now
        if commands_pending(world, id) {
            execute_pending_command(world, id);
        }
        return;
    }
    let movedir = plr.movedir;
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return;
    };
    plr.delay -= 1;
    if plr.delay != 0 {
        return;
    }
    // let input through again, and echo it if nothing waits
    if !commands_pending(world, id) {
        echo_on(world, id);
    }
    if movedir != -1 {
        // a move was waiting
        let moved = movement::queued_move(world, id, movedir);
        if let Some(s) = world.session_mut(id) {
            s.push_prompt();
        }
        if moved {
            movement::followers_move(world, id, movedir);
        }
        if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
            plr.movedir = -1;
        }
        if commands_pending(world, id) {
            execute_pending_command(world, id);
        }
    } else if commands_pending(world, id) {
        // nothing was waiting: run a typed-ahead command
        execute_pending_command(world, id);
    } else if let Some(s) = world.session_mut(id) {
        // or just show the prompt again
        s.push_prompt();
    }
}
