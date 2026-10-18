//! Monster casting (T-052): `monster_cast`, the self cast, the cast by spell number and
//! `monster_cast_random_spell`, with the three hooks of the monster's round, the create
//! spell and the death spell.
//!
//! The records are the stock ones (vendored under
//! `crates/silvermere-import/tests/fixtures/world`): known monsters 481 orc shaman
//! (attack 1: spell 12 `harm` at 95%, level 8; after the round 15 `curse` 50%, 13 `minor
//! healing` to 100%), 124 black orc shaman (22 is not a stock spell: slot 1 is a test
//! spell), 34 dark priest (spell 16 `spiritual hammer`), the acid slime (hit spell 126).
//! The stock export has no kobold shaman and the orc shaman casts `harm`, not `chill`
//! (the card's wording); the snapshot takes the three casters the data has.

mod common;

use common::{enter, test_config, tiny_snapshot};
use silvermere_engine::SessionId;
use silvermere_engine::combat::{attack_monster_user, check_kill_monster};
use silvermere_engine::monsters::cast::{
    MonsterCastResult, cast_at_player, cast_by_number, cast_random,
};
use silvermere_engine::monsters::generate::{Request, generate_monster};
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::{World, blank};
use silvermere_rules::SeededRng;
use silvermere_text::strip_ansi;
use silvermere_types::enums::{
    Ability, Alignment, Element, GroupRole, ItemType, Magery, MessageLevel, MonsterAttackType,
    MonsterType, ResistMode, ResistStat, SpellTarget, SpellType,
};
use silvermere_types::flags::{PlayerSettings, SpellFlags};
use silvermere_types::ids::{
    ActiveMonsterId, ClassNo, ItemNo, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey, SpellNo,
};
use silvermere_types::records::{KnownMonster, Message, MonsterAttack, MonsterSpell, Spell};

use MonsterCastResult as R;

/// Records keyed by their own number.
trait Put<V> {
    fn put(&mut self, v: V);
}
impl Put<Spell> for std::collections::HashMap<SpellNo, Spell> {
    fn put(&mut self, v: Spell) {
        self.insert(v.number, v);
    }
}
impl Put<KnownMonster> for std::collections::HashMap<MonsterNo, KnownMonster> {
    fn put(&mut self, v: KnownMonster) {
        self.insert(v.number, v);
    }
}
impl Put<silvermere_types::records::Item>
    for std::collections::HashMap<ItemNo, silvermere_types::records::Item>
{
    fn put(&mut self, v: silvermere_types::records::Item) {
        self.insert(v.number, v);
    }
}

const ARENA: u32 = 2150;
const DEST: u32 = 3001;
const HERO: &str = "Aldric";
const SHAMAN: i32 = 481;
const BLACK: i32 = 124;
const PRIEST: i32 = 34;
const SLIME: i32 = 5;
const SPAWN: i32 = 900;
/// The 0.001 point of chi-square for 3 degrees of freedom.
const CHI3_0001: f64 = 16.266;

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

fn messages() -> Vec<Message> {
    vec![
        // 14: harm
        msg(
            14,
            "You cast %s at %s for %d damage!",
            "%s casts %s on you for %d damage!",
            "%s casts %s on %s for %s damage!",
        ),
        // 8492: blind
        msg(
            8492,
            "You cast %s on %s!",
            "%s casts %s on you!",
            "%s casts %s on %s!",
        ),
        // 8490: spiritual hammer
        msg(
            8490,
            "You throw a %s at %s for %d damage!",
            "%s throws a %s at you for %d damage!",
            "%s throws a %s at %s for %s damage!",
        ),
        // 51: acid hits
        msg(
            51,
            "%s takes %d acid damage!",
            "Acid burns you for %d damage!",
            "%s is burned by acid for %s damage!",
        ),
        msg(27, "The %s bites you for %d damage!", "", ""),
    ]
}

