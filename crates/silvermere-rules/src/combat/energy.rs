//! Energy per blow and blows per round.

/// `maxenergy` of every player: set when the player is created and never changed.
pub const MAXENERGY: u32 = 1000;
/// The floor every attack's energy is clamped to.
pub const MIN_ENERGY: u32 = 200;
/// What the background tick adds to a fighter's energy.
pub const TICK_ENERGY: i32 = 1000;
/// The most blows one round makes: blows go on while the count so far is at most 5.
pub const MAX_BLOWS: u32 = 6;

/// The energy one blow with a weapon of
/// `speed` costs a fighter of `level`, class `combat` value and `agility`, plus the weapon's
/// own `extra` energy.
///
/// `skill = level*combat + 45`; `quick = agility + 150`; `divisor =
/// skill*quick*1500/9000`; `cost = extra + speed*1000/divisor`. A zero divisor gives the
/// fallback 50. The products are 32-bit.
pub fn compute_energy_used(level: i32, combat: i32, agility: i32, speed: i32, extra: i32) -> i32 {
    let skill = i64::from(level) * i64::from(combat) + 45;
    let quick = i64::from(agility) + 150;
    let divisor = skill * quick * 1500 / 9000;
    if divisor == 0 {
        return 50;
    }
    let cost = i64::from(extra) + i64::from(speed) * 1000 / divisor;
    // A 32-bit result the game keeps in 16 bits; sane sheets never reach the limit.
    cost.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// What one energy round gives: how many blows it makes and the energy left for the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnergyRound {
    /// Blows made this round, at most [`MAX_BLOWS`].
    pub blows: u32,
    /// Energy left after the last blow; auto-combat carries it into the next round, so
    /// `1000/605` gives one blow then two, alternating.
    pub leftover: i32,
}

/// One round of blows: the tick adds [`TICK_ENERGY`] to `carry`, then blows of `cost` are
/// made while the energy lasts, at most [`MAX_BLOWS`]. A round starts only once the fighter
/// is at full energy, so a first round always begins with no carry and the full tick.
///
/// `cost` below 1 is treated as [`MIN_ENERGY`], which every attack is floored to anyway.
pub fn energy_round(carry: i32, cost: i32) -> EnergyRound {
    let cost = if cost < 1 { MIN_ENERGY as i32 } else { cost };
    let mut energy = carry.saturating_add(TICK_ENERGY);
    let mut blows = 0;
    while blows < MAX_BLOWS && energy >= cost {
        energy -= cost;
        blows += 1;
    }
    EnergyRound {
        blows,
        leftover: energy,
    }
}
