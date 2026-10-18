//! Everything else: rest, meditate, search, hide, sneak, quit and the commands no family
//! claims.
//!
//! T-024 fills the small dispatcher arms (QUIT via T-021's `cmd_quit`, SAVE, QUIET/BRIEF,
//! VERBOSE, VERSION, TALK, STATLINE, PAGE, ACTION on/off); T-103 adds HIDE (oneself), SNEAK,
//! TRACK and DISARM TRAP, and the not-implemented TAME and MESMERIZE.

use silvermere_rules::Rng;
use silvermere_rules::combat::Move;
use silvermere_text::Msg;
use silvermere_types::enums::{
    Ability, ExitType, RoomType, SessionState, StatlineMode, SuicideStep,
};
use silvermere_types::flags::{PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{ActiveMonsterId, MessageNo, RoomId, RoomKey, SessionId};
use silvermere_types::sizes::{NAMTRAIL, NTRAIL};

use super::combat::{Found, find_action_target};
use super::dispatch::{Handled, arg_is};
use super::pending::{add_delay, delay_done};
use super::{Cmd, CommandLine, parse_command, social, starts_with_ignore_case};
use crate::combat::autocombat::{attacker_is_in_same_room, attacker_type, is_being_attacked};
use crate::combat::{debug_on, player, player_mut, room_of, user_has};
use crate::event_loop::{EchoMode, EngineOutput};
use crate::gangs::{Target, find_any_action_target, find_monster_in_memory};
use crate::items::inventory::show_options;
use crate::movement::exits::{
    Arg, fmt_msg, green_name, live_exit, live_exits, set_par2, tell_room_all,
};
use crate::movement::move_user::{MoveType, calc_hide_sneak_chance};
use crate::movement::timed_exits::add_exit_to_list;
use crate::movement::{HOOKS, direction, dirname};
use crate::output::clear_output;
use crate::player::suicide::commit_suicide;
use crate::state::login::leave;
use crate::state::newchar::suicide_pw;
use crate::tell::tell_user;
use crate::world::World;

/// QUIT: T-021's `state::login::leave::cmd_quit`, which answers `true` when the prompt should
/// follow.
pub fn cmd_quit(world: &mut World, id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    Handled::from_rc(leave::cmd_quit(world, id))
}

/// SAVE: the refusal, alone; with an argument the word is speech.
pub fn cmd_save(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() != 1 {
        return Handled::NotCommand;
    }
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::SaveRefused);
    }
    Handled::Done
}

/// QUIET and BRIEF: display mode 0 (`verbose` off) and `Quiet mode set`.
pub fn cmd_quiet(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    display_mode(world, id, line, 0, Msg::QuietSet)
}

/// VERBOSE: display mode 1 and `Verbose mode set`.
pub fn cmd_verbose(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    display_mode(world, id, line, 1, Msg::VerboseSet)
}

fn display_mode(
    world: &mut World,
    id: SessionId,
    line: &CommandLine,
    verbose: u8,
    msg: Msg,
) -> Handled {
    if line.argc() != 1 {
        return Handled::NotCommand;
    }
    if let Some(s) = world.session_mut(id) {
        if let Some(plr) = s.character.as_mut() {
            plr.verbose = verbose;
        }
        s.output.push(msg);
    }
    Handled::Done
}

/// VERSION: `MajorMUD v <mudver>` in grey, alone.
pub fn cmd_version(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() != 1 {
        return Handled::NotCommand;
    }
    let version = world.compat.mud_version.clone();
    if let Some(s) = world.session_mut(id) {
        s.output.push(Msg::Version { version });
    }
    Handled::Done
}

/// `TALK FAST` / `TALK SLOW` set `fasttalk`; any other word count gets the syntax line and
/// another second word is speech.
pub fn cmd_talk(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() != 2 {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::TalkSyntax);
        }
        return Handled::Done;
    }
    let fast = if arg_is(line, 1, "Fast") {
        1
    } else if arg_is(line, 1, "Slow") {
        0
    } else {
        return Handled::NotCommand;
    };
    if let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) {
        plr.fasttalk = fast;
    }
    Handled::Done
}

