//! The four attack routines: a player on a monster, a monster on a player, a player on a
//! player and a monster on a monster.
//!
//! Each keeps the original's checks, message order and points of view; they share the
//! fighter builders and verb picking of [`super`] and the texts of [`super::messages`].
//! Every blow is one `rules::combat::resolve`; a round is at most [`MAX_BLOWS`] of them
//! while the attacker's energy lasts. A function returns the outcome of the last blow
//! struck, or `None` when nothing was fought or the round was cut short.

use silvermere_rules::Rng;
use silvermere_rules::combat::{self, AlignGroup, Fighter, MAX_BLOWS, Move, Outcome, resolve};
use silvermere_text::{Msg, ansi};
use silvermere_types::enums::{Ability, Alignment, MonsterAttackType, MonsterType, RoomType};
use silvermere_types::flags::{
    ActiveMonsterFlags, PlayerSettings, PlayerStatus, PlayerStatus2, RoomFlags,
};
use silvermere_types::ids::{ActiveMonsterId, RoomKey, SessionId, SpellNo};

use super::messages::{
    CombatMsg, SlotTexts, monster_on_player, player_on_monster, player_on_player,
};
use super::{
    AutoTarget, Combatant, EvilKind, add_evil_points, already_evil, auto_use, check_confusion,
    check_kill_monster, check_kill_user, check_monster_confusion, debug_on,
    display_autocombat_broken, engage_autocombat, interrupt_output, is_combatting_user,
    item_ability, item_has, kill_autocombat, kill_autocombat_against_monster, knock_out_user,
    monster, monster_cast, monster_cast_random_spell, monster_group, monster_has, monster_mut,
    monster_side, pick_random_attack_move, player, player_mut, player_side, possessive_pronoun,
    room_of, subject_pronoun, user_ability, user_group,
};
use crate::commands::parse::eq_ignore_case;
use crate::tell::{Noise, TellBlind, tell_room, tell_user};
use crate::world::World;

/// Whether the call is the attack command itself or an auto-combat round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The command starts the fight; no blow is struck.
    Command,
    /// `do_autocombat_for_user` (T-034) runs the round's blows.
    Auto,
}

/// ANSI strings as the text of a message argument.
fn ansi(parts: &[&[u8]]) -> String {
    parts.iter().map(|p| String::from_utf8_lossy(p)).collect()
}

/// The lowest level a player may fight another player at.
pub const MIN_PVP_LEVEL: u16 = 4;
/// Evil points from which a player is a FIEND.
const FIEND_EVIL: i16 = 210;

/// The message, then the prompt, to one player.
fn say_prompt(world: &mut World, id: SessionId, msg: Msg) {
    if let Some(s) = world.session_mut(id) {
        s.output.push(msg);
        s.push_prompt();
    }
}

/// A message, with a prompt, to everyone in the room but the players in `except`.
fn room_say(
    world: &mut World,
    room: RoomKey,
    except: [Option<SessionId>; 2],
    noise: Noise,
    msg: &Msg,
) {
    tell_room(world, room, except, noise, TellBlind::All, true, None, msg);
}

/// The channel number a monster's `target` holds for a session.
fn channel(id: SessionId) -> i16 {
    id.0 as i16
}

/// A damage shield's sting: 1 to `dmg + 1`, at least 1.
fn shield_roll(world: &mut World, dmg: i32) -> i32 {
    let hi = if dmg + 1 < 1 { 1 } else { dmg + 1 };
    world.rng.range(1, hi)
}

/// The smash penalty taken off a downed fighter's AV and DV, not below 0.
fn down_penalty(f: &mut Fighter, penalty: i32) {
    f.av = if penalty > f.av { 0 } else { f.av - penalty };
    f.dv = if penalty > f.dv { 0 } else { f.dv - penalty };
}

/// The monster may start to follow the player: always a neutral, good or lawful good one,
/// any other by its follow percentage; not a guard already following someone. The roll is
/// made first, before the alignment test, so it is drawn whether or not it decides.
fn maybe_follow(world: &mut World, monnum: ActiveMonsterId, name: &str) {
    let roll = world.rng.range(1, 100);
    let Some(mon) = monster_mut(world, monnum) else {
        return;
    };
    let goodish = matches!(
        mon.alignment,
        Alignment::Neutral | Alignment::Good | Alignment::LawfulGood
    );
    if (roll < i32::from(mon.followpct) || goodish)
        && (mon.montype != MonsterType::Guard || mon.following.is_empty())
    {
        mon.following = name.to_owned();
        mon.friendly = 0;
    }
}

/// A "Required to hit" monster is hurt only by a weapon with exactly its value.
fn user_can_hit_monster(world: &World, id: SessionId, monnum: ActiveMonsterId) -> bool {
    let (Some(plr), Some(mon)) = (player(world, id), monster(world, monnum)) else {
        return false;
    };
    let req = super::monster_ability(world, mon, Ability::ReqToHit);
    if req == 0 {
        return true;
    }
    if plr.weapon.0 == 0 {
        return false;
    }
    world
        .items
        .get(&plr.weapon)
        .is_some_and(|w| item_ability(w, Ability::ReqToHit) == req)
}

/// The "Magical" total and the "Guarded by Monster" number of a known monster's slots.
fn magic_and_guard(world: &World, monnum: ActiveMonsterId) -> (i32, i32) {
    let Some(kmon) = monster(world, monnum).and_then(|m| world.monsters.get(&m.knownnum)) else {
        return (0, 0);
    };
    let mut magreq = 0;
    let mut guard = 0;
    for &(abl, val) in &kmon.abilities {
        match abl {
            Ability::Magical => magreq += i32::from(val),
            Ability::Guarded => guard = i32::from(val),
            _ => {}
        }
    }
    (magreq, guard)
}

/// The damage shields of a monster's items and spells on the attacking player. Returns
/// `(hurt, below_zero)`.
fn monster_shields(world: &mut World, id: SessionId, monnum: ActiveMonsterId) -> (bool, bool) {
    let Some(mon) = monster(world, monnum).cloned() else {
        return (false, false);
    };
    let name = player(world, id)
        .map(|p| p.firstname.clone())
        .unwrap_or_default();
    let room = player(world, id).map(room_of);
    let mut hurt = false;
    let mut below = false;
    let mut stings: Vec<(i32, i32)> = Vec::new();
    for &(item, _) in &mon.items {
        if item.0 == 0 {
            continue;
        }
        let Some(itm) = world.items.get(&item) else {
            continue;
        };
        let dmg = item_ability(itm, Ability::DamageShield);
        if dmg != 0 {
            stings.push((dmg, item_ability(itm, Ability::ShockMsg)));
        }
    }
    for cast in &mon.spells {
        if cast.spell == SpellNo(0) {
            continue;
        }
        let Some(spl) = world.spells.get(&cast.spell) else {
            continue;
        };
        let mut dmg = silvermere_rules::abilities::spell_ability_value(spl, Ability::DamageShield);
        if dmg == 0 && silvermere_rules::abilities::spell_has_ability(spl, Ability::DamageShield) {
            dmg = i32::from(cast.value);
        }
        if dmg != 0 {
            stings.push((
                dmg,
                silvermere_rules::abilities::spell_ability_value(spl, Ability::ShockMsg),
            ));
        }
    }
    for (dmg, msgnum) in stings {
        let dmg = shield_roll(world, dmg);
        if let Some(plr) = player_mut(world, id) {
            plr.hp = plr.hp.wrapping_sub(dmg as i16);
            if plr.hp < 0 {
                below = true;
            }
        }
        hurt = true;
        shock(
            world,
            id,
            room,
            msgnum,
            &[ShockArg::D(dmg)],
            &[ShockArg::S(name.clone()), ShockArg::D(dmg)],
            None,
            dmg,
            true,
        );
    }
    (hurt, below)
}

