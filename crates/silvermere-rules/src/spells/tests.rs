//! Layer 1 tests for `spells`: each test pins one behaviour of the spell engines.

use proptest::prelude::*;
use silvermere_types::enums::{
    Ability, Alignment, ArmourType, Currency, Element, Gender, GroupRole, ItemType, Magery,
    MessageLevel, MonsterAttackType, MonsterType, ResistMode, ResistStat, SpellTarget, SpellType,
    StatlineMode, WeaponClass, WeaponType, WornLocation,
};
use silvermere_types::flags::{
    ActiveMonsterFlags, PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2, SpellFlags,
};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, SpellNo};
use silvermere_types::records::{
    ActiveMonster, ActiveSpell, Character, Class, Coins, Item, KnownMonster, MonsterAttack,
    MonsterItem, MonsterSpell, Spell,
};
use silvermere_types::{
    NABILITY, NAMTRAIL, NMONATTACKS, NMONITEMS, NMONSPELLS, NSPELLCAST, NTRAIL, NWORN,
};

use super::apply::{
    Cast, CastPath, CastState, CasterStats, EndedSpell, ItemTarget, MonsterTarget, Outcome,
    PoisonCut, RoundOutcome, SummonDisposition, Target, UserTarget, apply_effect,
    apply_monster_round, apply_monster_termination, apply_round, apply_spell, apply_termination,
    cut_poison, gain_stolen_hp, monster_gate,
};
use super::apply_monster::{
    AreaOutcome, MonsterCast, MonsterCastState, MonsterCaster, RoomMonsterState,
    apply_monster_area_remove_pass, apply_monster_cast, apply_monster_cast_area,
    apply_monster_cast_area_spell, apply_monster_cast_spell, apply_monster_self_spell,
    apply_room_on_monster, apply_room_on_monster_spell, area_no_effect, monster_amount_range,
    monster_duration, player_resists_monster_cast, player_resists_room_cast, roll_monster_amount,
    room_charm_resistance, room_monster_power,
};
use super::effect::{SpellEffect, SpellEffects, Stat, TeleportPart};
use super::resist::{
    apply_element_resistance, element_resistance, item_negates, may_resist,
    monster_magic_resistance, monster_mr_cut_pct, monster_mr_damage, monster_resists_area,
    monster_resists_single, mr_cut_pct, mr_damage, player_resists, resist_chance, room_mr_damage,
    spell_immune,
};
use super::{
    AmountRange, CannotUse, CasterLevel, CharacterSlots, LegalBand, MAXCHANCE, MonsterCandidate,
    PlayerCandidate, SlotOutcome, TargetKind, TargetKindExt, ValidTarget, add_spell_damage_bonus,
    add_spell_to_character, add_spell_to_monster, amount_range, can_use_spell, capped_level,
    duration, is_valid_monster_target, is_valid_target, match_kinds,
    monster_add_spell_to_character, per_target_share, roll_amount, scale_spell_amount,
    spell_can_hit_monster,
};
use crate::abilities::CharacterAbilities;
use crate::rng::{Rng, SeededRng, chi_square_critical_001, chi_square_uniform};

// ---------------------------------------------------------------------------------------
// Fixtures: blank records (every field 0, empty or `None`) with the slots the test needs.

const NONE: (Ability, i16) = (Ability::None, 0);

/// A random source that hands out a fixed sequence (then repeats its last value), so a
/// test can pin a roll at a boundary.
struct Fixed {
    values: Vec<i32>,
    next: usize,
}

impl Fixed {
    fn of(values: &[i32]) -> Self {
        Self {
            values: values.to_vec(),
            next: 0,
        }
    }
}

impl Rng for Fixed {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let v = self.values[self.next.min(self.values.len() - 1)];
        self.next += 1;
        if hi <= lo {
            return lo;
        }
        v.clamp(lo, hi - 1)
    }

    fn long(&mut self, lo: i64, hi: i64) -> i64 {
        i64::from(self.range(lo as i32, hi as i32))
    }
}

fn slots<const N: usize>(given: &[(Ability, i16)]) -> [(Ability, i16); N] {
    let mut out = [NONE; N];
    out[..given.len()].copy_from_slice(given);
    out
}

fn character() -> Character {
    Character {
        userid: String::new(),
        firstname: String::new(),
        lastname: String::new(),
        exp: 0,
        spells: [SpellNo(0); NSPELLCAST],
        spellvals: [0; NSPELLCAST],
        spellrnds: [0; NSPELLCAST],
        title: String::new(),
        race: 0.into(),
        class: 0.into(),
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
        map: 0.into(),
        room: 0.into(),
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
        trailmaps: [0.into(); NTRAIL],
        trailrooms: [0.into(); NTRAIL],
        verbose: 0,
        bchannel: 0,
        fasttalk: 0,
        statline: StatlineMode::from_code(0),
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
        weapon: ItemNo(0),
        worn: [ItemNo(0); NWORN],
        wornuses: [0; NWORN],
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
        msglevel: MessageLevel::from_code(0),
        pvpcnt: 0,
        randomhp: 0,
        abils: [NONE; NABILITY],
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

fn class(abilities: &[(Ability, i16)]) -> Class {
    Class {
        number: 0.into(),
        name: String::new(),
        basehp: 0,
        hplevel: 0,
        exp: 0,
        attmult: 0,
        maxenergy: 0,
        restored: 0,
        abilities: slots(abilities),
        magery: Magery::from_code(0),
        magelvl: 0,
        weapon: WeaponClass::from_code(0),
        armour: ArmourType::from_code(0),
        combat: 0,
        titles: 0.into(),
    }
}

fn spell(number: i16, abilities: &[(Ability, i16)]) -> Spell {
    Spell {
        number: SpellNo(number),
        name: String::new(),
        descs: [String::new(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities: slots(abilities),
        energy: 0,
        level: 0,
        minval: 0,
        maxval: 0,
        spell_type: SpellType::from_code(0),
        resmode: ResistMode::from_code(0),
        diff: 0,
        durlvl: 0,
        target: SpellTarget::from_code(0),
        duration: 0,
        element: Element::from_code(0),
        resstat: ResistStat::from_code(0),
        magery: Magery::from_code(0),
        msgnum: 0.into(),
        mana: 0,
        maxinc: 0,
        maxlvls: 0,
        magelvl: 0,
        mininc: 0,
        minlvls: 0,
        durinc: 0,
        durlvls: 0,
        shortnm: String::new(),
    }
}

fn item(
    number: i32,
    item_type: ItemType,
    worn: WornLocation,
    abilities: &[(Ability, i16)],
) -> Item {
    Item {
        number: ItemNo(number),
        level: 0,
        limit: 0,
        name: String::new(),
        desc: Default::default(),
        hiddesc: Default::default(),
        active: 0,
        weight: 0,
        item_type,
        abilities: slots(abilities),
        charges: 0,
        energy: 0,
        cost: 0,
        classes: [0.into(); 10],
        mindmg: 0,
        maxdmg: 0,
        ac: 0,
        races: [0.into(); 10],
        negates: [SpellNo(0); 10],
        wpntype: WeaponType::from_code(0),
        armtype: ArmourType::from_code(0),
        worn,
        accuracy: 0,
        dr: 0,
        gettable: 0,
        strreq: 0,
        cash: Coins::default(),
        speed: 0,
        hitmsg: 0.into(),
        missmsg: 0.into(),
        textnum: 0.into(),
        destroymsg: 0.into(),
        nodrop: 0,
        currency: Currency::from_code(0),
        retain: 0,
        robable: 0,
        destroy: 0,
    }
}

fn known_monster(abilities: &[(Ability, i16)]) -> KnownMonster {
    KnownMonster {
        number: 0.into(),
        name: String::new(),
        montype: MonsterType::from_code(0),
        level: 0,
        genindex: 0,
        wornitem: ItemNo(0),
        weapon: ItemNo(0),
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
        abilities: slots(abilities),
        maxalive: 0,
        curalive: 0,
        grouprole: GroupRole::from_code(0),
        maxfollow: 0,
        undead: 0,
        alignment: Alignment::from_code(0),
        regenhrs: 0,
        killed_at: 0,
        arrivemsg: 0.into(),
        deathmsg: 0.into(),
        items: [MonsterItem {
            item: ItemNo(0),
            uses: 0,
            pct: 0,
        }; NMONITEMS],
        cash: Coins::default(),
        talktext: 0.into(),
        charmlvl: 0,
        namelist: 0,
        attacks: [MonsterAttack {
            kind: MonsterAttackType::from_code(0),
            accuracy: 0,
            pct: 0,
            min: 0,
            max: 0,
            hitmsg: 0.into(),
            defmsg: 0.into(),
            missmsg: 0.into(),
            energy: 0,
            hitspell: SpellNo(0),
        }; NMONATTACKS],
        randtext: 0.into(),
        charmres: 0,
        deathspell: SpellNo(0),
        createspell: SpellNo(0),
        spells: [MonsterSpell {
            spell: SpellNo(0),
            pct: 0,
            level: 0,
        }; NMONSPELLS],
        desc: Default::default(),
        gender: Gender::from_code(0),
    }
}

fn active_monster() -> ActiveMonster {
    ActiveMonster {
        number: ActiveMonsterId(1),
        knownnum: 0.into(),
        level: 0,
        map: 0.into(),
        room: 0.into(),
        poison: 0,
        energy: 0,
        hp: 0,
        following: String::new(),
        trail: [0.into(); NAMTRAIL],
        hunters: [ActiveMonsterId(0); 10],
        hunting: ActiveMonsterId(0),
        name: String::new(),
        wornitem: ItemNo(0),
        weapon: ItemNo(0),
        items: [(ItemNo(0), 0); NMONITEMS],
        cash: Coins::default(),
        maxhp: 0,
        alignment: Alignment::from_code(0),
        followpct: 0,
        dr: 0,
        ac: 0,
        exp: 0,
        maxenergy: 0,
        friendly: 0,
        genroom: 0.into(),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: MonsterType::from_code(0),
        target: 0,
        hpregen: 0,
        lastdir: 0,
        lastupd: 0,
        grouprole: GroupRole::from_code(0),
        spells: [ActiveSpell {
            spell: SpellNo(0),
            value: 0,
            rounds: 0,
        }; NMONSPELLS],
        downrounds: 0,
        created: 0,
    }
}

// ---------------------------------------------------------------------------------------
// Spell fixtures.

/// A spell with the given strength and growth fields.
fn growing_spell(
    minval: i16,
    maxval: i16,
    mininc: u8,
    minlvls: u8,
    maxinc: u8,
    maxlvls: u8,
) -> Spell {
    let mut s = spell(1, &[(Ability::DamageNoArmour, 0)]);
    s.minval = minval;
    s.maxval = maxval;
    s.mininc = mininc;
    s.minlvls = minlvls;
    s.maxinc = maxinc;
    s.maxlvls = maxlvls;
    s
}

fn caster_at<'a>(level: u16) -> CasterStats<'a> {
    CasterStats {
        firstname: "Thornwyn",
        level,
        spelldamage_pct: 0,
        spellduration_pct: 0,
        spellcast: 50,
    }
}

fn no_spells(_: SpellNo) -> Option<&'static Spell> {
    None
}

fn no_items(_: ItemNo) -> Option<&'static Item> {
    None
}

/// A player target around a blank character with `hp` of `maxhp`.
fn user<'a>(character: &'a mut Character) -> UserTarget<'a, 'static> {
    UserTarget {
        character,
        is_caster: false,
        anti_magic: false,
        poison_immune: false,
        element_resist_pct: 0,
        deathhp: -200,
        arena_safe: false,
        spells: &no_spells,
        items: &no_items,
    }
}

fn player(hp: i16, maxhp: i16) -> Character {
    let mut c = character();
    c.hp = hp;
    c.maxhp = maxhp;
    c.maxmana = 40;
    c.mana = 10;
    c.maxenergy = 1000;
    c.energy = 500;
    c.level = 10;
    c
}

fn monster<'a>(monster: &'a mut ActiveMonster, known: &'a KnownMonster) -> MonsterTarget<'a> {
    MonsterTarget {
        monster,
        known,
        magic_resistance: 1,
        anti_magic: false,
        poison_immune: false,
        element_resist_pct: 0,
    }
}

fn single<'a>(spell: &'a Spell, caster: &'a CasterStats<'a>, power: i32) -> Cast<'a> {
    Cast {
        spell,
        caster: Some(caster),
        path: CastPath::Single,
        power,
    }
}

fn applied(out: &Outcome) -> i32 {
    match out {
        Outcome::Applied { amount, .. } => *amount,
        other => panic!("expected Applied, got {other:?}"),
    }
}

fn effects_of(spell: &Spell) -> SpellEffects {
    SpellEffects::from_record(spell)
}

// ---------------------------------------------------------------------------------------
// Amount: the strength roll and its scaling.

// The strength range: minval/maxval grow by mininc/maxinc per minlvls/maxlvls levels,
// with the level capped.
#[test]
fn amount_range_grows_with_level_up_to_the_cap() {
    let mut s = growing_spell(4, 12, 1, 2, 2, 3);
    assert_eq!(amount_range(&s, 0), AmountRange { min: 4, max: 12 });
    // level 7: minadd = 1*7/2 = 3, maxadd = 2*7/3 = 4.
    assert_eq!(amount_range(&s, 7), AmountRange { min: 7, max: 16 });
    s.levelcap = 6;
    // capped at 6: minadd 3, maxadd 4.
    assert_eq!(amount_range(&s, 7), AmountRange { min: 7, max: 16 });
    assert_eq!(amount_range(&s, 60), AmountRange { min: 7, max: 16 });
    assert_eq!(capped_level(&s, 60), 6);
    // A cap of 0 does not cap; the stock data also holds garbage caps like 8224.
    s.levelcap = 0;
    assert_eq!(capped_level(&s, 60), 60);
}

// A divisor of 0 means no growth.
#[test]
fn amount_range_without_divisor_does_not_grow() {
    let s = growing_spell(4, 12, 1, 0, 1, 0);
    assert_eq!(amount_range(&s, 30), AmountRange { min: 4, max: 12 });
}

// A minimum above the maximum is brought down to it.
#[test]
fn amount_range_minimum_never_exceeds_maximum() {
    let s = growing_spell(10, 3, 0, 0, 0, 0);
    assert_eq!(amount_range(&s, 1), AmountRange { min: 3, max: 3 });
    let s = growing_spell(1, 2, 9, 1, 0, 0);
    assert_eq!(amount_range(&s, 5), AmountRange { min: 2, max: 2 });
}

// genrdn(0, max-min+1) + min covers both ends.
#[test]
fn amount_roll_reaches_both_ends() {
    let s = growing_spell(4, 12, 0, 0, 0, 0);
    assert_eq!(roll_amount(&s, 1, &mut Fixed::of(&[0])), 4);
    assert_eq!(roll_amount(&s, 1, &mut Fixed::of(&[8])), 12);
    assert_eq!(roll_amount(&s, 1, &mut Fixed::of(&[9])), 12);
}

// Layer 1: the roll's distribution against the uniform
// expectation, chi-square at 0.01 with a fixed seed.
#[test]
fn amount_roll_is_uniform_over_the_range() {
    let s = growing_spell(4, 12, 0, 0, 0, 0);
    let mut rng = SeededRng::new(0x5E11);
    let mut counts = [0u64; 9];
    for _ in 0..90_000 {
        let r = roll_amount(&s, 1, &mut rng);
        counts[(r - 4) as usize] += 1;
    }
    let chi = chi_square_uniform(&counts);
    let crit = chi_square_critical_001(8);
    assert!(chi < crit, "chi-square {chi} >= {crit}");
}

// scale_spell_amount: amount*(pct+100)/100, with no 16-bit overflow.
#[test]
fn scale_spell_amount_adds_the_percentage() {
    assert_eq!(scale_spell_amount(100, 20), 120);
    assert_eq!(scale_spell_amount(100, -20), 80);
    assert_eq!(scale_spell_amount(7, 50), 10);
    assert_eq!(scale_spell_amount(-7, 50), -10);
    assert_eq!(scale_spell_amount(32_000, 100), 64_000);
}

// The inline form used by the damage effects rounds the bonus toward zero first.
#[test]
fn inline_spell_damage_bonus_rounds_the_bonus_first() {
    assert_eq!(add_spell_damage_bonus(7, 50), 10);
    assert_eq!(add_spell_damage_bonus(7, -50), 4);
    assert_eq!(scale_spell_amount(7, -50), 3);
}

// Divided kinds split, full kinds give everyone everything.
#[test]
fn per_target_share_divides_only_the_divided_kinds() {
    assert_eq!(per_target_share(10, TargetKind::DivAttack, 4), 2);
    assert_eq!(per_target_share(10, TargetKind::DivArea, 4), 2);
    assert_eq!(per_target_share(10, TargetKind::DivParty, 3), 3);
    assert_eq!(per_target_share(10, TargetKind::Single, 1), 10);
    assert_eq!(per_target_share(10, TargetKind::FullArea, 4), 10);
    assert_eq!(per_target_share(10, TargetKind::FullAttack, 4), 10);
    assert_eq!(per_target_share(10, TargetKind::FullParty, 4), 10);
    assert_eq!(per_target_share(10, TargetKind::Single, 0), 10);
}

// ---------------------------------------------------------------------------------------
// Durations and the slot tables.

fn lasting_spell(number: i16, duration: i16) -> Spell {
    let mut s = spell(number, &[(Ability::Defence, 5)]);
    s.duration = duration;
    s
}

