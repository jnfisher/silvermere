//! Monster target choice: which visible player a monster attacks this round, the inner
//! loops of `do_monster_combat` as a pure function over the candidates (T-036). The engine
//! (`silvermere_engine::monsters::combat_ai`) builds the candidates in the shuffled session
//! order (`shuffle_mappings`), already filtered by `monster_can_see_user`, and makes the
//! attack.
//!
//! The rolls are made in the order the game makes them: one `genrdn(0,100)` per
//! candidate that qualifies, up to the first success. Nothing else rolls between them
//! (the attack comes after the scan), so choosing first and attacking after is exact.

use silvermere_types::enums::{Alignment, MonsterType};

use crate::evil::OUTLAW_POINTS;
use crate::rng::Rng;

/// A player the monster can see, in the shuffled session order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// The player's evil points.
    pub evil: i16,
    /// Monster attacks on the player since the last medium pass.
    pub attackers: i32,
    /// `is_combatting_monster`: the player's auto-combat is against this monster.
    pub fighting_me: bool,
}

/// Lawful Good, Good and Neutral monsters never start a fight.
pub fn starts_fights(alignment: Alignment) -> bool {
    !matches!(
        alignment,
        Alignment::LawfulGood | Alignment::Good | Alignment::Neutral
    )
}

/// The odds of one roll: `genrdn(0,100) < base - attackers*5`.
fn roll(rng: &mut dyn Rng, base: i32, attackers: i32) -> bool {
    rng.range(0, 100) < base - attackers.wrapping_mul(5)
}

