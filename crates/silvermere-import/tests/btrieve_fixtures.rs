//! The Btrieve reader against wbtrv32's synthetic test files (`assets/`, copied into
//! `tests/fixtures/btrieve/`; they hold test records, no game data). Expected numbers come
//! from wbtrv32's `btrieve/BtrieveDatabase_test.cc`. The real files, v5 and v6, are the stock
//! distribution's (`tests/stock_vir.rs`); a stock one is fuzzed here too.

use std::path::PathBuf;

use proptest::prelude::*;
use silvermere_import::btrieve::{
    BtrieveError, BtrieveFile, KeyAttributes as A, KeyDataType, OpenOptions, RecordType, Version,
};

use crate::stock as stock_data;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/btrieve")
        .join(name)
}

fn open(name: &str) -> BtrieveFile {
    BtrieveFile::open(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn open_variable(name: &str) -> BtrieveFile {
    OpenOptions::new()
        .variable_length(true)
        .open(&fixture(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A fixture's bytes; a `.VIR` name is the stock distribution's file.
fn bytes(name: &str) -> Vec<u8> {
    if name.ends_with(".VIR") {
        std::fs::read(stock_data::file(name)).unwrap()
    } else {
        std::fs::read(fixture(name)).unwrap()
    }
}

fn open_bytes(name: &str, data: Vec<u8>) -> Result<BtrieveFile, BtrieveError> {
    OpenOptions::new()
        .variable_length(true)
        .open_bytes(name, data)
}

/// Every record's payload, checking ordinals run 1, 2, 3, ...
fn all_records(file: &mut BtrieveFile) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut records = file.records();
    while let Some(record) = records.next_record() {
        let record = record.unwrap();
        assert_eq!(record.ordinal as usize, out.len() + 1);
        out.push(record.data.to_vec());
    }
    out
}

/// (number, length, offset, type, attributes) of every segment, in order.
fn segments(file: &BtrieveFile) -> Vec<(u16, u16, u16, KeyDataType, u16)> {
    file.keys
        .iter()
        .flat_map(|k| &k.segments)
        .map(|s| (s.number, s.length, s.offset, s.data_type, s.attributes.0))
        .collect()
}

// BtrieveDatabase_test.cc:7 LoadsMBBSEmuDat
#[test]
fn mbbsemu_dat_v5_fixed() {
    let mut file = open("MBBSEMU.DAT");
    let h = &file.header;
    assert_eq!(h.version, Version::V5);
    assert_eq!(h.page_length, 512);
    assert_eq!(h.page_count, 6);
    assert_eq!(h.record_length, 74);
    assert_eq!(h.physical_record_length, 90);
    assert_eq!(h.record_count, 4);
    assert_eq!(h.record_type, RecordType::Fixed);
    assert!(!h.is_variable_length());
    assert_eq!(file.keys.len(), 4);
    assert!(file.keys.iter().all(|k| !k.is_composite()));
    let ext = A::EXTENDED_TYPE;
    assert_eq!(
        segments(&file),
        vec![
            (0, 32, 2, KeyDataType::Zstring, A::DUPLICATES | ext),
            (1, 4, 34, KeyDataType::Integer, A::MODIFIABLE | ext),
            (
                2,
                32,
                38,
                KeyDataType::Zstring,
                A::DUPLICATES | A::MODIFIABLE | ext
            ),
            (3, 4, 70, KeyDataType::AutoInc, ext),
        ]
    );
    // No ACS in the file: no key carries one.
    assert!(file.acs.is_empty());
    assert!(file.keys.iter().all(|k| k.primary().acs.is_none()));
    assert!(file.keys.iter().all(|k| k.primary().null_value == 0));

    let records = all_records(&mut file);
    assert_eq!(records.len(), 4);
    assert!(records.iter().all(|r| r.len() == 74));
}

// BtrieveDatabase_test.cc:62 LoadsVariableDat
#[test]
fn variable_dat_is_refused_by_default() {
    let err = BtrieveFile::open(&fixture("VARIABLE.DAT")).unwrap_err();
    assert!(matches!(err, BtrieveError::VariableLength { .. }), "{err}");
    assert!(err.to_string().contains("VARIABLE.DAT"));
}

// BtrieveDatabase_test.cc:62 LoadsVariableDat
#[test]
fn variable_dat_v5_fragments_when_opted_in() {
    let mut file = open_variable("VARIABLE.DAT");
    let h = &file.header;
    assert_eq!(h.version, Version::V5);
    assert_eq!(h.record_length, 8);
    assert_eq!(h.physical_record_length, 20);
    assert_eq!(h.record_count, 1024);
    assert_eq!(h.page_length, 512);
    assert_eq!(h.page_count, 1156);
    assert!(h.is_variable_length());
    let ext = A::EXTENDED_TYPE;
    assert_eq!(
        segments(&file),
        vec![
            (0, 2, 4, KeyDataType::Integer, A::DUPLICATES | ext),
            (1, 2, 6, KeyDataType::Integer, ext),
        ]
    );

    let records = all_records(&mut file);
    assert_eq!(records.len(), 1024);
    for (i, record) in records.iter().enumerate() {
        // Each record: u32 0xDEADBEEF, u16 i % 64, u16 i, then i bytes counting up from 0.
        assert_eq!(record.len(), 8 + i);
        assert_eq!(
            u32::from_le_bytes(record[0..4].try_into().unwrap()),
            0xDEAD_BEEF
        );
        assert_eq!(u16::from_le_bytes([record[4], record[5]]) as usize, i % 64);
        assert_eq!(u16::from_le_bytes([record[6], record[7]]) as usize, i);
        assert!(record[8..].iter().enumerate().all(|(j, &b)| b == j as u8));
    }
}

// BtrieveDatabase_test.cc:209 LoadsMultiAcsDatV6
#[test]
fn multiacs_dat_v6_per_key_acs() {
    // MULTIACS.DAT is variable-length, so it needs the opt-in to be read at all.
    let err = BtrieveFile::open(&fixture("MULTIACS.DAT")).unwrap_err();
    assert!(matches!(err, BtrieveError::VariableLength { .. }), "{err}");

    let mut file = open_variable("MULTIACS.DAT");
    assert_eq!(file.header.version, Version::V6);
    assert_eq!(file.header.record_length, 128);
    assert_eq!(file.header.record_count, 0);
    assert_eq!(file.keys.len(), 3);
    assert_eq!(file.keys[0].primary().acs_name.as_deref(), Some("ALLCAPS"));
    assert_eq!(file.keys[2].primary().acs_name.as_deref(), Some("LOWER"));
    // Found: key 1 is a two-segment numeric key without an ACS; the file default is LOWER.
    assert_eq!(file.keys[1].segments.len(), 2);
    assert!(file.keys[1].segments.iter().all(|s| s.acs.is_none()));
    assert_eq!(file.acs[0].name, "LOWER");
    assert!(file.keys[0].primary().attributes.has(A::MULTIPLE_ACS));

    // A zero-record file enumerates nothing, without error.
    assert!(all_records(&mut file).is_empty());
}

#[test]
fn record_length_is_checked() {
    let file = open("MBBSEMU.DAT");
    file.expect_record_len(74).unwrap();
    match file.expect_record_len(75).unwrap_err() {
        BtrieveError::RecordLengthMismatch {
            expected, found, ..
        } => assert_eq!((expected, found), (75, 74)),
        other => panic!("{other}"),
    }
    // Variable-length: the argument is the largest record, the fixed part must fit.
    let variable = open_variable("VARIABLE.DAT");
    variable.expect_record_len(1031).unwrap();
    assert!(variable.expect_record_len(7).is_err());
}

fn patched(name: &str, at: usize, with: &[u8]) -> Result<BtrieveFile, BtrieveError> {
    let mut data = bytes(name);
    data[at..at + with.len()].copy_from_slice(with);
    open_bytes(name, data)
}

// BtrieveDatabase.cc:86 validateDatabase: `FC\0\0` is v6.
#[test]
fn v6_magic_branch() {
    let file = open_bytes("NEWMP001.VIR", bytes("NEWMP001.VIR")).unwrap();
    assert_eq!(file.header.version, Version::V6);
    // Without the magic, a v6 FCR has no v5 version code at +7.
    let err = patched("NEWMP001.VIR", 0, b"XC").unwrap_err();
    assert!(matches!(err, BtrieveError::NotBtrieve { .. }), "{err}");
}

// BtrieveDatabase.cc:124 validateDatabase: v5 version code 3, 4 or 5.
#[test]
fn v5_version_code_branch() {
    for code in [3u8, 4, 5] {
        assert_eq!(
            patched("MBBSEMU.DAT", 7, &[code]).unwrap().header.version,
            Version::V5
        );
    }
    for code in [0u8, 2, 6, 0xFF] {
        let err = patched("MBBSEMU.DAT", 7, &[code]).unwrap_err();
        assert!(
            matches!(err, BtrieveError::NotBtrieve { .. }),
            "{code}: {err}"
        );
    }
}

// BtrieveDatabase.cc:136 validateDatabase: 0xFFFF at 0x22 needs recovery.
#[test]
fn recovery_flag_is_refused() {
    let err = patched("MBBSEMU.DAT", 0x22, &[0xFF, 0xFF]).unwrap_err();
    assert!(matches!(err, BtrieveError::NeedsRecovery { .. }), "{err}");
}

// BtrieveDatabase.cc:155 validateDatabase: usrflgs bit 3 is compression.
#[test]
fn compression_is_refused() {
    let err = patched("MBBSEMU.DAT", 0x106, &[0x08, 0x00]).unwrap_err();
    assert!(matches!(err, BtrieveError::Compressed { .. }), "{err}");
    // A v6 file's active FCR is whichever copy has the higher usage count; patch both.
    let mut data = bytes("NEWMP001.VIR");
    data[0x106] = 0x08;
    data[4096 + 0x106] = 0x08;
    let both = open_bytes("NEWMP001.VIR", data).unwrap_err();
    assert!(matches!(both, BtrieveError::Compressed { .. }), "{both}");
}

// BtrieveDatabase.cc:95 and :147 validateDatabase: page length and accel flags.
#[test]
fn bad_page_length_and_accel_flags_are_refused() {
    for page_length in [0u16, 256, 600, 1000] {
        let err = patched("MBBSEMU.DAT", 8, &page_length.to_le_bytes()).unwrap_err();
        assert!(
            matches!(err, BtrieveError::NotBtrieve { .. }),
            "{page_length}: {err}"
        );
    }
    let err = patched("MBBSEMU.DAT", 0x0A, &[1, 0]).unwrap_err();
    assert!(matches!(err, BtrieveError::NotBtrieve { .. }), "{err}");
}

#[test]
fn short_and_foreign_files_are_not_btrieve() {
    for data in [vec![], vec![0u8; 100], b"PK\x03\x04".repeat(200)] {
        let err = open_bytes("x", data).unwrap_err();
        assert!(matches!(err, BtrieveError::NotBtrieve { .. }), "{err}");
    }
}

fn enumerate(data: Vec<u8>) {
    if let Ok(mut file) = open_bytes("fuzz", data) {
        let mut records = file.records();
        while let Some(record) = records.next_record() {
            if record.is_err() {
                break;
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_buffers_never_panic(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        enumerate(data);
    }

    #[test]
    fn mutated_fixtures_never_panic(
        which in 0usize..4,
        edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..12),
        header_only in any::<bool>(),
    ) {
        let mut data = match which {
            0 => bytes("MBBSEMU.DAT"),
            1 => bytes("NEWMP001.VIR"),
            2 => bytes("VARIABLE.DAT"),
            _ => bytes("MULTIACS.DAT"),
        };
        // Half the cases hit the first four pages, where the FCR, ACS and PAT live.
        let span = if header_only { data.len().min(4 * 4096) } else { data.len() };
        for (index, byte) in edits {
            data[index.index(span)] = byte;
        }
        enumerate(data);
    }
}