// When a spell is added to a player: +durinc per durlvls levels, re-rolled up to
// durlvl*level, then the Alter spell duration percentage.
#[test]
fn duration_adds_level_bonus_then_rerolls_then_scales() {
    let mut s = lasting_spell(1, 10);
    s.durinc = 2;
    s.durlvls = 3;
    s.durlvl = 0;
    let c = CasterLevel {
        level: 7,
        spellduration_pct: 0,
    };
    // 10 + 2*(7/3) = 14; maxdur 0 so no re-roll.
    assert_eq!(duration(&s, Some(c), &mut Fixed::of(&[99])), 14);
    // durlvl 3: maxdur 21 > 14, re-rolled in [14, 21]: the roll pins it.
    s.durlvl = 3;
    assert_eq!(duration(&s, Some(c), &mut Fixed::of(&[14])), 14);
    assert_eq!(duration(&s, Some(c), &mut Fixed::of(&[21])), 21);
    assert_eq!(duration(&s, Some(c), &mut Fixed::of(&[500])), 21);
    // 50% more: 21*150/100 = 31.
    let c = CasterLevel {
        level: 7,
        spellduration_pct: 50,
    };
    assert_eq!(duration(&s, Some(c), &mut Fixed::of(&[21])), 31);
    // The level cap applies: capped at 3, bonus 2*(3/3) = 2, maxdur 9 < 12.
    s.levelcap = 3;
    assert_eq!(duration(&s, Some(c), &mut Fixed::of(&[0])), 18);
}

// A room spell has no caster; the base duration stands.
#[test]
fn duration_without_caster_is_the_base() {
    let mut s = lasting_spell(1, 10);
    s.durinc = 5;
    s.durlvls = 1;
    s.durlvl = 9;
    assert_eq!(duration(&s, None, &mut Fixed::of(&[99])), 10);
}

fn slots_of(c: &mut Character) -> CharacterSlots<'_> {
    CharacterSlots {
        spells: &mut c.spells,
        spellvals: &mut c.spellvals,
        spellrnds: &mut c.spellrnds,
    }
}

// First free slot; renew replaces; not renewable fails; full fails.
#[test]
fn character_slots_add_renew_refuse_and_fill() {
    let mut c = character();
    assert_eq!(
        add_spell_to_character(slots_of(&mut c), SpellNo(5), 12, 7, true),
        SlotOutcome::Added(0)
    );
    assert_eq!(
        (c.spells[0], c.spellvals[0], c.spellrnds[0]),
        (SpellNo(5), 7, 12)
    );
    assert_eq!(
        add_spell_to_character(slots_of(&mut c), SpellNo(6), 3, 1, true),
        SlotOutcome::Added(1)
    );
    assert_eq!(
        add_spell_to_character(slots_of(&mut c), SpellNo(5), 20, 9, true),
        SlotOutcome::Renewed(0)
    );
    assert_eq!((c.spellvals[0], c.spellrnds[0]), (9, 20));
    assert_eq!(
        add_spell_to_character(slots_of(&mut c), SpellNo(5), 1, 1, false),
        SlotOutcome::AlreadyActive
    );
    assert_eq!((c.spellvals[0], c.spellrnds[0]), (9, 20));
    for n in 7..=14 {
        assert!(add_spell_to_character(slots_of(&mut c), SpellNo(n), 1, 1, true).is_success());
    }
    assert_eq!(
        add_spell_to_character(slots_of(&mut c), SpellNo(99), 1, 1, true),
        SlotOutcome::Full
    );
    assert_eq!(c.spells.len(), NSPELLCAST);
}

// add_spell_to_monster: always renewed; five slots.
#[test]
fn monster_slots_always_renew_and_hold_five() {
    let mut m = active_monster();
    assert_eq!(
        add_spell_to_monster(&mut m.spells, SpellNo(5), 4, 3),
        SlotOutcome::Added(0)
    );
    assert_eq!(
        add_spell_to_monster(&mut m.spells, SpellNo(5), 8, 1),
        SlotOutcome::Renewed(0)
    );
    assert_eq!((m.spells[0].value, m.spells[0].rounds), (1, 8));
    for n in 6..=9 {
        assert!(add_spell_to_monster(&mut m.spells, SpellNo(n), 1, 1).is_success());
    }
    assert_eq!(
        add_spell_to_monster(&mut m.spells, SpellNo(10), 1, 1),
        SlotOutcome::Full
    );
    assert_eq!(m.spells.len(), NMONSPELLS);
}

// monster_add_spell_to_character: renew only when stronger, the last free slot the scan
// saw.
#[test]
fn monster_cast_on_character_renews_only_when_stronger_and_takes_the_last_free_slot() {
    let mut c = character();
    assert_eq!(
        monster_add_spell_to_character(slots_of(&mut c), SpellNo(5), 4, 3, true),
        SlotOutcome::Added(NSPELLCAST - 1)
    );
    assert_eq!(
        monster_add_spell_to_character(slots_of(&mut c), SpellNo(5), 9, 3, true),
        SlotOutcome::AlreadyActive
    );
    assert_eq!(
        monster_add_spell_to_character(slots_of(&mut c), SpellNo(5), 9, 4, true),
        SlotOutcome::Renewed(NSPELLCAST - 1)
    );
    assert_eq!(
        (c.spellvals[NSPELLCAST - 1], c.spellrnds[NSPELLCAST - 1]),
        (4, 9)
    );
    assert_eq!(
        monster_add_spell_to_character(slots_of(&mut c), SpellNo(5), 1, 40, false),
        SlotOutcome::AlreadyActive
    );
}

// ---------------------------------------------------------------------------------------
// Resistance.

fn resist_spell(mode: ResistMode) -> Spell {
    let mut s = spell(3, &[(Ability::DamageArmour, 0)]);
    s.resmode = mode;
    s
}

// item_negates: any worn item's negates[] holds the number.
#[test]
fn worn_item_negates_the_spell() {
    let mut ring = item(1, ItemType::from_code(0), WornLocation::from_code(0), &[]);
    ring.negates[7] = SpellNo(42);
    let other = item(2, ItemType::from_code(0), WornLocation::from_code(0), &[]);
    assert!(item_negates([&other, &ring], SpellNo(42)));
    assert!(!item_negates([&other, &ring], SpellNo(41)));
    assert!(!item_negates([], SpellNo(42)));
}

// SRES_ALWAYS rolls; SRES_ANTIMAG only with Anti Magic; never never.
#[test]
fn resist_mode_decides_who_rolls() {
    assert!(may_resist(&resist_spell(ResistMode::Always), false));
    assert!(may_resist(&resist_spell(ResistMode::AntiMagic), true));
    assert!(!may_resist(&resist_spell(ResistMode::AntiMagic), false));
    assert!(!may_resist(&resist_spell(ResistMode::Never), true));
}

// genrdn(1,100) <= min(mr/2, 98); 99 always beats the cap.
#[test]
fn player_resist_roll_is_half_mr_capped_at_98() {
    let always = resist_spell(ResistMode::Always);
    assert_eq!(resist_chance(60, MAXCHANCE), 30);
    assert_eq!(resist_chance(61, MAXCHANCE), 30);
    assert_eq!(resist_chance(400, MAXCHANCE), 98);
    assert!(player_resists(
        &always,
        60,
        false,
        false,
        &mut Fixed::of(&[30])
    ));
    assert!(!player_resists(
        &always,
        60,
        false,
        false,
        &mut Fixed::of(&[31])
    ));
    assert!(player_resists(
        &always,
        400,
        false,
        false,
        &mut Fixed::of(&[98])
    ));
    assert!(!player_resists(
        &always,
        400,
        false,
        false,
        &mut Fixed::of(&[99])
    ));
    // A negated spell is resisted without a roll.
    assert!(player_resists(
        &resist_spell(ResistMode::Never),
        0,
        false,
        true,
        &mut Fixed::of(&[99])
    ));
}

// An Anti-Magic target resists friendly SRES_ANTIMAG spells at mr/2 too; one without
// Anti Magic never rolls for them.
#[test]
fn anti_magic_resists_friendly_spells_at_half_mr() {
    let friendly = resist_spell(ResistMode::AntiMagic);
    assert!(player_resists(
        &friendly,
        80,
        true,
        false,
        &mut Fixed::of(&[40])
    ));
    assert!(!player_resists(
        &friendly,
        80,
        true,
        false,
        &mut Fixed::of(&[41])
    ));
    assert!(!player_resists(
        &friendly,
        80,
        false,
        false,
        &mut Fixed::of(&[1])
    ));
}

// A monster's resistance is its Magic Resistance ability + magicres, at least 1; with
// Enslave the charm resistance stands in unless it is 0.
#[test]
fn monster_magic_resistance_is_ability_plus_record_at_least_one() {
    assert_eq!(monster_magic_resistance(20, 15, None), 35);
    assert_eq!(monster_magic_resistance(0, 0, None), 1);
    assert_eq!(monster_magic_resistance(0, -30, None), 1);
    assert_eq!(monster_magic_resistance(20, 15, Some(70)), 70);
    assert_eq!(monster_magic_resistance(20, 15, Some(0)), 35);
}

// The single-target cast on a monster: the resistance roll, cap 98.
#[test]
fn monster_resists_single_target_at_half_mr() {
    let always = resist_spell(ResistMode::Always);
    assert!(monster_resists_single(
        &always,
        100,
        false,
        &mut Fixed::of(&[50])
    ));
    assert!(!monster_resists_single(
        &always,
        100,
        false,
        &mut Fixed::of(&[51])
    ));
    assert!(monster_resists_single(
        &always,
        1000,
        false,
        &mut Fixed::of(&[98])
    ));
    assert!(!monster_resists_single(
        &always,
        1000,
        false,
        &mut Fixed::of(&[99])
    ));
    let anti = resist_spell(ResistMode::AntiMagic);
    assert!(monster_resists_single(
        &anti,
        100,
        true,
        &mut Fixed::of(&[1])
    ));
    assert!(!monster_resists_single(
        &anti,
        100,
        false,
        &mut Fixed::of(&[1])
    ));
}

// A monster's resistance check against an area spell can never succeed, so a monster
// never resists an area spell (sic, kept).
#[test]
fn monsters_never_resist_area_spells() {
    let always = resist_spell(ResistMode::Always);
    for roll in [1, 50, 99] {
        assert!(!monster_resists_area(
            &always,
            10_000,
            &mut Fixed::of(&[roll])
        ));
    }
    // The roll is still drawn.
    let mut rng = Fixed::of(&[1]);
    monster_resists_area(&always, 10_000, &mut rng);
    assert_eq!(rng.next, 1);
}

// A monster is immune above the spell level, by hand, to a magical spell.
#[test]
fn spell_immunity_above_the_spell_level_by_hand_only() {
    assert!(spell_immune(10, 9, false, false));
    assert!(!spell_immune(10, 10, false, false));
    assert!(!spell_immune(10, 9, true, false));
    assert!(!spell_immune(10, 9, false, true));
}

// A player's magic-resistance multiplier on "Inflict Damage on Armour", the same on the
// single-target cast, the area cast (cut carried, sic), the player-target path and the
// monster's area and single casts. Below 50 magic resistance the subtraction wraps (sic),
// so it gives the full 50% cut; only mr 50 and 51 reach the "no cut" branch, where the
// cut is 0 and -1. With Anti Magic the cut is `mr/2` held to 0..=75 and a 0 cut takes the
// full amount; the carried cut only counts at exactly 50 (and is held to 50); mr 51
// overwrites it with 0.
#[test]
fn player_mr_multiplier_reproduces_the_unsigned_wrap() {
    type Row = (i32, i32);
    // (mr, cut, damage of 100) without Anti Magic, carried 0 | with Anti Magic | without,
    // carried 40
    #[rustfmt::skip]
    let table: [(i32, Row, Row, Row); 9] = [
        (0,   (50, 50), (0, 100),  (50, 50)),
        (1,   (50, 50), (0, 100),  (50, 50)),
        (30,  (50, 50), (15, 85),  (50, 50)),
        (49,  (50, 50), (24, 76),  (50, 50)),
        (50,  (0, 100), (25, 75),  (40, 60)),
        (51,  (0, 99),  (25, 75),  (0, 99)),
        (52,  (1, 99),  (26, 74),  (1, 99)),
        (95,  (22, 78), (47, 53),  (22, 78)),
        (150, (50, 50), (75, 25),  (50, 50)),
    ];
    for (mr, plain, anti, carried) in table {
        assert_eq!(
            (mr_cut_pct(mr, false, 0), mr_damage(100, mr, false, 0)),
            plain,
            "mr {mr}"
        );
        assert_eq!(
            (mr_cut_pct(mr, true, 0), mr_damage(100, mr, true, 0)),
            anti,
            "mr {mr} AM"
        );
        // Anti Magic ignores the carried value (it always sets its own).
        assert_eq!(
            (mr_cut_pct(mr, true, 40), mr_damage(100, mr, true, 40)),
            anti,
            "mr {mr} AM"
        );
        assert_eq!(
            (mr_cut_pct(mr, false, 40), mr_damage(100, mr, false, 40)),
            carried,
            "mr {mr} carried 40"
        );
    }
    // A carried value above 50 is held to `MRCUT_MAX`.
    assert_eq!(mr_cut_pct(50, false, 120), 50);
    assert_eq!(mr_damage(100, 400, false, 0), 50);
}

// A monster's multiplier (the player's area and single casts on a monster) does not wrap:
// below 50 there is no cut and the monster takes (50-mr)% extra.
#[test]
fn monster_mr_multiplier_is_signed() {
    assert_eq!(monster_mr_cut_pct(30, false), 0);
    assert_eq!(monster_mr_damage(100, 30, false), 120);
    assert_eq!(monster_mr_damage(100, 1, false), 149);
    assert_eq!(monster_mr_damage(100, 50, false), 100);
    assert_eq!(monster_mr_damage(100, 51, false), 99);
    assert_eq!(monster_mr_cut_pct(95, false), 22);
    assert_eq!(monster_mr_damage(100, 95, false), 78);
    assert_eq!(monster_mr_damage(100, 150, false), 50);
    assert_eq!(monster_mr_cut_pct(30, true), 15);
    assert_eq!(monster_mr_damage(100, 1, true), 100);
    assert_eq!(monster_mr_damage(100, 150, true), 25);
}

// room_mr_damage: a room's damage is amount*max(150-mr,25)%.
#[test]
fn room_damage_multiplier_is_linear_with_a_quarter_floor() {
    assert_eq!(room_mr_damage(100, 50), 100);
    assert_eq!(room_mr_damage(100, 100), 50);
    assert_eq!(room_mr_damage(100, 130), 25);
    assert_eq!(room_mr_damage(100, 300), 25);
    assert_eq!(room_mr_damage(100, 0), 150);
}

// element_resistance: the element picks the ability; General spells and the Normal
// element give 0.
#[test]
fn element_resistance_reads_the_matching_ability() {
    let mut c = character();
    c.abils[0] = (Ability::ColdDefence, 30);
    c.abils[1] = (Ability::ResistStone, 12);
    c.abils[2] = (Ability::PoisonImmune, 7);
    let none_spells = |_: SpellNo| None::<&Spell>;
    let none_items = |_: ItemNo| None::<&Item>;
    let look = CharacterAbilities {
        character: &c,
        race: None,
        class: None,
        spells: &none_spells,
        items: &none_items,
    };
    let mut s = spell(1, &[(Ability::DamageArmour, 0)]);
    s.spell_type = SpellType::Autocombat;
    s.element = Element::Cold;
    assert_eq!(element_resistance(&s, &look), 30);
    s.element = Element::Stone;
    assert_eq!(element_resistance(&s, &look), 12);
    s.element = Element::Poison;
    assert_eq!(element_resistance(&s, &look), 7);
    s.element = Element::Hot;
    assert_eq!(element_resistance(&s, &look), 0);
    s.element = Element::Normal;
    assert_eq!(element_resistance(&s, &look), 0);
    s.element = Element::Cold;
    s.spell_type = SpellType::General;
    assert_eq!(element_resistance(&s, &look), 0);
    s.spell_type = SpellType::Poison;
    assert_eq!(element_resistance(&s, &look), 30);
    // Taken off as a percentage, with no 16-bit overflow.
    assert_eq!(apply_element_resistance(50, 30), 35);
    assert_eq!(apply_element_resistance(50, 100), 0);
    assert_eq!(apply_element_resistance(50, -50), 75);
}

// ---------------------------------------------------------------------------------------
// Target validity with a party and a stranger in the room.

fn in_room() -> PlayerCandidate {
    PlayerCandidate {
        same_room: true,
        pvp_range_ok: true,
        ..PlayerCandidate::default()
    }
}

fn targeted(spell: &Spell, c: &PlayerCandidate) -> bool {
    is_valid_target(spell, c).is_some()
}

fn attack_spell(target: TargetKind) -> Spell {
    let mut s = spell(1, &[(Ability::DamageNoArmour, 0)]);
    s.spell_type = SpellType::Autocombat;
    s.target = target;
    s
}

fn general_spell(target: TargetKind) -> Spell {
    let mut s = spell(1, &[(Ability::Heal, 0)]);
    s.spell_type = SpellType::General;
    s.target = target;
    s
}

