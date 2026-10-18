//! [`execute_input`]: the eleven stages a line typed in the realm goes through, ordered so that
//! what a player sees matches the original, and the key handler.
//!
//! | Stage | What | Where |
//! |---|---|---|
//! | 1 | waiting to exit: every line but `break` is refused | `state::login::leave` |
//! | 2 | a delay running or lines queued: the line is queued | `pending` |
//! | 3 | an empty line: the room again under the full status line, then the prompt | |
//! | 4 | `x` leaves | |
//! | 5 | confusion may lose the command | [`Confusion`] |
//! | 6 | the parser and `handle_commands` | `dispatch` |
//! | 7 | text and remote-action exits that react to typed words | |
//! | 8 | the room's command text block | [`RoomCommandText`] |
//! | 9 | a living, unparalysed player typing a spellbook name casts it | [`SpellByName`] |
//! | 10 | mute, profanity, social actions | [`SocialAction`] |
//! | 11 | speech: say, yell, `Your command had no effect.` | |
//!
//! Stages whose bodies belong to later cards are trait hooks defaulting to "no match", so
//! the order is fixed now and the bodies land later: [`HOOKS`] is the one place to change.
//! Stage 7's text exits are matched here (the room data is loaded); the move itself is
//! `movement::move_user` (T-026). Not carried: the BBS host's per-user bookkeeping, the
//! developer status line and debug stats, the developer mode's `sysop` escape at stage 5
//! and the host's profanity check at stage 10 (a BBS option, not game logic).

use silvermere_text::Msg;
use silvermere_types::enums::{Direction, ExitType, StatlineMode};
use silvermere_types::flags::{PlayerSettings, PlayerStatus, PlayerStatus2, SessionFlags};
use silvermere_types::ids::{MessageNo, RoomKey, SessionId};
use silvermere_types::names::DIRECTION_NAMES;
use silvermere_types::sizes::NDIRS;

use super::dispatch::{Handled, handle_commands};
use super::parse::{CommandLine, eq_ignore_case};
use super::{info, misc, movement, pending, social};
use crate::state::Outcome;
use crate::state::login::leave;
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// The line prefix that is always spoken.
pub const TALKCHAR: u8 = b'.';
/// The line prefix that is yelled.
pub const YELLCHAR: u8 = b'"';

/// Stage 5: the confusion check. `true` when the action is lost (the player and the room have been
/// told, a delay of 1 added). The abilities are T-050's.
pub trait Confusion: Sync {
    fn lost(&self, _world: &mut World, _id: SessionId) -> bool {
        false
    }
}

/// Stage 7's remote-action exits: `true` when one of the exit's three texts matched and the action
/// ran (it shows the prompt itself).
pub trait RemoteExits: Sync {
    fn perform(&self, _world: &mut World, _id: SessionId, _line: &str) -> bool {
        false
    }
}

/// Stage 8: `perform_special_command` over the room's `cmdtext` block.
pub trait RoomCommandText: Sync {
    fn perform(&self, _world: &mut World, _id: SessionId, _line: &str) -> bool {
        false
    }
}

/// Stage 9: `find_spell_in_spellbook` by whole name, then CAST or INVOKE (Kai). `true` when a spell
/// was cast (the prompt follows).
pub trait SpellByName: Sync {
    fn cast(&self, _world: &mut World, _id: SessionId, _line: &str) -> bool {
        false
    }
}

/// Stage 10: `handle_social_actions`: a line starting with a social verb.
pub trait SocialAction: Sync {
    fn handle(&self, _world: &mut World, _id: SessionId, _line: &str) -> bool {
        false
    }
}

/// The hooks later cards implement; every default answers "no match".
pub struct Hooks {
    pub confusion: &'static dyn Confusion,
    pub remote_exits: &'static dyn RemoteExits,
    pub room_text: &'static dyn RoomCommandText,
    pub spell_by_name: &'static dyn SpellByName,
    pub social: &'static dyn SocialAction,
}

/// The defaults: nothing matches.
pub struct NoMatch;

impl Confusion for NoMatch {}
impl RemoteExits for NoMatch {}
impl RoomCommandText for NoMatch {}
impl SpellByName for NoMatch {}
impl SocialAction for NoMatch {}

/// Stage 5: `check_confusion` ([`crate::spells::active::check_confusion`], T-050).
pub struct SpellConfusion;

impl Confusion for SpellConfusion {
    fn lost(&self, world: &mut World, id: SessionId) -> bool {
        crate::spells::active::check_confusion(world, id)
    }
}

/// The hooks in use. A later card points a field at its own implementation.
pub static HOOKS: Hooks = Hooks {
    confusion: &SpellConfusion,                              // T-050
    remote_exits: &crate::scripts::actions::RemoteExitWords, // T-054
    room_text: &crate::scripts::special::RoomText,           // T-054
    spell_by_name: &super::cast::SpellByName,                // T-050
    social: &social::Actions,                                // T-046
};

