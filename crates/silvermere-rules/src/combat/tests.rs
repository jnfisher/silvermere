//! Layer 1 tests for `combat`: one test per melee formula.
//!
//! The Kang Witchunter is the fixed test sheet: Witchunter combat 7,
//! Str 79, Int 30, Wil 45, Agi 42, Hea 74, Cha 30, with the stock quarterstaff (WCCITEMS 100:
//! 2-12, speed 1200, Str 30, accuracy 0, AC 10) and 700 of 3792 weight carried (18%). The
//! monsters are the stock giant rat (WCCKNMSR 1), kobold thief (7) and acid slime (5).

use std::collections::VecDeque;

use silvermere_types::enums::{Alignment, ArmourType, ItemType};
use silvermere_types::flags::{
    ActiveMonsterFlags, PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2,
};
use silvermere_types::ids::MessageNo;
use silvermere_types::records::{ActiveMonster, Character, Class, Item, KnownMonster};

use super::*;
use crate::rng::{Rng, SeededRng};

/// An `Rng` that hands out scripted values, so a test can force each roll of
/// `resolve` in turn: hit, negative dodge, critical, damage, dodge.
struct FixedRng(VecDeque<i32>);

impl FixedRng {
    fn new(rolls: &[i32]) -> Self {
        Self(rolls.iter().copied().collect())
    }
}

impl Rng for FixedRng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let v = self.0.pop_front().expect("a scripted roll");
        assert!(
            lo <= v && (v < hi || hi <= lo),
            "roll {v} outside [{lo}, {hi})"
        );
        v
    }

    fn long(&mut self, lo: i64, _hi: i64) -> i64 {
        lo
    }
}

fn fighter(av: i32, dv: i32) -> Fighter {
    Fighter {
        av,
        dv,
        mindmg: 10,
        maxdmg: 20,
        ..Fighter::default()
    }
}

/// Kasimir, the Kang Witchunter test sheet at level 1: alive, not in a group,
/// no spell cast this round, 18% laden.
fn kang() -> Character {
    let mut c = blank::character();
    c.level = 1;
    c.str = 79;
    c.intel = 30;
    c.wil = 45;
    c.agi = 42;
    c.hea = 74;
    c.cha = 30;
    c.hp = 20;
    c.maxenergy = 1000;
    c.encumpct = 18;
    c.rank = 0;
    // level/10 + (int-50)/10 + (agi-50)/20 + (cha-50)/30 = -2 -> 1.
    c.mabase = 1;
    c.settings = PlayerSettings::CANCAST;
    c
}

/// Witchunter (WCCCLASS 2): combat 7.
fn witchunter() -> Class {
    let mut c = blank::class();
    c.combat = 7;
    c.maxenergy = 1000;
    c
}

/// The stock quarterstaff (WCCITEMS 100).
fn quarterstaff() -> Item {
    let mut i = blank::item();
    i.item_type = ItemType::Weapon;
    i.speed = 1200;
    i.accuracy = 0;
    i.mindmg = 2;
    i.maxdmg = 12;
    i.ac = 10;
    i.strreq = 30;
    i.hitmsg = MessageNo(8237);
    i.missmsg = MessageNo(8289);
    i
}

fn armour(ac: i16, dr: i16, accuracy: i16, armtype: ArmourType) -> Item {
    let mut i = blank::item();
    i.item_type = ItemType::Armour;
    i.ac = ac;
    i.dr = dr;
    i.accuracy = accuracy;
    i.armtype = armtype;
    i
}

/// The blow a solo Kang makes at an evil monster: the same `atkmove` and `atktype`, normal light.
fn blow(mv: Move) -> Blow {
    Blow::new(mv, AlignGroup::Evil)
}

fn build(plr: &Character, weapon: Option<&Item>, mv: Move) -> Fighter {
    player_fighter(
        plr,
        &witchunter(),
        weapon,
        &[],
        PlayerAbilities::default(),
        blow(mv),
    )
}

fn build_with(plr: &Character, weapon: Option<&Item>, abil: PlayerAbilities, b: Blow) -> Fighter {
    player_fighter(plr, &witchunter(), weapon, &[], abil, b)
}

// ---- energy ----------------------------------------------------------------------------

// compute_energy_used: skill, quick, divisor and cost.
#[test]
fn energy_per_blow_formula() {
    // Kang with the quarterstaff: skill 52, quick 192, divisor 1664, 1200000/1664 = 721.
    assert_eq!(compute_energy_used(1, 7, 42, 1200, 0), 721);
    // The weapon's own energy is added on top.
    assert_eq!(compute_energy_used(1, 7, 42, 1200, 25), 746);
    // Level 10 combat 3 agility 70: skill 75, quick 220, divisor 2750, 1000000/2750 = 363.
    assert_eq!(compute_energy_used(10, 3, 70, 1000, 0), 363);
}

// compute_energy_used: a zero divisor gives 50.
#[test]
fn energy_divide_by_zero_falls_back_to_50() {
    assert_eq!(compute_energy_used(1, 3, -150, 1000, 0), 50);
}

// attack_user_monster: blows while energy lasts and `rounds <= 5`, so six
// at most.
#[test]
fn blows_per_round_is_min_6_and_energy_over_cost() {
    assert_eq!(energy_round(0, 150).blows, 6);
    assert_eq!(energy_round(0, 250).blows, 4);
    assert_eq!(energy_round(0, 1000).blows, 1);
    assert_eq!(energy_round(0, 0).blows, 5);
}

// fast_update_character: the tick adds maxenergy; the carry-over is
// capped at one round except in auto-combat, where a 605 attack lands once then twice.
#[test]
fn energy_carries_over_between_rounds() {
    let r1 = energy_round(0, 605);
    assert_eq!((r1.blows, r1.leftover), (1, 395));
    let r2 = energy_round(r1.leftover, 605);
    assert_eq!((r2.blows, r2.leftover), (2, 185));
    let r3 = energy_round(r2.leftover, 605);
    assert_eq!(r3.blows, 1);
}

// ---- player fighter --------------------------------------------------------------------

// player_fighter: the fighter starts from nothing, 1-4 damage;
// fists keep that range and cost the fists speed 1200.
#[test]
fn fists_keep_the_default_range() {
    let mut c = kang();
    c.str = 50;
    let f = build(&c, None, Move::Bash); // a non-martial move without a weapon
    assert_eq!((f.mindmg, f.maxdmg), (1, 4));
    // 1200000/1664 = 721 -> 605, doubled by the bash and capped.
    assert_eq!(f.energy, 1000);
    assert_eq!(f.hitverbs, MessageNo(0));
    let f = build(&c, None, Move::Smash);
    assert_eq!(f.energy, 1000);
    let mut c2 = c.clone();
    c2.encumpct = 50; // 721 * 100/100
    assert_eq!(build(&c2, None, Move::Unused8).energy, 721);
    c2.status2 = PlayerStatus2::SLOWED; // fists at 1800: 1800000/1664 = 1081
    assert_eq!(build(&c2, None, Move::Unused8).energy, 1000);
    c2.maxenergy = 2000;
    assert_eq!(build(&c2, None, Move::Unused8).energy, 1081);
}

// player_fighter: defence against the foe's alignment only when
// the groups differ; (sic) against a good foe the "defence
// against Evil" value replaces the "defence against Good" one.
#[test]
fn defence_against_alignment_falls_through() {
    let abil = PlayerAbilities {
        def_vs_good: 7,
        def_vs_evil: 3,
        ..PlayerAbilities::default()
    };
    let c = kang(); // neutral (0 evil points)
    let w = quarterstaff();
    let good = build_with(
        &c,
        Some(&w),
        abil,
        Blow::new(Move::Attack, AlignGroup::Good),
    );
    assert_eq!(good.defalign, 3);
    let evil = build_with(
        &c,
        Some(&w),
        abil,
        Blow::new(Move::Attack, AlignGroup::Evil),
    );
    assert_eq!(evil.defalign, 3);
    let neutral = build_with(
        &c,
        Some(&w),
        abil,
        Blow::new(Move::Attack, AlignGroup::Neutral),
    );
    assert_eq!(neutral.defalign, 0);
    // An Outlaw (40 evil points) is in the evil group: nothing against an evil foe.
    let mut outlaw = kang();
    outlaw.evil = 40;
    let f = build_with(
        &outlaw,
        Some(&w),
        abil,
        Blow::new(Move::Attack, AlignGroup::Evil),
    );
    assert_eq!(f.defalign, 0);
}

// player_fighter: Shadow Stealth adds 10 to that defence.
#[test]
fn shadow_stealth_adds_10_defalign() {
    let abil = PlayerAbilities {
        shadow_stealth: true,
        ..PlayerAbilities::default()
    };
    let f = build_with(&kang(), None, abil, blow(Move::Attack));
    assert_eq!(f.defalign, 10);
}

// player_fighter: crit = max(1, mabase + critbonus).
#[test]
fn crit_base_is_mabase_plus_bonus_never_below_1() {
    let mut c = kang();
    c.mabase = 7;
    c.critbonus = 5;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Attack).crit, 12);
    c.critbonus = -20;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Attack).crit, 1);
}