// is_valid_target: the attack areas spare the caster, his leader, his followers and his
// group; the party areas take exactly those; a stranger is hit by the attack areas and
// not by the party areas.
#[test]
fn party_and_stranger_in_the_room() {
    let me = PlayerCandidate {
        is_caster: true,
        ..in_room()
    };
    let leader = PlayerCandidate {
        leads_caster: true,
        ..in_room()
    };
    let follower = PlayerCandidate {
        led_by_caster: true,
        ..in_room()
    };
    let grouped = PlayerCandidate {
        in_casters_group: true,
        ..in_room()
    };
    let stranger = in_room();
    for kind in [TargetKind::DivAttack, TargetKind::FullAttack] {
        let s = attack_spell(kind);
        assert!(!targeted(&s, &me), "{kind:?} caster");
        assert!(!targeted(&s, &leader), "{kind:?} leader");
        assert!(!targeted(&s, &follower), "{kind:?} follower");
        assert!(!targeted(&s, &grouped), "{kind:?} group");
        assert!(targeted(&s, &stranger), "{kind:?} stranger");
    }
    for kind in [TargetKind::DivParty, TargetKind::FullParty] {
        let s = general_spell(kind);
        assert!(targeted(&s, &me), "{kind:?} caster");
        assert!(targeted(&s, &leader), "{kind:?} leader");
        assert!(targeted(&s, &follower), "{kind:?} follower");
        assert!(targeted(&s, &grouped), "{kind:?} group");
        assert!(!targeted(&s, &stranger), "{kind:?} stranger");
    }
    // "Divided Area Not You": everyone but the caster.
    let s = attack_spell(TargetKind::DivArea);
    assert!(!targeted(&s, &me));
    assert!(targeted(&s, &leader));
    assert!(targeted(&s, &stranger));
    // Full Area, Divided Area Incl. You and Any take anyone.
    for kind in [
        TargetKind::FullArea,
        TargetKind::DivAreaIncl,
        TargetKind::Any,
    ] {
        let s = general_spell(kind);
        assert!(targeted(&s, &me));
        assert!(targeted(&s, &stranger));
    }
    // Single and self kinds count only the caster.
    for kind in [
        TargetKind::Single,
        TargetKind::OnSelf,
        TargetKind::SelfOrSingle,
    ] {
        let s = general_spell(kind);
        assert!(targeted(&s, &me));
        assert!(!targeted(&s, &stranger));
    }
}

// Evil is flagged below EVIL_OUTLAW; another room never counts; the pvp rules apply to
// attacks and to General spells with Evil in Combat.
#[test]
fn target_validity_evil_room_and_pvp_rules() {
    let s = attack_spell(TargetKind::FullAttack);
    let evil_act = PlayerCandidate {
        evil: 39,
        attack_gives_evil: true,
        ..in_room()
    };
    assert_eq!(
        is_valid_target(&s, &evil_act),
        Some(ValidTarget { evil: true })
    );
    let outlaw = PlayerCandidate {
        evil: 40,
        ..evil_act
    };
    assert_eq!(
        is_valid_target(&s, &outlaw),
        Some(ValidTarget { evil: false })
    );
    let elsewhere = PlayerCandidate {
        same_room: false,
        ..in_room()
    };
    assert!(!targeted(&s, &elsewhere));
    // Out of pvp range: refused unless a safe arena or sysop debug.
    let far = PlayerCandidate {
        pvp_range_ok: false,
        ..in_room()
    };
    assert!(!targeted(&s, &far));
    assert!(targeted(
        &s,
        &PlayerCandidate {
            arena_safe: true,
            ..far
        }
    ));
    assert!(targeted(
        &s,
        &PlayerCandidate {
            sysop_debug: true,
            ..far
        }
    ));
    // A General spell skips the pvp rules unless it carries Evil in Combat.
    let heal = general_spell(TargetKind::FullArea);
    assert!(targeted(&heal, &far));
    let mut evil_heal = heal.clone();
    evil_heal.abilities[1] = (Ability::EvilInCombat, 0);
    assert!(!targeted(&evil_heal, &far));
    // A self spell on oneself passes the pvp rules.
    let own = general_spell(TargetKind::OnSelf);
    let mut evil_own = own.clone();
    evil_own.abilities[1] = (Ability::EvilInCombat, 0);
    let me_far = PlayerCandidate {
        is_caster: true,
        pvp_range_ok: false,
        ..in_room()
    };
    assert!(targeted(&evil_own, &me_far));
}

fn mon(montype: MonsterType, alignment: Alignment) -> MonsterCandidate {
    MonsterCandidate {
        alignment,
        montype,
        follows_caster: false,
        charmed: false,
        friendly: false,
        caster_evil: 0,
        can_hit: true,
    }
}

// is_valid_monster_target: guards and angels only for outlaws, never a lawful good
// monster, a charmed follower spared by attack areas and taken by party areas.
#[test]
fn monster_target_validity_in_areas() {
    let s = attack_spell(TargetKind::FullAttack);
    let wanderer = mon(MonsterType::Wanderer, Alignment::Evil);
    assert_eq!(
        is_valid_monster_target(&s, &wanderer),
        Some(ValidTarget { evil: false })
    );
    let good = mon(MonsterType::Wanderer, Alignment::Good);
    assert_eq!(
        is_valid_monster_target(&s, &good),
        Some(ValidTarget { evil: true })
    );
    let guard = mon(MonsterType::Guard, Alignment::Neutral);
    assert!(is_valid_monster_target(&s, &guard).is_none());
    let angel = mon(MonsterType::Angel, Alignment::Neutral);
    assert!(is_valid_monster_target(&s, &angel).is_none());
    let outlaw = MonsterCandidate {
        caster_evil: 40,
        ..guard
    };
    assert!(is_valid_monster_target(&s, &outlaw).is_some());
    let lawful = mon(MonsterType::Wanderer, Alignment::LawfulGood);
    assert!(is_valid_monster_target(&s, &lawful).is_none());
    let lawful_outlaw = MonsterCandidate {
        caster_evil: 99,
        ..lawful
    };
    assert!(is_valid_monster_target(&s, &lawful_outlaw).is_none());
    // A charmed follower is spared; an unfriendly follower is always hit.
    let pet = MonsterCandidate {
        charmed: true,
        follows_caster: true,
        ..guard
    };
    assert!(is_valid_monster_target(&s, &pet).is_none());
    let stalker = MonsterCandidate {
        follows_caster: true,
        ..guard
    };
    assert!(is_valid_monster_target(&s, &stalker).is_some());
    // Party areas: only a charmed follower.
    let party = general_spell(TargetKind::FullParty);
    assert!(is_valid_monster_target(&party, &pet).is_some());
    assert!(is_valid_monster_target(&party, &wanderer).is_none());
    // Single, self and item kinds never take a monster; the rest take any.
    assert!(is_valid_monster_target(&general_spell(TargetKind::Single), &wanderer).is_none());
    assert!(is_valid_monster_target(&general_spell(TargetKind::Item), &wanderer).is_none());
    assert!(is_valid_monster_target(&general_spell(TargetKind::FullArea), &lawful).is_some());
    // Required to hit.
    let unhittable = MonsterCandidate {
        can_hit: false,
        ..wanderer
    };
    assert!(is_valid_monster_target(&s, &unhittable).is_none());
}

// spell_can_hit_monster: a Required to hit value must be matched.
#[test]
fn required_to_hit_must_match() {
    let plain = spell(1, &[(Ability::DamageNoArmour, 0)]);
    let keyed = spell(2, &[(Ability::DamageNoArmour, 0), (Ability::ReqToHit, 3)]);
    assert!(spell_can_hit_monster(0, &plain));
    assert!(!spell_can_hit_monster(3, &plain));
    assert!(spell_can_hit_monster(3, &keyed));
    assert!(!spell_can_hit_monster(4, &keyed));
}

// can_use_spell: magery, level and alignment.
#[test]
fn can_use_spell_checks_magery_level_and_alignment() {
    let mut cls = class(&[]);
    cls.magery = Magery::Mage;
    cls.magelvl = 2;
    let mut s = spell(1, &[(Ability::Heal, 0)]);
    s.magery = Magery::Mage;
    s.magelvl = 2;
    s.level = 5;
    assert_eq!(can_use_spell(&s, &cls, 5, LegalBand::Neutral), Ok(()));
    assert_eq!(
        can_use_spell(&s, &cls, 4, LegalBand::Neutral),
        Err(CannotUse::TooPowerful)
    );
    s.magelvl = 3;
    assert_eq!(
        can_use_spell(&s, &cls, 5, LegalBand::Neutral),
        Err(CannotUse::MageryLevel)
    );
    s.magelvl = 2;
    s.magery = Magery::Priest;
    assert_eq!(
        can_use_spell(&s, &cls, 5, LegalBand::Neutral),
        Err(CannotUse::WrongMagery)
    );
    s.magery = Magery::None;
    assert_eq!(can_use_spell(&s, &cls, 5, LegalBand::Neutral), Ok(()));
    s.abilities[1] = (Ability::Good, 0);
    assert_eq!(can_use_spell(&s, &cls, 5, LegalBand::Good), Ok(()));
    assert_eq!(
        can_use_spell(&s, &cls, 5, LegalBand::Neutral),
        Err(CannotUse::Alignment)
    );
    assert_eq!(
        can_use_spell(&s, &cls, 5, LegalBand::Evil),
        Err(CannotUse::Alignment)
    );
    s.abilities[1] = (Ability::NotGood, 0);
    assert_eq!(
        can_use_spell(&s, &cls, 5, LegalBand::Good),
        Err(CannotUse::Alignment)
    );
    assert_eq!(can_use_spell(&s, &cls, 5, LegalBand::Neutral), Ok(()));
    s.abilities[1] = (Ability::Neutral, 0);
    assert_eq!(can_use_spell(&s, &cls, 5, LegalBand::Neutral), Ok(()));
    assert_eq!(
        can_use_spell(&s, &cls, 5, LegalBand::Evil),
        Err(CannotUse::Alignment)
    );
    // The alignment bands.
    assert_eq!(LegalBand::of_evil(-51), LegalBand::Good);
    assert_eq!(LegalBand::of_evil(-50), LegalBand::Neutral);
    assert_eq!(LegalBand::of_evil(39), LegalBand::Neutral);
    assert_eq!(LegalBand::of_evil(40), LegalBand::Evil);
}

// match_kinds: what each target kind can be cast on.
#[test]
fn match_kinds_by_target() {
    assert!(match_kinds(&attack_spell(TargetKind::FullArea)).none());
    let single = match_kinds(&attack_spell(TargetKind::Single));
    assert!(single.users && single.not_self && !single.monsters);
    let heal = match_kinds(&general_spell(TargetKind::Single));
    assert!(heal.users && !heal.not_self);
    let m = match_kinds(&attack_spell(TargetKind::Monster));
    assert!(m.monsters && m.charmed_last && !m.users);
    let i = match_kinds(&attack_spell(TargetKind::Item));
    assert!(i.inventory && i.floor && !i.users);
    let ms = match_kinds(&attack_spell(TargetKind::MonsterOrSingle));
    assert!(ms.monsters && ms.users && !ms.inventory);
    let any = match_kinds(&attack_spell(TargetKind::Any));
    assert!(any.users && any.monsters && any.inventory && any.floor && any.spells);
}

#[test]
fn target_kind_helpers() {
    assert!(TargetKind::DivAttack.is_area() && TargetKind::DivAttack.is_divided());
    assert!(TargetKind::FullParty.is_area() && !TargetKind::FullParty.is_divided());
    assert!(!TargetKind::Single.is_area() && TargetKind::Single.is_divided());
    assert!(TargetKind::DivAreaIncl.area_reaches_monsters());
    assert!(!TargetKind::DivParty.area_reaches_monsters());
    assert!(TargetKind::DivParty.area_includes_caster());
    assert!(!TargetKind::FullArea.area_includes_caster());
    assert!(TargetKind::Any.is_self());
}

// ---------------------------------------------------------------------------------------
// Effects: from_record.

// Every defined ability classifies, and the markers are the abilities with no effect of
// their own.
#[test]
fn every_ability_classifies_and_markers_are_the_no_effect_list() {
    for &a in Ability::ALL {
        let e = SpellEffect::from_slot(a, 0);
        assert!(!matches!(e, SpellEffect::Unknown(_)), "{a:?}");
    }
    assert!(SpellEffect::from_slot(Ability::DescMsg, 0).is_marker());
    assert!(SpellEffect::from_slot(Ability::NonMagical, 0).is_marker());
    assert!(!SpellEffect::from_slot(Ability::Heal, 0).is_marker());
    assert_eq!(
        SpellEffect::from_slot(Ability::DamageArmour, 0),
        SpellEffect::Damage {
            ignore_armour: false
        }
    );
    assert_eq!(
        SpellEffect::from_slot(Ability::Wisdom, 0),
        SpellEffect::StatAlter(Stat::Wisdom)
    );
    assert_eq!(
        SpellEffect::from_slot(Ability::TermSpell, 0),
        SpellEffect::RemoveSpell { chain: false }
    );
    assert_eq!(
        SpellEffect::from_slot(Ability::Shield, 0),
        SpellEffect::Lasting(Ability::Shield)
    );
    assert_eq!(
        SpellEffect::from_slot(Ability::Unknown(999), 0),
        SpellEffect::Unknown(999)
    );
    let s = spell(1, &[(Ability::Heal, 0), (Ability::TextBlock, 77)]);
    let e = effects_of(&s);
    assert_eq!(e.slots.len(), 2);
    assert_eq!(e.slots[1].amount(5), 77);
    assert_eq!(e.slots[0].amount(5), 5);
    assert!(e.has(Ability::TextBlock));
    assert_eq!(e.value_of(Ability::TextBlock), 77);
    assert_eq!(e.value_of(Ability::Heal), 0);
}

// ---------------------------------------------------------------------------------------
// Effects applied to a player.

// Damage without armour on a player target: scaled by the caster's spell damage bonus,
// the knock-out rule on the damage.
#[test]
fn damage_ignoring_armour_on_a_player_is_scaled_and_knocks_out() {
    let s = spell(1, &[(Ability::DamageNoArmour, 0)]);
    let mut c = player(20, 50);
    let mut caster = caster_at(10);
    caster.spelldamage_pct = 50;
    let cast = single(&s, &caster, 10);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::Applied {
            amount: 15,
            knocked_out: false
        }]
    );
    assert_eq!(c.hp, 5);
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::Applied {
            amount: 15,
            knocked_out: true
        }]
    );
    assert_eq!(c.hp, -10);
}

// The lasting form stores the unscaled value in a slot.
#[test]
fn lasting_damage_goes_into_a_slot_unscaled() {
    let mut s = spell(4, &[(Ability::DamageNoArmour, 0)]);
    s.duration = 3;
    let mut c = player(20, 50);
    let mut caster = caster_at(10);
    caster.spelldamage_pct = 50;
    let cast = single(&s, &caster, 10);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::Lasting(SlotOutcome::Added(0))]);
    assert_eq!(
        (c.spells[0], c.spellvals[0], c.spellrnds[0]),
        (SpellNo(4), 10, 3)
    );
    assert_eq!(c.hp, 20);
    assert!(state.told);
}

// The player's area cast: the element share, no spell damage bonus, knock-out tested
// with the undivided share (sic).
#[test]
fn area_damage_uses_the_element_share_and_the_undivided_knockout() {
    let mut s = attack_spell(TargetKind::FullAttack);
    s.element = Element::Cold;
    let mut c = player(10, 50);
    let mut caster = caster_at(10);
    caster.spelldamage_pct = 50;
    let cast = Cast {
        spell: &s,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 40 },
        power: 40,
    };
    let mut state = CastState::new();
    let mut u = user(&mut c);
    u.element_resist_pct = 50;
    let mut t = Target::User(u);
    let out = apply_effect(
        &effects_of(&s).slots[0],
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    // 40 * 50% = 20 off the share; hp 10 - 20 = -10; knock-out: -10 + 40 > 0.
    assert_eq!(
        out,
        Outcome::Applied {
            amount: 20,
            knocked_out: true
        }
    );
    assert_eq!(c.hp, -10);
    // The caster is hit only by the inclusive kinds.
    let mut c2 = player(10, 50);
    let mut u = user(&mut c2);
    u.is_caster = true;
    let mut t = Target::User(u);
    let out = apply_effect(
        &effects_of(&s).slots[0],
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, Outcome::NoEffect);
    assert_eq!(c2.hp, 10);
}

// Damage on armour on a player target: the bonus, then the multiplier.
#[test]
fn damage_on_armour_applies_the_multiplier_on_a_player() {
    let s = spell(1, &[(Ability::DamageArmour, 0)]);
    let mut c = player(100, 100);
    c.mr = 95;
    let mut caster = caster_at(10);
    caster.spelldamage_pct = 10;
    let cast = single(&s, &caster, 100);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    // 110 after the bonus, cut 22%: 110 - 24 = 86.
    assert_eq!(applied(&out[0]), 86);
    assert_eq!(c.hp, 14);
    // Anti Magic at mr 150: 75% off 110 = 28 (110 - 82).
    let mut c = player(100, 100);
    c.mr = 150;
    let mut u = user(&mut c);
    u.anti_magic = true;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 28);
    // A lasting record still deals instant damage (no duration test).
    let mut s2 = s.clone();
    s2.duration = 5;
    let cast = single(&s2, &caster, 100);
    let mut c = player(100, 100);
    c.mr = 50;
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s2),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 110);
    assert_eq!(c.spells[0], SpellNo(0));
}

// The area loop carries the previous player's damage as the next low-resistance
// player's cut (sic).
#[test]
fn area_damage_on_armour_carries_the_cut_between_players() {
    let mut s = attack_spell(TargetKind::FullAttack);
    s.abilities[0] = (Ability::DamageArmour, 0);
    let caster = caster_at(10);
    let cast = Cast {
        spell: &s,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 100 },
        power: 100,
    };
    let slot = effects_of(&s).slots[0];
    let mut state = CastState::new();
    let mut first = player(200, 200);
    first.mr = 130;
    let mut t = Target::User(user(&mut first));
    assert_eq!(
        applied(&apply_effect(
            &slot,
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        )),
        60
    );
    assert_eq!(state.area_armour_carry, 60);
    let mut second = player(200, 200);
    second.mr = 30;
    let mut t = Target::User(user(&mut second));
    // Alone he would take 120; after the first he keeps a cut of min(60, 50) = 50.
    assert_eq!(
        applied(&apply_effect(
            &slot,
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        )),
        50
    );
}

