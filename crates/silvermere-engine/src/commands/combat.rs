//! Combat commands: ATTACK, BACKSTAB, KILLBLOW, SMASH, BASH, JUMPKICK, KICK, PUNCH and BREAK.
//!
//! Each attack command picks its attack move and type and hands over to [`cmd_any_attack`], the
//! shared front half: the target is looked up, the refusals are made and `combat::attack_user_user`
//! or `combat::attack_user_monster` starts the fight. The blows come with the next auto-combat
//! round (T-034).

use silvermere_rules::combat::Move;
use silvermere_text::Msg;
use silvermere_types::enums::Ability;
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus, PlayerStatus2, RoomFlags};
use silvermere_types::ids::{ActiveMonsterId, SessionId};

use super::dispatch::Handled;
use super::{Cmd, CommandLine, eq_ignore_case};
use crate::combat::{
    self, Mode, debug_on, display_autocombat_broken, is_combatting_user, is_inside_autocombat,
    kill_autocombat, player, player_mut, restart_autocombat, room_of, user_has,
};
use crate::items::inventory::{matches_any_word, show_options};
use crate::output::clear_output;
use crate::tell::tell_user;
use crate::world::World;

/// The most passes a name search makes, dropping a trailing word each time.
const MAX_PASSES: usize = 50;

/// What [`find_action_target`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Nothing.
    None,
    /// A player in the room.
    User(SessionId),
    /// A monster in the room.
    Monster(ActiveMonsterId),
    /// Several matches: the "which one?" names, to be shown.
    Multiple(Vec<String>),
}

/// The monster and player steps of the action target search, not the searcher, with charmed
/// monsters last unless `rob` (the attack commands search that way; ROB does not): the active
/// monsters of the room (charmed ones on a second pass when they come last), each pass dropping the
/// last word of the name and trying again; then the players in the room by the first word. A full
/// name (before any word was dropped) ends the search at once. With several candidates and the last
/// kind found a player, the list is shown; the first monster found or the last player found is
/// returned.
///
/// A stand-in until T-046's version of the whole search is shared.
pub fn find_action_target(world: &World, id: SessionId, name: &str, rob: bool) -> Found {
    let Some(plr) = player(world, id) else {
        return Found::None;
    };
    let key = room_of(plr);
    let Some(rs) = world.room_state.get(&key) else {
        return Found::None;
    };
    let charm_last = !rob;
    let mut count = 0usize;
    let mut options: Vec<String> = Vec::new();
    let mut found: Option<Found> = None;
    let mons: Vec<_> = rs
        .monsters
        .iter()
        .filter(|n| n.0 != 0)
        .filter_map(|&n| combat::monster(world, n))
        .collect();
    let passes: &[Option<bool>] = if charm_last {
        &[Some(false), Some(true)]
    } else {
        &[None]
    };
    for &charmed in passes {
        let mut words: Vec<&str> = name.split(' ').collect();
        let mut dropped = 0;
        for _ in 0..MAX_PASSES {
            let typed = words.join(" ");
            for m in &mons {
                if let Some(c) = charmed
                    && m.flags.contains(ActiveMonsterFlags::CHARMED) != c
                {
                    continue;
                }
                if matches_any_word(&typed, &m.name) {
                    if count == 0 {
                        found = Some(Found::Monster(m.number));
                    }
                    count += 1;
                    if dropped == 0 && eq_ignore_case(&typed, &m.name) {
                        return Found::Monster(m.number);
                    }
                    options.push(m.name.clone());
                }
            }
            if words.len() <= 1 {
                break;
            }
            words.pop();
            dropped += 1;
        }
    }
    let mut last_monster = found.is_some();
    // players, by the first word only
    let first = name.split(' ').next().unwrap_or("");
    let sees_hidden = plr.status.contains(PlayerStatus::SEEHIDDEN);
    let mut ids = world.session_ids();
    ids.sort();
    for other in ids {
        if other == id || !world.is_in_game(other) {
            continue;
        }
        let Some(v) = player(world, other) else {
            continue;
        };
        if v.room != key.room || v.map != key.map || !matches_any_word(first, &v.firstname) {
            continue;
        }
        if v.hidden != 0 && !sees_hidden {
            continue;
        }
        count += 1;
        found = Some(Found::User(other));
        last_monster = false;
        if eq_ignore_case(first, &v.firstname) {
            return Found::User(other);
        }
        options.push(v.firstname.clone());
    }
    if count > 1 && !last_monster {
        return Found::Multiple(options);
    }
    found.unwrap_or(Found::None)
}

