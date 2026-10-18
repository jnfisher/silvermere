//! Layer 1 distribution tests for `combat`: a seeded `Rng`,
//! 100,000 resolutions each, histograms against the formulas with a chi-square tolerance.
//! The host's random sequence is never reproduced, only the distributions. Each blow
//! resolves a fresh copy of the attacker, as the builders refill the fighter before every
//! blow in the game.

use proptest::prelude::*;
use silvermere_rules::SeededRng;
use silvermere_rules::combat::{
    Fighter, MAX_BLOWS, Move, Outcome, dodge_chance, effective_crit, energy_round, hit_chance,
    resolve,
};
use silvermere_rules::rng::{Rng, chi_square_critical_001, chi_square_uniform};

const N: u32 = 100_000;

fn fighter(av: i32, dv: i32, min: i32, max: i32) -> Fighter {
    Fighter {
        av,
        dv,
        mindmg: min,
        maxdmg: max,
        ..Fighter::default()
    }
}

/// Five standard deviations of a binomial count: deterministic seeds make this a bound,
/// not a flake budget.
fn tolerance(p: f64) -> f64 {
    5.0 * (f64::from(N) * p * (1.0 - p)).sqrt()
}

fn assert_rate(observed: u32, p: f64, what: &str) {
    let expected = f64::from(N) * p;
    let diff = (f64::from(observed) - expected).abs();
    assert!(
        diff <= tolerance(p),
        "{what}: observed {observed}, expected {expected:.0} +/- {:.0}",
        tolerance(p)
    );
}

// resolve: the roll is 1..=99 and hits under the chance, so
// the hit rate is (hitchance-1)/99. Kang (AV 58) vs DV 30: 63 -> 62/99.
#[test]
fn hit_rate_matches_hitchance() {
    for (av, dv) in [(58, 30), (58, 40), (100, 10), (20, 50)] {
        let atk = fighter(av, 0, 1, 1);
        let def = fighter(0, dv, 0, 0);
        let chance = hit_chance(av, 0, dv, 0, false);
        let mut rng = SeededRng::new(0x5EED + u64::try_from(av * 100 + dv).unwrap());
        let hits = (0..N)
            .filter(|_| {
                resolve(&mut atk.clone(), &def, Move::Attack, &mut rng).outcome != Outcome::Miss
            })
            .count();
        let p = f64::from(chance - 1) / 99.0;
        assert_rate(
            u32::try_from(hits).unwrap(),
            p,
            &format!("hit rate av {av} dv {dv}"),
        );
    }
}

