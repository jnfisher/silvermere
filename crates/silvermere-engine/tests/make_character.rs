//! `player::make_character`: a finished character made outside the game for
//! `silvermere character create`. Its level, stats, hit points and equipment must survive
//! the entry's own checks (`load_player`), and the world must be left as it was.

use crate::common;
use silvermere_engine::player::load_player;
use silvermere_engine::player::make::{CharacterSpec, MakeError, make_character};
use silvermere_engine::world::blank;
use silvermere_engine::{SessionId, World};
use silvermere_rules::progression::new_calc_exp_needed;
use silvermere_types::enums::{
    ArmourType, ItemType, MessageLevel, WeaponClass, WeaponType, WornLocation,
};
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, RaceNo};
use silvermere_types::records::Item;

const PLATE: i32 = 10;
const HELM: i32 = 11;
const GREATSWORD: i32 = 12;
const RING: i32 = 13;
const RING2: i32 = 14;
const RELIC: i32 = 15;
const SHIELD: i32 = 16;
const ROBES: i32 = 17;

fn item(n: i32, name: &str, ty: ItemType) -> Item {
    let mut i = blank::clear_item(ItemNo(n));
    i.name = name.to_string();
    i.item_type = ty;
    i.weight = 10;
    i.gettable = 1;
    i.charges = -1;
    i
}

fn armour(n: i32, name: &str, worn: WornLocation, arm: ArmourType) -> Item {
    let mut i = item(n, name, ItemType::Armour);
    i.worn = worn;
    i.armtype = arm;
    i.ac = 5;
    i
}

/// The tiny world with a Human-like race 1 (stats 40 to 100), a Warrior-like class 1
/// (6 hit points a level, plate, every weapon), class 2 (robes only), and some gear.
fn world() -> World {
    // (not `common::tiny_world`: no stock text is needed, so these run anywhere)
    let mut w = World::load(common::test_config(), 8, common::tiny_snapshot()).unwrap();
    let r = w.races.get_mut(&RaceNo(1)).unwrap();
    r.name = "Human".into();
    (r.minstr, r.minint, r.minwil, r.minagi, r.minhea, r.mincha) = (40, 40, 40, 40, 40, 40);
    (r.maxstr, r.maxint, r.maxwil, r.maxagi, r.maxhea, r.maxcha) = (100, 100, 100, 100, 100, 100);
    r.basecp = 100;
    let c = w.classes.get_mut(&ClassNo(1)).unwrap();
    c.name = "Warrior".into();
    c.hplevel = 6;
    c.exp = 10;
    c.weapon = WeaponClass::All;
    c.armour = ArmourType::Platemail;
    c.combat = 3;
    let mut mage = blank::clear_class(ClassNo(2));
    mage.name = "Mage".into();
    mage.hplevel = 2;
    mage.weapon = WeaponClass::All;
    mage.armour = ArmourType::Robes;
    w.classes.insert(ClassNo(2), mage);
    let mut sword = item(GREATSWORD, "greatsword", ItemType::Weapon);
    sword.wpntype = WeaponType::TwoHandSharp;
    sword.mindmg = 10;
    sword.maxdmg = 30;
    let mut relic = item(RELIC, "relic", ItemType::General);
    relic.limit = 1;
    for i in [
        armour(
            PLATE,
            "platemail",
            WornLocation::Torso,
            ArmourType::Platemail,
        ),
        armour(
            HELM,
            "plate helm",
            WornLocation::Head,
            ArmourType::Platemail,
        ),
        armour(RING, "gold ring", WornLocation::Finger, ArmourType::Robes),
        armour(
            RING2,
            "silver ring",
            WornLocation::Finger,
            ArmourType::Robes,
        ),
        armour(
            SHIELD,
            "tower shield",
            WornLocation::OffHand,
            ArmourType::Platemail,
        ),
        armour(ROBES, "robes", WornLocation::Torso, ArmourType::Robes),
        sword,
        relic,
    ] {
        w.items.insert(i.number, i);
    }
    w
}

fn spec(level: u16, items: &[i32]) -> CharacterSpec {
    CharacterSpec {
        firstname: "tester".into(),
        lastname: "Operator".into(),
        race: RaceNo(1),
        class: ClassNo(1),
        sex: b'M',
        level,
        stats: [None; 6],
        items: items.iter().map(|&n| ItemNo(n)).collect(),
        gold: 1000,
    }
}

fn make(w: &mut World, s: &CharacterSpec) -> Result<silvermere_engine::player::Made, MakeError> {
    make_character(w, AccountId(1), "tester", s, 0)
}

