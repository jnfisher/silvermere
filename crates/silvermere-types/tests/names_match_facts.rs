//! Every name table equals the committed facts, entry by entry.

use crate::facts;

use silvermere_types::enums::{Currency, Direction, ItemType, RoomType, WornLocation};
use silvermere_types::names::*;

/// (table name in the facts, Rust static, words in the table not counting the end marker)
const TABLES: &[(&str, &[&str], usize)] = &[
    ("DIRECTION_NAMES", DIRECTION_NAMES, 10),
    ("DIRECTION_NAMES_2", DIRECTION_NAMES_2, 10),
    ("DIRECTION_NAMES_3", DIRECTION_NAMES_3, 10),
    ("DIRECTION_ABBREVIATIONS", DIRECTION_ABBREVIATIONS, 10),
    ("EXIT_TYPES", EXIT_TYPES, 25),
    ("ROOM_TYPES", ROOM_TYPES, 8),
    ("SHOP_TYPES", SHOP_TYPES, 13),
    ("MONSTER_TYPES", MONSTER_TYPES, 39),
    ("PRONOUNS", PRONOUNS, 3),
    ("FOLLOW_TYPES", FOLLOW_TYPES, 4),
    ("ALIGNMENTS", ALIGNMENTS, 7),
    ("ITEM_TYPES", ITEM_TYPES, 12),
    ("MATERIALS", MATERIALS, 7),
    ("MAGERY_TYPES", MAGERY_TYPES, 6),
    ("WEAPON_TYPES", WEAPON_TYPES, 4),
    ("ARMOUR_TYPES", ARMOUR_TYPES, 10),
    ("WEAPON_CLASSES", WEAPON_CLASSES, 10),
    ("WORN_LOCATIONS", WORN_LOCATIONS, 20),
    ("COIN_DESCRIPTIONS", COIN_DESCRIPTIONS, 5),
    ("CURRENCY_NAMES", CURRENCY_NAMES, 5),
    ("CURRENCY_NAMES_SINGULAR", CURRENCY_NAMES_SINGULAR, 6),
    ("OWNER_TYPES", OWNER_TYPES, 4),
    ("SPELL_TYPES", SPELL_TYPES, 4),
    ("RESIST_TYPES", RESIST_TYPES, 7),
    ("SPELL_TARGETS", SPELL_TARGETS, 14),
    ("HAIR_COLOURS", HAIR_COLOURS, 10),
    ("HAIR_LENGTHS", HAIR_LENGTHS, 6),
    ("EYE_COLOURS", EYE_COLOURS, 17),
    ("STAT_NAMES", STAT_NAMES, 7),
    ("GANG_HOUSES", GANG_HOUSES, 11),
    ("ABILITY_NAMES", ABILITY_NAMES, 188),
];

#[test]
fn table_counts() {
    for &(name, table, count) in TABLES {
        assert_eq!(table.len(), count, "{name}");
    }
    // The counts the card names, spelled out.
    assert_eq!(EXIT_TYPES.len(), 25);
    assert_eq!(ROOM_TYPES.len(), 8);
    assert_eq!(SHOP_TYPES.len(), 13);
    assert_eq!(MONSTER_TYPES.len(), 39);
    assert_eq!(ALIGNMENTS.len(), 7);
    assert_eq!(WORN_LOCATIONS.len(), 20);
    assert_eq!(SPELL_TARGETS.len(), 14);
    // ABILITY_NAMES is declared with 189 slots: 188 names (ABL_NONE..ABL_MEDITATE) and the end marker.
    assert_eq!(ABILITY_NAMES.len() + 1, 189);
    assert_eq!(CURRENCY_NAMES.len(), 5);
    assert_eq!(HAIR_COLOURS.len(), 10);
    assert_eq!(EYE_COLOURS.len(), 17);
}