// resolve: damage is uniform on [min-dr/10, max-dr/10].
#[test]
fn damage_histogram_is_uniform() {
    let atk = fighter(300, 0, 10, 24);
    let mut def = fighter(0, 1, 0, 0);
    def.dr = 35; // 3 off each blow: 7..=21
    let lo = 10 - 3;
    let hi = 24 - 3;
    let mut counts = vec![0u64; usize::try_from(hi - lo + 1).unwrap()];
    let mut rng = SeededRng::new(7);
    for _ in 0..N {
        match resolve(&mut atk.clone(), &def, Move::Attack, &mut rng).outcome {
            Outcome::Hit { damage, critical } => {
                assert!(!critical);
                assert!((lo..=hi).contains(&damage), "damage {damage}");
                counts[usize::try_from(damage - lo).unwrap()] += 1;
            }
            Outcome::Miss => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(counts.iter().all(|&c| c > 0));
    let chi = chi_square_uniform(&counts);
    let crit = chi_square_critical_001(u32::try_from(counts.len() - 1).unwrap());
    assert!(chi < crit, "chi-square {chi:.1} over {crit:.1}: {counts:?}");
}

// resolve: a hit criticals with probability
// effective_crit/100, and never on BASH or SMASH.
#[test]
fn crit_frequency() {
    for crit in [2, 22, 40, 75] {
        let mut atk = fighter(300, 0, 10, 20);
        atk.crit = crit;
        let def = fighter(0, 1, 0, 0);
        let mut rng = SeededRng::new(u64::try_from(crit).unwrap());
        let (mut hits, mut crits) = (0u32, 0u32);
        for _ in 0..N {
            if let Outcome::Hit { critical, damage } =
                resolve(&mut atk.clone(), &def, Move::Attack, &mut rng).outcome
            {
                hits += 1;
                if critical {
                    crits += 1;
                    assert!((40..=80).contains(&damage));
                }
            }
        }
        let p = f64::from(effective_crit(crit)) / 100.0;
        let expected = f64::from(hits) * p;
        let tol = 5.0 * (f64::from(hits) * p * (1.0 - p)).sqrt();
        assert!(
            (f64::from(crits) - expected).abs() <= tol,
            "crit {crit}: {crits} of {hits}, expected {expected:.0} +/- {tol:.0}"
        );
    }
    let mut atk = fighter(300, 0, 10, 20);
    atk.crit = 75;
    let def = fighter(0, 1, 0, 0);
    let mut rng = SeededRng::new(99);
    for mv in [Move::Bash, Move::Smash] {
        for _ in 0..10_000 {
            if let Outcome::Hit { critical, .. } =
                resolve(&mut atk.clone(), &def, mv, &mut rng).outcome
            {
                assert!(!critical, "{mv:?} never criticals");
            }
        }
    }
}

// resolve: a landed blow is dodged with probability
// dodge*10/(av/8) percent (cap 95).
#[test]
fn dodge_frequency() {
    for (dodge, av) in [(22, 58), (5, 100), (60, 64), (80, 200)] {
        let atk = fighter(av, 0, 10, 20);
        let mut def = fighter(0, 0, 0, 0);
        def.dodge = dodge;
        let mut rng = SeededRng::new(u64::try_from(dodge * 1000 + av).unwrap());
        let (mut landed, mut dodged) = (0u32, 0u32);
        for _ in 0..N {
            match resolve(&mut atk.clone(), &def, Move::Attack, &mut rng).outcome {
                Outcome::Miss => {}
                Outcome::Dodged => {
                    landed += 1;
                    dodged += 1;
                }
                _ => landed += 1,
            }
        }
        let p = f64::from(dodge_chance(dodge, av, false)) / 100.0;
        let expected = f64::from(landed) * p;
        let tol = 5.0 * (f64::from(landed) * p * (1.0 - p)).sqrt();
        assert!(
            (f64::from(dodged) - expected).abs() <= tol,
            "dodge {dodge} av {av}: {dodged} of {landed}, expected {expected:.0} +/- {tol:.0}"
        );
    }
}

// energy_round: a fresh round makes min(6, floor(1000/cost))
// blows; with the leftover carried, 605 gives one blow then two, averaging 1000/605.
#[test]
fn blows_per_round() {
    for cost in [150, 166, 167, 200, 250, 333, 334, 500, 605, 1000] {
        let fresh = energy_round(0, cost).blows;
        assert_eq!(
            fresh,
            (1000 / u32::try_from(cost).unwrap()).min(MAX_BLOWS),
            "cost {cost}"
        );
    }
    let (mut carry, mut total) = (0, 0u64);
    let mut seq = Vec::new();
    for _ in 0..N {
        let r = energy_round(carry, 605);
        carry = r.leftover;
        total += u64::from(r.blows);
        if seq.len() < 6 {
            seq.push(r.blows);
        }
    }
    assert_eq!(seq, [1, 2, 1, 2, 2, 1]);
    let mean = total as f64 / f64::from(N);
    assert!((mean - 1000.0 / 605.0).abs() < 0.001, "mean blows {mean}");
}

/// An `Rng` that replays one fixed list of rolls, so two moves can be resolved on the same
/// rolls.
struct Replay(Vec<i32>, usize);

impl Rng for Replay {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let v = self.0[self.1 % self.0.len()];
        self.1 += 1;
        if hi <= lo {
            lo
        } else {
            lo + v.rem_euclid(hi - lo)
        }
    }

    fn long(&mut self, lo: i64, _hi: i64) -> i64 {
        lo
    }
}

proptest! {
    // resolve: the hit chance is always in 10..=99.
    #[test]
    fn hitchance_always_10_to_99(
        av in -500i32..2000, hitmod in -75i32..0, dv in -100i32..2000,
        defalign in -50i32..50, backstab: bool,
    ) {
        let h = hit_chance(av, hitmod, dv, defalign, backstab);
        prop_assert!((10..=99).contains(&h));
    }

    // resolve: a Hit always carries positive damage, which is
    // also `rolled`, with the defender's expval; nothing or less is NoDamage.
    #[test]
    fn hit_damage_is_never_negative(
        av in 0i32..400, dv in 0i32..400, min in -5i32..60, max in -5i32..120,
        dr in 0i32..600, crit in 0i32..100, dodge in -50i32..100, seed: u64,
        mv in prop_oneof![
            Just(Move::Attack), Just(Move::Punch), Just(Move::Kick), Just(Move::JumpKick), Just(Move::Bash),
            Just(Move::Smash), Just(Move::Unused8), Just(Move::Backstab)
        ],
    ) {
        let mut atk = fighter(av, 0, min, max);
        atk.crit = crit;
        let mut def = fighter(0, dv, 0, 0);
        def.dr = dr;
        def.dodge = dodge;
        let mut rng = SeededRng::new(seed);
        for _ in 0..20 {
            let r = resolve(&mut atk.clone(), &def, mv, &mut rng);
            prop_assert_eq!(r.poison, 0);
            match r.outcome {
                Outcome::Hit { damage, .. } => {
                    prop_assert!(damage > 0);
                    prop_assert_eq!(r.rolled, damage);
                    prop_assert_eq!(r.exp, def.expval);
                }
                _ => {
                    prop_assert_eq!(r.rolled, 0);
                    prop_assert_eq!(r.exp, 0);
                }
            }
        }
    }

    // resolve: on the same rolls, a BASH hit is exactly three
    // times the +10% blow after damage reduction, and a SMASH five times the +20% one.
    #[test]
    fn bash_is_three_times_after_dr(
        av in 20i32..400, min in 1i32..40, max in 40i32..120, dr in 0i32..300,
        rolls in prop::collection::vec(0i32..1000, 4),
    ) {
        let atk = fighter(av, 0, min, max);
        let mut def = fighter(0, 0, 0, 0);
        def.dr = dr;
        let bash = resolve(&mut atk.clone(), &def, Move::Bash, &mut Replay(rolls.clone(), 0));
        let smash = resolve(&mut atk.clone(), &def, Move::Smash, &mut Replay(rolls.clone(), 0));
        // Each move is compared against its own pre-multiplier value on the same rolls.
        let roll = rolls[2];
        let pre = |pct: i32| {
            let lo = min * (pct + 100) / 100;
            let hi = max * (pct + 100) / 100;
            roll.rem_euclid(hi - lo + 1) + lo - dr / 10
        };
        if let Outcome::Hit { damage, critical } = bash.outcome {
            prop_assert!(!critical);
            prop_assert_eq!(damage, pre(10) * 3);
        }
        if let Outcome::Hit { damage, critical } = smash.outcome {
            prop_assert!(!critical);
            prop_assert_eq!(damage, pre(20) * 5);
        }
    }
}