/// An argument of a Shockshield Message text.
#[derive(Debug, Clone)]
enum ShockArg {
    S(String),
    D(i32),
}

fn args<'a>(a: &'a [ShockArg]) -> Vec<crate::movement::exits::Arg<'a>> {
    use crate::movement::exits::Arg;
    a.iter()
        .map(|x| match x {
            ShockArg::S(s) => Arg::S(s),
            ShockArg::D(d) => Arg::D(i64::from(*d)),
        })
        .collect()
}

/// The Shockshield Message of a sting (text 2 to the stung player, text 3 to the room),
/// or "You took %d damage." (`pvp` "!") when there is none.
#[allow(clippy::too_many_arguments)]
fn shock(
    world: &mut World,
    id: SessionId,
    room: Option<RoomKey>,
    msgnum: i32,
    you: &[ShockArg],
    others: &[ShockArg],
    except2: Option<SessionId>,
    dmg: i32,
    period: bool,
) {
    let msg = if msgnum != 0 {
        world
            .messages
            .get(&silvermere_types::ids::MessageNo(msgnum))
            .map(|m| m.msgs.clone())
    } else {
        None
    };
    match msg {
        Some(m) => {
            let t = crate::movement::exits::fmt_msg(&m[1], &args(you));
            tell_user(world, id, Msg::CombatShockText { text: t });
            if let Some(room) = room {
                let t = crate::movement::exits::fmt_msg(&m[2], &args(others));
                room_say(
                    world,
                    room,
                    [Some(id), except2],
                    Noise::Always,
                    &Msg::CombatShockText { text: t },
                );
            }
        }
        None if period => tell_user(world, id, Msg::CombatTookDamage { dmg }),
        None => tell_user(world, id, Msg::CombatTookDamagePvp { dmg }),
    }
}

