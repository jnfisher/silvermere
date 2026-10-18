//! Progression: experience needed per level, the training cap, kill experience, the
//! character-point cost ladder and what training a level costs and gives.
//!
//! Every function takes the few integers it needs and keeps the game's integer widths:
//! 32-bit signed as `i32`, 32-bit unsigned as `u32` (wrapping), and 16-bit for the rest.

use silvermere_types::records::BigExp;

use crate::rng::Rng;

/// Steps with their own growth factor in [`EXP_TABLE`].
pub const NEXPSTEPS: usize = 26;

/// Steps 26-53 grow by 15%.
const EXPEND15: i16 = 54;
/// Steps 54-56 grow by 9%, steps from 57 on by 8%.
const EXPEND9: i16 = 57;
/// The 15% growth factor's numerator.
const EXPFACT15: u32 = 115;
/// The 9% growth factor's numerator.
const EXPFACT9: u32 = 109;
/// The 8% growth factor's numerator.
const EXPFACT8: u32 = 108;
/// The fixed growth factors' denominator.
const EXPFACTDEN: u32 = 100;
/// The unit of the billions half of experience.
pub const BILLION: u32 = 1_000_000_000;

/// Lives a level of training adds.
pub const TRAINLIVES: i16 = 5;
/// The most lives a character holds.
pub const MAXLIVES: i16 = 9;
/// The default extra-levels option (`XTRALVL`): levels of experience a character may earn
/// beyond the level he has trained to.
pub const XTRALVL: i16 = 1;

/// The per-step growth factors `(numer, denom)` for steps 0-25. Both experience formulas
/// ([`new_calc_exp_needed`] and [`calc_exp_needed_old`]) use the same rows.
pub const EXP_TABLE: [(u32, u32); NEXPSTEPS] = [
    (1, 1),
    (40, 20),
    (44, 24),
    (44, 24),
    (48, 28),
    (48, 28),
    (52, 32),
    (52, 32),
    (56, 36),
    (56, 36),
    (60, 40),
    (60, 40),
    (65, 45),
    (65, 45),
    (70, 50),
    (70, 50),
    (75, 55),
    (50, 40),
    (50, 40),
    (50, 40),
    (50, 40),
    (50, 40),
    (50, 40),
    (50, 40),
    (50, 40),
    (23, 20),
];

/// The growth factor table, for callers that want it as a value.
pub const fn exp_table() -> &'static [(u32, u32); NEXPSTEPS] {
    &EXP_TABLE
}

/// The experience for `level` steps above level 1 on the 1.11 billions-plus-remainder
/// system, for a race and class experience percentage `modifier` (`race.exp + class.exp`).
///
/// Callers pass the level differently: the experience to have *reached* level `L` is
/// `new_calc_exp_needed(L-1, ..)` (lowering a character to a level); training from level
/// `L` to `L+1` checks `new_calc_exp_needed(L, ..)`; the earning cap is
/// `new_calc_exp_needed(L+xtralvl, ..)`. [`exp_needed_for_level`] wraps the "to reach level
/// `L`" convention.
///
/// The game's quirks are kept: the overflow rescaling in hundreds, the billions half growing
/// by `numer/1000` rather than `numer/denom`, the 32-bit wrap of that product, and `expbil`
/// keeping only the low 16 bits, sign-extended.
pub fn new_calc_exp_needed(level: i16, modifier: i16) -> BigExp {
    // The base is 10 x (100 + modifier), signed, kept unsigned.
    let mut rem = ((i32::from(modifier) + 100) * 1000 / 100) as u32;
    let mut nbil: u32 = 0;

    for i in 0..level {
        // The table for the first 26 steps, then the fixed factors.
        let (numer, denom) = if (i as usize) < NEXPSTEPS {
            EXP_TABLE[i as usize]
        } else if i < EXPEND15 {
            (EXPFACT15, EXPFACTDEN)
        } else if i < EXPEND9 {
            (EXPFACT9, EXPFACTDEN)
        } else {
            (EXPFACT8, EXPFACTDEN)
        };

        // Grow the remainder, rescaling by 100s on a 32-bit overflow.
        let mut product = rem.wrapping_mul(numer);
        if product >= rem && product / numer == rem {
            rem = product / denom;
        } else {
            let mut scales = 0u32;
            while product < rem || product / numer != rem {
                scales += 1;
                rem /= 100;
                product = rem.wrapping_mul(numer);
            }
            rem = if scales <= 1 {
                product / denom
            } else {
                // (sic) a second scale-down multiplies by 100 again.
                rem.wrapping_mul(numer).wrapping_mul(100) / denom
            };
            for _ in 0..scales {
                rem = rem.wrapping_mul(100);
            }
        }

        // (sic) the billions grow by numer/1000 of themselves.
        let mut carry = nbil.wrapping_mul(numer).wrapping_mul(1_000_000);
        while carry >= BILLION {
            carry -= BILLION;
            nbil = nbil.wrapping_add(1);
        }
        let mut total = rem.wrapping_add(carry);
        while total >= BILLION {
            total -= BILLION;
            nbil = nbil.wrapping_add(1);
        }
        rem = total;
    }

    BigExp {
        // The billions keep only their low 16 bits, sign-extended.
        expbil: i32::from(nbil as u16 as i16) as u32,
        exprem: rem,
    }
}

