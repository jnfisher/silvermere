//! The game's fixed name tables, word for word.
//!
//! Each static holds the table's strings in order, without the end marker most tables carry;
//! the original's spellings are kept ("Colliseum", "Lightening", "chesnut-brown", the shifted
//! [`ITEM_TYPES`]). `tests/names_match_facts.rs` compares every entry with the committed facts.
//! The tables the original rewrites at run time (the direction letter slots, the fifth
//! currency name) hold their initial text here; the engine owns the run-time copies.

use crate::enums::Direction;

/// The opposite of each direction, by [`Direction`].
pub static REVERSE_DIRECTIONS: [Direction; 10] = [
    Direction::South,
    Direction::North,
    Direction::West,
    Direction::East,
    Direction::SouthWest,
    Direction::SouthEast,
    Direction::NorthWest,
    Direction::NorthEast,
    Direction::Down,
    Direction::Up,
];

/// Direction names with the letter slot at 1: a space then a backspace, which the slow cycle fills with a random letter.
pub static DIRECTION_NAMES: &[&str] = &[
    "n \u{8}orth",
    "s \u{8}outh",
    "e \u{8}ast",
    "w \u{8}est",
    "n \u{8}ortheast",
    "n \u{8}orthwest",
    "s \u{8}outheast",
    "s \u{8}outhwest",
    "u \u{8}p",
    "d \u{8}own",
];

/// The same, letter slot at 2.
pub static DIRECTION_NAMES_2: &[&str] = &[
    "no \u{8}rth",
    "so \u{8}uth",
    "ea \u{8}st",
    "we \u{8}st",
    "no \u{8}rtheast",
    "no \u{8}rthwest",
    "so \u{8}utheast",
    "so \u{8}uthwest",
    "up \u{8}",
    "do \u{8}wn",
];

/// The same, letter slot at 3; the `DIR_UP` entry keeps its double space.
pub static DIRECTION_NAMES_3: &[&str] = &[
    "nor \u{8}th",
    "sou \u{8}th",
    "eas \u{8}t",
    "wes \u{8}t",
    "nor \u{8}theast",
    "nor \u{8}thwest",
    "sou \u{8}theast",
    "sou \u{8}thwest",
    "up  \u{8}",
    "dow \u{8}n",
];

/// Short direction commands.
pub static DIRECTION_ABBREVIATIONS: &[&str] =
    &["N", "S", "E", "W", "NE", "NW", "SE", "SW", "U", "D"];

/// Exit type names by [`ExitType`](crate::enums::ExitType) (26 slots: 25 words and the end marker).
pub static EXIT_TYPES: &[&str] = &[
    "Normal",
    "Spell",
    "Key",
    "Item",
    "Toll",
    "Action",
    "Hidden",
    "Door",
    "Map Change",
    "Trap",
    "Text",
    "Gate",
    "Remote Action",
    "Class",
    "Race",
    "Level",
    "Timed",
    "Ticket",
    "User Count",
    "Block Guard",
    "Alignment",
    "Delay",
    "Cast",
    "Ability",
    "Spell Trap",
];

/// Room type names by [`RoomType`](crate::enums::RoomType); "Colliseum" (sic).
pub static ROOM_TYPES: &[&str] = &[
    "Normal",
    "Shop",
    "Arena",
    "Lair",
    "Hotel",
    "Colliseum",
    "Jail",
    "Library",
];

/// Shop type names by [`ShopType`](crate::enums::ShopType).
pub static SHOP_TYPES: &[&str] = &[
    "General",
    "Weapons",
    "Armour",
    "Items",
    "Spells",
    "Hospital",
    "Tavern",
    "Bank",
    "Training",
    "Inn",
    "Specific",
    "Gang Shop",
    "Deed Shop",
];

/// Monster type names by [`MonsterType`](crate::enums::MonsterType).
pub static MONSTER_TYPES: &[&str] = &[
    "Lair", "Wanderer", "NPC", "Living", "Random", "Guard", "Group 1", "Group 2", "Group 3",
    "Group 4", "Group 5", "Group 6", "Group 7", "Group 8", "Group 9", "Group 10", "Group 11",
    "Group 12", "Group 13", "Group 14", "Group 15", "Group 16", "Group 17", "Group 18", "Group 19",
    "Group 20", "Group 21", "Group 22", "Group 23", "Group 24", "Group 25", "Group 26", "Group 27",
    "Group 28", "Group 29", "Group 30", "Arena", "Angel", "Quest",
];

