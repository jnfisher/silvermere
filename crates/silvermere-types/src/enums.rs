//! Code enums: one enum per family of stored codes, discriminant equal to the stored value.
//!
//! Every enum carries an `Unknown(u16)` variant so a record holding a code with no known meaning
//! decodes without loss and without panicking. Because of that variant the enums are not
//! fieldless, so the value is read with `code()` rather than an `as` cast. `code_name()` gives the
//! constant's name (`OWN_ROOM`); `tests/enum_codes.rs` checks every value against the committed
//! facts.

use serde::Serialize;

/// A code that does not fit the 16-bit field it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("code {0} is outside the 16-bit range")]
pub struct CodeOutOfRange(pub i32);

macro_rules! c_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $var:ident = $val:literal => $cname:literal, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        #[repr(u16)]
        pub enum $name {
            $( $(#[$vmeta])* $var = $val, )*
            /// A code with no known meaning; kept so records round-trip.
            Unknown(u16),
        }

        impl $name {
            /// Every defined value, in code order.
            pub const ALL: &'static [Self] = &[$(Self::$var),*];

            /// The numeric code the original stores.
            pub const fn code(self) -> u16 {
                match self {
                    $( Self::$var => $val, )*
                    Self::Unknown(code) => code,
                }
            }

            /// Decodes a stored code; unknown codes become `Unknown(code)`.
            pub const fn from_code(code: u16) -> Self {
                match code {
                    $( $val => Self::$var, )*
                    other => Self::Unknown(other),
                }
            }

            /// The constant's name (`OWN_ROOM`), or `None` for `Unknown`.
            pub const fn code_name(self) -> Option<&'static str> {
                match self {
                    $( Self::$var => Some($cname), )*
                    Self::Unknown(_) => None,
                }
            }
        }

        impl From<u16> for $name {
            fn from(code: u16) -> Self {
                Self::from_code(code)
            }
        }

        impl TryFrom<i32> for $name {
            type Error = CodeOutOfRange;

            fn try_from(value: i32) -> Result<Self, Self::Error> {
                u16::try_from(value)
                    .map(Self::from_code)
                    .map_err(|_| CodeOutOfRange(value))
            }
        }
    };
}

c_enum! {
/// Kind of owner of a limited item: an item-ownership record's owner type.
///
/// `OWN_` values.
    OwnerKind {
        /// `OWN_ROOM` = 0: lying in a room
        Room = 0 => "OWN_ROOM",
        /// `OWN_USER` = 1: carried by a player
        User = 1 => "OWN_USER",
        /// `OWN_MONSTER` = 2: carried by an active monster
        Monster = 2 => "OWN_MONSTER",
        /// `OWN_UNKNOWN` = 3: "Unknown" (in [`crate::names::OWNER_TYPES`]); never set
        UnknownOwner = 3 => "OWN_UNKNOWN",
    }
}

c_enum! {
/// Record type of a `WCCUPDAT` update record: the kind of record that follows.
///
/// `UPD_` values.
    UpdateRecordType {
        /// `UPD_RACE` = 1
        Race = 1 => "UPD_RACE",
        /// `UPD_CLASS` = 2
        Class = 2 => "UPD_CLASS",
        /// `UPD_ROOM` = 3
        Room = 3 => "UPD_ROOM",
        /// `UPD_SHOP` = 4
        Shop = 4 => "UPD_SHOP",
        /// `UPD_ITEM` = 5
        Item = 5 => "UPD_ITEM",
        /// `UPD_SPELL` = 6
        Spell = 6 => "UPD_SPELL",
        /// `UPD_KNMON` = 7
        KnownMonster = 7 => "UPD_KNMON",
        /// `UPD_MESSAGE` = 8
        Message = 8 => "UPD_MESSAGE",
        /// `UPD_TEXT` = 9
        Text = 9 => "UPD_TEXT",
        /// `UPD_ACTION` = 10
        Action = 10 => "UPD_ACTION",
    }
}

c_enum! {
/// A direction: the index of a room's exits and of [`crate::names::DIRECTION_NAMES`].
///
/// `DIR_` values.
    Direction {
        /// `DIR_NORTH` = 0: direction: north
        North = 0 => "DIR_NORTH",
        /// `DIR_SOUTH` = 1: south
        South = 1 => "DIR_SOUTH",
        /// `DIR_EAST` = 2: east
        East = 2 => "DIR_EAST",
        /// `DIR_WEST` = 3: west
        West = 3 => "DIR_WEST",
        /// `DIR_NE` = 4: northeast
        NorthEast = 4 => "DIR_NE",
        /// `DIR_NW` = 5: northwest
        NorthWest = 5 => "DIR_NW",
        /// `DIR_SE` = 6: southeast
        SouthEast = 6 => "DIR_SE",
        /// `DIR_SW` = 7: southwest
        SouthWest = 7 => "DIR_SW",
        /// `DIR_UP` = 8: up
        Up = 8 => "DIR_UP",
        /// `DIR_DOWN` = 9: down
        Down = 9 => "DIR_DOWN",
    }
}

/// `DIR_AUTOUSE`: passed where a direction goes to mean Auto Use in combat; not a direction
pub const DIR_AUTOUSE: u16 = 11;

c_enum! {
/// Exit type of each room exit; named in [`crate::names::EXIT_TYPES`].
///
/// `EXIT_` values.
    ExitType {
        /// `EXIT_NORMAL` = 0: exit type: "Normal"
        Normal = 0 => "EXIT_NORMAL",
        /// `EXIT_SPELL` = 1: "Spell"
        Spell = 1 => "EXIT_SPELL",
        /// `EXIT_KEY` = 2: "Key" (PICKLOCK, BASH, OPEN)
        Key = 2 => "EXIT_KEY",
        /// `EXIT_ITEM` = 3: "Item"
        Item = 3 => "EXIT_ITEM",
        /// `EXIT_TOLL` = 4: "Toll"
        Toll = 4 => "EXIT_TOLL",
        /// `EXIT_ACTION` = 5: "Action"
        Action = 5 => "EXIT_ACTION",
        /// `EXIT_HIDDEN` = 6: "Hidden" (EXF_ flags in par1)
        Hidden = 6 => "EXIT_HIDDEN",
        /// `EXIT_DOOR` = 7: "Door" (BASH, OPEN)
        Door = 7 => "EXIT_DOOR",
        /// `EXIT_MAPCHANGE` = 8: "Map Change"
        MapChange = 8 => "EXIT_MAPCHANGE",
        /// `EXIT_TRAP` = 9: "Trap"
        Trap = 9 => "EXIT_TRAP",
        /// `EXIT_TEXT` = 10: "Text"
        Text = 10 => "EXIT_TEXT",
        /// `EXIT_GATE` = 11: "Gate"
        Gate = 11 => "EXIT_GATE",
        /// `EXIT_REMOTE` = 12: "Remote Action"
        RemoteAction = 12 => "EXIT_REMOTE",
        /// `EXIT_CLASS` = 13: "Class"
        Class = 13 => "EXIT_CLASS",
        /// `EXIT_RACE` = 14: "Race"
        Race = 14 => "EXIT_RACE",
        /// `EXIT_LEVEL` = 15: "Level"
        Level = 15 => "EXIT_LEVEL",
        /// `EXIT_TIMED` = 16: "Timed"
        Timed = 16 => "EXIT_TIMED",
        /// `EXIT_TICKET` = 17: "Ticket"
        Ticket = 17 => "EXIT_TICKET",
        /// `EXIT_USERCOUNT` = 18: "User Count"
        UserCount = 18 => "EXIT_USERCOUNT",
        /// `EXIT_BLOCKGUARD` = 19: "Block Guard"
        BlockGuard = 19 => "EXIT_BLOCKGUARD",
        /// `EXIT_ALIGNMENT` = 20: "Alignment"
        Alignment = 20 => "EXIT_ALIGNMENT",
        /// `EXIT_DELAY` = 21: "Delay"
        Delay = 21 => "EXIT_DELAY",
        /// `EXIT_CAST` = 22: "Cast"
        Cast = 22 => "EXIT_CAST",
        /// `EXIT_ABILITY` = 23: "Ability"
        Ability = 23 => "EXIT_ABILITY",
        /// `EXIT_SPELLTRAP` = 24: "Spell Trap"
        SpellTrap = 24 => "EXIT_SPELLTRAP",
    }
}

c_enum! {
/// Room type; named in [`crate::names::ROOM_TYPES`]. `Colliseum` is the original spelling.
///
/// `ROOM_` values.
    RoomType {
        /// `ROOM_NORMAL` = 0: room type: "Normal"
        Normal = 0 => "ROOM_NORMAL",
        /// `ROOM_SHOP` = 1: "Shop" (shopnum is the shop)
        Shop = 1 => "ROOM_SHOP",
        /// `ROOM_ARENA` = 2: "Arena"
        Arena = 2 => "ROOM_ARENA",
        /// `ROOM_LAIR` = 3: "Lair" (monsters are generated)
        Lair = 3 => "ROOM_LAIR",
        /// `ROOM_HOTEL` = 4: "Hotel"
        Hotel = 4 => "ROOM_HOTEL",
        /// `ROOM_COLLISEUM` = 5: "Colliseum" (sic)
        Colliseum = 5 => "ROOM_COLLISEUM",
        /// `ROOM_JAIL` = 6: "Jail"
        Jail = 6 => "ROOM_JAIL",
        /// `ROOM_LIBRARY` = 7: "Library"
        Library = 7 => "ROOM_LIBRARY",
    }
}

c_enum! {
/// Gang house a room belongs to; named in [`crate::names::GANG_HOUSES`].
///
/// `GH_` values.
    GangHouse {
        /// `GH_NONE` = 0: gang house: "None"
        None = 0 => "GH_NONE",
        /// `GH_RED` = 1: "Red Gang House"
        Red = 1 => "GH_RED",
        /// `GH_ORANGE` = 2: "Orange Gang House"
        Orange = 2 => "GH_ORANGE",
        /// `GH_YELLOW` = 3: "Yellow Gang House"
        Yellow = 3 => "GH_YELLOW",
        /// `GH_GREEN` = 4: "Green Gang House"
        Green = 4 => "GH_GREEN",
        /// `GH_VIOLET` = 5: "Violet Gang House"
        Violet = 5 => "GH_VIOLET",
        /// `GH_BLUE` = 6: "Blue Gang House"
        Blue = 6 => "GH_BLUE",
        /// `GH_BLACK` = 7: "Black Gang House"
        Black = 7 => "GH_BLACK",
        /// `GH_SILVER` = 8: "Silver Gang House"
        Silver = 8 => "GH_SILVER",
        /// `GH_GOLD` = 9: "Gold Gang House"
        Gold = 9 => "GH_GOLD",
        /// `GH_WHITE` = 10: "White Gang House" (the last)
        White = 10 => "GH_WHITE",
    }
}

c_enum! {
/// Shop type; named in [`crate::names::SHOP_TYPES`].
///
/// `SHOP_` values.
    ShopType {
        /// `SHOP_GENERAL` = 0: shop type: "General"
        General = 0 => "SHOP_GENERAL",
        /// `SHOP_WEAPONS` = 1: "Weapons"
        Weapons = 1 => "SHOP_WEAPONS",
        /// `SHOP_ARMOUR` = 2: "Armour"
        Armour = 2 => "SHOP_ARMOUR",
        /// `SHOP_ITEMS` = 3: "Items"
        Items = 3 => "SHOP_ITEMS",
        /// `SHOP_SPELLS` = 4: "Spells"
        Spells = 4 => "SHOP_SPELLS",
        /// `SHOP_HOSPITAL` = 5: "Hospital"
        Hospital = 5 => "SHOP_HOSPITAL",
        /// `SHOP_TAVERN` = 6: "Tavern"
        Tavern = 6 => "SHOP_TAVERN",
        /// `SHOP_BANK` = 7: "Bank" (DEPOSIT, WITHDRAW)
        Bank = 7 => "SHOP_BANK",
        /// `SHOP_TRAINING` = 8: "Training" (TRAIN)
        Training = 8 => "SHOP_TRAINING",
        /// `SHOP_INN` = 9: "Inn"
        Inn = 9 => "SHOP_INN",
        /// `SHOP_SPECIFIC` = 10: "Specific"
        Specific = 10 => "SHOP_SPECIFIC",
        /// `SHOP_GANG` = 11: "Gang Shop" (STOCK, UNSTOCK)
        GangShop = 11 => "SHOP_GANG",
        /// `SHOP_DEED` = 12: "Deed Shop" (gang house deeds)
        DeedShop = 12 => "SHOP_DEED",
    }
}

c_enum! {
/// Item type. Numbered by the code, not by the shifted name table;
/// see [`crate::names::ITEM_TYPES`] and [`ItemType::display_name`].
///
/// `ITEM_` values.
    ItemType {
        /// `ITEM_ARMOUR` = 0: item type: armour
        Armour = 0 => "ITEM_ARMOUR",
        /// `ITEM_WEAPON` = 1: weapon
        Weapon = 1 => "ITEM_WEAPON",
        /// `ITEM_PROJECTILE` = 2: projectile (thrown)
        Projectile = 2 => "ITEM_PROJECTILE",
        /// `ITEM_SPECIAL` = 3: special (statues etc., inferred)
        Special = 3 => "ITEM_SPECIAL",
        /// `ITEM_FOOD` = 4: food (EAT)
        Food = 4 => "ITEM_FOOD",
        /// `ITEM_DRINK` = 5: drink (DRINK)
        Drink = 5 => "ITEM_DRINK",
        /// `ITEM_LIGHT` = 6: light source (LIGHT)
        Light = 6 => "ITEM_LIGHT",
        /// `ITEM_KEY` = 7: key (kept in the key slots)
        Key = 7 => "ITEM_KEY",
        /// `ITEM_CONTAINER` = 8: container (OPEN)
        Container = 8 => "ITEM_CONTAINER",
        /// `ITEM_SCROLL` = 9: scroll (Link to Spell)
        Scroll = 9 => "ITEM_SCROLL",
        /// `ITEM_GENERAL` = 10: general (inferred)
        General = 10 => "ITEM_GENERAL",
    }
}

