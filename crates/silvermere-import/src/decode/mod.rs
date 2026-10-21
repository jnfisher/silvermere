//! Record decoders: the byte images of the game's records to flat Rust structs and on to
//! the `silvermere-types` records.
//!
//! Each record type has a flat struct (`RoomRecord`, `ItemRecord`, ...) that mirrors the
//! record member for member, `unk_*` spans, cache bytes and run-time fields included. It
//! is declared with [`record!`], which takes the member list once and generates
//! [`Record::decode`], [`Record::encode`] and [`Record::to_json`]. The offsets are not
//! written here: every member is looked up by its member name in the `silvermere-types`
//! [`Layout`] (the `FIELD_OFFSETS` contract from T-002) and its size checked against the
//! Rust type, once per type and layout set; `tests/decode_offsets.rs` checks that every
//! layout member is covered. One struct serves both builds: [`Record::decode_as`] and
//! [`Record::encode_as`] take the [`LayoutSet`] (the DOS build's byte-packed records or the
//! Worldgroup 3.x build's aligned ones), and `decode`/`encode` are the DOS set. The flat struct is what `--json` writes ([`crate::json`]); the `split_*` /
//! `to_*` function beside each one maps it to the schema types the writers take.
//!
//! Member widths: unsigned bytes are [`u8`], signed bytes [`i8`], 16-bit words [`i16`] or
//! [`u16`], 32-bit words [`i32`] or [`u32`] by signedness; little-endian. A
//! text member (`N` bytes) is
//! [`Text`], an `unk_*` byte span [`Raw`], a 2-D text member an array of [`Text`].

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Map, Value};
use silvermere_types::enums::Ability;
use silvermere_types::fixed::{FixedStr, cp437_to_char};
use silvermere_types::records::{AbilitySlot, Coins, Layout, LayoutSet};

use crate::dos_time::{DosTimeError, dos_to_unix};

pub mod action;
pub mod actmon;
pub mod bankbook;
pub mod class;
pub mod gang;
pub mod item;
pub mod itemown;
pub mod message;
pub mod monster;
pub mod player;
pub mod race;
pub mod room;
pub mod shop;
pub mod spell;
pub mod textblk;

pub use action::{ActionRecord, to_action};
pub use actmon::{ActmonRecord, to_active_monster};
pub use bankbook::{BankbookRecord, to_bank_book};
pub use class::{ClassRecord, to_class};
pub use gang::{GangRecord, to_gang};
pub use item::{ItemRecord, to_item};
pub use itemown::{ItemownRecord, to_item_ownership};
pub use message::{MudmsgRecord, to_message};
pub use monster::{MonsterRecord, split_monster};
pub use player::{PlayerRecord, to_character};
pub use race::{RaceRecord, to_race};
pub use room::{RoomRecord, split_room};
pub use shop::{ShopRecord, split_shop};
pub use spell::{SpellRecord, to_spell};
pub use textblk::{TextblkRecord, decode_textblk, decode_textblk_as, to_text_block};

/// A record that does not fit its layout, or holds a date that is not one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// A nonzero DOS date or time with a field out of range.
    #[error("struct {record} {key}: {field}: {source}")]
    Date {
        /// The record type's name.
        record: &'static str,
        /// The record's key, e.g. `number 12`.
        key: String,
        /// The member (`killdate`, ...).
        field: &'static str,
        /// What was wrong.
        source: DosTimeError,
    },
    /// The byte image is not the record length.
    #[error("struct {record}: {found} bytes, expected {expected}")]
    Length {
        /// The record type's name.
        record: &'static str,
        /// The layout's record length.
        expected: usize,
        /// The bytes given.
        found: usize,
    },
}

/// How the records' DOS dates become Unix seconds: the zone the legacy server ran in
/// (`--legacy-tz`, default UTC) and the import day there, for the fields that hold only a
/// `now()` time (`actmon.created`). The schema stores Unix seconds UTC, 0 = never.
#[derive(Debug, Clone)]
pub struct LegacyClock {
    /// The legacy server's zone.
    pub tz: TimeZone,
    /// The import day in that zone, as a DOS date.
    pub today: u16,
}

impl LegacyClock {
    /// The clock for zone `tz` at the moment `now`.
    pub fn new(tz: TimeZone, now: Timestamp) -> Self {
        let day = now.to_zoned(tz.clone()).date();
        let years = u16::try_from(day.year() - 1980).unwrap_or(0).min(127);
        let today = years << 9 | (day.month() as u16) << 5 | day.day() as u16;
        Self { tz, today }
    }

    /// UTC, with the import day taken from the system clock.
    pub fn utc_now() -> Self {
        Self::new(TimeZone::UTC, Timestamp::now())
    }

    /// `date` and `time` (pass 0 for a date-only field) as Unix seconds, or an error naming
    /// the record.
    pub fn at(
        &self,
        record: &'static str,
        key: impl std::fmt::Display,
        field: &'static str,
        date: u16,
        time: u16,
    ) -> Result<i64, DecodeError> {
        dos_to_unix(date, time, &self.tz).map_err(|source| DecodeError::Date {
            record,
            key: key.to_string(),
            field,
            source,
        })
    }
}

