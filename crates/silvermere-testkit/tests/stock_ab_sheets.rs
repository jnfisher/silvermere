//! Rules A/B on the four fixed sheets (T-040): for each sheet pairing and each attack move the sheet describes, 200,000
//! resolutions on a seeded `Rng`, compared with the closed-form distribution of one attack
//! by chi-square and z-tests at p > 0.01: hit rate, the damage histogram, the critical rate,
//! the dodge rate, and blows per round.
//!
//! The four sheets are Kang (a level 1 Witchunter with the stock quarterstaff) and three
//! stock monsters (giant rat, kobold thief, acid slime), built by
//! `silvermere_rules::combat`'s fighter builders from the records of the downloaded
//! distribution.
//!
//! The closed form is enumerated here from the combat rules, written out independently of
//! `silvermere_rules::combat`, so the test checks that code rather than restating it. The
//! same numbers (expected and observed, sample sizes, p-values) are written as JSON for
//! `tools/differential/stats.py`, which compares the servers' swings with them: to
//! `$SILVERMERE_AB_EXPORT` when set, else `ab_sheets.json` in Cargo's per-test temporary
//! directory.

use crate::stock_records;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};
use silvermere_engine::player_session::clear_player;
use silvermere_engine::world::blank::clear_active_monster;
use silvermere_import::decode::{LegacyClock, split_monster, to_class, to_item};
use silvermere_rules::SeededRng;
use silvermere_rules::combat::{
    AlignGroup, Blow, Fighter, MonsterAbilities, Move, Outcome, PlayerAbilities, energy_round,
    monster_fighter, player_fighter, resolve,
};
use silvermere_testkit::stats::{Test, binomial_z, chi_square_gof};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::ActiveMonsterId;

const N: u32 = 200_000;
const ROUNDS: u32 = 20_000;
const ALPHA: f64 = 0.01;
/// Kang's maximum energy, the cost of a smash.
const KANG_MAXENERGY: u16 = 1000;

/// One blow's outcome as a histogram bin, ordered for a stable export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Bin {
    Miss,
    Dodged,
    NoDamage,
    Hit { damage: i32, critical: bool },
}

impl Bin {
    fn of(o: Outcome) -> Self {
        match o {
            Outcome::Miss => Bin::Miss,
            Outcome::Dodged => Bin::Dodged,
            Outcome::NoDamage => Bin::NoDamage,
            Outcome::Hit { damage, critical } => Bin::Hit { damage, critical },
        }
    }

    fn label(self) -> String {
        match self {
            Bin::Miss => "miss".into(),
            Bin::Dodged => "dodged".into(),
            Bin::NoDamage => "nodamage".into(),
            Bin::Hit { damage, critical } => {
                format!("{}{damage}", if critical { "crit:" } else { "hit:" })
            }
        }
    }
}

/// Kasimir, a level 1 Kang Witchunter, wielding the
/// stock quarterstaff (item 100), as the Witchunter class (class 2) of the download builds
/// him for `mv` against an evil monster in normal light.
fn kang(mv: Move) -> Fighter {
    let c = stock_records::content();
    let class = c
        .classes
        .iter()
        .map(|d| to_class(&d.record))
        .find(|k| k.number.0 == 2)
        .expect("class 2");
    let staff = c
        .items
        .iter()
        .map(|d| to_item(&d.record))
        .find(|i| i.number.0 == 100)
        .expect("item 100");
    let mut p = clear_player();
    p.level = 1;
    (p.str, p.intel, p.wil, p.agi, p.hea, p.cha) = (79, 30, 45, 42, 74, 30);
    p.hp = 20;
    p.maxenergy = KANG_MAXENERGY;
    p.encumpct = 18;
    p.rank = 0;
    p.mabase = 1;
    p.settings = PlayerSettings::CANCAST;
    player_fighter(
        &p,
        &class,
        Some(&staff),
        &[],
        PlayerAbilities::default(),
        Blow::new(mv, AlignGroup::Evil),
    )
}