// Heal: capped at the maximum; the area floors at 0.
#[test]
fn heal_is_capped_at_max_hp() {
    let s = general_spell(TargetKind::Single);
    let caster = caster_at(10);
    let cast = single(&s, &caster, 30);
    let mut c = player(40, 50);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 10);
    assert_eq!(c.hp, 50);
    // Over the maximum on the self path the gain goes negative; the area path floors
    // it.
    let mut c = player(60, 50);
    let on_self = Cast {
        path: CastPath::OnSelf,
        ..cast.clone()
    };
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &on_self,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), -10);
    assert_eq!(c.hp, 50);
    let mut c = player(60, 50);
    let area = Cast {
        path: CastPath::Area { undivided: 30 },
        ..cast.clone()
    };
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &area,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 0);
    assert_eq!(c.hp, 60);
}

// Heal mana: capped at the maximum, floored at 0.
#[test]
fn heal_mana_is_capped_and_floored() {
    let s = spell(1, &[(Ability::HealMana, 0)]);
    let caster = caster_at(10);
    let mut c = player(10, 10);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 100),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 30);
    assert_eq!(c.mana, 40);
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, -100),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), -100);
    assert_eq!(c.mana, 0);
}

// Poison: immune targets are skipped; the level is raised to the amount if lower; the
// lasting form also adds the spell.
#[test]
fn poison_raises_the_level_and_skips_the_immune() {
    let mut s = spell(9, &[(Ability::Poison, 0)]);
    let caster = caster_at(10);
    let mut c = player(10, 10);
    c.poison = 5;
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 8),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 8);
    assert_eq!(c.poison, 8);
    let mut t = Target::User(user(&mut c));
    apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 3),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(c.poison, 8);
    let mut u = user(&mut c);
    u.poison_immune = true;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 50),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::Immune]);
    assert_eq!(c.poison, 8);
    s.duration = 4;
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 12),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::Lasting(SlotOutcome::Added(0))]);
    assert_eq!(c.poison, 12);
    assert_eq!(c.spells[0], SpellNo(9));
}

// Cure poison ends the first poison spell: its slot is cleared and its ending listed for
// the engine (T-100), then the level falls by the cut, never below 0.
#[test]
fn cure_poison_ends_the_poison_spell_and_floors_the_level() {
    let mut poison_spell = spell(9, &[(Ability::Poison, 0), (Ability::EndCast, 77)]);
    poison_spell.element = Element::Poison;
    let other = spell(8, &[(Ability::Shield, 3)]);
    let lookup = move |n: SpellNo| -> Option<&'static Spell> {
        if n == SpellNo(9) {
            Some(Box::leak(Box::new(poison_spell.clone())))
        } else if n == SpellNo(8) {
            Some(Box::leak(Box::new(other.clone())))
        } else {
            None
        }
    };
    let s = spell(1, &[(Ability::CurePoison, 0)]);
    let caster = caster_at(10);
    let mut c = player(10, 10);
    c.poison = 12;
    c.spells[0] = SpellNo(8);
    c.spells[1] = SpellNo(9);
    c.spellvals[1] = 12;
    c.spellrnds[1] = 3;
    let mut state = CastState::new();
    let mut u = user(&mut c);
    u.spells = &lookup;
    let mut t = Target::User(u);
    // The ending's own 12 off is the engine's; the cure's 20, floored.
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 20),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::Ended {
            spells: vec![EndedSpell {
                number: SpellNo(9),
                power: 12,
                chain: true
            }],
            poison: Some(PoisonCut {
                amount: 20,
                floor: true
            })
        }]
    );
    assert_eq!(c.poison, 0);
    assert_eq!(c.spells[1], SpellNo(0));
    assert_eq!(c.spells[0], SpellNo(8));
}

// Steal hit points: the target loses, the caster gains up to his maximum.
#[test]
fn steal_hp_reports_the_casters_gain() {
    let s = spell(1, &[(Ability::StealHp, 0)]);
    let caster = caster_at(10);
    let mut c = player(30, 30);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 12),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::StolenHp { amount: 12 }]);
    assert_eq!(c.hp, 18);
    let mut me = player(40, 45);
    gain_stolen_hp(&mut me, 12);
    assert_eq!(me.hp, 45);
    // On oneself it is silly; lasting is silly.
    let on_self = Cast {
        path: CastPath::OnSelf,
        ..single(&s, &caster, 12)
    };
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &on_self,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Silly]
    );
}

// Intellect (and the other stats, and Alter hit points): lasting only; the stat rises
// when the slot entry is made; an active copy fails and raises nothing.
#[test]
fn stat_alters_are_lasting_and_do_not_stack() {
    let mut s = spell(5, &[(Ability::Intellect, 0), (Ability::AlterHp, 7)]);
    let caster = caster_at(10);
    let mut c = player(10, 10);
    c.intel = 50;
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 4),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::Silly, Outcome::Silly]);
    assert_eq!(c.intel, 50);
    s.duration = 6;
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 4),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    // The first slot adds the spell; the second finds it told and still raises its stat.
    assert_eq!(
        out,
        vec![Outcome::Lasting(SlotOutcome::Added(0)), Outcome::NoEffect]
    );
    assert_eq!((c.intel, c.maxhp, c.hp), (54, 17, 17));
    assert_eq!((c.spells[0], c.spellvals[0]), (SpellNo(5), 4));
    // Cast again: already active, not renewable, nothing raised.
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 4),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out[0], Outcome::AlreadyActive);
    assert_eq!(c.intel, 54);
}

// A player's spell ending: the stat and hit points go again, the poison wears off, Give
// Spell leaves the book, the chain rolls its chance.
#[test]
fn termination_undoes_the_lasting_changes() {
    let s = spell(
        5,
        &[
            (Ability::Intellect, 0),
            (Ability::AlterHp, 7),
            (Ability::Poison, 0),
            (Ability::GiveSpell, 33),
            (Ability::EndCast, 44),
            (Ability::EndCastChance, 30),
        ],
    );
    let mut c = player(17, 17);
    c.intel = 54;
    c.poison = 10;
    c.spellbook = vec![SpellNo(1), SpellNo(33)];
    let t = apply_termination(&mut c, &s, 4, true, &mut Fixed::of(&[29]));
    assert_eq!((c.intel, c.maxhp, c.hp, c.poison), (50, 10, 10, 6));
    assert_eq!(c.spellbook, vec![SpellNo(1)]);
    assert_eq!(t.chain, Some(SpellNo(44)));
    assert_eq!(t.purged, Some(SpellNo(33)));
    let t = apply_termination(&mut c, &s, 4, true, &mut Fixed::of(&[30]));
    assert_eq!(t.chain, None);
    assert_eq!(c.poison, 2);
    let t = apply_termination(&mut c, &s, 4, false, &mut Fixed::of(&[0]));
    assert_eq!(t.chain, None);
    assert_eq!(c.poison, 0);
}

// A player's per-round spell upkeep.
#[test]
fn per_round_effects_on_a_player() {
    let s = spell(
        5,
        &[
            (Ability::Heal, 0),
            (Ability::HealMana, 100),
            (Ability::Energy, 900),
            (Ability::Hunger, -2),
            (Ability::CurePoison, 3),
            (Ability::Flee, 50),
        ],
    );
    let mut c = player(45, 50);
    c.poison = 2;
    let out = apply_round(&mut c, &effects_of(&s), 10, &mut Fixed::of(&[49]));
    assert_eq!(
        (c.hp, c.mana, c.energy, c.hunger, c.poison),
        (50, 40, 1000, -2, 0)
    );
    assert_eq!(
        out,
        RoundOutcome {
            flee: true,
            knocked_out: false,
            prompt: true
        }
    );
    let out = apply_round(&mut c, &effects_of(&s), 10, &mut Fixed::of(&[50]));
    assert!(!out.flee);
    // Damage over time knocks out; a negative cure is ignored.
    let dot = spell(
        6,
        &[(Ability::DamageNoArmour, 0), (Ability::CurePoison, -5)],
    );
    let mut c = player(5, 50);
    c.poison = 1;
    let out = apply_round(&mut c, &effects_of(&dot), 8, &mut Fixed::of(&[0]));
    assert_eq!(
        out,
        RoundOutcome {
            flee: false,
            knocked_out: true,
            prompt: true
        }
    );
    assert_eq!((c.hp, c.poison), (-3, 1));
}

// A monster's per-round spell upkeep: Heal has no cap, Steal Hit Points does nothing; the
// ending frees an enslaved monster.
#[test]
fn per_round_and_termination_on_a_monster() {
    let s = spell(
        5,
        &[
            (Ability::Heal, 0),
            (Ability::StealHp, 4),
            (Ability::Energy, 50),
        ],
    );
    let mut m = active_monster();
    m.hp = 10;
    m.maxhp = 12;
    m.maxenergy = 100;
    m.energy = 80;
    apply_monster_round(&mut m, &effects_of(&s), 10, &mut Fixed::of(&[0]));
    assert_eq!((m.hp, m.energy), (20, 100));
    let charm = spell(6, &[(Ability::Enslave, 0), (Ability::Poison, 0)]);
    m.following = "Thornwyn".to_owned();
    m.friendly = 1;
    m.flags |= ActiveMonsterFlags::CHARMED;
    m.poison = 3;
    apply_monster_termination(&mut m, &charm, 5);
    assert!(
        m.following.is_empty() && m.friendly == 0 && !m.flags.contains(ActiveMonsterFlags::CHARMED)
    );
    assert_eq!(m.poison, 0);
}

// Slay: to one above death, or to death from below 0; lasting is silly.
#[test]
fn slay_takes_a_player_to_one_above_death() {
    let s = spell(1, &[(Ability::Slay, 0)]);
    let caster = caster_at(10);
    let mut c = player(30, 30);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(applied(&out[0]), 231);
    assert_eq!(c.hp, -201);
    let mut c = player(-5, 30);
    let mut t = Target::User(user(&mut c));
    apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(c.hp, -205);
}

// Shatter: the amount must beat the weapon's Quality; a safe arena only unwields.
#[test]
fn shatter_breaks_a_weapon_below_the_amount() {
    let sword = item(
        7,
        ItemType::from_code(1),
        WornLocation::from_code(0),
        &[(Ability::Quality, 5)],
    );
    let lookup = move |n: ItemNo| -> Option<&'static Item> {
        (n == ItemNo(7)).then(|| &*Box::leak(Box::new(sword.clone())))
    };
    let s = spell(1, &[(Ability::Shatter, 0)]);
    let caster = caster_at(10);
    let mut c = player(30, 30);
    c.weapon = ItemNo(7);
    let mut state = CastState::new();
    let mut u = user(&mut c);
    u.items = &lookup;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 5),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::NoEffect]);
    let mut u = user(&mut c);
    u.items = &lookup;
    u.arena_safe = true;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 6),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::Shattered {
            item: ItemNo(7),
            destroyed: false
        }]
    );
    assert_eq!(c.weapon, ItemNo(0));
}

// Remove curse: Cursed items and Major Curse items below the caster's level come off;
// only Self or Single and Any act on a player target (sic).
#[test]
fn remove_curse_lists_the_cursed_worn_slots() {
    let cursed = item(
        1,
        ItemType::from_code(0),
        WornLocation::from_code(0),
        &[(Ability::Cursed, 1)],
    );
    let major = item(
        2,
        ItemType::from_code(0),
        WornLocation::from_code(0),
        &[(Ability::MajorCurse, 9)],
    );
    let plain = item(3, ItemType::from_code(0), WornLocation::from_code(0), &[]);
    let lookup = move |n: ItemNo| -> Option<&'static Item> {
        let it = match n {
            ItemNo(1) => cursed.clone(),
            ItemNo(2) => major.clone(),
            ItemNo(3) => plain.clone(),
            _ => return None,
        };
        Some(Box::leak(Box::new(it)))
    };
    let mut s = spell(1, &[(Ability::RemoveCurse, 0)]);
    s.target = TargetKind::Any;
    let caster = caster_at(10);
    let mut c = player(30, 30);
    c.worn[2] = ItemNo(1);
    c.worn[5] = ItemNo(2);
    c.worn[6] = ItemNo(3);
    let mut state = CastState::new();
    let mut u = user(&mut c);
    u.items = &lookup;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::RemoveCurse {
            worn_slots: vec![2, 5]
        }]
    );
    // Level 9 does not beat a Major Curse of 9.
    let low = caster_at(9);
    let mut u = user(&mut c);
    u.items = &lookup;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &low, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::RemoveCurse {
            worn_slots: vec![2]
        }]
    );
    s.target = TargetKind::Single;
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(out, vec![Outcome::NoEffect]);
}

// Freedom and Dispel magic end matching active spells: the slots are cleared and the
// endings listed for the engine to run, in order (T-100); a slot whose record is gone
// is cleared.
#[test]
fn freedom_and_dispel_end_matching_spells() {
    let hold = spell(11, &[(Ability::HoldPerson, 1)]);
    let bless = spell(12, &[(Ability::AvBless, 3), (Ability::Strength, 2)]);
    let lookup = move |n: SpellNo| -> Option<&'static Spell> {
        let s = match n {
            SpellNo(11) => hold.clone(),
            SpellNo(12) => bless.clone(),
            _ => return None,
        };
        Some(Box::leak(Box::new(s)))
    };
    let caster = caster_at(10);
    let mut c = player(30, 30);
    c.str = 52;
    c.spells[0] = SpellNo(12);
    c.spells[1] = SpellNo(11);
    c.spells[2] = SpellNo(99);
    let mut state = CastState::new();
    let freedom = spell(1, &[(Ability::Freedom, 0)]);
    let mut u = user(&mut c);
    u.spells = &lookup;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&freedom),
        &single(&freedom, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::Ended {
            spells: vec![EndedSpell {
                number: SpellNo(11),
                power: 0,
                chain: true
            }],
            poison: None
        }]
    );
    assert_eq!(
        c.spells,
        [
            SpellNo(12),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0),
            SpellNo(0)
        ]
    );
    // Dispel by ability number: Strength (46) matches the bless; its ending (the engine's)
    // takes the 2 off.
    let dispel = spell(2, &[(Ability::DispelMagic, 46)]);
    let mut u = user(&mut c);
    u.spells = &lookup;
    let mut t = Target::User(u);
    let out = apply_spell(
        &effects_of(&dispel),
        &single(&dispel, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![Outcome::Ended {
            spells: vec![EndedSpell {
                number: SpellNo(12),
                power: 0,
                chain: true
            }],
            poison: None
        }]
    );
    assert_eq!(c.str, 52);
    assert_eq!(c.spells[0], SpellNo(0));
    // -1 ends everything; an area cast is silly except Full Party Area.
    let all = spell(3, &[(Ability::DispelMagic, -1)]);
    let area = Cast {
        spell: &all,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 1 },
        power: 1,
    };
    let mut s_area = all.clone();
    s_area.target = TargetKind::FullAttack;
    let area_cast = Cast {
        spell: &s_area,
        ..area
    };
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&s_area),
            &area_cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Silly]
    );
}

// The chain and teleport halves live in the cast state.
#[test]
fn end_cast_and_teleport_fill_the_cast_state() {
    let s = spell(
        1,
        &[
            (Ability::EndCast, 50),
            (Ability::EndCastChance, 25),
            (Ability::TeleportRoom, 300),
            (Ability::TeleportMap, 2),
            (Ability::ClearItem, 9),
            (Ability::TextBlock, 4),
        ],
    );
    let caster = caster_at(10);
    let mut c = player(30, 30);
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![
            Outcome::NoEffect,
            Outcome::NoEffect,
            Outcome::Teleport(TeleportPart::Room, 300),
            Outcome::Teleport(TeleportPart::Map, 2),
            Outcome::ClearItem(9),
            Outcome::TextBlock(4),
        ]
    );
    assert_eq!(state.chain, Some(SpellNo(50)));
    assert_eq!(state.chain_chance, 25);
    assert_eq!(state.teleport(), Some((300, 2)));
    // A lasting spell chains from its ending instead.
    let mut lasting = s.clone();
    lasting.duration = 2;
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    apply_spell(
        &effects_of(&lasting),
        &single(&lasting, &caster, 1),
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(state.chain, None);
}

// Who a summoned monster follows, on each cast path.
#[test]
fn summon_disposition_by_engine() {
    let s = spell(1, &[(Ability::Summon, 123)]);
    let caster = caster_at(10);
    let mut c = player(30, 30);
    let mut state = CastState::new();
    let on_self = Cast {
        path: CastPath::OnSelf,
        ..single(&s, &caster, 1)
    };
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &on_self,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Summon(123, SummonDisposition::CharmedByCaster)]
    );
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &single(&s, &caster, 1),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Summon(123, SummonDisposition::HuntsTarget)]
    );
    let known = known_monster(&[]);
    let mut m = active_monster();
    let mut t = Target::Monster(monster(&mut m, &known));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &single(&s, &caster, 1),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Summon(123, SummonDisposition::HuntsMonster)]
    );
}

// Open door: (spellcast - diff)/2 as the chance, instant only.
#[test]
fn open_door_chance_from_spellcasting_and_difficulty() {
    let mut s = spell(1, &[(Ability::OpenDoor, 0)]);
    s.diff = 20;
    let caster = caster_at(10);
    let cast = Cast {
        spell: &s,
        caster: Some(&caster),
        path: CastPath::OnSelf,
        power: 1,
    };
    let mut state = CastState::new();
    let mut t = Target::None;
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::OpenDoors { chance: 15 }]
    );
    let mut c = player(1, 1);
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::OpenDoors { chance: 15 }]
    );
    let mut lasting = s.clone();
    lasting.duration = 3;
    let cast = Cast {
        spell: &lasting,
        ..cast
    };
    let mut t = Target::None;
    assert_eq!(
        apply_spell(
            &effects_of(&lasting),
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Silly]
    );
}

