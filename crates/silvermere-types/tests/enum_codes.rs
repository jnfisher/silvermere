//! Every `FAMILY_NAME` constant of the committed facts has a Rust enum variant, flag or
//! constant of the same name and value, and every Rust name maps back to a constant.

use crate::facts;

use std::collections::BTreeMap;

use bitflags::Flags;
use silvermere_types::enums::*;
use silvermere_types::flags::*;
use silvermere_types::sizes;

/// The families this crate defines.
const FAMILIES: &[&str] = &[
    "RF", "GF", "AT", "AMF", "USF", "OWN", "UPD", "ST", "ST2", "PS", "PF", "EXF", "SPF", "DIR",
    "EXIT", "ROOM", "GH", "SHOP", "ITEM", "WORN", "WPN", "ARM", "WCLS", "CUR", "MTYPE", "ALIGN",
    "FOLLOW", "GENDER", "MATK", "SPT", "TGT", "MAGERY", "ELEM", "RSTAT", "SRES", "MLV", "SL",
    "MAT", "ABL", "SUBST", "UST", "DBG", "UIG", "ULS", "ESS", "CMD", "TGM", "TRN", "TRB", "TELL",
    "COL", "PUN",
];

/// Names among the command constants rather than the record constants.
const DEFINED_ELSEWHERE: &[&str] = &["SL_CUSTOM", "SL_FULLCUSTOM"];

macro_rules! add_enums {
    ($map:ident: $($ty:ty),* $(,)?) => {$(
        for &v in <$ty>::ALL {
            let name = v.code_name().expect("defined variants have a code name");
            assert_eq!(<$ty>::from_code(v.code()), v, "{name} round-trips");
            assert!($map.insert(name.to_string(), i64::from(v.code())).is_none(), "{name} twice");
        }
    )*};
}

macro_rules! add_flags {
    ($map:ident: $($ty:ty => $prefix:literal),* $(,)?) => {$(
        for flag in <$ty as Flags>::FLAGS {
            let name = format!("{}_{}", $prefix, flag.name());
            let bits = i64::from(flag.value().bits());
            assert!($map.insert(name.clone(), bits).is_none(), "{name} twice");
        }
    )*};
}

macro_rules! add_consts {
    ($map:ident: $($module:ident :: $name:ident => $prefix:literal),* $(,)?) => {$(
        $map.insert(
            format!("{}_{}", $prefix, stringify!($name)),
            i64::from($module::$name),
        );
    )*};
}

fn rust_names() -> BTreeMap<String, i64> {
    let mut map = BTreeMap::new();
    add_enums!(map:
        OwnerKind, UpdateRecordType, Direction, ExitType, RoomType, GangHouse, ShopType,
        ItemType, WornLocation, WeaponType, ArmourType, WeaponClass, Currency, MonsterType,
        Alignment, GroupRole, Gender, MonsterAttackType, SpellType, SpellTarget, Magery,
        Element, ResistStat, ResistMode, MessageLevel, StatlineMode, Material, Ability,
        Substate, SysopStatus, DebugLevel, InGame, EscState, CommandCode, HangupPunish,
    );
    add_flags!(map:
        RoomFlags => "RF", GangFlags => "GF", ActionTargets => "AT",
        ActiveMonsterFlags => "AMF", SessionFlags => "USF", PlayerStatus => "ST",
        PlayerStatus2 => "ST2", PlayerSettings => "PS", PlayerFlags => "PF",
        ExitFlags => "EXF", SpellFlags => "SPF",
    );
    map.insert("DIR_AUTOUSE".into(), i64::from(DIR_AUTOUSE));
    add_consts!(map:
        uls::NOTPLAYING => "ULS", uls::UNRANKED => "ULS", tgm::ALL => "TGM",
        trn::ALWAYS => "TRN", trn::MEDIUM => "TRN", trn::HIGH => "TRN",
        trb::SIGHT => "TRB", trb::ALL => "TRB",
        tell::NOPROMPT => "TELL", tell::PROMPT => "TELL", tell::NOCOLOR => "TELL",
        col::PROMPT => "COL", col::PROMPTVAL => "COL", col::HPHURT => "COL",
        col::HPLOW => "COL", col::MOVENAME => "COL", col::LINEEND => "COL",
        col::MOVETEXT => "COL", col::ACTION => "COL", col::BROADCAST => "COL",
        col::EXITS => "COL", col::ALSOHERE => "COL", col::SPELLGEN => "COL",
        col::SPELLOTH => "COL", col::HERENAMES => "COL", col::TELEPATH => "COL",
    );
    map
}

#[test]
fn rust_side_counts() {
    assert_eq!(CommandCode::ALL.len(), 138);
    assert_eq!(Substate::ALL.len(), 39);
    assert_eq!(Ability::ALL.len(), 188);
    assert_eq!(ExitType::ALL.len(), 25);
    assert_eq!(ExitType::Door.code(), 7);
    assert_eq!(Currency::Copper.code(), 0);
    assert_eq!(MessageLevel::Low.code(), 20);
    assert_eq!(StatlineMode::Full.code(), 2);
    assert_eq!(Substate::Playing.code(), 130);
    assert_eq!(CommandCode::Version.code(), 137);
    assert_eq!(PlayerFlags::NOGOSSIP.bits(), 0x0002);
    assert_eq!(PlayerFlags::NOAUCTION.bits(), 0x0004);
    assert_eq!(PlayerFlags::NEWEXP.bits(), 0x2000);
    assert_eq!(PlayerStatus::RESTING.bits(), 0x0400);
    assert_eq!(PlayerStatus2::MEDITATING.bits(), 0x0100);
    assert_eq!(PlayerSettings::EVILWARN.bits(), 0x0010);
    assert_eq!(PlayerSettings::KEEPEXP.bits(), 0x0400);
    assert_eq!(PlayerStatus::FROMABILS.bits(), 0x3980);
    assert_eq!(PlayerStatus2::FROMABILS.bits(), 0x00A6);
}