// The table entries themselves.
#[test]
fn tables_match_the_facts() {
    let facts = facts::facts();
    let consts = facts.consts();
    assert_eq!(facts.string_tables.len(), TABLES.len());
    for (fact, &(name, table, count)) in facts.string_tables.iter().zip(TABLES) {
        assert_eq!(fact.name, name);
        let (size, entries) = (&fact.size, &fact.entries);
        let declared = size
            .parse::<usize>()
            .ok()
            .or_else(|| consts.get(size.as_str()).map(|&v| v as usize))
            .unwrap_or_else(|| panic!("{name}: unknown size {size}"));
        assert_eq!(
            declared,
            entries.len(),
            "{name}[{size}] has {} entries",
            entries.len()
        );
        // The end marker only ever ends a table.
        let words: Vec<&str> = match entries.split_last() {
            Some((None, rest)) => rest
                .iter()
                .map(|e| e.as_deref().expect("end marker inside a table"))
                .collect(),
            _ => entries
                .iter()
                .map(|e| e.as_deref().expect("end marker inside a table"))
                .collect(),
        };
        assert_eq!(words.len(), count, "{name} word count");
        for (i, (fact, rust)) in words.iter().zip(table).enumerate() {
            assert_eq!(fact, rust, "{name}[{i}]");
        }
    }
}

// The reverse direction table
#[test]
fn reverse_directions_match_the_facts() {
    for (d, back) in Direction::COMPASS.iter().zip(REVERSE_DIRECTIONS) {
        assert_eq!(
            back.reverse(),
            Some(*d),
            "reversing twice gives the direction back"
        );
    }
    let facts = facts::facts();
    let consts = facts.consts();
    let idents = &facts.reverse_directions;
    assert_eq!(idents.len(), REVERSE_DIRECTIONS.len());
    for (i, (ident, rust)) in idents.iter().zip(REVERSE_DIRECTIONS).enumerate() {
        assert_eq!(
            consts[ident.as_str()],
            i64::from(rust.code()),
            "REVERSE_DIRECTIONS[{i}]"
        );
    }
}

// ITEM_TYPES: a stray "Shield" at 2 shifts every later name (sic).
#[test]
fn item_type_names_keep_the_shift() {
    assert_eq!(ItemType::Armour.display_name(), Some("Armour"));
    assert_eq!(ItemType::Weapon.display_name(), Some("Weapon"));
    assert_eq!(ItemType::Projectile.display_name(), Some("Shield"));
    assert_eq!(ItemType::Special.display_name(), Some("Projectile"));
    assert_eq!(ItemType::Food.display_name(), Some("Special"));
    assert_eq!(ItemType::General.display_name(), Some("Scroll"));
    assert_eq!(ItemType::Unknown(11).display_name(), Some("General"));
    assert_eq!(ItemType::Unknown(12).display_name(), None);
}

// ROOM_TYPES, WORN_LOCATIONS, CURRENCY_NAMES, EXIT_TYPES
#[test]
fn original_spellings_and_duplicates() {
    assert_eq!(RoomType::Colliseum.name(), Some("Colliseum"));
    assert_eq!(WornLocation::Finger.name(), WornLocation::Finger2.name());
    assert_eq!(WornLocation::Worn.name(), WornLocation::Worn2.name());
    assert_eq!(WornLocation::Wrist.name(), WornLocation::Wrist2.name());
    assert_eq!(
        Currency::Runic.plural_name(),
        Some("User Defined Currency Type ....")
    );
    assert_eq!(CURRENCY_NAMES[4].len(), 31);
    assert_eq!(CURRENCY_NAMES[4], CURRENCY_NAMES_SINGULAR[4]);
    assert_eq!(Currency::Undefined.singular_name(), Some("Undefined"));
    assert_eq!(EXIT_TYPES[8], "Map Change");
    assert_eq!(ABILITY_NAMES[0], "No ability");
    assert_eq!(ABILITY_NAMES[1], "Inflict Damage Ignoring Armour");
    assert_eq!(ABILITY_NAMES[2], "Alter Defence Value");
    assert_eq!(DIRECTION_NAMES[0], "n \u{8}orth");
    assert_eq!(DIRECTION_NAMES_3[8], "up  \u{8}");
}