c_enum! {
/// Wear location of an item; named in [`crate::names::WORN_LOCATIONS`].
///
/// `WORN_` values.
    WornLocation {
        /// `WORN_NOWHERE` = 0: worn location: "Nowhere"
        Nowhere = 0 => "WORN_NOWHERE",
        /// `WORN_WORN` = 1: "Worn"
        Worn = 1 => "WORN_WORN",
        /// `WORN_HEAD` = 2: "Head"
        Head = 2 => "WORN_HEAD",
        /// `WORN_HANDS` = 3: "Hands"
        Hands = 3 => "WORN_HANDS",
        /// `WORN_FINGER` = 4: "Finger" (first ring)
        Finger = 4 => "WORN_FINGER",
        /// `WORN_FEET` = 5: "Feet"
        Feet = 5 => "WORN_FEET",
        /// `WORN_ARMS` = 6: "Arms"
        Arms = 6 => "WORN_ARMS",
        /// `WORN_BACK` = 7: "Back"
        Back = 7 => "WORN_BACK",
        /// `WORN_NECK` = 8: "Neck"
        Neck = 8 => "WORN_NECK",
        /// `WORN_LEGS` = 9: "Legs"
        Legs = 9 => "WORN_LEGS",
        /// `WORN_WAIST` = 10: "Waist"
        Waist = 10 => "WORN_WAIST",
        /// `WORN_TORSO` = 11: "Torso"
        Torso = 11 => "WORN_TORSO",
        /// `WORN_OFFHAND` = 12: "Off-Hand" (shield)
        OffHand = 12 => "WORN_OFFHAND",
        /// `WORN_FINGER2` = 13: "Finger" (second ring)
        Finger2 = 13 => "WORN_FINGER2",
        /// `WORN_WRIST` = 14: "Wrist" (first bracelet)
        Wrist = 14 => "WORN_WRIST",
        /// `WORN_EARS` = 15: "Ears"
        Ears = 15 => "WORN_EARS",
        /// `WORN_WORN2` = 16: "Worn" (second)
        Worn2 = 16 => "WORN_WORN2",
        /// `WORN_WRIST2` = 17: "Wrist" (second bracelet)
        Wrist2 = 17 => "WORN_WRIST2",
        /// `WORN_EYES` = 18: "Eyes"
        Eyes = 18 => "WORN_EYES",
        /// `WORN_FACE` = 19: "Face"
        Face = 19 => "WORN_FACE",
    }
}

c_enum! {
/// Weapon type of an item; named in [`crate::names::WEAPON_TYPES`].
///
/// `WPN_` values.
    WeaponType {
        /// `WPN_1HBLUNT` = 0: weapon type: "1-Handed Blunt"
        OneHandBlunt = 0 => "WPN_1HBLUNT",
        /// `WPN_2HBLUNT` = 1: "2-Handed Blunt"
        TwoHandBlunt = 1 => "WPN_2HBLUNT",
        /// `WPN_1HSHARP` = 2: "1-Handed Sharp"
        OneHandSharp = 2 => "WPN_1HSHARP",
        /// `WPN_2HSHARP` = 3: "2-Handed Sharp"
        TwoHandSharp = 3 => "WPN_2HSHARP",
    }
}

c_enum! {
/// Armour type of an item or the heaviest a class may wear, lightest first; named in
/// [`crate::names::ARMOUR_TYPES`].
///
/// `ARM_` values.
    ArmourType {
        /// `ARM_NATURAL` = 0: armour type: "Natural"
        Natural = 0 => "ARM_NATURAL",
        /// `ARM_ROBES` = 1: "Robes"
        Robes = 1 => "ARM_ROBES",
        /// `ARM_PADDED` = 2: "Padded"
        Padded = 2 => "ARM_PADDED",
        /// `ARM_SOFTLEATHER` = 3: "Soft Leather"
        SoftLeather = 3 => "ARM_SOFTLEATHER",
        /// `ARM_SOFTSTUDDED` = 4: "Soft Studded Leather"
        SoftStuddedLeather = 4 => "ARM_SOFTSTUDDED",
        /// `ARM_RIGIDLEATHER` = 5: "Rigid Leather"
        RigidLeather = 5 => "ARM_RIGIDLEATHER",
        /// `ARM_RIGIDSTUDDED` = 6: "Studded Rigid Leather"
        StuddedRigidLeather = 6 => "ARM_RIGIDSTUDDED",
        /// `ARM_CHAINMAIL` = 7: "Chainmail"
        Chainmail = 7 => "ARM_CHAINMAIL",
        /// `ARM_SCALEMAIL` = 8: "Scalemail"
        Scalemail = 8 => "ARM_SCALEMAIL",
        /// `ARM_PLATEMAIL` = 9: "Platemail"
        Platemail = 9 => "ARM_PLATEMAIL",
    }
}

c_enum! {
/// Weapons a class may use; named in [`crate::names::WEAPON_CLASSES`].
///
/// `WCLS_` values.
    WeaponClass {
        /// `WCLS_1HBLUNT` = 0: class weapons: "1-handed Blunt"
        OneHandBlunt = 0 => "WCLS_1HBLUNT",
        /// `WCLS_2HBLUNT` = 1: "2-handed Blunt"
        TwoHandBlunt = 1 => "WCLS_2HBLUNT",
        /// `WCLS_1HSHARP` = 2: "1-handed Sharp"
        OneHandSharp = 2 => "WCLS_1HSHARP",
        /// `WCLS_2HSHARP` = 3: "2-handed Sharp"
        TwoHandSharp = 3 => "WCLS_2HSHARP",
        /// `WCLS_ANY1H` = 4: "Any 1-handed"
        AnyOneHand = 4 => "WCLS_ANY1H",
        /// `WCLS_ANY2H` = 5: "Any 2-handed"
        AnyTwoHand = 5 => "WCLS_ANY2H",
        /// `WCLS_ANYSHARP` = 6: "Any Sharp"
        AnySharp = 6 => "WCLS_ANYSHARP",
        /// `WCLS_ANYBLUNT` = 7: "Any Blunt"
        AnyBlunt = 7 => "WCLS_ANYBLUNT",
        /// `WCLS_ALL` = 8: "All Weapons"
        All = 8 => "WCLS_ALL",
        /// `WCLS_LIMITED` = 9: "Limited"
        Limited = 9 => "WCLS_LIMITED",
    }
}

c_enum! {
/// Currency of an item's price, copper first; named in [`crate::names::CURRENCY_NAMES`].
///
/// `CUR_` values.
    Currency {
        /// `CUR_COPPER` = 0: currency: copper farthings
        Copper = 0 => "CUR_COPPER",
        /// `CUR_SILVER` = 1: silver nobles
        Silver = 1 => "CUR_SILVER",
        /// `CUR_GOLD` = 2: gold crowns
        Gold = 2 => "CUR_GOLD",
        /// `CUR_PLATINUM` = 3: platinum pieces
        Platinum = 3 => "CUR_PLATINUM",
        /// `CUR_RUNIC` = 4: the sysop-named fifth currency
        Runic = 4 => "CUR_RUNIC",
        /// `CUR_UNDEFINED` = 5: "Undefined" (currname1\[\] only)
        Undefined = 5 => "CUR_UNDEFINED",
    }
}

c_enum! {
/// Monster generation type; named in [`crate::names::MONSTER_TYPES`].
///
/// `MTYPE_` values.
    MonsterType {
        /// `MTYPE_LAIR` = 0: monster type: "Lair"
        Lair = 0 => "MTYPE_LAIR",
        /// `MTYPE_WANDERER` = 1: "Wanderer"
        Wanderer = 1 => "MTYPE_WANDERER",
        /// `MTYPE_NPC` = 2: "NPC"
        Npc = 2 => "MTYPE_NPC",
        /// `MTYPE_LIVING` = 3: "Living"
        Living = 3 => "MTYPE_LIVING",
        /// `MTYPE_RANDOM` = 4: "Random"
        Random = 4 => "MTYPE_RANDOM",
        /// `MTYPE_GUARD` = 5: "Guard"
        Guard = 5 => "MTYPE_GUARD",
        /// `MTYPE_GROUP1` = 6: "Group 1"
        Group1 = 6 => "MTYPE_GROUP1",
        /// `MTYPE_GROUP2` = 7: "Group 2"
        Group2 = 7 => "MTYPE_GROUP2",
        /// `MTYPE_GROUP3` = 8: "Group 3"
        Group3 = 8 => "MTYPE_GROUP3",
        /// `MTYPE_GROUP4` = 9: "Group 4"
        Group4 = 9 => "MTYPE_GROUP4",
        /// `MTYPE_GROUP5` = 10: "Group 5"
        Group5 = 10 => "MTYPE_GROUP5",
        /// `MTYPE_GROUP6` = 11: "Group 6"
        Group6 = 11 => "MTYPE_GROUP6",
        /// `MTYPE_GROUP7` = 12: "Group 7"
        Group7 = 12 => "MTYPE_GROUP7",
        /// `MTYPE_GROUP8` = 13: "Group 8"
        Group8 = 13 => "MTYPE_GROUP8",
        /// `MTYPE_GROUP9` = 14: "Group 9"
        Group9 = 14 => "MTYPE_GROUP9",
        /// `MTYPE_GROUP10` = 15: "Group 10"
        Group10 = 15 => "MTYPE_GROUP10",
        /// `MTYPE_GROUP11` = 16: "Group 11"
        Group11 = 16 => "MTYPE_GROUP11",
        /// `MTYPE_GROUP12` = 17: "Group 12"
        Group12 = 17 => "MTYPE_GROUP12",
        /// `MTYPE_GROUP13` = 18: "Group 13"
        Group13 = 18 => "MTYPE_GROUP13",
        /// `MTYPE_GROUP14` = 19: "Group 14"
        Group14 = 19 => "MTYPE_GROUP14",
        /// `MTYPE_GROUP15` = 20: "Group 15"
        Group15 = 20 => "MTYPE_GROUP15",
        /// `MTYPE_GROUP16` = 21: "Group 16"
        Group16 = 21 => "MTYPE_GROUP16",
        /// `MTYPE_GROUP17` = 22: "Group 17"
        Group17 = 22 => "MTYPE_GROUP17",
        /// `MTYPE_GROUP18` = 23: "Group 18"
        Group18 = 23 => "MTYPE_GROUP18",
        /// `MTYPE_GROUP19` = 24: "Group 19"
        Group19 = 24 => "MTYPE_GROUP19",
        /// `MTYPE_GROUP20` = 25: "Group 20"
        Group20 = 25 => "MTYPE_GROUP20",
        /// `MTYPE_GROUP21` = 26: "Group 21"
        Group21 = 26 => "MTYPE_GROUP21",
        /// `MTYPE_GROUP22` = 27: "Group 22"
        Group22 = 27 => "MTYPE_GROUP22",
        /// `MTYPE_GROUP23` = 28: "Group 23"
        Group23 = 28 => "MTYPE_GROUP23",
        /// `MTYPE_GROUP24` = 29: "Group 24"
        Group24 = 29 => "MTYPE_GROUP24",
        /// `MTYPE_GROUP25` = 30: "Group 25"
        Group25 = 30 => "MTYPE_GROUP25",
        /// `MTYPE_GROUP26` = 31: "Group 26"
        Group26 = 31 => "MTYPE_GROUP26",
        /// `MTYPE_GROUP27` = 32: "Group 27"
        Group27 = 32 => "MTYPE_GROUP27",
        /// `MTYPE_GROUP28` = 33: "Group 28"
        Group28 = 33 => "MTYPE_GROUP28",
        /// `MTYPE_GROUP29` = 34: "Group 29"
        Group29 = 34 => "MTYPE_GROUP29",
        /// `MTYPE_GROUP30` = 35: "Group 30"
        Group30 = 35 => "MTYPE_GROUP30",
        /// `MTYPE_ARENA` = 36: "Arena"
        Arena = 36 => "MTYPE_ARENA",
        /// `MTYPE_ANGEL` = 37: "Angel"
        Angel = 37 => "MTYPE_ANGEL",
        /// `MTYPE_QUEST` = 38: "Quest"
        Quest = 38 => "MTYPE_QUEST",
    }
}

c_enum! {
/// Monster alignment; named in [`crate::names::ALIGNMENTS`].
///
/// `ALIGN_` values.
    Alignment {
        /// `ALIGN_GOOD` = 0: alignment: "Good"
        Good = 0 => "ALIGN_GOOD",
        /// `ALIGN_EVIL` = 1: "Evil"
        Evil = 1 => "ALIGN_EVIL",
        /// `ALIGN_CHAOTICEVIL` = 2: "Chaotic Evil"
        ChaoticEvil = 2 => "ALIGN_CHAOTICEVIL",
        /// `ALIGN_NEUTRAL` = 3: "Neutral"
        Neutral = 3 => "ALIGN_NEUTRAL",
        /// `ALIGN_LAWFULGOOD` = 4: "Lawful Good"
        LawfulGood = 4 => "ALIGN_LAWFULGOOD",
        /// `ALIGN_NEUTRALEVIL` = 5: "Neutral Evil"
        NeutralEvil = 5 => "ALIGN_NEUTRALEVIL",
        /// `ALIGN_LAWFULEVIL` = 6: "Lawful Evil"
        LawfulEvil = 6 => "ALIGN_LAWFULEVIL",
    }
}

