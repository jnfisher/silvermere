//! `TestClock`, the seeded RNG and the uniformity helper.

use std::time::Duration;

use silvermere_rules::{Clock, Rng};
use silvermere_testkit::rng::chi_square;
use silvermere_testkit::{TestClock, assert_uniform, seeded_rng};

#[test]
fn test_clock_only_moves_when_advanced() {
    let clock = TestClock::new();
    let t0 = clock.now();
    assert_eq!(clock.now(), t0, "time stands still");
    clock.advance(Duration::from_millis(1500));
    assert_eq!(clock.now() - t0, Duration::from_millis(1500));
    clock.advance_ticks(3);
    assert_eq!(clock.now() - t0, Duration::from_millis(4500));
    let shared = clock.clone();
    shared.advance(Duration::from_secs(1));
    assert_eq!(
        Clock::now(&clock) - t0,
        Duration::from_millis(5500),
        "clones share a timeline"
    );
}

#[test]
fn seeded_rng_is_reproducible() {
    let draw = |seed| {
        let mut rng = seeded_rng(seed);
        (0..100)
            .map(|_| rng.range(0, 100) as u32)
            .collect::<Vec<_>>()
    };
    assert_eq!(draw(7), draw(7), "same seed, same first 100 values");
    assert_ne!(draw(7), draw(8));
    // pinned: a change here means the generator or the seeding changed under every test
    assert_eq!(draw(7), PINNED_SEED_7);
}

const PINNED_SEED_7: [u32; 100] = [
    87, 70, 65, 4, 50, 77, 83, 13, 31, 2, 72, 75, 30, 21, 75, 29, 83, 42, 64, 39, 52, 61, 39, 91,
    8, 80, 93, 31, 76, 80, 78, 53, 46, 27, 31, 82, 16, 80, 23, 22, 96, 60, 1, 61, 9, 71, 9, 88, 6,
    30, 96, 91, 79, 34, 31, 73, 7, 61, 4, 17, 33, 47, 83, 14, 43, 86, 9, 12, 89, 62, 53, 13, 8, 62,
    59, 61, 14, 77, 6, 64, 83, 99, 59, 26, 13, 88, 72, 87, 5, 27, 31, 41, 59, 27, 34, 6, 8, 11, 98,
    92,
];

#[test]
fn rng_range_never_returns_the_bound() {
    let mut rng = seeded_rng(1);
    assert!((0..5000).all(|_| rng.range(0, 3) < 3));
    assert!((0..100).all(|_| rng.range(0, 1) == 0));
}

#[test]
fn uniform_draws_pass_the_chi_square_check() {
    let mut rng = seeded_rng(7);
    let samples: Vec<u32> = (0..10_000).map(|_| rng.range(0, 100) as u32).collect();
    assert_uniform(&samples, 100, 1.5);
}

#[test]
#[should_panic(expected = "chi-square")]
fn a_skewed_source_fails_the_check() {
    let samples: Vec<u32> = (0..10_000u32)
        .map(|i| if i % 4 == 0 { 99 } else { i % 50 })
        .collect();
    assert_uniform(&samples, 100, 1.5);
}

#[test]
fn chi_square_of_a_perfectly_flat_sample_is_zero() {
    let samples: Vec<u32> = (0..1000).map(|i| i % 10).collect();
    assert!(chi_square(&samples, 10).abs() < 1e-9);
}

#[test]
fn test_clock_wall_time_and_date_follow_the_offset() {
    let clock = TestClock::new();
    let (w0, d0) = (clock.wall(), clock.today());
    assert_eq!(d0.to_string(), "2026-01-01");
    clock.advance(Duration::from_secs(36 * 3600));
    assert_eq!(Clock::wall(&clock).as_second() - w0.as_second(), 36 * 3600);
    assert_eq!(Clock::today(&clock).to_string(), "2026-01-02");
}