#[test]
fn a_level_70_warrior_with_maximum_stats_and_gear_on() {
    let mut w = world();
    let made = make(&mut w, &spec(70, &[PLATE, HELM, GREATSWORD, RING, RING2])).unwrap();
    let c = &made.character;
    assert_eq!(
        (c.firstname.as_str(), c.lastname.as_str()),
        ("Tester", "Operator")
    );
    assert_eq!(c.level, 70);
    let need = new_calc_exp_needed(69, 10);
    assert_eq!((c.expbil, c.exprem), (need.expbil, need.exprem));
    assert_eq!([c.str, c.intel, c.wil, c.agi, c.hea, c.cha], [100; 6]);
    assert_eq!(c.basstr, 100);
    assert_eq!(c.cp, 0);
    // 6 a level for 70 levels, kept to the stored byte
    assert_eq!(c.randomhp, 255);
    assert!(c.maxhp > 255, "maxhp {}", c.maxhp);
    assert_eq!(c.hp, c.maxhp);
    assert_eq!(c.coins.gold, 1000);
    assert_eq!(c.weapon, ItemNo(GREATSWORD));
    for n in [PLATE, HELM, RING, RING2] {
        assert!(c.worn.contains(&ItemNo(n)), "{n} worn: {:?}", c.worn);
    }
    assert_eq!(
        made.items,
        [
            "platemail: worn",
            "plate helm: worn",
            "greatsword: wielded",
            "gold ring: worn",
            "silver ring: worn"
        ]
    );
    // the session used for the gear is gone
    assert!(w.sessions.is_empty());
}

#[test]
fn the_entry_keeps_what_was_made() {
    let mut w = world();
    let made = make(&mut w, &spec(70, &[PLATE, GREATSWORD])).unwrap();
    let id: SessionId = common::enter(&mut w, "x", common::special_rooms()[0], MessageLevel::High);
    load_player(&mut w, id, made.character.clone(), 0).unwrap();
    let c = w.sessions[&id].player().unwrap();
    assert_eq!(c.level, 70, "the experience pays for the level");
    assert!(c.cp >= 0, "no retraining");
    assert_eq!([c.str, c.intel, c.wil, c.agi, c.hea, c.cha], [100; 6]);
    assert_eq!(c.weapon, ItemNo(GREATSWORD));
    assert!(c.worn.contains(&ItemNo(PLATE)));
    assert_eq!(c.maxhp, made.character.maxhp);
}

#[test]
fn what_the_class_may_not_wear_is_carried() {
    let mut w = world();
    let mut s = spec(10, &[PLATE, ROBES, GREATSWORD, SHIELD]);
    s.class = ClassNo(2);
    let made = make(&mut w, &s).unwrap();
    let c = &made.character;
    assert!(!c.worn.contains(&ItemNo(PLATE)));
    assert!(c.worn.contains(&ItemNo(ROBES)));
    assert!(c.items.contains(&ItemNo(PLATE)), "still in the pack");
    assert!(
        made.items[0].starts_with("platemail: carried, not worn"),
        "{:?}",
        made.items
    );
    // a shield does not go with a two-handed weapon
    assert!(!c.worn.contains(&ItemNo(SHIELD)));
    assert!(
        made.items[3].starts_with("tower shield: carried"),
        "{:?}",
        made.items
    );
}

#[test]
fn stats_given_and_refused() {
    let mut w = world();
    let mut s = spec(5, &[]);
    s.stats = [Some(55), None, None, Some(60), None, None];
    let c = make(&mut w, &s).unwrap().character;
    assert_eq!([c.str, c.intel, c.agi], [55, 100, 60]);
    s.stats[0] = Some(101);
    assert_eq!(
        make(&mut w, &s).unwrap_err(),
        MakeError::Stat {
            stat: "strength",
            got: 101,
            min: 40,
            max: 100
        }
    );
}

#[test]
fn refusals() {
    let mut w = world();
    let bad = |f: &dyn Fn(&mut CharacterSpec)| {
        let mut s = spec(10, &[]);
        f(&mut s);
        make(&mut world(), &s).unwrap_err()
    };
    assert!(matches!(
        bad(&|s| s.firstname = "Two words".into()),
        MakeError::FirstName(_)
    ));
    assert!(matches!(
        bad(&|s| s.firstname = "Abcdefghijk".into()),
        MakeError::FirstName(_)
    ));
    assert!(matches!(
        bad(&|s| s.firstname = "look".into()),
        MakeError::NameIsCommand(_)
    ));
    assert!(matches!(
        bad(&|s| s.lastname = "O'Neil".into()),
        MakeError::LastName(_)
    ));
    assert_eq!(bad(&|s| s.race = RaceNo(9)), MakeError::Race(9));
    assert_eq!(bad(&|s| s.class = ClassNo(9)), MakeError::Class(9));
    assert!(matches!(bad(&|s| s.level = 0), MakeError::Level { .. }));
    assert_eq!(bad(&|s| s.items = vec![ItemNo(99)]), MakeError::Item(99));
    assert_eq!(
        bad(&|s| s.items = vec![ItemNo(RELIC)]),
        MakeError::Limited("relic".into())
    );
    // coins weigh: more than the pack has left is refused
    assert!(matches!(
        bad(&|s| s.gold = u32::MAX),
        MakeError::Gold { gold: u32::MAX, .. }
    ));
    // a refusal changes nothing
    let _ = make(&mut w, &spec(10, &[RELIC]));
    assert!(w.sessions.is_empty());
}