// player_fighter: dodge from the ability, charm, level, agility.
#[test]
fn dodge_formula() {
    let mut c = kang();
    c.encumpct = 50; // no light-load bonus
    c.cha = 75;
    c.level = 15;
    c.agi = 80;
    let abil = PlayerAbilities {
        dodge: 4,
        ..PlayerAbilities::default()
    };
    let f = build_with(&c, None, abil, blow(Move::Punch));
    // 4 + 25/5 + 15/5 + 30/3
    assert_eq!(f.dodge, 4 + 5 + 3 + 10);
}

// player_fighter: the dead cannot dodge (hp <= 0 gives -1), and
// the light-load bonus needs a living player, so -1 it stays.
#[test]
fn downed_player_has_dodge_minus_one() {
    let mut c = kang();
    c.hp = 0;
    let f = build(&c, Some(&quarterstaff()), Move::Attack);
    assert_eq!(f.dodge, -1);
    // Without the light-load accuracy (15 - 1 = 14, i.e. 7 skill, 14 AV) the AV is 44.
    assert_eq!(f.av, 58 - 14);
    c.hp = -5;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Attack).dodge, -1);
}

// player_fighter: a punch costs the fixed speed 1150 (1750
// slowed) and does level*n/8+2 to (level+3)*n/4+6 from the Punch ability `n`; from level 20
// the level-20 figures.
#[test]
fn punch_energy_and_range() {
    let mut c = kang();
    c.str = 50;
    c.encumpct = 50;
    c.level = 10;
    let abil = PlayerAbilities {
        punch: 8,
        ..PlayerAbilities::default()
    };
    let f = build_with(&c, Some(&quarterstaff()), abil, blow(Move::Punch));
    assert_eq!((f.mindmg, f.maxdmg), (10 * 8 / 8 + 2, 13 * 8 / 4 + 6));
    // Level 10 combat 7: skill 115, quick 192, divisor 3680; 1150000/3680 = 312.
    assert_eq!(f.energy, 312);
    c.level = 30;
    let f = build_with(&c, Some(&quarterstaff()), abil, blow(Move::Punch));
    assert_eq!((f.mindmg, f.maxdmg), (8 * 20 / 8 + 2, 8 * 23 / 4 + 6));
    c.level = 10;
    c.status2 = PlayerStatus2::SLOWED;
    let f = build_with(&c, Some(&quarterstaff()), abil, blow(Move::Punch));
    assert_eq!(f.energy, 1750 * 1000 / 3680);
}

// player_fighter: a kick costs speed 1400 (2000 slowed), max
// `level*n/6 + 7` from the Kick ability.
#[test]
fn kick_energy_and_range() {
    let mut c = kang();
    c.str = 50;
    c.encumpct = 50;
    c.level = 12;
    let abil = PlayerAbilities {
        kick: 9,
        ..PlayerAbilities::default()
    };
    let f = build_with(&c, None, abil, blow(Move::Kick));
    assert_eq!((f.mindmg, f.maxdmg), (12 * 9 / 8 + 2, 12 * 9 / 6 + 7));
    // Level 12 combat 7: skill 129, divisor 129*192*1500/9000 = 4128; 1400000/4128 = 339.
    assert_eq!(f.energy, 339);
}

// player_fighter: a jumpkick costs speed 1900 (2650 slowed),
// max `level*n/6 + 8` from the Jumpkick ability.
#[test]
fn jumpkick_energy_and_range() {
    let mut c = kang();
    c.str = 50;
    c.encumpct = 50;
    c.level = 12;
    let abil = PlayerAbilities {
        jumpkick: 9,
        ..PlayerAbilities::default()
    };
    let f = build_with(&c, None, abil, blow(Move::JumpKick));
    assert_eq!((f.mindmg, f.maxdmg), (12 * 9 / 8 + 2, 12 * 9 / 6 + 8));
    assert_eq!(f.energy, 1900 * 1000 / 4128);
}

// player_fighter: the weapon's AC, range, speed and energy;
// slowed, the speed is times 3/2.
#[test]
fn weapon_energy_before_modifiers() {
    let mut c = kang();
    c.encumpct = 50; // (50/2+75)/100 = 1.00: no encumbrance change
    let mut w = quarterstaff();
    w.energy = 25;
    // A punch ignores the weapon: fixed speed 1150, 1150000/1664 = 691.
    assert_eq!(build(&c, Some(&w), Move::Punch).energy, 691);
    let f = build(&c, Some(&w), Move::Attack);
    assert_eq!(f.energy, 746);
    assert_eq!((f.mindmg, f.maxdmg), (2, 12 + 2));
    c.status2 = PlayerStatus2::SLOWED;
    c.maxenergy = 2000;
    assert_eq!(
        build(&c, Some(&w), Move::Unused8).energy,
        25 + 1800 * 1000 / 1664
    );
}

// player_fighter: energy * ((strreq-str)*3 + 200)/200 when
// short, and no fast-weapon crit bonus then.
#[test]
fn strength_shortfall_raises_energy() {
    let mut c = kang();
    c.encumpct = 50;
    let mut w = quarterstaff();
    w.strreq = 89; // 10 short: 721 * 230 / 200 = 829
    assert_eq!(build(&c, Some(&w), Move::Unused8).energy, 721 * 230 / 200);
    w.strreq = 79;
    assert_eq!(build(&c, Some(&w), Move::Unused8).energy, 721);
}

// player_fighter: weapon accuracy feeds the skill term.
#[test]
fn weapon_accuracy_adds_to_av() {
    let mut w = quarterstaff();
    let base = build(&kang(), Some(&w), Move::Attack).av;
    w.accuracy = 20; // accuracy 20+14 = 34 vs 0 -> 1 -> 15: skill +10, AV +20
    assert_eq!(build(&kang(), Some(&w), Move::Attack).av, base + 20);
}

// player_fighter: Str adds (str-50)/10 to max and, past 100,
// (str-100)/10*2 to min; min never exceeds max, neither goes below 0.
#[test]
fn strength_damage_bonuses() {
    let mut c = kang();
    c.str = 95;
    let f = build(&c, Some(&quarterstaff()), Move::Unused8);
    assert_eq!((f.mindmg, f.maxdmg), (2, 12 + 4));
    c.str = 125;
    let f = build(&c, Some(&quarterstaff()), Move::Unused8);
    assert_eq!((f.mindmg, f.maxdmg), (2 + 4, 12 + 7));
    c.str = 40;
    let f = build(&c, Some(&quarterstaff()), Move::Unused8);
    assert_eq!((f.mindmg, f.maxdmg), (2, 12 - 1));
    let mut w = quarterstaff();
    w.mindmg = 3;
    w.maxdmg = 3;
    c.str = 125;
    let f = build(&c, Some(&w), Move::Unused8);
    assert_eq!((f.mindmg, f.maxdmg), (3 + 4, 3 + 7));
    w.mindmg = 0;
    w.maxdmg = 0;
    c.str = 30;
    let f = build(&c, Some(&w), Move::Unused8);
    assert_eq!((f.mindmg, f.maxdmg), (0, 0));
}

// player_fighter: each armour piece adds AC, DR and
// accuracy; other item types worn add nothing.
#[test]
fn armour_sums_ac_dr_and_accuracy() {
    let c = kang();
    let w = quarterstaff();
    let base = build(&c, Some(&w), Move::Attack);
    let a1 = armour(55, 15, 6, ArmourType::Padded);
    let a2 = armour(44, 5, 4, ArmourType::Robes);
    let mut not_armour = armour(100, 100, 100, ArmourType::Natural);
    not_armour.item_type = ItemType::Weapon;
    let abil = PlayerAbilities::default();
    let f = player_fighter(
        &c,
        &witchunter(),
        Some(&w),
        &[&a1, &a2, &not_armour],
        abil,
        blow(Move::Attack),
    );
    // accuracy 1+10 (the 0 -> 1 step is skipped now) + 14 = 24: skill 12+12 = 24, +6 = 30
    // -> 60 + 9 - 1 = 68.
    assert_eq!(f.av, base.av + 10);
    assert_eq!(f.dv, (10 + 55 + 44) / 10);
    assert_eq!(f.dr, 20);
}

// player_fighter: accuracy 0 counts as 1; under 33% load a
// living player gets accuracy += 15 - pct/10 and dodge += 10 - pct/10; nothing at 33.
#[test]
fn light_load_bonus() {
    let mut c = kang();
    c.encumpct = 33;
    let heavy = build(&c, Some(&quarterstaff()), Move::Attack);
    c.encumpct = 0;
    let light = build(&c, Some(&quarterstaff()), Move::Attack);
    // accuracy 1 -> 16: skill 0 -> 8, AV +16.
    assert_eq!(light.av, heavy.av + 16);
    assert_eq!(light.dodge, heavy.dodge + 10);
    c.encumpct = 29;
    let f = build(&c, Some(&quarterstaff()), Move::Attack);
    assert_eq!(f.dodge, heavy.dodge + 10 - 2);
    // Accuracy 1 at 33%: skill 12 + 0 + 0 = 12, training 6, (18)*2 + 9 - 1 = 44.
    assert_eq!(heavy.av, 44);
}

