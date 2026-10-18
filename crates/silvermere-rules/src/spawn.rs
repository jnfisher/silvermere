//! Spawn: the spawn pool and pick, the per-tick spawn chance, the regeneration delay, the
//! unique-monster wait and the random directions of `generate_monster` and the wanderers.
//!
//! Pure functions over plain values; `silvermere-engine`'s `monsters` module walks the
//! world and calls these (T-035). Every roll is the host's `genrdn(lo, hi)`, which
//! [`Rng::range`] reproduces (`[lo, hi)`, never `hi`).

use silvermere_types::ids::MonsterNo;

use crate::rng::Rng;

/// The spawn pass stops once it has made more than this many, so up to nine monsters (one
/// more than the constant) are made per pass.
pub const MAXGENERATE: u32 = 8;
/// Entries of the occupied-room table.
pub const NOCCROOMS: usize = 512;
/// Wanderers per medium pass.
pub const MAXMOVERS: u32 = 3;
/// Lost rounds before a follower gives up.
pub const GIVEUPRNDS: u8 = 15;
/// The chance a later usable exit replaces the choice.
pub const RANDEXIT_PCT: i32 = 40;
/// Minutes in a day.
pub const MINSPERDAY: u32 = 1440;

/// One entry of the known-monster quick reference: what the pool pick reads without
/// loading the whole record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickRef {
    /// The known monster.
    pub number: MonsterNo,
    /// Its `montype` code (the room's `mongroup` it spawns in).
    pub montype: i16,
    /// Its generation index.
    pub genindex: i16,
}

/// The monsters `generate_monster` may pick for a random spawn:
/// every quick-reference entry of type `areatype` (-1 for any) with a `genindex` of at
/// least `minlevel` (0: no lower limit) and at most `maxlevel` (0: no upper limit), in
/// quick-reference order.
pub fn pool(quickref: &[QuickRef], areatype: i16, minlevel: i16, maxlevel: i16) -> Vec<MonsterNo> {
    quickref
        .iter()
        .filter(|q| {
            (areatype == -1 || q.montype == areatype)
                && (minlevel == 0 || q.genindex >= minlevel)
                && (maxlevel == 0 || q.genindex <= maxlevel)
        })
        .map(|q| q.number)
        .collect()
}

/// The pick among the pool: the first entry is taken; for each later
/// one `genrdn(1,100)` rolls 1-99, a roll below 30 makes it the choice and a roll of 99
/// stops the search. `None` for an empty pool.
pub fn pick(pool: &[MonsterNo], rng: &mut dyn Rng) -> Option<MonsterNo> {
    let (first, rest) = pool.split_first()?;
    let mut choice = *first;
    for m in rest {
        let j = rng.range(1, 100);
        if j < 30 {
            choice = *m;
        } else if j >= 99 {
            break;
        }
    }
    Some(choice)
}

/// The spawn chance of a room the players outnumber the monsters in: Arena 90, Normal 5,
/// anything else 25 (never reached: only those two types get there). Compared as
/// `chance > genrdn(1,100)`.
pub fn spawn_chance(room_type: silvermere_types::enums::RoomType) -> i32 {
    use silvermere_types::enums::RoomType;
    match room_type {
        RoomType::Arena => 90,
        RoomType::Normal => 5,
        _ => 25,
    }
}

/// The pass-1 spawn test for a roll of `genrdn(1,100)`: room for a
/// monster, and the players outnumber the monsters with the roll below the room's chance.
/// The second alternative, a roll above 99 while the monsters number fewer than twice the
/// players, is kept although `genrdn(1,100)` never gives 100 (sic).
pub fn pass1_spawns(nmons: u32, nplayers: u32, chance: i32, roll: i32) -> bool {
    nmons < silvermere_types::sizes::NROOMMONS as u32
        && ((nmons < nplayers && chance > roll) || (roll > 99 && nplayers * 2 > nmons))
}

/// The regeneration delay of `generate_monster`: is a spawn refused
/// because the room's last kill, at minute stamp `lastkill`, lies within `regendelay`
/// minutes (the room's own, or `regenmins` when that is 0) before `now`? The stamps are
/// the game's 16-bit minute counts (minute of the day, plus a day once midnight has passed
/// since start-up); the subtraction wraps as unsigned 16-bit, so early in the start-up day
/// a delay longer than the minutes so far never blocks. A `lastkill` of 0 never blocks (the
/// caller's test).
pub fn regen_blocked(lastkill: u16, now: u16, regendelay: i16, regenmins: u16) -> bool {
    if lastkill == 0 || lastkill > now {
        return false;
    }
    let delay = if regendelay != 0 {
        regendelay as u16
    } else {
        regenmins
    };
    lastkill >= now.wrapping_sub(delay)
}

