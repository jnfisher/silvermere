//! The Btrieve reader against the stock MajorMUD 1.11p distribution.
//!
//! The 16 virgin `.VIR` files and `WCCUPDAT.DAT` (80 MB) come from the stock distribution
//! (the testkit's `stock` helper: downloaded once, checked and unpacked into a cache);
//! `MMUD_DIST_DIR` (another unpacked copy) takes precedence for `WCCUPDAT.DAT`. Run with
//! `--nocapture` to see the per-file counts.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use silvermere_import::btrieve::{
    BtrieveError, BtrieveFile, KeyDataType as T, OpenOptions, Version,
};

#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock_data;

/// The stock `.VIR` files.
fn stock() -> PathBuf {
    stock_data::vir_dir()
}

/// A directory holding `WCCUPDAT.DAT`: the environment's distribution when it names one,
/// else the stock one.
fn dist() -> &'static Path {
    static DIST: OnceLock<PathBuf> = OnceLock::new();
    DIST.get_or_init(|| {
        std::env::var_os("MMUD_DIST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(stock_data::dir)
    })
}

fn open(dir: &Path, name: &str) -> BtrieveFile {
    OpenOptions::new()
        .variable_length(name == "WCCTEXT.VIR")
        .open(&dir.join(name))
        .unwrap_or_else(|e| panic!("{e}"))
}

/// (file, record length the game opens it with, records found in the stock file).
/// Lengths: what the game passes to the host's `opnbtv`.
const STOCK: &[(&str, usize, u32)] = &[
    ("WCCRACE.VIR", 126, 13),
    ("WCCCLASS.VIR", 153, 15),
    ("WCCUSERS.VIR", 1998, 0),
    ("WCCMP001.VIR", 1528, 26_720),
    ("WCCACMSR.VIR", 386, 0),
    ("WCCSHOPS.VIR", 500, 178),
    ("WCCITEMS.VIR", 1061, 1950),
    ("WCCITOWN.VIR", 65, 0),
    ("WCCSPELS.VIR", 253, 1379),
    ("WCCKNMSR.VIR", 732, 1101),
    ("WCCBANKS.VIR", 72, 0),
    ("WCCMSG.VIR", 253, 3867),
    // Variable-length: a 22-byte fixed part and up to 2022 bytes (`opnbtv` buffer).
    ("WCCTEXT.VIR", 2022, 3467),
    ("WCCGANGS.VIR", 253, 0),
    ("WCCACTS.VIR", 1010, 64),
    // Not opened by the game; a v6 file with 1544-byte records (see `newmp001`).
    ("NEWMP001.VIR", 1544, 0),
];

// Every stock .VIR opens with the length the game passes to `opnbtv`.
#[test]
fn every_stock_vir_opens_and_enumerates_its_header_count() {
    let dir = &stock();
    let mut present: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_ascii_uppercase().ends_with(".VIR"))
        .collect();
    present.sort();
    let mut expected: Vec<String> = STOCK.iter().map(|(n, ..)| n.to_string()).collect();
    expected.sort();
    assert_eq!(present, expected, "the distribution's .VIR set changed");

    println!(
        "{:<14} {:>3} {:>5} {:>6} {:>7} {:>7}",
        "file", "ver", "page", "reclen", "header", "read"
    );
    for &(name, len, count) in STOCK {
        let mut file = open(dir, name);
        file.expect_record_len(len).unwrap();
        let h = file.header.clone();
        let mut records = file.records();
        let mut read = 0u32;
        while let Some(record) = records.next_record() {
            let record = record.unwrap();
            read += 1;
            assert!(
                record.data.len() <= len,
                "{name}: record {} too long",
                record.ordinal
            );
        }
        println!(
            "{name:<14} {:>3} {:>5} {:>6} {:>7} {read:>7}",
            if h.version == Version::V5 { "v5" } else { "v6" },
            h.page_length,
            h.record_length,
            h.record_count
        );
        assert_eq!(read, h.record_count, "{name}");
        assert_eq!(read, count, "{name}");
    }
}

