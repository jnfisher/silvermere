//! ASK and GREET and the monster's answer, `ask_monster_a_question`.
//!
//! A talking monster's known-monster record names a "talk" text block whose lines are
//! `KEYWORDS:textnumber`. ASK shows the text of the first line whose keywords appear in
//! the question (both upper-cased), and runs the special command block that text links
//! to; GREET (and ASK with no question) shows the text the talk block itself links to.

use silvermere_text::Msg;
use silvermere_types::enums::DebugLevel;
use silvermere_types::ids::{ActiveMonsterId, SessionId, TextBlockNo};

use super::dispatch::Handled;
use super::{Cmd, CommandLine};
use crate::display::long_text::display_long_text;
use crate::items::inventory::matches_any_word;
use crate::scripts::{leading_int32, pattern_lines, run_text_block, skip_blanks};
use crate::world::World;

/// The monster in the player's room that `name` names: the monster step of the action target
/// search, dropping trailing words of `name` until a monster's name fits (`matches_any_word`). The
/// "which one?" list for several matches is the full target search's (T-033); the first monster in
/// room order is taken.
fn find_monster(world: &World, id: SessionId, name: &str) -> Option<ActiveMonsterId> {
    let key = world.session(id)?.room_key()?;
    let state = world.room_state.get(&key)?;
    let words: Vec<&str> = name.split_whitespace().collect();
    for n in (1..=words.len()).rev() {
        let typed = words[..n].join(" ");
        for &m in state.monsters.iter().filter(|m| m.0 != 0) {
            if world
                .active_monsters
                .values()
                .any(|a| a.number == m && matches_any_word(&typed, &a.name))
            {
                return Some(m);
            }
        }
    }
    None
}

/// ASK {monster} [{question}]. The question is the line from the word after ASK on (the
/// rest of the raw line), so words of a long monster name are part of it. No monster by
/// that name: not a command (the line goes on to the social actions and speech).
pub fn cmd_ask(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() < 2 {
        return Handled::NotCommand;
    }
    let Some(m) = find_monster(world, id, line.rest_from(1)) else {
        return Handled::NotCommand;
    };
    // the match lookup ends: the pending colour reset is dropped
    crate::output::clear_output(world, id);
    let question = (line.argc() > 2).then(|| line.rest_from(2).to_owned());
    ask_monster_a_question(world, id, m, question.as_deref());
    Handled::Done
}

/// GREET {monster}: ASK with no question.
pub fn cmd_greet(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() < 2 {
        return Handled::NotCommand;
    }
    let Some(m) = find_monster(world, id, line.rest_from(1)) else {
        return Handled::NotCommand;
    };
    // the match lookup ends: the pending colour reset is dropped
    crate::output::clear_output(world, id);
    ask_monster_a_question(world, id, m, None);
    Handled::Done
}

/// The monster's answer to `id`. No talk block: `%s has nothing to tell you!`. A greeting
/// shows the block the talk block links to, with the monster's name for `%s`, or `%s
/// doesn't understand you.` when it links nowhere. A question takes the first line whose
/// keywords are in it (a substring, both upper case), shows that line's text block with the
/// name and runs the block that text links to as a whole; no line: `%s has nothing to
/// tell you!`. A talk block that cannot be read gives no answer at all.
pub fn ask_monster_a_question(
    world: &mut World,
    id: SessionId,
    monster: ActiveMonsterId,
    question: Option<&str>,
) {
    let Some(mon) = world
        .active_monsters
        .values()
        .find(|a| a.number == monster)
        .cloned()
    else {
        return;
    };
    let question = question.map(str::to_ascii_uppercase);
    let Some(talktext) = world.monsters.get(&mon.knownnum).map(|k| k.talktext) else {
        return;
    };
    let name = mon.name.clone();
    let say = |world: &mut World, msg: Msg| {
        if let Some(s) = world.session_mut(id) {
            s.output.push(msg);
        }
    };
    // no talk block
    if talktext.0 == 0 {
        say(world, Msg::MonsterNothingToTell { name });
        return;
    }
    // the first part only; a missing one is ignored without a word
    let Some(blk) = world.text_blocks.get(&(0, talktext)).cloned() else {
        return;
    };
    let Some(question) = question else {
        // a greeting
        if blk.link.0 == 0 {
            say(world, Msg::MonsterDoesntUnderstand { name });
        } else {
            display_long_text(world, id, blk.link, Some(&name));
        }
        return;
    };
    let debug = world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off);
    // the lines `KEYWORDS:textnumber`, in order
    for pl in pattern_lines(&blk.text) {
        let kw = pl.pattern.to_ascii_uppercase();
        let found = question.find(&kw);
        if debug {
            say(
                world,
                Msg::MonsterTalkDebug {
                    question: skip_blanks(&question).to_owned(),
                    keywords: match found {
                        Some(i) => skip_blanks(&question[i..]).to_owned(),
                        None => kw.clone(),
                    },
                },
            );
        }
        if found.is_some() {
            let textnum = TextBlockNo(leading_int32(&pl.action));
            let rc = display_long_text(world, id, textnum, Some(&name));
            if rc.0 != 0 {
                run_text_block(world, id, rc);
            }
            return;
        }
    }
    say(world, Msg::MonsterNothingToTell { name });
}