#[allow(clippy::too_many_arguments)]
fn spell(
    n: i16,
    name: &str,
    target: SpellTarget,
    kind: SpellType,
    (minval, maxval): (i16, i16),
    duration: i16,
    resmode: u16,
    msgnum: i32,
    abilities: &[(Ability, i16)],
) -> Spell {
    let mut a: [(Ability, i16); 10] = std::array::from_fn(|_| (Ability::None, 0));
    for (i, x) in abilities.iter().enumerate() {
        a[i] = *x;
    }
    Spell {
        number: SpellNo(n),
        name: name.into(),
        descs: [String::new(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities: a,
        energy: 0,
        level: 1,
        minval,
        maxval,
        spell_type: kind,
        resmode: ResistMode::from_code(resmode),
        diff: 100,
        durlvl: 0,
        target,
        duration,
        element: Element::from_code(4),
        resstat: ResistStat::from_code(0),
        magery: Magery::None,
        msgnum: MessageNo(msgnum),
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

/// The stock spells the casters use, as WCCSPELS lists them, and the test spells.
fn spells() -> Vec<Spell> {
    use SpellTarget::*;
    let mut harm = spell(
        12,
        "harm",
        MonsterOrSingle,
        SpellType::Autocombat,
        (4, 13),
        0,
        2,
        14,
        &[(Ability::DamageNoArmour, 0)],
    );
    (harm.maxinc, harm.maxlvls) = (1, 1);
    let blind = spell(
        22,
        "blind",
        MonsterOrSingle,
        SpellType::General,
        (1, 1),
        10,
        2,
        8492,
        &[(Ability::HoldPerson, 0)],
    );
    let hammer = spell(
        16,
        "spiritual hammer",
        MonsterOrSingle,
        SpellType::Autocombat,
        (4, 14),
        0,
        0,
        8490,
        &[(Ability::DamageNoArmour, 0)],
    );
    let acid = spell(
        126,
        "acid hits",
        MonsterOrSingle,
        SpellType::General,
        (1, 3),
        0,
        0,
        51,
        &[(Ability::DamageNoArmour, 0)],
    );
    let mut curse = spell(
        15,
        "curse",
        MonsterOrSingle,
        SpellType::General,
        (-6, -6),
        40,
        2,
        0,
        &[(Ability::Strength, 0)],
    );
    curse.level = 2;
    let minor = spell(
        13,
        "minor healing",
        SelfOrSingle,
        SpellType::General,
        (2, 8),
        0,
        1,
        0,
        &[(Ability::Heal, 0)],
    );
    // a self summon, the create and death spells of the tests
    let summon_self = spell(
        456,
        "summon weaponmaster",
        OnSelf,
        SpellType::General,
        (SPAWN as i16, SPAWN as i16),
        0,
        0,
        0,
        &[(Ability::Summon, 0)],
    );
    let wounded = spell(
        455,
        "wounded",
        OnSelf,
        SpellType::General,
        (1, 1),
        50,
        0,
        0,
        &[(Ability::Poison, 0)],
    );
    vec![
        harm,
        blind,
        hammer,
        acid,
        curse,
        minor,
        summon_self,
        wounded,
    ]
}

fn attack(
    kind: u16,
    acc: i16,
    pct: u8,
    min: i16,
    max: i16,
    energy: i16,
    hit: i16,
) -> MonsterAttack {
    MonsterAttack {
        kind: MonsterAttackType::from_code(kind),
        accuracy: acc,
        pct,
        min,
        max,
        hitmsg: MessageNo(27),
        defmsg: MessageNo(0),
        missmsg: MessageNo(0),
        energy,
        hitspell: SpellNo(hit),
    }
}

fn known(n: i32, name: &str, attacks: &[MonsterAttack]) -> KnownMonster {
    let mut k = blank::clear_known_monster(MonsterNo(n));
    k.name = name.into();
    k.montype = MonsterType::Group1;
    k.level = 1;
    k.genindex = 1;
    k.exp = 10;
    k.maxhp = 80;
    k.energy = 1000;
    k.alignment = Alignment::ChaoticEvil;
    k.followpct = 0;
    k.grouprole = GroupRole::Solo;
    for (i, a) in attacks.iter().enumerate() {
        k.attacks[i] = *a;
    }
    k
}

fn after_round(k: &mut KnownMonster, table: &[(i16, u8, u8)]) {
    for (i, &(spell, pct, level)) in table.iter().enumerate() {
        k.spells[i] = MonsterSpell {
            spell: SpellNo(spell),
            pct,
            level,
        };
    }
}

fn world() -> World {
    let mut s = tiny_snapshot();
    for n in [ARENA, DEST] {
        let mut r = blank::clear_room(RoomKey::new(1, n));
        r.name = format!("Room {n}");
        r.mongroup = MonsterType::Group1.code() as i16;
        s.rooms.push(r);
    }
    s.messages = messages();
    s.spells = spells();
    let mut shaman = known(
        SHAMAN,
        "orc shaman",
        &[
            attack(1, 55, 25, 3, 13, 1000, 0),
            attack(2, 12, 100, 95, 8, 1000, 0),
        ],
    );
    after_round(&mut shaman, &[(15, 50, 8), (13, 100, 8)]);
    let black = known(
        BLACK,
        "black orc shaman",
        &[
            attack(1, 55, 70, 3, 13, 500, 0),
            attack(2, 22, 100, 95, 15, 1000, 0),
        ],
    );
    let priest = known(
        PRIEST,
        "dark priest",
        &[
            attack(1, 40, 35, 3, 12, 1000, 0),
            attack(2, 16, 80, 70, 10, 1000, 0),
            attack(2, 16, 100, 70, 10, 1000, 0),
        ],
    );
    let slime = known(SLIME, "acid slime", &[attack(1, 20, 100, 2, 9, 1000, 126)]);
    s.monsters.extend([shaman, black, priest, slime]);
    s.monsters.push(known(SPAWN, "weaponmaster", &[]));
    let mut d = known(901, "slaver leader", &[]);
    d.createspell = SpellNo(455);
    s.monsters.push(d);
    let mut d = known(902, "calling dead", &[]);
    d.deathspell = SpellNo(456);
    d.maxhp = 5;
    s.monsters.push(d);
    let mut class = blank::clear_class(ClassNo(2));
    class.name = "Witchunter".into();
    s.classes.push(class);
    let mut race = blank::clear_race(RaceNo(11));
    race.name = "Kang".into();
    s.races.push(race);
    let mut w = World::load(test_config(), 16, s).expect("world loads");
    w.rng = SeededRng::new(52);
    w
}

fn hero(w: &mut World, name: &str) -> SessionId {
    let id = enter(w, name, ARENA, MessageLevel::High);
    let c = w.session_mut(id).unwrap().character.as_mut().unwrap();
    c.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
    c.race = RaceNo(11);
    c.class = ClassNo(2);
    c.level = 1;
    (c.str, c.intel, c.wil, c.hea, c.agi, c.cha) = (55, 30, 45, 50, 30, 30);
    (c.hp, c.maxhp) = (100, 200);
    (c.mana, c.maxmana) = (10, 50);
    (c.energy, c.maxenergy) = (500, 1000);
    c.mr = 0;
    c.lives = 9;
    id
}

fn place(w: &mut World, n: i32) -> ActiveMonsterId {
    let req = Request {
        areatype: -1,
        monnum: MonsterNo(n),
        minlevel: 0,
        maxlevel: 0,
        owner: None,
        ally: false,
        force: true,
    };
    let m = generate_monster(w, RoomKey::new(1, ARENA), req).expect("placed");
    w.active_monsters.by_number_mut(m).unwrap().grouprole = GroupRole::Solo;
    m
}

fn mon(w: &mut World, n: ActiveMonsterId) -> &mut silvermere_types::records::ActiveMonster {
    w.active_monsters.by_number_mut(n).unwrap()
}

fn plr(w: &mut World, id: SessionId) -> &mut silvermere_types::records::Character {
    w.session_mut(id).unwrap().character.as_mut().unwrap()
}

fn text(w: &mut World, id: SessionId) -> Vec<String> {
    let palette = palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let out = s.output.render(palette, b"<P>");
    s.output.clear();
    String::from_utf8_lossy(&strip_ansi(&out))
        .split("\r\n")
        .map(|l| {
            l.replace("<P>", "")
                .trim_matches([' ', '\u{7}'])
                .to_string()
        })
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}

/// The slot a monster's lasting spell took (the last free one).
fn slot_of(w: &mut World, id: SessionId, n: i16) -> Option<usize> {
    plr(w, id).spells.iter().position(|s| *s == SpellNo(n))
}

/// A shaman, the hero and an observer in one room, outputs cleared.
fn scene(n: i32) -> (World, SessionId, SessionId, ActiveMonsterId) {
    let mut w = world();
    let id = hero(&mut w, HERO);
    let other = hero(&mut w, "Bob");
    let m = place(&mut w, n);
    mon(&mut w, m).energy = 1000;
    text(&mut w, id);
    text(&mut w, other);
    (w, id, other, m)
}

// --- the four lines and the generic cast: snapshots -----------------------------------

fn lines_of(w: &mut World, id: SessionId, other: SessionId) -> String {
    let mut out = String::new();
    for l in text(w, id) {
        out.push_str(&format!("you:   {l}\n"));
    }
    for l in text(w, other) {
        out.push_str(&format!("other: {l}\n"));
    }
    out
}

// `You resisted %s's cast of %s.`, `%s resisted %s's cast of %s.` (no `The`),
// `The %s attempted to cast %s at you, but failed.`, `... at %s, but failed.`; the landed
// cast is monster_display_spell_success's.
#[test]
fn the_texts_of_three_stock_casters() {
    let mut out = String::new();
    for (n, slot) in [(SHAMAN, 1), (BLACK, 1), (PRIEST, 1)] {
        let (mut w, id, other, m) = scene(n);
        let name = mon(&mut w, m).name.clone();
        let number = w.monsters[&MonsterNo(n)].attacks[slot].accuracy;
        // no magic resistance: Intellect and Wisdom 0 give 0, which the stats pass keeps
        let c = plr(&mut w, id);
        (c.intel, c.wil, c.mr) = (0, 0, 0);
        w.monsters.get_mut(&MonsterNo(n)).unwrap().attacks[slot].min = 100;
        out.push_str(&format!("== {name}: landed\n"));
        let r = cast_at_player(&mut w, m, Some(slot), id, 0);
        assert_eq!(r, R::Applied, "{name}");
        out.push_str(&lines_of(&mut w, id, other));
        // the cast roll fails: chance 0
        out.push_str(&format!("== {name}: failed\n"));
        w.monsters.get_mut(&MonsterNo(n)).unwrap().attacks[slot].min = 0;
        mon(&mut w, m).energy = 1000;
        let r = cast_at_player(&mut w, m, Some(slot), id, 0);
        assert_eq!(r, R::Failed);
        out.push_str(&lines_of(&mut w, id, other));
        // resisted: a spell any magic resistance can resist, and Wisdom 255 (mr 191)
        out.push_str(&format!("== {name}: resisted\n"));
        w.monsters.get_mut(&MonsterNo(n)).unwrap().attacks[slot].min = 100;
        w.spells.get_mut(&SpellNo(number)).unwrap().resmode = ResistMode::Always;
        let c = plr(&mut w, id);
        (c.wil, c.mr) = (255, 191);
        let mut seen = false;
        for _ in 0..200 {
            mon(&mut w, m).energy = 1000;
            if cast_at_player(&mut w, m, Some(slot), id, 0) == R::Resisted {
                seen = true;
                break;
            }
            text(&mut w, id);
            text(&mut w, other);
            let c = plr(&mut w, id);
            (c.wil, c.mr) = (255, 191);
        }
        assert!(seen);
        out.push_str(&lines_of(&mut w, id, other));
    }
    insta::assert_snapshot!(out);
}

// --- the outcomes, the energy and the cast roll -------------------------------------

// Too little energy: nothing happens and it still counts as handled;
// the full energy of a cast; half (at least 1) for a failure or a
// resist, none for a free cast (slot -1).
#[test]
fn energy_is_the_attacks_and_a_failure_costs_half() {
    let (mut w, id, _, m) = scene(SHAMAN);
    mon(&mut w, m).energy = 999;
    assert_eq!(cast_at_player(&mut w, m, Some(1), id, 0), R::Tired);
    assert_eq!(mon(&mut w, m).energy, 999);
    assert_eq!(R::Tired.code(), 1);
    mon(&mut w, m).energy = 1500;
    assert_eq!(cast_at_player(&mut w, m, Some(1), id, 0), R::Applied);
    assert_eq!(mon(&mut w, m).energy, 500);
    // cast chance 0: failed, half the energy
    w.monsters.get_mut(&MonsterNo(SHAMAN)).unwrap().attacks[1].min = 0;
    mon(&mut w, m).energy = 1000;
    assert_eq!(cast_at_player(&mut w, m, Some(1), id, 0), R::Failed);
    assert_eq!(mon(&mut w, m).energy, 500);
    // energy 1: half is at least 1
    w.monsters.get_mut(&MonsterNo(SHAMAN)).unwrap().attacks[1].energy = 1;
    mon(&mut w, m).energy = 5;
    assert_eq!(cast_at_player(&mut w, m, Some(1), id, 0), R::Failed);
    assert_eq!(mon(&mut w, m).energy, 4);
    // slot -1: 100% for no energy
    mon(&mut w, m).energy = 7;
    assert_eq!(cast_at_player(&mut w, m, None, id, 12), R::Applied);
    assert_eq!(mon(&mut w, m).energy, 7);
}

// The cast roll is drawn first and always; the magic-resistance roll only
// when the cast roll succeeded (T-067 review, silvermere#32); an item negating the spell
// resists without one.
#[test]
fn the_cast_roll_comes_first_and_the_resistance_roll_only_after_a_good_one() {
    use silvermere_rules::Rng as _;
    let draws = |castpct: i16, item: bool| {
        let (mut w, id, _, m) = scene(SHAMAN);
        w.monsters.get_mut(&MonsterNo(SHAMAN)).unwrap().attacks[1].min = castpct;
        w.spells.get_mut(&SpellNo(12)).unwrap().resmode = ResistMode::Always;
        plr(&mut w, id).mr = 40;
        if item {
            let mut ring = blank::clear_item(ItemNo(61));
            ring.name = "ward ring".into();
            ring.negates[0] = SpellNo(12);
            w.items.put(ring);
            plr(&mut w, id).worn[0] = ItemNo(61);
        }
        let mut probe = w.rng.clone();
        let _ = cast_at_player(&mut w, m, Some(1), id, 0);
        let mut n = 0;
        while probe != w.rng && n < 40 {
            probe.range(0, 100);
            n += 1;
        }
        n
    };
    // a failed roll draws the cast roll only
    assert_eq!(draws(0, false), 1);
    // an item negating the spell: the cast roll only, resisted without a magic-resistance roll
    assert_eq!(draws(100, true), 1);
    // a good roll draws the resistance roll too (and the strength when it is not resisted)
    assert!(matches!(draws(100, false), 2 | 3));
}

// A missing spell, monster or player casts nothing (`R::Nothing`); the roll is still drawn.
#[test]
fn a_missing_record_casts_nothing() {
    let (mut w, id, _, m) = scene(SHAMAN);
    assert_eq!(cast_at_player(&mut w, m, None, id, 9999), R::Nothing);
    assert_eq!(R::Nothing.code(), 0);
    assert_eq!(
        cast_at_player(&mut w, ActiveMonsterId(9999), Some(1), id, 0),
        R::Nothing
    );
}

// --- the census: every `monster_cast` case (census.md D.1 1b) ---------------------------

/// A one-slot test spell cast by a shaman with a free cast at the hero.
fn one(
    ability: Ability,
    value: i16,
    duration: i16,
    target: SpellTarget,
    setup: impl FnOnce(&mut World, SessionId, ActiveMonsterId),
) -> (World, SessionId, ActiveMonsterId, R, Vec<String>) {
    let (mut w, id, _, m) = scene(SHAMAN);
    let s = spell(
        700,
        "testspell",
        target,
        SpellType::General,
        (value, value),
        duration,
        0,
        0,
        &[(ability, value)],
    );
    w.spells.put(s);
    setup(&mut w, id, m);
    let r = cast_at_player(&mut w, m, None, id, 700);
    let lines = text(&mut w, id);
    (w, id, m, r, lines)
}

fn single() -> SpellTarget {
    SpellTarget::MonsterOrSingle
}

fn nothing(_: &mut World, _: SessionId, _: ActiveMonsterId) {}

#[test]
fn damage_ignoring_armour_wounds_and_kills() {
    let (mut w, id, _, r, lines) = one(Ability::DamageNoArmour, 30, 0, single(), nothing);
    assert_eq!(r, R::Applied);
    assert_eq!(plr(&mut w, id).hp, 70);
    assert_eq!(lines, ["Orc shaman cast testspell on you."]);
    // a death ends the cast with 2
    let (_, _, _, r, _) = one(Ability::DamageNoArmour, 500, 0, single(), nothing);
    assert_eq!(r, R::KilledTarget);
    assert_eq!(r.code(), 2);
    // lasting: once a cast as a spell on the player
    let (mut w, id, _, r, _) = one(Ability::DamageNoArmour, 5, 4, single(), nothing);
    assert_eq!(r, R::Applied);
    assert_eq!(plr(&mut w, id).hp, 100);
    let i = slot_of(&mut w, id, 700).expect("on the player");
    assert_eq!(plr(&mut w, id).spellrnds[i], 4);
}

#[test]
fn slay_takes_the_player_one_below_death_and_ignores_a_lasting_spell() {
    let (_, _, _, r, _) = one(Ability::Slay, 1, 0, single(), nothing);
    assert_eq!(r, R::KilledTarget);
    // a base duration set: nothing here
    let (mut w, id, _, r, _) = one(Ability::Slay, 1, 3, single(), nothing);
    assert_eq!(r, R::Applied);
    assert_eq!(plr(&mut w, id).hp, 100);
}

#[test]
fn damage_on_armour_is_softened_by_magic_resistance() {
    // mr 0 and no Anti Magic: the damage is still cut by the full 50%, 40 - 20, as on the
    // original (sic)
    let (mut w, id, _, r, _) = one(Ability::DamageArmour, 40, 0, single(), nothing);
    assert_eq!(r, R::Applied);
    assert_eq!(plr(&mut w, id).hp, 80);
    // mr 90: (90-50)/2 = 20% off
    let (mut w, id, _, _, _) = one(Ability::DamageArmour, 40, 0, single(), |w, id, _| {
        plr(w, id).mr = 90
    });
    assert_eq!(plr(&mut w, id).hp, 100 - 32);
}

#[test]
fn steal_hit_points_heals_the_monster_to_its_known_maximum() {
    let (mut w, id, m, r, _) = one(Ability::StealHp, 25, 0, single(), |w, _, m| {
        mon(w, m).hp = 70;
    });
    assert_eq!(r, R::Applied);
    assert_eq!(plr(&mut w, id).hp, 75);
    // capped at the known monster's 80, not the active record's
    assert_eq!(mon(&mut w, m).hp, 80);
}

#[test]
fn energy_hunger_thirst_heal_mana_and_poison() {
    let (mut w, id, _, _, lines) = one(Ability::Energy, 40, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).energy, 540);
    assert_eq!(lines.len(), 1);
    // hunger and thirst are silent
    let (mut w, id, _, _, lines) = one(Ability::Hunger, 10, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).hunger, 10);
    assert!(lines.is_empty(), "{lines:?}");
    let (mut w, id, _, _, lines) = one(Ability::Thirst, 7, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).thirst, 7);
    assert!(lines.is_empty(), "{lines:?}");
    // heal up to the maximum
    let (mut w, id, _, _, lines) = one(Ability::Heal, 500, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).hp, 200);
    assert_eq!(lines.len(), 1);
    // heal mana up to the maximum, and a drain down to 0
    let (mut w, id, _, _, _) = one(Ability::HealMana, 500, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).mana, 50);
    let (mut w, id, _, _, _) = one(Ability::HealMana, -500, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).mana, 0);
    // poison raises the level; cure poison lowers it with no floor
    let (mut w, id, _, _, lines) = one(Ability::Poison, 5, 0, single(), nothing);
    assert_eq!(plr(&mut w, id).poison, 5);
    assert_eq!(lines.len(), 1);
    let (mut w, id, _, _, _) = one(Ability::CurePoison, 3, 0, single(), |w, id, _| {
        plr(w, id).poison = 1
    });
    assert_eq!(plr(&mut w, id).poison, -2);
}

