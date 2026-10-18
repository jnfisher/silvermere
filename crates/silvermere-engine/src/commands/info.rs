//! Information: look, status, exp, who, top, spells, help.
//!
//! T-024 registered the handlers below; T-025 filled the bare LOOK, EXITS and the
//! empty-line room; T-027 the rest. The displays themselves live in `crate::display`
//! (`status`, `who`, `topten`, `spells`, `describe`) and `crate::help`.

use silvermere_text::{Msg, cp437};
use silvermere_types::enums::{CommandCode, Direction, ExitType};
use silvermere_types::flags::{ExitFlags, PlayerStatus};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, RoomKey, SessionId, SpellNo};

use super::dispatch::Handled;
use super::{Cmd, CommandLine, eq_ignore_case, parse_command, starts_with_ignore_case};
use crate::display::exits::exits_msg;
use crate::display::{RoomMode, can_see, describe, display_room, spells, status, topten, who};
use crate::help::{help_text, matches_any_word};
use crate::output::clear_output;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Lines shown when no count is typed.
const TOPTEN_DEFAULT: usize = 10;

/// HELP alone shows the overview, HELP {topic} the topic (the rest of the raw line). `?`
/// parses to HELP too.
pub fn cmd_help(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() == 1 {
        help_text(world, id, None, 1, "");
    } else {
        help_text(world, id, Some(line.rest_from(1)), line.argc(), line.arg(2));
    }
    Handled::Done
}

