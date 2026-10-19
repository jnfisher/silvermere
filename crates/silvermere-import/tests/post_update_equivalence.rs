//! Post-update equivalence (plan `07` Layer 4, card T-009): the `.VIR` set decoded and
//! patched with `WCCUPDAT.DAT` equals T-006's post-update export (`post_update: true`)
//! on every content field of every table, rooms included, with no allowance for patches.
//!
//! Inputs as `tests/equivalence.rs`: the export's digests in `tests/fixtures/world/DIGEST.json`
//! (or the export itself, `MMUD_WORLD_JSON`), and the stock `.VIR` set and `WCCUPDAT.DAT` from
//! the stock distribution (the testkit's `stock` helper; or `MMUD_DIST_DIR`).
//!
//! The run-time fields the export's start-up cycle wrote are reported, not compared, as in
//! `tests/equivalence.rs`; every other field of every table must be equal (the text-block
//! merge keeps the bytes a 2015-byte record does not carry, `docs/import.md` "Patches").
//! `tests/patched_fields.json` records what the patch stream
//! changed (records per table and field, patched against unpatched); the test checks the
//! file is current (`UPDATE_PATCHED_FIELDS=1` rewrites it).

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::{Map, Value};
use silvermere_db::{Db, TestDb};
use silvermere_import::importer::{Content, Decoded, ImportOptions, Importer, read_content};
use silvermere_import::json::ExportRecord;
use silvermere_import::patch::{PatchKind, PatchReport, apply_patches};
use sqlx::{Column, Row};

use crate::support::world_digest;

#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock_data;

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn world_json() -> PathBuf {
    std::env::var_os("MMUD_WORLD_JSON")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir().join("tests/fixtures/world"))
}

/// The `.VIR` files and `WCCUPDAT.DAT`, in one directory.
fn dist() -> &'static Path {
    static DIST: OnceLock<PathBuf> = OnceLock::new();
    DIST.get_or_init(|| {
        std::env::var_os("MMUD_DIST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(stock_data::dir)
    })
}

/// The decoded set, unpatched and patched (with the report).
fn worlds() -> &'static (Content, Content, PatchReport) {
    static W: OnceLock<(Content, Content, PatchReport)> = OnceLock::new();
    W.get_or_init(|| {
        let virgin = read_content(&ImportOptions::new(dist())).unwrap();
        let mut patched = virgin.clone();
        let report = apply_patches(&mut patched, &dist().join("WCCUPDAT.DAT")).unwrap();
        (virgin, patched, report)
    })
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

fn rows<R: ExportRecord>(v: &[Decoded<R>]) -> BTreeMap<u64, Map<String, Value>> {
    v.iter()
        .map(|d| (u64::from(d.ordinal), d.record.export(d.ordinal)))
        .collect()
}

/// One table of `c` as export rows, by the manifest's table name.
fn table(c: &Content, name: &str) -> BTreeMap<u64, Map<String, Value>> {
    match name {
        "WCCMP001" => rows(&c.rooms),
        "WCCITEMS" => rows(&c.items),
        "WCCKNMSR" => rows(&c.monsters),
        "WCCACMSR" => rows(&c.active_monsters),
        "WCCSPELS" => rows(&c.spells),
        "WCCCLASS" => rows(&c.classes),
        "WCCRACE" => rows(&c.races),
        "WCCSHOPS" => rows(&c.shops),
        "WCCMSG" => rows(&c.messages),
        "WCCTEXT" => rows(&c.text_blocks),
        "WCCACTS" => rows(&c.actions),
        "WCCGANGS" => rows(&c.gangs),
        "WCCBANKS" => rows(&c.bank_books),
        "WCCITOWN" => rows(&c.item_ownership),
        "WCCUSERS" => rows(&c.users),
        other => panic!("{other}: not a table of the set"),
    }
}