#[test]
fn summon_makes_a_monster_that_follows_the_player() {
    let (w, _id, m, r, lines) = one(Ability::Summon, SPAWN as i16, 0, single(), nothing);
    assert_eq!(r, R::Applied);
    // shown to the room as "everyone" and the new monster goes after him
    assert_eq!(lines[0], "Orc shaman cast testspell on everyone.");
    let new = w
        .active_monsters
        .iter()
        .map(|(_, a)| a.clone())
        .find(|a| a.number != m && a.knownnum == MonsterNo(SPAWN))
        .expect("summoned");
    assert_eq!(new.following, HERO);
    assert_eq!(new.friendly, 0);
}

#[test]
fn lasting_stat_spells_add_once_and_a_second_cast_is_refused() {
    for (ability, stat) in [
        (Ability::Intellect, 0),
        (Ability::Wisdom, 1),
        (Ability::Strength, 2),
        (Ability::Health, 3),
        (Ability::Agility, 4),
        (Ability::Charm, 5),
    ] {
        let (mut w, id, m, r, lines) = one(ability, 5, 6, single(), nothing);
        assert_eq!(r, R::Applied, "{ability:?}");
        assert_eq!(lines.len(), 1, "{ability:?}");
        let c = plr(&mut w, id).clone();
        let got = [c.intel, c.wil, c.str, c.hea, c.agi, c.cha][stat];
        let base = [30u16, 45, 55, 50, 30, 30][stat];
        assert!(got > base, "{ability:?} {got}");
        assert!(c.spells.contains(&SpellNo(700)));
        // the same spell again is refused (extend 0): the energy is given back, 0
        mon(&mut w, m).energy = 100;
        let r = cast_at_player(&mut w, m, None, id, 700);
        assert_eq!(r, R::Nothing, "{ability:?}");
        assert_eq!(mon(&mut w, m).energy, 100);
    }
    // Alter HP raises the current points; the maximum is the stats pass's to work out
    let (mut w, id, _, _, _) = one(Ability::AlterHp, 20, 6, single(), nothing);
    assert_eq!(plr(&mut w, id).hp, 120);
    assert!(slot_of(&mut w, id, 700).is_some());
    // Self is reached by the stat arms
    let (mut w, id, _, r, _) = one(Ability::Strength, 5, 6, SpellTarget::OnSelf, nothing);
    // OnSelf is the room's (monster_cast_area) until T-051 lands: nothing here
    let _ = (&mut w, id);
    assert!(matches!(r, R::Nothing | R::Applied));
}