/// A player attacking a monster, with the blow's move `mv` and type `mtype`.
///
/// In [`Mode::Command`] it is the attack command: the evil hook for a good monster, the
/// required weapon, a guard stepping in, Hit Magical, `*Combat Engaged*` and the room's
/// `<Name> moves to attack <monster>.`, an empty energy pool, auto-combat engaged and the
/// monster's follow roll; no blow is struck. In [`Mode::Auto`] it is one round of blows.
pub fn attack_user_monster(
    world: &mut World,
    id: SessionId,
    monnum: ActiveMonsterId,
    mv: Move,
    mtype: Move,
    mode: Mode,
) -> Option<Outcome> {
    let mut mv = mv;
    let mut monnum = monnum;
    let plr = player(world, id)?.clone();
    let mon = monster(world, monnum)?.clone();
    // both in the same room (the room number only)
    if plr.room != mon.room {
        return None;
    }
    // a round starts only with a full pool
    if mode == Mode::Auto && plr.energy < plr.maxenergy {
        engage_autocombat(world, id, AutoTarget::Monster(monnum), mv);
        return None;
    }
    if plr.hp <= 0 || plr.status2.contains(PlayerStatus2::AFRAID) {
        if mode == Mode::Auto {
            display_autocombat_broken(world, id);
        }
        return None;
    }
    // no fighting in a protected room
    let key = room_of(&plr);
    let rm = world.room(key)?;
    if !world.config.tournament_play
        && rm.flags.contains(RoomFlags::PROTECTED)
        && !debug_on(world, id)
    {
        return None;
    }
    let cycle = world.cycles.slowmoncycle;
    if let Some(m) = monster_mut(world, monnum) {
        m.flags.insert(ActiveMonsterFlags::FOUGHT);
        crate::cycles::monsters::slow_update_monster(m, cycle);
    }
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
        p.status2.insert(PlayerStatus2::ATTACKED);
    }

    if mode == Mode::Command {
        // a good monster not following the player nor fighting them
        if matches!(mon.alignment, Alignment::Good | Alignment::LawfulGood)
            && !eq_ignore_case(&mon.following, &plr.firstname)
            && mon.target != channel(id)
            && add_evil_points(world, id, None, 10, 11, EvilKind::Attack)
        {
            return None;
        }
        // both sides worked out once; only the verb picks' rolls remain
        let group = user_group(&plr);
        let _ = player_side(world, id, &plr, mv, mtype, group);
        // the required weapon
        if world.monsters.contains_key(&mon.knownnum) && !user_can_hit_monster(world, id, monnum) {
            tell_user(world, id, Msg::CombatWrongWeapon);
            return None;
        }
        let (mut magreq, guardnum) = magic_and_guard(world, monnum);
        // a guard steps in
        if guardnum != 0 {
            let protname = mon.name.clone();
            if let Some(m) = monster_mut(world, monnum) {
                m.following = plr.firstname.clone();
            }
            let slots = world
                .room_state
                .get(&key)
                .map(|s| s.monsters)
                .unwrap_or_default();
            let mut guardmon = None;
            for n in slots {
                if n.0 == 0 {
                    continue;
                }
                if let Some(g) = monster_mut(world, n)
                    && g.knownnum.0 == guardnum
                    && n != monnum
                {
                    guardmon = Some(n);
                    g.following = plr.firstname.clone();
                }
            }
            if let Some(g) = guardmon {
                monnum = g;
                let gname = monster(world, g)
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
                tell_user(
                    world,
                    id,
                    Msg::CombatProtects {
                        mon: gname,
                        who: protname,
                    },
                );
                magreq = magic_and_guard(world, g).0;
            }
        }
        // Hit Magical
        if magreq != 0 {
            let hitmag = user_ability(world, &plr, Ability::HitMagical);
            let limbs =
                plr.weapon.0 == 0 || matches!(mv, Move::Punch | Move::Kick | Move::JumpKick);
            let weapon_magical = if limbs {
                None
            } else {
                Some(
                    world
                        .items
                        .get(&plr.weapon)
                        .map_or(0, |w| item_ability(w, Ability::Magical)),
                )
            };
            if !combat::can_hit(hitmag, magreq, weapon_magical) {
                let msg = if limbs {
                    Msg::CombatNoEffectLimbs {
                        what: if matches!(mv, Move::Kick | Move::JumpKick) {
                            "feet"
                        } else {
                            "fists"
                        }
                        .into(),
                    }
                } else {
                    Msg::CombatNoEffectWeapon
                };
                tell_user(world, id, msg);
                return None;
            }
        }
        let mname = monster(world, monnum)?.name.clone();
        // *Combat Engaged*; a backstab is silent
        if mv == Move::Backstab {
            tell_user(world, id, Msg::CombatEngagedSplit);
        } else {
            tell_user(world, id, Msg::CombatEngaged);
            room_say(
                world,
                key,
                [Some(id), None],
                Noise::Always,
                &Msg::CombatMovesToAttackMon {
                    who: plr.firstname.clone(),
                    mon: mname,
                },
            );
        }
        // the first round waits for a full pool
        if let Some(p) = player_mut(world, id) {
            p.energy = 0;
        }
        if let Some(m) = monster_mut(world, monnum)
            && m.energy as i32 == i32::from(m.maxenergy)
        {
            m.energy = 0;
        }
        engage_autocombat(world, id, AutoTarget::Monster(monnum), mv);
        // the monster may follow
        let m = monster(world, monnum)?;
        if m.flags.contains(ActiveMonsterFlags::CHARMED) || m.montype == MonsterType::Angel {
            return None;
        }
        maybe_follow(world, monnum, &plr.firstname);
        return None;
    }

    // a confused player loses the round
    if check_confusion(world, id) {
        if let Some(p) = player_mut(world, id) {
            p.energy = 0;
        }
        engage_autocombat(world, id, AutoTarget::Monster(monnum), mv);
        return None;
    }
    // both sides for the first blow
    pick_random_attack_move(&mut world.rng);
    let (mut atk, verbs) = player_side(world, id, &plr, mv, mtype, AlignGroup::Neutral)?;
    let (mut def, _) = monster_side(world, &mon, 0, mv, user_group(&plr))?;
    let penalty = world.config.smash_penalty as i32;
    // a smashed monster counts down its rounds on the floor
    if mon.flags.contains(ActiveMonsterFlags::DOWN) {
        let up = monster_mut(world, monnum).map(|m| {
            m.downrounds -= 1;
            if m.downrounds <= 0 {
                m.flags.remove(ActiveMonsterFlags::DOWN);
                true
            } else {
                false
            }
        });
        if up == Some(true) {
            let msg = Msg::CombatMonRises {
                mon: mon.name.clone(),
            };
            tell_user(world, id, msg.clone());
            room_say(world, key, [Some(id), None], Noise::Always, &msg);
        } else {
            down_penalty(&mut def, penalty);
        }
    }
    // attacking brings the player out of hiding
    if let Some(p) = player_mut(world, id) {
        p.hidden = 0;
        p.status.remove(PlayerStatus::SNEAKING);
    }

    let mut verbs = verbs;
    let mut killed = false;
    let mut done = false;
    let mut hurt = 0u8;
    let mut rounds = 0;
    let mut last = None;
    while !done && !killed && rounds < MAX_BLOWS {
        hurt = 0;
        rounds += 1;
        let mon = monster(world, monnum).cloned()?;
        let res = resolve(&mut atk, &def, mv, &mut world.rng);
        last = Some(res.outcome);
        // a backstab is only the first blow
        if mv == Move::Backstab {
            mv = Move::Attack;
        }
        // the blow's energy
        let energy = player(world, id).map_or(0, |p| p.energy);
        if res.energy > energy {
            if mode == Mode::Command {
                tell_user(world, id, Msg::CombatMustRestMon);
                return None;
            }
            engage_autocombat(world, id, AutoTarget::Monster(monnum), mv);
            return None;
        }
        if let Some(p) = player_mut(world, id) {
            p.energy -= res.energy;
        }
        // the damage, at most the monster's hit points
        let mut damage = match res.outcome {
            Outcome::Hit { damage, .. } => damage,
            _ => 0,
        };
        if let Some(m) = monster_mut(world, monnum) {
            if damage > i32::from(m.hp) {
                damage = i32::from(m.hp);
            }
            m.hp -= damage as i16;
            if res.poison > i32::from(m.poison) {
                m.poison = res.poison as i16;
            }
        }
        let name = player(world, id)
            .map(|p| p.firstname.clone())
            .unwrap_or_default();
        if damage > 0 {
            // the monster's damage shields
            let (h, below) = monster_shields(world, id, monnum);
            if h {
                hurt = 1;
            }
            if below {
                done = true;
            }
            // the hit
            let CombatMsg { you, room, .. } =
                player_on_monster(&name, &verbs, &mon.name, res.outcome);
            say_prompt(world, id, you);
            room_say(world, key, [Some(id), None], Noise::Always, &room);
            // the kill
            killed = check_kill_monster(world, monnum, Some(id));
            if killed {
                done = true;
                super::kill_monster::distribute_experience(
                    world,
                    Some(id),
                    res.exp,
                    None,
                    Some(monnum),
                    key,
                );
                kill_autocombat_against_monster(world, monnum);
            } else if !mon.flags.contains(ActiveMonsterFlags::CHARMED) {
                if mon.montype != MonsterType::Angel {
                    maybe_follow(world, monnum, &name);
                }
            } else if eq_ignore_case(&mon.following, &name) {
                break_charm(world, monnum);
            }
            // the weapon's own powers
            let wpn = player(world, id).map(|p| p.weapon).unwrap_or_default();
            if wpn.0 != 0
                && !matches!(mv, Move::Punch | Move::Kick | Move::JumpKick)
                && !killed
                && let Some(itm) = world.items.get(&wpn).cloned()
            {
                if item_has(&itm, Ability::StealHp)
                    && let Some(p) = player_mut(world, id)
                {
                    p.hp = p.hp.saturating_add(damage as i16).min(p.maxhp);
                }
                if item_has(&itm, Ability::AutoUse) {
                    auto_use(world, id, wpn, Combatant::Monster(monnum));
                    if check_kill_monster(world, monnum, Some(id)) {
                        done = true;
                        killed = true;
                    }
                }
            }
            // did a shield kill the player?
            if hurt != 0 && check_kill_user(world, id).died() {
                hurt = 2;
                done = true;
            }
            // SMASH may knock the monster down
            if mv == Move::Smash
                && !killed
                && let Some(kmon) = world.monsters.get(&mon.knownnum)
            {
                let (str, level) = player(world, id).map_or((0, 0), |p| (p.str, p.level));
                if combat::smash_knockdown(str, level, mon.level, kmon.magicres) {
                    let rounds = world.config.smash_rounds as i16;
                    if let Some(m) = monster_mut(world, monnum) {
                        m.flags.insert(ActiveMonsterFlags::DOWN);
                        m.downrounds = rounds;
                    }
                    let msg = Msg::CombatSmashedDownMon {
                        mon: mon.name.clone(),
                    };
                    room_say(world, key, [Some(id), None], Noise::Always, &msg);
                    tell_user(world, id, msg);
                }
            }
        } else {
            // no damage: shown to the room only at the high level
            let shown = match res.outcome {
                Outcome::Hit { .. } => Outcome::Miss,
                o => o,
            };
            let CombatMsg { you, room, .. } = player_on_monster(&name, &verbs, &mon.name, shown);
            match shown {
                Outcome::Dodged => tell_user(world, id, you),
                _ => say_prompt(world, id, you),
            }
            room_say(world, key, [Some(id), None], Noise::High, &room);
        }
        // both sides again for the next blow
        if !done {
            pick_random_attack_move(&mut world.rng);
            let plr = player(world, id)?.clone();
            let mon = monster(world, monnum)?.clone();
            let (a, v) = player_side(world, id, &plr, mv, mtype, AlignGroup::Neutral)?;
            let (d, _) = monster_side(world, &mon, 0, mv, user_group(&plr))?;
            atk = a;
            def = d;
            verbs = v;
        }
    }
    // carry on in auto-combat unless someone died
    if !killed && hurt != 2 {
        let next = if mtype == Move::Backstab {
            Move::Attack
        } else {
            mv
        };
        engage_autocombat(world, id, AutoTarget::Monster(monnum), next);
    }
    last
}