c_enum! {
/// Group role of a monster; named in [`crate::names::FOLLOW_TYPES`].
///
/// `FOLLOW_` values.
    GroupRole {
        /// `FOLLOW_SOLO` = 0: group role: "Solo"
        Solo = 0 => "FOLLOW_SOLO",
        /// `FOLLOW_LEADER` = 1: "Leader" (others follow it)
        Leader = 1 => "FOLLOW_LEADER",
        /// `FOLLOW_FOLLOWER` = 2: "Follower" (follows a leader)
        Follower = 2 => "FOLLOW_FOLLOWER",
        /// `FOLLOW_STATIONARY` = 3: "Stationary" (never moves)
        Stationary = 3 => "FOLLOW_STATIONARY",
    }
}

c_enum! {
/// Monster gender; its pronoun is in [`crate::names::PRONOUNS`].
///
/// `GENDER_` values.
    Gender {
        /// `GENDER_IT` = 0: monster gender: "It"
        It = 0 => "GENDER_IT",
        /// `GENDER_HE` = 1: "He"
        He = 1 => "GENDER_HE",
        /// `GENDER_SHE` = 2: "She"
        She = 2 => "GENDER_SHE",
    }
}

c_enum! {
/// Type of each monster attack slot.
///
/// `MATK_` values.
    MonsterAttackType {
        /// `MATK_NONE` = 0: monster attack: unused slot
        None = 0 => "MATK_NONE",
        /// `MATK_NORMAL` = 1: normal (weapon) attack
        Normal = 1 => "MATK_NORMAL",
        /// `MATK_SPELL` = 2: casts a spell
        Spell = 2 => "MATK_SPELL",
        /// `MATK_ROB` = 3: robs the player
        Rob = 3 => "MATK_ROB",
    }
}

c_enum! {
/// Spell type; named in [`crate::names::SPELL_TYPES`].
///
/// `SPT_` values.
    SpellType {
        /// `SPT_AUTOCOMBAT` = 0: spell type: "Auto-Combat"
        Autocombat = 0 => "SPT_AUTOCOMBAT",
        /// `SPT_POISON` = 1: "Poison"
        Poison = 1 => "SPT_POISON",
        /// `SPT_CHANGEME` = 2: "Change me" (sic, placeholder)
        ChangeMe = 2 => "SPT_CHANGEME",
        /// `SPT_GENERAL` = 3: "General"
        General = 3 => "SPT_GENERAL",
    }
}

c_enum! {
/// Spell target; named in [`crate::names::SPELL_TARGETS`].
///
/// `TGT_` values.
    SpellTarget {
        /// `TGT_SINGLE` = 0: spell target: "Single"
        Single = 0 => "TGT_SINGLE",
        /// `TGT_SELF` = 1: "Self"
        OnSelf = 1 => "TGT_SELF",
        /// `TGT_SELFORSINGLE` = 2: "Self or Single"
        SelfOrSingle = 2 => "TGT_SELFORSINGLE",
        /// `TGT_DIVAREA` = 3: "Divided Area Not You"
        DivArea = 3 => "TGT_DIVAREA",
        /// `TGT_MONSTER` = 4: "Monster"
        Monster = 4 => "TGT_MONSTER",
        /// `TGT_DIVAREAINCL` = 5: "Divided Area Incl. You"
        DivAreaIncl = 5 => "TGT_DIVAREAINCL",
        /// `TGT_ANY` = 6: "Any"
        Any = 6 => "TGT_ANY",
        /// `TGT_ITEM` = 7: "Item"
        Item = 7 => "TGT_ITEM",
        /// `TGT_MONORSINGLE` = 8: "Monster or Single"
        MonsterOrSingle = 8 => "TGT_MONORSINGLE",
        /// `TGT_DIVATTACK` = 9: "Divided Attack Area"
        DivAttack = 9 => "TGT_DIVATTACK",
        /// `TGT_DIVPARTY` = 10: "Divided Party Area"
        DivParty = 10 => "TGT_DIVPARTY",
        /// `TGT_FULLAREA` = 11: "Full Area"
        FullArea = 11 => "TGT_FULLAREA",
        /// `TGT_FULLATTACK` = 12: "Full Attack Area"
        FullAttack = 12 => "TGT_FULLATTACK",
        /// `TGT_FULLPARTY` = 13: "Full Party Area"
        FullParty = 13 => "TGT_FULLPARTY",
    }
}

c_enum! {
/// Magic school of a spell or class; named in [`crate::names::MAGERY_TYPES`].
///
/// `MAGERY_` values.
    Magery {
        /// `MAGERY_NONE` = 0: magery: "None"
        None = 0 => "MAGERY_NONE",
        /// `MAGERY_MAGE` = 1: "Mage"
        Mage = 1 => "MAGERY_MAGE",
        /// `MAGERY_PRIEST` = 2: "Priest"
        Priest = 2 => "MAGERY_PRIEST",
        /// `MAGERY_DRUID` = 3: "Druid"
        Druid = 3 => "MAGERY_DRUID",
        /// `MAGERY_BARD` = 4: "Bard"
        Bard = 4 => "MAGERY_BARD",
        /// `MAGERY_KAI` = 5: "Kai" (powers, kai not mana)
        Kai = 5 => "MAGERY_KAI",
    }
}

c_enum! {
/// Spell attack element; named in [`crate::names::RESIST_TYPES`].
///
/// `ELEM_` values.
    Element {
        /// `ELEM_COLD` = 0: spell element: "Cold"
        Cold = 0 => "ELEM_COLD",
        /// `ELEM_HOT` = 1: "Hot"
        Hot = 1 => "ELEM_HOT",
        /// `ELEM_STONE` = 2: "Stone"
        Stone = 2 => "ELEM_STONE",
        /// `ELEM_LIGHTNING` = 3: "Lightening" (sic)
        Lightning = 3 => "ELEM_LIGHTNING",
        /// `ELEM_NORMAL` = 4: "Normal"
        Normal = 4 => "ELEM_NORMAL",
        /// `ELEM_WATER` = 5: "Water"
        Water = 5 => "ELEM_WATER",
        /// `ELEM_POISON` = 6: "Poison"
        Poison = 6 => "ELEM_POISON",
    }
}

c_enum! {
/// Stat that resists a spell; named in [`crate::names::STAT_NAMES`].
///
/// `RSTAT_` values.
    ResistStat {
        /// `RSTAT_MR` = 0: resist stat: "Magic Resistance"
        MagicRes = 0 => "RSTAT_MR",
        /// `RSTAT_STR` = 1: "Strength"
        Strength = 1 => "RSTAT_STR",
        /// `RSTAT_AGI` = 2: "Agility"
        Agility = 2 => "RSTAT_AGI",
        /// `RSTAT_HEA` = 3: "Health"
        Health = 3 => "RSTAT_HEA",
        /// `RSTAT_INT` = 4: "Intellect"
        Intellect = 4 => "RSTAT_INT",
        /// `RSTAT_WIS` = 5: "Wisdom"
        Wisdom = 5 => "RSTAT_WIS",
        /// `RSTAT_CHA` = 6: "Charm"
        Charm = 6 => "RSTAT_CHA",
    }
}

c_enum! {
/// When magic resistance is rolled against a spell.
///
/// `SRES_` values.
    ResistMode {
        /// `SRES_NEVER` = 0: resist mode: never
        Never = 0 => "SRES_NEVER",
        /// `SRES_ANTIMAG` = 1: only if the target has Anti Magic
        AntiMagic = 1 => "SRES_ANTIMAG",
        /// `SRES_ALWAYS` = 2: always
        Always = 2 => "SRES_ALWAYS",
    }
}

c_enum! {
/// A player's message level.
///
/// Inverted: a lower number is a *higher* level. The numeric values are
/// kept as the original stores them; do not reorder.
///
/// `MLV_` values.
    MessageLevel {
        /// `MLV_HIGH` = 0: message level: "High"
        High = 0 => "MLV_HIGH",
        /// `MLV_MEDIUM` = 10: "Medium"
        Medium = 10 => "MLV_MEDIUM",
        /// `MLV_LOW` = 20: "Low"
        Low = 20 => "MLV_LOW",
    }
}

c_enum! {
/// A player's status line mode. `SL_OFF/ON/FULL` sit with the record codes; the two custom
/// modes `SL_CUSTOM`/`SL_FULLCUSTOM` with the command codes. The custom template itself lives
/// in the session and is kept in the player's permanent record.
///
/// `SL_` values.
    StatlineMode {
        /// `SL_OFF` = 0: status line: "Off"
        Off = 0 => "SL_OFF",
        /// `SL_ON` = 1: "On"
        On = 1 => "SL_ON",
        /// `SL_FULL` = 2: "Full"
        Full = 2 => "SL_FULL",
        /// `SL_CUSTOM`: the player's own text.
        Custom = 3 => "SL_CUSTOM",
        /// `SL_FULLCUSTOM`: the player's own text, full mode.
        FullCustom = 4 => "SL_FULLCUSTOM",
    }
}

c_enum! {
/// Item material; named in [`crate::names::MATERIALS`]. No record field is known to hold one.
///
/// `MAT_` values.
    Material {
        /// `MAT_IRON` = 0: material: "Iron" (no field known)
        Iron = 0 => "MAT_IRON",
        /// `MAT_STEEL` = 1: "Steel"
        Steel = 1 => "MAT_STEEL",
        /// `MAT_BRONZE` = 2: "Bronze"
        Bronze = 2 => "MAT_BRONZE",
        /// `MAT_SILVER` = 3: "Silver"
        Silver = 3 => "MAT_SILVER",
        /// `MAT_GOLD` = 4: "Gold"
        Gold = 4 => "MAT_GOLD",
        /// `MAT_PLATINUM` = 5: "Platinum"
        Platinum = 5 => "MAT_PLATINUM",
        /// `MAT_COTTON` = 6: "Cotton"
        Cotton = 6 => "MAT_COTTON",
    }
}