/// Pronouns by [`Gender`](crate::enums::Gender), capitalised.
pub static PRONOUNS: &[&str] = &["It", "He", "She"];

/// Monster group roles by [`GroupRole`](crate::enums::GroupRole).
pub static FOLLOW_TYPES: &[&str] = &["Solo", "Leader", "Follower", "Stationary"];

/// Alignment names by [`Alignment`](crate::enums::Alignment).
pub static ALIGNMENTS: &[&str] = &[
    "Good",
    "Evil",
    "Chaotic Evil",
    "Neutral",
    "Lawful Good",
    "Neutral Evil",
    "Lawful Evil",
];

/// Item type names, shifted (sic).
///
/// The table has a stray "Shield" at index 2, so from 2 up each name is printed for the wrong
/// type: a projectile (`ITEM_PROJECTILE` 2) shows as "Shield", food as "Special", and so on.
/// The display code indexes this table by the raw item type, so keep the shift;
/// [`ItemType::display_name`](crate::enums::ItemType::display_name) does that lookup.
pub static ITEM_TYPES: &[&str] = &[
    "Armour",
    "Weapon",
    "Shield",
    "Projectile",
    "Special",
    "Food",
    "Drink",
    "Light",
    "Key",
    "Container",
    "Scroll",
    "General",
];

/// Material names by [`Material`](crate::enums::Material); never shown in play.
pub static MATERIALS: &[&str] = &[
    "Iron", "Steel", "Bronze", "Silver", "Gold", "Platinum", "Cotton",
];

/// Magic school names by [`Magery`](crate::enums::Magery).
pub static MAGERY_TYPES: &[&str] = &["None", "Mage", "Priest", "Druid", "Bard", "Kai"];

/// Weapon type names by [`WeaponType`](crate::enums::WeaponType).
pub static WEAPON_TYPES: &[&str] = &[
    "1-Handed Blunt",
    "2-Handed Blunt",
    "1-Handed Sharp",
    "2-Handed Sharp",
];

/// Armour type names by [`ArmourType`](crate::enums::ArmourType).
pub static ARMOUR_TYPES: &[&str] = &[
    "Natural",
    "Robes",
    "Padded",
    "Soft Leather",
    "Soft Studded Leather",
    "Rigid Leather",
    "Studded Rigid Leather",
    "Chainmail",
    "Scalemail",
    "Platemail",
];

/// Weapons a class may use, by [`WeaponClass`](crate::enums::WeaponClass).
pub static WEAPON_CLASSES: &[&str] = &[
    "1-handed Blunt",
    "2-handed Blunt",
    "1-handed Sharp",
    "2-handed Sharp",
    "Any 1-handed",
    "Any 2-handed",
    "Any Sharp",
    "Any Blunt",
    "All Weapons",
    "Limited",
];

/// Wear locations by [`WornLocation`](crate::enums::WornLocation); "Finger", "Worn" and "Wrist" appear twice.
pub static WORN_LOCATIONS: &[&str] = &[
    "Nowhere", "Worn", "Head", "Hands", "Finger", "Feet", "Arms", "Back", "Neck", "Legs", "Waist",
    "Torso", "Off-Hand", "Finger", "Wrist", "Ears", "Worn", "Wrist", "Eyes", "Face",
];

/// Coin descriptions by [`Currency`](crate::enums::Currency); the fifth takes the currency name for `%s`.
pub static COIN_DESCRIPTIONS: &[&str] = &[
    "The copper farthings look like they've been around forever.",
    "The silver nobles glitter with use.",
    "The gold crowns are rustic and used.",
    "The platinum pieces shine as though they were new.",
    "The %s glitter like nothing you have ever seen before.",
];

/// Currency names, plural, by [`Currency`](crate::enums::Currency).
///
/// The fifth entry is the 31-character placeholder the reload replaces with the sysop's
/// `USRCURNM` option; the config supplies the real name at run time.
pub static CURRENCY_NAMES: &[&str] = &[
    "copper farthings",
    "silver nobles",
    "gold crowns",
    "platinum pieces",
    "User Defined Currency Type ....",
];

