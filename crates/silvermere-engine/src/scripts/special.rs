//! The three text block runners and their hooks: [`perform_special_command`] (a room's
//! command text against what was typed), [`perform_random_text_block`] (one line by a
//! percentage roll) and [`run_text_block`] (a whole block as an event).

use std::cell::Cell;

use silvermere_text::Msg;
use silvermere_types::enums::DebugLevel;
use silvermere_types::ids::{SessionId, SpellNo, TextBlockNo};

use super::actions::{LineResult, perform_matched_action};
use super::{
    MAX_RECORDS, STOCK_TELEPORTS, TeleportArea, leading_int32, pattern_lines, whole_lines,
    wildcard_match,
};
use crate::commands::input::RoomCommandText;
use crate::scheduler::Rng as _;
use crate::world::World;

/// What the scripts need from cards that have not landed, and the world's own content.
pub trait ScriptHooks: Sync {
    /// An item-mode cast with no target for the `cast` clause:
    /// `false` stops the line. T-050/T-051. Until then the clause passes, logged.
    fn cast_no_target(&self, _world: &mut World, id: SessionId, spell: SpellNo) -> bool {
        tracing::debug!(?id, spell = spell.0, "script cast not run (T-051)");
        true
    }
    /// The named teleports (`teleport_silvermere`, ...): the stock world's table, which
    /// a world with other areas replaces.
    fn teleport_areas(&self) -> &'static [TeleportArea] {
        &STOCK_TELEPORTS
    }
}

/// The defaults.
pub struct Defaults;

impl ScriptHooks for Defaults {}

/// The hooks in use: the `cast` clause casts through T-050's `cast_no_target` with
/// `MODE_ITEM` (no success roll, the item's magery test).
pub struct SpellScripts;

impl ScriptHooks for SpellScripts {
    fn cast_no_target(&self, world: &mut World, id: SessionId, spell: SpellNo) -> bool {
        let opts = crate::spells::cast::CastOpts {
            mode: crate::spells::cast::CastMode::Item,
            ..Default::default()
        };
        crate::spells::cast::cast_no_target(world, id, spell, opts).outcome
            != crate::spells::cast::CastOutcome::Refused
    }
}

/// The hooks in use.
pub struct Hooks {
    pub scripts: &'static dyn ScriptHooks,
}

/// The hooks in use. A later card points a field at its own implementation.
pub static HOOKS: Hooks = Hooks {
    scripts: &SpellScripts, // T-050 (cast)
};

/// How deep the runners may call each other (`random`, `checkspell` and `testskill` run
/// blocks). Under the original a block that runs itself brings the game down;
/// here such a chain is cut and logged, so bad data cannot abort the world thread.
const MAX_DEPTH: u32 = 32;

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f` one level deeper, or `None` past [`MAX_DEPTH`].
fn nested<T>(textid: TextBlockNo, f: impl FnOnce() -> T) -> Option<T> {
    let d = DEPTH.with(Cell::get);
    if d >= MAX_DEPTH {
        tracing::warn!(block = textid.0, "text block scripts nested too deep; cut");
        return None;
    }
    DEPTH.with(|c| c.set(d + 1));
    let r = f();
    DEPTH.with(|c| c.set(d));
    Some(r)
}

fn debug_on(world: &World, id: SessionId) -> bool {
    world
        .session(id)
        .is_some_and(|s| s.flags.debug != DebugLevel::Off)
}

/// The records of a block, part 0 on, at most [`MAX_RECORDS`], until one is missing.
fn records(world: &World, textid: TextBlockNo) -> Vec<String> {
    (0..MAX_RECORDS)
        .map_while(|seq| {
            world
                .text_blocks
                .get(&(seq, textid))
                .map(|t| t.text.clone())
        })
        .collect()
}

/// Each line of the block whose pattern matches the typed `input` (`wildcard_match`) has
/// its clauses run, until one runs through. Returns [`LineResult::Nothing`] when no pattern
/// matched, else the last line's result; the callers take anything else as "the room
/// handled it". Sysop debugging shows each test (`Checking for ... in ...`).
pub fn perform_special_command(
    world: &mut World,
    id: SessionId,
    textid: TextBlockNo,
    input: &str,
) -> LineResult {
    nested(textid, || {
        let mut result = LineResult::Nothing;
        for text in records(world, textid) {
            for pl in pattern_lines(&text) {
                if debug_on(world, id)
                    && let Some(s) = world.session_mut(id)
                {
                    s.output.push(Msg::ScriptDebugChecking {
                        input: input.to_owned(),
                        pattern: pl.pattern.clone(),
                    });
                }
                if wildcard_match(&pl.pattern, input) {
                    result = perform_matched_action(world, id, &pl.action);
                    if result == LineResult::Ok {
                        return result;
                    }
                }
            }
        }
        result
    })
    .unwrap_or_default()
}

/// The `random` clause: one roll (`range(0, 100)`) for the block, and the first line whose
/// percentage is above it runs (the percentages are cumulative thresholds). Returns that
/// line's result, [`LineResult::Nothing`] when no line was chosen.
pub fn perform_random_text_block(
    world: &mut World,
    id: SessionId,
    textid: TextBlockNo,
) -> LineResult {
    let roll = world.rng.range(0, 100);
    nested(textid, || {
        for text in records(world, textid) {
            for pl in pattern_lines(&text) {
                if roll < leading_int32(&pl.pattern) {
                    return perform_matched_action(world, id, &pl.action);
                }
            }
        }
        LineResult::Nothing
    })
    .unwrap_or_default()
}

/// The block as an event (a spell's or monster's text block, a death text, a monster's
/// answer, `checkspell` and `testskill`): its lines in turn until one runs through. Sysop
/// debugging shows each line (`Line N:...`).
pub fn run_text_block(world: &mut World, id: SessionId, textid: TextBlockNo) -> LineResult {
    nested(textid, || {
        let mut result = LineResult::Nothing;
        let mut lineno = 1i64;
        for text in records(world, textid) {
            for line in whole_lines(&text) {
                if debug_on(world, id)
                    && let Some(s) = world.session_mut(id)
                {
                    s.output.push(Msg::ScriptDebugLine {
                        n: lineno,
                        line: line.to_owned(),
                    });
                    lineno += 1;
                }
                result = perform_matched_action(world, id, line);
                if result == LineResult::Ok {
                    return result;
                }
            }
        }
        result
    })
    .unwrap_or_default()
}

/// The room's command text, if it has one, run with [`perform_special_command`] for what
/// the player typed, as the input stage (`execute_input`), LOOK, BUY and USE ask it: `true`
/// when the room handled the line (the caller then shows the prompt).
pub fn room_command(world: &mut World, id: SessionId, input: &str) -> bool {
    let Some(cmdtext) = world
        .session(id)
        .and_then(|s| s.room_key())
        .and_then(|k| world.room(k))
        .map(|r| r.cmdtext)
    else {
        return false;
    };
    cmdtext.0 != 0 && perform_special_command(world, id, cmdtext, input) != LineResult::Nothing
}

/// Stage 8 of `execute_input`: the room's command text.
pub struct RoomText;

impl RoomCommandText for RoomText {
    fn perform(&self, world: &mut World, id: SessionId, line: &str) -> bool {
        room_command(world, id, line)
    }
}