// player_fighter: skill, training with the whole square root
// of the level and the attack value.
#[test]
fn attack_value_with_training_root() {
    let mut c = kang();
    c.encumpct = 50;
    c.str = 50;
    c.agi = 50;
    let w = quarterstaff();
    for (level, root) in [
        (1, 1),
        (3, 1),
        (4, 2),
        (8, 2),
        (9, 3),
        (15, 3),
        (16, 4),
        (24, 4),
        (25, 5),
    ] {
        c.level = level;
        let skill = 12 + i32::from(level) / 2; // accuracy 1/2 = 0
        let training = 6 * root;
        assert_eq!(
            build(&c, Some(&w), Move::Attack).av,
            (skill + training) * 2,
            "level {level}"
        );
    }
}

// the square-root loop: the whole part of sqrt(level).
#[test]
fn isqrt_levels() {
    for (n, r) in [
        (0, 0),
        (1, 1),
        (3, 1),
        (4, 2),
        (8, 2),
        (9, 3),
        (15, 3),
        (16, 4),
        (24, 4),
        (25, 5),
    ] {
        assert_eq!(isqrt(n), r, "isqrt({n})");
    }
}

// player_fighter: a backstab's AV is (stealth+agi)/2, +5 with
// race and class stealth, -15 with race stealth alone, plus half the Backstab AV ability,
// -10 when spotted.
#[test]
fn backstab_attack_value() {
    let mut c = kang();
    c.stealth = 37;
    c.agi = 90;
    let abil = PlayerAbilities {
        backstab_av: 9,
        ..PlayerAbilities::default()
    };
    let w = quarterstaff();
    let base = (37 + 90) / 2 + 4;
    assert_eq!(
        build_with(&c, Some(&w), abil, blow(Move::Backstab)).av,
        base
    );
    c.flags = PlayerFlags::RACESTEALTH;
    assert_eq!(
        build_with(&c, Some(&w), abil, blow(Move::Backstab)).av,
        base - 15
    );
    c.flags = PlayerFlags::RACESTEALTH | PlayerFlags::CLASSSTEALTH;
    assert_eq!(
        build_with(&c, Some(&w), abil, blow(Move::Backstab)).av,
        base + 5
    );
    c.status = PlayerStatus::SPOTTED;
    assert_eq!(
        build_with(&c, Some(&w), abil, blow(Move::Backstab)).av,
        base + 5 - 10
    );
}

// player_fighter: DV = ac/10 (weapon and armour); against a
// backstab (ac/10 + perception)/2.
#[test]
fn defence_value_is_tenth_of_ac() {
    let mut c = kang();
    c.perception = 20;
    let a = armour(117, 0, 0, ArmourType::Padded);
    let w = quarterstaff();
    let abil = PlayerAbilities::default();
    let f = player_fighter(&c, &witchunter(), Some(&w), &[&a], abil, blow(Move::Attack));
    assert_eq!(f.dv, 12);
    let f = player_fighter(
        &c,
        &witchunter(),
        Some(&w),
        &[&a],
        abil,
        blow(Move::Backstab),
    );
    assert_eq!(f.dv, (12 + 20) / 2);
}

// player_fighter: av += avbonus; dv += acbonus (a quarter of it
// against a backstab); maxdmg += dmgbonus; dr += drbonus, then dr = (drpct+100)*dr/100.
#[test]
fn alter_totals_of_the_player_record() {
    let mut c = kang();
    c.avbonus = 7;
    c.acbonus = 11;
    c.dmgbonus = 3;
    c.drbonus = 5;
    c.drpct = 50;
    let a = armour(0, 15, 0, ArmourType::Padded);
    let w = quarterstaff();
    let abil = PlayerAbilities::default();
    let f = player_fighter(&c, &witchunter(), Some(&w), &[&a], abil, blow(Move::Attack));
    assert_eq!(f.av, 58 + 7);
    assert_eq!(f.dv, 1 + 11);
    assert_eq!((f.mindmg, f.maxdmg), (2, 14 + 3));
    assert_eq!(f.dr, 150 * 20 / 100);
    let f = player_fighter(
        &c,
        &witchunter(),
        Some(&w),
        &[&a],
        abil,
        blow(Move::Backstab),
    );
    // (ac/10 + perception 0)/2 = 0, plus a quarter of acbonus.
    assert_eq!(f.dv, 11 / 4);
}

// player_fighter: the martial moves' "Alter ... AV" and
// "... DMG" abilities.
#[test]
fn martial_arts_av_and_dmg_abilities() {
    let mut c = kang();
    c.str = 50;
    c.level = 10;
    let abil = PlayerAbilities {
        punch: 4,
        punch_av: 6,
        punch_dmg: 3,
        kick: 4,
        kick_av: 7,
        kick_dmg: 2,
        jumpkick: 4,
        jumpkick_av: 8,
        jumpkick_dmg: 1,
        ..PlayerAbilities::default()
    };
    let none = build(&c, None, Move::Punch);
    let f = build_with(&c, None, abil, blow(Move::Punch));
    assert_eq!(f.av, none.av + 6);
    assert_eq!(
        (f.mindmg, f.maxdmg),
        (10 * 4 / 8 + 2 + 3, 13 * 4 / 4 + 6 + 3)
    );
    let f = build_with(&c, None, abil, blow(Move::Kick));
    assert_eq!(f.av, none.av + 7);
    assert_eq!(
        (f.mindmg, f.maxdmg),
        (10 * 4 / 8 + 2 + 2, 10 * 4 / 6 + 7 + 2)
    );
    let f = build_with(&c, None, abil, blow(Move::JumpKick));
    assert_eq!(f.av, none.av + 8);
    assert_eq!(
        (f.mindmg, f.maxdmg),
        (10 * 4 / 8 + 2 + 1, 10 * 4 / 6 + 8 + 1)
    );
}

// player_fighter: a backstab (atktype) costs the whole round
// and does twice the damage plus level*2 + stealth/10 plus the Backstab Min./Max.
// abilities, cut to 75% for a race-only stealth, then raised by level percent.
#[test]
fn backstab_damage() {
    let mut c = kang();
    c.str = 50;
    c.level = 5;
    c.stealth = 42;
    let w = quarterstaff(); // 2-12
    let abil = PlayerAbilities {
        backstab_min: 1,
        backstab_max: 3,
        ..PlayerAbilities::default()
    };
    let f = build_with(&c, Some(&w), abil, blow(Move::Backstab));
    let av = 10 + 4;
    let (min, max) = (4 + av + 1, 24 + av + 3);
    assert_eq!((f.mindmg, f.maxdmg), (min * 105 / 100, max * 105 / 100));
    // The round's energy, then the encumbrance step (84% at 18%) and the clamp.
    assert_eq!(f.energy, 1000 * 84 / 100);
    c.flags = PlayerFlags::RACESTEALTH;
    let f = build_with(&c, Some(&w), abil, blow(Move::Backstab));
    assert_eq!(
        (f.mindmg, f.maxdmg),
        (min * 75 / 100 * 105 / 100, max * 75 / 100 * 105 / 100)
    );
}

// player_fighter: cmd_backstab sets atktype to backstab
// and atkmove to a plain blow when the weapon is not made for backstabbing: the damage and
// the cost are a backstab's, the attack and defence values ordinary.
#[test]
fn backstab_type_with_a_plain_move() {
    let mut c = kang();
    c.str = 50;
    c.level = 5;
    c.stealth = 42;
    let w = quarterstaff();
    let b = Blow {
        atkmove: Move::Punch,
        atktype: Move::Backstab,
        style: AlignGroup::Evil,
        light_level: 0,
    };
    let f = build_with(&c, Some(&w), PlayerAbilities::default(), b);
    let plain = build(&c, Some(&w), Move::Punch);
    assert_eq!((f.av, f.dv), (plain.av, plain.dv));
    assert_eq!(f.energy, 840);
    // A punch's range (level 5, Punch 0: 2 to 6) doubled plus 14, times 105%.
    assert_eq!(
        (f.mindmg, f.maxdmg),
        ((4 + 14) * 105 / 100, (12 + 14) * 105 / 100)
    );
}

// player_fighter: "Alter Energy Usage" is a percentage of the
// cost, applied only when positive (the test is unsigned 32-bit, so a negative value
// gives a nonsense cost rather than a reduction).
#[test]
fn energy_use_ability_percent() {
    let mut c = kang();
    c.encumpct = 50;
    c.energyuse = 80;
    assert_eq!(
        build(&c, Some(&quarterstaff()), Move::Unused8).energy,
        721 * 80 / 100
    );
    c.energyuse = 0;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Unused8).energy, 721);
    c.energyuse = -20;
    let f = build(&c, Some(&quarterstaff()), Move::Unused8);
    assert_ne!(f.energy, 721 * 80 / 100);
    assert!((200..=1000).contains(&f.energy));
}

