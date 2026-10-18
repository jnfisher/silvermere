//! The stock records the engine and rules tests run on, built at test time from the
//! downloaded distribution ([`silvermere_testkit::stock`]): its sixteen `.VIR` files decoded
//! by the importer, each record in the shape `silvermere import --json` writes (one object
//! per record, keyed by the record's member names).
//!
//! The tree holds no game records; these functions select what each test reads.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{Map, Value};
use silvermere_import::importer::{Content, Decoded, ImportOptions, read_content};
use silvermere_import::json::ExportRecord;

/// A record as `--json` writes it.
pub type Row = Map<String, Value>;

/// The `.VIR` set decoded once per test binary.
pub fn content() -> &'static Content {
    static CONTENT: OnceLock<Content> = OnceLock::new();
    CONTENT.get_or_init(|| {
        read_content(&ImportOptions::new(silvermere_testkit::stock::vir_dir()))
            .expect("the stock .VIR set decodes")
    })
}

fn export<R: ExportRecord>(v: &[Decoded<R>], file: Option<&str>) -> Vec<Row> {
    v.iter()
        .filter(|d| file.is_none_or(|f| d.file.eq_ignore_ascii_case(f)))
        .map(|d| d.record.export(d.ordinal))
        .collect()
}

/// Every record of `table` (`WCCITEMS`, `WCCMP001`, ...), in file order, built once.
pub fn table(table: &str) -> &'static [Row] {
    static TABLES: OnceLock<Mutex<HashMap<String, &'static [Row]>>> = OnceLock::new();
    let mut tables = TABLES.get_or_init(Default::default).lock().unwrap();
    if let Some(rows) = tables.get(table) {
        return rows;
    }
    let c = content();
    let rows = match table {
        "WCCMP001" | "NEWMP001" => export(&c.rooms, Some(&format!("{table}.VIR"))),
        "WCCITEMS" => export(&c.items, None),
        "WCCKNMSR" => export(&c.monsters, None),
        "WCCSPELS" => export(&c.spells, None),
        "WCCCLASS" => export(&c.classes, None),
        "WCCRACE" => export(&c.races, None),
        "WCCSHOPS" => export(&c.shops, None),
        "WCCMSG" => export(&c.messages, None),
        "WCCTEXT" => export(&c.text_blocks, None),
        "WCCACTS" => export(&c.actions, None),
        other => panic!("no table {other}"),
    };
    let rows: &'static [Row] = Vec::leak(rows);
    tables.insert(table.to_string(), rows);
    rows
}

/// The integer at `key`.
pub fn int(row: &Row, key: &str) -> i64 {
    row.get(key)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("{key} in {row:?}"))
}

/// The integers of the array at `key`.
pub fn ints(row: &Row, key: &str) -> Vec<i64> {
    row.get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{key} in {row:?}"))
        .iter()
        .map(|v| v.as_i64().expect("an integer"))
        .collect()
}

/// The text at `key`.
pub fn text<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{key} in {row:?}"))
}

/// The record of `table` whose `number` is `n`.
pub fn by_number(table_name: &str, n: i64) -> &'static Row {
    table(table_name)
        .iter()
        .find(|r| int(r, "number") == n)
        .unwrap_or_else(|| panic!("{table_name} has no record {n}"))
}

/// The map 1 room `n`.
pub fn room(n: i64) -> &'static Row {
    table("WCCMP001")
        .iter()
        .find(|r| int(r, "mapnum") == 1 && int(r, "roomnum") == n)
        .unwrap_or_else(|| panic!("map 1 has no room {n}"))
}

/// The `[ability, value]` pairs of a record whose ability number is set, in slot order.
pub fn ability_pairs(row: &Row) -> Value {
    let codes = ints(row, "abilities");
    let vals = ints(row, "abilvals");
    Value::Array(
        codes
            .iter()
            .zip(&vals)
            .filter(|(c, _)| **c != 0)
            .map(|(c, v)| Value::from(vec![*c, *v]))
            .collect(),
    )
}

/// `row` with `abilities` as [`ability_pairs`].
pub fn with_ability_pairs(row: &Row) -> Value {
    let mut out = row.clone();
    out.insert("abilities".into(), ability_pairs(row));
    Value::Object(out)
}