c_enum! {
/// Ability number: the index into [`crate::names::ABILITY_NAMES`].
///
/// `ABL_` values.
    Ability {
        /// `ABL_NONE` = 0: "No ability"
        None = 0 => "ABL_NONE",
        /// `ABL_DAMAGENOARMOUR` = 1: "Inflict Damage Ignoring Armour"
        DamageNoArmour = 1 => "ABL_DAMAGENOARMOUR",
        /// `ABL_DEFENCE` = 2: "Alter Defence Value"
        Defence = 2 => "ABL_DEFENCE",
        /// `ABL_COLDDEFENCE` = 3: "Alter Cold Attack Damage (Defence)"
        ColdDefence = 3 => "ABL_COLDDEFENCE",
        /// `ABL_DAMAGE` = 4: "Alter Damage (Attacking)"
        Damage = 4 => "ABL_DAMAGE",
        /// `ABL_HOTDEFENCE` = 5: "Alter Hot Attack Damage (Defence)"
        HotDefence = 5 => "ABL_HOTDEFENCE",
        /// `ABL_ENSLAVE` = 6: "Enslave Creature"
        Enslave = 6 => "ABL_ENSLAVE",
        /// `ABL_DAMAGERESIST` = 7: "Alter Damage Resistance Value"
        DamageResist = 7 => "ABL_DAMAGERESIST",
        /// `ABL_STEALHP` = 8: "Steal Hit Points"
        StealHp = 8 => "ABL_STEALHP",
        /// `ABL_SHADOWSTEALTH` = 9: "Shadow Stealth"
        ShadowStealth = 9 => "ABL_SHADOWSTEALTH",
        /// `ABL_SHIELD` = 10: "Protective Shield"
        Shield = 10 => "ABL_SHIELD",
        /// `ABL_ENERGY` = 11: "Alter Energy Level"
        Energy = 11 => "ABL_ENERGY",
        /// `ABL_SUMMON` = 12: "Summon Creature"
        Summon = 12 => "ABL_SUMMON",
        /// `ABL_USERLIGHT` = 13: "Alter User Light"
        UserLight = 13 => "ABL_USERLIGHT",
        /// `ABL_ROOMLIGHT` = 14: "Alter Room Light"
        RoomLight = 14 => "ABL_ROOMLIGHT",
        /// `ABL_HUNGER` = 15: "Alter Hunger"
        Hunger = 15 => "ABL_HUNGER",
        /// `ABL_THIRST` = 16: "Alter Thirst"
        Thirst = 16 => "ABL_THIRST",
        /// `ABL_DAMAGEARMOUR` = 17: "Inflict Damage on Armour"
        DamageArmour = 17 => "ABL_DAMAGEARMOUR",
        /// `ABL_HEAL` = 18: "Heal"
        Heal = 18 => "ABL_HEAL",
        /// `ABL_POISON` = 19: "Poison"
        Poison = 19 => "ABL_POISON",
        /// `ABL_CUREPOISON` = 20: "Cure Poison"
        CurePoison = 20 => "ABL_CUREPOISON",
        /// `ABL_POISONIMMUNE` = 21: "Poison Immunity"
        PoisonImmune = 21 => "ABL_POISONIMMUNE",
        /// `ABL_AVBLESS` = 22: "Alter AV (Bless)"
        AvBless = 22 => "ABL_AVBLESS",
        /// `ABL_KILLDEAD` = 23: "Kill Dead"
        KillDead = 23 => "ABL_KILLDEAD",
        /// `ABL_DEFVSEVIL` = 24: "Alter defence against Evil"
        DefVsEvil = 24 => "ABL_DEFVSEVIL",
        /// `ABL_DEFVSGOOD` = 25: "Alter defence against Good"
        DefVsGood = 25 => "ABL_DEFVSGOOD",
        /// `ABL_DETECTMAGIC` = 26: "Detect Magic"
        DetectMagic = 26 => "ABL_DETECTMAGIC",
        /// `ABL_STEALTH` = 27: "Stealth"
        Stealth = 27 => "ABL_STEALTH",
        /// `ABL_MAGICAL` = 28: "Magical"
        Magical = 28 => "ABL_MAGICAL",
        /// `ABL_PUNCH` = 29: "Punch"
        Punch = 29 => "ABL_PUNCH",
        /// `ABL_KICK` = 30: "Kick"
        Kick = 30 => "ABL_KICK",
        /// `ABL_BASH` = 31: "Bash"
        Bash = 31 => "ABL_BASH",
        /// `ABL_SMASH` = 32: "Smash"
        Smash = 32 => "ABL_SMASH",
        /// `ABL_KILLBLOW` = 33: "Killblow"
        Killblow = 33 => "ABL_KILLBLOW",
        /// `ABL_DODGE` = 34: "Dodge"
        Dodge = 34 => "ABL_DODGE",
        /// `ABL_JUMPKICK` = 35: "Jumpkick"
        Jumpkick = 35 => "ABL_JUMPKICK",
        /// `ABL_MAGICRESIST` = 36: "Magic Resistance"
        MagicResist = 36 => "ABL_MAGICRESIST",
        /// `ABL_PICKLOCKS` = 37: "Picklocks"
        Picklocks = 37 => "ABL_PICKLOCKS",
        /// `ABL_TRACKING` = 38: "Tracking"
        Tracking = 38 => "ABL_TRACKING",
        /// `ABL_THIEVERY` = 39: "Thievery"
        Thievery = 39 => "ABL_THIEVERY",
        /// `ABL_FINDTRAPS` = 40: "Find Traps"
        FindTraps = 40 => "ABL_FINDTRAPS",
        /// `ABL_DISARMTRAPS` = 41: "Disarm Traps"
        DisarmTraps = 41 => "ABL_DISARMTRAPS",
        /// `ABL_LINKSPELL` = 42: "Link to Spell"
        LinkSpell = 42 => "ABL_LINKSPELL",
        /// `ABL_ONETIMECAST` = 43: "One Time Cast"
        OneTimeCast = 43 => "ABL_ONETIMECAST",
        /// `ABL_INTELLECT` = 44: "Alter Intellect"
        Intellect = 44 => "ABL_INTELLECT",
        /// `ABL_WISDOM` = 45: "Alter Wisdom"
        Wisdom = 45 => "ABL_WISDOM",
        /// `ABL_STRENGTH` = 46: "Alter Strength"
        Strength = 46 => "ABL_STRENGTH",
        /// `ABL_HEALTH` = 47: "Alter Health"
        Health = 47 => "ABL_HEALTH",
        /// `ABL_AGILITY` = 48: "Alter Agility"
        Agility = 48 => "ABL_AGILITY",
        /// `ABL_CHARM` = 49: "Alter Charm"
        Charm = 49 => "ABL_CHARM",
        /// `ABL_QUEST` = 50: "Quest"
        Quest = 50 => "ABL_QUEST",
        /// `ABL_ANTIMAGIC` = 51: "Anti Magic"
        AntiMagic = 51 => "ABL_ANTIMAGIC",
        /// `ABL_EVILINCOMBAT` = 52: "Evil in Combat"
        EvilInCombat = 52 => "ABL_EVILINCOMBAT",
        /// `ABL_BLINDINGLIGHT` = 53: "Blinding Light"
        BlindingLight = 53 => "ABL_BLINDINGLIGHT",
        /// `ABL_GENLIGHT` = 54: "Alter General Light"
        GenLight = 54 => "ABL_GENLIGHT",
        /// `ABL_GENLIGHTTIME` = 55: "Alter General Light Duration"
        GenLightTime = 55 => "ABL_GENLIGHTTIME",
        /// `ABL_RECHARGE` = 56: "Recharge Item"
        Recharge = 56 => "ABL_RECHARGE",
        /// `ABL_SEEHIDDEN` = 57: "See Hidden"
        SeeHidden = 57 => "ABL_SEEHIDDEN",
        /// `ABL_CRITICAL` = 58: "Alter Critical Hit Chance"
        Critical = 58 => "ABL_CRITICAL",
        /// `ABL_CLASSITEM` = 59: "Class Item Inclusion"
        ClassItem = 59 => "ABL_CLASSITEM",
        /// `ABL_FLEE` = 60: "Flee"
        Flee = 60 => "ABL_FLEE",
        /// `ABL_AFFECTEXIT` = 61: "Affect Exit"
        AffectExit = 61 => "ABL_AFFECTEXIT",
        /// `ABL_EVILCHANCE` = 62: "Alter Evil Chance"
        EvilChance = 62 => "ABL_EVILCHANCE",
        /// `ABL_EXPERIENCE` = 63: "Alter Experience"
        Experience = 63 => "ABL_EXPERIENCE",
        /// `ABL_ADDCP` = 64: "Add CP"
        AddCp = 64 => "ABL_ADDCP",
        /// `ABL_RESISTSTONE` = 65: "Resist Stone"
        ResistStone = 65 => "ABL_RESISTSTONE",
        /// `ABL_RESISTLIGHTNING` = 66: "Resist Lightning"
        ResistLightning = 66 => "ABL_RESISTLIGHTNING",
        /// `ABL_QUICKNESS` = 67: "Quickness"
        Quickness = 67 => "ABL_QUICKNESS",
        /// `ABL_SLOWNESS` = 68: "Slowness"
        Slowness = 68 => "ABL_SLOWNESS",
        /// `ABL_MANA` = 69: "Mana"
        Mana = 69 => "ABL_MANA",
        /// `ABL_SPELLCASTING` = 70: "Spellcasting"
        Spellcasting = 70 => "ABL_SPELLCASTING",
        /// `ABL_CONFUSION` = 71: "Confusion"
        Confusion = 71 => "ABL_CONFUSION",
        /// `ABL_DAMAGESHIELD` = 72: "Damaging shield"
        DamageShield = 72 => "ABL_DAMAGESHIELD",
        /// `ABL_DISPELMAGIC` = 73: "Dispell magic"
        DispelMagic = 73 => "ABL_DISPELMAGIC",
        /// `ABL_HOLDPERSON` = 74: "Hold person"
        HoldPerson = 74 => "ABL_HOLDPERSON",
        /// `ABL_PARALYZE` = 75: "Paralyze"
        Paralyze = 75 => "ABL_PARALYZE",
        /// `ABL_MUTE` = 76: "Mute"
        Mute = 76 => "ABL_MUTE",
        /// `ABL_PERCEPTION` = 77: "Perception"
        Perception = 77 => "ABL_PERCEPTION",
        /// `ABL_ANIMAL` = 78: "Animal"
        Animal = 78 => "ABL_ANIMAL",
        /// `ABL_MAGEBIND` = 79: "Magebind"
        Magebind = 79 => "ABL_MAGEBIND",
        /// `ABL_AFFECTSANIMAL` = 80: "Affects Animal"
        AffectsAnimal = 80 => "ABL_AFFECTSANIMAL",
        /// `ABL_FREEDOM` = 81: "Freedom"
        Freedom = 81 => "ABL_FREEDOM",
        /// `ABL_CURSED` = 82: "Cursed"
        Cursed = 82 => "ABL_CURSED",
        /// `ABL_MAJORCURSE` = 83: "Major Curse"
        MajorCurse = 83 => "ABL_MAJORCURSE",
        /// `ABL_REMOVECURSE` = 84: "Remove Curse"
        RemoveCurse = 84 => "ABL_REMOVECURSE",
        /// `ABL_SHATTER` = 85: "Shatter"
        Shatter = 85 => "ABL_SHATTER",
        /// `ABL_QUALITY` = 86: "Quality"
        Quality = 86 => "ABL_QUALITY",
        /// `ABL_ENERGYUSE` = 87: "Alter Energy Usage"
        EnergyUse = 87 => "ABL_ENERGYUSE",
        /// `ABL_ALTERHP` = 88: "Alter HP"
        AlterHp = 88 => "ABL_ALTERHP",
        /// `ABL_PUNCHAV` = 89: "Alter Punch AV"
        PunchAv = 89 => "ABL_PUNCHAV",
        /// `ABL_KICKAV` = 90: "Alter Kick AV"
        KickAv = 90 => "ABL_KICKAV",
        /// `ABL_JUMPKICKAV` = 91: "Alter Jumpkick AV"
        JumpkickAv = 91 => "ABL_JUMPKICKAV",
        /// `ABL_PUNCHDMG` = 92: "Alter Punch DMG"
        PunchDmg = 92 => "ABL_PUNCHDMG",
        /// `ABL_KICKDMG` = 93: "Alter Kick DMG"
        KickDmg = 93 => "ABL_KICKDMG",
        /// `ABL_JUMPKICKDMG` = 94: "Alter Jumpkick DMG"
        JumpkickDmg = 94 => "ABL_JUMPKICKDMG",
        /// `ABL_SLAY` = 95: "Slay"
        Slay = 95 => "ABL_SLAY",
        /// `ABL_ENCUMBRANCE` = 96: "Alter Encumbrance"
        Encumbrance = 96 => "ABL_ENCUMBRANCE",
        /// `ABL_GOOD` = 97: "Good Aligned"
        Good = 97 => "ABL_GOOD",
        /// `ABL_EVIL` = 98: "Evil Aligned"
        Evil = 98 => "ABL_EVIL",
        /// `ABL_DRPERCENT` = 99: "Alter DR by percent"
        DrPercent = 99 => "ABL_DRPERCENT",
        /// `ABL_LOYALITEM` = 100: "Loyal Item"
        LoyalItem = 100 => "ABL_LOYALITEM",
        /// `ABL_CONFUSEMSG` = 101: "Confuse Message"
        ConfuseMsg = 101 => "ABL_CONFUSEMSG",
        /// `ABL_RACESTEALTH` = 102: "Race Stealth"
        RaceStealth = 102 => "ABL_RACESTEALTH",
        /// `ABL_CLASSSTEALTH` = 103: "Class Stealth"
        ClassStealth = 103 => "ABL_CLASSSTEALTH",
        /// `ABL_DEFENCEMOD` = 104: "Defence Modifier"
        DefenceMod = 104 => "ABL_DEFENCEMOD",
        /// `ABL_AVCURSE` = 105: "Alter AV (Curse)"
        AvCurse = 105 => "ABL_AVCURSE",
        /// `ABL_AVTHIRD` = 106: "3rd Alter Attack Value"
        AvThird = 106 => "ABL_AVTHIRD",
        /// `ABL_BLINDUSER` = 107: "Blind User"
        BlindUser = 107 => "ABL_BLINDUSER",
        /// `ABL_AFFECTSLIVING` = 108: "Affects Living"
        AffectsLiving = 108 => "ABL_AFFECTSLIVING",
        /// `ABL_NONLIVING` = 109: "Non Living"
        NonLiving = 109 => "ABL_NONLIVING",
        /// `ABL_NOTGOOD` = 110: "Not Good Aligned"
        NotGood = 110 => "ABL_NOTGOOD",
        /// `ABL_NOTEVIL` = 111: "Not Evil Aligned"
        NotEvil = 111 => "ABL_NOTEVIL",
        /// `ABL_NEUTRAL` = 112: "Neutral Aligned"
        Neutral = 112 => "ABL_NEUTRAL",
        /// `ABL_NOTNEUTRAL` = 113: "Not Neutral Aligned"
        NotNeutral = 113 => "ABL_NOTNEUTRAL",
        /// `ABL_AUTOUSE` = 114: "Auto Use Abilities"
        AutoUse = 114 => "ABL_AUTOUSE",
        /// `ABL_DESCMSG` = 115: "Descriptive Message"
        DescMsg = 115 => "ABL_DESCMSG",
        /// `ABL_BACKSTABAV` = 116: "Alter Backstab Attack Value"
        BackstabAv = 116 => "ABL_BACKSTABAV",
        /// `ABL_BACKSTABMIN` = 117: "Alter Backstab Min. Damage"
        BackstabMin = 117 => "ABL_BACKSTABMIN",
        /// `ABL_BACKSTABMAX` = 118: "Alter Backstab Max. Damage"
        BackstabMax = 118 => "ABL_BACKSTABMAX",
        /// `ABL_DELCLEANUP` = 119: "Delete at cleanup"
        DelCleanup = 119 => "ABL_DELCLEANUP",
        /// `ABL_STARTMSG` = 120: "Starting Message"
        StartMsg = 120 => "ABL_STARTMSG",
        /// `ABL_RECHARGECLEANUP` = 121: "Recharge at cleanup"
        RechargeCleanup = 121 => "ABL_RECHARGECLEANUP",
        /// `ABL_REMOVESPELL` = 122: "Remove spell"
        RemoveSpell = 122 => "ABL_REMOVESPELL",
        /// `ABL_HEALRATE` = 123: "Alter healing rate"
        HealRate = 123 => "ABL_HEALRATE",
        /// `ABL_NEGATE` = 124: "Negate ability"
        Negate = 124 => "ABL_NEGATE",
        /// `ABL_QUEST1` = 125: "Quest type one"
        Quest1 = 125 => "ABL_QUEST1",
        /// `ABL_QUEST2` = 126: "Quest type two"
        Quest2 = 126 => "ABL_QUEST2",
        /// `ABL_QUEST3` = 127: "Quest type three"
        Quest3 = 127 => "ABL_QUEST3",
        /// `ABL_QUEST4` = 128: "Quest type four"
        Quest4 = 128 => "ABL_QUEST4",
        /// `ABL_QUEST5` = 129: "Quest type five"
        Quest5 = 129 => "ABL_QUEST5",
        /// `ABL_QUEST6` = 130: "Quest type six"
        Quest6 = 130 => "ABL_QUEST6",
        /// `ABL_QUEST7` = 131: "Quest type seven"
        Quest7 = 131 => "ABL_QUEST7",
        /// `ABL_QUEST8` = 132: "Quest type eight"
        Quest8 = 132 => "ABL_QUEST8",
        /// `ABL_QUEST9` = 133: "Quest type nine"
        Quest9 = 133 => "ABL_QUEST9",
        /// `ABL_QUEST10` = 134: "Quest type ten"
        Quest10 = 134 => "ABL_QUEST10",
        /// `ABL_MINLEVEL` = 135: "Minimum level allowed"
        MinLevel = 135 => "ABL_MINLEVEL",
        /// `ABL_MAXLEVEL` = 136: "Maximum level allowed"
        MaxLevel = 136 => "ABL_MAXLEVEL",
        /// `ABL_SHOCKMSG` = 137: "Shockshield Message"
        ShockMsg = 137 => "ABL_SHOCKMSG",
        /// `ABL_VISIBLEPLACED` = 138: "Visible Placed Item"
        VisiblePlaced = 138 => "ABL_VISIBLEPLACED",
        /// `ABL_SPELLIMMUNE` = 139: "Spell Immunity"
        SpellImmune = 139 => "ABL_SPELLIMMUNE",
        /// `ABL_TELEPORTROOM` = 140: "Teleport Room"
        TeleportRoom = 140 => "ABL_TELEPORTROOM",
        /// `ABL_TELEPORTMAP` = 141: "Teleport Map"
        TeleportMap = 141 => "ABL_TELEPORTMAP",
        /// `ABL_HITMAGICAL` = 142: "Hit Magical"
        HitMagical = 142 => "ABL_HITMAGICAL",
        /// `ABL_CLEARITEM` = 143: "Clear Item"
        ClearItem = 143 => "ABL_CLEARITEM",
        /// `ABL_NONMAGICAL` = 144: "Non Magical Spell"
        NonMagical = 144 => "ABL_NONMAGICAL",
        /// `ABL_MANARATE` = 145: "Alter Mana Regeneration"
        ManaRate = 145 => "ABL_MANARATE",
        /// `ABL_GUARDED` = 146: "Guarded by Monster"
        Guarded = 146 => "ABL_GUARDED",
        /// `ABL_RESISTWATER` = 147: "Resist Water"
        ResistWater = 147 => "ABL_RESISTWATER",
        /// `ABL_TEXTBLOCK` = 148: "Trigger Text Block"
        TextBlock = 148 => "ABL_TEXTBLOCK",
        /// `ABL_DELINVCLEANUP` = 149: "Delete from inventory at cleanup"
        DelInvCleanup = 149 => "ABL_DELINVCLEANUP",
        /// `ABL_HEALMANA` = 150: "Heal Mana"
        HealMana = 150 => "ABL_HEALMANA",
        /// `ABL_ENDCAST` = 151: "Cast on ending"
        EndCast = 151 => "ABL_ENDCAST",
        /// `ABL_RUNE` = 152: "Rune"
        Rune = 152 => "ABL_RUNE",
        /// `ABL_TERMSPELL` = 153: "Terminate Spell"
        TermSpell = 153 => "ABL_TERMSPELL",
        /// `ABL_KEEPVISIBLE` = 154: "Remain Visible at cleanup"
        KeepVisible = 154 => "ABL_KEEPVISIBLE",
        /// `ABL_DEATHTEXT` = 155: "Death text block"
        DeathText = 155 => "ABL_DEATHTEXT",
        /// `ABL_QUESTITEM` = 156: "Quest item"
        QuestItem = 156 => "ABL_QUESTITEM",
        /// `ABL_SCATTER` = 157: "Scatter items"
        Scatter = 157 => "ABL_SCATTER",
        /// `ABL_REQTOHIT` = 158: "Required to hit"
        ReqToHit = 158 => "ABL_REQTOHIT",
        /// `ABL_KAIBIND` = 159: "Kai Bind"
        KaiBind = 159 => "ABL_KAIBIND",
        /// `ABL_GIVESPELL` = 160: "Give Spell (temporarily)"
        GiveSpell = 160 => "ABL_GIVESPELL",
        /// `ABL_OPENDOOR` = 161: "Open Door"
        OpenDoor = 161 => "ABL_OPENDOOR",
        /// `ABL_LORE` = 162: "Lore"
        Lore = 162 => "ABL_LORE",
        /// `ABL_COMPONENT` = 163: "Spell Component"
        Component = 163 => "ABL_COMPONENT",
        /// `ABL_ENDCASTCHANCE` = 164: "Cast on ending chance"
        EndCastChance = 164 => "ABL_ENDCASTCHANCE",
        /// `ABL_SPELLDAMAGE` = 165: "Alter spell damage"
        SpellDamage = 165 => "ABL_SPELLDAMAGE",
        /// `ABL_SPELLDURATION` = 166: "Alter spell duration"
        SpellDuration = 166 => "ABL_SPELLDURATION",
        /// `ABL_UNEQUIP` = 167: "Unequip item"
        Unequip = 167 => "ABL_UNEQUIP",
        /// `ABL_EQUIP` = 168: "Equip item"
        Equip = 168 => "ABL_EQUIP",
        /// `ABL_CANNOTWEAR` = 169: "Cannot wear location"
        CannotWear = 169 => "ABL_CANNOTWEAR",
        /// `ABL_SLEEP` = 170: "Put to sleep"
        Sleep = 170 => "ABL_SLEEP",
        /// `ABL_INVISIBLE` = 171: "Invisibility"
        Invisible = 171 => "ABL_INVISIBLE",
        /// `ABL_SEEINVISIBLE` = 172: "See Invisible"
        SeeInvisible = 172 => "ABL_SEEINVISIBLE",
        /// `ABL_SCRY` = 173: "Scry"
        Scry = 173 => "ABL_SCRY",
        /// `ABL_STEALMANA` = 174: "Steal Mana"
        StealMana = 174 => "ABL_STEALMANA",
        /// `ABL_STEALHPTOMANA` = 175: "Steal HP to Mana"
        StealHpToMana = 175 => "ABL_STEALHPTOMANA",
        /// `ABL_STEALMANATOHP` = 176: "Steal Mana to HP"
        StealManaToHp = 176 => "ABL_STEALMANATOHP",
        /// `ABL_SPELLCOLOURS` = 177: "Spell Colours"
        SpellColours = 177 => "ABL_SPELLCOLOURS",
        /// `ABL_SHADOWFORM` = 178: "Shadowform"
        Shadowform = 178 => "ABL_SHADOWFORM",
        /// `ABL_FINDTRAPSVAL` = 179: "Find traps value"
        FindTrapsVal = 179 => "ABL_FINDTRAPSVAL",
        /// `ABL_PICKLOCKSVAL` = 180: "Picklocks value"
        PicklocksVal = 180 => "ABL_PICKLOCKSVAL",
        /// `ABL_GANGDEED` = 181: "Gang House Deed"
        GangDeed = 181 => "ABL_GANGDEED",
        /// `ABL_GANGTAX` = 182: "Gang House Tax"
        GangTax = 182 => "ABL_GANGTAX",
        /// `ABL_GANGITEM` = 183: "Gang House Item"
        GangItem = 183 => "ABL_GANGITEM",
        /// `ABL_GANGSHOPITEM` = 184: "Gang Shop Item (controller)"
        GangShopItem = 184 => "ABL_GANGSHOPITEM",
        /// `ABL_NOATTACKITEM` = 185: "Do not Attack if Item Nmbr"
        NoAttackItem = 185 => "ABL_NOATTACKITEM",
        /// `ABL_PERFECTSTEALTH` = 186: "Perfect Stealth"
        PerfectStealth = 186 => "ABL_PERFECTSTEALTH",
        /// `ABL_MEDITATE` = 187: "Meditate"
        Meditate = 187 => "ABL_MEDITATE",
    }
}

