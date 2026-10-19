//! Importer equivalence (plan `07` Layer 4): every record this crate decodes equals the
//! row T-006's export holds for the same `_id`, on every field both have.
//!
//! The export is the game's records, so the tree keeps only its digests,
//! `tests/fixtures/world/DIGEST.json` (`tests/support/world_digest.rs`): per table, the records
//! compared and a hash per compared field. `MMUD_WORLD_JSON` names an export directory
//! (`MANIFEST.json` and `<TABLE>.jsonl.gz`) to compare against record by record instead, and
//! with `UPDATE_WORLD_DIGEST` set rewrites the digests from it. The `.VIR` files are the stock
//! distribution's (the testkit's `stock` helper: downloaded once, checked, unpacked) unless
//! `MMUD_DIST_DIR` (another unpacked copy) is set. The field names are the record member
//! names on both sides.
//!
//! The export was taken from an instance that started once, so its run-time fields hold
//! what the start-up cycle wrote, while the `.VIR` holds the virgin state. Those fields
//! ([`RUNTIME`], each with what changes it) are reported, not compared: the
//! import's state equals the `.VIR` (`tests/invariants.rs`), not a running game.
//!
//! The export is also post-update (`post_update: true`) and the `.VIR` files are not; on
//! the stock set no content field this test compares differs anyway (the patch stream's
//! images equal the virgin records but for the start-up relock of four doors, masked
//! below, and the text blocks, which equal the virgin ones in the export).
//! `tests/post_update_equivalence.rs` compares the patched content. Any difference fails.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};
use silvermere_import::decode::{self, Record};
use silvermere_import::importer::{Decoded, read_file};
use silvermere_import::json::ExportRecord;

use crate::support::world_digest;

#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock;

fn world_json() -> PathBuf {
    std::env::var_os("MMUD_WORLD_JSON")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/world"))
}

/// The directory holding the `.VIR` files.
fn vir_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MMUD_DIST_DIR") {
        return PathBuf::from(dir);
    }
    stock::vir_dir()
}

/// The rows of `<table>.jsonl.gz` by `_id`.
fn json_rows(dir: &Path, file: &str) -> HashMap<u64, Map<String, Value>> {
    let out = Command::new("gzip")
        .arg("-dc")
        .arg(dir.join(file))
        .output()
        .expect("`gzip` could not be run");
    assert!(out.status.success(), "gzip -dc {file} failed");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let Value::Object(row) = serde_json::from_str::<Value>(l).unwrap() else {
                panic!("{file}: a line is not an object");
            };
            (row["_id"].as_u64().unwrap(), row)
        })
        .collect()
}

/// Our records of `file` as export objects keyed by `_id`.
fn ours<R: ExportRecord>(
    dir: &Path,
    file: &str,
    decode: impl Fn(&[u8]) -> Result<R, decode::DecodeError>,
) -> BTreeMap<u64, Map<String, Value>> {
    let rows: Vec<Decoded<R>> = read_file(dir, file, decode).unwrap();
    rows.iter()
        .map(|d| (u64::from(d.ordinal), d.record.export(d.ordinal)))
        .collect()
}

fn decode_table(dir: &Path, table: &str, decoder: &str) -> BTreeMap<u64, Map<String, Value>> {
    let file = format!("{table}.VIR");
    match decoder {
        "room" => ours(dir, &file, decode::RoomRecord::decode),
        "item" => ours(dir, &file, decode::ItemRecord::decode),
        "monster" => ours(dir, &file, decode::MonsterRecord::decode),
        "actmon" => ours(dir, &file, decode::ActmonRecord::decode),
        "spell" => ours(dir, &file, decode::SpellRecord::decode),
        "class" => ours(dir, &file, decode::ClassRecord::decode),
        "race" => ours(dir, &file, decode::RaceRecord::decode),
        "shop" => ours(dir, &file, decode::ShopRecord::decode),
        "mudmsg" => ours(dir, &file, decode::MudmsgRecord::decode),
        "textblk" => ours(dir, &file, decode::decode_textblk),
        "action" => ours(dir, &file, decode::ActionRecord::decode),
        "gang" => ours(dir, &file, decode::GangRecord::decode),
        "bankbook" => ours(dir, &file, decode::BankbookRecord::decode),
        "itemown" => ours(dir, &file, decode::ItemownRecord::decode),
        "player" => ours(dir, &file, decode::PlayerRecord::decode),
        other => panic!("{table}: the manifest names struct {other}, which has no decoder"),
    }
}