fn push(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// LOOK: nothing while the player cannot see (`can_see`'s reason); LOOK alone is [`cmd_look_room`];
/// LOOK {direction} peeks into the next room ([`look_direction`]); LOOK {name} describes what the
/// name finds ([`look_at`]).
pub fn cmd_look(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() == 1 {
        cmd_look_room(world, id);
        return Handled::Done;
    }
    if can_see(world, id).is_none() {
        return Handled::Done;
    }
    // is the word after LOOK a direction (codes 0-9)?
    match parse_command(line.arg(1)) {
        Some(c) if c.code() < CommandCode::None.code() => {
            look_direction(world, id, usize::from(c.code()))
        }
        _ => look_at_typed(world, id, line.rest_from(1), Some(&line.raw)),
    }
    Handled::Done
}

/// `LOOK` with no argument: nothing in the dark or while blinded (`can_see`'s reason instead);
/// otherwise the room again with its description and, unless the player is hidden, `"%s is looking
/// around the room."` to the others there with their prompts. The dispatcher sends the looker's
/// prompt.
pub fn cmd_look_room(world: &mut World, id: SessionId) {
    if can_see(world, id).is_none() {
        return;
    }
    display_room(world, id, RoomMode::LONG);
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    if plr.hidden != 0 {
        return;
    }
    let room = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let who = plr.firstname.clone();
    // to everyone else in the room, always heard, by the blind too, with a prompt and no
    // colour
    tell_room(
        world,
        room,
        [Some(id), None],
        Noise::Always,
        TellBlind::All,
        true,
        None,
        &Msg::LookAround { who },
    );
}

/// The no-exit refusal in its three wordings.
fn no_exit(world: &World, dir: usize) -> Msg {
    match dir {
        8 => Msg::LookNoExitUp,
        9 => Msg::LookNoExitDown,
        d => Msg::LookNoExit {
            dir: world.display.dirnames.get(d).to_owned(),
        },
    }
}

/// LOOK {direction}: the exits that cannot be looked through are refused (a shut Door or Key exit,
/// a Hidden exit that is not `EXF_VISBLOCKED`, Text and Remote exits, a Trap exit with `exitpar2s`
/// 2); a Cast exit with a message shows it instead; otherwise the room is told which way the player
/// looks, the player stands in the room beyond for a moment (on the exit's map for a Map Change
/// exit), is shown it as on arrival, and its occupants see him peek in from the reverse direction.
/// The entry checks for a look (`user_allowed_in_room` in look-only mode) are T-026's.
pub fn look_direction(world: &mut World, id: SessionId, dir: usize) {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    let old = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let hidden = plr.hidden != 0;
    let who = plr.firstname.clone();
    let Some(exit) = world.room(old).map(|r| r.exits[dir]) else {
        let m = no_exit(world, dir);
        push(world, id, m);
        return;
    };
    if exit.target.0 == 0 {
        let m = no_exit(world, dir);
        push(world, id, m);
        return;
    }
    let mut destmap = old.map;
    match exit.kind {
        // a shut Key exit has the closed-door message
        ExitType::Key if exit.par2 != 0 => {
            push(world, id, Msg::LookDoorClosed);
            return;
        }
        // a Hidden exit only when visible but blocked (sic: the flag is tested twice)
        ExitType::Hidden
            if ExitFlags::from_bits_retain(exit.par1).contains(ExitFlags::VISBLOCKED) => {}
        ExitType::Hidden | ExitType::Text | ExitType::RemoteAction => {
            let m = no_exit(world, dir);
            push(world, id, m);
            return;
        }
        // a shut Door exit
        ExitType::Door if exit.par1 != 0 => {
            push(world, id, Msg::LookDoorClosed);
            return;
        }
        // a Map Change exit looks onto its own map
        ExitType::MapChange => destmap = MapId(exit.par1),
        // a Cast exit with a message shows it instead
        ExitType::Cast if exit.par4 != 0 => {
            let text = world
                .messages
                .get(&silvermere_types::ids::MessageNo(exit.par4))
                .map(|m| m.msgs[0].clone());
            if let Some(text) = text {
                push(world, id, Msg::LookCastExit { text });
            }
            return;
        }
        // a Trap exit of kind 2 is no exit
        ExitType::Trap if exit.par2 == 2 => {
            let m = no_exit(world, dir);
            push(world, id, m);
            return;
        }
        _ => {}
    }
    // the room is told which way the player looks
    if !hidden {
        let msg = match dir {
            8 => Msg::LookDirUp { who: who.clone() },
            9 => Msg::LookDirDown { who: who.clone() },
            d => Msg::LookDir {
                who: who.clone(),
                dir: world.display.dirnames.get(d).to_owned(),
            },
        };
        tell_room(
            world,
            old,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &msg,
        );
    }
    // into the room beyond for a moment, shown as on arrival
    let beyond = RoomKey {
        map: destmap,
        room: exit.target,
    };
    set_location(world, id, beyond);
    display_room(world, id, RoomMode::LONG);
    // its occupants see him peek in from the reverse direction
    if !hidden {
        let back = Direction::COMPASS
            .get(dir)
            .and_then(|d| d.reverse())
            .map(|d| usize::from(d.code()));
        let msg = match back {
            Some(8) => Msg::PeekInAbove { who },
            Some(9) => Msg::PeekInBelow { who },
            Some(d) => Msg::PeekIn {
                who,
                dir: world.display.dirnames.get(d).to_owned(),
            },
            None => Msg::PeekIn {
                who,
                dir: String::new(),
            },
        };
        tell_room(
            world,
            beyond,
            [Some(id), None],
            Noise::Always,
            TellBlind::All,
            true,
            None,
            &msg,
        );
    }
    // back where he stood
    set_location(world, id, old);
}

/// The player stands in `key` (no trail).
fn set_location(world: &mut World, id: SessionId, key: RoomKey) {
    if let Some(p) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        p.map = key.map;
        p.room = key.room;
    }
}

/// What a name typed after LOOK refers to.
enum Target {
    User(SessionId),
    Monster(ActiveMonsterId),
    Item(ItemNo),
    Spell(SpellNo),
}