c_enum! {
/// Raw session substate code, all 39 values verbatim.
///
/// The engine uses [`crate::enums::SessionState`]; this enum keeps the numbers for log lines
/// and for reading the stored codes.
///
/// `SUBST_` values.
    Substate {
        /// `SUBST_START` = 0: first call after selecting MajorMUD
        Start = 0 => "SUBST_START",
        /// `SUBST_PLAY_ALT` = 1: handled as SUBST_PLAYING; nothing in 1.11p sets it (purpose unknown)
        PlayAlt = 1 => "SUBST_PLAY_ALT",
        /// `SUBST_SYSOP_MENU` = 11: sysop menu (\[MAJORMUD Sysop\] prompt)
        SysopMenu = 11 => "SUBST_SYSOP_MENU",
        /// `SUBST_UPDATE_FILE` = 12: sysop: name of the update file?
        UpdateFile = 12 => "SUBST_UPDATE_FILE",
        /// `SUBST_UPDATING` = 13: database update running (polling)
        Updating = 13 => "SUBST_UPDATING",
        /// `SUBST_CLEARING` = 14: clearing evil points or profiles, X
        Clearing = 14 => "SUBST_CLEARING",
        /// `SUBST_REINDEXING` = 15: reindexing limited items, X stops
        Reindexing = 15 => "SUBST_REINDEXING",
        /// `SUBST_XREF_USERS` = 16: checking for deleted users, X stops
        XrefUsers = 16 => "SUBST_XREF_USERS",
        /// `SUBST_XREF_DELETE` = 17: "Delete this user?" (Y = delete)
        XrefDelete = 17 => "SUBST_XREF_DELETE",
        /// `SUBST_XREF_STRIP` = 18: "Remove ALL limited items from this user?" (Y = remove)
        XrefStrip = 18 => "SUBST_XREF_STRIP",
        /// `SUBST_XREF_ITEMS` = 19: checking users with limited items
        XrefItems = 19 => "SUBST_XREF_ITEMS",
        /// `SUBST_VALIDATING` = 20: new name being checked (polling)
        Validating = 20 => "SUBST_VALIDATING",
        /// `SUBST_NEW_NAME` = 21: name refused: new name, or X to exit
        NewName = 21 => "SUBST_NEW_NAME",
        /// `SUBST_PURGE_MINUTES` = 22: sysop: new defunct minutes limit
        PurgeMinutes = 22 => "SUBST_PURGE_MINUTES",
        /// `SUBST_PURGE_DAYS` = 23: sysop: new days-not-played limit
        PurgeDays = 23 => "SUBST_PURGE_DAYS",
        /// `SUBST_PURGE_EXP` = 24: sysop: new experience limit
        PurgeExp = 24 => "SUBST_PURGE_EXP",
        /// `SUBST_ADDON_NUMBER` = 25: sysop: add code for which add-on?
        AddonNumber = 25 => "SUBST_ADDON_NUMBER",
        /// `SUBST_ADDON_CODE` = 26: sysop: enter the add-on code given
        AddonCode = 26 => "SUBST_ADDON_CODE",
        /// `SUBST_NEW_RACE` = 51: new character: choose a race
        NewRace = 51 => "SUBST_NEW_RACE",
        /// `SUBST_NEW_CLASS` = 52: new character: choose a class
        NewClass = 52 => "SUBST_NEW_CLASS",
        /// `SUBST_NEW_CHAR` = 54: new character: show the race list; nothing in 1.11p sets it
        NewChar = 54 => "SUBST_NEW_CHAR",
        /// `SUBST_MAIN_MENU` = 56: main menu (\[MAJORMUD\] prompt)
        MainMenu = 56 => "SUBST_MAIN_MENU",
        /// `SUBST_HELP` = 57: main menu: "Enter Help Topic"
        Help = 57 => "SUBST_HELP",
        /// `SUBST_NEW_LAWFUL` = 58: new character: "Do you want to be Lawful?" (Y/N)
        NewLawful = 58 => "SUBST_NEW_LAWFUL",
        /// `SUBST_VACATION` = 59: vacation menu
        Vacation = 59 => "SUBST_VACATION",
        /// `SUBST_CLIENT_INFO` = 60: developer mode: awaits the reply to MUDCHARINFO, then SUBST_ENTER
        ClientInfo = 60 => "SUBST_CLIENT_INFO",
        /// `SUBST_ENTER` = 61: entering: set the channel up, show the main menu (passed through)
        Enter = 61 => "SUBST_ENTER",
        /// `SUBST_CHG_READHELP` = 70: race change: read the help files?
        ChgReadHelp = 70 => "SUBST_CHG_READHELP",
        /// `SUBST_CHG_TIMELIMIT` = 71: race change: short time to undo?
        ChgTimeLimit = 71 => "SUBST_CHG_TIMELIMIT",
        /// `SUBST_CHG_NOSYSOP` = 72: race change: sysop cannot undo it?
        ChgNoSysop = 72 => "SUBST_CHG_NOSYSOP",
        /// `SUBST_CHG_RACE` = 73: race change: choose the new race
        ChgRace = 73 => "SUBST_CHG_RACE",
        /// `SUBST_CHG_CLASS` = 74: race change: choose the new class
        ChgClass = 74 => "SUBST_CHG_CLASS",
        /// `SUBST_PLAYING` = 130: in the realm: input is a command; every value from here up is in it
        Playing = 130 => "SUBST_PLAYING",
        /// `SUBST_LISTING` = 131: TOPTEN or gang list paging, key ends
        Listing = 131 => "SUBST_LISTING",
        /// `SUBST_SUICIDE_OLDPW` = 132: SET SUICIDE: current password
        SuicideOldPw = 132 => "SUBST_SUICIDE_OLDPW",
        /// `SUBST_SUICIDE_NEWPW` = 133: SET SUICIDE: new suicide password
        SuicideNewPw = 133 => "SUBST_SUICIDE_NEWPW",
        /// `SUBST_SUICIDE` = 134: SUICIDE: enter the suicide password
        Suicide = 134 => "SUBST_SUICIDE",
        /// `SUBST_USER_REPORT` = 135: sysop user report paging, key stops
        UserReport = 135 => "SUBST_USER_REPORT",
        /// `SUBST_DISBAND` = 136: "Are you sure you want to disband?"
        Disband = 136 => "SUBST_DISBAND",
    }
}

