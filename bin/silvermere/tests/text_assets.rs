//! T-138: the game's long text at start-up. A data directory without the text files (an
//! install of 0.1.3 or earlier) gets them from the cached package; without a package the
//! start goes on, and the game says the text is not installed.

use std::path::PathBuf;

use silvermere_import::assets::{TEXT_ASSETS, missing_text_assets};
use silvermere_text::package::PackageText;

fn tmp(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn missing_text_comes_from_the_cached_package() {
    let data = tmp("text-assets-upgrade");
    let cache = silvermere::stock::cache_dir(&data);
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::copy(
        silvermere_testkit::stock::zip(),
        cache.join(silvermere_testkit::stock::FILE_NAME),
    )
    .unwrap();
    // the assets an older import wrote, and a sysop's own help file, which is kept
    let assets = data.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("WCCMMUD.NOT"), b"notes").unwrap();
    // the sysop's edit: the stock help with a note of their own after it (it still parses)
    let mut own =
        std::fs::read(silvermere_testkit::stock::assets_dir().join("WCCMMHLP.MSG")).unwrap();
    own.extend_from_slice(b"\r\nEdited by the sysop.\r\n");
    std::fs::write(assets.join("wccmmhlp.msg"), &own).unwrap();
    assert_eq!(
        missing_text_assets(&assets).unwrap().len(),
        TEXT_ASSETS.len() - 1
    );

    silvermere::stock::ensure_text_assets(&data);
    assert!(missing_text_assets(&assets).unwrap().is_empty());
    assert_eq!(std::fs::read(assets.join("wccmmhlp.msg")).unwrap(), own);
    assert_eq!(std::fs::read(assets.join("WCCMMUD.NOT")).unwrap(), b"notes");
    for name in ["WCCMMUD.MSG", "WCCTEXT.MSG", "DLLTEXT.TXT"] {
        assert_eq!(
            std::fs::read(assets.join(name)).unwrap(),
            std::fs::read(silvermere_testkit::stock::assets_dir().join(name)).unwrap(),
            "{name} is what the import writes"
        );
    }
    PackageText::load(&assets).expect("the text loads, the sysop's help with it");
}

#[test]
fn without_a_package_the_start_goes_on() {
    let data = tmp("text-assets-none");
    // a cache that cannot be made: no download, no package
    std::fs::write(silvermere::stock::cache_dir(&data), b"not a directory").unwrap();
    silvermere::stock::ensure_text_assets(&data);
    let assets = data.join("assets");
    assert_eq!(missing_text_assets(&assets).unwrap(), TEXT_ASSETS);
    let err = PackageText::load(&assets).unwrap_err();
    assert_eq!(err.file, "WCCMMHLP.MSG");
}

/// A data directory whose package is cached, its assets the stock text files.
fn with_stock_text(name: &str) -> (PathBuf, PathBuf) {
    let data = tmp(name);
    let cache = silvermere::stock::cache_dir(&data);
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::copy(
        silvermere_testkit::stock::zip(),
        cache.join(silvermere_testkit::stock::FILE_NAME),
    )
    .unwrap();
    let assets = data.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    for name in TEXT_ASSETS {
        let bytes = std::fs::read(silvermere_testkit::stock::assets_dir().join(name)).unwrap();
        std::fs::write(assets.join(name), bytes).unwrap();
    }
    (data, assets)
}

// A realm's own message file that does not parse (another format, other placeholders) is
// replaced by the stock package's, the realm's kept beside it; one file never blanks the rest.
#[test]
fn a_realm_message_file_that_does_not_parse_falls_back_to_the_stock_one() {
    let (data, assets) = with_stock_text("text-assets-unparsed");
    // a help file with an unterminated value, under a lower-case name, and a message file
    // without the text blocks
    std::fs::remove_file(assets.join("WCCMMHLP.MSG")).unwrap();
    std::fs::write(assets.join("wccmmhlp.msg"), b"HL1B0 {never closed\r\n").unwrap();
    std::fs::write(assets.join("WCCMMUD.MSG"), b"OTHER {x} T\r\n").unwrap();
    assert!(PackageText::load(&assets).is_err());

    silvermere::stock::complete_text_assets(&data, false);
    PackageText::load(&assets).expect("the text loads after the fallback");
    assert_eq!(
        std::fs::read(assets.join("WCCMMHLP.MSG.REALM")).unwrap(),
        b"HL1B0 {never closed\r\n"
    );
    assert_eq!(
        std::fs::read(assets.join("WCCMMUD.MSG.REALM")).unwrap(),
        b"OTHER {x} T\r\n"
    );
    for name in ["WCCMMHLP.MSG", "WCCMMUD.MSG"] {
        assert_eq!(
            std::fs::read(assets.join(name)).unwrap(),
            std::fs::read(silvermere_testkit::stock::assets_dir().join(name)).unwrap(),
            "{name} is the stock file"
        );
    }
    // a whole set is left alone
    silvermere::stock::complete_text_assets(&data, false);
    assert!(!assets.join("WCCTEXT.MSG.REALM").exists());
}

// After an import that skipped a realm's DLL, the screens come from the cached package
// without a download; with no package cached they wait for the start-up.
#[test]
fn a_skipped_dll_text_comes_from_the_cached_package() {
    let (data, assets) = with_stock_text("text-assets-dll");
    std::fs::remove_file(assets.join("DLLTEXT.TXT")).unwrap();
    silvermere::stock::complete_text_assets(&data, false);
    PackageText::load(&assets).expect("the text loads");

    std::fs::remove_file(assets.join("DLLTEXT.TXT")).unwrap();
    std::fs::remove_dir_all(silvermere::stock::cache_dir(&data)).unwrap();
    silvermere::stock::complete_text_assets(&data, false);
    assert_eq!(missing_text_assets(&assets).unwrap(), ["DLLTEXT.TXT"]);
}
