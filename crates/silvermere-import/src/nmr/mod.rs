//! Nightmare Redux databases: a realm exported to Microsoft Access (`.mdb`) by Nightmare
//! Redux (NMR), the Windows editor for MajorMUD's data files.
//!
//! Edited realms such as MudRev are passed around as these databases. NMR's own way back is
//! its import: into a set of data files it opens one record per row, looks the record up by
//! its key, and either updates the record it found (the members it has a column for take the
//! row's values; every other byte stays as it was) or inserts a new one. This module does the
//! same over records already read: the base is the stock 1.11p world, as `silvermere import`
//! reads and patches it, and each row of the database is written over the stock record with
//! its key, or becomes a new record. Stock records the database has no row for are kept, as
//! NMR keeps them.
//!
//! NMR's records are the Worldgroup 3.x layouts (its members, packed in the order of its
//! record types, sit at the `WG_*` offsets), so a base record is encoded in that layout, the
//! row's columns written at NMR's offsets ([`columns`]) and the result decoded again. A value
//! that does not fit its member is skipped, as NMR's Visual Basic skips the assignment that
//! overflows ("On Error ... Resume Next"); so is a missing column or an empty cell. Text is
//! trimmed of spaces, encoded in Windows-1252 (the code page NMR's export decoded it with)
//! and zero-filled rather than space-filled.
//!
//! Deliberate differences from NMR, for a world that has never run: a monster's `Active`,
//! `Date Killed` and `Time Killed` (counts and times from the server the database was
//! exported from) are written as 0, or the stale count of a limited monster would stop it
//! appearing. The stock `WCCUPDAT.DAT` pass runs on the base before the rows are applied, and
//! not after, so it cannot turn edited records back into stock ones.

mod columns;

use std::collections::HashMap;
use std::hash::Hash;
use std::path::Path;

use jetdb::{PageReader, Value, find_table, read_catalog, read_table_def, read_table_rows};
use silvermere_types::records::LayoutSet;

use crate::decode::{
    self, ActionRecord, ClassRecord, DecodeError, ItemRecord, MonsterRecord, MudmsgRecord,
    RaceRecord, Raw, Record, RoomRecord, ShopRecord, SpellRecord, TextblkRecord,
};
use crate::importer::{Content, Decoded};

/// The text bytes of a text block (`TEXTSIZE`).
const TEXT_LEN: usize = 2000;

/// The database's errors.
#[derive(Debug, thiserror::Error)]
pub enum NmrError {
    /// The file could not be read as an Access database.
    #[error("{file}: not a readable Access database: {reason}")]
    Open {
        /// The file.
        file: String,
        /// What went wrong.
        reason: String,
    },
    /// The database has none of NMR's tables.
    #[error(
        "{0}: not a Nightmare Redux export (no Items, Monsters, Rooms or Spells table); \
         export the realm from Nightmare Redux with Database > Export"
    )]
    NotNmr(String),
    /// A table could not be read.
    #[error("{file} [{table}]: {reason}")]
    Table {
        /// The file.
        file: String,
        /// The table.
        table: String,
        /// What went wrong.
        reason: String,
    },
    /// A rebuilt record did not decode.
    #[error("{file} [{table}]: {source}")]
    Decode {
        /// The file.
        file: String,
        /// The table.
        table: String,
        /// What was wrong.
        source: DecodeError,
    },
}

/// How a column is stored in NMR's record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A Visual Basic `Byte`: 0..=255.
    Byte,
    /// An `Integer`: 16-bit signed.
    Integer,
    /// A `Long`: 32-bit signed.
    Long,
    /// A `Long` the import converts from an unsigned value (`ULong2SLong`).
    ULong,
    /// A `String * n`.
    Text(usize),
}

/// One column of an NMR table and the offset its import writes it at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Col {
    name: &'static str,
    offset: usize,
    kind: Kind,
}

impl Col {
    const fn new(name: &'static str, offset: usize, kind: Kind) -> Self {
        Self { name, offset, kind }
    }
}