/// `STATLINE ON/OFF/FULL` sets the mode; anything else is the syntax line.
pub fn cmd_statline(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let mode = if line.argc() != 2 {
        None
    } else if arg_is(line, 1, "on") {
        Some(StatlineMode::On)
    } else if arg_is(line, 1, "off") {
        Some(StatlineMode::Off)
    } else if arg_is(line, 1, "full") {
        Some(StatlineMode::Full)
    } else {
        None
    };
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    match mode {
        Some(mode) => {
            if let Some(plr) = s.character.as_mut() {
                plr.statline = mode;
            }
        }
        None => s.output.push(Msg::StatlineSyntax),
    }
    Handled::Done
}

/// PAGE: toggles `paging`, says so and saves; with an argument the word is speech.
pub fn cmd_page(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() != 1 {
        return Handled::NotCommand;
    }
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    let Some(plr) = s.character.as_mut() else {
        return Handled::NotCommand;
    };
    plr.paging = u8::from(plr.paging == 0);
    let state = if plr.paging != 0 { "Ok" } else { "Off" }.to_string();
    s.output.push(Msg::PagingSet { state });
    leave::save_player(world, id);
    Handled::Done
}

/// ACTION: alone or `ACTION LIST` lists the social actions (T-046), `ON`/`OFF` clear or set
/// `PS_NOACTIONS`; another word is speech.
pub fn cmd_action(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() == 1 || arg_is(line, 1, "list") {
        social::display_list_of_actions(world, id);
        return Handled::Done;
    }
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return Handled::NotCommand;
    };
    if arg_is(line, 1, "on") {
        plr.settings.remove(PlayerSettings::NOACTIONS);
    } else if arg_is(line, 1, "off") {
        plr.settings.insert(PlayerSettings::NOACTIONS);
    } else {
        return Handled::NotCommand;
    }
    Handled::Done
}

/// HIDE. HIDE {item} and HIDE {amount} {currency} are T-030's [`super::inventory::hide_item`]; HIDE
/// alone is the player trying to hide in the room.
///
/// Any earlier hiding is given up first. In a fight (autocombat, being fought, engaged,
/// or a monster here that could attack) the attempt fails at once and still costs a
/// tick; held or knocked down, the player cannot move to hide; with the command delay
/// running he must wait. Otherwise the attempt costs one tick and works when
/// a roll of 0 to 99 comes in under the hide/sneak chance (at most 95). A failure looks
/// like a success unless a second roll comes in under his perception. HIDE alone always
/// answers 2: no prompt follows.
pub fn cmd_hide(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    if line.argc() > 1 {
        return super::inventory::hide_item(world, id, cmd, line);
    }
    let Some(plr) = player_mut(world, id) else {
        return Handled::NotCommand;
    };
    // rest, meditation and any earlier hiding end
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    plr.hidden = 0;
    let (attackers, status2) = (plr.attackers, plr.status2);
    // in a fight the attempt fails at once
    if HOOKS.combat.in_autocombat(world, id)
        || HOOKS.combat.being_attacked(world, id)
        || attackers > 0
        || HOOKS.combat.monster_could_attack(world, id)
    {
        tell_user(world, id, Msg::HideAttempt);
        tell_user(world, id, Msg::HideNotHidden);
        add_delay(world, id, 1);
    } else if status2.contains(PlayerStatus2::HELD) {
        tell_user(world, id, Msg::HideHeld);
    } else if status2.contains(PlayerStatus2::KNOCKEDDOWN) {
        tell_user(world, id, Msg::HideStunned);
    } else if delay_done(world, id) {
        // the attempt
        add_delay(world, id, 1);
        tell_user(world, id, Msg::HideAttemptClr);
        let chance = calc_hide_sneak_chance(world, id, 95, Some(id));
        let msg = if world.rng.range(0, 100) < chance {
            if let Some(p) = player_mut(world, id) {
                p.hidden = 1;
            }
            Msg::StealthLineEnd
        } else {
            let perception = player(world, id).map_or(0, |p| i32::from(p.perception));
            if world.rng.range(0, 100) < perception {
                Msg::HideNotHidden
            } else {
                Msg::StealthLineEnd
            }
        };
        tell_user(world, id, msg);
    } else {
        tell_user(world, id, Msg::MustWait);
    }
    Handled::Wait
}