/// The shared attack command, with attack move `mv` and attack type `mtype`.
pub fn cmd_any_attack(
    world: &mut World,
    id: SessionId,
    line: &CommandLine,
    mv: Move,
    mtype: Move,
) -> Handled {
    let Some(plr) = player(world, id).cloned() else {
        return Handled::NotCommand;
    };
    // a frightened player cannot start a fight
    if plr.status2.contains(PlayerStatus2::AFRAID) {
        if let Some(p) = player_mut(world, id) {
            p.status
                .remove(PlayerStatus::JUSTARRIVED | PlayerStatus::RESTING);
            p.status2.remove(PlayerStatus2::MEDITATING);
        }
        tell_user(world, id, Msg::CombatTooAfraid);
        return Handled::Done;
    }
    let prepare = |world: &mut World| {
        if let Some(p) = player_mut(world, id) {
            p.status
                .remove(PlayerStatus::JUSTARRIVED | PlayerStatus::RESTING);
            p.status2.remove(PlayerStatus2::MEDITATING);
            if mv != Move::Backstab {
                p.hidden = 0;
                p.status.remove(PlayerStatus::SNEAKING);
            }
        }
    };
    if line.argc() < 2 {
        // carry on against the old target
        prepare(world);
        if is_inside_autocombat(world, id) {
            display_autocombat_broken(world, id);
        }
        kill_autocombat(world, id);
        restart_autocombat(world, id);
        return Handled::Done;
    }
    // the encumbrance is brought up to date
    crate::movement::move_user::get_encumbrance_percent(world, id);
    let key = room_of(&plr);
    let found = find_action_target(world, id, line.rest_from(1), false);
    // the match lookup ends (the colour reset is dropped) unless the name was ambiguous
    if !matches!(found, Found::Multiple(_)) {
        clear_output(world, id);
    }
    match found {
        Found::Multiple(options) => {
            show_options(world, id, &options);
            Handled::Done
        }
        Found::None => Handled::NotCommand,
        Found::User(target) => {
            let Some(rm) = world.room(key).cloned() else {
                return Handled::Done;
            };
            prepare(world);
            if target == id && !debug_on(world, id) {
                tell_user(world, id, Msg::CombatAttackSelf);
                return Handled::Done;
            }
            if guilty(world, id, &rm) {
                return Handled::Done;
            }
            // (sic) the attack is announced either way
            let _ = is_combatting_user(world, id, target);
            let announce = true;
            if is_inside_autocombat(world, id) {
                display_autocombat_broken(world, id);
            }
            kill_autocombat(world, id);
            let hidden = player(world, target).is_some_and(|t| t.hidden != 0);
            if hidden && !plr.status.contains(PlayerStatus::SEEHIDDEN) {
                return Handled::NotCommand;
            }
            if let Some(t) = player_mut(world, target) {
                t.status.remove(PlayerStatus::RESTING);
                t.status2.remove(PlayerStatus2::MEDITATING);
            }
            combat::attack_user_user(world, id, target, mv, mtype, announce, Mode::Command);
            Handled::Done
        }
        Found::Monster(monnum) => {
            let Some(rm) = world.room(key).cloned() else {
                return Handled::Done;
            };
            prepare(world);
            if guilty(world, id, &rm) {
                return Handled::Done;
            }
            if is_inside_autocombat(world, id) {
                display_autocombat_broken(world, id);
            }
            kill_autocombat(world, id);
            combat::attack_user_monster(world, id, monnum, mv, mtype, Mode::Command);
            Handled::Done
        }
    }
}

/// The protected-room refusal: the room's own reason or the feeling of guilt.
fn guilty(world: &mut World, id: SessionId, rm: &silvermere_types::records::Room) -> bool {
    if world.config.tournament_play
        || !rm.flags.contains(RoomFlags::PROTECTED)
        || debug_on(world, id)
    {
        return false;
    }
    tell_user(world, id, Msg::CombatGuiltClr);
    if rm.protmsg.0 != 0 {
        // the room's text block, with no substitution
        crate::display::long_text::display_long_text(world, id, rm.protmsg, None);
    } else {
        tell_user(world, id, Msg::CombatGuilt);
    }
    true
}

/// Whether the player is hiding or sneaking.
fn sneaky(world: &World, id: SessionId) -> bool {
    player(world, id).is_some_and(|p| p.hidden != 0 || p.status.contains(PlayerStatus::SNEAKING))
}

/// PUNCH needs the Punch ability; hidden or sneaking without a weapon it is a backstab.
pub fn cmd_punch(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if !user_has(world, plr, Ability::Punch) {
        return Handled::NotCommand;
    }
    let mv = if sneaky(world, id) && plr.weapon.0 == 0 {
        Move::Backstab
    } else {
        Move::Punch
    };
    cmd_any_attack(world, id, line, mv, mv)
}

/// KICK needs the Kick ability.
pub fn cmd_kick(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    match player(world, id) {
        Some(plr) if user_has(world, plr, Ability::Kick) => {
            cmd_any_attack(world, id, line, Move::Kick, Move::Kick)
        }
        _ => Handled::NotCommand,
    }
}

