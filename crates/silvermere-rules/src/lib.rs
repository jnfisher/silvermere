//! Pure game rules: combat, spells, regen, exp, prices, movement checks.
//!
//! Every function here is a pure function of (state, data, rng) to an outcome: to-hit and
//! damage, spell effects and resistance, experience and level-up gains, buy and sell prices,
//! regeneration, light, evil and spawn maths. No I/O and no shared mutable state, so the
//! formulas can be tested in isolation against the original's behaviour. Randomness comes in
//! through [`Rng`] and time through [`Clock`].

#![forbid(unsafe_code)]

pub mod abilities;
pub mod clock;
pub mod combat;
pub mod economy;
pub mod evil;
pub mod hangup;
pub mod light;
pub mod monster_ai;
pub mod progression;
pub mod regen;
pub mod rng;
pub mod spawn;
pub mod spells;

pub use clock::{Clock, SystemClock};
pub use rng::{Rng, SeededRng};