// A cast on an item: Detect Magic reads the Magical value, Lore needs the item's level at
// or under the amount; damage on an item is silly.
#[test]
fn item_effects() {
    let mut it = item(
        7,
        ItemType::from_code(0),
        WornLocation::from_code(0),
        &[(Ability::Magical, 4)],
    );
    it.level = 10;
    let caster = caster_at(10);
    let mut state = CastState::new();
    let detect = spell(1, &[(Ability::DetectMagic, 0)]);
    let mut t = Target::Item(ItemTarget { item: &it });
    assert_eq!(
        apply_spell(
            &effects_of(&detect),
            &single(&detect, &caster, 1),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::DetectMagic { magical: 4 }]
    );
    let lore = spell(2, &[(Ability::Lore, 0)]);
    let mut t = Target::Item(ItemTarget { item: &it });
    assert_eq!(
        apply_spell(
            &effects_of(&lore),
            &single(&lore, &caster, 10),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Lore { revealed: true }]
    );
    let mut t = Target::Item(ItemTarget { item: &it });
    assert_eq!(
        apply_spell(
            &effects_of(&lore),
            &single(&lore, &caster, 9),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Lore { revealed: false }]
    );
    let dmg = spell(
        3,
        &[
            (Ability::DamageArmour, 0),
            (Ability::Heal, 0),
            (Ability::Shield, 1),
        ],
    );
    let mut t = Target::Item(ItemTarget { item: &it });
    assert_eq!(
        apply_spell(
            &effects_of(&dmg),
            &single(&dmg, &caster, 9),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Silly, Outcome::NoEffect, Outcome::Silly]
    );
}

// A room's spell on a player: the player effects act only for the kinds that name one
// player; an area kind gets nothing, not even the lasting add; Summon and Steal Hit
// Points need no duration; the stat alters also take Self.
#[test]
fn room_effects_need_a_single_target_kind() {
    let run = |ability: Ability, target: TargetKind, duration: i16| {
        let mut s = spell(1, &[(ability, 0)]);
        s.target = target;
        s.duration = duration;
        let mut c = player(20, 30);
        let cast = Cast {
            spell: &s,
            caster: None,
            path: CastPath::Room,
            power: 5,
        };
        let mut state = CastState::new();
        let mut t = Target::User(user(&mut c));
        let out = apply_spell(
            &effects_of(&s),
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[0]),
        );
        (out[0].clone(), c.hp)
    };
    for kind in [
        TargetKind::FullArea,
        TargetKind::FullAttack,
        TargetKind::DivArea,
        TargetKind::OnSelf,
    ] {
        assert_eq!(
            run(Ability::DamageNoArmour, kind, 0),
            (Outcome::NoEffect, 20),
            "{kind:?}"
        );
        assert_eq!(
            run(Ability::DamageNoArmour, kind, 5),
            (Outcome::NoEffect, 20),
            "{kind:?}"
        );
        assert_eq!(run(Ability::Heal, kind, 0).0, Outcome::NoEffect, "{kind:?}");
    }
    assert_eq!(
        run(Ability::DamageNoArmour, TargetKind::Any, 0),
        (
            Outcome::Applied {
                amount: 5,
                knocked_out: false
            },
            15
        )
    );
    assert!(matches!(
        run(Ability::DamageNoArmour, TargetKind::MonsterOrSingle, 5).0,
        Outcome::Lasting(_)
    ));
    // No duration only
    assert_eq!(
        run(Ability::StealHp, TargetKind::Single, 5),
        (Outcome::NoEffect, 20)
    );
    assert_eq!(
        run(Ability::Summon, TargetKind::Single, 5).0,
        Outcome::NoEffect
    );
    assert_eq!(
        run(Ability::Freedom, TargetKind::Single, 5).0,
        Outcome::NoEffect
    );
    assert!(matches!(
        run(Ability::Summon, TargetKind::Single, 0).0,
        Outcome::Summon(5, _)
    ));
    // Self too, never an area
    assert!(matches!(
        run(Ability::Intellect, TargetKind::OnSelf, 5).0,
        Outcome::Lasting(_)
    ));
    assert_eq!(
        run(Ability::Intellect, TargetKind::FullParty, 5).0,
        Outcome::NoEffect
    );
}

// A room's damage on a player has no caster bonus, the linear multiplier, and knocks out
// only from alive.
#[test]
fn room_damage_on_a_player() {
    let s = spell(
        1,
        &[(Ability::DamageNoArmour, 0), (Ability::DamageArmour, 0)],
    );
    let mut c = player(10, 50);
    c.mr = 100;
    let cast = Cast {
        spell: &s,
        caster: None,
        path: CastPath::Room,
        power: 8,
    };
    let mut state = CastState::new();
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out,
        vec![
            Outcome::Applied {
                amount: 8,
                knocked_out: false
            },
            Outcome::Applied {
                amount: 4,
                knocked_out: true
            }
        ]
    );
    assert_eq!(c.hp, -2);
    // Already down: no knock-out.
    let mut t = Target::User(user(&mut c));
    let out = apply_spell(
        &effects_of(&s),
        &cast,
        &mut t,
        &mut state,
        &mut Fixed::of(&[0]),
    );
    assert_eq!(
        out[0],
        Outcome::Applied {
            amount: 8,
            knocked_out: false
        }
    );
    // A lasting room spell takes the base duration.
    let mut l = spell(2, &[(Ability::Shield, 3)]);
    l.duration = 9;
    l.durlvl = 5;
    let cast = Cast { spell: &l, ..cast };
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&l),
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[99])
        ),
        vec![Outcome::Lasting(SlotOutcome::Added(0))]
    );
    assert_eq!(c.spellrnds[0], 9);
}

// ---------------------------------------------------------------------------------------
// Effects applied to a monster (the single-target cast and the area loop).

// Damage without armour on a monster: scaled, no duration test, then the survivor's
// turn-on-the-caster roll with followpct.
#[test]
fn damage_on_a_monster_scales_and_turns_it_on_the_caster() {
    let mut s = spell(1, &[(Ability::DamageNoArmour, 0)]);
    s.duration = 4;
    let mut known = known_monster(&[]);
    known.followpct = 60;
    let mut m = active_monster();
    m.hp = 100;
    let mut caster = caster_at(10);
    caster.spelldamage_pct = 25;
    let mut state = CastState::new();
    let mut t = Target::Monster(monster(&mut m, &known));
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 40),
        &mut t,
        &mut state,
        &mut Fixed::of(&[59]),
    );
    assert_eq!(applied(&out[0]), 50);
    assert_eq!(m.hp, 50);
    assert_eq!(m.following, "Thornwyn");
    assert_eq!(m.friendly, 0);
    assert_eq!(m.spells[0].spell, SpellNo(0));
    let mut m2 = active_monster();
    m2.hp = 100;
    let mut t = Target::Monster(monster(&mut m2, &known));
    apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 40),
        &mut t,
        &mut state,
        &mut Fixed::of(&[60]),
    );
    assert_eq!(m2.following, "");
    // An angel already following stops being friendly.
    let mut angel = active_monster();
    angel.hp = 100;
    angel.montype = MonsterType::Angel;
    angel.following = "thornwyn".to_owned();
    angel.friendly = 1;
    let mut t = Target::Monster(monster(&mut angel, &known));
    apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 40),
        &mut t,
        &mut state,
        &mut Fixed::of(&[1]),
    );
    assert_eq!(angel.friendly, 0);
}

// Damage on armour on a monster at mr 95 and with Anti Magic at 150.
#[test]
fn damage_on_armour_on_a_monster() {
    let s = spell(1, &[(Ability::DamageArmour, 0)]);
    let known = known_monster(&[]);
    let mut m = active_monster();
    m.hp = 200;
    let caster = caster_at(10);
    let mut state = CastState::new();
    let mut mt = monster(&mut m, &known);
    mt.magic_resistance = 95;
    let mut t = Target::Monster(mt);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 100),
        &mut t,
        &mut state,
        &mut Fixed::of(&[99]),
    );
    assert_eq!(applied(&out[0]), 78);
    let mut mt = monster(&mut m, &known);
    mt.magic_resistance = 150;
    mt.anti_magic = true;
    let mut t = Target::Monster(mt);
    let out = apply_spell(
        &effects_of(&s),
        &single(&s, &caster, 100),
        &mut t,
        &mut state,
        &mut Fixed::of(&[99]),
    );
    assert_eq!(applied(&out[0]), 25);
    assert_eq!(m.hp, 200 - 78 - 25);
}

// In an area the spell damage bonus compounds from one monster to the next, starting
// from the last player's share (sic).
#[test]
fn area_damage_on_armour_compounds_across_monsters() {
    let s = attack_spell(TargetKind::FullAttack);
    let mut s = s;
    s.abilities[0] = (Ability::DamageArmour, 0);
    let mut caster = caster_at(10);
    caster.spelldamage_pct = 100;
    let cast = Cast {
        spell: &s,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 10 },
        power: 10,
    };
    let known = known_monster(&[]);
    let mut state = CastState::new();
    let slot = effects_of(&s).slots[0];
    let mut a = active_monster();
    a.hp = 1000;
    let mut t = Target::Monster(monster(&mut a, &known));
    // 10 doubled = 20 at mr 1: a 0 cut and (50-1)% extra = 20*49/100 + 20 = 29.
    assert_eq!(
        applied(&apply_effect(
            &slot,
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[99])
        )),
        29
    );
    let mut b = active_monster();
    b.hp = 1000;
    let mut t = Target::Monster(monster(&mut b, &known));
    // The next monster starts from 20: 40, then 40*49/100 + 40 = 59.
    assert_eq!(
        applied(&apply_effect(
            &slot,
            &cast,
            &mut t,
            &mut state,
            &mut Fixed::of(&[99])
        )),
        59
    );
}

// Enslave: the caster's level must reach the charm level; the monster follows, friendly
// and charmed; a lasting cast also fills a slot.
#[test]
fn enslave_needs_the_charm_level() {
    let mut s = spell(1, &[(Ability::Enslave, 0)]);
    s.duration = 20;
    let mut known = known_monster(&[]);
    known.charmlvl = 12;
    let mut m = active_monster();
    let mut state = CastState::new();
    let low = caster_at(11);
    let mut t = Target::Monster(monster(&mut m, &known));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &single(&s, &low, 5),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::NoEffect]
    );
    assert!(m.following.is_empty());
    let ok = caster_at(12);
    let mut t = Target::Monster(monster(&mut m, &known));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &single(&s, &ok, 5),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Lasting(SlotOutcome::Added(0))]
    );
    assert_eq!(m.following, "Thornwyn");
    assert_eq!(m.friendly, 1);
    assert!(m.flags.contains(ActiveMonsterFlags::CHARMED));
    assert_eq!(m.spells[0].spell, SpellNo(1));
    // On a player it is silly.
    let mut c = player(1, 1);
    let mut t = Target::User(user(&mut c));
    assert_eq!(
        apply_spell(
            &effects_of(&s),
            &single(&s, &ok, 5),
            &mut t,
            &mut state,
            &mut Fixed::of(&[0])
        ),
        vec![Outcome::Silly]
    );
}

// Steal hit points in an area: the one case where a monster resists, after which the
// share is 0 for every later monster (sic).
#[test]
fn area_steal_hp_resist_zeroes_the_later_monsters() {
    let mut s = attack_spell(TargetKind::FullAttack);
    s.abilities[0] = (Ability::StealHp, 0);
    let caster = caster_at(10);
    let cast = Cast {
        spell: &s,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 10 },
        power: 10,
    };
    let known = known_monster(&[]);
    let slot = effects_of(&s).slots[0];
    let mut state = CastState::new();
    let mut a = active_monster();
    a.hp = 50;
    let mut t = Target::Monster(monster(&mut a, &known));
    assert_eq!(
        apply_effect(&slot, &cast, &mut t, &mut state, &mut Fixed::of(&[99])),
        Outcome::StolenHp { amount: 10 }
    );
    assert_eq!(a.hp, 40);
    let mut b = active_monster();
    b.hp = 50;
    let mut bt = monster(&mut b, &known);
    bt.magic_resistance = 100;
    let mut t = Target::Monster(bt);
    assert_eq!(
        apply_effect(&slot, &cast, &mut t, &mut state, &mut Fixed::of(&[50])),
        Outcome::Resisted
    );
    assert_eq!(b.hp, 50);
    let mut c = active_monster();
    c.hp = 50;
    let mut t = Target::Monster(monster(&mut c, &known));
    assert_eq!(
        apply_effect(&slot, &cast, &mut t, &mut state, &mut Fixed::of(&[99])),
        Outcome::StolenHp { amount: 0 }
    );
    assert_eq!(c.hp, 50);
}

// Heal on a monster caps at the known monster's maximum and may befriend it; Poison; the
// area uses the undivided amount; the area cure is inverted (sic).
#[test]
fn monster_heal_poison_and_cure() {
    let heal = spell(1, &[(Ability::Heal, 0)]);
    let mut known = known_monster(&[]);
    known.maxhp = 30;
    known.followpct = 40;
    let mut m = active_monster();
    m.hp = 25;
    m.maxhp = 50;
    let caster = caster_at(10);
    let mut state = CastState::new();
    let mut t = Target::Monster(monster(&mut m, &known));
    let out = apply_spell(
        &effects_of(&heal),
        &single(&heal, &caster, 20),
        &mut t,
        &mut state,
        &mut Fixed::of(&[41]),
    );
    assert_eq!(applied(&out[0]), 5);
    assert_eq!(
        (m.hp, m.friendly, m.following.as_str()),
        (30, 1, "Thornwyn")
    );
    let poison = spell(2, &[(Ability::Poison, 0)]);
    let area = Cast {
        spell: &poison,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 12 },
        power: 3,
    };
    let mut t = Target::Monster(monster(&mut m, &known));
    apply_spell(
        &effects_of(&poison),
        &area,
        &mut t,
        &mut state,
        &mut Fixed::of(&[99]),
    );
    assert_eq!(m.poison, 12);
    let cure = spell(3, &[(Ability::CurePoison, 0)]);
    let area = Cast {
        spell: &cure,
        caster: Some(&caster),
        path: CastPath::Area { undivided: 10 },
        power: 2,
    };
    let mut t = Target::Monster(monster(&mut m, &known));
    apply_spell(
        &effects_of(&cure),
        &area,
        &mut t,
        &mut state,
        &mut Fixed::of(&[99]),
    );
    assert_eq!(m.poison, 12);
    m.poison = 4;
    let mut t = Target::Monster(monster(&mut m, &known));
    apply_spell(
        &effects_of(&cure),
        &area,
        &mut t,
        &mut state,
        &mut Fixed::of(&[99]),
    );
    assert_eq!(m.poison, 0);
    // The single cure lowers and floors.
    m.poison = 4;
    let mut t = Target::Monster(monster(&mut m, &known));
    apply_spell(
        &effects_of(&cure),
        &single(&cure, &caster, 9),
        &mut t,
        &mut state,
        &mut Fixed::of(&[99]),
    );
    assert_eq!(m.poison, 0);
}

// The gates before a cast on a monster: Kill Dead, Affects Animal, Affects Living.
#[test]
fn monster_gates() {
    let kill = effects_of(&spell(
        1,
        &[(Ability::KillDead, 0), (Ability::DamageNoArmour, 0)],
    ));
    assert_eq!(
        monster_gate(&kill, false, false, false),
        Some(Outcome::Immune)
    );
    assert_eq!(monster_gate(&kill, true, false, false), None);
    let animal = effects_of(&spell(2, &[(Ability::AffectsAnimal, 0)]));
    assert_eq!(
        monster_gate(&animal, false, false, false),
        Some(Outcome::Immune)
    );
    assert_eq!(monster_gate(&animal, false, true, false), None);
    let living = effects_of(&spell(3, &[(Ability::AffectsLiving, 0)]));
    assert_eq!(
        monster_gate(&living, false, false, true),
        Some(Outcome::Immune)
    );
    assert_eq!(monster_gate(&living, false, false, false), None);
}

// ---------------------------------------------------------------------------------------
// Property tests.

proptest! {
    // The roll never leaves [min, max] at the level.
    #[test]
    fn amount_never_leaves_the_level_range(
        minval in -300i16..300, maxval in -300i16..300, mininc in 0u8..10, minlvls in 0u8..5,
        maxinc in 0u8..10, maxlvls in 0u8..5, levelcap in 0i16..40, level in 0u16..60, seed in any::<u64>(),
    ) {
        let mut s = growing_spell(minval, maxval, mininc, minlvls, maxinc, maxlvls);
        s.levelcap = levelcap;
        let range = amount_range(&s, level);
        prop_assert!(range.min <= range.max);
        let mut rng = SeededRng::new(seed);
        for _ in 0..8 {
            let r = roll_amount(&s, level, &mut rng);
            prop_assert!(r >= range.min && r <= range.max, "{r} outside {range:?}");
        }
    }

    // With a non-negative duration bonus a duration is never below the base.
    #[test]
    fn duration_never_below_base(
        base in 0i16..200, durinc in 0u8..10, durlvls in 0u8..5, durlvl in 0i16..10,
        levelcap in 0i16..40, level in 0u16..60, pct in 0i32..200, seed in any::<u64>(),
    ) {
        let mut s = lasting_spell(1, base);
        s.durinc = durinc;
        s.durlvls = durlvls;
        s.durlvl = durlvl;
        s.levelcap = levelcap;
        let c = CasterLevel { level, spellduration_pct: pct };
        let d = duration(&s, Some(c), &mut SeededRng::new(seed));
        prop_assert!(d >= i32::from(base), "{d} < {base}");
        prop_assert_eq!(duration(&s, None, &mut SeededRng::new(seed)), i32::from(base));
    }

    // Every SpellEffect on every Target kind, on every path, either applies or returns an
    // outcome without panicking.
    #[test]
    fn every_effect_on_every_target_has_an_outcome(
        ability in 0u16..190, value in -50i16..50, power in -100i32..100, lasting in any::<bool>(),
        target in 0u16..14, path in 0u8..4, seed in any::<u64>(),
    ) {
        let mut s = spell(1, &[(Ability::from_code(ability), value)]);
        s.duration = if lasting { 5 } else { 0 };
        s.target = TargetKind::from_code(target);
        s.spell_type = SpellType::Autocombat;
        let effects = effects_of(&s);
        let caster = caster_at(10);
        let cast = Cast {
            spell: &s,
            caster: if path == 3 { None } else { Some(&caster) },
            path: match path {
                0 => CastPath::OnSelf,
                1 => CastPath::Area { undivided: power * 2 },
                2 => CastPath::Single,
                _ => CastPath::Room,
            },
            power,
        };
        let mut rng = SeededRng::new(seed);
        let mut state = CastState::new();
        let mut c = player(20, 30);
        c.spells[0] = SpellNo(1);
        let mut t = Target::User(user(&mut c));
        let out = apply_spell(&effects, &cast, &mut t, &mut state, &mut rng);
        prop_assert_eq!(out.len(), effects.slots.len());
        let known = known_monster(&[]);
        let mut m = active_monster();
        m.hp = 20;
        let mut t = Target::Monster(monster(&mut m, &known));
        apply_spell(&effects, &cast, &mut t, &mut state, &mut rng);
        let it = item(1, ItemType::from_code(0), WornLocation::from_code(0), &[]);
        let mut t = Target::Item(ItemTarget { item: &it });
        apply_spell(&effects, &cast, &mut t, &mut state, &mut rng);
        let mut t = Target::None;
        apply_spell(&effects, &cast, &mut t, &mut state, &mut rng);
        apply_round(&mut c, &effects, power, &mut rng);
        apply_termination(&mut c, &s, power, true, &mut rng);
        apply_monster_round(&mut m, &effects, power, &mut rng);
        apply_monster_termination(&mut m, &s, power);
    }
}