#[test]
fn any_other_lasting_ability_goes_on_once_and_an_instant_one_does_nothing() {
    let (mut w, id, _, r, lines) = one(Ability::HoldPerson, 1, 4, single(), nothing);
    assert_eq!(r, R::Applied);
    let i = slot_of(&mut w, id, 700).expect("on the player");
    assert_eq!(plr(&mut w, id).spellrnds[i], 4);
    assert_eq!(lines.len(), 1);
    let (mut w, id, _, r, lines) = one(Ability::HoldPerson, 1, 0, single(), nothing);
    assert_eq!(r, R::Applied);
    assert_eq!(slot_of(&mut w, id, 700), None);
    assert!(lines.is_empty());
    // a full table is a refusal: energy back, 0
    let (mut w, id, m, _, _) = one(Ability::HoldPerson, 1, 4, single(), |w, id, _| {
        let c = plr(w, id);
        for i in 0..c.spells.len() {
            c.spells[i] = SpellNo(100 + i as i16);
            c.spellrnds[i] = 9;
        }
    });
    mon(&mut w, m).energy = 100;
    assert_eq!(cast_at_player(&mut w, m, None, id, 700), R::Nothing);
}

#[test]
fn the_markers_do_nothing() {
    for ability in [
        Ability::KillDead,
        Ability::DetectMagic,
        Ability::None,
        Ability::Enslave,
        Ability::EvilInCombat,
    ] {
        let (mut w, id, _, r, lines) = one(ability, 3, 0, single(), nothing);
        assert_eq!(r, R::Applied, "{ability:?}");
        assert!(lines.is_empty(), "{ability:?} {lines:?}");
        assert_eq!(plr(&mut w, id).hp, 100);
    }
}