/// Currency names, singular, by [`Currency`](crate::enums::Currency), with "Undefined" for `CUR_UNDEFINED`.
pub static CURRENCY_NAMES_SINGULAR: &[&str] = &[
    "copper farthing",
    "silver noble",
    "gold crown",
    "platinum piece",
    "User Defined Currency Type ....",
    "Undefined",
];

/// Item owner kinds by [`OwnerKind`](crate::enums::OwnerKind); never shown in play.
pub static OWNER_TYPES: &[&str] = &["Room", "Character", "Monster", "Unknown"];

/// Spell type names by [`SpellType`](crate::enums::SpellType); "Change me" is the original's placeholder.
pub static SPELL_TYPES: &[&str] = &["Auto-Combat", "Poison", "Change me", "General"];

/// Element names by [`Element`](crate::enums::Element); "Lightening" (sic).
pub static RESIST_TYPES: &[&str] = &[
    "Cold",
    "Hot",
    "Stone",
    "Lightening",
    "Normal",
    "Water",
    "Poison",
];

/// Spell target names by [`SpellTarget`](crate::enums::SpellTarget).
pub static SPELL_TARGETS: &[&str] = &[
    "Single",
    "Self",
    "Self or Single",
    "Divided Area Not You",
    "Monster",
    "Divided Area Incl. You",
    "Any",
    "Item",
    "Monster or Single",
    "Divided Attack Area",
    "Divided Party Area",
    "Full Area",
    "Full Attack Area",
    "Full Party Area",
];

/// Hair colours, indexed by `player.haircol`.
pub static HAIR_COLOURS: &[&str] = &[
    "black",
    "white",
    "silver",
    "red",
    "brown",
    "dark-brown",
    "blonde",
    "green",
    "blue",
    "grey",
];

/// Hair lengths, indexed by `player.hairlen`.
pub static HAIR_LENGTHS: &[&str] = &[
    "none",
    "short",
    "shoulder-length",
    "long",
    "waist-length",
    "ankle-length",
];

/// Eye colours, indexed by `player.eyecol`; "chesnut-brown" (sic).
pub static EYE_COLOURS: &[&str] = &[
    "black",
    "crimson",
    "yellow",
    "pale-blue",
    "sea-blue",
    "dark-blue",
    "grey-blue",
    "slate-grey",
    "bright-green",
    "forest-green",
    "pale-green",
    "chesnut-brown",
    "dark-brown",
    "hazel",
    "violet",
    "lavender",
    "golden",
];

/// Statistic names by [`ResistStat`](crate::enums::ResistStat).
pub static STAT_NAMES: &[&str] = &[
    "Magic Resistance",
    "Strength",
    "Agility",
    "Health",
    "Intellect",
    "Wisdom",
    "Charm",
];

/// Gang house names by [`GangHouse`](crate::enums::GangHouse).
pub static GANG_HOUSES: &[&str] = &[
    "None",
    "Red Gang House",
    "Orange Gang House",
    "Yellow Gang House",
    "Green Gang House",
    "Violet Gang House",
    "Blue Gang House",
    "Black Gang House",
    "Silver Gang House",
    "Gold Gang House",
    "White Gang House",
];