// player_fighter: rank 1 (front) +15 AV / -10 defalign also gets
// rank 2's -10 AV / +15, netting +5/+5; rank 2 (back) -10/+15;
// rank 0 nothing; any other value is the front rank; a jumpkick keeps its AV.
#[test]
fn rank_switch_fall_through() {
    let mut c = kang();
    let w = quarterstaff();
    let none = build(&c, Some(&w), Move::Attack);
    c.rank = 1;
    let front = build(&c, Some(&w), Move::Attack);
    c.rank = 2;
    let back = build(&c, Some(&w), Move::Attack);
    c.rank = 3;
    let other = build(&c, Some(&w), Move::Attack);
    assert_eq!((front.av - none.av, front.defalign - none.defalign), (5, 5));
    assert_eq!(
        (back.av - none.av, back.defalign - none.defalign),
        (-10, 15)
    );
    assert_eq!((other.av, other.defalign), (front.av, front.defalign));
    assert_eq!(front.dv, none.dv);
    c.rank = 1;
    let jk_none = {
        let mut c0 = c.clone();
        c0.rank = 0;
        build(&c0, Some(&w), Move::JumpKick)
    };
    let jk_front = build(&c, Some(&w), Move::JumpKick);
    assert_eq!(
        (
            jk_front.av - jk_none.av,
            jk_front.defalign - jk_none.defalign
        ),
        (0, 5)
    );
}

// player_fighter: blind, or a light level above 900: -10 AV
// and defence.
#[test]
fn blind_or_dark_costs_10() {
    let mut c = kang();
    let w = quarterstaff();
    let base = build(&c, Some(&w), Move::Attack);
    c.status = PlayerStatus::BLIND;
    let f = build(&c, Some(&w), Move::Attack);
    assert_eq!((f.av, f.defalign), (base.av - 10, base.defalign - 10));
    c.status = PlayerStatus::empty();
    let mut b = blow(Move::Attack);
    b.light_level = 901;
    let f = build_with(&c, Some(&w), PlayerAbilities::default(), b);
    assert_eq!((f.av, f.defalign), (base.av - 10, base.defalign - 10));
    b.light_level = 900;
    assert_eq!(
        build_with(&c, Some(&w), PlayerAbilities::default(), b).av,
        base.av
    );
}

// player_fighter: Protective Shield adds to DV, a quarter
// through platemail, a third through chain or scale, half through leather; against a
// backstab a quarter of what is left.
#[test]
fn shield_through_armour() {
    let mut c = kang();
    c.shield = 24;
    let w = quarterstaff();
    let abil = PlayerAbilities::default();
    let dv = |pieces: &[&Item], mv| {
        player_fighter(&c, &witchunter(), Some(&w), pieces, abil, blow(mv)).dv
    };
    assert_eq!(dv(&[], Move::Attack), 1 + 24);
    let plate = armour(0, 0, 0, ArmourType::Platemail);
    let chain = armour(0, 0, 0, ArmourType::Chainmail);
    let scale = armour(0, 0, 0, ArmourType::Scalemail);
    let leather = armour(0, 0, 0, ArmourType::RigidLeather);
    assert_eq!(dv(&[&plate, &leather], Move::Attack), 1 + 6);
    assert_eq!(dv(&[&chain], Move::Attack), 1 + 8);
    assert_eq!(dv(&[&scale, &leather], Move::Attack), 1 + 8);
    assert_eq!(dv(&[&leather], Move::Attack), 1 + 12);
    // (ac/10 + perception 0)/2 = 0, plus a quarter of the shield left.
    assert_eq!(dv(&[&leather], Move::Backstab), 12 / 4);
}

// player_fighter: energy = energy*(encumpct/2+75)/100.
#[test]
fn encumbrance_scales_energy() {
    let mut c = kang();
    c.encumpct = 100;
    assert_eq!(
        build(&c, Some(&quarterstaff()), Move::Unused8).energy,
        721 * 125 / 100
    );
    c.encumpct = 18;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Unused8).energy, 605);
    c.encumpct = 13; // the card's 13%: 721 * 81 / 100
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Unused8).energy, 584);
}

/// The quarterstaff at another speed: a dagger-class weapon under 200 energy.
fn fast_weapon(speed: i16) -> Item {
    let mut w = quarterstaff();
    w.speed = speed;
    w
}

// player_fighter: a weapon under 200 energy (before the floor)
// adds 200 - energy + (agi-50)/10 crit, capped at 20, if the load is at most 66% and the
// strength is met; halved from 33%.
#[test]
fn fast_weapon_crit_at_cost_150() {
    let mut c = kang();
    c.encumpct = 10; // 80/100: the cost is speed*1000/1664*80/100
    // Agi 42: (42-50)/10 = 0. speed 310 -> 186 -> 148: 200-148 = 52, capped at 20.
    let f = build(&c, Some(&fast_weapon(310)), Move::Unused8);
    assert_eq!((f.crit, f.energy), (1 + 20, 200));
    // speed 400 -> 240 -> 192: 200-192 = 8.
    let f = build(&c, Some(&fast_weapon(400)), Move::Unused8);
    assert_eq!(f.crit, 1 + 8);
    // At 33-66% load the bonus is halved: 40% is 95/100; speed 300 -> 180 -> 171: 29 -> 20 -> 10.
    c.encumpct = 40;
    let f = build(&c, Some(&fast_weapon(300)), Move::Unused8);
    assert_eq!(f.crit, 1 + 10);
    c.encumpct = 67;
    assert_eq!(build(&c, Some(&fast_weapon(300)), Move::Unused8).crit, 1);
    c.encumpct = 10;
    let mut w = fast_weapon(310);
    w.strreq = 200;
    assert_eq!(build(&c, Some(&w), Move::Unused8).crit, 1);
    c.agi = 100; // +5: speed 400 -> 1200000... quick 250, divisor 2000: 400000/2000 = 200 -> 160: 45 -> 20
    let f = build(&c, Some(&fast_weapon(400)), Move::Unused8);
    assert_eq!(f.crit, 1 + 20);
}

// player_fighter: a cast this round (PlayerSettings::CANCAST cleared) costs
// 10% more energy, except for a backstab.
#[test]
fn cast_this_round_raises_energy() {
    let mut c = kang();
    c.settings = PlayerSettings::empty();
    assert_eq!(
        build(&c, Some(&quarterstaff()), Move::Unused8).energy,
        605 * 110 / 100
    );
    c.encumpct = 50;
    assert_eq!(
        build(&c, Some(&quarterstaff()), Move::Backstab).energy,
        1000
    );
}

// player_fighter: BASH doubles energy, SMASH spends the round;
// then the 200 floor and the maxenergy cap.
#[test]
fn bash_doubles_and_smash_spends_the_round() {
    assert_eq!(
        build(&kang(), Some(&quarterstaff()), Move::Bash).energy,
        1000
    );
    let mut c = kang();
    c.energyuse = 50; // 721 * 50/100 = 360 -> 302 -> 604
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Bash).energy, 604);
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Smash).energy, 1000);
    c.energyuse = 20; // 144 -> 120 -> 200
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Unused8).energy, 200);
    c.energyuse = 300;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Unused8).energy, 1000);
}

// player_fighter: knocked down, AV and DV lose the smash
// penalty, not below 0.
#[test]
fn knocked_down_loses_smash_penalty() {
    let mut c = kang();
    c.status2 = PlayerStatus2::KNOCKEDDOWN;
    let f = build(&c, Some(&quarterstaff()), Move::Attack);
    assert_eq!((f.av, f.dv), (58 - 35, 0));
}

// The Kang Witchunter sheet: AV 58 with the quarterstaff, 605
// energy per attack and 1000 per BASH, crit 1, dodge 3.
#[test]
fn kang_witchunter_sheet() {
    let f = build(&kang(), Some(&quarterstaff()), Move::Attack);
    assert_eq!(f.av, 58);
    assert_eq!(f.energy, 605);
    assert_eq!(f.crit, 1);
    assert_eq!(f.dodge, 3);
    assert_eq!(f.dv, 1);
    assert_eq!((f.mindmg, f.maxdmg), (2, 14));
    assert_eq!(f.hitverbs, MessageNo(8237));
    assert_eq!(f.missverbs, MessageNo(8289));
    assert_eq!(f.expval, 0);
    assert_eq!(
        build(&kang(), Some(&quarterstaff()), Move::Bash).energy,
        1000
    );
}

