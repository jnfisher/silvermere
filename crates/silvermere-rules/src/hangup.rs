//! The punishment for dropping the connection while in the realm (when the host reports
//! the hang-up).
//!
//! The engine decides where the player stands and what is dropped
//! (`World::on_disconnect`); this module holds the two decisions that are pure maths:
//! whether `PUNHUP` punishes this hang-up, and how many hit points it costs.

use silvermere_types::config::{GameConfig, PunHup};

use crate::Rng;

/// What the player was doing when the line dropped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HangupSituation {
    /// In auto-combat (`is_inside_autocombat`).
    pub in_autocombat: bool,
    /// Under attack (`is_being_attacked`).
    pub being_attacked: bool,
    /// In player-versus-player combat (`is_in_pvp_combat`).
    pub in_pvp: bool,
}

/// Whether `PUNHUP` punishes this hang-up: `High` always, `Medium` only in auto-combat or
/// under attack, `Low` only in player-versus-player combat, `None` never.
pub fn punishes(pun_hup: PunHup, s: HangupSituation) -> bool {
    match pun_hup {
        PunHup::High => true,
        PunHup::Medium => s.in_autocombat || s.being_attacked,
        PunHup::Low => s.in_pvp,
        PunHup::None => false,
    }
}

/// The punishment of one hang-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HangupPenalty {
    /// Hit points taken (`dmg`), never negative.
    pub hp_loss: i32,
    /// The most items and keys dropped (`MAXHUPIT`).
    pub max_items: u32,
}

/// The hit-point loss of a punished hang-up for a player with hit points left:
/// `lo = maxhp*MINHUPHP/100`, `hi = maxhp*MAXHUPHP/100`, the smaller bound wins when the
/// options are the wrong way round, then `lo + genrdn(0, hi-lo+1)`, uniform in `[lo, hi]`,
/// floored at 0. (A player at 0 hit points or below gets `DEATHHP` added instead; the
/// caller does that.)
pub fn penalty(maxhp: i32, config: &GameConfig, rng: &mut dyn Rng) -> HangupPenalty {
    let lo = maxhp * config.min_hup_hp as i32 / 100;
    let hi = maxhp * config.max_hup_hp as i32 / 100;
    let lo = lo.min(hi);
    let dmg = (rng.range(0, hi - lo + 1) + lo).max(0);
    HangupPenalty {
        hp_loss: dmg,
        max_items: config.max_hup_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SeededRng;

    fn cfg(min: u32, max: u32) -> GameConfig {
        GameConfig {
            min_hup_hp: min,
            max_hup_hp: max,
            max_hup_items: 3,
            ..GameConfig::default()
        }
    }

    #[test]
    fn punhup_levels() {
        let calm = HangupSituation::default();
        let fight = HangupSituation {
            being_attacked: true,
            ..calm
        };
        let pvp = HangupSituation {
            in_pvp: true,
            ..calm
        };
        assert!(punishes(PunHup::High, calm));
        assert!(!punishes(PunHup::Medium, calm));
        assert!(punishes(PunHup::Medium, fight));
        assert!(!punishes(PunHup::Low, fight));
        assert!(punishes(PunHup::Low, pvp));
        assert!(!punishes(PunHup::None, pvp));
    }

    // the smaller bound wins
    #[test]
    fn reversed_options_use_the_upper_bound() {
        let mut rng = SeededRng::new(3);
        for _ in 0..100 {
            assert_eq!(penalty(100, &cfg(50, 10), &mut rng).hp_loss, 10);
        }
    }

    // uniform in [lo, hi] over 10,000 draws
    #[test]
    fn loss_is_uniform_between_the_bounds() {
        let mut rng = SeededRng::new(21);
        let (maxhp, c) = (200, cfg(10, 20));
        let (lo, hi) = (20, 40);
        let mut counts = vec![0u32; (hi - lo + 1) as usize];
        for _ in 0..10_000 {
            let p = penalty(maxhp, &c, &mut rng);
            assert!((lo..=hi).contains(&p.hp_loss), "{}", p.hp_loss);
            assert_eq!(p.max_items, 3);
            counts[(p.hp_loss - lo) as usize] += 1;
        }
        // Chi-square against the flat distribution, 20 degrees of freedom: the 0.999
        // quantile is 45.3.
        let expected = 10_000.0 / counts.len() as f64;
        let chi2: f64 = counts
            .iter()
            .map(|&n| (f64::from(n) - expected).powi(2) / expected)
            .sum();
        assert!(chi2 < 45.3, "chi-square {chi2}");
    }
}