/// One line typed in the realm, through the stages in order. With `queued` (a line replayed from
/// the pending queue) stage 2 is skipped.
pub fn execute_input(world: &mut World, id: SessionId, line: &str, queued: bool) -> Outcome {
    let Some(s) = world.session_mut(id) else {
        return Outcome::Logoff;
    };
    if s.character.is_none() {
        // no record means not in the game: the line is dropped
        return Outcome::Continue;
    }
    // the player is active again
    s.ust.flags.remove(SessionFlags::INACTWARN);

    // Stage 1: waiting to exit.
    if leave::is_exiting(world, id) {
        leave::exiting_line(world, id, line);
        return Outcome::Continue;
    }

    // Stage 2: a delay running or lines waiting: queued, in typed order.
    if !queued {
        let delayed = world
            .session(id)
            .and_then(|s| s.character.as_ref())
            .is_some_and(|p| p.delay != 0);
        if delayed || pending::commands_pending(world, id) {
            pending::add_delayed_command(world, id, line);
            return Outcome::Continue;
        }
    }

    let cl = CommandLine::parse(line);

    // Stage 3: an empty line.
    if cl.argc() == 0 {
        let full = world
            .session(id)
            .and_then(|s| s.character.as_ref())
            .is_some_and(|p| matches!(p.statline, StatlineMode::Full | StatlineMode::FullCustom));
        if full {
            info::display_room_desc(world, id);
        }
        prompt(world, id);
        return Outcome::Continue;
    }

    // Stage 4: `x` leaves.
    if eq_ignore_case(cl.first(), "x") {
        if misc::cmd_quit(world, id, super::Cmd::Quit, &cl) == Handled::Done {
            prompt(world, id);
        }
        return Outcome::Continue;
    }

    // Stage 5: confusion.
    if HOOKS.confusion.lost(world, id) {
        return Outcome::Continue;
    }

    // Stage 6: the parser and the handlers.
    match handle_commands(world, id, &cl) {
        Handled::Wait => return Outcome::Continue,
        Handled::Done => {
            prompt(world, id);
            return Outcome::Continue;
        }
        Handled::NotCommand => {}
    }

    // the room's words and special commands work for the living only
    let (alive, key) = match world.session(id).and_then(|s| s.character.as_ref()) {
        Some(p) => (p.hp > 0, RoomKey::new(p.map.0, p.room.0)),
        None => return Outcome::Continue,
    };
    let room = world.room(key).cloned();
    if let (true, Some(rm)) = (alive, room.as_ref()) {
        // Stage 7: exits that react to typed words.
        if text_exit(world, id, rm, line) {
            return Outcome::Continue;
        }
        if HOOKS.remote_exits.perform(world, id, line) {
            return Outcome::Continue;
        }
        // Stage 8: the room's own command text.
        if rm.cmdtext.0 != 0 && HOOKS.room_text.perform(world, id, line) {
            prompt(world, id);
            return Outcome::Continue;
        }
    }

    // Stage 9: a spell typed by name, for the living and unparalysed.
    let can_cast = world
        .session(id)
        .and_then(|s| s.character.as_ref())
        .is_some_and(|p| p.hp > 0 && !p.status.contains(PlayerStatus::PARALYSED));
    if can_cast && HOOKS.spell_by_name.cast(world, id, line) {
        prompt(world, id);
        return Outcome::Continue;
    }

    // Stage 10: a mute player cannot speak; social actions.
    let mute = world
        .session(id)
        .and_then(|s| s.character.as_ref())
        .is_some_and(|p| p.status.contains(PlayerStatus::MUTE));
    if mute {
        // the dynamic stats are brought up to date: the mute may have worn off, but the
        // line is refused all the same (the next one speaks)
        if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
            crate::player::secondary::apply_dynamic(world, &mut c);
            if let Some(s) = world.session_mut(id) {
                s.character = Some(c);
            }
        }
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::CannotSpeak);
        }
        prompt(world, id);
        return Outcome::Continue;
    }
    if HOOKS.social.handle(world, id, line) {
        return Outcome::Continue;
    }

    // Stage 11: speech.
    speech(world, id, room.as_ref(), line);
    prompt(world, id);
    Outcome::Continue
}

fn prompt(world: &mut World, id: SessionId) {
    if let Some(s) = world.session_mut(id) {
        s.push_prompt();
    }
}