/// Run-time fields per table (as `tests/equivalence.rs`): written by the game while it
/// runs, so the export, taken after a start, holds the start-up's values.
const RUNTIME: &[(&str, &[&str])] = &[
    // item `active`, copies in play.
    ("WCCITEMS", &["active"]),
    // monster state.
    ("WCCKNMSR", &["curalive", "killdate", "killtime"]),
    // run-time members of the room record.
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

/// `RF_SPECALIVE`.
const RF_SPECALIVE: u64 = 0x08;
/// `EXIT_DOOR`, `EXIT_GATE`.
const DOOR_LIKE: [i64; 2] = [7, 11];

/// The run-time parts of a room that share a member with content: the `RF_SPECALIVE` bit
/// and `exitpar1s` of doors and gates (relocked to 2 by the start-up preload and by the
/// room merge).
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

/// Fields that differ between two tables: field -> (records, first example).
fn diff(
    ours: &BTreeMap<u64, Map<String, Value>>,
    theirs: &BTreeMap<u64, Map<String, Value>>,
    rooms: bool,
) -> BTreeMap<String, (usize, String)> {
    let mut fields: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (id, row) in ours {
        let Some(other) = theirs.get(id) else {
            fields.entry("<only ours>".into()).or_default().0 += 1;
            continue;
        };
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
                let e = fields.entry(key.clone()).or_default();
                if e.0 == 0 {
                    let show = |v: &Value| {
                        let s = v.to_string();
                        if s.len() > 100 {
                            format!("{}...", &s[..100])
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
    fields
}

fn manifest() -> Value {
    serde_json::from_str(&std::fs::read_to_string(world_json().join("MANIFEST.json")).unwrap())
        .unwrap()
}

// The update polling: every record, in seqnum order.
#[test]
fn the_patch_report_covers_every_updrec() {
    let (_, _, report) = worlds();
    print!("{report}");
    // tests/stock_vir.rs: 38,754 records, seqnum 1..38754 in physical order.
    assert_eq!(report.total(), 38_754);
    assert_eq!(report.not_reached, 0);
    assert_eq!((report.first_seq, report.last_seq), (Some(1), Some(38_754)));
    let n = |k| report.applied_by_kind.get(&k).copied().unwrap_or(0);
    // The stream holds one image of every stock record and nothing new.
    assert!(report.inserted_by_kind.is_empty(), "{report}");
    assert_eq!(n(PatchKind::Room), 26_720);
    assert_eq!(n(PatchKind::Item), 1_950);
    assert_eq!(n(PatchKind::Text), 3_467);
}

fn digest_path() -> PathBuf {
    manifest_dir().join("tests/fixtures/world/DIGEST.json")
}

/// The tables of the set, by the export's names.
fn table_names() -> Vec<String> {
    world_digest::load(&digest_path()).into_keys().collect()
}

// The importer-equivalence gate, post-update (plan 07 Layer 4, card T-009), against the
// export's digests (tests/support/world_digest.rs): every compared field of every record
// hashes the same once the patches are applied.
#[test]
fn patched_content_equals_the_post_update_digest() {
    if std::env::var_os("MMUD_WORLD_JSON").is_some() {
        return patched_content_equals_an_export();
    }
    let (_, patched, _) = worlds();
    let mut failures = Vec::new();
    for (name, digest) in world_digest::load(&digest_path()) {
        let mut ours = table(patched, &name);
        if name == "WCCMP001" {
            ours.values_mut().for_each(mask_room);
        }
        let f = world_digest::check(&name, &ours, &digest);
        println!("{name:<9} ours {:>6} differing {}", ours.len(), f.len());
        failures.extend(f);
    }
    assert!(failures.is_empty(), "differences: {failures:?}");
}

/// With `MMUD_WORLD_JSON` naming an export directory: the record-by-record comparison
/// against it, reporting each field that differs.
fn patched_content_equals_an_export() {
    let (_, patched, _) = worlds();
    let world = world_json();
    let manifest = manifest();
    assert_eq!(manifest["post_update"].as_bool(), Some(true));
    let mut failures = Vec::new();
    for (name, t) in manifest["tables"].as_object().unwrap() {
        let ours = table(patched, name);
        let theirs: BTreeMap<u64, Map<String, Value>> =
            json_rows(&world, t["file"].as_str().unwrap())
                .into_iter()
                .collect();
        let runtime: &[&str] = RUNTIME
            .iter()
            .find(|(t, _)| t == name)
            .map_or(&[], |(_, f)| f);
        let fields = diff(&ours, &theirs, name == "WCCMP001");
        println!(
            "{name:<9} ours {:>6} theirs {:>6} differing fields {}",
            ours.len(),
            theirs.len(),
            fields.len()
        );
        for (field, (n, example)) in &fields {
            let verdict = if runtime.contains(&field.as_str()) {
                "run-time state"
            } else {
                failures.push(format!("{name}.{field} ({n})"));
                "FAIL"
            };
            println!("    {field}: {n} records differ [{verdict}]; first {example}");
        }
    }
    assert!(failures.is_empty(), "differences: {failures:?}");
}

/// What the patch stream changed: table -> field -> records, patched against unpatched.
fn patched_fields() -> Value {
    let (virgin, patched, _) = worlds();
    let mut out = Map::new();
    for name in table_names() {
        let fields = diff(&table(patched, &name), &table(virgin, &name), false);
        if fields.is_empty() {
            continue;
        }
        out.insert(
            name.clone(),
            Value::Object(
                fields
                    .into_iter()
                    .map(|(f, (n, _))| (f, Value::from(n)))
                    .collect(),
            ),
        );
    }
    Value::Object(out)
}

// tests/patched_fields.json is the record of what WCCUPDAT.DAT changes (docs/import.md).
#[test]
fn patched_fields_json_is_current() {
    let path = manifest_dir().join("tests/patched_fields.json");
    let now = patched_fields();
    let text = serde_json::to_string_pretty(&now).unwrap() + "\n";
    if std::env::var_os("UPDATE_PATCHED_FIELDS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    println!("{text}");
    assert_eq!(
        committed, text,
        "tests/patched_fields.json is stale (UPDATE_PATCHED_FIELDS=1 rewrites it)"
    );
}

// ---------------------------------------------------------------------------------------
// Idempotence of the patch pass (card T-009 "Done when"; plan 04 "Also used by tools":
// a `--content-only` re-import with patches is world patching).

/// The 25 content tables (as `tests/invariants.rs`).
const CONTENT_TABLES: &[&str] = &[
    "rooms",
    "room_exits",
    "room_placed_items",
    "items",
    "item_abilities",
    "item_classes",
    "item_races",
    "item_negates",
    "monsters",
    "monster_abilities",
    "monster_attacks",
    "monster_items",
    "monster_spells",
    "spells",
    "spell_abilities",
    "classes",
    "class_abilities",
    "races",
    "race_abilities",
    "shops",
    "shop_items",
    "messages",
    "text_blocks",
    "actions",
    "action_texts",
];

/// A hash of every row of `table` in a stable order, every column but a child table's
/// surrogate `id` and the state column `items.active` (as `tests/invariants.rs`).
async fn checksum(db: &Db, table: &str) -> (usize, u64) {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}")))
        .fetch_all(&db.pool())
        .await
        .unwrap();
    let parent = matches!(table, "rooms" | "actions");
    let skip = |name: &str| (!parent && name == "id") || (table == "items" && name == "active");
    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| {
            row.columns()
                .iter()
                .filter(|c| !skip(c.name()))
                .map(|c| {
                    let i = c.ordinal();
                    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
                        format!("{v:?}")
                    } else if let Ok(v) = row.try_get::<Option<String>, _>(i) {
                        format!("{v:?}")
                    } else if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
                        format!("{v:?}")
                    } else {
                        panic!("{table}.{}: no text, integer or boolean", c.name())
                    }
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    lines.sort();
    let mut h = DefaultHasher::new();
    lines.hash(&mut h);
    (lines.len(), h.finish())
}

async fn content_checksums(db: &Db) -> Vec<(&'static str, usize, u64)> {
    let mut out = Vec::new();
    for t in CONTENT_TABLES {
        let (n, h) = checksum(db, t).await;
        out.push((*t, n, h));
    }
    out
}

async fn import(db: &Db, f: impl FnOnce(&mut ImportOptions)) -> silvermere_import::Report {
    let mut opts = ImportOptions::new(dist());
    f(&mut opts);
    Importer::new(opts, Some(db)).run().await.expect("import")
}

#[tokio::test]
async fn content_only_patching_equals_a_patched_import() {
    // `--no-patches`, then `--content-only` with the patches.
    let two_step = TestDb::from_env().await.unwrap();
    let report = import(&two_step, |o| o.no_patches = true).await;
    assert!(report.patches.is_none());
    let unpatched = content_checksums(&two_step).await;
    let report = import(&two_step, |o| o.content_only = true).await;
    assert_eq!(report.patches.map(|p| p.total()), Some(38_754));
    let patched_later = content_checksums(&two_step).await;
    two_step.close().await;

    // One patched import.
    let one_step = TestDb::from_env().await.unwrap();
    let report = import(&one_step, |_| {}).await;
    print!("{report}");
    let patched = content_checksums(&one_step).await;
    // And the pass again on patched content changes nothing.
    import(&one_step, |o| o.content_only = true).await;
    let again = content_checksums(&one_step).await;
    one_step.close().await;

    assert_eq!(patched_later, patched);
    assert_eq!(again, patched);
    let changed: Vec<&str> = unpatched
        .iter()
        .zip(&patched)
        .filter(|(a, b)| a != b)
        .map(|(a, _)| a.0)
        .collect();
    println!("content tables the patches change: {changed:?}");
}
