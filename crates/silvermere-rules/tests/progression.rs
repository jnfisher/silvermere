//! Layer 1 tests for `progression`: each test names the rule it checks.

use proptest::prelude::*;
use silvermere_rules::SeededRng;
use silvermere_rules::progression::{
    BILLION, EXP_TABLE, MAXLIVES, XTRALVL, big_exp_total, calc_exp_needed_old, can_gain_exp,
    cp_cost, cp_gain_for_level, exp_needed_for_level, kill_exp, new_calc_exp_needed, train_gains,
    training_fee_copper,
};
use silvermere_rules::rng::{chi_square_critical_001, chi_square_uniform};
use silvermere_types::records::BigExp;

/// Kang Witchunter: race + class experience percentage 65, from the observed level-2
/// requirement of 1,650.
const KANG_WITCHUNTER: i16 = 65;

/// An independent oracle for `new_calc_exp_needed`, written from the table rows and the
/// rescaling rule, in 64-bit arithmetic: a step overflows 32 bits exactly when
/// `rem*numer >= 2^32`; then `rem` is cut by hundreds until it fits, grown, and put back.
fn oracle_exp(level: u16, modifier: i64) -> u64 {
    const TABLE: [(u64, u64); 26] = [
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
    const B: u64 = 1_000_000_000;
    let mut rem: u64 = ((modifier + 100) * 1000 / 100) as u64;
    let mut bil: u64 = 0;
    for step in 0..u64::from(level) {
        let (n, d) = match step {
            0..=25 => TABLE[step as usize],
            26..=53 => (115, 100),
            54..=56 => (109, 100),
            _ => (108, 100),
        };
        let mut hundreds = 0;
        while rem * n >= 1 << 32 {
            rem /= 100;
            hundreds += 1;
        }
        assert!(
            hundreds <= 1,
            "the (sic) double scale-down branch is unreachable"
        );
        rem = rem * n / d * 100u64.pow(hundreds);
        // The billions grow by n/1000 of themselves, in 32 bits; whole billions of the
        // carry move over, then those of the remainder plus the carry (also 32-bit).
        let carry = (bil * n * 1_000_000) % (1 << 32);
        bil += carry / B;
        let total = (rem + carry % B) % (1 << 32);
        bil += total / B;
        rem = total % B;
    }
    bil * B + rem
}

// The base is 10 x (100 + modifier); L2 is one x1 step (Kang Witchunter level 2 at 1,650).
#[test]
fn kang_witchunter_level_2_is_1650_and_level_3_is_3300() {
    assert_eq!(exp_needed_for_level(2, KANG_WITCHUNTER), 1_650);
    // Step 1, 40/20, doubles it.
    assert_eq!(exp_needed_for_level(3, KANG_WITCHUNTER), 3_300);
}

// new_calc_exp_needed, worked by hand from the table rows.
#[test]
fn first_levels_by_hand() {
    // 1650; x40/20 = 3300; x44/24 = 6050; x44/24 = 11091; x48/28 = 19013; x48/28 = 32593;
    // x52/32 = 52963; x52/32 = 86064.
    let by_hand = [
        0, 0, 1_650, 3_300, 6_050, 11_091, 19_013, 32_593, 52_963, 86_064,
    ];
    for (level, &want) in by_hand.iter().enumerate().skip(1) {
        assert_eq!(
            exp_needed_for_level(level as u16, KANG_WITCHUNTER),
            want,
            "level {level}"
        );
    }
}

// new_calc_exp_needed against the 64-bit oracle over the table rows 0-25.
#[test]
fn exp_needed_levels_1_to_75_match_the_table_oracle() {
    for level in 2..=75u16 {
        assert_eq!(
            exp_needed_for_level(level, KANG_WITCHUNTER),
            oracle_exp(level - 1, i64::from(KANG_WITCHUNTER)),
            "level {level}"
        );
    }
    assert_eq!(exp_needed_for_level(1, KANG_WITCHUNTER), 0);
}

// Whole billions move into expbil; exprem stays below a billion.
#[test]
fn billion_split_is_exercised() {
    let level = (2..=200u16)
        .find(|&l| exp_needed_for_level(l, KANG_WITCHUNTER) > u64::from(BILLION))
        .expect("some level needs over a billion");
    let big = new_calc_exp_needed((level - 1) as i16, KANG_WITCHUNTER);
    assert!(big.expbil >= 1, "level {level}: {big:?}");
    assert!(big.exprem < BILLION);
    assert_eq!(
        big_exp_total(big),
        exp_needed_for_level(level, KANG_WITCHUNTER)
    );
    let prev = new_calc_exp_needed((level - 2) as i16, KANG_WITCHUNTER);
    assert_eq!(prev.expbil, 0);
}

// The growth table's rows 0-25.
#[test]
fn exp_table_rows() {
    assert_eq!(EXP_TABLE.len(), 26);
    assert_eq!(EXP_TABLE[0], (1, 1));
    assert_eq!(EXP_TABLE[1], (40, 20));
    assert_eq!(EXP_TABLE[16], (75, 55));
    assert_eq!(EXP_TABLE[17], (50, 40));
    assert_eq!(EXP_TABLE[24], (50, 40));
    assert_eq!(EXP_TABLE[25], (23, 20));
}

// The old formula: same base and table, its own fixed factors.
#[test]
fn old_calc_exp_needed_agrees_on_the_table_steps() {
    for level in 0..=20i16 {
        assert_eq!(
            u64::from(calc_exp_needed_old(level, KANG_WITCHUNTER)),
            big_exp_total(new_calc_exp_needed(level, KANG_WITCHUNTER)),
            "steps {level}"
        );
    }
}

// Adding experience: the cap is the experience for level + xtralvl.
#[test]
fn earning_stops_past_the_xtralvl_cap() {
    let cap = new_calc_exp_needed(5 + XTRALVL, KANG_WITCHUNTER);
    assert!(can_gain_exp(cap, 5, KANG_WITCHUNTER, XTRALVL, true));
    let over = BigExp {
        expbil: cap.expbil,
        exprem: cap.exprem + 1,
    };
    assert!(!can_gain_exp(over, 5, KANG_WITCHUNTER, XTRALVL, true));
    assert!(!can_gain_exp(over, 5, KANG_WITCHUNTER, XTRALVL, false));
    // (sic) more billions than the cap refuse only a verbose call.
    let rich = BigExp {
        expbil: cap.expbil + 1,
        exprem: 0,
    };
    assert!(!can_gain_exp(rich, 5, KANG_WITCHUNTER, XTRALVL, true));
    assert!(can_gain_exp(rich, 5, KANG_WITCHUNTER, XTRALVL, false));
}

// A monster's kill experience is its experience times its level.
#[test]
fn kill_exp_is_exp_times_level() {
    assert_eq!(kill_exp(9, 1), 9);
    assert_eq!(kill_exp(100, 3), 300);
    // 32-bit unsigned arithmetic wraps
    assert_eq!(kill_exp(-1, 1), u64::from(u32::MAX));
}

/// The stat cost written out as a chain of band checks, one per band edge.
fn stat_amount_used(base: u32, target: u32) -> u32 {
    if target <= base {
        return 0;
    }
    if target <= base + 10 {
        return target - base;
    }
    for (band, full) in [
        (2, 10),
        (3, 30),
        (4, 60),
        (5, 100),
        (6, 150),
        (7, 210),
        (8, 280),
        (9, 360),
    ] {
        if target <= base + band * 10 {
            return (target - (band - 1) * 10 - base) * band + full;
        }
    }
    (target - 90 - base) * 10 + 450
}

// The stat cost: the nine band edges.
#[test]
fn cp_cost_band_edges() {
    for (over, want) in [
        (0, 0),
        (10, 10),
        (20, 30),
        (30, 60),
        (40, 100),
        (50, 150),
        (60, 210),
        (70, 280),
        (80, 360),
        (90, 450),
        (91, 460),
    ] {
        assert_eq!(cp_cost(70, 70 + over), want, "+{over}");
    }
    assert_eq!(cp_cost(70, 60), 0);
    assert_eq!(cp_cost(70, 71), 1);
    assert_eq!(cp_cost(70, 81), 12);
}

// The stat cost: every (base, target) with target - base in 0..=120.
#[test]
fn cp_cost_equals_stat_amount_used() {
    for base in 0..=120u16 {
        for over in 0..=120u16 {
            let target = base + over;
            assert_eq!(
                cp_cost(base, target),
                stat_amount_used(u32::from(base), u32::from(target)),
                "{base} -> {target}"
            );
        }
    }
}

proptest! {
    // The stat cost: raising the target never costs less.
    #[test]
    fn cp_cost_is_monotone(base in 0u16..200, target in 0u16..400) {
        prop_assert!(cp_cost(base, target + 1) >= cp_cost(base, target));
        prop_assert!(cp_cost(base + 1, target) <= cp_cost(base, target));
    }
}

// Training a level: 10 to L10, 15 to L20, then ((level-1)/10-2)*5+20.
#[test]
fn cp_gain_bands() {
    for level in 2..=40u16 {
        let want = match level {
            ..=10 => 10,
            11..=20 => 15,
            21..=30 => 20,
            _ => 25,
        };
        assert_eq!(cp_gain_for_level(level), want, "level {level}");
    }
    assert_eq!(cp_gain_for_level(41), 30);
    // The last formula alone gives the same bands at every edge: the first two bands only
    // restate it.
    for level in [2u16, 10, 11, 20, 21, 30, 31, 40, 41, 50, 51] {
        let int_level = i32::from(level); // signed: (1/10-2) is -2, not a wrap
        assert_eq!(
            i32::from(cp_gain_for_level(level)),
            ((int_level - 1) / 10 - 2) * 5 + 20,
            "level {level}"
        );
    }
}

// Training fee: level*5 silver, times (markup+100)/100 in silver.
#[test]
fn training_fee() {
    assert_eq!(training_fee_copper(2, 0), 100);
    assert_eq!(training_fee_copper(2, 50), 150);
    // 50 silver * 125 / 100 = 62 silver (the 0.5 is dropped in silver), not 625 copper.
    assert_eq!(training_fee_copper(10, 25), 620);
    assert_eq!(training_fee_copper(1, 0), 50);
    // The stock class trainers (WCCSHOPS 14-31, e.g. 19 "Witchunter Training Room") mark
    // up 300%: 5 * 400 / 100 = 20 silver at level 1. The universal one (38) has 0.
    assert_eq!(training_fee_copper(1, 300), 200);
}

// Training a level: the hit point gain is genrdn(0, hplevel+1), so 0..=hplevel, uniform.
#[test]
fn train_gains_hp_is_uniform_in_0_to_hplevel() {
    let hplevel = 6i16;
    let mut rng = SeededRng::new(0x7EA1);
    let mut counts = [0u64; 7];
    for _ in 0..1_000 {
        let g = train_gains(5, hplevel, 3, &mut rng);
        assert!((0..=hplevel).contains(&g.hp));
        counts[g.hp as usize] += 1;
        assert_eq!(g.cp, 10);
    }
    let chi = chi_square_uniform(&counts);
    assert!(chi < chi_square_critical_001(6), "chi-square {chi}");
}

// Training a level: a negative hplevel draws from an empty range, so 0.
#[test]
fn train_gains_negative_hplevel_gives_no_hp() {
    let mut rng = SeededRng::new(3);
    assert_eq!(train_gains(2, -4, 0, &mut rng).hp, 0);
}

// Training a level: lives += TRAINLIVES (5), capped at MAXLIVES (9).
#[test]
fn train_gains_lives_never_exceed_the_cap() {
    let mut rng = SeededRng::new(9);
    for lives in -2..=12i16 {
        let g = train_gains(3, 4, lives, &mut rng);
        assert!(g.lives <= MAXLIVES);
        assert_eq!(g.lives, (lives + 5).min(MAXLIVES));
    }
}

/// The committed oracle: `tools/parsegen/expneeded.py`, an independent Python version of
/// the experience formula, run once.
fn expneeded_fixture() -> Vec<(i16, i16, u32, u32)> {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/expneeded.json")).expect("fixture parses");
    doc["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| {
            let n = |i: usize| r[i].as_i64().expect("integer");
            (n(0) as i16, n(1) as i16, n(2) as u32, n(3) as u32)
        })
        .collect()
}