/// Run-time fields per table: written by the game while it runs, so an export taken
/// after a start differs from the virgin `.VIR` there.
const RUNTIME: &[(&str, &[&str])] = &[
    // item `active`, copies in play (counted as monsters are generated).
    ("WCCITEMS", &["active"]),
    // curalive, killdate, killtime.
    ("WCCKNMSR", &["curalive", "killdate", "killtime"]),
    // run-time members of the room record (monsters standing here,
    // floor items and coins, area counts, the cache byte).
    (
        "WCCMP001",
        &[
            "monsters",
            "nummonsters",
            "areacount",
            "lastkill",
            "foundmask",
            "searchers",
            "items",
            "itemuses",
            "itemcounts",
            "hiditems",
            "hiduses",
            "hidcounts",
            "runic",
            "platinum",
            "gold",
            "silver",
            "copper",
            "hidrunic",
            "hidplatinum",
            "hidgold",
            "hidsilver",
            "hidcopper",
            "changed",
        ],
    ),
];

/// `RF_SPECALIVE`: set while the room's specific monster lives.
const RF_SPECALIVE: u64 = 0x08;
/// `EXIT_DOOR` and `EXIT_GATE` codes (`mmud_data.EXITTYPES` 7 and 11).
const DOOR_LIKE: [i64; 2] = [7, 11];

/// Clears the run-time parts of a room row that share a member with content: the
/// `RF_SPECALIVE` bit of `flags`, and `exitpar1s` of door and gate exits, which the
/// start-up preload relocks to 2.
fn mask_room(row: &mut Map<String, Value>) {
    if let Some(flags) = row.get("flags").and_then(Value::as_u64) {
        row.insert("flags".into(), Value::from(flags & !RF_SPECALIVE));
    }
    let types: Vec<i64> = row
        .get("exittypes")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_i64().unwrap_or(0)).collect())
        .unwrap_or_default();
    if let Some(Value::Array(par1)) = row.get_mut("exitpar1s") {
        for (p, t) in par1.iter_mut().zip(&types) {
            if DOOR_LIKE.contains(t) {
                *p = Value::from(0);
            }
        }
    }
}

#[derive(Default)]
struct TableDiff {
    compared: usize,
    only_ours: usize,
    only_theirs: usize,
    /// field -> (records differing, first example)
    fields: BTreeMap<String, (usize, String)>,
}

fn compare(
    ours: &BTreeMap<u64, Map<String, Value>>,
    theirs: &HashMap<u64, Map<String, Value>>,
    rooms: bool,
) -> TableDiff {
    let mut d = TableDiff::default();
    for (id, row) in ours {
        let Some(other) = theirs.get(id) else {
            d.only_ours += 1;
            continue;
        };
        d.compared += 1;
        let (mut row, mut other) = (row.clone(), other.clone());
        if rooms {
            mask_room(&mut row);
            mask_room(&mut other);
        }
        for (key, value) in &row {
            let Some(theirs) = other.get(key) else {
                continue;
            };
            if value != theirs {
                let e = d.fields.entry(key.clone()).or_insert((0, String::new()));
                if e.0 == 0 {
                    let show = |v: &Value| {
                        let s = v.to_string();
                        if s.len() > 120 {
                            format!("{}...", &s[..120])
                        } else {
                            s
                        }
                    };
                    e.1 = format!("_id {id}: ours {} theirs {}", show(value), show(theirs));
                }
                e.0 += 1;
            }
        }
    }
    d.only_theirs = theirs.keys().filter(|id| !ours.contains_key(id)).count();
    d
}

fn digest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/world/DIGEST.json")
}

fn runtime_fields(table: &str) -> &'static [&'static str] {
    RUNTIME
        .iter()
        .find(|(t, _)| *t == table)
        .map_or(&[], |(_, f)| f)
}

/// Our rows of `table`, masked as the digest was.
fn ours_masked(vir: &Path, table: &str, decoder: &str) -> BTreeMap<u64, Map<String, Value>> {
    let mut ours = decode_table(vir, table, decoder);
    if decoder == "room" {
        ours.values_mut().for_each(mask_room);
    }
    ours
}

// The importer-equivalence gate (plan 07 Layer 4, 04 "Equivalence and invariants"), against
// the export's digests (tests/support/world_digest.rs): every compared field of every record
// the virgin import and the export share hashes the same.
#[test]
fn every_record_equals_the_export_digest() {
    if std::env::var_os("MMUD_WORLD_JSON").is_some() {
        return every_record_equals_an_export();
    }
    let vir = vir_dir();
    let mut failures = Vec::new();
    for (table, digest) in world_digest::load(&digest_path()) {
        let ours = ours_masked(&vir, &table, &digest.decoder);
        let f = world_digest::check(&table, &ours, &digest);
        println!(
            "{table:<9} ours {:>6} export {:>6} compared {:>6} fields {:>3} differing {}",
            ours.len(),
            digest.rows,
            digest.compared,
            digest.fields.len(),
            f.len()
        );
        failures.extend(f);
    }
    assert!(failures.is_empty(), "differences: {failures:?}");
}

