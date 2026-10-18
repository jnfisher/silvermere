//! The slow cycle, every `secslow` (30) seconds, and [`slow_update_character`].
//!
//! The scheduler runs [`slow_cycle`], then its own restock count,
//! then [`slow_cycle_end`], in that order.

use silvermere_rules::Rng;
use silvermere_rules::regen;
use silvermere_text::Msg;
use silvermere_types::ids::{ActiveMonsterId, SessionId, TextBlockNo};
use silvermere_types::sizes::{NDIRS, NROOMMONS};

use super::{monsters, prompt, room_of};
use crate::tell::{Noise, TellBlind, tell_room};
use crate::world::World;

/// A random-event roll of 95 or more shows nothing.
pub const NOEVENT: i32 = 95;

/// Most text lines read from a random-event block.
const MAX_EVENT_LINES: usize = 30;

/// The first half of the slow cycle: a new session order, the
/// slow update of every player in the realm, then of every monster.
pub fn slow_cycle(world: &mut World) {
    world.cycles.slow_runs += 1;
    shuffle_mappings(world);
    slow_update_characters(world);
    monsters::slow_update_monsters(world);
}

/// The second half of the slow cycle, after the restock count: a fresh anti-script letter
/// in each of the three direction name tables, then maybe a random monster text. The
/// `pollrate` adjustment that followed is not carried.
pub fn slow_cycle_end(world: &mut World) {
    // (sic) the letter is never 'Z': 'A'..'Y'.
    for dir in 0..NDIRS {
        for table in 0..3 {
            world.cycles.dir_letters[dir][table] =
                world.rng.range(i32::from(b'A'), i32::from(b'Z')) as u8;
        }
        // The room display's live `dirnames[]` (T-025) carries the first table's letter.
        let letter = char::from(world.cycles.dir_letters[dir][0]);
        world.display.dirnames.set_letter(dir, letter);
    }
    let roll = world.rng.range(0, 100);
    perform_random_event(world, roll);
}

/// A new session order: the identity order of the sessions,
/// then `nterms` swaps of two random slots.
pub fn shuffle_mappings(world: &mut World) {
    let mut order = world.session_ids();
    let n = order.len() as i32;
    for _ in 0..n {
        let a = world.rng.range(0, n) as usize;
        let b = world.rng.range(0, n) as usize;
        order.swap(a, b);
    }
    world.cycles.order = order;
}

/// Every session in the realm, in id order. Charging BBS credits per channel is not
/// carried.
pub fn slow_update_characters(world: &mut World) {
    for id in world.session_ids() {
        if world.is_in_game(id) {
            slow_update_character(world, id);
        }
    }
}

/// One player's slow update.
pub fn slow_update_character(world: &mut World, id: SessionId) {
    let hooks = world.cycles.hooks;
    // The player's evil timers count down (T-038).
    crate::evil::decrement_evil_timers(world, id);
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    let mut changed = false;
    // The per-tick move count starts again
    plr.moves = 0;
    // Hungrier and thirstier; nothing reads them at 0 in this routine.
    plr.hunger = plr.hunger.wrapping_sub(1);
    plr.thirst = plr.thirst.wrapping_sub(1);
    // Poison does its damage each tick
    if plr.poison > 0 {
        let wasup = plr.hp > 0;
        s.output.push(Msg::PoisonTick);
        changed = true;
        plr.hp = plr.hp.wrapping_sub(plr.poison);
        let knocked_out = wasup && plr.hp < 0;
        world.touch(id);
        if knocked_out {
            (hooks.knock_out_user)(world, id);
        }
        // Died of poison: nothing more for this session (the experience share
        // after it is dead code, sic: `exp` is never set).
        if (hooks.check_kill_user)(world, id) {
            return;
        }
    }
    let Some(plr) = world
        .sessions
        .get_mut(&id)
        .and_then(|s| s.character.as_mut())
    else {
        return;
    };
    if plr.hp <= 0 && plr.aided == 0 {
        // Down and nobody has helped: bleed one hit point
        plr.hp -= 1;
        world.touch(id);
        if (hooks.check_kill_user)(world, id) {
            return;
        }
        changed = true;
    } else if plr.hp <= 0 {
        // Down but aided: recover one hit point
        plr.hp += 1;
        changed = true;
    } else if plr.hp != plr.maxhp {
        // Regeneration, never above the maximum
        changed = true;
        let regen = regen::hp_regen(plr.level, plr.hea, plr.healrate);
        if plr.hp < plr.maxhp {
            plr.hp = clamp_i16(i32::from(plr.hp) + regen);
        }
        if plr.hp > plr.maxhp {
            plr.hp = plr.maxhp;
        }
        if plr.hp > 0 {
            plr.aided = 0;
        }
    }
    // Mana (or kai), while below the maximum
    let Some(s) = world.sessions.get_mut(&id) else {
        return;
    };
    let Some(plr) = s.character.as_mut() else {
        return;
    };
    if i32::from(plr.mana) < i32::from(plr.maxmana) {
        if let Some(cls) = world.classes.get(&plr.class) {
            changed = true;
            let stat = regen::class_stat(cls.magery, plr.intel, plr.wil, plr.cha);
            let r = regen::mana_regen(cls.magery, stat, plr.level, cls.magelvl, plr.manarate);
            plr.mana = clamp_mana(i32::from(plr.mana) + r, plr.maxmana);
        } else {
            // Class record missing: one point a tick
            plr.mana = plr.mana.wrapping_add(1);
        }
    }
    // Session time
    if let Some(plr) = s.character.as_mut() {
        plr.sesstime = plr.sesstime.wrapping_add(1);
    }
    // The figures that hang on the abilities, before the prompt
    if let Some(mut c) = world.session_mut(id).and_then(|s| s.character.take()) {
        crate::player::secondary::apply_dynamic(world, &mut c);
        if let Some(s) = world.session_mut(id) {
            s.character = Some(c);
        }
    }
    // The prompt again, with the new figures
    if changed {
        prompt(world, id);
    }
}