c_enum! {
/// Activity code of the developer status line.
///
/// `UST_` values.
    SysopStatus {
        /// `UST_STARTUP` = 0: loading and generating buffers
        Startup = 0 => "UST_STARTUP",
        /// `UST_MIDNIGHT` = 1: midnight cleanup
        Midnight = 1 => "UST_MIDNIGHT",
        /// `UST_COMMAND` = 2: executing a game command
        Command = 2 => "UST_COMMAND",
        /// `UST_AUTOPLAY` = 3: generating autoplay input
        Autoplay = 3 => "UST_AUTOPLAY",
        /// `UST_ENERGY` = 4: background: energy cycle
        Energy = 4 => "UST_ENERGY",
        /// `UST_SLOW` = 5: background: slow cycle
        Slow = 5 => "UST_SLOW",
        /// `UST_MEDIUM` = 6: background: medium cycle
        Medium = 6 => "UST_MEDIUM",
        /// `UST_FAST` = 7: background: fast cycle
        Fast = 7 => "UST_FAST",
        /// `UST_OTHER` = 8: background: other cycle
        Other = 8 => "UST_OTHER",
        /// `UST_MONCREATE` = 9: background: monster creation
        MonCreate = 9 => "UST_MONCREATE",
        /// `UST_SAVEBUFS` = 10: background: saving buffers
        SaveBufs = 10 => "UST_SAVEBUFS",
        /// `UST_IDLE` = 11: done: back to the kernel
        Idle = 11 => "UST_IDLE",
        /// `UST_RESTOCK` = 12: restocking shops
        Restock = 12 => "UST_RESTOCK",
        /// `UST_POLLING` = 13: in-game polling
        Polling = 13 => "UST_POLLING",
        /// `UST_VALIDATE` = 14: new name being checked (polling)
        Validate = 14 => "UST_VALIDATE",
        /// `UST_LOGON` = 15: user logon
        Logon = 15 => "UST_LOGON",
        /// `UST_LOGOFF` = 16: user logoff
        Logoff = 16 => "UST_LOGOFF",
        /// `UST_AUTOUPDATE` = 17: automatic database update polling
        AutoUpdate = 17 => "UST_AUTOUPDATE",
        /// `UST_UPDATE` = 18: database update polling
        Update = 18 => "UST_UPDATE",
        /// `UST_INPUT` = 19: handling user input
        Input = 19 => "UST_INPUT",
    }
}

c_enum! {
/// A channel's sysop debug level.
///
/// `DBG_` values.
    DebugLevel {
        /// `DBG_OFF` = 0: "off"
        Off = 0 => "DBG_OFF",
        /// `DBG_ON` = 1: "on"
        On = 1 => "DBG_ON",
        /// `DBG_PARTIAL` = 2: "partial"
        Partial = 2 => "DBG_PARTIAL",
        /// `DBG_BUFFERS` = 3: "buffers": buffer statistics (inf)
        Buffers = 3 => "DBG_BUFFERS",
    }
}

c_enum! {
/// Progress of a client channel.
///
/// `UIG_` values.
    InGame {
        /// `UIG_OUTSIDE` = 0: not entered, or has left
        Outside = 0 => "UIG_OUTSIDE",
        /// `UIG_MODULE` = 1: in the module, not the realm (inf)
        Module = 1 => "UIG_MODULE",
        /// `UIG_REALM` = 2: in the realm
        Realm = 2 => "UIG_REALM",
    }
}

c_enum! {
/// A channel's escape sequence parser state.
///
/// `ESS_` values.
    EscState {
        /// `ESS_NONE` = 0: no escape sequence in progress
        None = 0 => "ESS_NONE",
        /// `ESS_ESC` = 1: ESC received
        Esc = 1 => "ESS_ESC",
        /// `ESS_CSI` = 2: ESC \[ received, final letter next
        Csi = 2 => "ESS_CSI",
    }
}