/// One member's value type: how it is read from and written to its bytes, and its JSON.
pub trait Wire: Sized {
    /// Bytes the member occupies.
    const SIZE: usize;
    /// Reads the member from the start of `b` (at least [`Wire::SIZE`] bytes).
    fn read(b: &[u8]) -> Self;
    /// Writes the member to the start of `out` (at least [`Wire::SIZE`] bytes).
    fn write(&self, out: &mut [u8]);
    /// The member as T-006's export writes it.
    fn json(&self) -> Value;
}

macro_rules! wire_int {
    ($($t:ty),*) => { $(
        impl Wire for $t {
            const SIZE: usize = size_of::<$t>();
            fn read(b: &[u8]) -> Self {
                let mut a = [0u8; size_of::<$t>()];
                a.copy_from_slice(&b[..size_of::<$t>()]);
                <$t>::from_le_bytes(a)
            }
            fn write(&self, out: &mut [u8]) {
                out[..size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
            }
            fn json(&self) -> Value {
                Value::from(*self)
            }
        }
    )* };
}
wire_int!(u8, i8, u16, i16, u32, i32);

impl<T: Wire, const N: usize> Wire for [T; N] {
    const SIZE: usize = T::SIZE * N;
    fn read(b: &[u8]) -> Self {
        std::array::from_fn(|i| T::read(&b[i * T::SIZE..]))
    }
    fn write(&self, out: &mut [u8]) {
        for (i, v) in self.iter().enumerate() {
            v.write(&mut out[i * T::SIZE..]);
        }
    }
    fn json(&self) -> Value {
        Value::Array(self.iter().map(Wire::json).collect())
    }
}

/// An `N`-byte text member: the CP437 bytes up to the first NUL, untrimmed (the
/// original trims trailing spaces only where it uses the text).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text<const N: usize>(pub String);

impl<const N: usize> Text<N> {
    /// The text as T-006 compares and the schema stores it: [`trim`]med.
    pub fn trimmed(&self) -> String {
        trim(&self.0).to_string()
    }
}

impl<const N: usize> Wire for Text<N> {
    const SIZE: usize = N;
    fn read(b: &[u8]) -> Self {
        Self(FixedStr::<N>::decode(&b[..N]))
    }
    fn write(&self, out: &mut [u8]) {
        out[..N].copy_from_slice(&FixedStr::<N>::encode(&self.0));
    }
    fn json(&self) -> Value {
        Value::String(self.trimmed())
    }
}

/// An `unk_*` byte span, kept verbatim (T-009 copies some of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw<const N: usize>(pub Vec<u8>);

impl<const N: usize> Default for Raw<N> {
    fn default() -> Self {
        Self(vec![0; N])
    }
}

impl<const N: usize> Wire for Raw<N> {
    const SIZE: usize = N;
    fn read(b: &[u8]) -> Self {
        Self(b[..N].to_vec())
    }
    fn write(&self, out: &mut [u8]) {
        out[..N].copy_from_slice(&self.0[..N]);
    }
    fn json(&self) -> Value {
        Value::String(base64(&self.0))
    }
}

/// The text trimming T-006 applies (`mmud_data._s`: Python's `str.strip()` after the
/// NUL cut), so both sides compare alike: Unicode white space and the ASCII separators
/// 0x1C-0x1F, at both ends.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// Standard base64 with padding (what Python's `base64.b64encode` writes).
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// CP437 bytes to a `String` (no NUL handling).
pub(crate) fn cp437(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| cp437_to_char(b)).collect()
}

/// A `cash[5]` array in the records' runic-first order as [`Coins`].
pub fn coins_runic_first(cash: [u32; 5]) -> Coins {
    Coins::from_runic_first(cash)
}

/// Parallel `abilities[N]` / `abilvals[N]` arrays as ability slots.
pub fn abilities<const N: usize>(codes: &[i16; N], values: &[i16; N]) -> [AbilitySlot; N] {
    std::array::from_fn(|i| (Ability::from_code(codes[i] as u16), values[i]))
}

/// Coins from five fields named `runic..copper`.
pub fn coins_named(runic: u32, platinum: u32, gold: u32, silver: u32, copper: u32) -> Coins {
    Coins {
        copper,
        silver,
        gold,
        platinum,
        runic,
    }
}