// The attack value's and dodge's stat terms: integer division truncates toward zero, so
// Agi 42 costs
// 1 AV ((42-50)/6 = -1) and Cha 30 costs 4 dodge ((30-50)/5 = -4).
#[test]
fn negative_division_breakpoints() {
    let mut c = kang();
    c.agi = 50;
    let agi50 = build(&c, Some(&quarterstaff()), Move::Attack);
    assert_eq!(
        build(&kang(), Some(&quarterstaff()), Move::Attack).av,
        agi50.av - 1
    );
    c.agi = 45;
    assert_eq!(build(&c, Some(&quarterstaff()), Move::Attack).av, agi50.av);
    c = kang();
    c.cha = 50;
    let cha50 = build(&c, Some(&quarterstaff()), Move::Attack);
    assert_eq!(
        build(&kang(), Some(&quarterstaff()), Move::Attack).dodge,
        cha50.dodge - 4
    );
    c.cha = 46; // -4/5 truncates to 0
    assert_eq!(
        build(&c, Some(&quarterstaff()), Move::Attack).dodge,
        cha50.dodge
    );
    c.cha = 45; // -5/5 is -1
    assert_eq!(
        build(&c, Some(&quarterstaff()), Move::Attack).dodge,
        cha50.dodge - 1
    );
}

// ---- monster fighter -------------------------------------------------------------------

/// A stock monster: `(level, ac, dr, exp, magicres, attack 0 (accuracy, min, max, energy))`.
fn stock(
    level: u32,
    ac: i16,
    dr: i16,
    exp: i32,
    magicres: i16,
    attack: (i16, i16, i16, i16),
) -> (ActiveMonster, KnownMonster) {
    let mut k = blank::known_monster();
    k.level = level;
    k.ac = ac;
    k.dr = dr;
    k.exp = exp;
    k.energy = 1000;
    k.magicres = magicres;
    k.alignment = Alignment::ChaoticEvil;
    k.attacks[0].accuracy = attack.0;
    k.attacks[0].min = attack.1;
    k.attacks[0].max = attack.2;
    k.attacks[0].energy = attack.3;
    let mut a = blank::active_monster();
    a.level = level;
    a.ac = ac;
    a.dr = dr;
    a.exp = exp;
    (a, k)
}

/// giant rat (WCCKNMSR 1): second attack slot accuracy 70, 4-20, 1000.
fn giant_rat() -> (ActiveMonster, KnownMonster) {
    let (a, mut k) = stock(1, 0, 1, 9, 30, (10, 2, 10, 1000));
    k.attacks[1].accuracy = 70;
    k.attacks[1].min = 4;
    k.attacks[1].max = 20;
    k.attacks[1].energy = 1000;
    k.bsdefense = 0;
    (a, k)
}

/// kobold thief (WCCKNMSR 7), wielding item 67.
fn kobold_thief() -> (ActiveMonster, KnownMonster) {
    stock(1, 10, 1, 13, 32, (15, 1, 8, 666))
}

/// acid slime (WCCKNMSR 5).
fn acid_slime() -> (ActiveMonster, KnownMonster) {
    stock(1, 0, 0, 16, 32, (20, 2, 9, 1000))
}

fn mon(a: &ActiveMonster, k: &KnownMonster, attack: usize, mv: Move) -> Fighter {
    monster_fighter(
        a,
        k,
        attack,
        None,
        MonsterAbilities::default(),
        Blow::new(mv, AlignGroup::Neutral),
    )
}

// monster_fighter: defence against the foe's alignment when
// the groups differ (against a good foe the evil value counts); Shadow Stealth +10.
#[test]
fn monster_defence_against_alignment() {
    let (a, k) = giant_rat(); // Chaotic Evil: the evil group
    let abil = MonsterAbilities {
        def_vs_good: 7,
        def_vs_evil: 3,
        shadow_stealth: true,
        ..MonsterAbilities::default()
    };
    let f = |style| monster_fighter(&a, &k, 0, None, abil, Blow::new(Move::Punch, style)).defalign;
    assert_eq!(f(AlignGroup::Good), 3 + 10);
    assert_eq!(f(AlignGroup::Neutral), 10);
    assert_eq!(f(AlignGroup::Evil), 10);
}

// monster_fighter: av = the attack slot's accuracy, plus the three
// "Alter AV" abilities.
#[test]
fn monster_attack_value() {
    let (a, k) = giant_rat();
    assert_eq!(mon(&a, &k, 0, Move::Punch).av, 10);
    assert_eq!(mon(&a, &k, 1, Move::Punch).av, 70);
    let abil = MonsterAbilities {
        av_bless: 5,
        av_curse: -3,
        av_third: 1,
        ..MonsterAbilities::default()
    };
    let f = monster_fighter(
        &a,
        &k,
        1,
        None,
        abil,
        Blow::new(Move::Punch, AlignGroup::Neutral),
    );
    assert_eq!(f.av, 73);
}

// monster_fighter: dv = ac, or against a backstab
// (ac/2 + bsdefense*2)/2; plus "Alter Defence Value".
#[test]
fn monster_defence_value() {
    let (a, mut k) = kobold_thief();
    k.bsdefense = 7;
    assert_eq!(mon(&a, &k, 0, Move::Punch).dv, 10);
    assert_eq!(mon(&a, &k, 0, Move::Backstab).dv, (5 + 14) / 2);
    let abil = MonsterAbilities {
        defence: 4,
        ..MonsterAbilities::default()
    };
    let f = monster_fighter(
        &a,
        &k,
        0,
        None,
        abil,
        Blow::new(Move::Punch, AlignGroup::Neutral),
    );
    assert_eq!(f.dv, 14);
}

// monster_fighter: dr = dr*10 (tenths), plus "Alter Damage
// Resistance"; expval = exp*level; energy, min and max of the slot,
// both ends plus "Alter Damage".
#[test]
fn monster_dr_exp_energy_and_range() {
    let (mut a, k) = giant_rat();
    a.level = 3;
    a.exp = 9;
    let f = mon(&a, &k, 0, Move::Punch);
    assert_eq!(f.dr, 10);
    assert_eq!(f.expval, 27);
    assert_eq!((f.energy, f.mindmg, f.maxdmg), (1000, 2, 10));
    let abil = MonsterAbilities {
        damage: 2,
        damage_resist: 5,
        ..MonsterAbilities::default()
    };
    let f = monster_fighter(
        &a,
        &k,
        1,
        None,
        abil,
        Blow::new(Move::Punch, AlignGroup::Neutral),
    );
    assert_eq!((f.dr, f.mindmg, f.maxdmg), (15, 6, 22));
}

// monster_fighter: monsters dodge only through the Dodge
// ability and never critical; the Defence Modifier.
#[test]
fn monster_dodge_only_through_the_ability() {
    let (a, k) = giant_rat();
    let f = mon(&a, &k, 0, Move::Punch);
    assert_eq!((f.dodge, f.crit, f.defmod), (0, 0, 0));
    let abil = MonsterAbilities {
        dodge: 12,
        defence_mod: -4,
        ..MonsterAbilities::default()
    };
    let f = monster_fighter(
        &a,
        &k,
        0,
        None,
        abil,
        Blow::new(Move::Punch, AlignGroup::Neutral),
    );
    assert_eq!((f.dodge, f.crit, f.defmod), (12, 0, -4));
}

// monster_fighter: "Alter Energy Usage" scales the cost by a
// percentage, never above the monster's energy per round.
#[test]
fn monster_energy_use_ability() {
    let (a, k) = kobold_thief(); // slot energy 666, 1000 per round
    let f = |pct| {
        let abil = MonsterAbilities {
            energyuse: pct,
            ..MonsterAbilities::default()
        };
        monster_fighter(
            &a,
            &k,
            0,
            None,
            abil,
            Blow::new(Move::Punch, AlignGroup::Neutral),
        )
        .energy
    };
    assert_eq!(f(0), 666);
    assert_eq!(f(50), 333);
    assert_eq!(f(200), 1000);
    // A negative value is a huge unsigned percentage: the cap brings it to the round.
    assert_eq!(f(-10), 1000);
}

// monster_fighter: ActiveMonsterFlags::ATKPEN takes the smash penalty off AV
// and DV, not below 0.
#[test]
fn monster_attack_penalty_flag() {
    let (mut a, k) = giant_rat();
    a.flags = ActiveMonsterFlags::ATKPEN;
    let f = mon(&a, &k, 1, Move::Punch);
    assert_eq!((f.av, f.dv), (70 - 35, 0));
}

// monster_fighter: the hit and miss verbs come from the
// wielded weapon's messages; none without a weapon.
#[test]
fn monster_verbs_from_its_weapon() {
    let (a, k) = kobold_thief();
    let mut w = blank::item();
    w.hitmsg = MessageNo(9001);
    w.missmsg = MessageNo(9002);
    let f = monster_fighter(
        &a,
        &k,
        0,
        Some(&w),
        MonsterAbilities::default(),
        blow(Move::Punch),
    );
    assert_eq!(
        (f.hitverbs, f.missverbs),
        (MessageNo(9001), MessageNo(9002))
    );
    assert_eq!(mon(&a, &k, 0, Move::Punch).hitverbs, MessageNo(0));
}