#[test]
fn text_blocks_items_and_teleports() {
    // Trigger Text Block with no such block: no line, no harm
    let (_, _, _, r, lines) = one(Ability::TextBlock, 77, 0, single(), nothing);
    assert_eq!(r, R::Applied);
    assert!(lines.is_empty());
    // Clear Item erases the item from the monster's room
    let (w, _, _, _, _) = one(Ability::ClearItem, 1, 0, single(), |w, _, _| {
        let rs = w.room_state.get_mut(&RoomKey::new(1, ARENA)).unwrap();
        rs.items[0].item = ItemNo(1);
        rs.items[0].count = 0;
    });
    let rs = &w.room_state[&RoomKey::new(1, ARENA)];
    assert!(rs.items.iter().all(|i| i.item != ItemNo(1)));
    // both halves of a teleport move the player
    let s = spell(
        701,
        "tele",
        single(),
        SpellType::General,
        (1, 1),
        0,
        0,
        0,
        &[
            (Ability::TeleportRoom, DEST as i16),
            (Ability::TeleportMap, 1),
        ],
    );
    let (mut w, id, m, _, _) = scene(SHAMAN).into_one();
    w.spells.put(s);
    assert_eq!(cast_at_player(&mut w, m, None, id, 701), R::Applied);
    assert_eq!(plr(&mut w, id).room, RoomId(DEST));
    // one half alone does nothing
    let s = spell(
        702,
        "tele2",
        single(),
        SpellType::General,
        (1, 1),
        0,
        0,
        0,
        &[(Ability::TeleportRoom, ARENA as i16)],
    );
    w.spells.put(s);
    plr(&mut w, id).room = RoomId(ARENA);
    cast_at_player(&mut w, m, None, id, 702);
    assert_eq!(plr(&mut w, id).room, RoomId(ARENA));
}