/// One table of the database: its column names and rows.
#[derive(Debug, Default)]
pub struct Table {
    columns: HashMap<String, usize>,
    rows: Vec<Vec<Value>>,
}

/// One row, read by column name.
#[derive(Debug, Clone, Copy)]
struct Row<'a> {
    table: &'a Table,
    values: &'a [Value],
}

impl<'a> Row<'a> {
    fn get(&self, column: &str) -> Option<&'a Value> {
        self.table
            .columns
            .get(column)
            .and_then(|&i| self.values.get(i))
    }

    /// A number, as Visual Basic converts it on assignment (a Double rounds half to even);
    /// `None` for an empty cell, a missing column or text that is no number.
    fn number(&self, column: &str) -> Option<f64> {
        match self.get(column)? {
            Value::Bool(b) => Some(if *b { -1.0 } else { 0.0 }),
            Value::Byte(v) => Some(f64::from(*v)),
            Value::Int(v) => Some(f64::from(*v)),
            Value::Long(v) => Some(f64::from(*v)),
            Value::BigInt(v) => Some(*v as f64),
            Value::Float(v) => Some(f64::from(*v).round_ties_even()),
            Value::Double(v) => Some(v.round_ties_even()),
            Value::Text(s) | Value::Money(s) | Value::Numeric(s) => {
                s.trim().parse::<f64>().ok().map(f64::round_ties_even)
            }
            _ => None,
        }
    }

    fn int(&self, column: &str) -> Option<i64> {
        self.number(column)
            .filter(|v| v.is_finite())
            .map(|v| v as i64)
    }

    /// Text up to its first NUL: NMR's export copies whole fixed-length members, so a
    /// name keeps the NUL that ends it and whatever bytes followed.
    fn text(&self, column: &str) -> Option<&'a str> {
        match self.get(column)? {
            Value::Text(s) => Some(s.split('\0').next().unwrap_or("")),
            _ => None,
        }
    }
}

/// A Nightmare Redux database, read whole.
#[derive(Debug)]
pub struct Database {
    /// The file name, for the count lines.
    pub name: String,
    tables: HashMap<String, Table>,
}

/// The tables NMR exports.
const TABLES: [&str; 10] = [
    "Actions",
    "Classes",
    "Items",
    "Messages",
    "Monsters",
    "Races",
    "Rooms",
    "Shops",
    "Spells",
    "Textblocks",
];

impl Database {
    /// Reads every NMR table of `path`.
    pub fn open(path: &Path) -> Result<Self, NmrError> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let open_err = |e: &dyn std::fmt::Display| NmrError::Open {
            file: name.clone(),
            reason: e.to_string(),
        };
        let mut reader = PageReader::open(path).map_err(|e| open_err(&e))?;
        let catalog = read_catalog(&mut reader).map_err(|e| open_err(&e))?;
        let mut tables = HashMap::new();
        for table in TABLES {
            let Ok(entry) = find_table(&catalog, table) else {
                continue;
            };
            let table_err = |e: &dyn std::fmt::Display| NmrError::Table {
                file: name.clone(),
                table: table.to_string(),
                reason: e.to_string(),
            };
            let def = read_table_def(&mut reader, &entry.name, entry.table_page)
                .map_err(|e| table_err(&e))?;
            let rows = read_table_rows(&mut reader, &def).map_err(|e| table_err(&e))?;
            let columns = def
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| (c.name.clone(), i))
                .collect();
            tables.insert(
                table.to_string(),
                Table {
                    columns,
                    rows: rows.rows,
                },
            );
        }
        if !["Items", "Monsters", "Rooms", "Spells"]
            .iter()
            .any(|t| tables.contains_key(*t))
        {
            return Err(NmrError::NotNmr(name));
        }
        Ok(Self { name, tables })
    }

    /// The rows of `table` (none when the database lacks it).
    fn rows<'a>(&'a self, table: &str) -> impl Iterator<Item = Row<'a>> {
        self.tables
            .get(table)
            .into_iter()
            .flat_map(|t| t.rows.iter().map(move |values| Row { table: t, values }))
    }

    /// Rows per table, for the report.
    pub fn row_count(&self, table: &str) -> usize {
        self.tables.get(table).map_or(0, |t| t.rows.len())
    }
}