/// The digest of the export `theirs`, over the records it shares with `ours` and the fields
/// both have, run-time ones left out, rooms masked.
fn digest_of(
    ours: &BTreeMap<u64, Map<String, Value>>,
    theirs: &HashMap<u64, Map<String, Value>>,
    rooms: bool,
    runtime: &[&str],
    decoder: &str,
) -> world_digest::Table {
    let mut shared: BTreeMap<u64, Map<String, Value>> = BTreeMap::new();
    let mut only = BTreeMap::new();
    for id in ours.keys().filter(|id| theirs.contains_key(id)) {
        let mut row = theirs[id].clone();
        if rooms {
            mask_room(&mut row);
        }
        shared.insert(*id, row);
        only.insert(*id, ());
    }
    let mut fields = std::collections::BTreeSet::new();
    for (id, row) in ours {
        if let Some(other) = theirs.get(id) {
            fields.extend(row.keys().filter(|k| other.contains_key(*k)).cloned());
        }
    }
    fields.retain(|f| !runtime.contains(&f.as_str()));
    world_digest::Table {
        decoder: decoder.to_owned(),
        rows: theirs.len() as u64,
        compared: shared.len() as u64,
        ids: world_digest::ids_hash(shared.keys()),
        fields: fields
            .into_iter()
            .map(|f| {
                let h = world_digest::field_hash(&shared, &f, Some(&only));
                (f, h)
            })
            .collect(),
    }
}

/// A wrong value in one record, or a missing record, fails the digest check, naming it.
#[test]
fn a_wrong_record_fails_the_digest() {
    let digests = world_digest::load(&digest_path());
    let digest = &digests["WCCCLASS"];
    let mut ours = ours_masked(&vir_dir(), "WCCCLASS", "class");
    assert!(world_digest::check("WCCCLASS", &ours, digest).is_empty());
    let hp = ours.get_mut(&3).unwrap().get_mut("basehp").unwrap();
    *hp = Value::from(hp.as_i64().unwrap() + 1);
    assert_eq!(
        world_digest::check("WCCCLASS", &ours, digest),
        ["WCCCLASS.basehp"]
    );
    ours.remove(&3);
    assert!(world_digest::check("WCCCLASS", &ours, digest)[0].starts_with("WCCCLASS: 14 records"));
}

/// With `MMUD_WORLD_JSON` naming an export directory (`MANIFEST.json` and
/// `<TABLE>.jsonl.gz`): the record-by-record comparison against it, reporting each field that
/// differs; with `UPDATE_WORLD_DIGEST` set too, it rewrites `DIGEST.json` from that export.
fn every_record_equals_an_export() {
    let world = world_json();
    let vir = vir_dir();
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(world.join("MANIFEST.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    let mut digests = BTreeMap::new();
    for (table, t) in manifest["tables"].as_object().unwrap() {
        let decoder = t["decoder"].as_str().unwrap();
        let file = t["file"].as_str().unwrap();
        let ours = decode_table(&vir, table, decoder);
        let theirs = json_rows(&world, file);
        let rooms = decoder == "room";
        let runtime = runtime_fields(table);
        digests.insert(
            table.clone(),
            digest_of(&ours, &theirs, rooms, runtime, decoder),
        );
        let d = compare(&ours, &theirs, rooms);
        println!(
            "{table:<9} {decoder:<9} ours {:>6} theirs {:>6} compared {:>6} only-ours {} only-theirs {} differing fields {}",
            ours.len(),
            theirs.len(),
            d.compared,
            d.only_ours,
            d.only_theirs,
            d.fields.len()
        );
        for (field, (n, example)) in &d.fields {
            let verdict = if runtime.contains(&field.as_str()) {
                "run-time state"
            } else {
                failures.push(format!("{table}.{field}"));
                "FAIL"
            };
            println!("    {field}: {n} records differ [{verdict}]; first {example}");
        }
        // Rows only in the export are run-time state of an instance that ran (WCCACMSR,
        // WCCITOWN start empty in the virgin set); rows only here would be a reader bug.
        if d.only_ours > 0 {
            failures.push(format!(
                "{table}: {} records not in the export",
                d.only_ours
            ));
        }
    }
    assert!(failures.is_empty(), "differences: {failures:?}");
    if std::env::var_os("UPDATE_WORLD_DIGEST").is_some() {
        world_digest::write(&digest_path(), &digests);
    }
}