/// TRACK {name} follows the trail of a player or monster.
///
/// Ends resting and meditating. The name is looked for in the room (monsters, then other
/// players), then among everyone in the realm and the active monsters held in memory
/// ([`find_any_action_target`]); a player is tracked by [`track_player`], a monster by
/// [`track_monster`], anything else fails. An attempt costs a tick of delay and answers 2
/// (no prompt); an ambiguous name shows the choices (1).
pub fn cmd_track(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player_mut(world, id) else {
        return Handled::NotCommand;
    };
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    if line.argc() == 1 {
        tell_user(world, id, Msg::TrackSyntax);
        return Handled::Done;
    }
    let name = line.rest_from(1);
    let found = find_action_target(world, id, name, true);
    // several matches: the choices
    if let Found::Multiple(options) = found {
        show_options(world, id, &options);
        return Handled::Done;
    }
    clear_output(world, id);
    let target = match found {
        Found::User(t) => Some(Tracked::Player(t)),
        Found::Monster(m) => Some(Tracked::Monster(m)),
        _ => match find_any_action_target(world, name) {
            Target::User(t) => Some(Tracked::Player(t)),
            Target::Monster => find_monster_in_memory(world, name).map(Tracked::Monster),
            Target::None | Target::Multiple(_) => None,
        },
    };
    match target {
        Some(Tracked::Player(t)) => track_player(world, t, id),
        Some(Tracked::Monster(m)) => track_monster(world, m, id),
        None => tell_user(world, id, Msg::TrackFail),
    }
    add_delay(world, id, 1);
    Handled::Wait
}

/// What TRACK follows.
enum Tracked {
    Player(SessionId),
    Monster(ActiveMonsterId),
}

/// The exits of the tracker's room that lead to `next`, each found with a roll under the
/// tracking skill: `true` when one was shown.
fn track_exits(world: &mut World, id: SessionId, next: RoomId, who: &str) -> bool {
    let Some(plr) = player(world, id) else {
        return false;
    };
    let (key, tracking) = (room_of(plr), i32::from(plr.tracking));
    let Some(exits) = live_exits(world, key) else {
        return false;
    };
    let mut found = false;
    for (j, e) in exits.iter().enumerate() {
        if e.target.0 != 0 && e.target == next && world.rng.range(0, 100) < tracking {
            found = true;
            let dir = world.display.dirnames.get(j).to_owned();
            tell_user(
                world,
                id,
                Msg::TrackWent {
                    who: who.to_owned(),
                    dir,
                },
            );
        }
    }
    found
}

/// Every time the target's trail of the last `NTRAIL` rooms passes through the tracker's room, each
/// exit to the room he went to next gets a roll; no find, the tracking fails.
pub fn track_player(world: &mut World, target: SessionId, id: SessionId) {
    let (Some(tplr), Some(plr)) = (player(world, target), player(world, id)) else {
        tell_user(world, id, Msg::TrackFail);
        return;
    };
    let (rooms, maps, name) = (tplr.trailrooms, tplr.trailmaps, tplr.firstname.clone());
    let here = room_of(plr);
    let mut found = false;
    for i in 1..NTRAIL {
        if rooms[i] == here.room && maps[i] == here.map {
            found |= track_exits(world, id, rooms[i - 1], &name);
        }
    }
    if !found {
        tell_user(world, id, Msg::TrackFail);
    }
}

/// [`track_player`] over the monster's trail of `NAMTRAIL` rooms, by room number only (the map is
/// not compared).
pub fn track_monster(world: &mut World, monster: ActiveMonsterId, id: SessionId) {
    let (Some(mon), Some(plr)) = (crate::combat::monster(world, monster), player(world, id)) else {
        tell_user(world, id, Msg::TrackFail);
        return;
    };
    let (trail, name) = (mon.trail, mon.name.clone());
    let here = room_of(plr);
    let mut found = false;
    for i in 1..NAMTRAIL {
        if trail[i] == here.room {
            found |= track_exits(world, id, trail[i - 1], &name);
        }
    }
    if !found {
        tell_user(world, id, Msg::TrackFail);
    }
}