/// A stand-in for the action target search over monsters, players, items carried, items on
/// the floor and spells (T-033's to do in full): the first match, in that order, of the monsters in
/// the room (dropping trailing words of `name` until one fits), the players in the room by the
/// first word (a hidden one only for himself or a searcher who sees hidden), the items carried, the
/// items lying in the room, and the spells in the book. The "which one did you mean?" list for
/// several matches is not carried.
fn find_target(world: &World, id: SessionId, name: &str) -> Option<Target> {
    let plr = world.session(id)?.player()?;
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let state = world.room_state.get(&key);
    // monsters, dropping trailing words
    let words: Vec<&str> = name.split_whitespace().collect();
    for n in (1..=words.len()).rev() {
        let typed = words[..n].join(" ");
        if let Some(st) = state {
            for &m in st.monsters.iter().filter(|m| m.0 != 0) {
                if let Some(mon) = world.active_monsters.by_number(m)
                    && matches_any_word(&typed, &mon.name)
                {
                    return Some(Target::Monster(m));
                }
            }
        }
    }
    // players in the room, by the first word
    let first = words.first().copied().unwrap_or("");
    let sees_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    for other in world.session_ids() {
        if !world.is_in_game(other) {
            continue;
        }
        let Some(o) = world.session(other).and_then(|s| s.player()) else {
            continue;
        };
        if o.map == key.map
            && o.room == key.room
            && (other == id || o.hidden == 0 || sees_hidden)
            && matches_any_word(first, &o.firstname)
        {
            return Some(Target::User(other));
        }
    }
    // the items carried
    for &n in plr.items.iter().filter(|n| n.0 != 0) {
        if world
            .items
            .get(&n)
            .is_some_and(|i| matches_any_word(name, &i.name))
        {
            return Some(Target::Item(n));
        }
    }
    // the items lying in the room
    if let Some(st) = state {
        for slot in st.items.iter().filter(|s| s.item.0 != 0) {
            if world
                .items
                .get(&slot.item)
                .is_some_and(|i| matches_any_word(name, &i.name))
            {
                return Some(Target::Item(slot.item));
            }
        }
    }
    // the spells in the book
    for &n in plr.spellbook.iter().filter(|n| n.0 != 0) {
        if world
            .spells
            .get(&n)
            .is_some_and(|s| matches_any_word(name, &s.name) || eq_ignore_case(name, &s.shortnm))
        {
            return Some(Target::Spell(n));
        }
    }
    None
}

/// LOOK {name}. When nothing matches, the original tries the shop's items (`display_item_in_shop`,
/// T-041), a coin pile and the room's command text (`perform_special_command`, T-054) before the
/// player's own name; the coin pile is not carried yet.
pub fn look_at(world: &mut World, id: SessionId, name: &str) {
    look_at_typed(world, id, name, None);
}

/// [`look_at`] with the whole typed line, which the room's command text matches
/// (the raw line with its spaces).
pub fn look_at_typed(world: &mut World, id: SessionId, name: &str, input: Option<&str>) {
    let Some(plr) = world.session(id).and_then(|s| s.player()) else {
        return;
    };
    let me = plr.firstname.clone();
    let my_hidden = plr.hidden != 0;
    let key = RoomKey {
        map: plr.map,
        room: plr.room,
    };
    let sees_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    let found = find_target(world, id, name);
    // the match lookup ends, dropping the colour reset (this search carries no list)
    clear_output(world, id);
    match found {
        // nothing matched
        None => {
            // an item for sale in this shop (T-041)
            match crate::shops::display_item_in_shop(world, key, name) {
                crate::shops::ShopLook::Describe(n) => {
                    describe::display_item_desc(world, id, n);
                    return;
                }
                crate::shops::ShopLook::Ambiguous(options) => {
                    crate::items::inventory::show_options(world, id, &options);
                    return;
                }
                crate::shops::ShopLook::NotHandled => {}
            }
            // the room's command text; the prompt follows the command
            if let Some(input) = input
                && crate::scripts::room_command(world, id, input)
            {
                return;
            }
            if starts_with_ignore_case(name, &me) {
                describe::display_user_desc(world, id, id);
            } else {
                push(world, id, Msg::LookNotHere { what: name.into() });
            }
        }
        // a player
        Some(Target::User(t)) => {
            let Some(tplr) = world.session(t).and_then(|s| s.player()) else {
                return;
            };
            let t_hidden = tplr.hidden != 0;
            let them = tplr.firstname.clone();
            if !(t == id || !t_hidden || sees_hidden) {
                push(world, id, Msg::LookNotHere { what: name.into() });
                return;
            }
            describe::display_user_desc(world, t, id);
            if !t_hidden && !my_hidden {
                tell_room(
                    world,
                    key,
                    [Some(id), Some(t)],
                    Noise::Always,
                    TellBlind::All,
                    true,
                    None,
                    &Msg::LookUpDown {
                        who: me.clone(),
                        target: them,
                    },
                );
            }
            if !my_hidden
                && t != id
                && let Some(s) = world.session_mut(t)
            {
                s.output.push(Msg::LookAtYou { who: me });
                s.push_prompt();
            }
        }
        Some(Target::Monster(m)) => describe::display_monster_desc(world, id, m),
        Some(Target::Item(n)) => describe::display_item_desc(world, id, n),
        Some(Target::Spell(n)) => describe::display_spell_desc(world, id, n),
    }
}