// new_calc_exp_needed against the Python oracle: levels 1..60
// for modifiers 0, 30, 65 and 100, and 0..100 for -20, 65 and 200, bit for bit.
#[test]
fn exp_needed_matches_the_python_oracle() {
    let rows = expneeded_fixture();
    assert!(rows.len() > 400, "{} rows", rows.len());
    for (level, modifier, expbil, exprem) in rows {
        assert_eq!(
            new_calc_exp_needed(level, modifier),
            BigExp { expbil, exprem },
            "level {level} modifier {modifier}"
        );
    }
}

/// The exact product `base * prod(numer/denom)` stepwise in `u128`, truncating each step
/// only to an integer: what the function would give without the 32-bit rescaling and
/// with the billions growing like the rest.
fn exact_exp(level: i16, modifier: i16) -> u128 {
    let mut total = ((i32::from(modifier) + 100) * 1000 / 100) as u128;
    for i in 0..level {
        let (n, d) = match i {
            0..=25 => EXP_TABLE[i as usize],
            26..=53 => (115, 100),
            54..=56 => (109, 100),
            _ => (108, 100),
        };
        total = total * u128::from(n) / u128::from(d);
    }
    total
}

// The game's value equals the exact stepwise product while `rem * numer` fits in 32 bits
// and no billion has been carried. They part at the first step whose product overflows
// (the rescale drops the low two digits): for Kang Witchunter (+65) at level argument 33
// (47,641,900 against the exact 47,642,041), for +0 at 36, for +200 at 28. From there the
// game's value is the one kept; the test checks each parting follows an overflow.
#[test]
fn exact_u128_reference_agrees_until_the_first_overflow() {
    for modifier in [-20i16, 0, 30, 65, 100, 200] {
        let mut parted = None;
        for level in 0..=100i16 {
            let c = big_exp_total(new_calc_exp_needed(level, modifier));
            let exact = exact_exp(level, modifier);
            if parted.is_none() && u128::from(c) != exact {
                parted = Some(level);
            }
        }
        let parted = parted.expect("the two part somewhere below level 100");
        if modifier == KANG_WITCHUNTER {
            assert_eq!(parted, 33);
        }
        // The step before the parting overflowed 32 bits: rem * numer >= 2^32.
        let step = parted - 1;
        let rem_before = new_calc_exp_needed(step, modifier).exprem;
        let numer = match step {
            0..=25 => EXP_TABLE[step as usize].0,
            26..=53 => 115,
            54..=56 => 109,
            _ => 108,
        };
        assert!(
            u64::from(rem_before) * u64::from(numer) >= 1 << 32,
            "modifier {modifier}: parted at {parted} without an overflow"
        );
    }
}

proptest! {
    // More levels never need less experience (levels 0..=100 and
    // every stock-reachable modifier; past that the 16-bit expbil could wrap).
    #[test]
    fn exp_needed_is_monotone_in_level(level in 0i16..100, modifier in -20i16..=200) {
        let a = big_exp_total(new_calc_exp_needed(level, modifier));
        let b = big_exp_total(new_calc_exp_needed(level + 1, modifier));
        prop_assert!(a <= b, "level {} modifier {}: {} > {}", level, modifier, a, b);
    }
}