#[test]
fn unknown_codes_decode_without_panicking() {
    assert_eq!(ExitType::from_code(25), ExitType::Unknown(25));
    assert_eq!(ExitType::try_from(7), Ok(ExitType::Door));
    assert_eq!(ExitType::try_from(-1), Err(CodeOutOfRange(-1)));
    assert_eq!(ExitType::Unknown(99).code(), 99);
    assert_eq!(ExitType::Unknown(99).code_name(), None);
    assert_eq!(
        PlayerFlags::from_bits_retain(0x8002).bits(),
        0x8002,
        "unnamed bits survive"
    );
}

// SUBST_*: the shaped state keeps every code but the dropped four.
#[test]
fn session_state_round_trips_substt() {
    let dropped = [
        Substate::AddonNumber,
        Substate::AddonCode,
        Substate::Vacation,
        Substate::ClientInfo,
    ];
    for &s in Substate::ALL {
        let code = s.code() as u8;
        match SessionState::from_substt(code) {
            None => assert!(dropped.contains(&s), "{s:?} must map to a state"),
            Some(state) => {
                let back = match s {
                    Substate::PlayAlt => Substate::Playing.code(),
                    Substate::NewChar => Substate::NewRace.code(),
                    _ => s.code(),
                };
                assert_eq!(u16::from(state.substt()), back, "{s:?}");
            }
        }
    }
    assert_eq!(SessionState::from_substt(200), None);
    assert_eq!(SessionState::Playing.substt(), 130);
}

// Array sizes
#[test]
fn array_sizes_match_the_facts() {
    let facts = facts::facts();
    let want: BTreeMap<&str, usize> = [
        ("NSPELLCAST", sizes::NSPELLCAST),
        ("NINVITEMS", sizes::NINVITEMS),
        ("NKEYS", sizes::NKEYS),
        ("NSPELLBOOK", sizes::NSPELLBOOK),
        ("NTRAIL", sizes::NTRAIL),
        ("NWORN", sizes::NWORN),
        ("NGROUP", sizes::NGROUP),
        ("NABILITY", sizes::NABILITY),
        ("NDIRS", sizes::NDIRS),
        ("NROOMITEMS", sizes::NROOMITEMS),
        ("NHIDITEMS", sizes::NHIDITEMS),
        ("NROOMMONS", sizes::NROOMMONS),
        ("NPLACED", sizes::NPLACED),
        ("NCURRENCY", sizes::NCURRENCY),
        ("NITEMABILS", sizes::NITEMABILS),
        ("NSHOPITEMS", sizes::NSHOPITEMS),
        ("NRECABILS", sizes::NRECABILS),
        ("NMONITEMS", sizes::NMONITEMS),
        ("NMONATTACKS", sizes::NMONATTACKS),
        ("NMONSPELLS", sizes::NMONSPELLS),
        ("NAMTRAIL", sizes::NAMTRAIL),
        ("NTOPLIST", sizes::NTOPLIST),
        ("NACTTEXTS", sizes::NACTTEXTS),
        ("TEXTSIZE", sizes::TEXTSIZE),
    ]
    .into_iter()
    .collect();
    let mut seen = 0;
    for d in &facts.constants {
        if let Some(&v) = want.get(d.name.as_str()) {
            assert_eq!(d.value as usize, v, "{}", d.name);
            seen += 1;
        }
    }
    assert_eq!(seen, want.len());
}

// Every constant of the families this crate defines, by name.
#[test]
fn every_define_has_its_value() {
    let rust = rust_names();
    let facts = facts::facts();
    let defines: Vec<_> = facts
        .constants
        .iter()
        .filter(|d| FAMILIES.contains(&d.family()))
        .collect();

    let mut per_family: BTreeMap<&str, usize> = BTreeMap::new();
    for d in &defines {
        *per_family.entry(d.family()).or_default() += 1;
        let got = rust
            .get(&d.name)
            .unwrap_or_else(|| panic!("{} has no Rust counterpart", d.name));
        assert_eq!(*got, d.value, "{}", d.name);
    }
    for name in rust.keys() {
        assert!(
            defines.iter().any(|d| &d.name == name) || DEFINED_ELSEWHERE.contains(&name.as_str()),
            "{name} is not among the record constants"
        );
    }
    assert_eq!(per_family["CMD"], 138);
    assert_eq!(per_family["SUBST"], 39);
    assert_eq!(per_family["ABL"], 188);
    assert_eq!(per_family["EXIT"], 25);
    assert_eq!(per_family.len(), FAMILIES.len(), "every family has defines");
}

// SL_CUSTOM, SL_FULLCUSTOM
#[test]
fn custom_statline_codes_match_the_command_constants() {
    assert_eq!(StatlineMode::Custom.code(), 3);
    assert_eq!(StatlineMode::FullCustom.code(), 4);
    let facts = facts::facts();
    let defs = facts.command_consts();
    assert_eq!(defs.get("SL_CUSTOM"), Some(&3));
    assert_eq!(defs.get("SL_FULLCUSTOM"), Some(&4));
}
