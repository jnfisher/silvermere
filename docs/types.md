# silvermere-types: what came from where

Every id, enum, flag set, record struct and name table in `crates/silvermere-types`. The facts
the tests check (integer constants, record member offsets, record lengths, name tables, reverse
directions) are kept in the committed snapshot `crates/silvermere-types/tests/fixtures/rules_facts.json`,
and the tests in `crates/silvermere-types/tests/` compare the crate against that on every run;
`tests/facts.rs` holds the helpers that read it.

## Array sizes

`silvermere_types::sizes` (re-exported at the crate root): every array size
the records use, and the text size. Checked by `tests/enum_codes.rs`
`array_sizes_match_the_facts`.

## Enums (`silvermere_types::enums`)

One `#[repr(u16)]` enum per family. Discriminants equal the original's codes; each enum has an
`Unknown(u16)` variant for codes the original never defined, so decoding never fails. Because
of that variant, read the value with `code()` (not `as u16`); `from_code`, `TryFrom<i32>` and
`code_name()` (the constant's name) are on every enum. `tests/enum_codes.rs` checks every code.

| Rust | Values |
|---|---|
| `OwnerKind` | 4 |
| `UpdateRecordType` | 10 |
| `Direction` | 10 (11, Auto Use in combat, is the constant `enums::DIR_AUTOUSE`, not a direction) |
| `ExitType` | 25 |
| `RoomType` | 8 |
| `GangHouse` | 11 |
| `ShopType` | 13 |
| `ItemType` | 11 |
| `WornLocation` | 20 |
| `WeaponType` | 4 |
| `ArmourType` | 10 |
| `WeaponClass` | 10 |
| `Currency` | 6 |
| `MonsterType` | 39 |
| `Alignment` | 7 |
| `GroupRole` | 4 |
| `Gender` | 3 |
| `MonsterAttackType` | 4 |
| `SpellType` | 4 |
| `SpellTarget` | 14 |
| `Magery` | 6 |
| `Element` | 7 |
| `ResistStat` | 7 |
| `ResistMode` | 3 |
| `MessageLevel` | 3 — inverted: lower is higher |
| `StatlineMode` | 3 (+ `Custom` 3, `FullCustom` 4) |
| `Material` | 7 |
| `Ability` | 188 |
| `Substate` | 39 |
| `SysopStatus` | 20 |
| `DebugLevel` | 4 |
| `InGame` | 3 |
| `EscState` | 3 |
| `CommandCode` | 138 |
| `HangupPunish` | 4 |

Constants without an enum (modules of `pub const`):

| Rust |
|---|
| `enums::uls` |
| `enums::tgm` |
| `enums::trn` |
| `enums::trb` |
| `enums::tell` |
| `enums::col` |

`SessionState` (with `NewCharStep`, `RaceChangeStep`, `SysopStep`, `SuicideStep`) is the
shaped form of `Substate`: it drops
`AddonNumber` 25, `AddonCode` 26, `Vacation` 59 and `ClientInfo` 60,
and `substt()` gives the original number for log lines. `Substate` keeps all 39 codes verbatim.

## Flag sets (`silvermere_types::flags`)

`bitflags` types; constant names are the original's flag names without the family prefix (e.g.
`PlayerFlags::NOGOSSIP`). `PlayerStatus::FROMABILS` (0x3980) and `PlayerStatus2::FROMABILS`
(0x00A6) are kept as named masks. `PlayerFlags::SPLITSCREEN` is kept only so flag words round-trip.

| Rust | Bits |
|---|---|
| `RoomFlags` | 7 |
| `GangFlags` | 4 |
| `ActionTargets` | 6 |
| `ActiveMonsterFlags` | 5 |
| `SessionFlags` | 7 |
| `PlayerStatus` | 16 |
| `PlayerStatus2` | 12 |
| `PlayerSettings` | 15 |
| `PlayerFlags` | 15 |
| `ExitFlags` | 14 |
| `SpellFlags` | 1 |

## Ids (`silvermere_types::ids`)

`MapId(u32)`, `RoomId(u32)`, `RoomKey { map, room }` (8 bytes in the records),
`ItemNo(i32)`, `MonsterNo(i32)`, `ActiveMonsterId(i32)`, `SpellNo(i16)`, `ClassNo(u16)`,
`RaceNo(u16)`, `ShopNo(i32)`, `MessageNo(i32)`, `TextBlockNo(i32)`, `SessionId(u16)`,
`AccountId(i64)`, `CharacterId(i64)`. The inner type is the width the original uses for that value.

## Records (`silvermere_types::records`)