/// MAP: the ANSI map file the room names (`room.mapfile`), line by line, at most 500 line reads
/// of up to 499 characters, then an empty line. A file that cannot be read gives the empty line
/// alone; a room with no map file says so.
///
/// The file is read in DOS text mode: `\r\n` comes back as `\n` and a Ctrl-Z ends the
/// file. Each piece is sent as a format string; a `%` in a map file is sent as it stands here.
pub fn cmd_map(world: &mut World, id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    let Some(key) = world.session(id).and_then(|s| s.room_key()) else {
        return Handled::NotCommand;
    };
    let mapfile = world
        .room(key)
        .map(|r| r.mapfile.clone())
        .unwrap_or_default();
    if mapfile.is_empty() {
        tell_user(world, id, Msg::MapNone);
        return Handled::Done;
    }
    let pieces = world
        .display
        .assets
        .get(&mapfile)
        .map(map_pieces)
        .unwrap_or_default();
    tell_user(world, id, Msg::Clrline);
    for line in pieces {
        tell_user(world, id, Msg::MapLine { line });
    }
    tell_user(world, id, Msg::MapEnd);
    Handled::Done
}

/// Reads of the map file.
const MAP_MAX_LINES: usize = 500;
/// The line-read buffer, a piece of at most 499 characters.
const MAP_LINE_BUF: usize = 500;

/// The pieces a line read into a [`MAP_LINE_BUF`] buffer returns from a text-mode file, at most
/// [`MAP_MAX_LINES`] reads (the read that meets the end counts too).
fn map_pieces(bytes: &[u8]) -> Vec<String> {
    let end = bytes.iter().position(|&b| b == 0x1A).unwrap_or(bytes.len());
    let mut text = Vec::with_capacity(end);
    let mut i = 0;
    while i < end {
        if bytes[i] == b'\r' && i + 1 < end && bytes[i + 1] == b'\n' {
            i += 1;
        }
        text.push(bytes[i]);
        i += 1;
    }
    let mut out = Vec::new();
    let mut rest = text.as_slice();
    while !rest.is_empty() && out.len() < MAP_MAX_LINES {
        let max = rest.len().min(MAP_LINE_BUF - 1);
        let len = rest[..max]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(max, |p| p + 1);
        out.push(cp437::decode(&rest[..len]));
        rest = &rest[len..];
    }
    out
}

/// IDENTITY: not implemented in 1.11p, so the word is not a command.
pub fn cmd_identity(_world: &mut World, _id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    Handled::NotCommand
}