// The stock world has 26,720 rooms. The 2 s bound is a smoke test, not a
// benchmark; it holds in debug builds too.
#[test]
fn wccmp001_enumerates_26720_rooms() {
    let dir = &stock();
    let mut file = open(dir, "WCCMP001.VIR");
    assert_eq!(
        file.header.page_length, 1536,
        "read from 0x08, not WCCMMUD.MDF's 2048"
    );
    let started = Instant::now();
    let mut records = file.records();
    let mut previous = None;
    while let Some(record) = records.next_record() {
        let record = record.unwrap();
        assert_eq!(record.data.len(), 1528);
        previous = Some(record.ordinal);
    }
    let elapsed = started.elapsed();
    println!(
        "WCCMP001.VIR: {} records in {elapsed:?}",
        previous.unwrap_or(0)
    );
    assert_eq!(previous, Some(26_720));
    assert!(elapsed.as_secs_f64() < 2.0, "took {elapsed:?}");
}

// WCCTEXT is variable-length: 22 fixed bytes plus fragments, 2022 bytes per record.
#[test]
fn wcctext_is_variable_length() {
    let dir = &stock();
    let err = BtrieveFile::open(&dir.join("WCCTEXT.VIR")).unwrap_err();
    assert!(matches!(err, BtrieveError::VariableLength { .. }), "{err}");
    let mut file = open(dir, "WCCTEXT.VIR");
    assert_eq!(file.header.record_length, 22);
    let mut records = file.records();
    while let Some(record) = records.next_record() {
        assert_eq!(record.unwrap().data.len(), 2022);
    }
}

// The card expected 1528-byte room records; the file says otherwise.
#[test]
fn newmp001_is_an_empty_v6_file_with_1544_byte_records() {
    let dir = &stock();
    let file = open(dir, "NEWMP001.VIR");
    let h = &file.header;
    assert_eq!(h.version, Version::V6);
    assert_eq!(
        (h.page_length, h.record_length, h.physical_record_length),
        (4096, 1544, 1546)
    );
    assert_eq!(h.record_count, 0);
    match file.expect_record_len(1528).unwrap_err() {
        BtrieveError::RecordLengthMismatch {
            expected, found, ..
        } => assert_eq!((expected, found), (1528, 1544)),
        other => panic!("{other}"),
    }
    let key = file.keys[0].primary();
    assert_eq!(
        (file.keys.len(), key.data_type, key.offset, key.length),
        (1, T::UnsignedBinary, 0, 8)
    );
}

/// (key, segment, type, offset, length, attributes) of one segment.
type Segment = (u16, u16, T, u16, u16, u16);

/// The shape of every segment, in order.
fn shape(file: &BtrieveFile) -> Vec<Segment> {
    file.keys
        .iter()
        .flat_map(|k| &k.segments)
        .map(|s| {
            (
                s.number,
                s.segment,
                s.data_type,
                s.offset,
                s.length,
                s.attributes.0,
            )
        })
        .collect()
}

// The key definitions of the MajorMUD files, as the stock files hold them.
#[test]
fn majormud_key_definitions() {
    let dir = &stock();
    let cases: &[(&str, &[Segment])] = &[
        (
            "WCCUSERS.VIR",
            &[
                (0, 0, T::Zstring, 0, 30, 0x0100),
                (1, 0, T::Zstring, 30, 11, 0x0102),
                (2, 0, T::UnsignedBinary, 60, 4, 0x0147),
            ],
        ),
        ("WCCMP001.VIR", &[(0, 0, T::UnsignedBinary, 0, 8, 0x0104)]),
        (
            "WCCGANGS.VIR",
            &[
                (0, 0, T::Zstring, 0, 20, 0x0100),
                (1, 0, T::UnsignedBinary, 40, 4, 0x0147),
            ],
        ),
        (
            "WCCITOWN.VIR",
            &[
                (0, 0, T::AutoInc, 0, 4, 0x0100),
                (1, 0, T::UnsignedBinary, 4, 4, 0x0115),
                (1, 1, T::Zstring, 8, 1, 0x0101),
            ],
        ),
        (
            "WCCBANKS.VIR",
            &[
                (0, 0, T::Zstring, 0, 30, 0x0113),
                (0, 1, T::Integer, 30, 4, 0x0103),
            ],
        ),
        ("WCCTEXT.VIR", &[(0, 0, T::UnsignedBinary, 0, 18, 0x0104)]),
    ];
    for (name, expected) in cases {
        let file = open(dir, name);
        println!("{name}: {:?}", shape(&file));
        assert_eq!(shape(&file), *expected, "{name}");
    }
    // No stock file carries an ACS, so no Btrieve key is case-insensitive.
    for &(name, ..) in STOCK {
        let file = open(dir, name);
        assert!(file.acs.is_empty(), "{name} has an ACS: {:?}", file.acs);
    }
}