/// A charmed monster hit by its master breaks free: every "Enslave Creature" spell on it
/// ends and slots of spells that no longer exist are cleared. The spells' ending is
/// T-050's.
fn break_charm(world: &mut World, monnum: ActiveMonsterId) {
    let enslaving: Vec<bool> = match monster(world, monnum) {
        Some(m) => m
            .spells
            .iter()
            .map(|c| {
                c.spell != SpellNo(0)
                    && world
                        .spells
                        .get(&c.spell)
                        .is_none_or(|s| s.abilities.iter().any(|(a, _)| *a == Ability::Enslave))
            })
            .collect(),
        None => return,
    };
    if let Some(m) = monster_mut(world, monnum) {
        m.friendly = 0;
        m.flags.remove(ActiveMonsterFlags::CHARMED);
        for (slot, clear) in m.spells.iter_mut().zip(enslaving) {
            if clear {
                *slot = silvermere_types::records::ActiveSpell {
                    spell: SpellNo(0),
                    value: 0,
                    rounds: 0,
                };
            }
        }
    }
}

/// One round of a monster's attacks on a player, at most [`MAX_BLOWS`].
pub fn attack_monster_user(
    world: &mut World,
    monnum: ActiveMonsterId,
    id: SessionId,
) -> Option<Outcome> {
    let mon = monster(world, monnum)?.clone();
    let plr = player(world, id)?.clone();
    // a full round of energy
    if (mon.energy as i32) < i32::from(mon.maxenergy) {
        return None;
    }
    let mkey = RoomKey {
        map: mon.map,
        room: mon.room,
    };
    if world
        .room(mkey)
        .is_some_and(|r| r.flags.contains(RoomFlags::PROTECTED))
    {
        return None;
    }
    let kmon = world.monsters.get(&mon.knownnum)?.clone();
    // "Do not Attack if Item Nmbr"
    if monster_has(world, &mon, Ability::NoAttackItem) {
        let noatk = super::monster_ability(world, &mon, Ability::NoAttackItem);
        // match_combat_target (T-034): the player's auto-combat target has this name
        let targeting = false;
        let p = player_mut(world, id)?;
        if targeting {
            if noatk != p.lastmon {
                p.lastmoncnt = 100;
                p.lastmon = noatk;
            } else {
                p.lastmoncnt = p.lastmoncnt.wrapping_sub(1);
            }
        }
        if noatk != p.lastmon {
            let carries = p.items.iter().chain(p.keys.iter()).any(|n| n.0 == noatk);
            if carries {
                kill_autocombat(world, id);
                kill_autocombat_against_monster(world, monnum);
                return None;
            }
        } else if p.lastmoncnt == 0 {
            p.lastmon = 0;
            p.lastmoncnt = 0;
        } else {
            p.lastmoncnt = p.lastmoncnt.wrapping_sub(1);
        }
    }
    // a player who is down: only some monsters go on hitting
    if plr.hp <= 0 {
        match kmon.alignment {
            Alignment::Evil
            | Alignment::ChaoticEvil
            | Alignment::LawfulGood
            | Alignment::NeutralEvil
            | Alignment::LawfulEvil => {
                if mon.montype == MonsterType::Guard && plr.evil < 80 {
                    return None;
                }
            }
            _ => return None,
        }
    }
    // a monster missing from its room's list is put back and announced
    let listed = world
        .room_state
        .get(&mkey)
        .is_some_and(|s| s.monsters.contains(&monnum));
    if !listed {
        room_say(
            world,
            mkey,
            [None, None],
            Noise::Always,
            &Msg::CombatAppears {
                mon: mon.name.clone(),
            },
        );
        if let Some(s) = world.room_state.get_mut(&mkey)
            && let Some(slot) = s.monsters.iter_mut().find(|n| n.0 == 0)
        {
            *slot = monnum;
        }
    }
    if mon.room != plr.room || mon.map != plr.map {
        return None;
    }
    // fleeing or confused monsters do not attack
    if monster_has(world, &mon, Ability::Flee) || check_monster_confusion(world, monnum) {
        return None;
    }
    // the fight is on
    if let Some(p) = player_mut(world, id) {
        p.status2.insert(PlayerStatus2::ATTACKED);
        p.status
            .remove(PlayerStatus::SNEAKING | PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
    crate::state::login::leave::stop_users_exit(world, id);
    if let Some(m) = monster_mut(world, monnum) {
        m.target = channel(id);
        m.inactive = 0;
    }
    let key = room_of(&plr);
    let penalty = world.config.smash_penalty as i32;
    let mut slot = 0usize;
    let mut stop = false;
    let mut fightover = false;
    let mut rounds = 0;
    let mut last = None;
    while !stop && rounds < MAX_BLOWS {
        rounds += 1;
        let wasup = player(world, id).is_some_and(|p| p.hp > 0);
        // the attack slot by the cumulative percentages
        let roll = world.rng.range(0, 100);
        if let Some(i) = kmon.attacks.iter().position(|a| i32::from(a.pct) > roll) {
            slot = i;
        }
        match kmon.attacks[slot].kind {
            MonsterAttackType::Spell => {
                let r = monster_cast(world, monnum, Some(slot), id, 0);
                if r != 0 {
                    if r == 2 {
                        fightover = true;
                        stop = true;
                    }
                    continue;
                }
                slot = 0;
                if kmon.attacks[0].kind != MonsterAttackType::Normal {
                    continue;
                }
            }
            MonsterAttackType::Rob => {
                if super::rob::monster_rob_user(world, monnum, slot, id) {
                    continue;
                }
                slot = 0;
                if kmon.attacks[0].kind != MonsterAttackType::Normal {
                    continue;
                }
            }
            MonsterAttackType::Normal => {}
            _ => continue,
        }
        // both sides
        let mon = monster(world, monnum)?.clone();
        if let Some(m) = monster_mut(world, monnum) {
            m.flags.insert(ActiveMonsterFlags::FOUGHT);
        }
        let plr = player(world, id)?.clone();
        let (mut atk, verbs) = monster_side(world, &mon, slot, Move::Attack, AlignGroup::Neutral)?;
        let (def, _) = player_side(
            world,
            id,
            &plr,
            Move::Attack,
            Move::Attack,
            monster_group(&kmon),
        )?;
        // a knocked-down monster
        if mon.flags.contains(ActiveMonsterFlags::DOWN) {
            let up = monster_mut(world, monnum).map(|m| {
                m.downrounds -= 1;
                if m.downrounds <= 0 {
                    m.flags.remove(ActiveMonsterFlags::DOWN);
                    true
                } else {
                    false
                }
            });
            if up == Some(true) {
                let msg = Msg::CombatMonRises {
                    mon: mon.name.clone(),
                };
                tell_user(world, id, msg.clone());
                room_say(world, key, [Some(id), None], Noise::Always, &msg);
            } else {
                down_penalty(&mut atk, penalty);
            }
        }
        let res = resolve(&mut atk, &def, Move::Attack, &mut world.rng);
        // not enough energy left: no blow, a miss
        if u32::from(res.energy) > u32::from(monster(world, monnum)?.energy) {
            last = Some(Outcome::Miss);
            break;
        }
        if let Some(m) = monster_mut(world, monnum) {
            m.energy -= res.energy;
            if res.energy > m.energy {
                stop = true;
            }
        }
        last = Some(res.outcome);
        let damage = match res.outcome {
            Outcome::Hit { damage, .. } => damage,
            _ => 0,
        };
        // the damage is not capped here
        if let Some(p) = player_mut(world, id) {
            p.hp = p.hp.wrapping_sub(damage as i16);
            if res.poison > i32::from(p.poison) {
                p.poison = res.poison as i16;
            }
        }
        let texts = SlotTexts {
            hit: super::message(world, kmon.attacks[slot].hitmsg),
            def: super::message(world, kmon.attacks[slot].defmsg),
            miss: super::message(world, kmon.attacks[slot].missmsg),
        };
        let vplr = player(world, id).cloned();
        let msgs = monster_on_player(
            &mon.name,
            &verbs,
            &texts,
            &plr.firstname,
            possessive_pronoun(vplr.as_ref()),
            subject_pronoun(vplr.as_ref()),
            res.outcome,
        );
        if damage > 0 {
            player_shields(world, id, monnum, &kmon);
            say_prompt(world, id, msgs.you);
            room_say(world, key, [Some(id), None], Noise::Always, &msgs.room);
            // the slot's hit spell
            let hitspell = kmon.attacks[slot].hitspell;
            if hitspell != SpellNo(0)
                && monster_cast(world, monnum, None, id, i32::from(hitspell.0)) == 2
            {
                stop = true;
                fightover = true;
                if let Some(m) = monster_mut(world, monnum) {
                    m.following.clear();
                }
            }
            if fightover {
                continue;
            }
            // knocked out by this blow
            if wasup && player(world, id).is_some_and(|p| p.hp <= 0) {
                knock_out_user(world, id);
                if let Some(m) = monster_mut(world, monnum) {
                    m.following.clear();
                }
            }
            // killed
            if check_kill_user(world, id).died() {
                stop = true;
                fightover = true;
                if let Some(m) = monster_mut(world, monnum) {
                    m.following.clear();
                }
                if res.exp > 0 {
                    super::kill_monster::distribute_experience(
                        world,
                        None,
                        res.exp,
                        Some(id),
                        None,
                        mkey,
                    );
                }
            }
        } else {
            let shown = match res.outcome {
                Outcome::Hit { .. } => Outcome::Miss,
                o => o,
            };
            let msgs = if shown == res.outcome {
                msgs
            } else {
                monster_on_player(
                    &mon.name,
                    &verbs,
                    &texts,
                    &plr.firstname,
                    possessive_pronoun(vplr.as_ref()),
                    subject_pronoun(vplr.as_ref()),
                    shown,
                )
            };
            say_prompt(world, id, msgs.you);
            room_say(world, key, [Some(id), None], Noise::High, &msgs.room);
        }
    }
    // an after-combat spell
    if !fightover {
        monster_cast_random_spell(world, monnum, id);
    }
    // following
    if !fightover && kmon.montype != MonsterType::Angel {
        let guard_busy = kmon.montype == MonsterType::Guard
            && monster(world, monnum).is_some_and(|m| !m.following.is_empty());
        if !guard_busy {
            let roll = world.rng.range(1, 100);
            let name = plr.firstname.clone();
            if let Some(m) = monster_mut(world, monnum) {
                if roll < i32::from(kmon.followpct) {
                    m.following = name;
                    m.friendly = 0;
                } else if !matches!(
                    m.alignment,
                    Alignment::LawfulGood | Alignment::Good | Alignment::Neutral
                ) {
                    m.following.clear();
                }
            }
        }
    }
    last
}

/// The player's damage shields stinging the monster that hit him: worn items, then spells;
/// (sic) a spell's message is looked up on the worn item in the same slot number. Text 1 to
/// the player, text 3 to the room.
fn player_shields(
    world: &mut World,
    id: SessionId,
    monnum: ActiveMonsterId,
    kmon: &silvermere_types::records::KnownMonster,
) {
    let Some(plr) = player(world, id).cloned() else {
        return;
    };
    let key = room_of(&plr);
    let mon_name = monster(world, monnum)
        .map(|m| m.name.clone())
        .unwrap_or_default();
    let worn_msg = |world: &World, i: usize| -> i32 {
        plr.worn
            .get(i)
            .filter(|n| n.0 != 0)
            .and_then(|n| world.items.get(n))
            .map_or(0, |itm| item_ability(itm, Ability::ShockMsg))
    };
    let mut stings: Vec<(i32, i32)> = Vec::new();
    for (i, n) in plr.worn.iter().enumerate() {
        if n.0 == 0 {
            continue;
        }
        let dmg = world
            .items
            .get(n)
            .map_or(0, |itm| item_ability(itm, Ability::DamageShield));
        if dmg != 0 {
            stings.push((dmg, worn_msg(world, i)));
        }
    }
    for (i, (&spell, &val)) in plr.spells.iter().zip(&plr.spellvals).enumerate() {
        if spell == SpellNo(0) {
            continue;
        }
        let Some(spl) = world.spells.get(&spell) else {
            continue;
        };
        let mut dmg = silvermere_rules::abilities::spell_ability_value(spl, Ability::DamageShield);
        if dmg == 0 && silvermere_rules::abilities::spell_has_ability(spl, Ability::DamageShield) {
            dmg = i32::from(val);
        }
        if dmg != 0 {
            stings.push((dmg, worn_msg(world, i)));
        }
    }
    for (dmg, msgnum) in stings {
        let dmg = shield_roll(world, dmg);
        if let Some(m) = monster_mut(world, monnum) {
            m.hp = m.hp.wrapping_sub(dmg as i16);
            if m.hp > kmon.maxhp {
                m.hp = kmon.maxhp;
            }
        }
        let msg = if msgnum != 0 {
            world
                .messages
                .get(&silvermere_types::ids::MessageNo(msgnum))
                .map(|m| m.msgs.clone())
        } else {
            None
        };
        if let Some(m) = msg {
            use crate::movement::exits::{Arg, fmt_msg};
            let a = [Arg::S(&mon_name), Arg::D(i64::from(dmg))];
            tell_user(
                world,
                id,
                Msg::CombatShockText {
                    text: fmt_msg(&m[0], &a),
                },
            );
            room_say(
                world,
                key,
                [None, Some(id)],
                Noise::Always,
                &Msg::CombatShockText {
                    text: fmt_msg(&m[2], &a),
                },
            );
        }
    }
}

/// Whether `attacker` may fight `victim`: PvP switched off (`lvldiff` -1) or a newcomer
/// below [`MIN_PVP_LEVEL`] on either side refuses; a victim who attacked first, or a FIEND,
/// may be attacked; otherwise the levels may differ by at most `lvldiff`.
pub fn pvp_level_range_ok(world: &World, attacker: SessionId, victim: SessionId) -> bool {
    let (Some(a), Some(v)) = (player(world, attacker), player(world, victim)) else {
        return false;
    };
    let lvldiff = world.config.lvl_diff;
    if lvldiff == -1 || a.level < MIN_PVP_LEVEL || v.level < MIN_PVP_LEVEL {
        return false;
    }
    if already_evil(world, victim, attacker) {
        return true;
    }
    if v.evil >= FIEND_EVIL {
        return true;
    }
    let d1 = i32::from(a.level.wrapping_sub(v.level) as i16);
    let d2 = i32::from(v.level.wrapping_sub(a.level) as i16);
    !(d1 > lvldiff || d2 > lvldiff)
}

/// A player attacking a player, with the blow's move `mv` and type `mtype`.
pub fn attack_user_user(
    world: &mut World,
    id: SessionId,
    target: SessionId,
    mv: Move,
    mtype: Move,
    announce: bool,
    mode: Mode,
) -> Option<Outcome> {
    let mut mv = mv;
    let plr = player(world, id)?.clone();
    let tplr = player(world, target)?.clone();
    let alive = tplr.hp > 0;
    // a hidden victim
    if mode == Mode::Command && tplr.hidden != 0 && !plr.status.contains(PlayerStatus::SEEHIDDEN) {
        tell_user(
            world,
            id,
            Msg::CombatNotHere {
                who: tplr.firstname.clone(),
            },
        );
        return None;
    }
    if let Some(p) = player_mut(world, id) {
        p.status2.insert(PlayerStatus2::ATTACKED);
        p.status.remove(PlayerStatus::RESTING);
        p.status2.remove(PlayerStatus2::MEDITATING);
    }
    // the victim must be in the same room
    if plr.room != tplr.room {
        if mode == Mode::Command {
            return None;
        }
        if plr.status.contains(PlayerStatus::TARGETGONE) {
            let cnt = player_mut(world, id).map_or(0, |p| {
                p.pvpcnt = p.pvpcnt.wrapping_add(1);
                p.pvpcnt
            });
            if cnt > 1 {
                kill_autocombat(world, id);
                display_autocombat_broken(world, id);
                return None;
            }
        }
        engage_autocombat(world, id, AutoTarget::Player(target), mv);
        return None;
    }
    if mode == Mode::Auto && plr.energy < plr.maxenergy {
        engage_autocombat(world, id, AutoTarget::Player(target), mv);
        return None;
    }
    if let Some(p) = player_mut(world, id) {
        p.status.remove(PlayerStatus::TARGETGONE);
    }
    if plr.hp <= 0 || plr.status2.contains(PlayerStatus2::AFRAID) {
        if mode == Mode::Auto {
            display_autocombat_broken(world, id);
        }
        return None;
    }
    let key = room_of(&plr);
    let rm = world.room(key)?.clone();
    if !world.config.tournament_play
        && rm.flags.contains(RoomFlags::PROTECTED)
        && !debug_on(world, id)
    {
        kill_autocombat(world, id);
        display_autocombat_broken(world, id);
        return None;
    }
    let arena = world.display.arena_safe && rm.room_type == RoomType::Colliseum;
    if mode == Mode::Command {
        // levels and evil points, except in a safe arena
        if !arena {
            if !pvp_level_range_ok(world, id, target) && !debug_on(world, id) {
                tell_user(world, id, Msg::CombatUnbalanced);
                return None;
            }
            if add_evil_points(world, id, Some(target), 10, 11, EvilKind::Attack) {
                return None;
            }
        }
        if let Some(t) = player_mut(world, target) {
            t.status2.insert(PlayerStatus2::ATTACKED);
            t.status
                .remove(PlayerStatus::SNEAKING | PlayerStatus::RESTING);
            t.status2.remove(PlayerStatus2::MEDITATING);
        }
        if mtype == Move::Backstab {
            // silent; a victim already fighting the attacker is hit normally
            tell_user(world, id, Msg::CombatEngagedSplit);
            if is_combatting_user(world, target, id) {
                mv = Move::Attack;
            }
        } else {
            tell_user(world, id, Msg::CombatEngaged);
            if announce {
                let warn = Msg::CombatMovesToAttackYou {
                    who: plr.firstname.clone(),
                };
                if tplr.settings.contains(PlayerSettings::ATKINTERRUPT) {
                    // the waiting output is thrown away first
                    interrupt_output(world, target);
                }
                say_prompt(world, target, warn);
                room_say(
                    world,
                    key,
                    [Some(id), Some(target)],
                    Noise::Always,
                    &Msg::CombatMovesToAttackOther {
                        who: plr.firstname.clone(),
                        target: tplr.firstname.clone(),
                    },
                );
            }
        }
        if let Some(p) = player_mut(world, id) {
            p.energy = 0;
        }
        engage_autocombat(world, id, AutoTarget::Player(target), mv);
        crate::state::login::leave::stop_users_exit(world, target);
        return None;
    }

    if let Some(t) = player_mut(world, target) {
        t.status2.insert(PlayerStatus2::ATTACKED);
        t.status
            .remove(PlayerStatus::SNEAKING | PlayerStatus::RESTING);
        t.status2.remove(PlayerStatus2::MEDITATING);
    }
    crate::state::login::leave::stop_users_exit(world, target);
    if check_confusion(world, id) {
        if let Some(p) = player_mut(world, id) {
            p.energy = 0;
        }
        engage_autocombat(world, id, AutoTarget::Player(target), mv);
        return None;
    }
    pick_random_attack_move(&mut world.rng);
    let tplr = player(world, target)?.clone();
    let (mut atk, mut verbs) = player_side(world, id, &plr, mv, mtype, AlignGroup::Neutral)?;
    let (mut def, _) = player_side(world, target, &tplr, mv, mtype, user_group(&plr))?;
    let smash_rounds = world.config.smash_rounds as u8;
    let mut stop = false;
    let mut killed = false;
    let mut hurt = 0u8;
    let mut rounds = 0;
    let mut last = None;
    while !stop && !killed && rounds < MAX_BLOWS {
        hurt = 0;
        rounds += 1;
        let res = resolve(&mut atk, &def, mv, &mut world.rng);
        last = Some(res.outcome);
        // knocked-down fighters count down
        for who in [target, id] {
            let up = player_mut(world, who).and_then(|p| {
                if !p.status2.contains(PlayerStatus2::KNOCKEDDOWN) {
                    return None;
                }
                p.pvptimer = p.pvptimer.wrapping_sub(1);
                if (p.pvptimer as i8) <= 0 {
                    p.status2.remove(PlayerStatus2::KNOCKEDDOWN);
                    Some((p.firstname.clone(), room_of(p)))
                } else {
                    None
                }
            });
            if let Some((name, room)) = up {
                tell_user(world, who, Msg::StunRise);
                room_say(
                    world,
                    room,
                    [Some(who), None],
                    Noise::Always,
                    &Msg::StunRiseOther { who: name },
                );
            }
        }
        // the blow's energy
        let energy = player(world, id).map_or(0, |p| p.energy);
        if res.energy > energy {
            if mode == Mode::Command {
                tell_user(world, id, Msg::CombatMustRestPvp);
            } else {
                engage_autocombat(world, id, AutoTarget::Player(target), mv);
            }
            return None;
        }
        if let Some(p) = player_mut(world, id) {
            p.hidden = 0;
            p.status.remove(PlayerStatus::SNEAKING);
        }
        if mv == Move::Backstab {
            mv = Move::Attack;
        }
        if let Some(p) = player_mut(world, id) {
            p.energy -= res.energy;
            if res.energy > p.energy {
                stop = true;
            }
        }
        let damage = match res.outcome {
            Outcome::Hit { damage, .. } => damage,
            _ => 0,
        };
        if let Some(t) = player_mut(world, target) {
            t.hp = t.hp.wrapping_sub(damage as i16);
            if res.poison > i32::from(t.poison) {
                t.poison = res.poison as i16;
            }
        }
        let tname = tplr.firstname.clone();
        let his_atk = possessive_pronoun(player(world, id)).to_owned();
        let his_def = possessive_pronoun(player(world, target)).to_owned();
        let msgs = player_on_player(
            &plr.firstname,
            &verbs,
            &tname,
            &his_atk,
            &his_def,
            res.outcome,
        );
        if damage > 0 {
            // the victim's shields sting the attacker
            if pvp_shields(world, id, target) {
                hurt = 1;
            }
            say_prompt(world, id, msgs.you);
            if let Some(v) = msgs.victim {
                say_prompt(world, target, v);
            }
            // the victim's name is kept, the death handled, then the room told
            killed = check_kill_user(world, target).died();
            room_say(
                world,
                key,
                [Some(id), Some(target)],
                Noise::Medium,
                &msgs.room,
            );
            if killed {
                stop = true;
                if res.exp > 0 {
                    super::kill_monster::distribute_experience(
                        world,
                        Some(id),
                        res.exp,
                        Some(target),
                        None,
                        key,
                    );
                }
            } else if alive && player(world, target).is_some_and(|t| t.hp <= 0) {
                knock_out_user(world, target);
            }
            // the attacker's weapon
            let wpn = player(world, id).map(|p| p.weapon).unwrap_or_default();
            if wpn.0 != 0
                && !matches!(mv, Move::Punch | Move::Kick | Move::JumpKick)
                && let Some(itm) = world.items.get(&wpn).cloned()
            {
                if item_has(&itm, Ability::StealHp)
                    && let Some(p) = player_mut(world, id)
                {
                    p.hp = p.hp.saturating_add(damage as i16).min(p.maxhp);
                }
                if !killed
                    && item_has(&itm, Ability::AutoUse)
                    && auto_use(world, id, wpn, Combatant::Player(target)) == 2
                {
                    stop = true;
                    killed = true;
                }
            }
            // a shield may have killed the attacker
            if hurt != 0 {
                if player(world, id).is_some_and(|p| p.hp < 0) {
                    stop = true;
                }
                if check_kill_user(world, id).died() {
                    hurt = 2;
                    stop = true;
                }
            }
            // SMASH
            if mv == Move::Smash {
                let weight = |world: &World, who: SessionId| -> i32 {
                    player(world, who).map_or(0, |p| {
                        i32::from(p.encumpct) + i32::from(p.level) + i32::from(p.str)
                    })
                };
                if weight(world, id) > weight(world, target) {
                    if let Some(t) = player_mut(world, target) {
                        t.status2.insert(PlayerStatus2::KNOCKEDDOWN);
                        t.pvptimer = smash_rounds;
                    }
                    tell_user(world, id, Msg::CombatSmashedIt { who: tname.clone() });
                    tell_user(world, target, Msg::CombatSmashedYou);
                    room_say(
                        world,
                        key,
                        [Some(id), Some(target)],
                        Noise::Always,
                        &Msg::CombatSmashedDownPvp { who: tname.clone() },
                    );
                }
            }
        } else {
            let shown = match res.outcome {
                Outcome::Hit { .. } => Outcome::Miss,
                o => o,
            };
            let msgs = player_on_player(&plr.firstname, &verbs, &tname, &his_atk, &his_def, shown);
            say_prompt(world, id, msgs.you);
            if let Some(v) = msgs.victim {
                say_prompt(world, target, v);
            }
            room_say(
                world,
                key,
                [Some(id), Some(target)],
                Noise::High,
                &msgs.room,
            );
        }
        // both sides again
        if !stop {
            pick_random_attack_move(&mut world.rng);
            let p = player(world, id)?.clone();
            let t = player(world, target)?.clone();
            let (a, v) = player_side(world, id, &p, mv, mtype, AlignGroup::Neutral)?;
            let (d, _) = player_side(world, target, &t, mv, mtype, user_group(&p))?;
            atk = a;
            def = d;
            verbs = v;
        }
    }
    // with auto-engage
    let autoengage = player(world, id).is_some_and(|p| p.autoengage != 0);
    if !killed && hurt != 2 && autoengage {
        engage_autocombat(world, id, AutoTarget::Player(target), mv);
    } else if hurt != 2 && autoengage {
        kill_autocombat(world, id);
        display_autocombat_broken(world, id);
    }
    last
}

/// The victim's damage shields on the attacker: text 1 to the wearer, text 2 to the
/// attacker, text 3 to the room; (sic) a spell's message comes from the worn item in the
/// same slot.
fn pvp_shields(world: &mut World, id: SessionId, target: SessionId) -> bool {
    let Some(t) = player(world, target).cloned() else {
        return false;
    };
    let name = player(world, id)
        .map(|p| p.firstname.clone())
        .unwrap_or_default();
    let key = room_of(&t);
    let worn_msg = |world: &World, i: usize| -> i32 {
        t.worn
            .get(i)
            .filter(|n| n.0 != 0)
            .and_then(|n| world.items.get(n))
            .map_or(0, |itm| item_ability(itm, Ability::ShockMsg))
    };
    let mut stings = Vec::new();
    for (i, n) in t.worn.iter().enumerate() {
        if n.0 == 0 {
            continue;
        }
        let dmg = world
            .items
            .get(n)
            .map_or(0, |itm| item_ability(itm, Ability::DamageShield));
        if dmg != 0 {
            stings.push((dmg, worn_msg(world, i)));
        }
    }
    for (i, (&spell, &val)) in t.spells.iter().zip(&t.spellvals).enumerate() {
        if spell == SpellNo(0) {
            continue;
        }
        let Some(spl) = world.spells.get(&spell) else {
            continue;
        };
        let mut dmg = silvermere_rules::abilities::spell_ability_value(spl, Ability::DamageShield);
        if dmg == 0 && silvermere_rules::abilities::spell_has_ability(spl, Ability::DamageShield) {
            dmg = i32::from(val);
        }
        if dmg != 0 {
            stings.push((dmg, worn_msg(world, i)));
        }
    }
    let hurt = !stings.is_empty();
    for (dmg, msgnum) in stings {
        let dmg = shield_roll(world, dmg);
        if let Some(p) = player_mut(world, id) {
            p.hp = p.hp.wrapping_sub(dmg as i16);
        }
        let msg = if msgnum != 0 {
            world
                .messages
                .get(&silvermere_types::ids::MessageNo(msgnum))
                .map(|m| m.msgs.clone())
        } else {
            None
        };
        match msg {
            Some(m) => {
                use crate::movement::exits::{Arg, fmt_msg};
                let named = [Arg::S(&name), Arg::D(i64::from(dmg))];
                tell_user(
                    world,
                    target,
                    Msg::CombatShockText {
                        text: fmt_msg(&m[0], &named),
                    },
                );
                tell_user(
                    world,
                    id,
                    Msg::CombatShockText {
                        text: fmt_msg(&m[1], &[Arg::D(i64::from(dmg))]),
                    },
                );
                room_say(
                    world,
                    key,
                    [Some(id), Some(target)],
                    Noise::Always,
                    &Msg::CombatShockText {
                        text: fmt_msg(&m[2], &named),
                    },
                );
            }
            None => tell_user(world, id, Msg::CombatTookDamagePvp { dmg }),
        }
    }
    hurt
}

/// One blow of a hunting or charmed monster at another, told to the room in one line. T-036
/// calls it.
pub fn attack_monster_monster(
    world: &mut World,
    attacker: ActiveMonsterId,
    target: ActiveMonsterId,
) -> Option<Outcome> {
    let atkm = monster(world, attacker)?.clone();
    let tgt = monster(world, target)?.clone();
    if (atkm.energy as i32) < i32::from(atkm.maxenergy) {
        return None;
    }
    let kmon = world.monsters.get(&atkm.knownnum)?.clone();
    if monster_has(world, &atkm, Ability::Flee) {
        return None;
    }
    let (mut a, _) = monster_side(world, &atkm, 0, Move::Attack, monster_group(&kmon))?;
    let (d, _) = monster_side(world, &tgt, 0, Move::Attack, AlignGroup::Neutral)?;
    let res = resolve(&mut a, &d, Move::Attack, &mut world.rng);
    if u32::from(res.energy) > u32::from(atkm.energy) {
        return None;
    }
    let mut damage = match res.outcome {
        Outcome::Hit { damage, .. } => damage,
        _ => 0,
    };
    if let Some(m) = monster_mut(world, attacker) {
        m.energy -= res.energy;
    }
    if let Some(t) = monster_mut(world, target) {
        if damage > i32::from(t.hp) {
            damage = i32::from(t.hp);
        }
        t.hp -= damage as i16;
        if res.poison > i32::from(t.poison) {
            t.poison = res.poison as i16;
        }
    }
    let tgt = monster(world, target)?.clone();
    let shield = super::monster_ability(world, &tgt, Ability::DamageShield);
    let sting = |world: &mut World| {
        if shield != 0 {
            let dmg = shield_roll(world, shield);
            if let Some(m) = monster_mut(world, attacker) {
                m.hp = m.hp.wrapping_sub(dmg as i16);
                if m.hp > m.maxhp {
                    m.hp = m.maxhp;
                }
            }
        }
    };
    let akey = RoomKey {
        map: atkm.map,
        room: atkm.room,
    };
    let tkey = RoomKey {
        map: tgt.map,
        room: tgt.room,
    };
    if check_kill_monster(world, target, None) {
        sting(world);
        let text =
            super::messages::upper_first(&format!("{} just killed {}.", atkm.name, tgt.name));
        let colour = ansi(&[ansi::ANSI_BOLD, ansi::ANSI_FGGREY]);
        room_say(
            world,
            akey,
            [None, None],
            Noise::Always,
            &Msg::CombatMonVsMon { colour, text },
        );
        super::kill_monster::distribute_experience(world, None, res.exp, None, Some(target), akey);
        kill_autocombat_against_monster(world, target);
    } else {
        let (colour, text) = if damage > 0 {
            sting(world);
            (
                ansi::ANSI_LTRED,
                format!("{} just attacked {}!", atkm.name, tgt.name),
            )
        } else {
            match res.outcome {
                Outcome::NoDamage => (
                    ansi::ANSI_RED,
                    format!("{}'s just glanced off of {}'s armour.", atkm.name, tgt.name),
                ),
                Outcome::Dodged => (
                    ansi::ANSI_CYAN,
                    format!("{} just dodged an attack from {}.", tgt.name, atkm.name),
                ),
                _ => (
                    ansi::ANSI_CYAN,
                    format!("{} just missed an attack against {}.", atkm.name, tgt.name),
                ),
            }
        };
        let colour = ansi(&[colour]);
        room_say(
            world,
            tkey,
            [None, None],
            Noise::Always,
            &Msg::CombatMonVsMon {
                colour,
                text: super::messages::upper_first(&text),
            },
        );
    }
    Some(res.outcome)
}