// The three stock monsters as fighters (WCCKNMSR 1, 7, 5) against a plain blow.
#[test]
fn stock_monster_sheets() {
    let (a, k) = giant_rat();
    let f = mon(&a, &k, 0, Move::Punch);
    assert_eq!(
        (f.av, f.dv, f.dr, f.expval, f.energy, f.mindmg, f.maxdmg),
        (10, 0, 10, 9, 1000, 2, 10)
    );
    let (a, k) = kobold_thief();
    let f = mon(&a, &k, 0, Move::Punch);
    assert_eq!(
        (f.av, f.dv, f.dr, f.expval, f.energy, f.mindmg, f.maxdmg),
        (15, 10, 10, 13, 666, 1, 8)
    );
    let (a, k) = acid_slime();
    let f = mon(&a, &k, 0, Move::Punch);
    assert_eq!(
        (f.av, f.dv, f.dr, f.expval, f.energy, f.mindmg, f.maxdmg),
        (20, 0, 0, 16, 1000, 2, 9)
    );
}

// ---- attack resolution -----------------------------------------------------------------

// resolve: the move table.
#[test]
fn move_table() {
    let row = |m| {
        let r = move_mods(m);
        (r.hitmod, r.dmgpct, r.can_crit)
    };
    assert_eq!(row(Move::Attack), (0, 0, true));
    assert_eq!(row(Move::Punch), (0, 0, true));
    assert_eq!(row(Move::Kick), (0, 33, true));
    assert_eq!(row(Move::JumpKick), (0, 66, true));
    assert_eq!(row(Move::Bash), (-15, 10, false));
    assert_eq!(row(Move::Smash), (-25, 20, false));
    assert_eq!(row(Move::Unused8), (-75, 125, false));
    assert_eq!(row(Move::Backstab), (0, 0, false));
}

// resolve: the defender's defmod is added to the hit modifier.
#[test]
fn defender_defmod_joins_hitmod() {
    // AV 58 vs DV 30 hits at 63; a defmod of -20 makes atkpower 38^2/14/10 = 10 and the
    // chance 100 - 900/10 = 10.
    let mut atk = fighter(58, 0);
    let mut def = fighter(0, 30);
    def.defmod = -20;
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[10]));
    assert_eq!(r.outcome, Outcome::Miss);
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[9, 99, 0]));
    assert!(matches!(r.outcome, Outcome::Hit { .. }));
}

// resolve: the move's percent raises both ends of the
// attacker's range, in place.
#[test]
fn move_percent_scales_range() {
    let mut atk = fighter(100, 0); // 10-20
    let def = fighter(0, 0);
    // Kick: 13-26; the damage roll 0 gives the minimum.
    let r = resolve(&mut atk, &def, Move::Kick, &mut FixedRng::new(&[1, 99, 0]));
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 13,
            critical: false
        }
    );
    assert_eq!((atk.mindmg, atk.maxdmg), (13, 26));
    // JumpKick: 16-33; roll 17 gives the maximum.
    let mut atk = fighter(100, 0);
    let r = resolve(
        &mut atk,
        &def,
        Move::JumpKick,
        &mut FixedRng::new(&[1, 99, 17]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 33,
            critical: false
        }
    );
}

// resolve: the hit roll is genrdn(1,100), 1..=99; the blow
// lands when the roll is under the chance.
#[test]
fn hit_when_roll_under_chance() {
    let mut atk = fighter(58, 0);
    let def = fighter(0, 30); // 63
    let r = resolve(
        &mut atk,
        &def,
        Move::Punch,
        &mut FixedRng::new(&[62, 99, 0]),
    );
    assert!(matches!(r.outcome, Outcome::Hit { .. }));
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[63]));
    assert_eq!(r.outcome, Outcome::Miss);
}

// resolve: a defender with negative dodge is hit at 99 when
// genrdn(0,100) > dodge+100.
#[test]
fn negative_dodge_hits_at_99() {
    let mut atk = fighter(20, 0); // atkpower 400/14/10 = 2, vs DV 30: 100-450 -> 10
    let mut def = fighter(0, 30);
    def.dodge = -25;
    // Hit roll 98, negative-dodge roll 76 > 75: chance 99, so 98 hits.
    let r = resolve(
        &mut atk,
        &def,
        Move::Punch,
        &mut FixedRng::new(&[98, 76, 99, 0]),
    );
    assert!(matches!(r.outcome, Outcome::Hit { .. }));
    // Roll 75 is not over 75: the formula's 10 applies and 98 misses.
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[98, 75]));
    assert_eq!(r.outcome, Outcome::Miss);
}

// resolve: a backstab's hit chance is av - dv.
#[test]
fn backstab_hit_chance_is_av_minus_dv() {
    assert_eq!(hit_chance(80, 0, 35, 0, true), 45);
    assert_eq!(hit_chance(80, -15, 35, 10, true), 45);
    let mut atk = fighter(80, 0);
    let def = fighter(0, 35);
    let r = resolve(&mut atk, &def, Move::Backstab, &mut FixedRng::new(&[45]));
    assert_eq!(r.outcome, Outcome::Miss);
}

// resolve: atkpower = (av+hitmod)^2/14/10 in two divisions,
// hitchance = 100 - (dv+defalign)^2/atkpower. Kang: AV 58 vs DV 30 is 63, vs DV 40 is 34
// (the KB's `100 - 140*DV^2/AV^2` rounds that one to 33); vs the giant rat's DV 0, 99.
#[test]
fn hit_chance_formula_two_divisions() {
    assert_eq!(hit_chance(58, 0, 30, 0, false), 63);
    assert_eq!(hit_chance(58, 0, 40, 0, false), 34);
    assert_eq!(hit_chance(58, 0, 0, 0, false), 99);
    // defalign joins DV before the square.
    assert_eq!(hit_chance(58, 0, 30, 10, false), 34);
    // BASH: 43^2 = 1849, /14 = 132, /10 = 13; 900/13 = 69 -> 31.
    assert_eq!(hit_chance(58, -15, 30, 0, false), 31);
}

// resolve: hitchance is 5 (then clamped to 10) when atkpower
// is 0: the giant rat's AV 10 against anyone.
#[test]
fn zero_attack_power_gives_the_floor() {
    assert_eq!(hit_chance(11, 0, 0, 0, false), 10);
    assert_eq!(hit_chance(10, 0, 1, 0, false), 10);
    assert_eq!(hit_chance(0, 0, 50, 0, false), 10);
}

// resolve: the chance is clamped to 10..=99.
#[test]
fn hit_chance_clamped_10_to_99() {
    assert_eq!(hit_chance(300, 0, 1, 0, false), 99);
    assert_eq!(hit_chance(10, 0, 90, 0, false), 10);
    assert_eq!(hit_chance(10, 0, 90, 0, true), 10);
    assert_eq!(hit_chance(300, 0, 1, 0, true), 99);
}

// resolve: over 40 only a third of the excess crit counts,
// written back to the attacker.
#[test]
fn crit_over_40_compressed() {
    assert_eq!(effective_crit(40), 40);
    assert_eq!(effective_crit(41), 40);
    assert_eq!(effective_crit(46), 42);
    assert_eq!(effective_crit(75), 51);
    assert_eq!(effective_crit(5), 5);
    let mut atk = fighter(100, 0);
    atk.crit = 75;
    resolve(
        &mut atk,
        &fighter(0, 0),
        Move::Punch,
        &mut FixedRng::new(&[99]),
    );
    assert_eq!(atk.crit, 51);
}

// resolve: a critical (genrdn(0,100) < crit) sets
// min = max*2 and max *= 4; never for BASH or SMASH, whose crit is zeroed.
#[test]
fn critical_doubles_and_quadruples() {
    let mut atk = fighter(100, 0); // 10-20
    atk.crit = 30;
    let def = fighter(0, 0);
    // Rolls: hit 1, crit 29 (< 30), damage 0 -> min 40.
    let r = resolve(
        &mut atk.clone(),
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 29, 0]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 40,
            critical: true
        }
    );
    // Damage roll 40 -> max 80.
    let r = resolve(
        &mut atk.clone(),
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 29, 40]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 80,
            critical: true
        }
    );
    // Crit roll 30 is not under 30.
    let r = resolve(
        &mut atk.clone(),
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 30, 0]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 10,
            critical: false
        }
    );
    // BASH and SMASH zero the chance: the crit roll is made but can never be under 0.
    let mut bash = atk.clone();
    let r = resolve(&mut bash, &def, Move::Bash, &mut FixedRng::new(&[1, 0, 0]));
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 33,
            critical: false
        }
    );
    assert_eq!(bash.crit, 0);
    let r = resolve(
        &mut atk.clone(),
        &def,
        Move::Smash,
        &mut FixedRng::new(&[1, 0, 0]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 60,
            critical: false
        }
    );
}

// resolve: an inverted range raises the maximum to the
// minimum.
#[test]
fn inverted_range_is_fixed() {
    let mut atk = fighter(100, 0);
    atk.mindmg = 30;
    atk.maxdmg = 20;
    let def = fighter(0, 0);
    // genrdn(0, 1) only ever gives 0, so the scripted 0 is the sole legal roll.
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[1, 99, 0]));
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 30,
            critical: false
        }
    );
    assert_eq!((atk.mindmg, atk.maxdmg), (30, 30));
}