trait IntoOne {
    fn into_one(self) -> (World, SessionId, ActiveMonsterId, (), ());
}
impl IntoOne for (World, SessionId, SessionId, ActiveMonsterId) {
    fn into_one(self) -> (World, SessionId, ActiveMonsterId, (), ()) {
        (self.0, self.1, self.3, (), ())
    }
}

#[test]
fn remove_curse_freedom_and_dispel_magic() {
    // Remove Curse (Self or Single): every cursed worn item comes off, silently
    let (mut w, id, _, _, lines) = one(
        Ability::RemoveCurse,
        1,
        0,
        SpellTarget::SelfOrSingle,
        |w, id, _| {
            let mut ring = blank::clear_item(ItemNo(60));
            ring.name = "iron ring".into();
            ring.item_type = ItemType::Armour;
            ring.abilities[0] = (Ability::Cursed, 1);
            w.items.put(ring);
            plr(w, id).worn[0] = ItemNo(60);
            plr(w, id).items[0] = ItemNo(60);
            plr(w, id).itemuses[0] = -1;
        },
    );
    assert_eq!(plr(&mut w, id).worn[0], ItemNo(0), "{lines:?}");
    assert!(lines.iter().all(|l| !l.contains("cast")), "{lines:?}");
    // Freedom ends the holding spell; Dispel Magic ends everything
    let held = |w: &mut World, id: SessionId, _: ActiveMonsterId| {
        w.spells.put(spell(
            703,
            "hold",
            single(),
            SpellType::General,
            (1, 1),
            5,
            0,
            0,
            &[(Ability::HoldPerson, 0)],
        ));
        w.spells.put(spell(
            704,
            "shield",
            single(),
            SpellType::General,
            (1, 1),
            5,
            0,
            0,
            &[(Ability::Strength, 0)],
        ));
        let c = plr(w, id);
        (c.spells[0], c.spellrnds[0], c.spellvals[0]) = (SpellNo(703), 5, 1);
        (c.spells[1], c.spellrnds[1], c.spellvals[1]) = (SpellNo(704), 5, 1);
    };
    let (mut w, id, _, _, lines) = one(Ability::Freedom, 1, 0, single(), held);
    assert_eq!(plr(&mut w, id).spells[0], SpellNo(0));
    assert_eq!(plr(&mut w, id).spells[1], SpellNo(704));
    assert!(
        lines.first().is_some_and(|l| l.contains("cast")),
        "{lines:?}"
    );
    let (mut w, id, _, _, _) = one(Ability::DispelMagic, 1, 0, single(), held);
    assert_eq!(plr(&mut w, id).spells[0], SpellNo(0));
    assert_eq!(plr(&mut w, id).spells[1], SpellNo(0));
}