/// The game's minute stamp of `now_secs`: the minute of the UTC day, plus a day when it
/// lies before the minute of the day the game started at (midnight has passed since
/// start-up).
pub fn minute_stamp(now_secs: i64, boot_secs: i64) -> u16 {
    let day = i64::from(MINSPERDAY);
    let mins = (now_secs.div_euclid(60)).rem_euclid(day) as u32;
    let bootmin = (boot_secs.div_euclid(60)).rem_euclid(day) as u32;
    let m = if bootmin > mins {
        mins + MINSPERDAY
    } else {
        mins
    };
    m as u16
}

/// The wait of a killed unique monster: `regenhrs` hours, plus
/// `lngrnd(0, regen/4) - regen/8` minutes.
pub fn unique_wait_minutes(regenhrs: u8, rng: &mut dyn Rng) -> i64 {
    let regen = i64::from(regenhrs) * 60;
    regen + rng.long(0, regen >> 2) - (regen >> 3)
}

/// Is a monster with a `maxalive` of 1 ready to come back? Never killed
/// (`killed_at` 0, Unix seconds) or no `regenhrs`: yes, with no roll. Otherwise the
/// minutes since the kill (0 if the kill lies in the future) against
/// [`unique_wait_minutes`].
pub fn unique_ready(killed_at: i64, now_secs: i64, regenhrs: u8, rng: &mut dyn Rng) -> bool {
    if killed_at == 0 || regenhrs == 0 {
        return true;
    }
    let since = if killed_at > now_secs {
        0
    } else {
        (now_secs - killed_at) / 60
    };
    since >= unique_wait_minutes(regenhrs, rng)
}

/// The direction a new monster seems to arrive from: of the eight
/// compass exits (`usable[k]`: an exit of type Normal), the first is taken and each later
/// one replaces it when `genrdn(1,10)` rolls 6-9. `None`: from nowhere.
pub fn arrival_direction(usable: [bool; 8], rng: &mut dyn Rng) -> Option<usize> {
    let mut dir = None;
    for (k, ok) in usable.into_iter().enumerate() {
        if ok && (dir.is_none() || rng.range(1, 10) > 5) {
            dir = Some(k);
        }
    }
    dir
}

/// `pick_valid_random_direction` over the ten exits (`usable[i]`:
/// an exit of a walkable kind): the first is taken, each later one replaces it with
/// [`RANDEXIT_PCT`] percent (`genrdn(0,100)`). `None`: no usable exit.
pub fn random_direction(
    usable: [bool; silvermere_types::sizes::NDIRS],
    rng: &mut dyn Rng,
) -> Option<usize> {
    let mut dir = None;
    for (i, ok) in usable.into_iter().enumerate() {
        if !ok {
            continue;
        }
        if dir.is_none() || rng.range(0, 100) < RANDEXIT_PCT {
            dir = Some(i);
        }
    }
    dir
}