// The `updrec` record is 2027 bytes (the length the game opens the file with); the file's
// records are 2015.
#[test]
fn wccupdat_dat_enumerates_in_seqnum_order() {
    let mut file = open(dist(), "WCCUPDAT.DAT");
    assert_eq!(file.header.version, Version::V5);
    assert_eq!(file.header.record_length, 2015);
    file.expect_record_len(2015).unwrap();
    assert!(matches!(
        file.expect_record_len(2027),
        Err(BtrieveError::RecordLengthMismatch { found: 2015, .. })
    ));
    let count = file.header.record_count;
    let mut records = file.records();
    let mut seqnums = Vec::new();
    while let Some(record) = records.next_record() {
        let data = record.unwrap().data;
        seqnums.push(i32::from_le_bytes(data[..4].try_into().unwrap()));
    }
    println!(
        "WCCUPDAT.DAT: {} records, seqnum {}..{}",
        seqnums.len(),
        seqnums.first().unwrap(),
        seqnums.last().unwrap()
    );
    assert_eq!(seqnums.len() as u32, count);
    assert_eq!(count, 38_754);
    // Physical order is seqnum order: 1, 2, 3, ...
    assert!(seqnums.iter().zip(1..).all(|(&s, i)| s == i));
}

/// FCR record counts from T-006's MANIFEST.json, when it is present. The manifest's exact
/// shape is T-006's; this looks for an object naming the file with a numeric field whose
/// name contains "fcr".
fn manifest_fcr_count(manifest: &serde_json::Value, stem: &str) -> Option<u64> {
    use serde_json::Value;
    fn names(value: &Value, stem: &str) -> bool {
        value.as_object().is_some_and(|o| {
            o.values().any(|v| {
                v.as_str()
                    .is_some_and(|s| s.to_ascii_uppercase().contains(stem))
            })
        })
    }
    fn fcr(value: &Value) -> Option<u64> {
        value.as_object()?.iter().find_map(|(k, v)| {
            k.to_ascii_lowercase()
                .contains("fcr")
                .then(|| v.as_u64())
                .flatten()
        })
    }
    match manifest {
        Value::Object(map) => map
            .iter()
            .find_map(|(k, v)| {
                if k.to_ascii_uppercase().contains(stem) || names(v, stem) {
                    fcr(v)
                } else {
                    None
                }
            })
            .or_else(|| map.values().find_map(|v| manifest_fcr_count(v, stem))),
        Value::Array(items) => items.iter().find_map(|v| {
            if names(v, stem) {
                fcr(v)
            } else {
                manifest_fcr_count(v, stem)
            }
        }),
        _ => None,
    }
}

#[test]
fn counts_match_t006_manifest_when_present() {
    let dir = &stock();
    let path = std::env::var_os("MMUD_WORLD_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/world/MANIFEST.json")
        });
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: no T-006 manifest at {}", path.display());
        return;
    };
    let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
    for stem in ["WCCKNMSR", "WCCITEMS", "WCCMSG"] {
        let mut file = open(dir, &format!("{stem}.VIR"));
        let read = file.records().count_remaining().unwrap();
        let expected = manifest_fcr_count(&manifest, stem)
            .unwrap_or_else(|| panic!("no FCR count for {stem} in {}", path.display()));
        assert_eq!(u64::from(read), expected, "{stem}");
    }
}
