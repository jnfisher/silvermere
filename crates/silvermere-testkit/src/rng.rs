//! Seeded randomness for tests: `rand_chacha::ChaCha8Rng`, which implements
//! `silvermere_rules::Rng`, plus a chi-square uniformity check.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A reproducible generator: the same seed gives the same sequence on every run and
/// platform.
pub fn seeded_rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// Pearson's chi-square statistic of `samples` (values in `0..bins`) against a uniform
/// distribution over `bins` outcomes.
pub fn chi_square(samples: &[u32], bins: usize) -> f64 {
    let mut counts = vec![0u64; bins];
    for &s in samples {
        counts[s as usize] += 1;
    }
    silvermere_rules::rng::chi_square_uniform(&counts)
}

/// Asserts `samples` (each in `0..bins`) look uniform: the chi-square statistic divided by
/// its degrees of freedom (`bins - 1`) must not exceed `tolerance`. A fair source gives
/// about 1.0; 1.5 is far in the tail for 99 degrees of freedom.
pub fn assert_uniform(samples: &[u32], bins: usize, tolerance: f64) {
    assert!(bins >= 2, "need at least two bins");
    assert!(
        samples.iter().all(|&s| (s as usize) < bins),
        "a sample is outside 0..{bins}"
    );
    let stat = chi_square(samples, bins) / (bins - 1) as f64;
    assert!(
        stat <= tolerance,
        "chi-square per degree of freedom {stat:.3} exceeds {tolerance} over {} samples",
        samples.len()
    );
}