/// Stock monster `number` as an active monster of its kind, one fighter per attack slot
/// that does damage (its maximum above 0): `(slot, fighter)`.
fn monster(number: i32) -> Vec<(usize, Fighter)> {
    let record = &stock_records::content()
        .monsters
        .iter()
        .find(|d| d.record.number == number)
        .expect("the monster")
        .record;
    let (known, _) = split_monster(record, &LegacyClock::utc_now()).expect("decodes");
    let mut active = clear_active_monster(ActiveMonsterId(1));
    active.knownnum = known.number;
    active.level = known.level;
    active.ac = known.ac;
    active.dr = known.dr;
    active.exp = known.exp;
    (0..known.attacks.len())
        .filter(|&slot| known.attacks[slot].max > 0)
        .map(|slot| {
            let f = monster_fighter(
                &active,
                &known,
                slot,
                None,
                MonsterAbilities::default(),
                Blow::new(Move::Attack, AlignGroup::Neutral),
            );
            (slot, f)
        })
        .collect()
}

/// The numbers a fighter is compared by: `(av, dv, defalign, dr, expval, energy, mindmg,
/// maxdmg, dodge, crit, defmod)`.
type Sheet = (i32, i32, i32, i32, u32, u32, i32, i32, i32, i32, i32);

fn numbers(f: &Fighter) -> Sheet {
    (
        f.av, f.dv, f.defalign, f.dr, f.expval, f.energy, f.mindmg, f.maxdmg, f.dodge, f.crit,
        f.defmod,
    )
}

/// The fighter of `numbers`, as the tests below resolve it.
fn fighter(n: Sheet) -> Fighter {
    let (av, dv, defalign, dr, expval, energy, mindmg, maxdmg, dodge, crit, defmod) = n;
    Fighter {
        av,
        dv,
        defalign,
        dr,
        expval,
        energy,
        mindmg,
        maxdmg,
        dodge,
        crit,
        defmod,
        ..Fighter::default()
    }
}

/// The sheets' numbers, pinned: Kang's attack, then each monster's slots.
const KANG: Sheet = (58, 1, 0, 0, 0, 605, 2, 14, 3, 1, 0);
/// `(name, monster number, [(attack slot, numbers)])`.
type MonsterSheet = (&'static str, i32, &'static [(usize, Sheet)]);
const MONSTERS: [MonsterSheet; 3] = [
    (
        "giant-rat",
        1,
        &[
            (0, (10, 0, 0, 10, 9, 1000, 2, 10, 0, 0, 0)),
            (1, (70, 0, 0, 10, 9, 1000, 4, 20, 0, 0, 0)),
        ],
    ),
    (
        "kobold-thief",
        7,
        &[(0, (15, 10, 0, 10, 13, 666, 1, 8, 0, 0, 0))],
    ),
    (
        "acid-slime",
        5,
        &[(0, (20, 0, 0, 0, 16, 1000, 2, 9, 0, 0, 0))],
    ),
];

/// The move table: hit modifier, damage percentage, can it critical.
fn move_row(mv: Move) -> (i64, i64, bool) {
    match mv {
        Move::Attack | Move::Punch => (0, 0, true),
        Move::Kick => (0, 33, true),
        Move::JumpKick => (0, 66, true),
        Move::Bash => (-15, 10, false),
        Move::Smash => (-25, 20, false),
        Move::Unused8 => (-75, 125, false),
        Move::Backstab => (0, 0, false),
    }
}

/// The chance that a roll of 0..=99 is under `x`.
fn under(x: i64) -> f64 {
    x.clamp(0, 100) as f64 / 100.0
}