/// The wander chance of a non-guard monster: `(100-followpct)/2`
/// percent, compared with `genrdn(0,100)`.
pub fn wander_chance(followpct: i16) -> i32 {
    (100 - i32::from(followpct)) / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{SeededRng, chi_square_critical_001, chi_square_uniform};

    fn q(n: i32, t: i16, g: i16) -> QuickRef {
        QuickRef {
            number: MonsterNo(n),
            montype: t,
            genindex: g,
        }
    }

    #[test]
    fn pool_filters_by_type_and_genindex() {
        let qr = [q(1, 6, 1), q(2, 6, 4), q(3, 7, 1), q(4, 6, 3), q(5, 6, 0)];
        assert_eq!(pool(&qr, 6, 1, 3), vec![MonsterNo(1), MonsterNo(4)]);
        // minlevel 0: no lower limit; maxlevel 0: no upper limit
        assert_eq!(
            pool(&qr, 6, 0, 3),
            vec![MonsterNo(1), MonsterNo(4), MonsterNo(5)]
        );
        assert_eq!(pool(&qr, 6, 2, 0), vec![MonsterNo(2), MonsterNo(4)]);
        assert_eq!(pool(&qr, -1, 1, 1), vec![MonsterNo(1), MonsterNo(3)]);
    }

    #[test]
    fn pick_takes_the_only_entry_and_none_of_nothing() {
        let mut rng = SeededRng::new(1);
        assert_eq!(pick(&[], &mut rng), None);
        assert_eq!(pick(&[MonsterNo(9)], &mut rng), Some(MonsterNo(9)));
    }

    /// With two entries the second wins on 1..29 of 1..99, 29/99.
    #[test]
    fn pick_of_two_matches_the_odds() {
        let mut rng = SeededRng::new(7);
        let pool = [MonsterNo(1), MonsterNo(2)];
        let n = 99_000;
        let second = (0..n)
            .filter(|_| pick(&pool, &mut rng) == Some(MonsterNo(2)))
            .count();
        let p = 29.0 / 99.0;
        let expect = p * f64::from(n);
        let sd = (f64::from(n) * p * (1.0 - p)).sqrt();
        assert!(
            (second as f64 - expect).abs() < 4.0 * sd,
            "{second} vs {expect}"
        );
    }

    #[test]
    fn spawn_test_and_chances() {
        use silvermere_types::enums::RoomType;
        assert_eq!(spawn_chance(RoomType::Arena), 90);
        assert_eq!(spawn_chance(RoomType::Normal), 5);
        assert_eq!(spawn_chance(RoomType::Lair), 25);
        assert!(pass1_spawns(0, 1, 90, 89));
        assert!(!pass1_spawns(0, 1, 90, 90));
        assert!(!pass1_spawns(1, 1, 90, 1));
        assert!(!pass1_spawns(15, 20, 90, 1));
        // the dead second chance: only a roll of 100 would reach it
        assert!(pass1_spawns(1, 1, 90, 100));
    }

    #[test]
    fn regen_delay_blocks_for_the_delay_after_a_kill() {
        // killed at minute 600, default 5 minutes
        assert!(regen_blocked(600, 600, 0, 5));
        assert!(regen_blocked(600, 605, 0, 5));
        assert!(!regen_blocked(600, 606, 0, 5));
        // the room's own delay wins
        assert!(regen_blocked(600, 615, 20, 5));
        assert!(!regen_blocked(600, 621, 20, 5));
        // never killed, or a stamp in the future
        assert!(!regen_blocked(0, 3, 0, 5));
        assert!(!regen_blocked(700, 600, 0, 5));
        // unsigned wrap: minute 3 minus 5 is 65534
        assert!(!regen_blocked(2, 3, 0, 5));
    }

    #[test]
    fn minute_stamp_adds_a_day_past_midnight() {
        let boot = 22 * 3600; // 22:00
        assert_eq!(minute_stamp(boot + 60, boot), 22 * 60 + 1);
        // 01:00 the next day
        assert_eq!(minute_stamp(86_400 + 3600, boot), 60 + 1440);
    }

    /// 4 hours give 240 + [0,60) - 30 minutes, 210..270.
    #[test]
    fn unique_monster_waits_regenhrs_with_variation() {
        let mut rng = SeededRng::new(3);
        for _ in 0..1000 {
            let w = unique_wait_minutes(4, &mut rng);
            assert!((210..270).contains(&w), "{w}");
        }
        let killed = 1_000_000;
        assert!(unique_ready(0, killed, 4, &mut rng));
        assert!(unique_ready(killed, killed, 0, &mut rng));
        assert!(!unique_ready(killed, killed + 209 * 60, 4, &mut rng));
        assert!(unique_ready(killed, killed + 270 * 60, 4, &mut rng));
        assert!(!unique_ready(killed + 60, killed, 4, &mut rng));
    }

    /// Two usable exits, the second wins 4 times in 9.
    #[test]
    fn arrival_direction_odds() {
        let mut rng = SeededRng::new(11);
        assert_eq!(arrival_direction([false; 8], &mut rng), None);
        let mut usable = [false; 8];
        usable[2] = true;
        assert_eq!(arrival_direction(usable, &mut rng), Some(2));
        usable[5] = true;
        let n = 90_000;
        let late = (0..n)
            .filter(|_| arrival_direction(usable, &mut rng) == Some(5))
            .count();
        let expect = f64::from(n) * 4.0 / 9.0;
        assert!((late as f64 - expect).abs() < 600.0, "{late}");
    }

    /// Three usable exits are taken 36, 24 and 40 percent.
    #[test]
    fn random_direction_is_not_even() {
        let mut rng = SeededRng::new(5);
        let mut usable = [false; 10];
        usable[0] = true;
        usable[3] = true;
        usable[9] = true;
        let mut counts = [0u64; 3];
        let n = 100_000;
        for _ in 0..n {
            match random_direction(usable, &mut rng) {
                Some(0) => counts[0] += 1,
                Some(3) => counts[1] += 1,
                Some(9) => counts[2] += 1,
                other => panic!("{other:?}"),
            }
        }
        // scale to an even target: 36/24/40 -> each count over its share
        let scaled = [
            counts[0] * 100 / 36,
            counts[1] * 100 / 24,
            counts[2] * 100 / 40,
        ];
        assert!(
            chi_square_uniform(&scaled) < chi_square_critical_001(2) * 3.0,
            "{counts:?}"
        );
        assert_eq!(random_direction([false; 10], &mut rng), None);
    }

    #[test]
    fn wander_chance_is_half_the_rest_of_followpct() {
        assert_eq!(wander_chance(0), 50);
        assert_eq!(wander_chance(50), 25);
        assert_eq!(wander_chance(100), 0);
    }
}