/// Ability names by [`Ability`](crate::enums::Ability) (189 slots: 188 names and the end marker). Index 0 is "No ability".
pub static ABILITY_NAMES: &[&str] = &[
    "No ability",
    "Inflict Damage Ignoring Armour",
    "Alter Defence Value",
    "Alter Cold Attack Damage (Defence)",
    "Alter Damage (Attacking)",
    "Alter Hot Attack Damage (Defence)",
    "Enslave Creature",
    "Alter Damage Resistance Value",
    "Steal Hit Points",
    "Shadow Stealth",
    "Protective Shield",
    "Alter Energy Level",
    "Summon Creature",
    "Alter User Light",
    "Alter Room Light",
    "Alter Hunger",
    "Alter Thirst",
    "Inflict Damage on Armour",
    "Heal",
    "Poison",
    "Cure Poison",
    "Poison Immunity",
    "Alter AV (Bless)",
    "Kill Dead",
    "Alter defence against Evil",
    "Alter defence against Good",
    "Detect Magic",
    "Stealth",
    "Magical",
    "Punch",
    "Kick",
    "Bash",
    "Smash",
    "Killblow",
    "Dodge",
    "Jumpkick",
    "Magic Resistance",
    "Picklocks",
    "Tracking",
    "Thievery",
    "Find Traps",
    "Disarm Traps",
    "Link to Spell",
    "One Time Cast",
    "Alter Intellect",
    "Alter Wisdom",
    "Alter Strength",
    "Alter Health",
    "Alter Agility",
    "Alter Charm",
    "Quest",
    "Anti Magic",
    "Evil in Combat",
    "Blinding Light",
    "Alter General Light",
    "Alter General Light Duration",
    "Recharge Item",
    "See Hidden",
    "Alter Critical Hit Chance",
    "Class Item Inclusion",
    "Flee",
    "Affect Exit",
    "Alter Evil Chance",
    "Alter Experience",
    "Add CP",
    "Resist Stone",
    "Resist Lightning",
    "Quickness",
    "Slowness",
    "Mana",
    "Spellcasting",
    "Confusion",
    "Damaging shield",
    "Dispell magic",
    "Hold person",
    "Paralyze",
    "Mute",
    "Perception",
    "Animal",
    "Magebind",
    "Affects Animal",
    "Freedom",
    "Cursed",
    "Major Curse",
    "Remove Curse",
    "Shatter",
    "Quality",
    "Alter Energy Usage",
    "Alter HP",
    "Alter Punch AV",
    "Alter Kick AV",
    "Alter Jumpkick AV",
    "Alter Punch DMG",
    "Alter Kick DMG",
    "Alter Jumpkick DMG",
    "Slay",
    "Alter Encumbrance",
    "Good Aligned",
    "Evil Aligned",
    "Alter DR by percent",
    "Loyal Item",
    "Confuse Message",
    "Race Stealth",
    "Class Stealth",
    "Defence Modifier",
    "Alter AV (Curse)",
    "3rd Alter Attack Value",
    "Blind User",
    "Affects Living",
    "Non Living",
    "Not Good Aligned",
    "Not Evil Aligned",
    "Neutral Aligned",
    "Not Neutral Aligned",
    "Auto Use Abilities",
    "Descriptive Message",
    "Alter Backstab Attack Value",
    "Alter Backstab Min. Damage",
    "Alter Backstab Max. Damage",
    "Delete at cleanup",
    "Starting Message",
    "Recharge at cleanup",
    "Remove spell",
    "Alter healing rate",
    "Negate ability",
    "Quest type one",
    "Quest type two",
    "Quest type three",
    "Quest type four",
    "Quest type five",
    "Quest type six",
    "Quest type seven",
    "Quest type eight",
    "Quest type nine",
    "Quest type ten",
    "Minimum level allowed",
    "Maximum level allowed",
    "Shockshield Message",
    "Visible Placed Item",
    "Spell Immunity",
    "Teleport Room",
    "Teleport Map",
    "Hit Magical",
    "Clear Item",
    "Non Magical Spell",
    "Alter Mana Regeneration",
    "Guarded by Monster",
    "Resist Water",
    "Trigger Text Block",
    "Delete from inventory at cleanup",
    "Heal Mana",
    "Cast on ending",
    "Rune",
    "Terminate Spell",
    "Remain Visible at cleanup",
    "Death text block",
    "Quest item",
    "Scatter items",
    "Required to hit",
    "Kai Bind",
    "Give Spell (temporarily)",
    "Open Door",
    "Lore",
    "Spell Component",
    "Cast on ending chance",
    "Alter spell damage",
    "Alter spell duration",
    "Unequip item",
    "Equip item",
    "Cannot wear location",
    "Put to sleep",
    "Invisibility",
    "See Invisible",
    "Scry",
    "Steal Mana",
    "Steal HP to Mana",
    "Steal Mana to HP",
    "Spell Colours",
    "Shadowform",
    "Find traps value",
    "Picklocks value",
    "Gang House Deed",
    "Gang House Tax",
    "Gang House Item",
    "Gang Shop Item (controller)",
    "Do not Attack if Item Nmbr",
    "Perfect Stealth",
    "Meditate",
];