/// JUMPKICK needs the Jumpkick ability.
pub fn cmd_jumpkick(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    match player(world, id) {
        Some(plr) if user_has(world, plr, Ability::Jumpkick) => {
            cmd_any_attack(world, id, line, Move::JumpKick, Move::JumpKick)
        }
        _ => Handled::NotCommand,
    }
}

/// BASH: resting and meditating end; BASH alone restarts auto-combat; `BASH <direction>` at a door
/// or gate is the door bash (T-026, `movement::doors`); otherwise the attack, which needs the Bash
/// ability and a wielded weapon.
pub fn cmd_bash(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player_mut(world, id) else {
        return Handled::NotCommand;
    };
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    if line.argc() == 1 {
        kill_autocombat(world, id);
        restart_autocombat(world, id);
        return Handled::Done;
    }
    if let Some(handled) = crate::movement::doors::cmd_bash_door(world, id, line) {
        return handled;
    }
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if !user_has(world, plr, Ability::Bash) {
        tell_user(world, id, Msg::CombatBashGate);
        return Handled::Done;
    }
    if plr.weapon.0 == 0 {
        tell_user(world, id, Msg::CombatBashNoWeapon);
        return Handled::Done;
    }
    cmd_any_attack(world, id, line, Move::Bash, Move::Bash)
}

/// SMASH needs the Smash ability.
pub fn cmd_smash(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if user_has(world, plr, Ability::Smash) {
        cmd_any_attack(world, id, line, Move::Smash, Move::Smash)
    } else {
        tell_user(world, id, Msg::CombatSmashGate);
        Handled::Done
    }
}

/// KILLBLOW: a command word without an effect; always "not a game command".
pub fn cmd_killblow(_world: &mut World, _id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    Handled::NotCommand
}

/// BACKSTAB: a backstab only from hiding or sneaking; a wielded weapon must have "Alter
/// Backstab Attack Value", else the blow is a normal one while the attack type still says
/// backstab.
pub fn cmd_backstab(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    if !sneaky(world, id) {
        return cmd_any_attack(world, id, line, Move::Attack, Move::Attack);
    }
    if plr.weapon.0 == 0 {
        return cmd_any_attack(world, id, line, Move::Backstab, Move::Backstab);
    }
    let Some(itm) = world.items.get(&plr.weapon) else {
        return Handled::Done;
    };
    if combat::item_has(itm, Ability::BackstabAv) {
        cmd_any_attack(world, id, line, Move::Backstab, Move::Backstab)
    } else {
        tell_user(world, id, Msg::CombatNoBackstabWeapon);
        cmd_any_attack(world, id, line, Move::Attack, Move::Backstab)
    }
}

/// ATTACK: a plain attack; bare-handed with the Punch ability a punch, or a backstab from hiding.
pub fn cmd_attack(world: &mut World, id: SessionId, _cmd: Cmd, line: &CommandLine) -> Handled {
    let Some(plr) = player(world, id) else {
        return Handled::NotCommand;
    };
    let mut mv = Move::Attack;
    if plr.weapon.0 == 0 && user_has(world, plr, Ability::Punch) {
        mv = if sneaky(world, id) {
            Move::Backstab
        } else {
            Move::Punch
        };
    }
    cmd_any_attack(world, id, line, mv, mv)
}

/// BREAK: ends resting and meditating, breaks auto-combat (`*Combat Off*`, and `<Name> breaks off
/// combat.` to the room), stops dragging and sneaking with a line each.
pub fn cmd_break(world: &mut World, id: SessionId, _cmd: Cmd, _line: &CommandLine) -> Handled {
    let Some(s) = world.session_mut(id) else {
        return Handled::NotCommand;
    };
    let dragging = s.party.dragging;
    let Some(plr) = s.character.as_mut() else {
        return Handled::NotCommand;
    };
    plr.status.remove(PlayerStatus::RESTING);
    plr.status2.remove(PlayerStatus2::MEDITATING);
    // the room is told too
    if is_inside_autocombat(world, id) {
        combat::autocombat::display_autocombat_broken(world, id, true);
    }
    kill_autocombat(world, id);
    let Some(plr) = player_mut(world, id) else {
        return Handled::NotCommand;
    };
    let was_dragging = plr.status.contains(PlayerStatus::DRAGGING);
    let was_sneaking = plr.status.contains(PlayerStatus::SNEAKING);
    plr.status
        .remove(PlayerStatus::DRAGGING | PlayerStatus::SNEAKING);
    if was_dragging {
        let who = dragging
            .and_then(|d| world.session(d))
            .and_then(|o| o.character.as_ref())
            .map(|c| c.firstname.clone())
            .unwrap_or_default();
        if let Some(s) = world.session_mut(id) {
            s.output.push(Msg::NoLongerDragging { who });
        }
    }
    if was_sneaking && let Some(s) = world.session_mut(id) {
        s.output.push(Msg::NoLongerSneaking);
    }
    Handled::Done
}