/// Stage 7's text exits: a Text exit whose WCCMSG record has one of its three texts equal to the
/// whole typed line (`eq_ignore_case`) walks the player through it (how 4), unless held or knocked
/// down; walking ends sneaking and hiding. The line is dealt with once a text matches, whether or
/// not the player could move.
fn text_exit(
    world: &mut World,
    id: SessionId,
    rm: &silvermere_types::records::Room,
    line: &str,
) -> bool {
    for (i, exit) in rm.exits.iter().enumerate().take(NDIRS) {
        if exit.target.0 == 0 || exit.kind != ExitType::Text {
            continue;
        }
        let Some(msg) = world.messages.get(&MessageNo(exit.par1 as i32)) else {
            continue;
        };
        if !msg.msgs.iter().any(|t| eq_ignore_case(line, t)) {
            continue;
        }
        let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
            return true;
        };
        if plr.status2.contains(PlayerStatus2::HELD) {
            if let Some(s) = world.session_mut(id) {
                s.output.push(Msg::CantMoveHeld);
            }
        } else if plr.status2.contains(PlayerStatus2::KNOCKEDDOWN) {
            if let Some(s) = world.session_mut(id) {
                s.output.push(Msg::CantMoveStunned);
            }
        } else {
            plr.status.remove(PlayerStatus::SNEAKING);
            plr.hidden = 0;
            movement::move_user(world, id, i as i16, 4);
        }
        return true;
    }
    false
}

/// Stage 11. With `talk fast` on, any line not starting with the yell character or `/` is said to
/// the room; a line starting with the talk character is always said, with the character dropped;
/// one starting with the yell character is yelled to the room and, with the direction it came from,
/// to every room next to it. Then the speaker is told: without `talk fast` a line nobody heard
/// (every line that was not speech at all) is `Your command had no effect.`; otherwise `You
/// say/yell ...` or `Well Spoken...`.
fn speech(
    world: &mut World,
    id: SessionId,
    rm: Option<&silvermere_types::records::Room>,
    line: &str,
) {
    let Some(plr) = world.session(id).and_then(|s| s.character.as_ref()) else {
        return;
    };
    let fast = plr.fasttalk != 0;
    let brief = plr.settings.contains(PlayerSettings::BRIEFTALK);
    let who = plr.firstname.clone();
    let key = RoomKey::new(plr.map.0, plr.room.0);
    let first = line.as_bytes().first().copied();
    let mut heard = 0;
    let mut text = line;
    if (fast && first != Some(YELLCHAR) && first != Some(b'/')) || first == Some(TALKCHAR) {
        give_away(world, id);
        if first == Some(TALKCHAR) {
            text = &line[1..];
        }
        heard = tell_room(
            world,
            key,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &Msg::Says {
                who: who.clone(),
                text: text.to_string(),
            },
        );
    } else if first == Some(YELLCHAR) {
        give_away(world, id);
        text = &line[1..];
        if let Some(rm) = rm {
            // the player's own room hears who yelled ...
            heard = tell_room(
                world,
                rm.key,
                [Some(id), None],
                Noise::Always,
                TellBlind::All,
                true,
                None,
                &Msg::YellFrom {
                    who: who.clone(),
                    text: text.to_string(),
                },
            );
            // ... and every room next to it hears the direction the yell came from
            for (i, exit) in rm.exits.iter().enumerate().take(NDIRS) {
                if exit.target.0 == 0 {
                    continue;
                }
                let rev = Direction::from_code(i as u16)
                    .reverse()
                    .unwrap_or(Direction::North);
                let msg = match rev {
                    Direction::Up => Msg::YellAbove {
                        text: text.to_string(),
                    },
                    Direction::Down => Msg::YellBelow {
                        text: text.to_string(),
                    },
                    d => Msg::YellFromDir {
                        dir: DIRECTION_NAMES[usize::from(d.code())].to_string(),
                        text: text.to_string(),
                    },
                };
                let there = RoomKey::new(rm.key.map.0, exit.target.0);
                heard |= tell_room(
                    world,
                    there,
                    [Some(id), None],
                    Noise::Always,
                    TellBlind::All,
                    true,
                    None,
                    &msg,
                );
            }
        }
    }
    let Some(s) = world.session_mut(id) else {
        return;
    };
    if !fast && heard == 0 {
        s.output.push(Msg::NoEffect);
    } else {
        if let Some(plr) = s.character.as_mut() {
            plr.status.remove(PlayerStatus::SNEAKING);
            plr.hidden = 0;
        }
        if brief {
            s.output.push(Msg::WellSpoken);
        } else {
            s.output.push(Msg::YouSay {
                verb: if first == Some(YELLCHAR) {
                    "yell"
                } else {
                    "say"
                }
                .to_string(),
                text: text.to_string(),
            });
        }
    }
}

/// Ends sneaking and hiding: speaking gives the player away.
fn give_away(world: &mut World, id: SessionId) {
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.status.remove(PlayerStatus::SNEAKING);
        plr.hidden = 0;
    }
}

/// One keystroke in character mode (in the original, split-screen players only, whose line runs
/// on Enter). Left to the stat editor (T-039) and the character-mode
/// paths. A paged listing is stopped by an `X` line, not a key (`state::listing::on_line`).
pub fn on_key(world: &mut World, id: SessionId, _byte: u8) -> Outcome {
    match world.session(id) {
        Some(s) => Outcome::Unimplemented(s.state),
        None => Outcome::Logoff,
    }
}
