//! Layer 1 tests for the `Rng` trait: the `genrdn` range semantics and the distribution.

use silvermere_rules::rng::{chi_square_critical_001, chi_square_uniform};
use silvermere_rules::{Rng, SeededRng};

// Host `genrdn(lo,hi)`: [lo, hi), never hi.
#[test]
fn range_1_100_never_returns_100_and_is_uniform() {
    let mut rng = SeededRng::new(0x5EED);
    let mut counts = [0u64; 99];
    for _ in 0..100_000 {
        let r = rng.range(1, 100);
        assert!((1..100).contains(&r), "genrdn(1,100) gave {r}");
        counts[(r - 1) as usize] += 1;
    }
    let chi = chi_square_uniform(&counts);
    let crit = chi_square_critical_001(98);
    assert!(chi < crit, "chi-square {chi} >= {crit}");
}

#[test]
fn same_seed_same_sequence() {
    let mut a = SeededRng::new(42);
    let mut b = SeededRng::new(42);
    let xs: Vec<i32> = (0..1000).map(|_| a.range(0, 1000)).collect();
    let ys: Vec<i32> = (0..1000).map(|_| b.range(0, 1000)).collect();
    assert_eq!(xs, ys);
    let mut c = SeededRng::new(43);
    let zs: Vec<i32> = (0..1000).map(|_| c.range(0, 1000)).collect();
    assert_ne!(xs, zs);
}

// Training a level with a negative hit-point gain draws from an empty range; it gives 0.
#[test]
fn empty_range_gives_lo() {
    let mut rng = SeededRng::new(1);
    assert_eq!(rng.range(0, 0), 0);
    assert_eq!(rng.range(5, 3), 5);
    assert_eq!(rng.long(7, 7), 7);
}

#[test]
fn long_and_pct_stay_in_range() {
    let mut rng = SeededRng::new(7);
    for _ in 0..10_000 {
        let l = rng.long(-5_000_000_000, 5_000_000_000);
        assert!((-5_000_000_000..5_000_000_000).contains(&l));
        assert!(rng.pct() < 100);
    }
}

#[test]
fn chi_square_critical_matches_tables() {
    // Tabulated 0.99 quantiles: df 6 -> 16.812, df 98 -> 133.476.
    assert!((chi_square_critical_001(6) - 16.812).abs() < 0.2);
    assert!((chi_square_critical_001(98) - 133.476).abs() < 0.5);
}