/// TAME: not implemented in 1.11p, so the word is not a command.
pub fn cmd_tame(_world: &mut World, _id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    Handled::NotCommand
}

/// MESMERIZE: not implemented in 1.11p, so the word is not a command.
pub fn cmd_mesmerize(
    _world: &mut World,
    _id: SessionId,
    _cmd: Cmd,
    _line: &CommandLine,
) -> Handled {
    Handled::NotCommand
}

/// ROB: [`crate::combat::rob::cmd_rob`] (T-033).
pub fn cmd_rob(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    crate::combat::rob::cmd_rob(world, id, cmd, line)
}

/// `SUICIDE`, `SUICIDE NOW` and `REROLL`.
///
/// Refused during a tournament and in a Colliseum room while the arena switch is on; more
/// words than `SUICIDE x`, or any after `REROLL`, are no command of ours. Otherwise dragging,
/// sneaking, an old `ST_SUICIDE` and hiding are cleared, `ST_SUICIDE` is set for `SUICIDE NOW` and
/// `REROLL` (the character is then deleted, not a life lost), and a player with a suicide password
/// is asked for it (`SUBST_SUICIDE`, `state::suicide`); one without goes straight to
/// `player::suicide::commit_suicide`. No prompt follows a password question or a return to the
/// menu.
pub fn cmd_suicide(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    // no suicide during a tournament
    if world.config.tournament_play {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::SuicideTournament);
        }
        return Handled::Done;
    }
    // too many words
    if line.argc() > 2 || (cmd == Cmd::Reroll && line.argc() != 1) {
        return Handled::NotCommand;
    }
    let Some(key) = world
        .session(id)
        .and_then(|s| s.player())
        .map(|p| RoomKey::new(p.map.0, p.room.0))
    else {
        return Handled::NotCommand;
    };
    // the arena
    let colliseum = world
        .room(key)
        .is_some_and(|rm| rm.room_type == RoomType::Colliseum);
    if colliseum && world.display.arena_safe {
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::SuicideRoomRefused);
        }
        return Handled::Done;
    }
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    let Some(plr) = s.character.as_mut() else {
        return Handled::NotCommand;
    };
    // dragging, sneaking, an old suicide flag and hiding are cleared
    plr.status
        .remove(PlayerStatus::DRAGGING | PlayerStatus::SNEAKING | PlayerStatus::SUICIDE);
    plr.hidden = 0;
    // (an abbreviation: "n", "no" and "now" all do)
    if (line.argc() == 2 && starts_with_ignore_case(line.arg(1), "now")) || cmd == Cmd::Reroll {
        plr.status.insert(PlayerStatus::SUICIDE);
    }
    if !plr.suicpw.is_empty() {
        // the password, shown as stars
        s.output.push(Msg::SuicideEnterPassword);
        s.state = SessionState::Suicide(SuicideStep::Confirm);
        world.outbox.push(EngineOutput::SetEcho {
            id,
            mode: EchoMode::Secure {
                ch: b'*',
                width: suicide_pw::MAX_LEN as u8,
            },
        });
    } else {
        // no password: straight to the deed
        let today = world.today;
        if !commit_suicide(world, id, today).gone() {
            return Handled::Done;
        }
        if let Some(s) = world.session_mut(id) {
            s.state = SessionState::MainMenu;
        }
    }
    Handled::Wait
}