c_enum! {
/// Command code: what a typed command word parses to. Codes 0-9 equal the [`Direction`] values.
///
/// `CMD_` values.
    CommandCode {
        /// `CMD_NORTH` = 0: N, NORTH
        North = 0 => "CMD_NORTH",
        /// `CMD_SOUTH` = 1: S, SOUTH
        South = 1 => "CMD_SOUTH",
        /// `CMD_EAST` = 2: E, EAS\[T\]
        East = 2 => "CMD_EAST",
        /// `CMD_WEST` = 3: W, WEST
        West = 3 => "CMD_WEST",
        /// `CMD_NE` = 4: NE, NORTHE\[AST\]
        NorthEast = 4 => "CMD_NE",
        /// `CMD_NW` = 5: NW, NORTHW\[EST\]
        NorthWest = 5 => "CMD_NW",
        /// `CMD_SE` = 6: SE, SOUTHE\[AST\]
        SouthEast = 6 => "CMD_SE",
        /// `CMD_SW` = 7: SW, SOUTHW\[EST\]
        SouthWest = 7 => "CMD_SW",
        /// `CMD_UP` = 8: U\[P\]
        Up = 8 => "CMD_UP",
        /// `CMD_DOWN` = 9: D, DOW\[N\]
        Down = 9 => "CMD_DOWN",
        /// `CMD_NONE` = 10: no command matched
        None = 10 => "CMD_NONE",
        /// `CMD_ATTACK` = 11: A, AT\[TACK\], AU
        Attack = 11 => "CMD_ATTACK",
        /// `CMD_HELP` = 12: ?, HEL\[P\]
        Help = 12 => "CMD_HELP",
        /// `CMD_SAVE` = 13: SA\[VE\]: refused, with a message
        Save = 13 => "CMD_SAVE",
        /// `CMD_LOOK` = 14: L\[OOK\]
        Look = 14 => "CMD_LOOK",
        /// `CMD_SOUND` = 15: SOUN\[D\]: parsed but never acted on
        Sound = 15 => "CMD_SOUND",
        /// `CMD_MAP` = 16: MA\[P\]
        Map = 16 => "CMD_MAP",
        /// `CMD_STATUS` = 17: ST\[ATUS\]
        Status = 17 => "CMD_STATUS",
        /// `CMD_HEALTH` = 18: HE\[ALTH\]
        Health = 18 => "CMD_HEALTH",
        /// `CMD_INVENTORY` = 19: I, INVE\[NTORY\]
        Inventory = 19 => "CMD_INVENTORY",
        /// `CMD_SPELLS` = 20: SP\[ELLS\]
        Spells = 20 => "CMD_SPELLS",
        /// `CMD_LIGHT` = 21: LIG\[HT\]
        Light = 21 => "CMD_LIGHT",
        /// `CMD_USE` = 22: US\[E\]
        Use = 22 => "CMD_USE",
        /// `CMD_KEYS` = 23: KE\[YS\]
        Keys = 23 => "CMD_KEYS",
        /// `CMD_EAT` = 24: EAT
        Eat = 24 => "CMD_EAT",
        /// `CMD_DRINK` = 25: DRI\[NK\]
        Drink = 25 => "CMD_DRINK",
        /// `CMD_GET` = 26: G\[ET\]
        Get = 26 => "CMD_GET",
        /// `CMD_DROP` = 27: DRO\[P\]
        Drop = 27 => "CMD_DROP",
        /// `CMD_TRACK` = 28: TRAC\[K\]
        Track = 28 => "CMD_TRACK",
        /// `CMD_TAME` = 29: TAM\[E\]
        Tame = 29 => "CMD_TAME",
        /// `CMD_MESMERIZE` = 30: ME\[SMERIZE\]
        Mesmerize = 30 => "CMD_MESMERIZE",
        /// `CMD_CAST` = 31: C\[AST\]
        Cast = 31 => "CMD_CAST",
        /// `CMD_GIVE` = 32: GI\[VE\]
        Give = 32 => "CMD_GIVE",
        /// `CMD_THROW` = 33: TH\[ROW\]: parsed but never acted on
        Throw = 33 => "CMD_THROW",
        /// `CMD_FIRE` = 34: FI\[RE\]: parsed but never acted on
        Fire = 34 => "CMD_FIRE",
        /// `CMD_ROB` = 35: RO\[B\]
        Rob = 35 => "CMD_ROB",
        /// `CMD_LIST` = 36: LIS\[T\]
        List = 36 => "CMD_LIST",
        /// `CMD_BUY` = 37: BU\[Y\], ORD\[ER\]
        Buy = 37 => "CMD_BUY",
        /// `CMD_SELL` = 38: SEL\[L\]
        Sell = 38 => "CMD_SELL",
        /// `CMD_DONATE` = 39: DON\[ATE\]: parsed but never acted on
        Donate = 39 => "CMD_DONATE",
        /// `CMD_PLAY` = 40: PL\[AY\]: parsed but never acted on
        Play = 40 => "CMD_PLAY",
        /// `CMD_WHISPER` = 41: /..., WHI\[SPER\]
        Whisper = 41 => "CMD_WHISPER",
        /// `CMD_JOIN` = 42: JO\[IN\]
        Join = 42 => "CMD_JOIN",
        /// `CMD_BROADCAST` = 43: '..., -..., BR\[OADCAST\]
        Broadcast = 43 => "CMD_BROADCAST",
        /// `CMD_SUICIDE` = 44: SUICIDE
        Suicide = 44 => "CMD_SUICIDE",
        /// `CMD_READY` = 45: REA, READY
        Ready = 45 => "CMD_READY",
        /// `CMD_WEAR` = 46: WEA\[R\]
        Wear = 46 => "CMD_WEAR",
        /// `CMD_UNREADY` = 47: UNR\[EADY\]
        Unready = 47 => "CMD_UNREADY",
        /// `CMD_FOLLOW` = 48: FO\[LLOW\]
        Follow = 48 => "CMD_FOLLOW",
        /// `CMD_REROLL` = 49: REROLL
        Reroll = 49 => "CMD_REROLL",
        /// `CMD_IDENTITY` = 50: ID\[ENTITY\]
        Identity = 50 => "CMD_IDENTITY",
        /// `CMD_TOPTEN` = 51: TO\[PTEN\]
        Topten = 51 => "CMD_TOPTEN",
        /// `CMD_QUIET` = 52: QU\[IET\]
        Quiet = 52 => "CMD_QUIET",
        /// `CMD_VERBOSE` = 53: VERB...
        Verbose = 53 => "CMD_VERBOSE",
        /// `CMD_SHOOT` = 54: SHO\[OT\]: parsed but never acted on
        Shoot = 54 => "CMD_SHOOT",
        /// `CMD_TAKE` = 55: TAK\[E\]: parsed but never acted on
        Take = 55 => "CMD_TAKE",
        /// `CMD_LEAVE` = 56: LE\[AVE\]
        Leave = 56 => "CMD_LEAVE",
        /// `CMD_TALK` = 57: TAL\[K\]
        Talk = 57 => "CMD_TALK",
        /// `CMD_SCAN` = 58: SC\[AN\], WHO
        Scan = 58 => "CMD_SCAN",
        /// `CMD_STOCK` = 59: STO\[CK\]
        Stock = 59 => "CMD_STOCK",
        /// `CMD_SMASH` = 60: SMA\[SH\]
        Smash = 60 => "CMD_SMASH",
        /// `CMD_KILLBLOW` = 61: KIL\[LLBLO\] (sic)
        Killblow = 61 => "CMD_KILLBLOW",
        /// `CMD_BACKSTAB` = 62: BS, BACKS\[TAB\]
        Backstab = 62 => "CMD_BACKSTAB",
        /// `CMD_GUARD` = 63: GU\[ARD\]
        Guard = 63 => "CMD_GUARD",
        /// `CMD_SNEAK` = 64: SN\[EAK\]
        Sneak = 64 => "CMD_SNEAK",
        /// `CMD_HIDE` = 65: HID\[E\]
        Hide = 65 => "CMD_HIDE",
        /// `CMD_PICKLOCK` = 66: PI\[CKLOCK\]
        PickLock = 66 => "CMD_PICKLOCK",
        /// `CMD_SEARCH` = 67: SEA\[RCH\]
        Search = 67 => "CMD_SEARCH",
        /// `CMD_DISARM` = 68: DI\[SARM\]
        Disarm = 68 => "CMD_DISARM",
        /// `CMD_REMOVE` = 69: REM\[OVE\]
        Remove = 69 => "CMD_REMOVE",
        /// `CMD_REST` = 70: REST: rest
        Rest = 70 => "CMD_REST",
        /// `CMD_BRIEF` = 71: BRI\[EF\]
        Brief = 71 => "CMD_BRIEF",
        /// `CMD_TEXT` = 72: TE\[XT\]: parsed but never acted on
        Text = 72 => "CMD_TEXT",
        /// `CMD_EXITS` = 73: EXI\[TS\]: exit list
        Exits = 73 => "CMD_EXITS",
        /// `CMD_STATLINE` = 74: STATL\[INE\]: status line
        StatLine = 74 => "CMD_STATLINE",
        /// `CMD_PROFILE` = 75: PR\[OFILE\]
        Profile = 75 => "CMD_PROFILE",
        /// `CMD_DEPOSIT` = 76: DEP\[OSIT\]
        Deposit = 76 => "CMD_DEPOSIT",
        /// `CMD_WITHDRAW` = 77: WIT\[HDRAW\]
        Withdraw = 77 => "CMD_WITHDRAW",
        /// `CMD_BORROW` = 78: BO\[RROW\]
        Borrow = 78 => "CMD_BORROW",
        /// `CMD_REPAY` = 79: REP\[AY\]
        Repay = 79 => "CMD_REPAY",
        /// `CMD_BANKBOOK` = 80: BAL\[ANCE\], BAN\[KBOOK\]
        BankBook = 80 => "CMD_BANKBOOK",
        /// `CMD_OPEN` = 81: OP\[EN\]
        Open = 81 => "CMD_OPEN",
        /// `CMD_CLOSE` = 82: CL\[OSE\]
        Close = 82 => "CMD_CLOSE",
        /// `CMD_BREAK` = 83: BRE\[AK\]: stop dragging
        Break = 83 => "CMD_BREAK",
        /// `CMD_AID` = 84: AI\[D\]
        Aid = 84 => "CMD_AID",
        /// `CMD_DRAG` = 85: DRA\[G\]
        Drag = 85 => "CMD_DRAG",
        /// `CMD_AUTOCOMBAT` = 86: AUT\[OCOMBAT\]: parsed but never acted on
        Autocombat = 86 => "CMD_AUTOCOMBAT",
        /// `CMD_SYSOP` = 87: SY\[SOP\]
        Sysop = 87 => "CMD_SYSOP",
        /// `CMD_BASH` = 88: AA, AL\[LOUT\], BAS\[H\], FORC\[E\]
        Bash = 88 => "CMD_BASH",
        /// `CMD_DEFEND` = 89: DEF\[END\]: parsed but never acted on
        Defend = 89 => "CMD_DEFEND",
        /// `CMD_KICK` = 90: KIC\[K\]
        Kick = 90 => "CMD_KICK",
        /// `CMD_EQUIP` = 91: EQ\[UIP\]
        Equip = 91 => "CMD_EQUIP",
        /// `CMD_INVITE` = 92: INVI\[TE\]
        Invite = 92 => "CMD_INVITE",
        /// `CMD_UNINVITE` = 93: UNI\[NVITE\]
        Uninvite = 93 => "CMD_UNINVITE",
        /// `CMD_BACKRANK` = 94: BACKR\[ANK\]
        BackRank = 94 => "CMD_BACKRANK",
        /// `CMD_FRONTRANK` = 95: FR\[ONTRANK\]
        FrontRank = 95 => "CMD_FRONTRANK",
        /// `CMD_STASH` = 96: STAS\[H\]
        Stash = 96 => "CMD_STASH",
        /// `CMD_PUNCH` = 97: PU, PUN\[CH\], PUR\[G\]
        Punch = 97 => "CMD_PUNCH",
        /// `CMD_ARM` = 98: AR\[M\]
        Arm = 98 => "CMD_ARM",
        /// `CMD_WIELD` = 99: WIE\[LD\]
        Wield = 99 => "CMD_WIELD",
        /// `CMD_TRAIN` = 100: TRAI\[N\]
        Train = 100 => "CMD_TRAIN",
        /// `CMD_PARTY` = 101: PAR\[TY\]
        Party = 101 => "CMD_PARTY",
        /// `CMD_PAGE` = 102: PAG\[E\]: page
        Page = 102 => "CMD_PAGE",
        /// `CMD_LOCK` = 103: LOC\[K\]
        Lock = 103 => "CMD_LOCK",
        /// `CMD_SET` = 104: SET
        Set = 104 => "CMD_SET",
        /// `CMD_JUMPKICK` = 105: JU\[MPKICK\]
        Jumpkick = 105 => "CMD_JUMPKICK",
        /// `CMD_EXPERIENCE` = 106: EXP\[ERIENCE\]
        Experience = 106 => "CMD_EXPERIENCE",
        /// `CMD_POWERS` = 107: PO\[WERS\]
        Powers = 107 => "CMD_POWERS",
        /// `CMD_INVOKE` = 108: INVO\[KE\]
        Invoke = 108 => "CMD_INVOKE",
        /// `CMD_HUNT` = 109: HU\[NT\]: parsed but never acted on
        Hunt = 109 => "CMD_HUNT",
        /// `CMD_READ` = 110: READ
        Read = 110 => "CMD_READ",
        /// `CMD_DISMISS` = 111: DISM\[ISS\]: parsed but never acted on
        Dismiss = 111 => "CMD_DISMISS",
        /// `CMD_CREATE` = 112: CR\[EATE\]
        Create = 112 => "CMD_CREATE",
        /// `CMD_FEEDBACK` = 113: FE\[EDBACK\]
        Feedback = 113 => "CMD_FEEDBACK",
        /// `CMD_BROADGANG` = 114: BG, GB, BROADG\[ANG\]
        Broadgang = 114 => "CMD_BROADGANG",
        /// `CMD_ASK` = 115: AS\[K\]
        Ask = 115 => "CMD_ASK",
        /// `CMD_GREET` = 116: GR\[EET\]
        Greet = 116 => "CMD_GREET",
        /// `CMD_DISBAND` = 117: DISB\[AND\]
        Disband = 117 => "CMD_DISBAND",
        /// `CMD_RESEARCH` = 118: RESE\[ARCH\]: parsed but never acted on
        Research = 118 => "CMD_RESEARCH",
        /// `CMD_FORGIVE` = 119: FORG\[IVE\]
        Forgive = 119 => "CMD_FORGIVE",
        /// `CMD_MIDRANK` = 120: MI\[DRANK\]
        Midrank = 120 => "CMD_MIDRANK",
        /// `CMD_QUIT` = 121: Q, QUIT
        Quit = 121 => "CMD_QUIT",
        /// `CMD_SHARE` = 122: SHA\[RE\]
        Share = 122 => "CMD_SHARE",
        /// `CMD_WEALTH` = 123: WEAL\[TH\]
        Wealth = 123 => "CMD_WEALTH",
        /// `CMD_ACTION` = 124: AC\[TION\]
        Action = 124 => "CMD_ACTION",
        /// `CMD_DIRECTMSG` = 125: >...
        DirectMsg = 125 => "CMD_DIRECTMSG",
        /// `CMD_GOSSIP` = 126: GOS\[SIP\]
        Gossip = 126 => "CMD_GOSSIP",
        /// `CMD_APPRAISE` = 127: AP\[PRAISE\]
        Appraise = 127 => "CMD_APPRAISE",
        /// `CMD_AUCTION` = 128: AUC\[TION\]
        Auction = 128 => "CMD_AUCTION",
        /// `CMD_PURGE` = 129: PURGE
        Purge = 129 => "CMD_PURGE",
        /// `CMD_GLOBAL` = 130: GLO\[BAL\]
        Global = 130 => "CMD_GLOBAL",
        /// `CMD_FORGET` = 131: FORGE\[T\]
        Forget = 131 => "CMD_FORGET",
        /// `CMD_PROMOTE` = 132: PROM\[OTE\]
        Promote = 132 => "CMD_PROMOTE",
        /// `CMD_DEMOTE` = 133: DEM\[OTE\]
        Demote = 133 => "CMD_DEMOTE",
        /// `CMD_UNSTOCK` = 134: UNS\[TOCK\]
        Unstock = 134 => "CMD_UNSTOCK",
        /// `CMD_MARKUP` = 135: MAR\[KUP\]
        MarkUp = 135 => "CMD_MARKUP",
        /// `CMD_MEDITATE` = 136: MED\[ITATE\]: meditate
        Meditate = 136 => "CMD_MEDITATE",
        /// `CMD_VERSION` = 137: VERS...: version
        Version = 137 => "CMD_VERSION",
    }
}

c_enum! {
/// Hang-up punishment level (`PUNHUP` option).
///
/// `PUN_` values.
    HangupPunish {
        /// `PUN_HIGH` = 1: punish every hang-up (also what an unknown option value becomes)
        High = 1 => "PUN_HIGH",
        /// `PUN_MEDIUM` = 2: only in auto-combat or under attack
        Medium = 2 => "PUN_MEDIUM",
        /// `PUN_LOW` = 3: only in player-vs-player combat
        Low = 3 => "PUN_LOW",
        /// `PUN_NONE` = 4: never punish a hang-up
        None = 4 => "PUN_NONE",
    }
}

/// A channel's user list rank sentinels. `ULS_` values.
pub mod uls {
    /// `ULS_NOTPLAYING`: not in the game
    pub const NOTPLAYING: i16 = -1;
    /// `ULS_UNRANKED`: in the game, not yet ranked
    pub const UNRANKED: i16 = -2;
}

/// Game-wide message mask: 0, or the player flag that blocks the message. `TGM_` values.
pub mod tgm {
    /// `TGM_ALL`: everyone in the game hears it; the other masks are PF_NOENTRYMSG, PF_NOGOSSIP and PF_NOAUCTION (the last two are also capped per listener by the gossip and auction limits)
    pub const ALL: u16 = 0;
}

/// Room message noise: how minor a room message is (compare [`MessageLevel`]). `TRN_` values.
pub mod trn {
    /// `TRN_ALWAYS`: every listener gets it
    pub const ALWAYS: u16 = 0;
    /// `TRN_MEDIUM`: not for players at MLV_LOW
    pub const MEDIUM: u16 = 10;
    /// `TRN_HIGH`: only for players at MLV_HIGH
    pub const HIGH: u16 = 20;
}

/// Room message sight: may blinded listeners get it? `TRB_` values.
pub mod trb {
    /// `TRB_SIGHT`: no: only seen (skips ST_BLIND)
    pub const SIGHT: u16 = 0;
    /// `TRB_ALL`: yes: blinded players get it too
    pub const ALL: u16 = 1;
}

/// Game and room message prompt and colour arguments. `TELL_` values.
pub mod tell {
    /// `TELL_NOPROMPT`: message only
    pub const NOPROMPT: u16 = 0;
    /// `TELL_PROMPT`: follow with the listener's prompt
    pub const PROMPT: u16 = 1;
    /// `TELL_NOCOLOR`: no colour prefix (above every colour row)
    pub const NOCOLOR: u16 = 255;
}

/// Colour table rows indexed directly by the display code. `COL_` values.
pub mod col {
    /// `COL_PROMPT`: prompt text; ends a coloured value
    pub const PROMPT: u16 = 2;
    /// `COL_PROMPTVAL`: prompt values; hit points above 1/2
    pub const PROMPTVAL: u16 = 3;
    /// `COL_HPHURT`: hit points at 1/2 or less, above 1/4
    pub const HPHURT: u16 = 4;
    /// `COL_HPLOW`: hit points at 1/4 or less
    pub const HPLOW: u16 = 5;
    /// `COL_MOVENAME`: name of a player coming or going
    pub const MOVENAME: u16 = 9;
    /// `COL_LINEEND`: end of a movement line (CR)
    pub const LINEEND: u16 = 10;
    /// `COL_MOVETEXT`: "just left to", "walks into the"
    pub const MOVETEXT: u16 = 11;
    /// `COL_ACTION`: social action seen in the room
    pub const ACTION: u16 = 12;
    /// `COL_BROADCAST`: gossip and auction lines
    pub const BROADCAST: u16 = 14;
    /// `COL_EXITS`: "Obvious exits:" label
    pub const EXITS: u16 = 0;
    /// `COL_ALSOHERE`: "Also here:" label and closing "."
    pub const ALSOHERE: u16 = 1;
    /// `COL_SPELLGEN`: messages of SPT_GENERAL spells
    pub const SPELLGEN: u16 = 6;
    /// `COL_SPELLOTH`: messages of all other spell types
    pub const SPELLOTH: u16 = 7;
    /// `COL_HERENAMES`: names listed after "Also here:"
    pub const HERENAMES: u16 = 8;
    /// `COL_TELEPATH`: telepathed words
    pub const TELEPATH: u16 = 13;
}
// ---------------------------------------------------------------------------------------------
// Display words. Each family indexes its fixed name table by its code.

macro_rules! table_name {
    ($($ty:ident => $table:ident, $doc:literal;)*) => {
        $(
            impl $ty {
                #[doc = $doc]
                pub fn name(self) -> Option<&'static str> {
                    crate::names::$table.get(usize::from(self.code())).copied()
                }
            }
        )*
    };
}