// resolve: damage = genrdn(0, max-min+1) + min - dr/10.
#[test]
fn damage_roll_less_tenth_of_dr() {
    let mut atk = fighter(100, 0); // 10-20
    let mut def = fighter(0, 0);
    def.dr = 35;
    let r = resolve(
        &mut atk,
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 99, 10]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 20 - 3,
            critical: false
        }
    );
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[1, 99, 0]));
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 10 - 3,
            critical: false
        }
    );
}

// resolve: dodge chance dodge*10/(av/8) when av > 8, capped
// at 95, a fifth against a backstab.
#[test]
fn dodge_chance_formula() {
    assert_eq!(dodge_chance(22, 58, false), 220 / 7);
    assert_eq!(dodge_chance(22, 8, false), 0);
    assert_eq!(dodge_chance(22, 9, false), 95);
    assert_eq!(dodge_chance(22, 58, true), 220 / 7 / 5);
    assert_eq!(dodge_chance(22, 9, true), 19);
    // Kang's dodge 3 against the giant rat's second attack (AV 70): 30/8 = 3%.
    assert_eq!(dodge_chance(3, 70, false), 3);
}

// resolve: BASH x3 and SMASH x5 after damage reduction.
#[test]
fn bash_triples_and_smash_quintuples_after_dr() {
    let mut atk = fighter(100, 0); // BASH 11-22, SMASH 12-24
    let mut def = fighter(0, 0);
    def.dr = 40;
    let r = resolve(
        &mut atk.clone(),
        &def,
        Move::Bash,
        &mut FixedRng::new(&[1, 99, 0]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: (11 - 4) * 3,
            critical: false
        }
    );
    let r = resolve(
        &mut atk,
        &def,
        Move::Smash,
        &mut FixedRng::new(&[1, 99, 12]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: (24 - 4) * 5,
            critical: false
        }
    );
}

// resolve: dodged when dodge > 0 and genrdn(0,100) is under
// the dodge chance.
#[test]
fn dodged_blow() {
    let mut atk = fighter(58, 0);
    let mut def = fighter(0, 0);
    def.dodge = 22; // 31%
    let r = resolve(
        &mut atk,
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 99, 0, 30]),
    );
    assert_eq!(r.outcome, Outcome::Dodged);
    assert_eq!((r.rolled, r.exp), (0, 0));
    let r = resolve(
        &mut atk,
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 99, 0, 31]),
    );
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 10,
            critical: false
        }
    );
    // No dodge roll at all without positive dodge.
    def.dodge = 0;
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[1, 99, 0]));
    assert_eq!(
        r.outcome,
        Outcome::Hit {
            damage: 10,
            critical: false
        }
    );
}

// resolve: nothing through the armour is NoDamage with
// no experience; a real hit copies the defender's expval.
#[test]
fn no_damage_when_dr_absorbs_it() {
    let mut atk = fighter(100, 0);
    let mut def = fighter(0, 0);
    def.dr = 200;
    def.expval = 27;
    let r = resolve(
        &mut atk,
        &def,
        Move::Punch,
        &mut FixedRng::new(&[1, 99, 10]),
    );
    assert_eq!((r.outcome, r.rolled, r.exp), (Outcome::NoDamage, 0, 0));
    def.dr = 100;
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[1, 99, 0]));
    assert_eq!(r.outcome, Outcome::NoDamage);
    def.dr = 90;
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[1, 99, 0]));
    assert_eq!(
        (r.outcome, r.rolled, r.exp),
        (
            Outcome::Hit {
                damage: 1,
                critical: false
            },
            1,
            27
        )
    );
}

// AttackResult: energy is the low word of the attacker's energy; rolled
// is the damage shown; poison is always 0.
#[test]
fn result_carries_energy_rolled_and_poison() {
    let mut atk = fighter(58, 0);
    atk.energy = 605;
    let def = fighter(0, 30);
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[70]));
    assert_eq!(
        (r.outcome, r.rolled, r.energy, r.exp, r.poison),
        (Outcome::Miss, 0, 605, 0, 0)
    );
    atk.energy = 70_000;
    let r = resolve(&mut atk, &def, Move::Punch, &mut FixedRng::new(&[70]));
    assert_eq!(r.energy, 70_000_u32 as u16);
}

// resolve: the random move and percentage have no effect, so two blows with the
// same rolls give the same result.
#[test]
fn resolution_is_a_pure_function_of_the_rolls() {
    let atk = fighter(58, 0);
    let def = fighter(0, 30);
    let a = resolve(
        &mut atk.clone(),
        &def,
        Move::Punch,
        &mut FixedRng::new(&[20, 99, 3]),
    );
    let b = resolve(
        &mut atk.clone(),
        &def,
        Move::Punch,
        &mut FixedRng::new(&[20, 99, 3]),
    );
    assert_eq!(a, b);
}

// ---- the rest of the module ------------------------------------------------------------

// attack_user_monster: knockdown when str+level > mon.level + magicres + 30, unsigned;
// SMASH_ROUNDS 15, SMASH_PENALTY 35.
#[test]
fn smash_knockdown_predicate() {
    assert!(smash_knockdown(79, 1, 1, 30)); // Kang vs the giant rat: 80 > 61
    assert!(!smash_knockdown(60, 5, 20, 15)); // 65 > 65 is false
    assert!(smash_knockdown(61, 5, 20, 15));
    // The comparison is unsigned: a "negative" resistance is huge.
    assert!(!smash_knockdown(300, 50, 1, -100));
    // Only the low word of the monster's level counts.
    assert!(smash_knockdown(79, 1, 0x1_0001, 30));
    assert_eq!(SMASH_ROUNDS, 15);
    assert_eq!(SMASH_PENALTY, 35);
}

// monster_align_group: Neutral Evil and unknown codes are neutral.
#[test]
fn monster_alignment_groups() {
    assert_eq!(monster_align_group(Alignment::Good), AlignGroup::Good);
    assert_eq!(monster_align_group(Alignment::LawfulGood), AlignGroup::Good);
    assert_eq!(monster_align_group(Alignment::Neutral), AlignGroup::Neutral);
    assert_eq!(
        monster_align_group(Alignment::NeutralEvil),
        AlignGroup::Neutral
    );
    assert_eq!(
        monster_align_group(Alignment::Unknown(9)),
        AlignGroup::Neutral
    );
    assert_eq!(monster_align_group(Alignment::Evil), AlignGroup::Evil);
    assert_eq!(
        monster_align_group(Alignment::ChaoticEvil),
        AlignGroup::Evil
    );
    assert_eq!(monster_align_group(Alignment::LawfulEvil), AlignGroup::Evil);
}

// user_align_group over the legal levels: Good and
// Saint are good, Outlaw to FIEND evil, Neutral and Seedy neutral.
#[test]
fn user_alignment_groups() {
    assert_eq!(user_align_group(0), AlignGroup::Neutral);
    assert_eq!(user_align_group(-50), AlignGroup::Neutral);
    assert_eq!(user_align_group(29), AlignGroup::Neutral);
    assert_eq!(user_align_group(39), AlignGroup::Neutral); // Seedy
    assert_eq!(user_align_group(40), AlignGroup::Evil); // Outlaw
    assert_eq!(user_align_group(210), AlignGroup::Evil); // FIEND
    assert_eq!(user_align_group(-51), AlignGroup::Good);
    assert_eq!(user_align_group(-201), AlignGroup::Good); // Saint
}

// attack_user_monster: Hit Magical covers the requirement first; the
// weapon's Magical makes up the rest; hands and feet never do.
#[test]
fn can_hit_subtracts_hit_magical() {
    assert!(can_hit(0, 0, None));
    assert!(!can_hit(0, 1, None));
    assert!(can_hit(1, 1, None));
    assert!(can_hit(10, 2, None)); // the Witchunter's Hit Magical 10
    assert!(!can_hit(3, 5, Some(1)));
    assert!(can_hit(3, 5, Some(2)));
    assert!(!can_hit(0, 5, Some(4)));
}

// genrdn(lo, hi) is uniform on [lo, hi), never hi (genrdn(1,10) gives 1-9), as MBBSEmu's
// `genrnd` (max exclusive) and `Rng::range` have it: the damage roll genrdn(0, max-min+1)
// lands in min..=max.
#[test]
fn genrdn_upper_bound_is_exclusive() {
    let mut rng = SeededRng::new(31);
    let mut seen = [false; 10];
    for _ in 0..5_000 {
        let v = rng.range(1, 10);
        assert!((1..10).contains(&v));
        seen[v as usize] = true;
    }
    assert_eq!(
        seen,
        [false, true, true, true, true, true, true, true, true, true]
    );
    for _ in 0..1_000 {
        assert!((1..100).contains(&rng.range(1, 100)));
        assert!(rng.pct() < 100);
    }
}