/// DISARM TRAP {direction}; the dispatcher sends any other DISARM to UNREADY.
///
/// Only an armed Trap or Spell Trap (`exitpar2s` 0, or 3 for one that carries the player
/// through) can be disarmed: the player's `disarm` must beat a roll of 0 to 99. A success
/// marks the exit disarmed (0 to 1, 3 to 4) and queues it to be armed again; a Spell Trap
/// also shows its disarm message (`exitpar3s`). A failed Spell Trap shows its "goes off"
/// message (`exitpar4s`) and then throws the player through the exit (state 3) or casts
/// `exitpar1s` on him; a Trap missed by less than 10 only fails, a worse miss goes off the
/// same way with damage from `exitpar1s/2` to `exitpar1s`
/// Anything else fails, except a Spell Trap in another state, which says nothing (sic).
/// Always handled.
pub fn cmd_disarm(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::Done;
    };
    let (here, name, skill) = (room_of(plr), plr.firstname.clone(), i32::from(plr.disarm));
    // the third word as a direction
    let Some(dir) = parse_command(line.arg(2)).and_then(|c| direction(c.code() as i16)) else {
        return Handled::Done;
    };
    let Some(exit) = live_exit(world, here, dir) else {
        return Handled::Done;
    };
    let dname = dirname(world, dir);
    let beyond = RoomKey {
        map: here.map,
        room: exit.target,
    };
    let armed = matches!(exit.par2, 0 | 3);
    let spell = match exit.kind {
        ExitType::SpellTrap if armed => true,
        ExitType::SpellTrap => return Handled::Done,
        ExitType::Trap if armed => false,
        _ => {
            tell_user(world, id, Msg::DisarmFailed { dir: dname });
            return Handled::Done;
        }
    };
    let roll = world.rng.range(0, 100);
    if skill > roll {
        // disarmed
        tell_user(world, id, Msg::DisarmOk { dir: dname });
        set_par2(world, here, dir, if exit.par2 == 0 { 1 } else { 4 });
        if spell {
            trap_message(world, id, &name, here, beyond, exit.par3);
        }
        add_exit_to_list(world, here, dir, 1);
        return Handled::Done;
    }
    if spell {
        tell_user(world, id, Msg::DisarmFailed { dir: dname });
    } else if skill > roll - 10 {
        // a near miss
        tell_user(world, id, Msg::DisarmFailed { dir: dname });
        return Handled::Done;
    }
    // the trap goes off
    trap_message(world, id, &name, here, beyond, exit.par4);
    if exit.par2 == 3 {
        crate::movement::move_user(world, id, dir.code() as i16, MoveType::TrapOnPurpose.code());
        return Handled::Done;
    }
    let wasup = player(world, id).is_some_and(|p| p.hp > 0);
    if spell {
        HOOKS.spells.room_cast_on_user(world, id, exit.par1 as i32);
    } else {
        let par1 = exit.par1 as i32;
        let damage = world.rng.range(par1 / 2, par1 + 1);
        if let Some(p) = player_mut(world, id) {
            p.hp = (i32::from(p.hp) - damage) as _;
        }
    }
    if wasup && player(world, id).is_some_and(|p| p.hp <= 0) {
        crate::combat::knock_out_user(world, id);
    }
    Handled::Done
}

/// A trap's WCCMSG message `n`, when there is one: `msgs[1]` to the player's room and `msgs[2]` to
/// the room behind the exit, each with the player's name in green, then `msgs[0]` to the player as
/// it stands.
fn trap_message(
    world: &mut World,
    id: SessionId,
    name: &str,
    here: RoomKey,
    beyond: RoomKey,
    n: i32,
) {
    let Some(m) = world.messages.get(&MessageNo(n)).map(|m| m.msgs.clone()) else {
        return;
    };
    let who = green_name(name);
    let text = fmt_msg(&m[1], &[Arg::S(&who)]);
    tell_room_all(world, here, [Some(id), None], Msg::ExitTextYellow { text });
    let text = fmt_msg(&m[2], &[Arg::S(&who)]);
    tell_room_all(
        world,
        beyond,
        [Some(id), None],
        Msg::ExitTextYellow { text },
    );
    tell_user(world, id, Msg::ExitTextGreen { text: m[0].clone() });
}

/// GUARD: not implemented, so the word is not a command.
pub fn cmd_guard(_world: &mut World, _id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    Handled::NotCommand
}