// ---------------------------------------------------------------------------------------
// The monster-side engines (T-049b): apply_monster_cast, apply_monster_cast_area,
// apply_monster_self_spell and apply_room_on_monster, census sections C.2 and D.

/// A monster cast of `spell` with the given base value and rounds (the single-target
/// cast's value after the element cut and its rounds; the area cast's `baseroll` and
/// rounds).
fn mcast<'a>(spell: &'a Spell, power: i32, duration: i32) -> MonsterCast<'a> {
    MonsterCast {
        spell,
        power,
        share: power,
        duration,
    }
}

/// A monster with `hp` of `maxhp` (active record) whose known record has `known_maxhp`.
fn caster_monster(hp: i16, maxhp: i16, known_maxhp: i16) -> (ActiveMonster, KnownMonster) {
    let mut m = active_monster();
    m.hp = hp;
    m.maxhp = maxhp;
    let mut k = known_monster(&[]);
    k.maxhp = known_maxhp;
    k.genindex = 7;
    (m, k)
}

fn single_spell(abilities: &[(Ability, i16)]) -> Spell {
    let mut s = spell(21, abilities);
    s.target = TargetKind::Single;
    s
}

fn area_spell(kind: TargetKind, abilities: &[(Ability, i16)]) -> Spell {
    let mut s = spell(22, abilities);
    s.target = kind;
    s
}

fn monster_spell(abilities: &[(Ability, i16)]) -> Spell {
    let mut s = spell(23, abilities);
    s.target = TargetKind::Monster;
    s.maxval = 100;
    s
}

fn cast_one(
    s: &Spell,
    power: i32,
    duration: i32,
    c: &mut Character,
    state: &mut MonsterCastState,
) -> Vec<Outcome> {
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut u = user(c);
    apply_monster_cast_spell(
        &effects_of(s),
        &mcast(s, power, duration),
        &mut caster,
        &mut u,
        state,
    )
}

// The monster's cast level grows the range with no level cap; the duration adds durinc
// per durlvls of the slot level, dividing first.
#[test]
fn monster_amount_range_and_duration_have_no_cap() {
    let mut s = growing_spell(10, 20, 1, 2, 2, 2);
    s.levelcap = 5;
    assert_eq!(amount_range(&s, 30), AmountRange { min: 12, max: 25 });
    assert_eq!(
        monster_amount_range(&s, 30),
        AmountRange { min: 25, max: 50 }
    );
    assert_eq!(
        monster_amount_range(&s, 0),
        AmountRange { min: 10, max: 20 }
    );
    let mut rng = Fixed::of(&[0, 25]);
    assert_eq!(roll_monster_amount(&s, 30, &mut rng), 25);
    assert_eq!(roll_monster_amount(&s, 30, &mut rng), 50);
    s.duration = 4;
    s.durlvls = 3;
    s.durinc = 2;
    assert_eq!(monster_duration(&s, Some(7)), 4 + 7 / 3 * 2);
    assert_eq!(monster_duration(&s, Some(7)), 8);
    assert_eq!(monster_duration(&s, None), 4);
    s.durlvls = 0;
    assert_eq!(monster_duration(&s, Some(7)), 4);
}

// The player's roll against a monster or a room caps at 97 (the player engines at 98); a
// room draws before the items.
#[test]
fn player_resist_roll_against_a_monster_caps_at_97() {
    let s = resist_spell(ResistMode::Always);
    assert!(player_resists(&s, 300, false, false, &mut Fixed::of(&[98])));
    assert!(!player_resists_monster_cast(
        &s,
        300,
        false,
        false,
        &mut Fixed::of(&[98])
    ));
    assert!(player_resists_monster_cast(
        &s,
        300,
        false,
        false,
        &mut Fixed::of(&[97])
    ));
    assert!(player_resists_monster_cast(
        &s,
        40,
        false,
        false,
        &mut Fixed::of(&[20])
    ));
    assert!(!player_resists_monster_cast(
        &s,
        40,
        false,
        false,
        &mut Fixed::of(&[21])
    ));
    assert!(player_resists_monster_cast(
        &s,
        0,
        false,
        true,
        &mut Fixed::of(&[100])
    ));
    let mut rng = Fixed::of(&[100, 50]);
    assert!(player_resists_room_cast(&s, 300, false, true, &mut rng));
    assert_eq!(rng.next, 1);
    assert!(player_resists_room_cast(&s, 300, false, false, &mut rng));
    let never = resist_spell(ResistMode::Never);
    assert!(!player_resists_monster_cast(
        &never,
        300,
        false,
        false,
        &mut Fixed::of(&[1])
    ));
}

// The value as it is (no caster bonus), knocked out from above 0; the lasting form keeps
// the computed rounds with no adjustment.
#[test]
fn monster_cast_damage_has_no_caster_bonus() {
    let s = single_spell(&[(Ability::DamageNoArmour, 0)]);
    let mut c = player(10, 30);
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&s, 12, 0, &mut c, &mut state),
        vec![Outcome::Applied {
            amount: 12,
            knocked_out: true
        }]
    );
    assert_eq!(c.hp, -2);
    let mut c = player(10, 30);
    assert_eq!(
        cast_one(&s, 12, 7, &mut c, &mut state),
        vec![Outcome::Lasting(SlotOutcome::Added(NSPELLCAST - 1))]
    );
    assert_eq!(c.hp, 10);
    assert_eq!(
        (c.spells[9], c.spellvals[9], c.spellrnds[9]),
        (SpellNo(21), 12, 7)
    );
    assert!(state.added && !state.refused);
    // An area kind never reaches the player through this engine.
    let mut state = MonsterCastState::new();
    let area = area_spell(TargetKind::FullArea, &[(Ability::DamageNoArmour, 0)]);
    assert_eq!(
        cast_one(&area, 12, 0, &mut c, &mut state),
        vec![Outcome::NoEffect]
    );
}

// A weaker or equal renewal is refused and the cast ends (energy refunded by the
// engine); a stronger one renews in place.
#[test]
fn monster_cast_renews_only_when_stronger_and_a_refusal_ends_the_cast() {
    let s = single_spell(&[(Ability::DamageNoArmour, 0), (Ability::Energy, 0)]);
    let mut c = player(10, 30);
    let mut state = MonsterCastState::new();
    assert_eq!(cast_one(&s, 12, 7, &mut c, &mut state).len(), 2);
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&s, 12, 9, &mut c, &mut state),
        vec![Outcome::Lasting(SlotOutcome::AlreadyActive)]
    );
    assert!(state.refused);
    assert_eq!((c.spellvals[9], c.spellrnds[9]), (12, 7));
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&s, 13, 9, &mut c, &mut state),
        vec![
            Outcome::Lasting(SlotOutcome::Renewed(NSPELLCAST - 1)),
            Outcome::NoEffect
        ]
    );
    assert_eq!((c.spellvals[9], c.spellrnds[9]), (13, 9));
}

// The magic-resistance multiplier on the raw value, whatever the duration, never added;
// the message names the unsoftened value and the knock-out test uses the softened one.
#[test]
fn monster_cast_damage_on_armour_reports_the_unsoftened_value() {
    let s = single_spell(&[(Ability::DamageArmour, 0)]);
    let mut c = player(80, 100);
    c.mr = 95;
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&s, 100, 5, &mut c, &mut state),
        vec![Outcome::Applied {
            amount: 100,
            knocked_out: false
        }]
    );
    assert_eq!(c.hp, 80 - 78);
    assert_eq!(c.spells[9], SpellNo(0));
    // Knocked out by the softened 78, which the shown 100 would also have done; at hp 79
    // the softened hit takes him to 1 and the test with the shown value would be wrong.
    let mut c = player(78, 100);
    c.mr = 95;
    assert_eq!(
        cast_one(&s, 100, 0, &mut c, &mut MonsterCastState::new()),
        vec![Outcome::Applied {
            amount: 100,
            knocked_out: true
        }]
    );
    let mut c = player(79, 100);
    c.mr = 95;
    assert_eq!(
        cast_one(&s, 100, 0, &mut c, &mut MonsterCastState::new()),
        vec![Outcome::Applied {
            amount: 100,
            knocked_out: false
        }]
    );
    assert_eq!(c.hp, 1);
}

// Every active spell ends, the value ignored (the player engine ends those with ability
// `value`).
#[test]
fn monster_cast_dispel_ends_every_spell_whatever_the_value() {
    let hold = spell(11, &[(Ability::HoldPerson, 1)]);
    let bless = spell(12, &[(Ability::AvBless, 3)]);
    let lookup = move |n: SpellNo| -> Option<&'static Spell> {
        let s = match n {
            SpellNo(11) => hold.clone(),
            SpellNo(12) => bless.clone(),
            _ => return None,
        };
        Some(Box::leak(Box::new(s)))
    };
    let s = single_spell(&[(Ability::DispelMagic, 150)]);
    let mut c = player(30, 30);
    c.spells[0] = SpellNo(12);
    c.spells[1] = SpellNo(11);
    c.spells[2] = SpellNo(99);
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut u = user(&mut c);
    u.spells = &lookup;
    let mut state = MonsterCastState::new();
    let out = apply_monster_cast_spell(
        &effects_of(&s),
        &mcast(&s, 1, 0),
        &mut caster,
        &mut u,
        &mut state,
    );
    assert_eq!(
        out,
        vec![Outcome::Ended {
            spells: vec![
                EndedSpell {
                    number: SpellNo(12),
                    power: 0,
                    chain: true
                },
                EndedSpell {
                    number: SpellNo(11),
                    power: 0,
                    chain: true
                }
            ],
            poison: None
        }]
    );
    assert!(c.spells.iter().all(|&s| s == SpellNo(0)));
}

// Only items whose Cursed value is set come off; no Major Curse clause (the player
// engine strips those below the caster's level); Self or Single and Any only.
#[test]
fn monster_cast_remove_curse_takes_only_cursed_items() {
    let cursed = item(
        1,
        ItemType::from_code(0),
        WornLocation::from_code(0),
        &[(Ability::Cursed, 1)],
    );
    let major = item(
        2,
        ItemType::from_code(0),
        WornLocation::from_code(0),
        &[(Ability::MajorCurse, 1)],
    );
    let lookup = move |n: ItemNo| -> Option<&'static Item> {
        let it = match n {
            ItemNo(1) => cursed.clone(),
            ItemNo(2) => major.clone(),
            _ => return None,
        };
        Some(Box::leak(Box::new(it)))
    };
    let mut s = single_spell(&[(Ability::RemoveCurse, 0)]);
    s.target = TargetKind::Any;
    let mut c = player(30, 30);
    c.worn[2] = ItemNo(1);
    c.worn[5] = ItemNo(2);
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let cast = |s: &Spell, duration: i32, c: &mut Character, caster: &mut MonsterCaster<'_>| {
        let mut u = user(c);
        u.items = &lookup;
        apply_monster_cast(
            &effects_of(s).slots[0],
            &mcast(s, 1, duration),
            caster,
            &mut u,
            &mut MonsterCastState::new(),
        )
    };
    assert_eq!(
        cast(&s, 0, &mut c, &mut caster),
        Outcome::RemoveCurse {
            worn_slots: vec![2]
        }
    );
    s.target = TargetKind::Single;
    assert_eq!(cast(&s, 0, &mut c, &mut caster), Outcome::NoEffect);
    s.target = TargetKind::SelfOrSingle;
    s.duration = 2;
    assert_eq!(cast(&s, 2, &mut c, &mut caster), Outcome::NoEffect);
}

// The poison spell ends and the level falls with no floor (the player engine floors at
// 0).
#[test]
fn monster_cast_cure_poison_has_no_floor() {
    let venom = {
        let mut v = spell(31, &[(Ability::Poison, 4)]);
        v.element = Element::Poison;
        v
    };
    let lookup = move |n: SpellNo| -> Option<&'static Spell> {
        (n == SpellNo(31)).then(|| &*Box::leak(Box::new(venom.clone())))
    };
    let s = single_spell(&[(Ability::CurePoison, 0)]);
    let mut c = player(30, 30);
    c.poison = 3;
    c.spells[1] = SpellNo(31);
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut u = user(&mut c);
    u.spells = &lookup;
    let out = apply_monster_cast(
        &effects_of(&s).slots[0],
        &mcast(&s, 5, 0),
        &mut caster,
        &mut u,
        &mut MonsterCastState::new(),
    );
    let cut = PoisonCut {
        amount: 5,
        floor: false,
    };
    assert_eq!(
        out,
        Outcome::Ended {
            spells: vec![EndedSpell {
                number: SpellNo(31),
                power: 0,
                chain: true
            }],
            poison: Some(cut)
        }
    );
    assert_eq!(c.poison, 3);
    assert_eq!(c.spells[1], SpellNo(0));
    // The engine's order: the poison spell's ending takes its 4 off first (never below 0),
    // then the cure's 5, with no floor.
    let venom = lookup(SpellNo(31)).unwrap();
    apply_termination(&mut c, venom, 0, true, &mut Fixed::of(&[0]));
    cut_poison(&mut c, cut);
    assert_eq!(c.poison, -5);
}

// The summon is ranged by the caster's generation index and hunts the target; the stolen
// gain caps at the known monster's maximum.
#[test]
fn monster_cast_summon_range_and_steal_cap() {
    let s = single_spell(&[(Ability::Summon, 44)]);
    let mut c = player(30, 30);
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&s, 1, 0, &mut c, &mut state),
        vec![Outcome::Summon(
            44,
            SummonDisposition::HuntsTargetByCasterIndex
        )]
    );
    assert_eq!(
        cast_one(&s, 1, 3, &mut c, &mut state),
        vec![Outcome::NoEffect]
    );
    let steal = single_spell(&[(Ability::StealHp, 0)]);
    let (mut m, k) = caster_monster(35, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut u = user(&mut c);
    let out = apply_monster_cast(
        &effects_of(&steal).slots[0],
        &mcast(&steal, 12, 0),
        &mut caster,
        &mut u,
        &mut MonsterCastState::new(),
    );
    assert_eq!(out, Outcome::StolenHp { amount: 12 });
    assert_eq!((c.hp, m.hp), (18, 40));
}

// Slay reads the record's own duration; a spell whose rounds come only from the slot
// level still slays while Inflict Damage becomes lasting.
#[test]
fn monster_cast_slay_reads_the_record_duration() {
    let s = single_spell(&[(Ability::DamageNoArmour, 0), (Ability::Slay, 0)]);
    let mut c = player(30, 30);
    let mut state = MonsterCastState::new();
    let out = cast_one(&s, 5, 4, &mut c, &mut state);
    assert_eq!(
        out,
        vec![
            Outcome::Lasting(SlotOutcome::Added(NSPELLCAST - 1)),
            Outcome::Applied {
                amount: 30 + 200 + 1,
                knocked_out: true
            },
        ]
    );
    assert_eq!(c.hp, -201);
}

// The cast ends at a death; a lasting Poison adds first and raises only when not refused;
// a stat alter with `extend = 0` refuses an active instance and ends the cast, and
// raises when an earlier slot added the spell.
#[test]
fn monster_cast_stops_at_a_death_and_orders_poison_and_stat_alters() {
    let s = single_spell(&[(Ability::DamageNoArmour, 0), (Ability::Energy, 0)]);
    let mut c = player(5, 30);
    let mut state = MonsterCastState::new();
    assert_eq!(cast_one(&s, 300, 0, &mut c, &mut state).len(), 1);
    assert_eq!(c.hp, -295);

    let venom = single_spell(&[(Ability::Poison, 0)]);
    let mut c = player(30, 30);
    c.poison = 2;
    let mut state = MonsterCastState::new();
    assert_eq!(cast_one(&venom, 6, 3, &mut c, &mut state).len(), 1);
    assert_eq!(c.poison, 6);
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&venom, 5, 3, &mut c, &mut state),
        vec![Outcome::Lasting(SlotOutcome::AlreadyActive)]
    );
    assert_eq!(c.poison, 6);
    let mut c = player(30, 30);
    let mut u = user(&mut c);
    u.poison_immune = true;
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    assert_eq!(
        apply_monster_cast(
            &effects_of(&venom).slots[0],
            &mcast(&venom, 6, 0),
            &mut caster,
            &mut u,
            &mut MonsterCastState::new(),
        ),
        Outcome::Immune
    );

    let buff = single_spell(&[(Ability::Strength, 3), (Ability::AlterHp, 2)]);
    let mut c = player(30, 30);
    c.str = 10;
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&buff, 1, 5, &mut c, &mut state),
        vec![
            Outcome::Lasting(SlotOutcome::Added(NSPELLCAST - 1)),
            Outcome::NoEffect
        ]
    );
    assert_eq!((c.str, c.maxhp, c.hp), (13, 32, 32));
    let mut state = MonsterCastState::new();
    assert_eq!(
        cast_one(&buff, 9, 5, &mut c, &mut state),
        vec![Outcome::Lasting(SlotOutcome::AlreadyActive)]
    );
    assert!(state.refused);
    assert_eq!((c.str, c.maxhp), (13, 32));
    assert_eq!(
        cast_one(&buff, 1, 0, &mut c, &mut MonsterCastState::new()),
        vec![Outcome::NoEffect, Outcome::NoEffect]
    );
}