/// TOPTEN, TOPTEN {n}, TOPTEN {class}, TOPTEN GANGS and TOPTEN GANGS {n} (either order). Only a
/// sysop may ask for more than `maxtop` lines. A players' list pages ([`topten::display_topten`])
/// and has no prompt here; a form the command does not know is not a command.
pub fn cmd_topten(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let maxtop = world.config.max_top as usize;
    let sysop = world.session(id).is_some_and(|s| s.flags.sysop);
    // no class filter to start with
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    s.ust.topclass = 0;
    if s.player().is_none() {
        return Handled::NotCommand;
    }
    let argc = line.argc();
    // the last class whose name the word begins
    if argc == 2 {
        let mut classes: Vec<(u16, String)> = world
            .classes
            .values()
            .map(|c| (c.number.0, c.name.clone()))
            .collect();
        classes.sort_unstable();
        let found = classes
            .iter()
            .rev()
            .find(|(_, n)| starts_with_ignore_case(line.arg(1), n))
            .map(|(c, _)| *c);
        if let Some(c) = found
            && let Some(s) = world.session_mut(id)
        {
            s.ust.topclass = c as i16;
            s.ust.toprank = 1;
        }
    }
    let capped = |n: usize| if n > maxtop && !sysop { maxtop } else { n };
    let leading_int = |w: &str| -> i64 {
        let t = w.trim_start();
        let end = t
            .char_indices()
            .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+'))))
            .map_or(t.len(), |(i, _)| i);
        t[..end].parse().unwrap_or(0)
    };
    let topclass = world.session(id).map_or(0, |s| s.ust.topclass);
    if argc == 1 {
        topten::display_topten(world, id, TOPTEN_DEFAULT);
        return Handled::Wait;
    }
    if argc == 3 {
        // TOPTEN GANGS {n}, either order
        let n = if starts_with_ignore_case(line.arg(1), "gangs") {
            Some(leading_int(line.arg(2)))
        } else if starts_with_ignore_case(line.arg(2), "gangs") {
            Some(leading_int(line.arg(1)))
        } else {
            None
        };
        if let Some(n) = n {
            topten::display_top_gangs(world, id, capped(n.max(0) as usize));
        }
        return Handled::Done;
    }
    if topclass != 0 {
        // a class list
        topten::display_topten(world, id, maxtop);
        return Handled::Wait;
    }
    if starts_with_ignore_case(line.arg(1), "gangs") {
        topten::display_top_gangs(world, id, TOPTEN_DEFAULT);
        return Handled::Done;
    }
    let n = leading_int(line.arg(1));
    if n > 0 {
        topten::display_topten(world, id, capped(n as usize));
        return Handled::Wait;
    }
    Handled::NotCommand
}

/// EXPERIENCE alone; anything after it is not the command.
pub fn cmd_experience(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() > 1 || world.session(id).and_then(|s| s.player()).is_none() {
        return Handled::NotCommand;
    }
    status::show_experience(world, id);
    Handled::Done
}

/// STATUS.
pub fn show_status(world: &mut World, id: SessionId) {
    status::show_status(world, id, id);
}

/// HEALTH.
pub fn show_health(world: &mut World, id: SessionId) {
    status::show_health(world, id);
}

/// SPELLS and POWERS.
pub fn display_users_spells(world: &mut World, id: SessionId, cmd: Cmd) {
    spells::display_users_spells(world, id, id, cmd);
}

/// SCAN and WHO.
pub fn display_scan_of_users(world: &mut World, id: SessionId) {
    who::display_scan_of_users(world, id);
}

/// EXITS: `can_see`, then the visible exits of the player's own room: the `Obvious exits:`
/// line alone.
pub fn display_visible_exits(world: &mut World, id: SessionId) {
    if can_see(world, id).is_none() {
        return;
    }
    let Some(key) = world.session(id).and_then(|s| s.room_key()) else {
        return;
    };
    // T-026: the exits as they stand now
    let Some(room) = crate::movement::exits::live_room(world, key) else {
        return;
    };
    let msg = exits_msg(&room, &world.display.dirnames, &world.messages);
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
    }
}

/// PROFILE, the settings screen (T-028).
pub fn display_profile(world: &mut World, id: SessionId) {
    crate::display::profile::display_profile(world, id);
}

/// The short room again on an empty line under the full status line.
pub fn display_room_desc(world: &mut World, id: SessionId) {
    display_room(world, id, RoomMode::SHORT);
}
