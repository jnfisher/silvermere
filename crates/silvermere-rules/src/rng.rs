//! The random source every rule draws from: the game's `genrdn`/`lngrnd` on
//! 15-bit `rand()` values from a seeded `ChaCha8Rng`, so results match the game's.

use rand::{RngExt as _, SeedableRng as _};
use rand_chacha::ChaCha8Rng;

/// The largest value Borland's `rand()` returns.
pub const RAND_MAX: u16 = 0x7fff;

/// The game's `genrdn(min, max)`, also `lngrnd`, drawing from `rand`. A result is in
/// `[min, max)` only for `0 <= min` and `max <= 32768`: one `rand()` is at most 32767, so
/// `lngrnd(0, 50000)` never exceeds 32767, and a negative `min` never comes out.
pub fn genrdn(min: i64, max: i64, mut rand: impl FnMut() -> u16) -> i64 {
    if max == 0 {
        return 0;
    }
    if min >= max {
        return min;
    }
    let mut rnum = i64::from(rand()) % max;
    while rnum < min {
        rnum += i64::from(rand()) % (max - rnum);
    }
    rnum
}

/// The random source the rules take. Engine code and tests pass a [`SeededRng`]; any
/// `ChaCha8Rng` works too.
pub trait Rng {
    /// `genrdn(lo, hi)`.
    #[doc(alias = "genrdn")]
    fn range(&mut self, lo: i32, hi: i32) -> i32;

    /// `lngrnd(lo, hi)`.
    #[doc(alias = "lngrnd")]
    fn long(&mut self, lo: i64, hi: i64) -> i64;

    /// A percentile roll in `0..100`: `genrdn(0,100)`.
    fn pct(&mut self) -> u8 {
        // `range(0, 100)` is in 0..100, so the cast never truncates.
        self.range(0, 100) as u8
    }
}

/// One Borland `rand()` value from a ChaCha stream.
fn rand15(rng: &mut ChaCha8Rng) -> u16 {
    rng.random_range(0..=RAND_MAX)
}

impl Rng for ChaCha8Rng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        genrdn(lo.into(), hi.into(), || rand15(self)) as i32
    }

    fn long(&mut self, lo: i64, hi: i64) -> i64 {
        genrdn(lo, hi, || rand15(self))
    }
}

/// The engine's seeded random source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededRng {
    rng: ChaCha8Rng,
    draws: u64,
}

impl SeededRng {
    /// A source that always gives the same sequence for the same `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
            draws: 0,
        }
    }

    /// The next `rand()` value, `0..=RAND_MAX`.
    pub fn rand(&mut self) -> u16 {
        self.draws += 1;
        trace_draw();
        rand15(&mut self.rng)
    }

    /// How many `rand()` values have been drawn.
    pub fn draws(&self) -> u64 {
        self.draws
    }
}

/// With `SILVERMERE_RAND_TRACE` naming a file (read once, at the first
/// draw), each draw appends a line with the functions that made it,
/// innermost first, `SILVERMERE_RAND_TRACE_DEPTH` of them (4 by default; slow; for
/// debugging).
fn trace_draw() {
    static PATH: std::sync::OnceLock<Option<std::ffi::OsString>> = std::sync::OnceLock::new();
    static DEPTH: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let Some(path) = PATH.get_or_init(|| std::env::var_os("SILVERMERE_RAND_TRACE")) else {
        return;
    };
    let depth = *DEPTH.get_or_init(|| {
        std::env::var("SILVERMERE_RAND_TRACE_DEPTH")
            .ok()
            .and_then(|d| d.parse().ok())
            .unwrap_or(4)
    });
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    let names: Vec<&str> = trace
        .lines()
        .filter_map(|l| l.trim().split_once(": ").map(|(_, n)| n))
        .filter(|n| n.starts_with("silvermere_") && !n.starts_with("silvermere_rules::rng"))
        .filter(|n| !n.contains("{closure"))
        .map(|n| n.rsplit("::").next().unwrap_or(n))
        .take(depth)
        .collect();
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", names.join(" < "));
    }
}

impl Rng for SeededRng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        genrdn(lo.into(), hi.into(), || self.rand()) as i32
    }

    fn long(&mut self, lo: i64, hi: i64) -> i64 {
        genrdn(lo, hi, || self.rand())
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