// Record builders for the tests: every field at its zero.
mod blank {
    use silvermere_types::enums::*;
    use silvermere_types::flags::*;
    use silvermere_types::ids::*;
    use silvermere_types::records::*;

    pub fn character() -> Character {
        Character {
            userid: String::new(),
            firstname: String::new(),
            lastname: String::new(),
            exp: 0,
            spells: std::array::from_fn(|_| SpellNo::default()),
            spellvals: std::array::from_fn(|_| 0),
            spellrnds: std::array::from_fn(|_| 0),
            title: String::new(),
            race: RaceNo::default(),
            class: ClassNo::default(),
            level: 0,
            basint: 0,
            baswil: 0,
            basstr: 0,
            bashea: 0,
            basagi: 0,
            bascha: 0,
            intel: 0,
            wil: 0,
            str: 0,
            hea: 0,
            agi: 0,
            cha: 0,
            maxhp: 0,
            hp: 0,
            maxwgt: 0,
            wgt: 0,
            maxenergy: 0,
            energy: 0,
            poison: 0,
            maxmr: 0,
            mr: 0,
            map: MapId::default(),
            room: RoomId::default(),
            light: 0,
            hunger: 0,
            thirst: 0,
            ninvslots: 0,
            items: Vec::new(),
            itemuses: Vec::new(),
            nkeyslots: 0,
            keys: Vec::new(),
            keyuses: Vec::new(),
            qflags2: 0,
            hpbonus: 0,
            manabonus: 0,
            expbil: 0,
            exprem: 0,
            nspellslots: 0,
            spellbook: Vec::new(),
            evil: 0,
            lawful: 0,
            trailmaps: std::array::from_fn(|_| MapId::default()),
            trailrooms: std::array::from_fn(|_| RoomId::default()),
            verbose: 0,
            bchannel: 0,
            fasttalk: 0,
            statline: StatlineMode::Off,
            hidden: 0,
            perception: 0,
            stealth: 0,
            martial: 0,
            thievery: 0,
            maxmana: 0,
            mana: 0,
            spellcast: 0,
            traps: 0,
            disarm: 0,
            picklocks: 0,
            tracking: 0,
            coins: Coins::default(),
            weapon: ItemNo::default(),
            worn: std::array::from_fn(|_| ItemNo::default()),
            wornuses: std::array::from_fn(|_| 0),
            autoengage: 0,
            lives: 0,
            aided: 0,
            rank: 0,
            lighttime: 0,
            readied: 0,
            delay: 0,
            roomlight: 0,
            gang: String::new(),
            hairlen: 0,
            haircol: 0,
            eyecol: 0,
            paging: 0,
            cp: 0,
            suicpw: String::new(),
            attackers: 0,
            status: PlayerStatus::empty(),
            movedir: 0,
            dragging: 0,
            totalcp: 0,
            immortal: 0,
            pagecnt: 0,
            pagemax: 0,
            settings: PlayerSettings::empty(),
            gossmax: 0,
            auctmax: 0,
            gosscnt: 0,
            auctcnt: 0,
            encumpct: 0,
            avbonus: 0,
            acbonus: 0,
            dmgbonus: 0,
            mabase: 0,
            arenahp: 0,
            qflags: 0,
            version: 0,
            msglevel: MessageLevel::High,
            pvpcnt: 0,
            randomhp: 0,
            abils: std::array::from_fn(|_| (Ability::None, 0)),
            resttick: 0,
            critbonus: 0,
            drbonus: 0,
            drpct: 0,
            energyuse: 0,
            shield: 0,
            defmod: 0,
            sesstime: 0,
            lifemins: 0,
            moves: 0,
            lastdate: 0,
            status2: PlayerStatus2::empty(),
            palette: 0,
            deathdate: 0,
            flags: PlayerFlags::empty(),
            sex: 0,
            healrate: 0,
            manarate: 0,
            racechg: 0,
            lastmon: 0,
            lastmoncnt: 0,
            pvptimer: 0,
            medtick: 0,
        }
    }

    pub fn class() -> Class {
        Class {
            number: ClassNo::default(),
            name: String::new(),
            basehp: 0,
            hplevel: 0,
            exp: 0,
            attmult: 0,
            maxenergy: 0,
            restored: 0,
            abilities: std::array::from_fn(|_| (Ability::None, 0)),
            magery: Magery::None,
            magelvl: 0,
            weapon: WeaponClass::OneHandBlunt,
            armour: ArmourType::Natural,
            combat: 0,
            titles: TextBlockNo::default(),
        }
    }

    pub fn item() -> Item {
        Item {
            number: ItemNo::default(),
            level: 0,
            limit: 0,
            name: String::new(),
            desc: std::array::from_fn(|_| String::new()),
            hiddesc: std::array::from_fn(|_| String::new()),
            active: 0,
            weight: 0,
            item_type: ItemType::Armour,
            abilities: std::array::from_fn(|_| (Ability::None, 0)),
            charges: 0,
            energy: 0,
            cost: 0,
            classes: std::array::from_fn(|_| ClassNo::default()),
            mindmg: 0,
            maxdmg: 0,
            ac: 0,
            races: std::array::from_fn(|_| RaceNo::default()),
            negates: std::array::from_fn(|_| SpellNo::default()),
            wpntype: WeaponType::OneHandBlunt,
            armtype: ArmourType::Natural,
            worn: WornLocation::Nowhere,
            accuracy: 0,
            dr: 0,
            gettable: 0,
            strreq: 0,
            cash: Coins::default(),
            speed: 0,
            hitmsg: MessageNo::default(),
            missmsg: MessageNo::default(),
            textnum: TextBlockNo::default(),
            destroymsg: MessageNo::default(),
            nodrop: 0,
            currency: Currency::Copper,
            retain: 0,
            robable: 0,
            destroy: 0,
        }
    }

    pub fn known_monster() -> KnownMonster {
        KnownMonster {
            number: MonsterNo::default(),
            name: String::new(),
            montype: MonsterType::Lair,
            level: 0,
            genindex: 0,
            wornitem: ItemNo::default(),
            weapon: ItemNo::default(),
            dr: 0,
            ac: 0,
            group: 0,
            followpct: 0,
            magicres: 0,
            bsdefense: 0,
            exp: 0,
            maxhp: 0,
            energy: 0,
            hpregen: 0,
            abilities: std::array::from_fn(|_| (Ability::None, 0)),
            maxalive: 0,
            curalive: 0,
            grouprole: GroupRole::Solo,
            maxfollow: 0,
            undead: 0,
            alignment: Alignment::Neutral,
            regenhrs: 0,
            killed_at: 0,
            arrivemsg: MessageNo::default(),
            deathmsg: MessageNo::default(),
            items: std::array::from_fn(|_| MonsterItem {
                item: ItemNo::default(),
                uses: 0,
                pct: 0,
            }),
            cash: Coins::default(),
            talktext: TextBlockNo::default(),
            charmlvl: 0,
            namelist: 0,
            attacks: std::array::from_fn(|_| monster_attack()),
            randtext: TextBlockNo::default(),
            charmres: 0,
            deathspell: SpellNo::default(),
            createspell: SpellNo::default(),
            spells: std::array::from_fn(|_| MonsterSpell {
                spell: SpellNo::default(),
                pct: 0,
                level: 0,
            }),
            desc: std::array::from_fn(|_| String::new()),
            gender: Gender::It,
        }
    }

    pub fn active_monster() -> ActiveMonster {
        ActiveMonster {
            number: ActiveMonsterId::default(),
            knownnum: MonsterNo::default(),
            level: 0,
            map: MapId::default(),
            room: RoomId::default(),
            poison: 0,
            energy: 0,
            hp: 0,
            following: String::new(),
            trail: std::array::from_fn(|_| RoomId::default()),
            hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
            hunting: ActiveMonsterId::default(),
            name: String::new(),
            wornitem: ItemNo::default(),
            weapon: ItemNo::default(),
            items: std::array::from_fn(|_| (ItemNo::default(), 0)),
            cash: Coins::default(),
            maxhp: 0,
            alignment: Alignment::Neutral,
            followpct: 0,
            dr: 0,
            ac: 0,
            exp: 0,
            maxenergy: 0,
            friendly: 0,
            genroom: RoomId::default(),
            inactive: 0,
            flags: ActiveMonsterFlags::empty(),
            montype: MonsterType::Lair,
            target: 0,
            hpregen: 0,
            lastdir: 0,
            lastupd: 0,
            grouprole: GroupRole::Solo,
            spells: std::array::from_fn(|_| ActiveSpell {
                spell: SpellNo::default(),
                value: 0,
                rounds: 0,
            }),
            downrounds: 0,
            created: 0,
        }
    }

    pub fn monster_attack() -> MonsterAttack {
        MonsterAttack {
            kind: MonsterAttackType::None,
            accuracy: 0,
            pct: 0,
            min: 0,
            max: 0,
            hitmsg: MessageNo::default(),
            defmsg: MessageNo::default(),
            missmsg: MessageNo::default(),
            energy: 0,
            hitspell: SpellNo::default(),
        }
    }
}