/// A flat record: a record's members in record order.
pub trait Record: Sized {
    /// The record's byte-packed (DOS) layout in `silvermere-types`.
    const LAYOUT: &'static Layout;
    /// Every member: name and size in bytes, in record order.
    const FIELDS: &'static [(&'static str, usize)];

    /// Each member's offset, resolved by name in the set's layout (cached per type and set).
    fn offsets(set: LayoutSet) -> &'static [usize];
    /// Reads every member; `raw` is exactly the record length.
    fn read_fields(raw: &[u8], offs: &[usize]) -> Self;
    /// Writes every member into `out` (the record length).
    fn write_fields(&self, out: &mut [u8], offs: &[usize]);
    /// Appends every member to `map` under its member name.
    fn json_fields(&self, map: &mut Map<String, Value>);

    /// The record's layout in `set`.
    fn layout(set: LayoutSet) -> &'static Layout {
        set.layout(Self::LAYOUT)
    }

    /// Decodes one byte-packed (DOS) record image.
    fn decode(raw: &[u8]) -> Result<Self, DecodeError> {
        Self::decode_as(raw, LayoutSet::Dos)
    }

    /// Decodes one record image in `set`'s layout. The length is checked once here, so the
    /// member reads cannot run out of bytes.
    fn decode_as(raw: &[u8], set: LayoutSet) -> Result<Self, DecodeError> {
        let layout = Self::layout(set);
        if raw.len() != layout.len {
            return Err(DecodeError::Length {
                record: layout.decoder,
                expected: layout.len,
                found: raw.len(),
            });
        }
        Ok(Self::read_fields(raw, Self::offsets(set)))
    }

    /// The byte-packed (DOS) record image.
    fn encode(&self) -> Vec<u8> {
        self.encode_as(LayoutSet::Dos)
    }

    /// The record image in `set`'s layout: every member at its offset, the bytes no member
    /// covers (`pad_*` included) zero.
    fn encode_as(&self, set: LayoutSet) -> Vec<u8> {
        let mut out = vec![0; Self::layout(set).len];
        self.write_fields(&mut out, Self::offsets(set));
        out
    }

    /// The members as one JSON object (keys sorted), under their member names.
    fn to_json(&self) -> Map<String, Value> {
        let mut map = Map::new();
        self.json_fields(&mut map);
        map
    }
}

/// Resolves `fields` against `layout`: the offset of each, by member name. Panics when a member
/// is missing from the layout or has another size, which is a bug in a `record!` list
/// (`tests/decode_offsets.rs` exercises every type).
pub fn resolve_offsets(layout: &Layout, fields: &[(&str, usize)]) -> Vec<usize> {
    fields
        .iter()
        .map(|&(name, size)| {
            let f = layout
                .field(name)
                .unwrap_or_else(|| panic!("struct {}: no member {name}", layout.decoder));
            assert_eq!(
                f.size, size,
                "struct {}: member {name} is {} bytes in the layout, {size} in the decoder",
                layout.decoder, f.size
            );
            f.offset
        })
        .collect()
}

/// The stored name of a member: the field name, or the literal given for one that is a Rust
/// keyword (`type_ = "type"`).
macro_rules! field_name {
    ($field:ident) => {
        stringify!($field)
    };
    ($field:ident $c:literal) => {
        $c
    };
}
pub(crate) use field_name;

/// Declares a flat record struct and its [`Record`] impl from the member list.
macro_rules! record {
    (
        $(#[$meta:meta])*
        pub struct $name:ident = $layout:path {
            $( $field:ident $(= $c:literal)? : $ty:ty, )*
        }
    ) => {
        $(#[$meta])*
        // The members keep their record member names, `unk_0x5AF` and the like included.
        #[allow(non_snake_case)]
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            $( pub $field: $ty, )*
        }

        impl $crate::decode::Record for $name {
            const LAYOUT: &'static silvermere_types::records::Layout = &$layout;
            const FIELDS: &'static [(&'static str, usize)] = &[
                $( ($crate::decode::field_name!($field $($c)?), <$ty as $crate::decode::Wire>::SIZE), )*
            ];

            fn offsets(set: silvermere_types::records::LayoutSet) -> &'static [usize] {
                use silvermere_types::records::LayoutSet;
                static DOS: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();
                static WORLDGROUP: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();
                let cell = match set {
                    LayoutSet::Dos => &DOS,
                    LayoutSet::Worldgroup => &WORLDGROUP,
                };
                cell.get_or_init(|| {
                    $crate::decode::resolve_offsets(Self::layout(set), Self::FIELDS)
                })
            }

            fn read_fields(raw: &[u8], offs: &[usize]) -> Self {
                let mut offs = offs.iter();
                Self {
                    $( $field: <$ty as $crate::decode::Wire>::read(&raw[*offs.next().unwrap()..]), )*
                }
            }

            fn write_fields(&self, out: &mut [u8], offs: &[usize]) {
                let mut offs = offs.iter();
                $( $crate::decode::Wire::write(&self.$field, &mut out[*offs.next().unwrap()..]); )*
            }

            fn json_fields(&self, map: &mut serde_json::Map<String, serde_json::Value>) {
                $( map.insert(
                    $crate::decode::field_name!($field $($c)?).to_string(),
                    $crate::decode::Wire::json(&self.$field),
                ); )*
            }
        }
    };
}
pub(crate) use record;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_python() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff, 0, 0x7f, 1]), "/wB/AQ==");
    }

    #[test]
    fn trim_matches_python_strip() {
        assert_eq!(trim("  a b \t\r\n"), "a b");
        assert_eq!(trim("\x1fx\u{a0}"), "x");
    }
}