table_name! {
    ExitType => EXIT_TYPES, "The [`crate::names::EXIT_TYPES`] word for this exit type.";
    RoomType => ROOM_TYPES, "The [`crate::names::ROOM_TYPES`] word for this room type.";
    GangHouse => GANG_HOUSES, "The [`crate::names::GANG_HOUSES`] name of this gang house.";
    ShopType => SHOP_TYPES, "The [`crate::names::SHOP_TYPES`] word for this shop type.";
    WornLocation => WORN_LOCATIONS, "The [`crate::names::WORN_LOCATIONS`] word for this wear location.";
    WeaponType => WEAPON_TYPES, "The [`crate::names::WEAPON_TYPES`] word for this weapon type.";
    ArmourType => ARMOUR_TYPES, "The [`crate::names::ARMOUR_TYPES`] word for this armour type.";
    WeaponClass => WEAPON_CLASSES, "The [`crate::names::WEAPON_CLASSES`] words for this weapon class.";
    MonsterType => MONSTER_TYPES, "The [`crate::names::MONSTER_TYPES`] word for this monster type.";
    Alignment => ALIGNMENTS, "The [`crate::names::ALIGNMENTS`] word for this alignment.";
    GroupRole => FOLLOW_TYPES, "The [`crate::names::FOLLOW_TYPES`] word for this group role.";
    Gender => PRONOUNS, "The [`crate::names::PRONOUNS`] pronoun for this gender, capitalised.";
    SpellType => SPELL_TYPES, "The [`crate::names::SPELL_TYPES`] word for this spell type.";
    SpellTarget => SPELL_TARGETS, "The [`crate::names::SPELL_TARGETS`] words for this spell target.";
    Magery => MAGERY_TYPES, "The [`crate::names::MAGERY_TYPES`] word for this magic school.";
    Element => RESIST_TYPES, "The [`crate::names::RESIST_TYPES`] word for this element (\"Lightening\", sic).";
    ResistStat => STAT_NAMES, "The [`crate::names::STAT_NAMES`] word for this stat.";
    Material => MATERIALS, "The [`crate::names::MATERIALS`] word for this material.";
    OwnerKind => OWNER_TYPES, "The [`crate::names::OWNER_TYPES`] word for this owner kind.";
    Ability => ABILITY_NAMES, "The [`crate::names::ABILITY_NAMES`] text for this ability.";
}

impl ItemType {
    /// The word the original prints for this item type: [`crate::names::ITEM_TYPES`] at the
    /// type's code, shifted (sic).
    ///
    /// `ItemType::Projectile` shows as "Shield" and every later type as the name of the type
    /// before it; type 11 would show "General".
    pub fn display_name(self) -> Option<&'static str> {
        crate::names::ITEM_TYPES
            .get(usize::from(self.code()))
            .copied()
    }
}

impl Currency {
    /// The plural name; `Runic` gives the placeholder the config replaces.
    pub fn plural_name(self) -> Option<&'static str> {
        crate::names::CURRENCY_NAMES
            .get(usize::from(self.code()))
            .copied()
    }

    /// The singular name; `Undefined` gives "Undefined".
    pub fn singular_name(self) -> Option<&'static str> {
        crate::names::CURRENCY_NAMES_SINGULAR
            .get(usize::from(self.code()))
            .copied()
    }
}

impl Direction {
    /// The ten directions in `DIR_` order (`NDIRS`).
    pub const COMPASS: [Direction; 10] = [
        Direction::North,
        Direction::South,
        Direction::East,
        Direction::West,
        Direction::NorthEast,
        Direction::NorthWest,
        Direction::SouthEast,
        Direction::SouthWest,
        Direction::Up,
        Direction::Down,
    ];

    /// The way back; `None` for an unknown direction.
    pub fn reverse(self) -> Option<Direction> {
        crate::names::REVERSE_DIRECTIONS
            .get(usize::from(self.code()))
            .copied()
    }

    /// The short command: "N", "NE", "U", ...
    pub fn abbreviation(self) -> Option<&'static str> {
        crate::names::DIRECTION_ABBREVIATIONS
            .get(usize::from(self.code()))
            .copied()
    }
}

/// Ability numbers are what the records store; the name reads better at use sites.
pub type AbilityNo = Ability;

// ---------------------------------------------------------------------------------------------
// Session state machine.

/// Where a session is in the dialogue: the `SUBST_*` substates, shaped.
///
/// The raw [`Substate`] minus the states silvermere drops:
/// `SUBST_ADDON_NUMBER` 25 and `SUBST_ADDON_CODE` 26 (add-on licence codes),
/// `SUBST_VACATION` 59 and `SUBST_CLIENT_INFO` 60 (developer client).
/// [`SessionState::substt`] gives the original number for log lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SessionState {
    /// `SUBST_START` 0: first call after selecting the game.
    Start,
    /// `SUBST_ENTER` 61: set the session up and show the main menu.
    Enter,
    /// `SUBST_MAIN_MENU` 56: the `[MAJORMUD]:` menu.
    MainMenu,
    /// `SUBST_HELP` 57: "Enter Help Topic".
    Help,
    /// Creating a new character.
    NewChar(NewCharStep),
    /// The one-time race and class change.
    RaceChange(RaceChangeStep),
    /// The sysop menu and its sub-prompts.
    SysopMenu(SysopStep),
    /// `SUBST_PLAYING` 130: in the realm, input is a command. `SUBST_PLAY_ALT` 1, which
    /// behaves the same and is never seen in 1.11p, also decodes to this.
    Playing,
    /// `SUBST_LISTING` 131: TOPTEN or gang list paging; a key ends it.
    Listing,
    /// SET SUICIDE and SUICIDE password prompts.
    Suicide(SuicideStep),
    /// `SUBST_USER_REPORT` 135: sysop user report paging.
    UserReport,
    /// `SUBST_DISBAND` 136: "Are you sure you want to disband?".
    Disband,
}

/// The steps of character creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum NewCharStep {
    /// `SUBST_NEW_RACE` 51: choose a race. `SUBST_NEW_CHAR` 54 (show the race list, never set
    /// in 1.11p) decodes to this.
    Race,
    /// `SUBST_NEW_CLASS` 52: choose a class.
    Class,
    /// `SUBST_NEW_LAWFUL` 58: "Do you want to be Lawful?".
    Lawful,
    /// The full-screen stats editor, opened from the class or lawful answer. It has no substate
    /// code of its own; it reports `SUBST_NEW_LAWFUL` 58.
    Stats,
    /// `SUBST_VALIDATING` 20: the new name is being checked.
    Validating,
    /// `SUBST_NEW_NAME` 21: name refused; enter a new one or X.
    Name,
}

/// The steps of the one-time race change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum RaceChangeStep {
    /// `SUBST_CHG_READHELP` 70: read the help files?
    ReadHelp,
    /// `SUBST_CHG_TIMELIMIT` 71: short time to undo?
    TimeLimit,
    /// `SUBST_CHG_NOSYSOP` 72: the sysop cannot undo it?
    NoSysop,
    /// `SUBST_CHG_RACE` 73: choose the new race.
    Race,
    /// `SUBST_CHG_CLASS` 74: choose the new class.
    Class,
}

/// The sysop menu and its prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SysopStep {
    /// `SUBST_SYSOP_MENU` 11: the `[MAJORMUD Sysop]` prompt.
    Menu,
    /// `SUBST_UPDATE_FILE` 12: name of the update file.
    UpdateFile,
    /// `SUBST_UPDATING` 13: database update running.
    Updating,
    /// `SUBST_CLEARING` 14: clearing evil points or profiles.
    Clearing,
    /// `SUBST_REINDEXING` 15: reindexing limited items.
    Reindexing,
    /// `SUBST_XREF_USERS` 16: checking for deleted users.
    XrefUsers,
    /// `SUBST_XREF_DELETE` 17: "Delete this user?".
    XrefDelete,
    /// `SUBST_XREF_STRIP` 18: "Remove ALL limited items from this user?".
    XrefStrip,
    /// `SUBST_XREF_ITEMS` 19: checking users with limited items.
    XrefItems,
    /// `SUBST_PURGE_MINUTES` 22: new defunct minutes limit.
    PurgeMinutes,
    /// `SUBST_PURGE_DAYS` 23: new days-not-played limit.
    PurgeDays,
    /// `SUBST_PURGE_EXP` 24: new experience limit.
    PurgeExp,
}

/// The suicide password prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SuicideStep {
    /// `SUBST_SUICIDE_OLDPW` 132: SET SUICIDE, current password.
    OldPw,
    /// `SUBST_SUICIDE_NEWPW` 133: SET SUICIDE, new suicide password.
    NewPw,
    /// `SUBST_SUICIDE` 134: SUICIDE, enter the suicide password.
    Confirm,
}

impl SessionState {
    /// The original substate number, for log lines.
    pub const fn substt(&self) -> u8 {
        let code = match *self {
            Self::Start => Substate::Start,
            Self::Enter => Substate::Enter,
            Self::MainMenu => Substate::MainMenu,
            Self::Help => Substate::Help,
            Self::NewChar(step) => match step {
                NewCharStep::Race => Substate::NewRace,
                NewCharStep::Class => Substate::NewClass,
                NewCharStep::Lawful | NewCharStep::Stats => Substate::NewLawful,
                NewCharStep::Validating => Substate::Validating,
                NewCharStep::Name => Substate::NewName,
            },
            Self::RaceChange(step) => match step {
                RaceChangeStep::ReadHelp => Substate::ChgReadHelp,
                RaceChangeStep::TimeLimit => Substate::ChgTimeLimit,
                RaceChangeStep::NoSysop => Substate::ChgNoSysop,
                RaceChangeStep::Race => Substate::ChgRace,
                RaceChangeStep::Class => Substate::ChgClass,
            },
            Self::SysopMenu(step) => match step {
                SysopStep::Menu => Substate::SysopMenu,
                SysopStep::UpdateFile => Substate::UpdateFile,
                SysopStep::Updating => Substate::Updating,
                SysopStep::Clearing => Substate::Clearing,
                SysopStep::Reindexing => Substate::Reindexing,
                SysopStep::XrefUsers => Substate::XrefUsers,
                SysopStep::XrefDelete => Substate::XrefDelete,
                SysopStep::XrefStrip => Substate::XrefStrip,
                SysopStep::XrefItems => Substate::XrefItems,
                SysopStep::PurgeMinutes => Substate::PurgeMinutes,
                SysopStep::PurgeDays => Substate::PurgeDays,
                SysopStep::PurgeExp => Substate::PurgeExp,
            },
            Self::Playing => Substate::Playing,
            Self::Listing => Substate::Listing,
            Self::Suicide(step) => match step {
                SuicideStep::OldPw => Substate::SuicideOldPw,
                SuicideStep::NewPw => Substate::SuicideNewPw,
                SuicideStep::Confirm => Substate::Suicide,
            },
            Self::UserReport => Substate::UserReport,
            Self::Disband => Substate::Disband,
        };
        // Every substate code is below 256.
        code.code() as u8
    }

    /// The state for an original substate number; `None` for the dropped states and for
    /// numbers with no known meaning.
    pub const fn from_substt(code: u8) -> Option<Self> {
        Some(match Substate::from_code(code as u16) {
            Substate::Start => Self::Start,
            Substate::PlayAlt | Substate::Playing => Self::Playing,
            Substate::SysopMenu => Self::SysopMenu(SysopStep::Menu),
            Substate::UpdateFile => Self::SysopMenu(SysopStep::UpdateFile),
            Substate::Updating => Self::SysopMenu(SysopStep::Updating),
            Substate::Clearing => Self::SysopMenu(SysopStep::Clearing),
            Substate::Reindexing => Self::SysopMenu(SysopStep::Reindexing),
            Substate::XrefUsers => Self::SysopMenu(SysopStep::XrefUsers),
            Substate::XrefDelete => Self::SysopMenu(SysopStep::XrefDelete),
            Substate::XrefStrip => Self::SysopMenu(SysopStep::XrefStrip),
            Substate::XrefItems => Self::SysopMenu(SysopStep::XrefItems),
            Substate::Validating => Self::NewChar(NewCharStep::Validating),
            Substate::NewName => Self::NewChar(NewCharStep::Name),
            Substate::PurgeMinutes => Self::SysopMenu(SysopStep::PurgeMinutes),
            Substate::PurgeDays => Self::SysopMenu(SysopStep::PurgeDays),
            Substate::PurgeExp => Self::SysopMenu(SysopStep::PurgeExp),
            Substate::NewRace | Substate::NewChar => Self::NewChar(NewCharStep::Race),
            Substate::NewClass => Self::NewChar(NewCharStep::Class),
            Substate::MainMenu => Self::MainMenu,
            Substate::Help => Self::Help,
            Substate::NewLawful => Self::NewChar(NewCharStep::Lawful),
            Substate::Enter => Self::Enter,
            Substate::ChgReadHelp => Self::RaceChange(RaceChangeStep::ReadHelp),
            Substate::ChgTimeLimit => Self::RaceChange(RaceChangeStep::TimeLimit),
            Substate::ChgNoSysop => Self::RaceChange(RaceChangeStep::NoSysop),
            Substate::ChgRace => Self::RaceChange(RaceChangeStep::Race),
            Substate::ChgClass => Self::RaceChange(RaceChangeStep::Class),
            Substate::Listing => Self::Listing,
            Substate::SuicideOldPw => Self::Suicide(SuicideStep::OldPw),
            Substate::SuicideNewPw => Self::Suicide(SuicideStep::NewPw),
            Substate::Suicide => Self::Suicide(SuicideStep::Confirm),
            Substate::UserReport => Self::UserReport,
            Substate::Disband => Self::Disband,
            Substate::AddonNumber
            | Substate::AddonCode
            | Substate::Vacation
            | Substate::ClientInfo
            | Substate::Unknown(_) => return None,
        })
    }
}