/// The exact outcome distribution of one attack.
fn closed_form(atk: &Fighter, def: &Fighter, mv: Move) -> BTreeMap<Bin, f64> {
    let (hitmod, pct, can_crit) = move_row(mv);
    // the defender's defence modifier joins the move's hit modifier
    let hitmod = hitmod + i64::from(def.defmod);
    let mut min = i64::from(atk.mindmg);
    let mut max = i64::from(atk.maxdmg);
    if pct != 0 {
        // the move's percentage raises both ends of the range
        min = min * (pct + 100) / 100;
        max = max * (pct + 100) / 100;
    }
    let mut crit = if can_crit { i64::from(atk.crit) } else { 0 };
    if crit > 40 {
        crit = (crit - 40) / 3 + 40; // above 40, a third counts
    }
    let bash_smash = matches!(mv, Move::Bash | Move::Smash);
    let backstab = mv == Move::Backstab;
    let av = i64::from(atk.av);

    // the hit chance; the roll is 1..=99 and lands under it
    let clamp = |h: i64| h.clamp(10, 99);
    let normal = clamp(if backstab {
        av - i64::from(def.dv)
    } else {
        let p = av + hitmod;
        let power = p * p / 14 / 10;
        if power == 0 {
            5
        } else {
            let d = i64::from(def.dv) + i64::from(def.defalign);
            100 - d * d / power
        }
    });
    let off_guard = if def.dodge < 0 {
        // a roll of 0..=99 above dodge + 100: caught off guard
        (0..100).filter(|v| *v > i64::from(def.dodge) + 100).count() as f64 / 100.0
    } else {
        0.0
    };
    let p_hit = off_guard * 98.0 / 99.0 + (1.0 - off_guard) * (normal - 1) as f64 / 99.0;

    // the dodge chance, rolled only for a positive dodge rating
    let mut dc = if av > 8 {
        i64::from(def.dodge) * 10 / (av / 8)
    } else {
        0
    };
    dc = dc.min(95);
    if backstab {
        dc /= 5;
    }
    let p_dodge = if def.dodge > 0 { under(dc) } else { 0.0 };

    let p_crit = if bash_smash { 0.0 } else { under(crit) };
    let mut out = BTreeMap::new();
    *out.entry(Bin::Miss).or_insert(0.0) += 1.0 - p_hit;
    for (critical, p_branch) in [(true, p_crit), (false, 1.0 - p_crit)] {
        if p_branch == 0.0 {
            continue;
        }
        let (mut lo, mut hi) = (min, max);
        if critical {
            lo = max * 2; // a critical deals two to four times the maximum
            hi = max * 4;
        }
        if lo > hi {
            hi = lo;
        }
        let width = (hi - lo + 1) as f64;
        for roll in lo..=hi {
            // less a tenth of the damage resistance, then BASH x3 and SMASH x5
            let mut damage = roll - i64::from(def.dr) / 10;
            match mv {
                Move::Bash => damage *= 3,
                Move::Smash => damage *= 5,
                _ => {}
            }
            let p = p_hit * p_branch / width;
            *out.entry(Bin::Dodged).or_insert(0.0) += p * p_dodge;
            let bin = if damage <= 0 {
                Bin::NoDamage
            } else {
                Bin::Hit {
                    damage: i32::try_from(damage).unwrap(),
                    critical,
                }
            };
            *out.entry(bin).or_insert(0.0) += p * (1.0 - p_dodge);
        }
    }
    out.retain(|_, p| *p > 0.0);
    out
}

/// The comparisons for one pairing and move; panics with every failure listed.
struct Case {
    name: String,
    mv: Move,
    expected: BTreeMap<Bin, f64>,
    observed: BTreeMap<Bin, u64>,
}

impl Case {
    fn run(name: String, atk: &Fighter, def: &Fighter, mv: Move, seed: u64) -> Self {
        let expected = closed_form(atk, def, mv);
        let mut rng = SeededRng::new(seed);
        let mut observed = BTreeMap::new();
        for _ in 0..N {
            let r = resolve(&mut atk.clone(), def, mv, &mut rng);
            *observed.entry(Bin::of(r.outcome)).or_insert(0u64) += 1;
        }
        Self {
            name,
            mv,
            expected,
            observed,
        }
    }

    fn obs(&self, f: impl Fn(&Bin) -> bool) -> u64 {
        self.observed
            .iter()
            .filter(|(b, _)| f(b))
            .map(|(_, c)| c)
            .sum()
    }

    fn exp(&self, f: impl Fn(&Bin) -> bool) -> f64 {
        self.expected
            .iter()
            .filter(|(b, _)| f(b))
            .map(|(_, p)| p)
            .sum()
    }

    /// The joint histogram against the closed form over the expected bins.
    fn joint(&self) -> Test {
        let bins: Vec<Bin> = self.expected.keys().copied().collect();
        let counts: Vec<u64> = bins
            .iter()
            .map(|b| self.observed.get(b).copied().unwrap_or(0))
            .collect();
        let probs: Vec<f64> = bins.iter().map(|b| self.expected[b]).collect();
        chi_square_gof(&counts, &probs)
    }