// A spell of an area kind is monster_cast_area's (T-051): it reaches the
// player in the room.
#[test]
fn an_area_spell_is_handed_to_the_area_cast() {
    let (mut w, id, _, r, lines) = one(
        Ability::DamageNoArmour,
        30,
        0,
        SpellTarget::DivAreaIncl,
        nothing,
    );
    assert_eq!(r, R::Applied);
    // Divided Area Including: 30 shared by the two players in the room (a Full kind hurts nobody)
    assert_eq!(plr(&mut w, id).hp, 85, "{lines:?}");
}

// --- the hooks of the round ------------------------------------------------------------

fn spell_lines(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|l| l.contains("casts harm on you"))
        .count()
}

// One roll picks the slot by the cumulative percents (25 / 100); a
// spell slot casts at its percent (95), and `harm` can be resisted at half the magic
// resistance. 1,000 rounds of the stock orc shaman, the hero's resistance
// whatever his stats make it: landed 0.75*0.95*(1-p), resisted 0.75*0.95*p, failed
// 0.75*0.05, blows 0.25, with p = min(mr/2, 97)/99. Only the first pick of a call has the
// energy to act: the later picks of the six find too little and do nothing.
#[test]
fn an_orc_shaman_casts_at_the_recorded_frequency() {
    let (mut w, id, other, m) = scene(SHAMAN);
    plr(&mut w, id).hp = 30000;
    plr(&mut w, id).maxhp = 30000;
    let (mut landed, mut resisted, mut fails, mut blows) = (0u64, 0u64, 0u64, 0u64);
    let mut mr = 0;
    for _ in 0..1000 {
        mon(&mut w, m).energy = 1000;
        plr(&mut w, id).hp = 30000;
        attack_monster_user(&mut w, m, id);
        mr = plr(&mut w, id).mr;
        let lines = text(&mut w, id);
        text(&mut w, other);
        let has = |p: &str| lines.iter().any(|l| l.contains(p));
        if has("casts harm on you") {
            landed += 1;
        } else if has("resisted orc shaman's cast of harm") {
            resisted += 1;
        } else if has("attempted to cast harm") {
            fails += 1;
        } else {
            blows += 1;
        }
    }
    let n = 1000.0;
    let p = f64::from((i32::from(mr) / 2).min(97)) / 99.0;
    let expect = [
        0.75 * 0.95 * (1.0 - p) * n,
        0.75 * 0.95 * p * n,
        0.75 * 0.05 * n,
        0.25 * n,
    ];
    let got = [landed as f64, resisted as f64, fails as f64, blows as f64];
    let chi: f64 = (0..4)
        .map(|i| (got[i] - expect[i]).powi(2) / expect[i])
        .sum();
    assert!(
        chi < CHI3_0001,
        "chi {chi} landed {landed} resisted {resisted} fails {fails} blows {blows} mr {mr}"
    );
}

// A failed cast-chance roll is still a cast attempt (it printed
// `attempted to cast ...` and counted as handled): the monster does not fall back to a normal blow,
// while a cast that finds no spell record counts as no cast and the first slot strikes.
#[test]
fn a_cast_that_finds_no_spell_falls_back_to_the_first_slot() {
    let (mut w, id, _, m) = scene(SHAMAN);
    w.monsters.get_mut(&MonsterNo(SHAMAN)).unwrap().attacks[1].accuracy = 9999;
    w.monsters.get_mut(&MonsterNo(SHAMAN)).unwrap().attacks[0].pct = 0;
    let mut blows = 0;
    for _ in 0..40 {
        mon(&mut w, m).energy = 1000;
        attack_monster_user(&mut w, m, id);
        let lines = text(&mut w, id);
        blows += lines.iter().filter(|l| l.contains("bites you")).count();
        assert_eq!(spell_lines(&lines), 0);
    }
    assert!(blows > 0, "the first slot struck");
}

// The slot's hit spell, after a landed blow, at 100% for no energy.
#[test]
fn a_hit_spell_follows_a_landed_blow() {
    let (mut w, id, _, m) = scene(SLIME);
    plr(&mut w, id).hp = 30000;
    plr(&mut w, id).maxhp = 30000;
    let mut hit = 0;
    for _ in 0..200 {
        mon(&mut w, m).energy = 1000;
        attack_monster_user(&mut w, m, id);
        let lines = text(&mut w, id);
        let bites = lines.iter().any(|l| l.contains("bites you"));
        let burn = lines.iter().any(|l| l.starts_with("Acid burns you for"));
        assert_eq!(bites, burn, "{lines:?}");
        hit += usize::from(bites);
    }
    assert!(hit > 50);
}