/// SNEAK alone starts sneaking.
///
/// Rest, meditation and any earlier sneak end first. Refused (with a tick of delay) while
/// [`can_sneak`] says no; `SNEAK {word}` is no command of ours. Otherwise autocombat is broken off
/// and, with the delay run out, the attempt costs a tick: Perfect Stealth always works, anyone else
/// needs a roll of 0 to 99 under the hide/sneak chance (at most 95). A failure looks like a success
/// unless a second roll comes in under the player's perception. More words show the syntax. A sysop
/// with debugging on is told when Perfect Stealth made the sneak.
pub fn cmd_sneak(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player_mut(world, id) else {
        return Handled::NotCommand;
    };
    plr.status
        .remove(PlayerStatus::RESTING | PlayerStatus::SNEAKING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    // not now
    if !can_sneak(world, id) {
        tell_user(world, id, Msg::SneakNotNow);
        add_delay(world, id, 1);
        return Handled::Done;
    }
    if line.argc() == 2 {
        return Handled::NotCommand;
    }
    let mut rc = Handled::Done;
    let perfect = player(world, id).is_some_and(|p| user_has(world, p, Ability::PerfectStealth));
    if line.argc() == 1 {
        // autocombat is broken off
        if HOOKS.combat.in_autocombat(world, id) {
            HOOKS.combat.display_autocombat_broken(world, id);
        }
        HOOKS.combat.kill_autocombat(world, id);
        if delay_done(world, id) {
            rc = Handled::Wait;
            add_delay(world, id, 1);
            tell_user(world, id, Msg::SneakAttempt);
            let end = if perfect {
                sneak_on(world, id);
                Msg::StealthLineEnd
            } else {
                let chance = calc_hide_sneak_chance(world, id, 95, Some(id));
                let perception = player(world, id).map_or(0, |p| i32::from(p.perception));
                if world.rng.range(0, 100) < chance {
                    sneak_on(world, id);
                    Msg::StealthLineEnd
                } else if world.rng.range(0, 100) >= perception {
                    Msg::StealthLineEnd
                } else {
                    Msg::SneakNotSneaking
                }
            };
            tell_user(world, id, end);
        } else {
            tell_user(world, id, Msg::SneakMustWait);
        }
    } else {
        tell_user(world, id, Msg::SneakSyntax);
    }
    // a sysop with debugging on hears of Perfect Stealth
    let sneaking = player(world, id).is_some_and(|p| p.status.contains(PlayerStatus::SNEAKING));
    if debug_on(world, id) && perfect && sneaking {
        tell_user(world, id, Msg::PerfectStealth);
    }
    rc
}

/// Starts sneaking.
fn sneak_on(world: &mut World, id: SessionId) {
    if let Some(p) = player_mut(world, id) {
        p.status.insert(PlayerStatus::SNEAKING);
    }
}

/// Not while a player in the room fights him (a backstab aside), while anything engages him, or
/// while a monster here could attack him.
pub fn can_sneak(world: &mut World, id: SessionId) -> bool {
    let Some(plr) = player(world, id) else {
        return false;
    };
    let attackers = plr.attackers;
    if is_being_attacked(world, id)
        && attacker_type(world, id) != Some(Move::Backstab)
        && attacker_is_in_same_room(world, id)
    {
        return false;
    }
    if attackers > 0 {
        return false;
    }
    !HOOKS.combat.monster_could_attack(world, id)
}

/// PICKLOCK: T-026's `movement::doors::cmd_picklock`.
pub fn cmd_picklock(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    crate::movement::doors::cmd_picklock(world, id, line)
}

/// SEARCH: T-026's `movement::search::cmd_search`.
pub fn cmd_search(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    crate::movement::search::cmd_search(world, id, line)
}

/// AID: [`super::group::cmd_aid`].
pub fn cmd_aid(world: &mut World, id: SessionId, cmd: Cmd, line: &CommandLine) -> Handled {
    super::group::cmd_aid(world, id, cmd, line)
}

/// FORGIVE: T-038's `commands::evil::cmd_forgive`.
pub fn cmd_forgive(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    super::evil::cmd_forgive(world, id, line)
}

/// REST. T-037.
pub fn cmd_rest(world: &mut World, id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    super::rest::cmd_rest(world, id);
    Handled::Done
}

/// MEDITATE. T-037.
pub fn cmd_meditate(world: &mut World, id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    super::rest::cmd_meditate(world, id);
    Handled::Done
}

/// FORGET: the body is `comms::forget` (T-045).
pub fn cmd_forget(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    crate::comms::forget::cmd_forget(world, id, line)
}
