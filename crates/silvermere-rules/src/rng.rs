//! The random source every rule draws from.
//!
//! The game draws from the host's `genrdn(lo, hi)` (an integer in `[lo, hi)`, never `hi`)
//! and `lngrnd` for longs. silvermere reproduces the distributions, not the host's sequence
//! [`SeededRng`] wraps `ChaCha8Rng`, so a world or a test
//! seeded the same way replays the same draws.

use rand::{RngExt as _, SeedableRng as _};
use rand_chacha::ChaCha8Rng;

/// The random source the rules take. Engine code and tests pass a [`SeededRng`]; any
/// `ChaCha8Rng` works too.
pub trait Rng {
    /// A uniform `i32` in `[lo, hi)`, like the host's `genrdn(lo, hi)`: never `hi`. An empty
    /// range (`hi <= lo`) gives `lo`, as the host did for `genrdn(0,0)`.
    #[doc(alias = "genrdn")]
    fn range(&mut self, lo: i32, hi: i32) -> i32;

    /// A uniform `i64` in `[lo, hi)`, the long form (`lngrnd`). An empty range gives `lo`.
    #[doc(alias = "lngrnd")]
    fn long(&mut self, lo: i64, hi: i64) -> i64;

    /// A percentile roll in `0..100`: `genrdn(0,100)`.
    fn pct(&mut self) -> u8 {
        // `range(0, 100)` is in 0..100, so the cast never truncates.
        self.range(0, 100) as u8
    }
}

impl Rng for ChaCha8Rng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        self.random_range(lo..hi)
    }

    fn long(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        self.random_range(lo..hi)
    }
}

/// The engine's seeded random source: `ChaCha8Rng` from a 64-bit seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededRng(ChaCha8Rng);

impl SeededRng {
    /// A source that always gives the same sequence for the same `seed`.
    pub fn new(seed: u64) -> Self {
        Self(ChaCha8Rng::seed_from_u64(seed))
    }
}

impl Rng for SeededRng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        self.0.range(lo, hi)
    }

    fn long(&mut self, lo: i64, hi: i64) -> i64 {
        self.0.long(lo, hi)
    }
}

/// Pearson's chi-square statistic of `observed` counts against a uniform distribution over
/// the same number of buckets. Distribution tests (layer 1 in `docs/testing.md`)
/// compare it with [`chi_square_critical_001`].
pub fn chi_square_uniform(observed: &[u64]) -> f64 {
    let total: u64 = observed.iter().sum();
    if observed.is_empty() || total == 0 {
        return 0.0;
    }
    let expected = total as f64 / observed.len() as f64;
    observed
        .iter()
        .map(|&o| {
            let d = o as f64 - expected;
            d * d / expected
        })
        .sum()
}

/// The chi-square value a uniform sample exceeds with probability 0.01, for `df` degrees
/// of freedom (Wilson-Hilferty approximation; within 1% of the tables for `df >= 3`).
pub fn chi_square_critical_001(df: u32) -> f64 {
    // The standard normal quantile for 0.99.
    const Z: f64 = 2.326_348;
    let k = f64::from(df.max(1));
    let a = 2.0 / (9.0 * k);
    k * (1.0 - a + Z * a.sqrt()).powi(3)
}