    /// The damage histogram of the hits (critical or not) against the closed form.
    fn damage(&self) -> Option<Test> {
        let hits: Vec<Bin> = self
            .expected
            .keys()
            .copied()
            .filter(|b| matches!(b, Bin::Hit { .. }))
            .collect();
        let total: f64 = hits.iter().map(|b| self.expected[b]).sum();
        if hits.len() < 2 || total == 0.0 {
            return None;
        }
        let counts: Vec<u64> = hits
            .iter()
            .map(|b| self.observed.get(b).copied().unwrap_or(0))
            .collect();
        let probs: Vec<f64> = hits.iter().map(|b| self.expected[b] / total).collect();
        Some(chi_square_gof(&counts, &probs))
    }

    /// `(name, successes, trials, expected rate, test)` for the rates.
    fn rates(&self) -> Vec<(&'static str, u64, u64, f64, Test)> {
        let landed = |b: &Bin| *b != Bin::Miss;
        let hit = |b: &Bin| matches!(b, Bin::Hit { .. });
        let crit = |b: &Bin| matches!(b, Bin::Hit { critical: true, .. });
        let dodged = |b: &Bin| *b == Bin::Dodged;
        let mut out = Vec::new();
        let n = u64::from(N);
        let x = self.obs(landed);
        let p = self.exp(landed);
        out.push(("hit", x, n, p, binomial_z(x, n, p)));
        let nl = x;
        let x = self.obs(dodged);
        let p = self.exp(dodged) / self.exp(landed);
        out.push(("dodge", x, nl, p, binomial_z(x, nl, p)));
        let nh = self.obs(hit);
        let x = self.obs(crit);
        let p = self.exp(crit) / self.exp(hit);
        out.push(("crit", x, nh, p, binomial_z(x, nh, p)));
        out
    }

    fn check(&self, failures: &mut Vec<String>) -> Value {
        for b in self.observed.keys() {
            if !self.expected.contains_key(b) {
                failures.push(format!(
                    "{}: outcome {b:?} outside the closed form",
                    self.name
                ));
            }
        }
        let mut tests = serde_json::Map::new();
        let mut record = |what: &str, t: Test, failures: &mut Vec<String>| {
            if !t.passes(ALPHA) {
                failures.push(format!(
                    "{}: {what} p = {:.4} ({t:?})",
                    self.name, t.p_value
                ));
            }
            tests.insert(
                what.into(),
                json!({"statistic": t.statistic, "df": t.df, "p_value": t.p_value}),
            );
        };
        record("joint", self.joint(), failures);
        if let Some(t) = self.damage() {
            record("damage", t, failures);
        }
        let mut rates = serde_json::Map::new();
        for (what, x, n, p, t) in self.rates() {
            record(what, t, failures);
            rates.insert(
                what.into(),
                json!({"successes": x, "trials": n, "expected": p,
                       "observed": if n == 0 { 0.0 } else { x as f64 / n as f64 }}),
            );
        }
        let bins = |m: &dyn Fn(Bin) -> Option<Value>| -> serde_json::Map<String, Value> {
            self.expected
                .keys()
                .filter_map(|b| m(*b).map(|v| (b.label(), v)))
                .collect()
        };
        json!({
            "case": self.name,
            "move": format!("{:?}", self.mv),
            "n": N,
            "expected": bins(&|b| Some(json!(self.expected[&b]))),
            "observed": bins(&|b| Some(json!(self.observed.get(&b).copied().unwrap_or(0)))),
            "rates": rates,
            "tests": tests,
        })
    }
}

fn export_path() -> PathBuf {
    std::env::var_os("SILVERMERE_AB_EXPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ab_sheets.json"))
}

/// The pairings: Kang on each monster with each move the sheet carries a weapon for
/// (attack, bash, smash), and each monster attack slot on Kang. The fighters built from
/// the download must give the sheets' pinned numbers first.
fn cases() -> Vec<(String, Fighter, Fighter, Move)> {
    assert_eq!(numbers(&kang(Move::Attack)), KANG, "kang");
    let kang = fighter(KANG);
    let mut out = Vec::new();
    for (m, number, pinned) in MONSTERS {
        let built: Vec<(usize, Sheet)> = monster(number)
            .iter()
            .map(|(slot, f)| (*slot, numbers(f)))
            .collect();
        assert_eq!(built, pinned, "{m}");
        let defender = fighter(pinned[0].1);
        for mv in [Move::Attack, Move::Bash, Move::Smash] {
            out.push((
                format!("kang -> {m} {mv:?}"),
                kang.clone(),
                defender.clone(),
                mv,
            ));
        }
        for &(n, slot) in pinned {
            out.push((
                format!("{m} attack {n} -> kang"),
                fighter(slot),
                kang.clone(),
                Move::Attack,
            ));
        }
    }
    out
}

