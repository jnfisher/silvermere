//! The stock world export as digests (T-120). The export MBBSEmu's database gave for the stock
//! world (the oracle of `tests/equivalence.rs` and `tests/post_update_equivalence.rs`) is not
//! kept in the tree, since it is the game's records. `tests/fixtures/world/DIGEST.json` keeps,
//! per table, what those tests compare: the records both sides have (their number and a sha256
//! of their ids) and, per compared field, a sha256 of every record's value. Run-time fields are
//! left out and rooms are masked before hashing, as the tests do. No value is stored, only hashes.
//!
//! A field's hash is the sha256 of one line per record, in `_id` order: `<_id>\t<value as
//! JSON>\n`, or `<_id>\t-\n` when the record has no such field; the first 16 hex digits are
//! kept. The ids' hash is the full sha256 of `<_id>\n` per record.

#![allow(dead_code)] // each test uses part of it

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

/// Hex digits kept of a field's sha256.
const FIELD_HEX: usize = 16;

/// One table's digest.
pub struct Table {
    /// The record type (the decoder to use).
    pub decoder: String,
    /// Records the export holds.
    pub rows: u64,
    /// Records compared: those both the export and the virgin import hold.
    pub compared: u64,
    /// sha256 of the compared ids.
    pub ids: String,
    /// Compared field -> hash.
    pub fields: BTreeMap<String, String>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The ids' hash of `rows`.
pub fn ids_hash<'a>(ids: impl Iterator<Item = &'a u64>) -> String {
    let mut h = Sha256::new();
    for id in ids {
        h.update(format!("{id}\n"));
    }
    hex(&h.finalize())
}

/// The hash of `field` over `rows` (only the ids in `only`, when given).
pub fn field_hash(
    rows: &BTreeMap<u64, Map<String, Value>>,
    field: &str,
    only: Option<&BTreeMap<u64, ()>>,
) -> String {
    let mut h = Sha256::new();
    for (id, row) in rows {
        if only.is_some_and(|o| !o.contains_key(id)) {
            continue;
        }
        match row.get(field) {
            Some(v) => h.update(format!("{id}\t{v}\n")),
            None => h.update(format!("{id}\t-\n")),
        }
    }
    hex(&h.finalize())[..FIELD_HEX].to_owned()
}

/// Reads `DIGEST.json`: table name -> digest.
pub fn load(path: &Path) -> BTreeMap<String, Table> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["tables"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, t)| {
            let fields = t["fields"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(f, h)| (f.clone(), h.as_str().unwrap().to_owned()))
                .collect();
            let table = Table {
                decoder: t["decoder"].as_str().unwrap().to_owned(),
                rows: t["rows"].as_u64().unwrap(),
                compared: t["compared"].as_u64().unwrap(),
                ids: t["ids_sha256"].as_str().unwrap().to_owned(),
                fields,
            };
            (name.clone(), table)
        })
        .collect()
}

/// Writes `DIGEST.json` from `tables`.
pub fn write(path: &Path, tables: &BTreeMap<String, Table>) {
    let mut out = Map::new();
    for (name, t) in tables {
        out.insert(
            name.clone(),
            json!({
                "decoder": t.decoder,
                "rows": t.rows,
                "compared": t.compared,
                "ids_sha256": t.ids,
                "fields": t.fields,
            }),
        );
    }
    let doc = json!({
        "format": 1,
        "about": "Digests of the stock world export MBBSEmu's database gave (post-update, after one start), per table: the records compared with the virgin import, the sha256 of their ids, and per compared field (run-time fields left out, rooms masked) the first 16 hex digits of a sha256 over every record's value. See tests/support/world_digest.rs.",
        "dist_zip_sha256": "815f7b35cdbc6c4de8deb0b686031e34905da147f360655858058a1f642d3583",
        "tables": out,
    });
    std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
}

/// Checks `ours` (one table's rows as export objects, masked as the digest was) against
/// `digest`; returns what differs, as `TABLE.field` (or `TABLE: ...`) entries.
pub fn check(name: &str, ours: &BTreeMap<u64, Map<String, Value>>, digest: &Table) -> Vec<String> {
    let mut failures = Vec::new();
    if ours.len() as u64 != digest.compared || ids_hash(ours.keys()) != digest.ids {
        failures.push(format!(
            "{name}: {} records, the export's compared set has {} (or other ids)",
            ours.len(),
            digest.compared
        ));
        return failures;
    }
    for (field, want) in &digest.fields {
        if &field_hash(ours, field, None) != want {
            failures.push(format!("{name}.{field}"));
        }
    }
    failures
}