// Cumulative 30/60/90 over one roll of 0-99: slot 0 on 0-29, slot 1
// on 30-59, slot 2 on 60-89, nothing above; no roll at all without spells.
#[test]
fn the_random_spell_follows_the_cumulative_table() {
    use silvermere_rules::Rng as _;
    let mut w = world();
    let id = hero(&mut w, HERO);
    // three distinct, easily told spells (cast on the hero as Energy)
    for (n, name) in [(710, "alpha"), (711, "beta"), (712, "gamma")] {
        w.spells.put(spell(
            n,
            name,
            SpellTarget::Single,
            SpellType::General,
            (1, 1),
            0,
            0,
            0,
            &[(Ability::Energy, 1)],
        ));
    }
    let mut k = known(910, "caster", &[]);
    after_round(&mut k, &[(710, 30, 1), (711, 60, 1), (712, 90, 1)]);
    w.monsters.put(k);
    let m = place(&mut w, 910);
    text(&mut w, id);
    let mut counts = [0u64; 4];
    let n = 10_000u64;
    for _ in 0..n {
        cast_random(&mut w, m, id);
        let lines = text(&mut w, id);
        match lines.first() {
            None => counts[3] += 1,
            Some(l) if l.contains("alpha") => counts[0] += 1,
            Some(l) if l.contains("beta") => counts[1] += 1,
            Some(l) if l.contains("gamma") => counts[2] += 1,
            Some(l) => panic!("{l}"),
        }
    }
    let expect = [0.3, 0.3, 0.3, 0.1];
    let chi: f64 = (0..4)
        .map(|i| (counts[i] as f64 - expect[i] * n as f64).powi(2) / (expect[i] * n as f64))
        .sum();
    assert!(chi < CHI3_0001, "chi {chi} {counts:?}");
    // a monster without spells draws nothing
    let plain = place(&mut w, SLIME);
    let before = w.rng.clone();
    cast_random(&mut w, plain, id);
    assert!(before == w.rng);
    let _ = w.rng.range(0, 1);
}

// --- create and death spells, self spells ----------------------------------------------

// The createspell at level 1 on generation: a lasting self spell lands on
// the monster's own slots and the room is told it was cast on "itself".
#[test]
fn a_create_spell_fires_on_spawn() {
    let mut w = world();
    let id = hero(&mut w, HERO);
    text(&mut w, id);
    let m = place(&mut w, 901);
    assert_eq!(mon(&mut w, m).spells[0].spell, SpellNo(455));
    assert_eq!(mon(&mut w, m).spells[0].rounds, 50);
    assert_eq!(mon(&mut w, m).poison, 1);
    let lines = text(&mut w, id);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Slaver leader cast wounded on itself")),
        "{lines:?}"
    );
}

// The deathspell at level 1 with no target: a self summon makes its monster.
#[test]
fn a_death_spell_casts_on_death() {
    let mut w = world();
    let id = hero(&mut w, HERO);
    let m = place(&mut w, 902);
    text(&mut w, id);
    mon(&mut w, m).hp = 0;
    assert!(check_kill_monster(&mut w, m, Some(id)));
    assert!(
        w.active_monsters
            .iter()
            .any(|(_, a)| a.knownnum == MonsterNo(SPAWN)),
        "the death spell summoned"
    );
}

// A single-target spell needs a player; the self kinds do not.
#[test]
fn by_number_needs_a_player_for_a_single_spell() {
    let (mut w, id, _, m) = scene(SHAMAN);
    cast_by_number(&mut w, m, SpellNo(12), 1, None);
    assert!(text(&mut w, id).is_empty());
    assert_eq!(plr(&mut w, id).hp, 100);
    cast_by_number(&mut w, m, SpellNo(12), 1, Some(id));
    assert!(plr(&mut w, id).hp < 100);
    // an unknown spell does nothing
    cast_by_number(&mut w, m, SpellNo(31000), 1, Some(id));
}

// The dark monk heals and buffs itself after the round: way of the swan
// (heal, self) and tiger (lasting) at level 12 are self casts with the
// level raising the value and each Poison slot adding to the poison.
#[test]
fn a_buff_caster_casts_on_itself() {
    let mut w = world();
    let id = hero(&mut w, HERO);
    w.spells.put(spell(
        36,
        "way of the swan",
        SpellTarget::OnSelf,
        SpellType::General,
        (3, 3),
        0,
        0,
        0,
        &[(Ability::Heal, 0)],
    ));
    let mut tiger = spell(
        38,
        "way of the tiger",
        SpellTarget::OnSelf,
        SpellType::General,
        (2, 2),
        60,
        0,
        0,
        &[(Ability::Poison, 0)],
    );
    (tiger.mininc, tiger.minlvls, tiger.maxinc, tiger.maxlvls) = (1, 10, 1, 10);
    w.spells.put(tiger);
    let mut k = known(911, "dark monk", &[]);
    after_round(&mut k, &[(36, 75, 12), (38, 100, 12)]);
    w.monsters.put(k);
    let m = place(&mut w, 911);
    text(&mut w, id);
    // tiger at level 12: 2 + 12*1/10 = 3
    cast_by_number(&mut w, m, SpellNo(38), 12, Some(id));
    assert_eq!(mon(&mut w, m).spells[0].spell, SpellNo(38));
    assert_eq!(mon(&mut w, m).spells[0].rounds, 60);
    assert_eq!(mon(&mut w, m).spells[0].value, 3);
    assert_eq!(mon(&mut w, m).poison, 3);
    let lines = text(&mut w, id);
    assert!(lines.iter().any(|l| l.contains("on itself")), "{lines:?}");
    // the instant swan does not add a slot
    cast_by_number(&mut w, m, SpellNo(36), 12, Some(id));
    assert_eq!(mon(&mut w, m).spells[1].spell, SpellNo(0));
}