/// The hit chance, the critical, the damage, the dodge and BASH x3 / SMASH x5 after damage
/// resistance: every sheet pairing's 200,000 blows match the closed form at p > 0.01.
#[test]
fn sheets_match_the_closed_form() {
    let mut failures = Vec::new();
    let mut export = Vec::new();
    for (i, (name, atk, def, mv)) in cases().into_iter().enumerate() {
        let case = Case::run(name, &atk, &def, mv, 0xAB00 + i as u64);
        export.push(case.check(&mut failures));
    }
    let blows = blows_per_round(&mut failures);
    let doc = json!({
        "source": "crates/silvermere-testkit/tests/stock_ab_sheets.rs",
        "alpha": ALPHA,
        "cases": export,
        "blows_per_round": blows,
    });
    let path = export_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("export dir");
    }
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).expect("export");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// At most six blows while energy lasts, and the tick adds 1000 with auto-combat carrying
/// the rest: for cost `c`, a fresh round makes `min(6, 1000/c)` blows and a carried run
/// makes `floor(1000/c)` or one more, the larger in a fraction `1000/c - floor(1000/c)` of
/// rounds. The costs are Kang's: attack 605, bash 1000 (2 x 605 capped at his maximum
/// energy) and smash his maximum energy.
fn blows_per_round(failures: &mut Vec<String>) -> Value {
    let attack = i32::try_from(kang(Move::Attack).energy).unwrap();
    let bash = i32::try_from(kang(Move::Bash).energy).unwrap();
    assert_eq!((attack, bash), (605, 1000));
    let maxenergy = i32::from(KANG_MAXENERGY);
    let mut out = Vec::new();
    for (what, cost) in [("attack", attack), ("bash", bash), ("smash", maxenergy)] {
        let fresh = energy_round(0, cost).blows;
        let expect_fresh = (1000 / cost).min(6);
        if i64::from(fresh) != i64::from(expect_fresh) {
            failures.push(format!(
                "{what}: fresh round {fresh} blows, expected {expect_fresh}"
            ));
        }
        let mut hist = [0u64; 7];
        let mut carry = 0;
        for _ in 0..ROUNDS {
            let r = energy_round(carry, cost);
            carry = r.leftover;
            hist[r.blows as usize] += 1;
        }
        let ratio = 1000.0 / f64::from(cost);
        let lo = ratio.floor().min(6.0) as usize;
        let mut probs = [0.0; 7];
        if lo >= 6 {
            probs[6] = 1.0;
        } else {
            let frac = ratio - ratio.floor();
            probs[lo] = 1.0 - frac;
            if frac > 0.0 {
                probs[lo + 1] = frac;
            }
        }
        let t = chi_square_gof(&hist, &probs);
        if !t.passes(ALPHA) || hist.iter().zip(probs).any(|(&c, p)| c > 0 && p == 0.0) {
            failures.push(format!(
                "{what} (cost {cost}): blows {hist:?} vs {probs:?}, {t:?}"
            ));
        }
        out.push(
            json!({"move": what, "cost": cost, "rounds": ROUNDS, "fresh": fresh,
                        "histogram": hist, "expected": probs, "p_value": t.p_value}),
        );
    }
    json!(out)
}

/// Kang's hit chances (AV 58 gives 99, 84, 63, 34 against DV 0, 20, 30, 40) are what the
/// closed form above gives.
#[test]
fn kang_hit_chance_table() {
    let atk = fighter(KANG);
    for (dv, chance) in [(0, 99), (20, 84), (30, 63), (40, 34)] {
        let def = Fighter {
            dv,
            ..Fighter::default()
        };
        let miss = closed_form(&atk, &def, Move::Attack)[&Bin::Miss];
        let want = 1.0 - f64::from(chance - 1) / 99.0;
        assert!(
            (miss - want).abs() < 1e-12,
            "dv {dv}: miss {miss}, want {want}"
        );
    }
}