/// The target choice of `do_monster_combat` for a monster that neither follows a player
/// nor hunts a monster: the index into `candidates` of the player it attacks, or `None`.
///
/// - A guard ([`MonsterType::Guard`]) attacks the first candidate already fighting it.
///   Otherwise a Lawful Evil guard rolls `< 100 - 5*attackers` on each candidate under 40
///   evil points and leaves the others alone; any other guard rolls the same on each
///   candidate with 40 or more. No fallback.
/// - Anything else that [`starts_fights`]: each candidate is remembered as
///   `lastseen` and rolled `< 50 - 5*attackers`, except that a Lawful Evil monster skips
///   (without a roll) a candidate with 40 or more evil points who is not fighting it. When
///   no roll succeeds, the last candidate remembered is attacked.
pub fn choose(
    montype: MonsterType,
    alignment: Alignment,
    candidates: &[Candidate],
    rng: &mut dyn Rng,
) -> Option<usize> {
    if montype == MonsterType::Guard {
        for (i, c) in candidates.iter().enumerate() {
            let ok = if c.fighting_me {
                true
            } else if alignment == Alignment::LawfulEvil {
                // a candidate not fighting it is attacked only on the roll
                c.evil < OUTLAW_POINTS && roll(rng, 100, c.attackers)
            } else {
                c.evil >= OUTLAW_POINTS && roll(rng, 100, c.attackers)
            };
            if ok {
                return Some(i);
            }
        }
        return None;
    }
    if !starts_fights(alignment) {
        return None;
    }
    let mut lastseen = None;
    for (i, c) in candidates.iter().enumerate() {
        if alignment == Alignment::LawfulEvil && c.evil >= OUTLAW_POINTS && !c.fighting_me {
            continue;
        }
        lastseen = Some(i);
        if roll(rng, 50, c.attackers) {
            return Some(i);
        }
    }
    lastseen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SeededRng;

    fn c(evil: i16, attackers: i32, fighting_me: bool) -> Candidate {
        Candidate {
            evil,
            attackers,
            fighting_me,
        }
    }

    /// A source that answers a fixed list of rolls and counts them.
    struct Fixed(Vec<i32>, usize);

    impl Rng for Fixed {
        fn range(&mut self, _lo: i32, _hi: i32) -> i32 {
            let r = self.0[self.1];
            self.1 += 1;
            r
        }
        fn long(&mut self, lo: i64, _hi: i64) -> i64 {
            lo
        }
    }

    #[test]
    fn good_lawful_good_and_neutral_never_start() {
        let cands = [c(0, 0, false), c(100, 0, true)];
        for a in [Alignment::LawfulGood, Alignment::Good, Alignment::Neutral] {
            let mut r = Fixed(vec![], 0);
            assert_eq!(choose(MonsterType::Lair, a, &cands, &mut r), None);
            assert_eq!(r.1, 0, "no roll for {a:?}");
        }
    }

    #[test]
    fn ordinary_rolls_fifty_minus_five_per_attacker() {
        // 2 attackers: < 40 attacks
        let cands = [c(0, 2, false)];
        let mut r = Fixed(vec![39], 0);
        assert_eq!(
            choose(MonsterType::Wanderer, Alignment::Evil, &cands, &mut r),
            Some(0)
        );
        // a failed roll on the only candidate: the lastseen fallback still attacks
        let mut r = Fixed(vec![40], 0);
        assert_eq!(
            choose(MonsterType::Wanderer, Alignment::Evil, &cands, &mut r),
            Some(0)
        );
    }

    #[test]
    fn lastseen_is_the_last_candidate() {
        let cands = [c(0, 0, false), c(0, 0, false), c(0, 0, false)];
        let mut r = Fixed(vec![99, 99, 99], 0);
        assert_eq!(
            choose(MonsterType::Lair, Alignment::ChaoticEvil, &cands, &mut r),
            Some(2)
        );
        assert_eq!(r.1, 3);
        // the first success ends the scan
        let mut r = Fixed(vec![99, 0], 0);
        assert_eq!(
            choose(MonsterType::Lair, Alignment::ChaoticEvil, &cands, &mut r),
            Some(1)
        );
        assert_eq!(r.1, 2);
    }

    #[test]
    fn lawful_evil_leaves_outlaws_alone_until_fought() {
        let outlaw = [c(40, 0, false)];
        let mut r = Fixed(vec![], 0);
        assert_eq!(
            choose(MonsterType::Lair, Alignment::LawfulEvil, &outlaw, &mut r),
            None
        );
        assert_eq!(r.1, 0);
        let fighting = [c(40, 0, true)];
        let mut r = Fixed(vec![0], 0);
        assert_eq!(
            choose(MonsterType::Lair, Alignment::LawfulEvil, &fighting, &mut r),
            Some(0)
        );
        // 39 points is no outlaw
        let mut r = Fixed(vec![0], 0);
        assert_eq!(
            choose(
                MonsterType::Lair,
                Alignment::LawfulEvil,
                &[c(39, 0, false)],
                &mut r
            ),
            Some(0)
        );
    }

    #[test]
    fn guards() {
        // already fighting it: no roll
        let mut r = Fixed(vec![], 0);
        assert_eq!(
            choose(
                MonsterType::Guard,
                Alignment::Good,
                &[c(0, 9, true)],
                &mut r
            ),
            Some(0)
        );
        // an ordinary guard: the non-outlaw is never rolled, the outlaw is
        let mut r = Fixed(vec![89], 0);
        assert_eq!(
            choose(
                MonsterType::Guard,
                Alignment::Good,
                &[c(39, 0, false), c(40, 2, false)],
                &mut r
            ),
            Some(1)
        );
        let mut r = Fixed(vec![90], 0);
        assert_eq!(
            choose(
                MonsterType::Guard,
                Alignment::Good,
                &[c(40, 2, false)],
                &mut r
            ),
            None,
            "no fallback for guards"
        );
        // a Lawful Evil guard: the reverse
        let mut r = Fixed(vec![0], 0);
        assert_eq!(
            choose(
                MonsterType::Guard,
                Alignment::LawfulEvil,
                &[c(40, 0, false), c(0, 0, false)],
                &mut r
            ),
            Some(1)
        );
        assert_eq!(r.1, 1);
    }

    /// Layer 1: over many trials a lone candidate with `n` attackers is picked on the first
    /// roll `50-5n`% of the time; with three, the first-roll picks follow the order.
    #[test]
    fn distribution_of_three_candidates() {
        let mut rng = SeededRng::new(36);
        let cands = [c(0, 0, false), c(0, 1, false), c(0, 2, false)];
        let trials = 100_000u64;
        let mut seen = [0u64; 3];
        for _ in 0..trials {
            let i = choose(MonsterType::Lair, Alignment::Evil, &cands, &mut rng).unwrap();
            seen[i] += 1;
        }
        // P(0)=.5, P(1)=.5*.45, P(2)=.5*.55 (its own .40 plus the fallback .60)
        let p = [0.5, 0.5 * 0.45, 0.5 * 0.55];
        let chi: f64 = (0..3)
            .map(|i| {
                let e = p[i] * trials as f64;
                (seen[i] as f64 - e).powi(2) / e
            })
            .sum();
        // 9.21: the 0.01 point of chi-square with 2 degrees of freedom
        assert!(chi < 9.21, "chi {chi} {seen:?}");
    }
}