// Hunger, Thirst, Text Block and the rest of the no-effect list do nothing in an area
// cast but act on a single target; a marker with a duration goes to the lasting
// default.
#[test]
fn monster_area_no_effect_list_against_the_single_engine() {
    for a in [
        Ability::Hunger,
        Ability::Thirst,
        Ability::DispelMagic,
        Ability::Freedom,
    ] {
        assert!(area_no_effect(a), "{a:?}");
    }
    assert!(!area_no_effect(Ability::Heal) && !area_no_effect(Ability::NonMagical));
    let s = single_spell(&[(Ability::Hunger, 3), (Ability::TextBlock, 9)]);
    let mut c = player(30, 30);
    assert_eq!(
        cast_one(&s, 1, 0, &mut c, &mut MonsterCastState::new()),
        vec![
            Outcome::Applied {
                amount: 3,
                knocked_out: false
            },
            Outcome::TextBlock(9)
        ]
    );
    assert_eq!(c.hunger, 3);
    let marker = single_spell(&[(Ability::DescMsg, 1)]);
    assert_eq!(
        cast_one(&marker, 1, 4, &mut c, &mut MonsterCastState::new()),
        vec![Outcome::Lasting(SlotOutcome::Added(NSPELLCAST - 1))]
    );
    let mut rng = Fixed::of(&[0]);
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let area = area_spell(
        TargetKind::FullArea,
        &[(Ability::Hunger, 3), (Ability::TextBlock, 9)],
    );
    let mut p = player(30, 30);
    let mut players = [user(&mut p)];
    let out = apply_monster_cast_area_spell(
        &effects_of(&area),
        &mcast(&area, 1, 0),
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    let _ = &mut rng;
    assert!(
        out.iter()
            .all(|o| o.caster == Outcome::NoEffect && o.players.is_empty())
    );
    assert_eq!(p.hunger, 0);
}

/// Three marked players for the area tests: hp 20 of 30, the second with 50% element
/// resistance, the third immune to poison.
fn three_players() -> [Character; 3] {
    [player(20, 30), player(20, 30), player(20, 30)]
}

fn marked<'a>(ps: &'a mut [Character; 3]) -> [UserTarget<'a, 'static>; 3] {
    let [a, b, c] = ps;
    let mut b = user(b);
    b.element_resist_pct = 50;
    let mut c = user(c);
    c.poison_immune = true;
    [user(a), b, c]
}

fn area_one(
    s: &Spell,
    power: i32,
    share: i32,
    duration: i32,
    caster: &mut MonsterCaster<'_>,
    players: &mut [UserTarget<'_, '_>],
    state: &mut MonsterCastState,
) -> AreaOutcome {
    let cast = MonsterCast {
        spell: s,
        power,
        share,
        duration,
    };
    apply_monster_cast_area(&effects_of(s).slots[0], &cast, caster, players, state)
}

