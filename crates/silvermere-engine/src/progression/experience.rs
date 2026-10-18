//! Experience: [`add_experience`].
//!
//! The kill path (T-033), quests and the sysop commands call [`add_experience`]; the
//! experience a level needs is `silvermere_rules::progression::new_calc_exp_needed`.

use silvermere_rules::progression::{BILLION, can_gain_exp};
use silvermere_text::Msg;
use silvermere_types::ids::SessionId;

use crate::world::World;

/// What [`add_experience`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpOutcome {
    /// The amount was added.
    pub added: bool,
    /// Refused: the character is past the experience for `level + XTRALVL` and must
    /// train first.
    pub capped: bool,
}

/// Adds `exp` experience to the session's character. `exp` is a 32-bit signed amount.
///
/// A positive amount is refused, with `You have progressed too far without training to
/// acquire anymore experience!` and the prompt, once the character holds more than
/// `new_calc_exp_needed(level + xtralvl, race.exp + class.exp)` (the (sic) of a silent call
/// past the cap on billions alone is kept; see
/// `silvermere_rules::progression::can_gain_exp`). Otherwise the gang's total grows too
/// (amounts above 1), the 32-bit `exp` and the 1.11 pair `expbil`/`exprem` take the amount,
/// `verbose` prints `You gain N experience.` and the prompt, and the top list is brought up
/// to date.
///
/// (sic) The amount is compared with a billion unsigned, so a negative amount counts as
/// over four billion: -5 adds 4 to `expbil` and 294,967,291 to `exprem`. The gain line
/// prints what is left of the amount after whole billions were taken off.
///
/// Not carried: the demo version's level-4 cap and the pre-1.11 refusal (every character
/// here is on the 1.11 system). The original updated the top list only when combat stopped;
/// it is updated on every gain here, so TOPTEN is current between fights.
pub fn add_experience(world: &mut World, id: SessionId, exp: i32, verbose: bool) -> ExpOutcome {
    let xtralvl = i16::try_from(world.config.xtra_lvl).unwrap_or(i16::MAX);
    let Some(plr) = world.sessions.get(&id).and_then(|s| s.character.as_ref()) else {
        return ExpOutcome {
            added: false,
            capped: false,
        };
    };
    let modifier = match (world.classes.get(&plr.class), world.races.get(&plr.race)) {
        (Some(cls), Some(rac)) => Some(cls.exp.wrapping_add(rac.exp)),
        _ => None,
    };
    // The training cap
    if let Some(modifier) = modifier
        && exp > 0
    {
        let have = silvermere_types::records::BigExp {
            expbil: plr.expbil,
            exprem: plr.exprem,
        };
        let level = plr.level as i16;
        if !can_gain_exp(have, level, modifier, xtralvl, verbose) {
            if let Some(s) = world.session_mut(id) {
                s.output.push(Msg::ExpCapped);
                s.push_prompt();
            }
            return ExpOutcome {
                added: false,
                capped: true,
            };
        }
    }
    let gang = plr.gang.clone();
    // The gang's share.
    if exp > 1
        && !gang.is_empty()
        && let Some(g) = world.gangs.get_mut(&gang.to_ascii_uppercase())
    {
        crate::gangs::add_kill_exp(g, exp as u32);
        // The gang record is marked changed
        let key = g.key.clone();
        crate::gangs::save_gang(world, &key);
    }
    let Some(plr) = world.session_mut(id).and_then(|s| s.character.as_mut()) else {
        return ExpOutcome {
            added: false,
            capped: false,
        };
    };
    // The old 32-bit total.
    plr.exp = plr.exp.wrapping_add(exp as u32);
    // The billions pair (unsigned comparisons).
    let mut left = exp as u32;
    while left >= BILLION {
        left -= BILLION;
        plr.expbil = plr.expbil.wrapping_add(1);
    }
    plr.exprem = plr.exprem.wrapping_add(left);
    if plr.exprem >= BILLION {
        plr.exprem -= BILLION;
        plr.expbil = plr.expbil.wrapping_add(1);
    }
    if verbose && let Some(s) = world.session_mut(id) {
        // The gain line prints the amount as a signed 32-bit number
        s.output.push(Msg::ExpGain {
            exp: i64::from(left as i32),
        });
        s.push_prompt();
    }
    world.update_top_list(id);
    ExpOutcome {
        added: true,
        capped: false,
    }
}