/// Windows-1252 for a char, `None` where it has no byte (written as `?`, as Visual Basic's
/// conversion does).
fn cp1252(c: char) -> Option<u8> {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}',
        '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    let code = u32::from(c);
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return u8::try_from(code).ok();
    }
    HIGH.iter()
        .position(|&h| h == c)
        .and_then(|i| u8::try_from(0x80 + i).ok())
}

/// Text as NMR stores it: spaces trimmed, Windows-1252.
fn ansi(s: &str) -> Vec<u8> {
    s.trim_matches(' ')
        .chars()
        .map(|c| cp1252(c).unwrap_or(b'?'))
        .collect()
}

/// Writes one cell at its offset; a value that does not fit is skipped.
fn put(buf: &mut [u8], col: &Col, row: &Row<'_>) {
    let at = col.offset;
    match col.kind {
        Kind::Text(n) => {
            let Some(s) = row.text(col.name) else { return };
            let bytes = ansi(s);
            let field = &mut buf[at..at + n];
            field.fill(0);
            let len = bytes.len().min(n);
            field[..len].copy_from_slice(&bytes[..len]);
        }
        Kind::Byte => {
            if let Some(v) = row.int(col.name).and_then(|v| u8::try_from(v).ok()) {
                buf[at] = v;
            }
        }
        Kind::Integer => {
            if let Some(v) = row.int(col.name).and_then(|v| i16::try_from(v).ok()) {
                buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        Kind::Long => {
            if let Some(v) = row.int(col.name).and_then(|v| i32::try_from(v).ok()) {
                buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        Kind::ULong => {
            if let Some(v) = row.int(col.name).and_then(ulong_to_slong) {
                buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

/// NMR's `ULong2SLong`: 0..=4294967295 into a signed `Long`.
fn ulong_to_slong(v: i64) -> Option<i32> {
    let u = u32::try_from(v).ok()?;
    Some(u as i32)
}

/// Every row of `table` written over the base record with its key, or into a new record;
/// base records with no row are kept, in their order, the new ones after them.
fn overlay<R, K>(
    db: &Database,
    table: &str,
    cols: &[Col],
    base: &[Decoded<R>],
    key_of: impl Fn(&R) -> K,
    row_key: impl Fn(&Row<'_>) -> Option<K>,
    fix: impl Fn(&mut R),
) -> Result<Vec<Decoded<R>>, NmrError>
where
    R: Record + Clone,
    K: Hash + Eq + Clone,
{
    let set = LayoutSet::Worldgroup;
    let len = R::layout(set).len;
    let label = format!("{} [{table}]", db.name);
    let mut by_key: HashMap<K, usize> = HashMap::new();
    for (i, d) in base.iter().enumerate() {
        by_key.entry(key_of(&d.record)).or_insert(i);
    }
    let mut replaced: HashMap<usize, Decoded<R>> = HashMap::new();
    let mut added: Vec<Decoded<R>> = Vec::new();
    let mut added_at: HashMap<K, usize> = HashMap::new();
    for (n, row) in db.rows(table).enumerate() {
        let Some(key) = row_key(&row) else { continue };
        let ordinal = u32::try_from(n + 1).unwrap_or(u32::MAX);
        let existing = by_key.get(&key).copied();
        let mut buf = match (existing, added_at.get(&key)) {
            (_, Some(&j)) => added[j].record.encode_as(set),
            (Some(i), None) => replaced.get(&i).map_or_else(
                || base[i].record.encode_as(set),
                |d| d.record.encode_as(set),
            ),
            (None, None) => vec![0; len],
        };
        for col in cols {
            put(&mut buf, col, &row);
        }
        let mut record = R::decode_as(&buf, set).map_err(|source| NmrError::Decode {
            file: db.name.clone(),
            table: table.to_string(),
            source,
        })?;
        fix(&mut record);
        let decoded = Decoded {
            file: label.clone(),
            ordinal,
            record,
        };
        match (existing, added_at.get(&key)) {
            (_, Some(&j)) => added[j] = decoded,
            (Some(i), None) => {
                replaced.insert(i, decoded);
            }
            (None, None) => {
                added_at.insert(key, added.len());
                added.push(decoded);
            }
        }
    }
    let mut out: Vec<Decoded<R>> = base
        .iter()
        .enumerate()
        .map(|(i, d)| replaced.remove(&i).unwrap_or_else(|| d.clone()))
        .collect();
    out.extend(added);
    Ok(out)
}

fn int_key(row: &Row<'_>, column: &str) -> Option<i64> {
    row.int(column)
}

/// Stored text-block text: each Windows-1252 byte up to 223 plus 0x20 (higher ones are
/// dropped), at most 2000 bytes, zero-filled (`EncryptTextblock`).
fn encrypt_text(s: &str) -> Vec<u8> {
    let mut out: Vec<u8> = s
        .chars()
        .map(|c| cp1252(c).unwrap_or(b'?'))
        .filter(|&b| b <= 223)
        .map(|b| b + decode::textblk::TEXT_SHIFT)
        .take(TEXT_LEN)
        .collect();
    out.resize(TEXT_LEN, 0);
    out
}

/// The text blocks: a block number the database has replaces every stored part of that
/// number (NMR deletes the parts after the last one it imports), the others are kept.
fn text_blocks(db: &Database, base: &[Decoded<TextblkRecord>]) -> Vec<Decoded<TextblkRecord>> {
    let lead_in = base
        .first()
        .map_or_else(Raw::<12>::default, |d| d.record.unk_0x002.clone());
    let label = format!("{} [Textblocks]", db.name);
    let mut rows: Vec<TextblkRecord> = Vec::new();
    let mut numbers = std::collections::HashSet::new();
    for row in db.rows("Textblocks") {
        let (Some(number), Some(part)) = (row.int("Number"), row.int("Part #")) else {
            continue;
        };
        let (Ok(number), Ok(part)) = (i32::try_from(number), i16::try_from(part)) else {
            continue;
        };
        numbers.insert(number);
        let link = row
            .int("Link To")
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(0);
        rows.push(TextblkRecord {
            part,
            unk_0x002: lead_in.clone(),
            number,
            link,
            text: Raw(encrypt_text(row.text("Data").unwrap_or(""))),
        });
    }
    let mut out: Vec<Decoded<TextblkRecord>> = base
        .iter()
        .filter(|d| !numbers.contains(&d.record.number))
        .cloned()
        .collect();
    // one record per (number, part): the last row wins, as NMR's update of the same key
    let mut seen: HashMap<(i32, i16), usize> = HashMap::new();
    for (n, record) in rows.into_iter().enumerate() {
        let decoded = Decoded {
            file: label.clone(),
            ordinal: u32::try_from(n + 1).unwrap_or(u32::MAX),
            record,
        };
        let key = (decoded.record.number, decoded.record.part);
        if let Some(&i) = seen.get(&key) {
            out[i] = decoded;
        } else {
            seen.insert(key, out.len());
            out.push(decoded);
        }
    }
    out
}

/// Applies the database to `content` (the stock world, read and patched), table by table.
pub fn apply(content: &mut Content, db: &Database) -> Result<(), NmrError> {
    content.items = overlay(
        db,
        "Items",
        columns::ITEMS,
        &content.items,
        |r: &ItemRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        |_| {},
    )?;
    content.rooms = overlay(
        db,
        "Rooms",
        columns::ROOMS,
        &content.rooms,
        |r: &RoomRecord| (i64::from(r.mapnum), i64::from(r.roomnum)),
        |row| Some((row.int("Map Number")?, row.int("Room Number")?)),
        |_| {},
    )?;
    content.spells = overlay(
        db,
        "Spells",
        columns::SPELLS,
        &content.spells,
        |r: &SpellRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        |_| {},
    )?;
    content.actions = overlay(
        db,
        "Actions",
        columns::ACTIONS,
        &content.actions,
        |r: &ActionRecord| r.name.trimmed().to_ascii_lowercase(),
        |row| {
            row.text("Action")
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
        },
        |_| {},
    )?;
    content.classes = overlay(
        db,
        "Classes",
        columns::CLASSES,
        &content.classes,
        |r: &ClassRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        |_| {},
    )?;
    content.races = overlay(
        db,
        "Races",
        columns::RACES,
        &content.races,
        |r: &RaceRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        |_| {},
    )?;
    content.shops = overlay(
        db,
        "Shops",
        columns::SHOPS,
        &content.shops,
        |r: &ShopRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        |_| {},
    )?;
    content.monsters = overlay(
        db,
        "Monsters",
        columns::MONSTERS,
        &content.monsters,
        |r: &MonsterRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        fix_monster,
    )?;
    content.messages = overlay(
        db,
        "Messages",
        columns::MESSAGES,
        &content.messages,
        |r: &MudmsgRecord| i64::from(r.number),
        |row| int_key(row, "Number"),
        |_| {},
    )?;
    content.text_blocks = text_blocks(db, &content.text_blocks);
    for (vir, table) in [
        ("WCCITEMS.VIR", "Items"),
        ("WCCMP001.VIR", "Rooms"),
        ("WCCSPELS.VIR", "Spells"),
        ("WCCACTS.VIR", "Actions"),
        ("WCCCLASS.VIR", "Classes"),
        ("WCCRACE.VIR", "Races"),
        ("WCCSHOPS.VIR", "Shops"),
        ("WCCKNMSR.VIR", "Monsters"),
        ("WCCMSG.VIR", "Messages"),
        ("WCCTEXT.VIR", "Textblocks"),
    ] {
        content
            .files
            .insert(vir.to_string(), format!("{} [{table}]", db.name));
    }
    Ok(())
}

/// NMR's experience rule (an award over 2147483646 becomes 65538 x 32767), and a fresh
/// world's counts: none alive, never killed.
fn fix_monster(r: &mut MonsterRecord) {
    let award = f64::from(r.exp as u32) * f64::from(r.level);
    if award > 2_147_483_646.0 {
        r.exp = 65_538;
        r.level = 32_767;
    }
    r.curalive = 0;
    r.killdate = 0;
    r.killtime = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp1252_round_trips_the_high_half() {
        for b in 0u8..=255 {
            let c = match b {
                0x80..=0x9F => [
                    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}',
                    '\u{2020}', '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}',
                    '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}',
                    '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}',
                    '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}',
                    '\u{0178}',
                ][usize::from(b - 0x80)],
                _ => char::from(b),
            };
            assert_eq!(cp1252(c), Some(b));
        }
        assert_eq!(cp1252('\u{263A}'), None);
    }

    #[test]
    fn text_blocks_are_shifted_and_drop_bytes_over_223() {
        let t = encrypt_text("a b\u{00F0}c");
        assert_eq!(
            &t[..4],
            &[b'a' + 0x20, b' ' + 0x20, b'b' + 0x20, b'c' + 0x20]
        );
        assert_eq!(t.len(), TEXT_LEN);
        assert!(t[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn ulong_wraps_into_a_signed_long() {
        assert_eq!(ulong_to_slong(4_294_967_295), Some(-1));
        assert_eq!(ulong_to_slong(42), Some(42));
        assert_eq!(ulong_to_slong(-1), None);
    }

    fn table(columns: &[&str], rows: Vec<Vec<Value>>) -> Table {
        Table {
            columns: columns
                .iter()
                .enumerate()
                .map(|(i, c)| (c.to_string(), i))
                .collect(),
            rows,
        }
    }

    fn message(number: i32, text: &str) -> Decoded<MudmsgRecord> {
        let mut buf = vec![0; MudmsgRecord::layout(LayoutSet::Dos).len];
        buf[..4].copy_from_slice(&number.to_le_bytes());
        buf[4..4 + text.len()].copy_from_slice(text.as_bytes());
        // a byte no column covers, which an update must keep
        buf[0xFB] = 7;
        Decoded {
            file: "WCCMSG.VIR".into(),
            ordinal: 1,
            record: MudmsgRecord::decode(&buf).unwrap(),
        }
    }

    #[test]
    fn rows_update_by_key_keep_other_bytes_and_add_new_records() {
        let db = Database {
            name: "realm.mdb".into(),
            tables: HashMap::from([(
                "Messages".to_string(),
                table(
                    &["Number", "Line 1", "Line 2", "Line 3"],
                    vec![
                        vec![
                            Value::Long(2),
                            Value::Text("  edited\0junk".into()),
                            Value::Null,
                            Value::Text("caf\u{e9}".into()),
                        ],
                        vec![
                            Value::Long(9),
                            Value::Text("new".into()),
                            Value::Text(String::new()),
                            Value::Text(String::new()),
                        ],
                    ],
                ),
            )]),
        };
        let base = vec![message(1, "one"), message(2, "two")];
        let out = overlay(
            &db,
            "Messages",
            columns::MESSAGES,
            &base,
            |r: &MudmsgRecord| i64::from(r.number),
            |row| int_key(row, "Number"),
            |_| {},
        )
        .unwrap();
        let got: Vec<(i32, String, String, String, &str)> = out
            .iter()
            .map(|d| {
                let r = &d.record;
                (
                    r.number,
                    r.msgs[0].trimmed(),
                    r.msgs[1].trimmed(),
                    r.msgs[2].trimmed(),
                    d.file.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (1, "one".into(), "".into(), "".into(), "WCCMSG.VIR"),
                // an empty cell leaves the line as it was; text stops at its NUL; 0xE9 in
                // Windows-1252 is the byte CP437 reads as theta
                (
                    2,
                    "edited".into(),
                    "".into(),
                    "caf\u{398}".into(),
                    "realm.mdb [Messages]"
                ),
                (
                    9,
                    "new".into(),
                    "".into(),
                    "".into(),
                    "realm.mdb [Messages]"
                ),
            ]
        );
        assert_eq!(
            out[1].record.unk_0x0FB.0[0], 7,
            "bytes no column covers are kept"
        );
    }

    #[test]
    fn a_value_that_overflows_its_member_is_skipped() {
        let t = table(
            &["Weight", "Cost Type"],
            vec![vec![Value::Long(70_000), Value::Long(300)]],
        );
        let row = Row {
            table: &t,
            values: &t.rows[0],
        };
        let mut buf = vec![0xAA; 0x430];
        for col in columns::ITEMS
            .iter()
            .filter(|c| c.name == "Weight" || c.name == "Cost Type")
        {
            put(&mut buf, col, &row);
        }
        assert!(buf.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn every_column_fits_its_record() {
        for (cols, len) in [
            (
                columns::ITEMS,
                ItemRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::ROOMS,
                RoomRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::SPELLS,
                SpellRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::ACTIONS,
                ActionRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::CLASSES,
                ClassRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::RACES,
                RaceRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::SHOPS,
                ShopRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::MONSTERS,
                MonsterRecord::layout(LayoutSet::Worldgroup).len,
            ),
            (
                columns::MESSAGES,
                MudmsgRecord::layout(LayoutSet::Worldgroup).len,
            ),
        ] {
            for c in cols {
                let size = match c.kind {
                    Kind::Byte => 1,
                    Kind::Integer => 2,
                    Kind::Long | Kind::ULong => 4,
                    Kind::Text(n) => n,
                };
                assert!(c.offset + size <= len, "{} at {:#x}", c.name, c.offset);
            }
        }
    }
}