/// A [`BigExp`] as one number: `expbil * 1e9 + exprem`.
pub fn big_exp_total(exp: BigExp) -> u64 {
    u64::from(exp.expbil) * u64::from(BILLION) + u64::from(exp.exprem)
}

/// The experience a character must have to be `level`, for the race and class experience
/// percentage `exp_mod`: `new_calc_exp_needed(level-1, exp_mod)`. Level 1 (and 0) needs
/// nothing.
///
/// Training from level `L` needs `exp_needed_for_level(L+1, ..)`.
pub fn exp_needed_for_level(level: u16, exp_mod: i16) -> u64 {
    if level <= 1 {
        return 0;
    }
    // Levels are well inside 16 bits; clamp rather than wrap a nonsense argument.
    let steps = i16::try_from(level - 1).unwrap_or(i16::MAX);
    big_exp_total(new_calc_exp_needed(steps, exp_mod))
}

/// The older experience formula, which the game still shows in help text, the saved
/// character record and prompt substitutions. It starts at `1000*(100+exp_mod)/100` like
/// the new one, uses [`EXP_TABLE`] for steps 0-25 and then 115/100 to step 52, 110/100 for
/// steps 53-60 and 105/100 from step 61, all in one `u32` with the same overflow rescaling
/// in hundreds.
pub fn calc_exp_needed_old(level: i16, exp_mod: i16) -> u32 {
    // The base is 10 x (100 + exp_mod).
    let mut total = (1000 * (i32::from(exp_mod) + 100) / 100) as u32;
    for i in 0..level {
        let (mult, divisor) = if (i as usize) < NEXPSTEPS {
            EXP_TABLE[i as usize]
        } else if i > 60 {
            (105, 100)
        } else if i > 52 {
            (110, 100)
        } else {
            (115, 100)
        };
        let prod = total.wrapping_mul(mult);
        if prod >= total && prod / mult == total {
            total = prod / divisor;
        } else {
            total /= 100;
            let prod = total.wrapping_mul(mult);
            if prod >= total && prod / mult == total {
                total = prod / divisor;
            } else {
                total /= 100;
                total = total.wrapping_mul(mult) / divisor;
                total = total.wrapping_mul(100);
            }
            total = total.wrapping_mul(100);
        }
    }
    total
}

/// The earning cap when experience is added: may a character of `level` holding `have`
/// still earn experience? He may not once he holds more than
/// `new_calc_exp_needed(level+xtralvl, exp_mod)` ("You have progressed too far without
/// training to acquire anymore experience!").
///
/// (sic) When only his billions are past the cap, the refusal happens only for a `verbose`
/// call; a silent call lets the experience through. A character with
/// exactly the cap may still earn. The demo-mode level-4 cap and the pre-1.11 conversion
/// refusal are not reproduced.
pub fn can_gain_exp(have: BigExp, level: i16, exp_mod: i16, xtralvl: i16, verbose: bool) -> bool {
    let need = new_calc_exp_needed(level.wrapping_add(xtralvl), exp_mod);
    let past = (need.expbil < have.expbil && verbose)
        || (need.expbil == have.expbil && need.exprem < have.exprem);
    !past
}