Normalised structs (text as `String`, parallel arrays merged into slot structs, `unk_*` and
cache "dirty" bytes left out). Each has `RECORD_LEN` and `LAYOUT`, the byte layout
(`records::layout`: every member's name, offset, size and element size) that T-008 decodes
with. `tests/record_sizes.rs` checks every record length, and every `LAYOUT` entry's offset and
size, against the fixture.

| Rust | Bytes |
|---|---|
| `records::character::Character` | 1998 |
| `records::character::PermanentRecord` | 205 |
| `records::character::EvilRecord` | 57 |
| `ids::RoomKey` | 8 |
| `records::room::Room` | 1528 |
| `records::item::Item` | 1061 |
| `records::shop::Shop` | 500 |
| `records::bankbook::BankBook` | 72 |
| `records::item_ownership::ItemOwnership` | 65 |
| `records::monster::KnownMonster` | 732 |
| `records::monster::ActiveMonster` | 386 |
| `records::spell::Spell` | 253 |
| `records::class::Class` | 153 |
| `records::race::Race` | 126 |
| `records::message::Message` | 253 |
| `records::gang::Gang` | 253 |
| `records::action::Action` | 1010 |
| `records::text_block::TextBlock` | 2022 |
| `records::update::UpdateRecord` | 2027 |

Run-time structures (`records::runtime`, engine types with no byte layout): `PendingCommand`,
`ForgetEntry`, `UserState`, `UserFlags`, `TopEntry`, `BigExp`.

`Character` is the player record without `leader`, `group[5]` and `scanpos`
(session-only); `dragging` is kept and stored (the record
saves it, and a new character starts at 0, so the first march refusal shows once, T-048);
`sesstime` is kept and stored, since the
midnight cleanup folds it into `lifemins` (T-056). Coins are `Coins { copper, silver,
gold, platinum, runic }` everywhere: the player and room records name them `runic..copper`
and the `cash[5]` arrays of items, known monsters and active
monsters are runic first;
`Coins::from_runic_first` reverses them.

## Name tables (`silvermere_types::names`)

Word for word, without the NULL end marker. `tests/names_match_facts.rs` compares every entry.

| Rust | Words |
|---|---|
| `REVERSE_DIRECTIONS` | 10 |
| `DIRECTION_NAMES` | 10 |
| `DIRECTION_NAMES_2` | 10 |
| `DIRECTION_NAMES_3` | 10 |
| `DIRECTION_ABBREVIATIONS` | 10 |
| `EXIT_TYPES` | 25 |
| `ROOM_TYPES` | 8 |
| `SHOP_TYPES` | 13 |
| `MONSTER_TYPES` | 39 |
| `PRONOUNS` | 3 |
| `FOLLOW_TYPES` | 4 |
| `ALIGNMENTS` | 7 |
| `ITEM_TYPES` | 12 |
| `MATERIALS` | 7 |
| `MAGERY_TYPES` | 6 |
| `WEAPON_TYPES` | 4 |
| `ARMOUR_TYPES` | 10 |
| `WEAPON_CLASSES` | 10 |
| `WORN_LOCATIONS` | 20 |
| `COIN_DESCRIPTIONS` | 5 |
| `CURRENCY_NAMES` | 5 |
| `CURRENCY_NAMES_SINGULAR` | 6 |
| `OWNER_TYPES` | 4 |
| `SPELL_TYPES` | 4 |
| `RESIST_TYPES` | 7 |
| `SPELL_TARGETS` | 14 |
| `HAIR_COLOURS` | 10 |
| `HAIR_LENGTHS` | 6 |
| `EYE_COLOURS` | 17 |
| `STAT_NAMES` | 7 |
| `GANG_HOUSES` | 11 |
| `ABILITY_NAMES` | 188 |

Kept as the original has them: "Colliseum" (`ROOM_TYPES`), "Lightening" (`RESIST_TYPES`),
"chesnut-brown" (`EYE_COLOURS`), "Change me" (`SPELL_TYPES`), the duplicate "Finger", "Worn" and
"Wrist" of `WORN_LOCATIONS`, the 31-character currency placeholder of
`CURRENCY_NAMES[4]`/`CURRENCY_NAMES_SINGULAR[4]` that the config replaces at run time, and the
shifted `ITEM_TYPES` (a stray
"Shield" at index 2 makes every later type print the previous type's name;
`ItemType::display_name` does the shifted lookup).

## Fixed-width strings (`silvermere_types::fixed`)

`FixedStr<N>::decode`/`encode`: NUL-terminated CP437 fields to `String` and back, trailing
spaces kept (`depad` trims them, like the original). For T-008 and tests.

## Open questions

- The player record's `expbil`/`exprem` (experience split at a billion): two `u32`
  fields here, as in the record. Whether `Character` keeps two fields or one `u64` is T-023's
  decision.