/// An `int` result stored back into a 16-bit field.
pub(crate) fn clamp_i16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// `mana += regen`, then kept between 0 and `maxmana`.
pub(crate) fn clamp_mana(v: i32, maxmana: u16) -> i16 {
    clamp_i16(v.max(0).min(i32::from(maxmana)))
}

/// The random monster text: on a roll under [`NOEVENT`], the players are visited in the
/// shuffled order; for each monster in a player's room whose kind has a random-event text
/// block, a fresh roll picks the first line (of at most 30) whose leading chance (its low
/// 16 bits) beats it, and the text after the first space
/// goes to the whole room. One event per cycle; at most 22 monster slots are looked at.
/// A room entry naming a monster that no longer exists is removed.
pub fn perform_random_event(world: &mut World, roll: i32) {
    if roll >= NOEVENT {
        return;
    }
    let mut cnt = 0;
    for id in world.shuffled_sessions() {
        if !world.is_in_game(id) {
            continue;
        }
        let Some(key) = room_of(world, id) else {
            continue;
        };
        for i in 0..NROOMMONS {
            let Some(rs) = world.room_state.get(&key) else {
                break;
            };
            let mid = rs.monsters[i];
            if mid == ActiveMonsterId(0) {
                continue;
            }
            match world.active_monsters.by_number(mid) {
                Some(mon) => {
                    let block = world
                        .monsters
                        .get(&mon.knownnum)
                        .filter(|k| k.randtext != TextBlockNo(0))
                        .and_then(|k| world.text_blocks.get(&(0, k.randtext)));
                    if let Some(block) = block {
                        let rnd = world.rng.range(0, 100);
                        if let Some(text) = event_line(&block.text, rnd) {
                            let msg = Msg::RandomEvent { text };
                            tell_room(
                                world,
                                key,
                                [None, None],
                                Noise::Always,
                                TellBlind::All,
                                true,
                                None,
                                &msg,
                            );
                            return;
                        }
                    }
                }
                None => {
                    // Stale room entry: reported and removed
                    tracing::warn!(?key, monster = mid.0, "Removing monster?");
                    if let Some(rs) = world.room_state.get_mut(&key) {
                        rs.monsters[i] = ActiveMonsterId(0);
                    }
                }
            }
            // The count is compared before it goes up.
            let over = cnt > 20;
            cnt += 1;
            if over {
                return;
            }
        }
    }
}

/// The line a random-event roll picks: lines are the non-empty `\r`/`\n` separated
/// tokens; the first of at most 30 whose leading number, cut to 16 bits, is greater than
/// `rnd` gives the text after its first space. A winning line without a space ends the
/// search.
pub fn event_line(text: &str, rnd: i32) -> Option<String> {
    let lines = text.split(['\r', '\n']).filter(|l| !l.is_empty());
    for line in lines.take(MAX_EVENT_LINES) {
        if i32::from(leading_int32(line) as i16) > rnd {
            return line.find(' ').map(|sp| line[sp + 1..].to_string());
        }
    }
    None
}

/// The leading number of a line: leading blanks, an optional sign, then digits.
fn leading_int32(s: &str) -> i64 {
    let t = s.trim_start_matches([' ', '\t']);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        v = v.wrapping_mul(10).wrapping_add(i64::from(b - b'0'));
    }
    if neg { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_line_takes_the_first_chance_that_beats_the_roll() {
        let block = "10 The orc grunts.\r\n50 The orc scratches itself.\r\n100 The orc yawns.";
        assert_eq!(event_line(block, 5).as_deref(), Some("The orc grunts."));
        assert_eq!(
            event_line(block, 10).as_deref(),
            Some("The orc scratches itself.")
        );
        assert_eq!(event_line(block, 99).as_deref(), Some("The orc yawns."));
        assert_eq!(event_line("50", 10), None);
        assert_eq!(event_line("5 too low", 10), None);
        // Only the low 16 bits count: 65546 is 10.
        assert_eq!(event_line("65546 wraps", 20), None);
    }
}
