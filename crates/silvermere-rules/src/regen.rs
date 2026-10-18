//! Regen: hit point and mana regeneration.
//!
//! The formulas of the slow update of a character and of the rest and meditate ticks of
//! the fast update. The engine applies them (`silvermere-engine`'s `cycles`); this module
//! only does the arithmetic, in the game's integer widths: the products are 32-bit, the
//! results 16-bit, and division truncates towards zero.
//!
//! The rest tick reuses the slow formula and triples it before the healing rate is applied;
//! the meditate tick is the slow mana formula unchanged.

use silvermere_types::enums::Magery;

/// `resttick` counts 0..20; the heal comes when it would pass 20,
/// so on every 21st fast tick of resting.
pub const REST_TICKS: u8 = 21;

/// `medtick` counts 0..14; mana comes when it would pass 14, so on
/// every 15th fast tick of meditating.
pub const MEDITATE_TICKS: u8 = 15;

/// A percentage rate bonus as the game applies `healrate` and `manarate`: unchanged at 0,
/// else `(rate+100)*regen/100`.
pub fn apply_rate(regen: i32, rate: i16) -> i32 {
    if rate == 0 {
        regen
    } else {
        (i32::from(rate) + 100) * regen / 100
    }
}

/// The base hit point regeneration: `max((level+20)*hea/750, 1)`, for the slow tick and
/// the rest tick alike.
pub fn hp_regen_base(level: u16, hea: u16) -> i32 {
    let v = (i64::from(level) + 20) * i64::from(hea) / 750;
    (v as i32).max(1)
}

/// Hit points one slow tick gives a standing player below the maximum: the base, scaled
/// by "Alter healing rate".
pub fn hp_regen(level: u16, hea: u16, healrate: i16) -> i32 {
    apply_rate(hp_regen_base(level, hea), healrate)
}

/// Hit points one rest tick gives: the base tripled, then scaled by the healing rate.
pub fn rest_hp_regen(level: u16, hea: u16, healrate: i16) -> i32 {
    apply_rate(hp_regen_base(level, hea) * 3, healrate)
}

/// The stat that drives a class's mana: Mage intellect, Priest willpower, Druid the mean
/// of both (integer
/// division), Bard charm, Kai and anything else 0.
pub fn class_stat(magery: Magery, intel: u16, wil: u16, cha: u16) -> i64 {
    match magery {
        Magery::Mage => i64::from(intel),
        Magery::Priest => i64::from(wil),
        Magery::Druid => (i64::from(wil) + i64::from(intel)) / 2,
        Magery::Bard => i64::from(cha),
        _ => 0,
    }
}

/// Mana (or kai) one slow tick or one meditate tick gives:
/// `castval*(level+20)*(magelvl+2)/1650`, exactly 1 for Kai, then scaled by "Alter Mana
/// Regeneration". The meditate tick does not apply the rate; see [`meditate_mana_regen`].
pub fn mana_regen(magery: Magery, class_stat: i64, level: u16, magelvl: i16, manarate: i16) -> i32 {
    apply_rate(
        meditate_mana_regen(magery, class_stat, level, magelvl),
        manarate,
    )
}

/// Mana one meditate tick gives: the slow formula without the `manarate` scaling.
pub fn meditate_mana_regen(magery: Magery, class_stat: i64, level: u16, magelvl: i16) -> i32 {
    if magery == Magery::Kai {
        return 1;
    }
    (class_stat * (i64::from(level) + 20) * (i64::from(magelvl) + 2) / 1650) as i32
}

/// One fast tick of a 0-based counter that fires when it would reach `period`: the
/// `resttick`/`medtick` pattern: count up, and on reaching `period` start again at 0 and
/// fire.
fn advance(counter: &mut u8, period: u8) -> bool {
    *counter = counter.wrapping_add(1);
    if *counter >= period {
        *counter = 0;
        true
    } else {
        false
    }
}

/// One fast tick of resting; `true` on the tick that heals.
pub fn rest_tick(resttick: &mut u8) -> bool {
    advance(resttick, REST_TICKS)
}

/// One fast tick of meditating; `true` on the tick that restores mana.
pub fn meditate_tick(medtick: &mut u8) -> bool {
    advance(medtick, MEDITATE_TICKS)
}

#[cfg(test)]
mod tests {
    use super::*;

    // max((level+20)*hea/750, 1), then healrate.
    #[test]
    fn hp_regen_table() {
        // (level, hea, healrate) -> slow, rest
        let rows: &[(u16, u16, i16, i32, i32)] = &[
            (1, 40, 0, 1, 3),     // L1 Kang: 21*40/750 = 1
            (1, 70, 0, 1, 3),     // 21*70/750 = 1
            (1, 0, 0, 1, 3),      // at least 1
            (10, 50, 0, 2, 6),    // 30*50/750 = 2
            (30, 90, 0, 6, 18),   // 50*90/750 = 6
            (55, 100, 0, 10, 30), // 75*100/750 = 10
            (10, 50, 50, 3, 9),   // 150*2/100 = 3, 150*6/100 = 9
            (10, 50, -50, 1, 3),  // 50*2/100 = 1
            (1, 40, -100, 0, 0),  // rate -100 cancels it
        ];
        for &(level, hea, rate, slow, rest) in rows {
            assert_eq!(
                hp_regen(level, hea, rate),
                slow,
                "slow {level} {hea} {rate}"
            );
            assert_eq!(
                rest_hp_regen(level, hea, rate),
                rest,
                "rest {level} {hea} {rate}"
            );
        }
    }

    // The class stat, the regeneration, then manarate.
    #[test]
    fn mana_regen_table() {
        // (magery, int, wil, cha, level, magelvl, manarate) -> slow, meditate
        type Row = (Magery, u16, u16, u16, u16, i16, i16, i32, i32);
        let rows: &[Row] = &[
            (Magery::Mage, 60, 40, 40, 1, 3, 0, 3, 3), // 60*21*5/1650 = 3
            (Magery::Mage, 90, 40, 40, 10, 3, 0, 8, 8), // 90*30*5/1650 = 8
            (Magery::Priest, 40, 70, 40, 5, 2, 0, 4, 4), // 70*25*4/1650 = 4
            (Magery::Druid, 50, 61, 40, 5, 2, 0, 3, 3), // 55*25*4/1650 = 3
            (Magery::Bard, 40, 40, 66, 20, 1, 0, 4, 4), // 66*40*3/1650 = 4
            (Magery::Kai, 90, 90, 90, 30, 3, 0, 1, 1),
            (Magery::Kai, 90, 90, 90, 30, 3, 100, 2, 1),
            (Magery::None, 90, 90, 90, 30, 3, 0, 0, 0),
            (Magery::Mage, 90, 40, 40, 10, 3, 50, 12, 8), // 150*8/100 = 12
        ];
        for &(m, int, wil, cha, level, ml, rate, slow, med) in rows {
            let stat = class_stat(m, int, wil, cha);
            assert_eq!(mana_regen(m, stat, level, ml, rate), slow, "{m:?} slow");
            assert_eq!(meditate_mana_regen(m, stat, level, ml), med, "{m:?} med");
        }
    }

    #[test]
    fn tick_counters_fire_on_the_21st_and_15th() {
        let mut t = 0;
        let fired: Vec<u32> = (1..=63).filter(|_| rest_tick(&mut t)).collect();
        assert_eq!(fired, [21, 42, 63]);
        let mut m = 0;
        let fired: Vec<u32> = (1..=45).filter(|_| meditate_tick(&mut m)).collect();
        assert_eq!(fired, [15, 30, 45]);
    }
}
