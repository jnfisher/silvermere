//! The runtime text files unpacked into the assets directory (T-107). The inputs are the
//! stock distribution's own files (the testkit's `stock` helper: downloaded once, checked and
//! unpacked), laid out as the distribution ships them: the menu and map texts loose, the
//! description files in the five archives `WCCMISC.BAT` unpacks.

use std::path::{Path, PathBuf};

use silvermere_import::assets::{ASSET_ARCHIVES, AssetCount, LOOSE, unpack_assets};
use silvermere_import::stock::sha256_hex;

use crate::stock as stock_data;

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("assets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A fresh directory holding the distribution's text files: the menu and map texts loose, the
/// description files in their five archives (the Btrieve files are not needed here).
fn dist(name: &str) -> PathBuf {
    let d = dir(name);
    let loose = [
        "WCCMAP01.ANS",
        "WCCMMUD.NOT",
        "WCCMMUD.RLN",
        "WCCMMSYS.NOT",
        "WCCMMSYS.REL",
    ];
    for f in loose.iter().chain(ASSET_ARCHIVES) {
        std::fs::copy(stock_data::file(f), d.join(f)).unwrap();
    }
    d
}

fn count<'a>(counts: &'a [AssetCount], source: &str) -> &'a AssetCount {
    counts.iter().find(|c| c.source == source).unwrap()
}

#[test]
fn stock_assets_unpacked() {
    let from = dist("stock");
    let dest = dir("stock-out");
    let counts = unpack_assets(&from, &dest, false, false).unwrap();
    for c in &counts {
        println!("{}", c.line(false));
    }
    let hse = count(&counts, "WCCHSE.ZIP");
    assert_eq!((hse.files, hse.written, hse.kept), (47, 47, 0));
    for (zip, n) in [
        ("WCCEMB.ZIP", 10),
        ("WCCBAN.ZIP", 10),
        ("WCCSTA.ZIP", 14),
        ("WCCSTO.ZIP", 7),
    ] {
        assert_eq!(count(&counts, zip).written, n, "{zip}");
    }
    // WCCMAP01.ANS, WCCMMUD.NOT, WCCMMUD.RLN, WCCMMSYS.NOT, WCCMMSYS.REL
    assert_eq!(count(&counts, LOOSE).written, 5);

    // the bytes as shipped
    for (name, sha) in [
        (
            "WCC85015.HSE",
            "3d35d98be298f9d2395f38a6f778c6d617ac3fb0cbab244b66d99b88ef17e8c8",
        ),
        (
            "WCC90415.HSE",
            "ffada301366b08d12f4092557125c5180e994e6b893eeee5651ba418b9616002",
        ),
        (
            "WCCMAP01.ANS",
            "b17c6be41be4940d163338111fd720a198f04041acec5d0fd0ab21359182b455",
        ),
    ] {
        assert_eq!(
            sha256_hex(&std::fs::read(dest.join(name)).unwrap()),
            sha,
            "{name}"
        );
    }
    for name in [
        "WCCMMUD.RLN",
        "WCCMMSYS.REL",
        "WCCMMSYS.NOT",
        "WCCMMUD.NOT",
        "WCCYEL.BAN",
    ] {
        assert!(dest.join(name).is_file(), "{name}");
    }

    // a second run keeps every file; a dry run writes nothing
    let again = unpack_assets(&from, &dest, false, false).unwrap();
    assert!(again.iter().all(|c| c.written == 0 && c.kept == c.files));
    assert!(
        again[0]
            .line(false)
            .ends_with("(already there; --overwrite-assets replaces them)")
    );
    let empty = dir("dry");
    let dry = unpack_assets(&from, &empty, false, true).unwrap();
    assert_eq!(count(&dry, "WCCHSE.ZIP").written, 47);
    assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);
}

/// Names match in any case; an existing file is kept, or replaced under its own name.
#[test]
fn existing_files_kept_or_overwritten() {
    let from = dist("case");
    let dest = dir("case-out");
    std::fs::write(dest.join("wccmap01.ans"), b"mine").unwrap();
    let counts = unpack_assets(&from, &dest, false, false).unwrap();
    assert_eq!(count(&counts, LOOSE).kept, 1);
    assert_eq!(std::fs::read(dest.join("wccmap01.ans")).unwrap(), b"mine");
    assert!(!dest.join("WCCMAP01.ANS").exists());

    unpack_assets(&from, &dest, true, false).unwrap();
    assert_ne!(std::fs::read(dest.join("wccmap01.ans")).unwrap(), b"mine");
    assert!(!dest.join("WCCMAP01.ANS").exists());
}

/// A running BBS's own (possibly edited) description file wins over the archive's copy.
#[test]
fn loose_file_wins_over_archive() {
    let from = dist("loose");
    std::fs::write(from.join("wcc85015.hse"), b"Custom House\r\n").unwrap();
    let dest = dir("loose-out");
    let counts = unpack_assets(&from, &dest, false, false).unwrap();
    assert_eq!(count(&counts, "WCCHSE.ZIP").written, 46);
    assert_eq!(count(&counts, LOOSE).written, 6);
    assert_eq!(
        std::fs::read(dest.join("WCC85015.HSE")).unwrap(),
        b"Custom House\r\n"
    );
}

/// A broken archive is an error naming it.
#[test]
fn broken_archive_refused() {
    let from = dir("broken");
    std::fs::write(from.join("WCCHSE.ZIP"), b"not a zip").unwrap();
    let err = unpack_assets(&from, &dir("broken-out"), false, false).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("WCCHSE.ZIP: not a readable zip archive"),
        "{err}"
    );
}

/// The cached package is the pinned file, and the testkit's unpacked copy of the assets is
/// what the import writes.
#[test]
fn stock_package_is_the_pinned_file() {
    let bytes = std::fs::read(stock_data::zip()).unwrap();
    assert!(silvermere_import::stock::verify(&bytes));
    assert_eq!(stock_data::SHA256, silvermere_import::stock::SHA256);

    let dest = dir("pinned-out");
    unpack_assets(&stock_data::dir(), &dest, false, false).unwrap();
    let mut written: Vec<_> = std::fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    written.sort();
    let mut helper: Vec<_> = std::fs::read_dir(stock_data::assets_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    helper.sort();
    assert_eq!(written.len(), 93);
    assert_eq!(written, helper);
    for n in &written {
        assert_eq!(
            std::fs::read(dest.join(n)).unwrap(),
            std::fs::read(stock_data::assets_dir().join(n)).unwrap(),
            "{n:?}"
        );
    }
}
