//! One blow: [`resolve`].

use crate::rng::Rng;

use super::Move;
use super::fighter::Fighter;

/// What a blow did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The roll was not under the hit chance.
    Miss,
    /// Landed for `damage` after damage reduction and the BASH/SMASH multiplier.
    Hit {
        damage: i32,
        /// The critical roll came under the (compressed) crit chance.
        critical: bool,
    },
    /// Landed, but the defender's dodge roll came under the dodge chance.
    Dodged,
    /// Landed, but damage reduction left nothing.
    NoDamage,
}

/// The result of one blow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackResult {
    pub outcome: Outcome,
    /// The damage figure the messages show. It is the damage, so 0 for a miss, a dodge and
    /// an absorbed blow.
    pub rolled: i32,
    /// Poison given; always 0.
    pub poison: i32,
    /// The defender's `expval`, on a real hit only; else 0.
    pub exp: u32,
    /// The attacker's energy for the blow. The fighter keeps it in 32 bits; only the low
    /// word survives. With the 1000 cap on a player's blow the two never differ.
    pub energy: u16,
}

/// A row of the move table: the hit modifier, the damage-range
/// percentage and whether the move can critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveMods {
    pub hitmod: i32,
    /// Percent added to both ends of the damage range.
    pub dmgpct: i32,
    /// `false` zeroes the attacker's critical chance.
    pub can_crit: bool,
}

/// The move table: a plain attack and any other value get no
/// modifier; kick and jumpkick hit harder; bash, smash and move 8 hit harder but less
/// often; backstab, bash, smash and move 8 can never be critical hits.
pub const fn move_mods(mv: Move) -> MoveMods {
    let (hitmod, dmgpct, can_crit) = match mv {
        Move::Attack | Move::Punch => (0, 0, true),
        Move::Kick => (0, 33, true),
        Move::JumpKick => (0, 66, true),
        Move::Bash => (-15, 10, false),
        Move::Smash => (-25, 20, false),
        Move::Unused8 => (-75, 125, false),
        Move::Backstab => (0, 0, false),
    };
    MoveMods {
        hitmod,
        dmgpct,
        can_crit,
    }
}

/// The hit chance of a blow without the negative-dodge roll (see [`resolve`]): a backstab
/// is `av - dv`; otherwise `atkpower = (av+hitmod)^2/14/10` (two integer divisions in 32
/// bits) and
/// `hitchance = 100 - (dv+defalign)^2/atkpower`, or 5 without any attack
/// power. There is always at least a 10% chance to hit, and never 100%.
/// The hit roll is 1..=99, so 99 is `98/99` and 10 is `9/99`.
pub fn hit_chance(av: i32, hitmod: i32, dv: i32, defalign: i32, backstab: bool) -> i32 {
    let hitchance = if backstab {
        i64::from(av) - i64::from(dv)
    } else {
        let power = i64::from(av) + i64::from(hitmod);
        let atkpower = power * power / 14 / 10;
        if atkpower == 0 {
            5
        } else {
            let def = i64::from(dv) + i64::from(defalign);
            100 - def * def / atkpower
        }
    };
    hitchance.clamp(10, 99) as i32
}

/// The critical chance a blow rolls against: over 40 only a third
/// of the excess counts.
pub const fn effective_crit(crit: i32) -> i32 {
    if crit > 40 {
        (crit - 40) / 3 + 40
    } else {
        crit
    }
}

/// The defender's chance to dodge a landed blow: ten times its
/// dodge rating divided by an eighth of the attacker's attack value (0 when the attack
/// value is 8 or less), at most 95%, and a fifth of that against a backstab.
pub fn dodge_chance(dodge: i32, av: i32, backstab: bool) -> i32 {
    let mut chance = if av > 8 { dodge * 10 / (av / 8) } else { 0 };
    if chance > 95 {
        chance = 95;
    }
    if backstab {
        chance /= 5;
    }
    chance
}

/// One blow of `atk` on `def` with move `mv` (`atkmove`). As in the game the attacker's
/// damage range and critical chance are changed in place; the builders are called before
/// every blow, so nothing compounds. The rolls are made in the game's order: hit, negative
/// dodge, critical, damage, dodge. The game's random move and random percentage have no
/// effect and are not rolled.
pub fn resolve(atk: &mut Fighter, def: &Fighter, mv: Move, rng: &mut impl Rng) -> AttackResult {
    let mods = move_mods(mv);
    let backstab = mv == Move::Backstab;
    // the kind of blow; the moves that never critical zero the chance.
    if !mods.can_crit {
        atk.crit = 0;
    }
    // the defender's "Defence Modifier" is added to the attacker's modifier.
    let hitmod = mods.hitmod + def.defmod;
    // the damage bonus raises both ends of the damage range.
    if mods.dmgpct != 0 {
        atk.mindmg = (i64::from(atk.mindmg) * (i64::from(mods.dmgpct) + 100) / 100) as i32;
        atk.maxdmg = (i64::from(atk.maxdmg) * (i64::from(mods.dmgpct) + 100) / 100) as i32;
    }

    // the energy this blow costs (only the low word is kept), then the hit roll.
    let energy = atk.energy as u16;
    let roll = rng.range(1, 100);
    let result = |outcome, damage, exp| AttackResult {
        outcome,
        rolled: damage,
        poison: 0,
        exp,
        energy,
    };

    // a defender with a negative dodge rating is, with a chance of about that
    // many percent, caught off guard: then the blow is all but certain (99%).
    let hitchance = if def.dodge < 0 && i32::from(rng.pct()) > def.dodge + 100 {
        99
    } else {
        hit_chance(atk.av, hitmod, def.dv, def.defalign, backstab)
    };

    // critical hit chances over 40% count only a third above 40.
    if atk.crit > 40 {
        atk.crit = (atk.crit - 40) / 3 + 40;
    }

    // the roll decides whether the blow lands at all.
    if roll >= hitchance {
        return result(Outcome::Miss, 0, 0);
    }

    // a critical hit makes the damage range run from twice the old maximum to
    // four times the old maximum (bash and smash are checked again here, though their
    // critical chance was already set to 0 above).
    let mut critical = false;
    if i32::from(rng.pct()) < atk.crit && !mv.is_bash_or_smash() {
        critical = true;
        atk.mindmg = atk.maxdmg * 2;
        atk.maxdmg *= 4;
    }
    // the range is never inverted: the maximum is raised to the minimum.
    if atk.mindmg > atk.maxdmg {
        atk.maxdmg = atk.mindmg;
    }

    // the damage roll in the range (genrdn(0,n) is 0..n-1, so min..max), less
    // a tenth of the defender's damage resistance (dr is kept in tenths).
    let mut damage = rng.range(0, atk.maxdmg - atk.mindmg + 1);
    damage += atk.mindmg;
    damage -= def.dr / 10;

    // the defender's chance to dodge.
    let dodgechance = dodge_chance(def.dodge, atk.av, backstab);

    // bash does triple damage, smash five times the damage.
    match mv {
        Move::Bash => damage *= 3,
        Move::Smash => damage *= 5,
        _ => {}
    }

    // a dodged blow does no damage at all.
    if def.dodge > 0 && i32::from(rng.pct()) < dodgechance {
        return result(Outcome::Dodged, 0, 0);
    }
    // a blow the armour absorbs completely does no damage; a real hit copies
    // the defender's experience value into the result.
    if damage <= 0 {
        return result(Outcome::NoDamage, 0, 0);
    }
    result(Outcome::Hit { damage, critical }, damage, def.expval)
}