// An instant area Inflict Damage hurts nobody for Divided Area, Divided Attack, Full Area
// and Full Attack, and hurts every marked player by his element share for the
// inclusive kinds; the self kinds get a lasting effect on the caster only.
#[test]
fn area_damage_hits_only_the_inclusive_kinds() {
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    for kind in [
        TargetKind::DivArea,
        TargetKind::DivAttack,
        TargetKind::FullArea,
        TargetKind::FullAttack,
    ] {
        let s = area_spell(kind, &[(Ability::DamageNoArmour, 0)]);
        let mut ps = three_players();
        let mut players = marked(&mut ps);
        let out = area_one(
            &s,
            30,
            10,
            0,
            &mut caster,
            &mut players,
            &mut MonsterCastState::new(),
        );
        assert_eq!(out.players, vec![Outcome::NoEffect; 3], "{kind:?}");
        assert!(ps.iter().all(|p| p.hp == 20), "{kind:?}");
    }
    for kind in [
        TargetKind::DivAreaIncl,
        TargetKind::FullParty,
        TargetKind::DivParty,
    ] {
        let s = area_spell(kind, &[(Ability::DamageNoArmour, 0)]);
        let mut ps = three_players();
        let mut players = marked(&mut ps);
        let mut state = MonsterCastState::new();
        let out = area_one(&s, 30, 20, 0, &mut caster, &mut players, &mut state);
        assert_eq!(
            out.players,
            vec![
                Outcome::Applied {
                    amount: 20,
                    knocked_out: true
                },
                Outcome::Applied {
                    amount: 10,
                    knocked_out: false
                },
                Outcome::Applied {
                    amount: 20,
                    knocked_out: true
                },
            ],
            "{kind:?}"
        );
        assert_eq!([ps[0].hp, ps[1].hp, ps[2].hp], [0, 10, 0]);
        assert!(state.added);
    }
    let s = area_spell(TargetKind::Monster, &[(Ability::DamageNoArmour, 0)]);
    let mut ps = three_players();
    let mut players = marked(&mut ps);
    let out = area_one(
        &s,
        30,
        10,
        6,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(out.caster, Outcome::Lasting(SlotOutcome::Added(0)));
    assert!(out.players.is_empty());
    assert_eq!(
        (m.spells[0].spell, m.spells[0].value, m.spells[0].rounds),
        (SpellNo(22), 30, 6)
    );
    assert_eq!(m.hp, 10);
}

// An area Inflict Damage on Armour damages at once whatever the duration and is never
// added; the cut is cleared per player, unlike the player engine's carry; a later
// lasting slot is then skipped because `added` is set.
#[test]
fn area_damage_on_armour_is_instant_even_when_lasting() {
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut s = area_spell(
        TargetKind::FullArea,
        &[(Ability::DamageArmour, 0), (Ability::Energy, 0)],
    );
    s.duration = 9;
    let mut ps = [player(100, 100), player(100, 100), player(100, 100)];
    ps[0].mr = 95;
    ps[1].mr = 50;
    ps[2].mr = 50;
    let [pa, pb, pc] = &mut ps;
    let mut players = [user(pa), user(pb), user(pc)];
    players[2].anti_magic = true;
    let mut state = MonsterCastState::new();
    let out = apply_monster_cast_area_spell(
        &effects_of(&s),
        &MonsterCast {
            spell: &s,
            power: 50,
            share: 50,
            duration: 9,
        },
        &mut caster,
        &mut players,
        &mut state,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(
        out[0].players,
        vec![
            Outcome::Applied {
                amount: 50 - 11,
                knocked_out: false
            },
            Outcome::Applied {
                amount: 50,
                knocked_out: false
            },
            Outcome::Applied {
                amount: 50 - 12,
                knocked_out: false
            },
        ]
    );
    assert_eq!(out[1].players, vec![Outcome::NoEffect; 3]);
    assert_eq!([ps[0].hp, ps[1].hp, ps[2].hp], [61, 50, 62]);
    assert!(ps.iter().all(|p| p.spells[9] == SpellNo(0)));
}

// The casting monster gains each player's element share, capped at the active record's
// maximum (not the known monster's); the self kinds and a lasting area cast do
// nothing.
#[test]
fn area_steal_hp_feeds_the_caster_capped_at_the_active_maximum() {
    let (mut m, k) = caster_monster(30, 40, 80);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let s = area_spell(TargetKind::FullAttack, &[(Ability::StealHp, 0)]);
    let mut ps = three_players();
    let mut players = marked(&mut ps);
    let mut state = MonsterCastState::new();
    let out = area_one(&s, 8, 8, 0, &mut caster, &mut players, &mut state);
    assert_eq!(
        out.players,
        vec![
            Outcome::StolenHp { amount: 8 },
            Outcome::StolenHp { amount: 4 },
            Outcome::StolenHp { amount: 8 },
        ]
    );
    assert_eq!(caster.monster.hp, 40);
    assert_eq!([ps[0].hp, ps[1].hp, ps[2].hp], [12, 16, 12]);
    assert!(state.added);
    let mut lasting = s.clone();
    lasting.duration = 3;
    let mut players = marked(&mut ps);
    let mut state = MonsterCastState::new();
    let out = area_one(&lasting, 8, 8, 3, &mut caster, &mut players, &mut state);
    assert!(out.players.is_empty() && !state.added);
    let myself = area_spell(TargetKind::OnSelf, &[(Ability::StealHp, 0)]);
    let out = area_one(&myself, 8, 8, 0, &mut caster, &mut players, &mut state);
    assert_eq!(out.caster, Outcome::NoEffect);
    assert_eq!(caster.monster.hp, 40);
}

// A monster's summon has nothing set on it; the area kinds summon nothing, and
// TGT_MONSTER is not a self kind here.
#[test]
fn area_and_self_summons_set_nothing_on_the_summon() {
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut ps = three_players();
    let mut players = marked(&mut ps);
    let s = area_spell(TargetKind::SelfOrSingle, &[(Ability::Summon, 77)]);
    let out = area_one(
        &s,
        1,
        1,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(out.caster, Outcome::Summon(77, SummonDisposition::Plain));
    for kind in [TargetKind::FullArea, TargetKind::Monster] {
        let s = area_spell(kind, &[(Ability::Summon, 77)]);
        let out = area_one(
            &s,
            1,
            1,
            0,
            &mut caster,
            &mut players,
            &mut MonsterCastState::new(),
        );
        assert_eq!(out.caster, Outcome::NoEffect, "{kind:?}");
    }
    let mut lasting = spell(5, &[(Ability::Summon, 77), (Ability::Poison, 0)]);
    lasting.duration = 3;
    assert_eq!(
        apply_monster_self_spell(&lasting, 4, &mut caster),
        vec![
            Outcome::Lasting(SlotOutcome::Added(0)),
            Outcome::Applied {
                amount: 4,
                knocked_out: false
            }
        ]
    );
    assert_eq!(
        (
            caster.monster.poison,
            caster.monster.spells[0].rounds,
            caster.monster.spells[0].value
        ),
        (4, 3, 4)
    );
    assert_eq!(
        apply_monster_self_spell(&lasting, 4, &mut caster),
        vec![
            Outcome::Lasting(SlotOutcome::Renewed(0)),
            Outcome::Applied {
                amount: 4,
                knocked_out: false
            }
        ]
    );
    assert_eq!(caster.monster.poison, 8);
    lasting.duration = 0;
    assert_eq!(
        apply_monster_self_spell(&lasting, 4, &mut caster),
        vec![Outcome::Summon(77, SummonDisposition::Plain)]
    );
    assert_eq!(caster.monster.poison, 8);
}

// The caster's poison falls by the undivided roll, never below 0, and the message names
// the share; the area form skips the immune (sic) and has no floor; Poison raises the
// marked, immune skipped, before the add.
#[test]
fn area_cure_poison_skips_the_immune_and_has_no_floor() {
    let (mut m, k) = caster_monster(10, 50, 40);
    m.poison = 5;
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut ps = three_players();
    ps.iter_mut().for_each(|p| p.poison = 3);
    let mut players = marked(&mut ps);
    let s = area_spell(TargetKind::Any, &[(Ability::CurePoison, 9)]);
    let mut state = MonsterCastState::new();
    let out = area_one(&s, 2, 1, 4, &mut caster, &mut players, &mut state);
    assert_eq!(
        out.caster,
        Outcome::Applied {
            amount: 1,
            knocked_out: false
        }
    );
    assert_eq!(caster.monster.poison, 0);
    assert!(state.added);
    let s = area_spell(TargetKind::DivArea, &[(Ability::CurePoison, 9)]);
    let out = area_one(
        &s,
        2,
        5,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(
        out.players,
        vec![
            Outcome::Applied {
                amount: 5,
                knocked_out: false
            },
            Outcome::Applied {
                amount: 5,
                knocked_out: false
            },
            Outcome::Immune,
        ]
    );
    assert_eq!([ps[0].poison, ps[1].poison, ps[2].poison], [-2, -2, 3]);
    let venom = area_spell(TargetKind::DivArea, &[(Ability::Poison, 9)]);
    let mut players = marked(&mut ps);
    let out = area_one(
        &venom,
        2,
        6,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(out.players[2], Outcome::Immune);
    assert_eq!([ps[0].poison, ps[1].poison, ps[2].poison], [6, 6, 3]);
    let mut lasting = venom.clone();
    lasting.duration = 2;
    let mut players = marked(&mut ps);
    let out = area_one(
        &lasting,
        2,
        7,
        2,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(
        out.players,
        vec![
            Outcome::Lasting(SlotOutcome::Added(9)),
            Outcome::Lasting(SlotOutcome::Added(9)),
            Outcome::Lasting(SlotOutcome::Added(9)),
        ]
    );
    assert_eq!([ps[0].poison, ps[1].poison, ps[2].poison], [7, 7, 3]);
    assert_eq!(
        [ps[0].spellvals[9], ps[1].spellvals[9], ps[2].spellvals[9]],
        [7, 3, 7]
    );
}

// A slot's own value reaches only the effects on the caster and the room; every
// per-player instant effect uses the divided base roll (sic). The caster heals by the
// roll up to the active record's maximum; the players by the share up to theirs; mana
// never below 0; energy by the element share.
#[test]
fn area_fixed_slot_values_are_ignored_by_per_player_effects() {
    let (mut m, k) = caster_monster(45, 50, 80);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let heal = area_spell(TargetKind::OnSelf, &[(Ability::Heal, 7)]);
    let mut ps = three_players();
    let mut players = marked(&mut ps);
    let out = area_one(
        &heal,
        2,
        1,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(
        out.caster,
        Outcome::Applied {
            amount: 5,
            knocked_out: false
        }
    );
    assert_eq!(caster.monster.hp, 50);
    let heal = area_spell(TargetKind::FullArea, &[(Ability::Heal, 7)]);
    ps[0].hp = 29;
    let mut players = marked(&mut ps);
    let out = area_one(
        &heal,
        2,
        3,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(
        out.players,
        vec![
            Outcome::Applied {
                amount: 1,
                knocked_out: false
            },
            Outcome::Applied {
                amount: 3,
                knocked_out: false
            },
            Outcome::Applied {
                amount: 3,
                knocked_out: false
            },
        ]
    );
    assert_eq!([ps[0].hp, ps[1].hp, ps[2].hp], [30, 23, 23]);
    let mana = area_spell(TargetKind::FullArea, &[(Ability::HealMana, 7)]);
    let mut players = marked(&mut ps);
    let out = area_one(
        &mana,
        2,
        -25,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(applied(&out.players[0]), -10);
    assert_eq!(ps[0].mana, 0);
    let energy = area_spell(TargetKind::FullArea, &[(Ability::Energy, 7)]);
    let mut players = marked(&mut ps);
    let out = area_one(
        &energy,
        2,
        100,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(applied(&out.players[1]), 50);
    assert_eq!((ps[0].energy, ps[1].energy), (600, 550));
    let clear = area_spell(TargetKind::FullArea, &[(Ability::ClearItem, 7)]);
    let mut players = marked(&mut ps);
    let out = area_one(
        &clear,
        2,
        1,
        0,
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(out.caster, Outcome::ClearItem(7));
}

// The lasting area spell goes on every marked player with the base duration (not the
// slot-level rounds) and his element share, once per cast; a refusal before any
// success ends the cast, after one it is skipped. A self kind puts the default effect
// on the caster with the computed rounds; Item and Monster or Single do nothing.
#[test]
fn area_lasting_add_uses_the_base_duration_and_each_players_share_once() {
    let (mut m, k) = caster_monster(10, 50, 40);
    let mut caster = MonsterCaster {
        monster: &mut m,
        known: &k,
    };
    let mut s = area_spell(
        TargetKind::FullArea,
        &[(Ability::Shield, 0), (Ability::Defence, 9)],
    );
    s.duration = 4;
    let mut ps = three_players();
    let mut players = marked(&mut ps);
    let mut state = MonsterCastState::new();
    let out = apply_monster_cast_area_spell(
        &effects_of(&s),
        &MonsterCast {
            spell: &s,
            power: 10,
            share: 10,
            duration: 12,
        },
        &mut caster,
        &mut players,
        &mut state,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(
        out[0].players,
        vec![Outcome::Lasting(SlotOutcome::Added(9)); 3]
    );
    assert_eq!(out[1].players, vec![Outcome::NoEffect; 3]);
    assert_eq!(
        [
            (ps[0].spellvals[9], ps[0].spellrnds[9]),
            (ps[1].spellvals[9], ps[1].spellrnds[9]),
            (ps[2].spellvals[9], ps[2].spellrnds[9]),
        ],
        [(10, 4), (5, 4), (10, 4)]
    );
    // The first player refuses (not stronger): the cast ends with nobody else tried.
    let mut players = marked(&mut ps);
    let mut state = MonsterCastState::new();
    let out = apply_monster_cast_area_spell(
        &effects_of(&s),
        &MonsterCast {
            spell: &s,
            power: 10,
            share: 10,
            duration: 12,
        },
        &mut caster,
        &mut players,
        &mut state,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].players,
        vec![Outcome::Lasting(SlotOutcome::AlreadyActive)]
    );
    assert!(state.refused);
    // After one success a refusal is skipped: the second player is stronger, the third not.
    ps[0].spellvals[9] = 1;
    let mut players = marked(&mut ps);
    let mut state = MonsterCastState::new();
    let out = area_one(&s, 11, 11, 12, &mut caster, &mut players, &mut state);
    assert_eq!(
        out.players,
        vec![
            Outcome::Lasting(SlotOutcome::Renewed(9)),
            Outcome::Lasting(SlotOutcome::AlreadyActive),
            Outcome::Lasting(SlotOutcome::Renewed(9)),
        ]
    );
    assert!(!state.refused && state.added);
    // Nobody marked at all: the add "succeeds" for nobody and the cast ends.
    let mut none: [UserTarget<'_, '_>; 0] = [];
    let mut state = MonsterCastState::new();
    area_one(&s, 11, 11, 12, &mut caster, &mut none, &mut state);
    assert!(state.refused);
    // The self kinds: the computed rounds on the caster, the slot's value or the roll.
    let mut myself = area_spell(
        TargetKind::Any,
        &[(Ability::Defence, 9), (Ability::Shield, 0)],
    );
    myself.duration = 4;
    let mut players = marked(&mut ps);
    let out = apply_monster_cast_area_spell(
        &effects_of(&myself),
        &MonsterCast {
            spell: &myself,
            power: 10,
            share: 10,
            duration: 12,
        },
        &mut caster,
        &mut players,
        &mut MonsterCastState::new(),
    );
    assert_eq!(out[0].caster, Outcome::Lasting(SlotOutcome::Added(0)));
    assert_eq!(out[1].caster, Outcome::Lasting(SlotOutcome::Renewed(0)));
    assert_eq!(
        (
            caster.monster.spells[0].value,
            caster.monster.spells[0].rounds
        ),
        (10, 12)
    );
    for kind in [
        TargetKind::Item,
        TargetKind::MonsterOrSingle,
        TargetKind::Single,
    ] {
        let mut other = myself.clone();
        other.target = kind;
        let out = area_one(
            &other,
            10,
            10,
            12,
            &mut caster,
            &mut players,
            &mut MonsterCastState::new(),
        );
        assert!(
            out.caster == Outcome::NoEffect && out.players.is_empty(),
            "{kind:?}"
        );
    }
}

// The Remove spell pre-pass ends the named spell on every marked player (its slot
// cleared, its ending listed for the engine) and takes his mark off; Terminate Spell
// does the same without the chain cast; a slot value of 0 and a non-area kind do
// nothing.
#[test]
fn area_remove_pass_ends_the_spell_and_unmarks_the_player() {
    let mut ending = spell(41, &[(Ability::EndCast, 55)]);
    ending.duration = 3;
    let lookup = move |n: SpellNo| -> Option<&'static Spell> {
        (n == SpellNo(41)).then(|| &*Box::leak(Box::new(ending.clone())))
    };
    let s = area_spell(
        TargetKind::FullArea,
        &[
            (Ability::RemoveSpell, 41),
            (Ability::TermSpell, 41),
            (Ability::RemoveSpell, 0),
        ],
    );
    let mut ps = three_players();
    ps[0].spells[2] = SpellNo(41);
    ps[2].spells[4] = SpellNo(41);
    let mut players = marked(&mut ps);
    players[0].spells = &lookup;
    players[2].spells = &lookup;
    let out = apply_monster_area_remove_pass(&effects_of(&s), &s, &mut players);
    // Remove spell (chain 1) finds it first; the engine runs the ending.
    let ended = EndedSpell {
        number: SpellNo(41),
        power: 0,
        chain: true,
    };
    assert_eq!(
        out.players,
        vec![
            Outcome::Ended {
                spells: vec![ended],
                poison: None
            },
            Outcome::Ended {
                spells: vec![],
                poison: None
            },
            Outcome::Ended {
                spells: vec![ended],
                poison: None
            },
        ]
    );
    assert_eq!(out.unmarked, vec![0, 2]);
    assert_eq!((ps[0].spells[2], ps[2].spells[4]), (SpellNo(0), SpellNo(0)));
    let mut single = s.clone();
    single.target = TargetKind::Any;
    ps[0].spells[2] = SpellNo(41);
    let mut players = marked(&mut ps);
    players[0].spells = &lookup;
    let out = apply_monster_area_remove_pass(&effects_of(&single), &single, &mut players);
    assert!(out.unmarked.is_empty());
    assert_eq!(ps[0].spells[2], SpellNo(41));
}

fn room_target<'a>(m: &'a mut ActiveMonster, k: &'a KnownMonster) -> MonsterTarget<'a> {
    monster(m, k)
}

// A room's cast on a monster: the "MR" is the charm resistance only (0 without an
// Enslave slot); the multiplier is the linear 150 - charmres percent, at least 25,
// capped at the spell's maxval; the roll and every slot value are capped at maxval
// too.
#[test]
fn room_on_monster_uses_charm_resistance_capped_at_maxval() {
    let mut k = known_monster(&[]);
    k.charmres = 140;
    k.magicres = 90;
    let plain = monster_spell(&[(Ability::DamageArmour, 0)]);
    assert_eq!(room_charm_resistance(&plain, &k), 0);
    let enslave = monster_spell(&[(Ability::Enslave, 0), (Ability::DamageArmour, 0)]);
    assert_eq!(room_charm_resistance(&enslave, &k), 140);
    assert_eq!(room_monster_power(&plain, 90, 50), 45);
    assert_eq!(room_monster_power(&plain, 400, 0), 100);
    let mut m = active_monster();
    m.hp = 500;
    let mut state = RoomMonsterState::default();
    let out = apply_room_on_monster_spell(&plain, 80, &mut room_target(&mut m, &k), &mut state);
    assert_eq!(out.len(), 10);
    assert_eq!(
        out[0],
        (
            0,
            Outcome::Applied {
                amount: 100,
                knocked_out: false
            }
        )
    );
    assert_eq!(m.hp, 400);
    assert!(state.spelladded);
    let out = apply_room_on_monster_spell(
        &enslave,
        80,
        &mut room_target(&mut m, &k),
        &mut RoomMonsterState::default(),
    );
    assert_eq!(
        out,
        vec![
            (0, Outcome::Lasting(SlotOutcome::Added(0))),
            (6, Outcome::NoEffect),
            (7, Outcome::NoEffect),
            (8, Outcome::NoEffect),
            (9, Outcome::NoEffect),
        ]
    );
    assert_eq!(m.hp, 400);
    let mut over = monster_spell(&[(Ability::DamageNoArmour, 250)]);
    over.maxval = 60;
    let out = apply_room_on_monster(
        &effects_of(&over).slots[0],
        &over,
        10,
        &mut room_target(&mut m, &k),
        140,
        &mut RoomMonsterState::default(),
    );
    assert_eq!(applied(&out), 60);
    assert_eq!(m.hp, 340);
    let out = apply_room_on_monster(
        &effects_of(&enslave).slots[1],
        &enslave,
        80,
        &mut room_target(&mut m, &k),
        140,
        &mut RoomMonsterState::default(),
    );
    assert_eq!(applied(&out), 20);
    assert_eq!(m.hp, 320);
}

// Heal never heals at once; it and every ability without a case renew the spell where
// it is and write it into every free slot (sic) with the base duration, once while
// `spelladded` is clear; Summon and Cure Poison do nothing but set the flag.
#[test]
fn room_on_monster_heal_is_never_instant_and_fills_every_free_slot() {
    let k = known_monster(&[]);
    let mut m = active_monster();
    m.hp = 10;
    m.maxhp = 50;
    m.spells[1] = ActiveSpell {
        spell: SpellNo(8),
        value: 1,
        rounds: 1,
    };
    let mut heal = monster_spell(&[(Ability::Heal, 0)]);
    heal.duration = 0;
    let mut state = RoomMonsterState::default();
    let out = apply_room_on_monster(
        &effects_of(&heal).slots[0],
        &heal,
        30,
        &mut room_target(&mut m, &k),
        0,
        &mut state,
    );
    assert_eq!(out, Outcome::Lasting(SlotOutcome::Added(0)));
    assert_eq!(m.hp, 10);
    let want = ActiveSpell {
        spell: SpellNo(23),
        value: 30,
        rounds: 0,
    };
    assert_eq!(
        m.spells,
        [
            want,
            ActiveSpell {
                spell: SpellNo(8),
                value: 1,
                rounds: 1
            },
            want,
            want,
            want
        ]
    );
    heal.duration = 6;
    let out = apply_room_on_monster(
        &effects_of(&heal).slots[0],
        &heal,
        31,
        &mut room_target(&mut m, &k),
        0,
        &mut RoomMonsterState::default(),
    );
    assert_eq!(out, Outcome::Lasting(SlotOutcome::Renewed(0)));
    assert!(
        m.spells
            .iter()
            .filter(|s| s.spell == SpellNo(23))
            .all(|s| (s.value, s.rounds) == (31, 6))
    );
    assert_eq!(
        apply_room_on_monster(
            &effects_of(&heal).slots[0],
            &heal,
            31,
            &mut room_target(&mut m, &k),
            0,
            &mut state,
        ),
        Outcome::NoEffect
    );
    let mut state = RoomMonsterState::default();
    for a in [Ability::Summon, Ability::CurePoison] {
        let s = monster_spell(&[(a, 5)]);
        state.spelladded = false;
        assert_eq!(
            apply_room_on_monster(
                &effects_of(&s).slots[0],
                &s,
                5,
                &mut room_target(&mut m, &k),
                0,
                &mut state,
            ),
            Outcome::NoEffect
        );
        assert!(state.spelladded, "{a:?}");
    }
    assert_eq!(m.poison, 0);
}

// Poison raises the level; the lasting form also hits the hit points at once with no
// kill check and the spell is recorded for the base duration (first free slot) even
// without one, once; the immune are skipped and the flag stays clear. A lasting
// Energy is dropped; Slay applies with any duration.
#[test]
fn room_on_monster_poison_energy_and_slay() {
    let k = known_monster(&[]);
    let mut m = active_monster();
    m.hp = 10;
    let mut venom = monster_spell(&[(Ability::Poison, 0)]);
    venom.duration = 5;
    let mut state = RoomMonsterState::default();
    let out = apply_room_on_monster_spell(&venom, 12, &mut room_target(&mut m, &k), &mut state);
    assert_eq!(out.len(), 10);
    assert_eq!(out[0], (0, Outcome::Lasting(SlotOutcome::Added(0))));
    assert_eq!((m.poison, m.hp), (12, -2));
    assert_eq!(
        (m.spells[0].value, m.spells[0].rounds, m.spells[1].spell),
        (12, 5, SpellNo(0))
    );
    venom.duration = 0;
    let out = apply_room_on_monster_spell(&venom, 20, &mut room_target(&mut m, &k), &mut state);
    assert_eq!(out.len(), 10);
    assert_eq!(
        out[0].1,
        Outcome::Applied {
            amount: 20,
            knocked_out: false
        }
    );
    assert_eq!((m.poison, m.hp), (20, -2));
    let mut state = RoomMonsterState::default();
    let mut t = room_target(&mut m, &k);
    t.poison_immune = true;
    let out = apply_room_on_monster(
        &effects_of(&venom).slots[0],
        &venom,
        20,
        &mut t,
        0,
        &mut state,
    );
    assert_eq!(out, Outcome::Immune);
    assert!(!state.spelladded);
    let mut energy = monster_spell(&[(Ability::Energy, 0)]);
    m.energy = 100;
    let mut state = RoomMonsterState::default();
    assert_eq!(
        apply_room_on_monster(
            &effects_of(&energy).slots[0],
            &energy,
            20,
            &mut room_target(&mut m, &k),
            0,
            &mut state
        ),
        Outcome::Applied {
            amount: 20,
            knocked_out: false
        }
    );
    energy.duration = 3;
    assert_eq!(
        apply_room_on_monster(
            &effects_of(&energy).slots[0],
            &energy,
            20,
            &mut room_target(&mut m, &k),
            0,
            &mut state
        ),
        Outcome::NoEffect
    );
    assert_eq!(m.energy, 120);
    let mut slay = monster_spell(&[(Ability::Slay, 0)]);
    slay.duration = 3;
    m.hp = 40;
    assert_eq!(
        apply_room_on_monster_spell(
            &slay,
            1,
            &mut room_target(&mut m, &k),
            &mut RoomMonsterState::default()
        ),
        vec![(
            0,
            Outcome::Applied {
                amount: 42,
                knocked_out: false
            }
        )]
    );
    assert_eq!(m.hp, -2);
    let mut single = slay.clone();
    single.target = TargetKind::Single;
    m.hp = 40;
    assert_eq!(
        apply_room_on_monster_spell(
            &single,
            1,
            &mut room_target(&mut m, &k),
            &mut RoomMonsterState::default()
        )
        .len(),
        10
    );
    assert_eq!(m.hp, 40);
}

// After the lasting block the loop goes on at slot 6, whatever slot it was on; the loop
// stops when a damage slot killed the monster; `spelladded` is the engine's to carry
// (the game never clears it between monster casts).
#[test]
fn room_on_monster_loop_counter_clobber_and_stop() {
    let k = known_monster(&[]);
    let mut m = active_monster();
    m.hp = 100;
    let mut s = monster_spell(&[]);
    s.abilities[1] = (Ability::Shield, 3);
    s.abilities[2] = (Ability::DamageNoArmour, 5);
    s.abilities[7] = (Ability::DamageNoArmour, 1);
    s.abilities[8] = (Ability::DamageNoArmour, 2);
    let out = apply_room_on_monster_spell(
        &s,
        10,
        &mut room_target(&mut m, &k),
        &mut RoomMonsterState::default(),
    );
    let indexes: Vec<usize> = out.iter().map(|(i, _)| *i).collect();
    assert_eq!(indexes, vec![0, 1, 6, 7, 8, 9]);
    assert_eq!(m.hp, 97);
    let mut late = monster_spell(&[]);
    late.abilities[6] = (Ability::Hunger, 1);
    late.abilities[7] = (Ability::Shield, 3);
    late.abilities[8] = (Ability::DamageNoArmour, 2);
    let mut m = active_monster();
    m.hp = 100;
    let out = apply_room_on_monster_spell(
        &late,
        10,
        &mut room_target(&mut m, &k),
        &mut RoomMonsterState::default(),
    );
    let indexes: Vec<usize> = out.iter().map(|(i, _)| *i).collect();
    assert_eq!(indexes, vec![0, 1, 2, 3, 4, 5, 6, 7, 6, 7, 8, 9]);
    assert_eq!(m.hp, 98);
    assert_eq!(out[7].1, Outcome::Lasting(SlotOutcome::Added(0)));
    assert_eq!(out[9].1, Outcome::NoEffect);
    let mut m = active_monster();
    m.hp = 3;
    let out = apply_room_on_monster_spell(
        &s,
        10,
        &mut room_target(&mut m, &k),
        &mut RoomMonsterState::default(),
    );
    let indexes: Vec<usize> = out.iter().map(|(i, _)| *i).collect();
    assert_eq!(indexes, vec![0, 1, 6, 7, 8]);
    assert_eq!(m.hp, 0);
    let mut m = active_monster();
    m.hp = 100;
    let mut carried = RoomMonsterState { spelladded: true };
    let out = apply_room_on_monster_spell(&s, 10, &mut room_target(&mut m, &k), &mut carried);
    assert_eq!(out.len(), 10);
    assert!(m.spells.iter().all(|s| s.spell == SpellNo(0)));
    assert_eq!(m.hp, 92);
}

proptest! {
    // Every ability on every target kind, instant or lasting, on the monster paths: the
    // single-target cast, the area cast with up to three marked players, the remove pass,
    // the self spell and the room's cast on a monster, without a panic.
    #[test]
    fn every_effect_on_the_monster_paths_has_an_outcome(
        ability in 0u16..190, value in -50i16..50, power in -100i32..100, share in -40i32..40,
        duration in 0i32..8, target in 0u16..14, nplayers in 0usize..4,
    ) {
        let mut s = spell(1, &[(Ability::from_code(ability), value), (Ability::Heal, 0)]);
        s.duration = ((duration % 2) * 5) as i16;
        s.maxval = 60;
        s.target = TargetKind::from_code(target);
        s.spell_type = SpellType::Autocombat;
        let effects = effects_of(&s);
        let cast = MonsterCast { spell: &s, power, share, duration };
        let (mut m, k) = caster_monster(20, 30, 25);
        let mut caster = MonsterCaster { monster: &mut m, known: &k };
        let mut c = player(20, 30);
        c.spells[0] = SpellNo(1);
        let mut u = user(&mut c);
        let mut state = MonsterCastState::new();
        let out = apply_monster_cast_spell(&effects, &cast, &mut caster, &mut u, &mut state);
        prop_assert!(out.len() <= effects.slots.len());
        let mut ps = three_players();
        let mut all = marked(&mut ps);
        let players = &mut all[..nplayers.min(3)];
        let pass = apply_monster_area_remove_pass(&effects, &s, players);
        prop_assert_eq!(pass.players.len(), players.len());
        let mut state = MonsterCastState::new();
        let out = apply_monster_cast_area_spell(&effects, &cast, &mut caster, players, &mut state);
        for o in &out {
            prop_assert!(o.players.is_empty() || o.players.len() <= players.len());
        }
        apply_monster_self_spell(&s, power, &mut caster);
        let mut t = monster(&mut m, &k);
        let mut rstate = RoomMonsterState::default();
        let out = apply_room_on_monster_spell(&s, room_monster_power(&s, power, 10), &mut t, &mut rstate);
        prop_assert!(!out.is_empty() && out.len() <= 14);
    }
}
