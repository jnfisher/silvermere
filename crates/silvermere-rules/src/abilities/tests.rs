//! Layer 1 tests for `abilities`: each test names the rule it checks.

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
use silvermere_types::names::ABILITY_NAMES;
use silvermere_types::records::{
    ActiveMonster, ActiveSpell, Character, Class, Coins, Item, KnownMonster, MonsterAttack,
    MonsterItem, MonsterSpell, Race, Spell,
};
use silvermere_types::{
    NABILITY, NAMTRAIL, NMONATTACKS, NMONITEMS, NMONSPELLS, NRECABILS, NSPELLCAST, NTRAIL, NWORN,
};

use super::{
    AbilityLookup, AbilityName, BEST_VALUE, CharacterAbilities, DynamicStats, MONSTER_BEST_VALUE,
    MonsterAbilities, NABILNAMES, ability_name, dynamic_stats, item_ability_value,
    item_has_ability, spell_ability_value, spell_has_ability,
};

// ---------------------------------------------------------------------------------------
// Fixtures: blank records (every field 0, empty or `None`) with the slots the test needs.

const NONE: (Ability, i16) = (Ability::None, 0);

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

fn race(abilities: &[(Ability, i16)]) -> Race {
    Race {
        number: 0.into(),
        name: String::new(),
        minint: 0,
        minwil: 0,
        minstr: 0,
        minhea: 0,
        minagi: 0,
        mincha: 0,
        hpbonus: 0,
        maxenergy: 0,
        restored: 0,
        abilities: slots(abilities),
        basecp: 0,
        exp: 0,
        maxint: 0,
        maxwil: 0,
        maxstr: 0,
        maxhea: 0,
        maxagi: 0,
        maxcha: 0,
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

/// A wearable item (worn location Finger, type Armour).
fn ring(number: i32, abilities: &[(Ability, i16)]) -> Item {
    item(
        number,
        ItemType::from_code(2),
        WornLocation::from_code(10),
        abilities,
    )
}

/// A weapon.
fn sword(number: i32, abilities: &[(Ability, i16)]) -> Item {
    item(number, ItemType::Weapon, WornLocation::Nowhere, abilities)
}

/// An item that is neither a weapon nor wearable (type General, worn Nowhere).
fn trinket(number: i32, abilities: &[(Ability, i16)]) -> Item {
    item(
        number,
        ItemType::from_code(0),
        WornLocation::Nowhere,
        abilities,
    )
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

/// The world a test character lives in: its race and class, the spells and items its
/// numbers resolve to.
struct World {
    character: Character,
    race: Option<Race>,
    class: Option<Class>,
    spells: Vec<Spell>,
    items: Vec<Item>,
}

impl World {
    fn new() -> Self {
        Self {
            character: character(),
            race: None,
            class: None,
            spells: Vec::new(),
            items: Vec::new(),
        }
    }

    fn own(mut self, abilities: &[(Ability, i16)]) -> Self {
        self.character.abils = slots(abilities);
        self
    }

    fn race(mut self, abilities: &[(Ability, i16)]) -> Self {
        self.race = Some(race(abilities));
        self
    }

    fn class(mut self, abilities: &[(Ability, i16)]) -> Self {
        self.class = Some(class(abilities));
        self
    }

    /// Casts a spell into the next free cast slot, with `rolled` as the value rolled.
    fn cast(mut self, spell: Spell, rolled: i16) -> Self {
        let slot = self
            .character
            .spells
            .iter()
            .position(|&s| s == SpellNo(0))
            .expect("a free cast slot");
        self.character.spells[slot] = spell.number;
        self.character.spellvals[slot] = rolled;
        self.spells.push(spell);
        self
    }

    /// Casts a spell into cast slot `slot`.
    fn cast_at(mut self, slot: usize, spell: Spell, rolled: i16) -> Self {
        self.character.spells[slot] = spell.number;
        self.character.spellvals[slot] = rolled;
        self.spells.push(spell);
        self
    }

    /// Notes a spell number as cast without a record for it to resolve to.
    fn cast_missing(mut self, number: i16, rolled: i16) -> Self {
        self.character.spells[0] = SpellNo(number);
        self.character.spellvals[0] = rolled;
        self
    }

    fn wear(mut self, item: Item) -> Self {
        let slot = self
            .character
            .worn
            .iter()
            .position(|&i| i == ItemNo(0))
            .expect("a free worn slot");
        self.character.worn[slot] = item.number;
        self.items.push(item);
        self
    }

    fn wield(mut self, item: Item) -> Self {
        self.character.weapon = item.number;
        self.items.push(item);
        self
    }

    fn carry(mut self, item: Item) -> Self {
        self.character.items.push(item.number);
        self.items.push(item);
        self
    }

    fn with<R>(&self, f: impl FnOnce(&CharacterAbilities<'_>) -> R) -> R {
        let spells = |n: SpellNo| self.spells.iter().find(|s| s.number == n);
        let items = |n: ItemNo| self.items.iter().find(|i| i.number == n);
        let lookup = CharacterAbilities {
            character: &self.character,
            race: self.race.as_ref(),
            class: self.class.as_ref(),
            spells: &spells,
            items: &items,
        };
        f(&lookup)
    }

    fn value(&self, ability: Ability) -> i32 {
        self.with(|l| l.value(ability))
    }

    fn has(&self, ability: Ability) -> bool {
        self.with(|l| l.has(ability))
    }

    fn dynamic(&self) -> DynamicStats {
        self.with(dynamic_stats)
    }
}

/// The level 1 Kang Witchunter test sheet: Kang gives Poison immunity 100,
/// Damage Resist 10 and Defence 5; the Witchunter class gives Bash, Anti-Magic and Hit
/// Magical 10.
fn kang_witchunter() -> World {
    World::new()
        .race(&[
            (Ability::PoisonImmune, 100),
            (Ability::DamageResist, 10),
            (Ability::Defence, 5),
        ])
        .class(&[
            (Ability::Bash, 1),
            (Ability::AntiMagic, 1),
            (Ability::HitMagical, 10),
        ])
}

const DEFENCE: Ability = Ability::Defence;

// ---------------------------------------------------------------------------------------
// A character's totals: the five record sources and the item rules.

// Own slots, spells, race and class all add up.
#[test]
fn additive_ability_stacks_across_every_source() {
    let world = World::new()
        .own(&[(DEFENCE, 1), (Ability::Damage, 9), (DEFENCE, 2)])
        .cast(spell(1, &[(DEFENCE, 4)]), 0)
        .race(&[(DEFENCE, 8)])
        .class(&[(DEFENCE, 16)])
        .wear(ring(1, &[(DEFENCE, 32)]))
        .wield(sword(2, &[(DEFENCE, 64)]))
        .carry(trinket(3, &[(DEFENCE, 128)]));
    assert_eq!(world.value(DEFENCE), 1 + 2 + 4 + 8 + 16 + 32 + 64 + 128);
    assert_eq!(world.value(Ability::Damage), 9);
    assert_eq!(world.value(Ability::Critical), 0);
}

// Ability 0 gives 0 whatever the slots hold.
#[test]
fn ability_none_is_zero() {
    let world = World::new().own(&[(Ability::None, 5)]);
    assert_eq!(world.value(Ability::None), 0);
    assert_eq!(world.with(|l| l.pair(Ability::None, DEFENCE)), (0, 0));
    assert!(!world.has(Ability::None));
}

// The seven best-value abilities keep the
// highest single value over the sources instead of adding.
#[test]
fn best_value_abilities_keep_the_highest_source() {
    for ability in BEST_VALUE {
        let world = World::new()
            .own(&[(ability, 3)])
            .cast(spell(1, &[(ability, 5)]), 0)
            .race(&[(ability, 2)])
            .class(&[(ability, 4)])
            .wear(ring(1, &[(ability, 1)]));
        assert_eq!(world.value(ability), 5, "{ability:?}");
        assert_eq!(world.with(|l| l.best(ability)), Some(5), "{ability:?}");
    }
    // Additive abilities are not in the set.
    assert!(!BEST_VALUE.contains(&DEFENCE));
    assert!(!BEST_VALUE.contains(&Ability::DamageResist));
}

// A best-value total nothing gives has no best (`None`) and becomes 0; a negative best from
// a record keeps its sign.
#[test]
fn best_value_nothing_found_is_none_then_zero() {
    let empty = World::new();
    assert_eq!(empty.with(|l| l.best(Ability::AvCurse)), None);
    assert_eq!(empty.value(Ability::AvCurse), 0);
    let cursed = World::new().race(&[(Ability::AvCurse, -7)]);
    assert_eq!(cursed.with(|l| l.best(Ability::AvCurse)), Some(-7));
    assert_eq!(cursed.value(Ability::AvCurse), -7);
    // Additive abilities always have a best.
    assert_eq!(empty.with(|l| l.best(DEFENCE)), Some(0));
}

// An item contributes the value of its first slot
// with the ability, 0 when it has none, and that 0 is compared like any other value: a worn
// item without the ability lifts a negative best-value total to 0.
#[test]
fn item_without_best_value_ability_contributes_zero() {
    let world = World::new()
        .race(&[(Ability::AvCurse, -7)])
        .wear(ring(1, &[(DEFENCE, 1)]));
    assert_eq!(world.value(Ability::AvCurse), 0);
    assert_eq!(world.with(|l| l.best(Ability::AvCurse)), Some(0));
}

// Only an item's first slot with the ability counts for the total; the dynamic stats apply
// every slot.
#[test]
fn item_total_reads_first_slot_but_dynamic_applies_every_slot() {
    let world = World::new().wear(ring(1, &[(DEFENCE, 3), (DEFENCE, 4)]));
    assert_eq!(world.value(DEFENCE), 3);
    assert_eq!(world.dynamic().acbonus, 7);
}

// The wielded weapon counts; a weapon
// or a wearable item in the pack gives nothing; a pack item that is neither counts.
#[test]
fn weapon_counts_pack_weapon_and_unworn_ring_do_not() {
    let world = World::new()
        .wield(sword(1, &[(DEFENCE, 10)]))
        .carry(sword(2, &[(DEFENCE, 100)]))
        .carry(ring(3, &[(DEFENCE, 1000)]))
        .carry(trinket(4, &[(DEFENCE, 1)]));
    assert_eq!(world.value(DEFENCE), 11);
    assert_eq!(world.dynamic().acbonus, 11);
    assert!(world.has(DEFENCE));
    let pack_only = World::new().carry(ring(3, &[(DEFENCE, 1000)]));
    assert_eq!(pack_only.value(DEFENCE), 0);
    assert!(!pack_only.has(DEFENCE));
}

// Totals and dynamic stats: an item number whose record is missing is skipped.
#[test]
fn missing_item_record_is_skipped() {
    let mut world = World::new().wear(ring(1, &[(DEFENCE, 5)]));
    world.character.worn[1] = ItemNo(99);
    world.character.weapon = ItemNo(98);
    assert_eq!(world.value(DEFENCE), 5);
    assert_eq!(world.value(Ability::AvCurse), 0);
    assert_eq!(world.dynamic().acbonus, 5);
}

// ---------------------------------------------------------------------------------------
// Spells: fixed and rolled values, missing records, negation.

// A spell slot whose value is 0 takes the value rolled when the spell was cast; the dynamic
// stats do the same.
#[test]
fn rolled_spell_value_stands_in_for_zero() {
    let world = World::new()
        .cast(spell(1, &[(DEFENCE, 0)]), 7)
        .cast(spell(2, &[(DEFENCE, 2)]), 100);
    assert_eq!(world.value(DEFENCE), 9);
    assert_eq!(world.dynamic().acbonus, 9);
}

// Totals and dynamic stats: a cast slot whose spell record no longer exists is skipped.
#[test]
fn missing_spell_record_is_skipped() {
    let world = World::new().cast_missing(77, 50).race(&[(DEFENCE, 5)]);
    assert_eq!(world.value(DEFENCE), 5);
    assert_eq!(world.dynamic().acbonus, 5);
    assert!(!world.has(Ability::Damage));
}

// "Negate ability" on a spell, with the
// ability's number as its value, makes the total 0 whatever the other sources give.
#[test]
fn negate_spell_zeroes_the_total() {
    let world = World::new()
        .own(&[(DEFENCE, 3)])
        .race(&[(DEFENCE, 5), (Ability::Damage, 4)])
        .wear(ring(1, &[(DEFENCE, 2)]))
        .cast(spell(1, &[(Ability::Negate, DEFENCE.code() as i16)]), 0);
    assert_eq!(world.value(DEFENCE), 0);
    assert_eq!(world.value(Ability::Damage), 4);
    // the second ability of a pair is negated on its own.
    assert_eq!(world.with(|l| l.pair(DEFENCE, Ability::Damage)), (0, 4));
    assert_eq!(world.with(|l| l.pair(Ability::Damage, DEFENCE)), (4, 0));
    // a negated best-value ability is 0, not `None`.
    let blessed = World::new().race(&[(Ability::AvBless, 5)]).cast(
        spell(1, &[(Ability::Negate, Ability::AvBless.code() as i16)]),
        0,
    );
    assert_eq!(blessed.with(|l| l.best(Ability::AvBless)), Some(0));
}

// "Negate ability" names the ability by its
// value; a negation slot naming another ability, or on a race or item, changes nothing.
#[test]
fn negate_only_counts_from_a_spell_naming_the_ability() {
    let world = World::new()
        .race(&[(DEFENCE, 5), (Ability::Negate, DEFENCE.code() as i16)])
        .wear(ring(1, &[(Ability::Negate, DEFENCE.code() as i16)]))
        .cast(
            spell(1, &[(Ability::Negate, Ability::Damage.code() as i16)]),
            0,
        );
    assert_eq!(world.value(DEFENCE), 5);
    assert_eq!(world.dynamic().acbonus, 5);
}

// The pair form is the two single totals
// from one pass.
#[test]
fn pair_matches_two_single_totals() {
    let world = World::new()
        .own(&[(Ability::Perception, 3), (Ability::Stealth, 2)])
        .race(&[(Ability::Perception, 4)])
        .cast(spell(1, &[(Ability::Stealth, 0)]), 6)
        .wear(ring(1, &[(Ability::Stealth, 1), (Ability::Perception, 1)]));
    let (perception, stealth) = world.with(|l| l.pair(Ability::Perception, Ability::Stealth));
    assert_eq!((perception, stealth), (8, 9));
    assert_eq!(perception, world.value(Ability::Perception));
    assert_eq!(stealth, world.value(Ability::Stealth));
}

// ---------------------------------------------------------------------------------------
// Whether a character has an ability.

// Any source with the ability answers yes; (sic)
// a spell's abilities are seen only while the spell's own slot i, the cast slot's index,
// is in use.
#[test]
fn has_finds_every_source_with_the_spell_slot_quirk() {
    assert!(World::new().own(&[(DEFENCE, 1)]).has(DEFENCE));
    assert!(World::new().race(&[(DEFENCE, 1)]).has(DEFENCE));
    assert!(World::new().class(&[(DEFENCE, 1)]).has(DEFENCE));
    assert!(World::new().wear(ring(1, &[(DEFENCE, 1)])).has(DEFENCE));
    assert!(World::new().wield(sword(1, &[(DEFENCE, 1)])).has(DEFENCE));
    assert!(World::new().carry(trinket(1, &[(DEFENCE, 1)])).has(DEFENCE));
    assert!(!World::new().carry(sword(1, &[(DEFENCE, 1)])).has(DEFENCE));
    assert!(!World::new().has(DEFENCE));
    // A one-ability spell in cast slot 0: its slot 0 is in use, so it is seen.
    let seen = World::new().cast_at(0, spell(1, &[(DEFENCE, 1)]), 0);
    assert!(seen.has(DEFENCE));
    // The same spell in cast slot 3: its slot 3 is empty, so has() misses what value() adds.
    let missed = World::new().cast_at(3, spell(1, &[(DEFENCE, 1)]), 0);
    assert!(!missed.has(DEFENCE));
    assert_eq!(missed.value(DEFENCE), 1);
}

// Whether a character has an ability: a "Negate ability" spell is not looked at.
#[test]
fn has_ignores_negation() {
    let world = World::new()
        .race(&[(DEFENCE, 5)])
        .cast(spell(1, &[(Ability::Negate, DEFENCE.code() as i16)]), 0);
    assert!(world.has(DEFENCE));
    assert_eq!(world.value(DEFENCE), 0);
}

// ---------------------------------------------------------------------------------------
// The single-record helpers.

// item_ability_value, item_has_ability, spell_ability_value, spell_has_ability.
#[test]
fn single_record_helpers_read_the_first_slot() {
    let ring = ring(1, &[(Ability::Damage, 2), (DEFENCE, 3), (DEFENCE, 4)]);
    assert_eq!(item_ability_value(&ring, DEFENCE), 3);
    assert_eq!(item_ability_value(&ring, Ability::Critical), 0);
    assert!(item_has_ability(&ring, Ability::Damage));
    assert!(!item_has_ability(&ring, Ability::Critical));
    let spell = spell(1, &[(Ability::Damage, 0), (DEFENCE, 6)]);
    assert_eq!(spell_ability_value(&spell, DEFENCE), 6);
    assert_eq!(spell_ability_value(&spell, Ability::Damage), 0);
    assert!(spell_has_ability(&spell, Ability::Damage));
    assert!(!spell_has_ability(&spell, Ability::Critical));
}

// ---------------------------------------------------------------------------------------
// dynamic_stats.

// dynamic_stats for the Kang Witchunter sheet: Defence 5 goes to acbonus (combat adds it to
// the defence value, a quarter of it against a backstab) and Damage Resist 10 to drbonus
// (combat adds it to the damage resistance kept in tenths, then subtracts a tenth of that:
// 1 point). Poison immunity, Bash, Anti-Magic and Hit Magical touch no bonus field.
#[test]
fn kang_witchunter_sheet_routes_defence_and_damage_resist() {
    let world = kang_witchunter();
    let stats = world.dynamic();
    assert_eq!(
        stats,
        DynamicStats {
            acbonus: 5,
            drbonus: 10,
            ..DynamicStats::default()
        }
    );
    assert_eq!(world.value(DEFENCE), 5);
    assert_eq!(world.value(Ability::DamageResist), 10);
    assert_eq!(world.value(Ability::PoisonImmune), 100);
    assert!(world.has(Ability::Bash) && world.has(Ability::AntiMagic));
    assert_eq!(world.value(Ability::HitMagical), 10);
}

// Every summed field.
#[test]
fn dynamic_sums_each_bonus_field() {
    let world = World::new()
        .own(&[
            (DEFENCE, 1),
            (Ability::Damage, 2),
            (Ability::Critical, 3),
            (Ability::DamageResist, 4),
            (Ability::DrPercent, 5),
            (Ability::Shield, 6),
            (Ability::DefenceMod, 7),
            (Ability::HealRate, 8),
            (Ability::ManaRate, 9),
            (Ability::RoomLight, 10),
        ])
        .race(&[
            (DEFENCE, 10),
            (Ability::Damage, 20),
            (Ability::Critical, 30),
            (Ability::DamageResist, 40),
            (Ability::DrPercent, 50),
            (Ability::Shield, 60),
            (Ability::DefenceMod, 70),
            (Ability::HealRate, 80),
            (Ability::ManaRate, 90),
            (Ability::RoomLight, 100),
        ]);
    assert_eq!(
        world.dynamic(),
        DynamicStats {
            avbonus: 0,
            acbonus: 11,
            dmgbonus: 22,
            critbonus: 33,
            drbonus: 44,
            drpct: 55,
            energyuse: 0,
            shield: 66,
            defmod: 77,
            healrate: 88,
            manarate: 99,
            roomlight: 110,
            status_from_abilities: PlayerStatus::empty(),
            status2_from_abilities: PlayerStatus2::empty(),
        }
    );
}

// The three attack value abilities share avbonus and keep the best; none at all gives 0.
#[test]
fn dynamic_av_trio_keeps_the_best() {
    let world = World::new()
        .own(&[(Ability::AvCurse, -3)])
        .race(&[(Ability::AvBless, 5)])
        .class(&[(Ability::AvThird, 4)]);
    assert_eq!(world.dynamic().avbonus, 5);
    assert_eq!(
        World::new()
            .own(&[(Ability::AvCurse, -3)])
            .dynamic()
            .avbonus,
        -3
    );
    assert_eq!(World::new().dynamic().avbonus, 0);
}

// Energy use is averaged with what earlier sources gave, in source order, with truncating
// division.
#[test]
fn dynamic_energyuse_is_a_running_average() {
    let world = World::new()
        .own(&[(Ability::EnergyUse, 10)])
        .race(&[(Ability::EnergyUse, 21)])
        .class(&[(Ability::EnergyUse, 6)]);
    // (10+21)/2 = 15, (15+6)/2 = 10.
    assert_eq!(world.dynamic().energyuse, 10);
    assert_eq!(
        World::new()
            .race(&[(Ability::EnergyUse, 21)])
            .dynamic()
            .energyuse,
        21
    );
    // The total keeps the best instead.
    assert_eq!(world.value(Ability::EnergyUse), 21);
}

// The status abilities set their flag whatever their value; only the FROMABILS bits are
// touched.
#[test]
fn dynamic_sets_status_flags_within_the_from_abils_masks() {
    let world = World::new().own(&[
        (Ability::Confusion, 0),
        (Ability::Mute, 1),
        (Ability::Paralyze, -1),
        (Ability::BlindUser, 2),
        (Ability::SeeHidden, 3),
        (Ability::Slowness, 4),
        (Ability::Quickness, 5),
        (Ability::HoldPerson, 6),
        (Ability::Flee, 7),
    ]);
    let stats = world.dynamic();
    assert_eq!(stats.status_from_abilities, PlayerStatus::FROMABILS);
    assert_eq!(stats.status2_from_abilities, PlayerStatus2::FROMABILS);
    assert!(PlayerStatus::FROMABILS.contains(
        PlayerStatus::CONFUSED
            | PlayerStatus::MUTE
            | PlayerStatus::PARALYSED
            | PlayerStatus::BLIND
            | PlayerStatus::SEEHIDDEN
    ));
    assert_eq!(
        World::new().dynamic().status_from_abilities,
        PlayerStatus::empty()
    );
}

// dynamic_stats: a "Negate ability" slot on a spell cancels the whole field the ability
// feeds, whatever source gave it, or clears its flag.
#[test]
fn dynamic_negate_clears_the_whole_field() {
    let world = kang_witchunter()
        .own(&[
            (Ability::Confusion, 1),
            (Ability::AvBless, 4),
            (Ability::HealRate, 2),
        ])
        .wear(ring(1, &[(DEFENCE, 3)]))
        .cast(
            spell(
                1,
                &[
                    (Ability::Negate, DEFENCE.code() as i16),
                    (Ability::Negate, Ability::Confusion.code() as i16),
                    (Ability::Negate, Ability::AvCurse.code() as i16),
                    (Ability::Negate, Ability::HealRate.code() as i16),
                    (Ability::Negate, Ability::Bash.code() as i16),
                    (Ability::Negate, -1),
                ],
            ),
            0,
        );
    let stats = world.dynamic();
    assert_eq!(stats.acbonus, 0);
    assert_eq!(stats.drbonus, 10);
    assert_eq!(stats.avbonus, 0);
    assert_eq!(stats.healrate, 0);
    assert_eq!(stats.status_from_abilities, PlayerStatus::empty());
}

// dynamic_stats: the sources are applied in the totals' order,
// which the energy-use average exposes: own, spells, race, class, worn, weapon, pack.
#[test]
fn dynamic_applies_sources_in_the_original_order() {
    let world = World::new()
        .own(&[(Ability::EnergyUse, 100)])
        .cast(spell(1, &[(Ability::EnergyUse, 0)]), 50)
        .race(&[(Ability::EnergyUse, 20)])
        .class(&[(Ability::EnergyUse, 10)])
        .wear(ring(1, &[(Ability::EnergyUse, 8)]))
        .wield(sword(2, &[(Ability::EnergyUse, 6)]))
        .carry(trinket(3, &[(Ability::EnergyUse, 4)]));
    // 100 -> (100+50)/2=75 -> (75+20)/2=47 -> (47+10)/2=28 -> (28+8)/2=18 -> (18+6)/2=12
    // -> (12+4)/2=8.
    assert_eq!(world.dynamic().energyuse, 8);
}

// ---------------------------------------------------------------------------------------
// Monsters.

struct Lair {
    monster: ActiveMonster,
    known: Option<KnownMonster>,
    spells: Vec<Spell>,
    items: Vec<Item>,
}

impl Lair {
    fn new(known: Option<KnownMonster>) -> Self {
        Self {
            monster: active_monster(),
            known,
            spells: Vec::new(),
            items: Vec::new(),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&MonsterAbilities<'_>) -> R) -> R {
        let spells = |n: SpellNo| self.spells.iter().find(|s| s.number == n);
        let items = |n: ItemNo| self.items.iter().find(|i| i.number == n);
        f(&MonsterAbilities {
            monster: &self.monster,
            known: self.known.as_ref(),
            spells: &spells,
            items: &items,
        })
    }
}

// A monster's total: spells, the known record, every carried item, the weapon and the worn
// item add up; a wearable item in the pack counts too.
#[test]
fn monster_total_counts_every_carried_item() {
    let mut lair = Lair::new(Some(known_monster(&[(DEFENCE, 1)])));
    lair.monster.spells[0] = ActiveSpell {
        spell: SpellNo(1),
        value: 20,
        rounds: 3,
    };
    lair.spells
        .push(spell(1, &[(DEFENCE, 2), (Ability::Damage, 0)]));
    lair.monster.items[0] = (ItemNo(1), 1);
    lair.monster.items[1] = (ItemNo(2), 1);
    lair.monster.weapon = ItemNo(3);
    lair.monster.wornitem = ItemNo(4);
    lair.items.push(ring(1, &[(DEFENCE, 4)]));
    lair.items.push(sword(2, &[(DEFENCE, 8)]));
    lair.items.push(sword(3, &[(DEFENCE, 16)]));
    lair.items.push(ring(4, &[(DEFENCE, 32)]));
    assert_eq!(lair.with(|l| l.value(DEFENCE)), 63);
    // The rolled value stands in for a 0.
    assert_eq!(lair.with(|l| l.value(Ability::Damage)), 20);
    assert!(lair.with(|l| l.has(DEFENCE)));
    assert!(lair.with(|l| l.has(Ability::Damage)));
    assert!(!lair.with(|l| l.has(Ability::Critical)));
    assert_eq!(lair.with(|l| l.pair(DEFENCE, Ability::Damage)), (63, 20));
}

// The elemental defences and resistances keep the best value for monsters, on top of the
// seven of characters; an item whose
// record is missing still contributes 0.
#[test]
fn monster_best_value_set_and_missing_item() {
    assert!(
        MONSTER_BEST_VALUE.len() == 11 && BEST_VALUE.iter().all(|a| MONSTER_BEST_VALUE.contains(a))
    );
    for ability in MONSTER_BEST_VALUE {
        let mut lair = Lair::new(Some(known_monster(&[(ability, 3), (ability, 5)])));
        assert_eq!(lair.with(|l| l.value(ability)), 5, "{ability:?}");
        lair.known = Some(known_monster(&[(ability, -5)]));
        assert_eq!(lair.with(|l| l.value(ability)), -5, "{ability:?}");
        lair.monster.weapon = ItemNo(9);
        assert_eq!(lair.with(|l| l.value(ability)), 0, "{ability:?}");
        assert!(lair.with(|l| l.has(ability)), "{ability:?}");
    }
    assert_eq!(
        Lair::new(Some(known_monster(&[(DEFENCE, 3), (DEFENCE, 5)]))).with(|l| l.value(DEFENCE)),
        8
    );
    assert_eq!(Lair::new(None).with(|l| l.best(Ability::ColdDefence)), None);
}

// A monster's total: "Negate ability" on a spell gives 0; the 65535 slot has nothing.
#[test]
fn monster_negate_and_no_monster_slot() {
    let mut lair = Lair::new(Some(known_monster(&[(DEFENCE, 9)])));
    lair.monster.spells[0] = ActiveSpell {
        spell: SpellNo(1),
        value: 0,
        rounds: 1,
    };
    lair.spells
        .push(spell(1, &[(Ability::Negate, DEFENCE.code() as i16)]));
    assert_eq!(lair.with(|l| l.value(DEFENCE)), 0);
    assert!(lair.with(|l| l.has(DEFENCE)));
    lair.monster.number = ActiveMonsterId(65535);
    assert!(!lair.with(|l| l.has(DEFENCE)));
    assert_eq!(lair.with(|l| l.value(DEFENCE)), 0);
}

// ---------------------------------------------------------------------------------------
// The name table.

// The ability names and numbers.
#[test]
fn every_ability_has_a_name_and_the_numbering_matches_the_header() {
    assert_eq!(ABILITY_NAMES.len(), NABILNAMES - 1);
    assert_eq!(Ability::ALL.len(), NABILNAMES - 1);
    for (i, ability) in Ability::ALL.iter().enumerate() {
        assert_eq!(usize::from(ability.code()), i, "{ability:?}");
        assert!(!ability_name(*ability).is_empty(), "{ability:?}");
        assert_eq!(ability_name(*ability), ABILITY_NAMES[i]);
    }
    assert_eq!(ability_name(Ability::Unknown(188)), "");
    for (ability, code, name) in [
        (Ability::None, 0, "No ability"),
        (Ability::Defence, 2, "Alter Defence Value"),
        (Ability::DamageResist, 7, "Alter Damage Resistance Value"),
        (Ability::PoisonImmune, 21, "Poison Immunity"),
        (Ability::Magical, 28, "Magical"),
        (Ability::Bash, 31, "Bash"),
        (Ability::Smash, 32, "Smash"),
        (Ability::Dodge, 34, "Dodge"),
        (Ability::AntiMagic, 51, "Anti Magic"),
        (Ability::SeeHidden, 57, "See Hidden"),
        (Ability::Encumbrance, 96, "Alter Encumbrance"),
        (Ability::DefenceMod, 104, "Defence Modifier"),
        (Ability::Negate, 124, "Negate ability"),
        (Ability::HitMagical, 142, "Hit Magical"),
        (Ability::Meditate, 187, "Meditate"),
    ] {
        assert_eq!(ability.code(), code, "{ability:?}");
        assert_eq!(ability_name(ability), name, "{ability:?}");
    }
    assert_eq!(
        AbilityName(Ability::Defence).to_string(),
        "Alter Defence Value (2)"
    );
}

// ---------------------------------------------------------------------------------------
// Properties.

proptest! {
    // An additive total is the sum of its slots, whatever source holds them and in whatever
    // order.
    #[test]
    fn additive_total_is_order_independent(
        values in prop::collection::vec(-300i16..300, 0..24),
        rotate in 0usize..24,
        split in 0usize..3,
    ) {
        let expected: i32 = values.iter().map(|&v| i32::from(v)).sum();
        let mut rotated = values.clone();
        if !rotated.is_empty() {
            let by = rotate % rotated.len();
            rotated.rotate_left(by);
        }
        // Spread the slots over own, race and class in two different ways.
        let cut = rotated.len().min(split * 4);
        let (own, rest) = rotated.split_at(cut);
        let (race_part, class_part) = rest.split_at(rest.len().min(NRECABILS));
        let as_slots = |vals: &[i16]| vals.iter().map(|&v| (DEFENCE, v)).collect::<Vec<_>>();
        let straight = World::new()
            .own(&as_slots(&values[..values.len().min(NABILITY)]))
            .race(&as_slots(&values[values.len().min(NABILITY)..]));
        let spread = World::new()
            .own(&as_slots(own))
            .race(&as_slots(race_part))
            .class(&as_slots(&class_part[..class_part.len().min(NRECABILS)]));
        prop_assert_eq!(straight.value(DEFENCE), expected);
        if class_part.len() <= NRECABILS {
            prop_assert_eq!(spread.value(DEFENCE), expected);
            prop_assert_eq!(spread.dynamic().acbonus, expected);
        }
    }

    // A "Negate ability" spell naming the ability always gives 0; dynamic_stats clears the
    // field.
    #[test]
    fn negate_always_yields_zero(
        own in prop::collection::vec(-300i16..300, 0..NRECABILS),
        race_val in -300i16..300,
        item_val in -300i16..300,
        ability in prop::sample::select(vec![DEFENCE, Ability::DamageResist, Ability::AvBless, Ability::EnergyUse]),
    ) {
        let as_slots = own.iter().map(|&v| (ability, v)).collect::<Vec<_>>();
        let world = World::new()
            .own(&as_slots)
            .race(&[(ability, race_val)])
            .wear(ring(1, &[(ability, item_val)]))
            .cast(spell(1, &[(ability, 1), (Ability::Negate, ability.code() as i16)]), 0);
        prop_assert_eq!(world.value(ability), 0);
        prop_assert_eq!(world.with(|l| l.best(ability)), Some(0));
        let stats = world.dynamic();
        let field = match ability {
            DEFENCE => stats.acbonus,
            Ability::DamageResist => stats.drbonus,
            Ability::AvBless => stats.avbonus,
            _ => stats.energyuse,
        };
        prop_assert_eq!(field, 0);
    }
}