/// Experience for beating a monster: its experience times its level, unsigned 32-bit and
/// wrapping. Splitting it among a party is the engine's (`distribute_experience`).
pub fn kill_exp(monster_exp: i32, monster_level: u32) -> u64 {
    u64::from((monster_exp as u32).wrapping_mul(monster_level))
}

/// Character points to raise a stat from the race minimum `base` to `target`. Each band of
/// 10 points above the minimum
/// costs one more CP per point (1, 2, ... 9), then 10 per point beyond 90; the full bands
/// add up to 10, 30, 60, 100, 150, 210, 280, 360 and 450. At or below the minimum it is 0.
///
/// The game keeps it in 16 bits; that would wrap only past 3,321 points above the minimum,
/// which no stat reaches, so the result is a plain `u32`.
pub fn cp_cost(base: u16, target: u16) -> u32 {
    if target <= base {
        return 0;
    }
    let over = u32::from(target - base);
    if over > 90 {
        return (over - 90) * 10 + 450;
    }
    // Band `b` (1..=9) costs `b` per point after the full bands.
    let band = over.div_ceil(10);
    let full = band - 1;
    (over - full * 10) * band + 5 * full * (full + 1)
}

/// Character points training to `new_level` gives: 10 up to level 10, 15 up to level 20,
/// then `((level-1)/10-2)*5+20` (20 for 21-30, 25 for 31-40, ...). The same rule gives the
/// level-based CP when a character's secondary stats are worked out.
pub fn cp_gain_for_level(new_level: u16) -> u16 {
    if new_level <= 10 {
        10
    } else if new_level <= 20 {
        15
    } else {
        ((new_level - 1) / 10 - 2) * 5 + 20
    }
}

/// The training fee in copper for a character of `level` (before training) at a trainer
/// with `markup` percent: `level*5` silver, times `(markup+100)/100` in whole silver, then
/// charged as silver, so 10 copper each.
///
/// The markup is applied to silver, so the fee is always a whole number of silver: level
/// 10 at 25% is 62 silver (620 copper), not 625 copper. `level*5` is 16-bit unsigned and
/// the markup product 32-bit unsigned, both wrapping, as in the game.
pub fn training_fee_copper(level: u16, markup: u16) -> u64 {
    // 16-bit unsigned
    let silver = u32::from(level.wrapping_mul(5));
    // 32-bit unsigned
    let silver = silver.wrapping_mul(u32::from(markup) + 100) / 100;
    u64::from(crate::economy::convert_currency(0, 0, 0, silver, 0))
}

/// What training a level gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainGains {
    /// Character points added to `cp` and `totalcp`.
    pub cp: u16,
    /// Random hit points added to `randomhp` (0 when the roll is not positive).
    pub hp: i16,
    /// The character's lives after training (not the number added).
    pub lives: i16,
}

/// The gains of training to `new_level` for a class with `hplevel` random hit points per
/// level, for a character holding `lives`:
///
/// - CP by [`cp_gain_for_level`];
/// - hit points `genrdn(0, hplevel+1)`, so `0..=hplevel` (an empty range, so 0, for a
///   negative `hplevel`), kept in a signed byte and added only when positive;
/// - lives `+TRAINLIVES`, then cut to [`MAXLIVES`], so a character
///   already above the cap is brought down to it.
pub fn train_gains(new_level: u16, hplevel: i16, lives: i16, rng: &mut impl Rng) -> TrainGains {
    let hi = i32::from(hplevel) + 1;
    let roll = rng.range(0, if hi < 0 { 0 } else { hi }) as i8;
    let hp = if roll > 0 { i16::from(roll) } else { 0 };
    let mut new_lives = lives.wrapping_add(TRAINLIVES);
    if new_lives > MAXLIVES {
        new_lives = MAXLIVES;
    }
    TrainGains {
        cp: cp_gain_for_level(new_level),
        hp,
        lives: new_lives,
    }
}
