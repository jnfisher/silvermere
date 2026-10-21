//! The stock MajorMUD 1.11p distribution for tests (T-120). The tree holds no game data: every
//! test that reads the `.VIR` set, `WCCUPDAT.DAT` or the asset files gets them from here.
//!
//! [`zip`] is the MBBSEmu package, downloaded once into a cache shared by every checkout
//! (`SILVERMERE_STOCK_CACHE`, else `$XDG_CACHE_HOME/silvermere/stock`, else
//! `~/.cache/silvermere/stock`) and checked against the sha256 pinned in silvermere-import's
//! `stock` module, which this module includes rather than copies. [`dir`] is the package
//! unpacked once beside it: its files flat, as `silvermere import FILE.zip` unpacks them,
//! `assets/` holding what the import writes into `<data_dir>/assets/` (the long text too:
//! the three message files and the DLL's screens), and `vir/` holding the
//! sixteen `.VIR` files alone ([`vir_dir`]), from which an import applies no patches.
//!
//! With no cached copy and no network, a test fails here with a message saying what to fetch;
//! it never skips.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[path = "../../silvermere-import/src/dll_text.rs"]
#[allow(dead_code)]
mod dll_text;
#[path = "../../silvermere-import/src/stock.rs"]
#[allow(dead_code)]
mod fetch;
#[path = "../../silvermere-import/src/zip.rs"]
#[allow(dead_code)]
mod pkzip;

#[allow(unused_imports)] // each test that includes this file uses some
pub use fetch::{FILE_NAME, SHA256, SIZE, URL};

/// Overrides the cache directory.
pub const CACHE_ENV: &str = "SILVERMERE_STOCK_CACHE";

/// The extensions of the loose package files the import puts in the assets directory (the
/// members of the nested archives all go there).
const LOOSE_ASSETS: &[&str] = &["ANS", "NOT", "REL", "RLN"];

/// The message files the import copies as they are (the game's long text, T-138).
const TEXT_FILES: &[&str] = &["WCCMMUD.MSG", "WCCMMHLP.MSG", "WCCTEXT.MSG"];

/// The cache directory (see the module docs).
pub fn cache_dir() -> PathBuf {
    let set = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    set(CACHE_ENV)
        .or_else(|| set("XDG_CACHE_HOME").map(|d| d.join("silvermere/stock")))
        .or_else(|| set("HOME").map(|d| d.join(".cache/silvermere/stock")))
        .or_else(|| set("USERPROFILE").map(|d| d.join(".cache/silvermere/stock")))
        .unwrap_or_else(|| std::env::temp_dir().join("silvermere-stock"))
}

/// The package, checked (downloaded on first use).
pub fn zip() -> PathBuf {
    static ZIP: OnceLock<PathBuf> = OnceLock::new();
    ZIP.get_or_init(|| {
        let dir = cache_dir();
        match fetch::cached(&dir, false) {
            Ok((path, _)) => path,
            Err(e) => panic!(
                "the tests need the stock distribution and could not get it: {e}\n\
                 (cache directory {}; set {CACHE_ENV} to use another)",
                dir.display()
            ),
        }
    })
    .clone()
}

/// The package unpacked: `WCC*.VIR`, `WCCUPDAT.DAT` and the rest of its files, flat, and
/// `assets/`. Read only: copy what a test changes.
pub fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let zip = zip();
        let root = cache_dir();
        // the layout's version (4: the long text in assets/), then the package's: a change to what
        // is unpacked starts afresh
        let dir = root.join(format!("unpacked-4-{}", &SHA256[..12]));
        let done = dir.join(".complete");
        if done.exists() {
            return dir;
        }
        let lock_path = root.join(".lock");
        let lock = std::fs::File::create(&lock_path)
            .unwrap_or_else(|e| panic!("{}: {e}", lock_path.display()));
        lock.lock()
            .unwrap_or_else(|e| panic!("{}: {e}", lock_path.display()));
        if !done.exists() {
            // a tree left half done: its files may already be read-only
            let _ = set_readonly(&dir, false);
            let _ = std::fs::remove_dir_all(&dir);
            unpack(&zip, &dir).unwrap_or_else(|e| panic!("unpacking {}: {e}", zip.display()));
            // a test that writes into the tree fails instead of changing it for later runs
            set_readonly(&dir, true).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            std::fs::write(&done, b"").unwrap();
        }
        dir
    })
    .clone()
}

/// `dir()/assets`: the files the import writes into `<data_dir>/assets/`.
pub fn assets_dir() -> PathBuf {
    dir().join("assets")
}

/// The sixteen `.VIR` files alone: an import from here finds no `WCCUPDAT.DAT`, so it
/// builds the virgin world without the patches.
pub fn vir_dir() -> PathBuf {
    dir().join("vir")
}

/// One file of the unpacked package, by name (`WCCMP001.VIR`).
pub fn file(name: &str) -> PathBuf {
    let path = dir().join(name);
    assert!(path.is_file(), "the stock distribution has no {name}");
    path
}

/// Sets or clears the read-only bit of every file under `dir` (the directories stay
/// writable, so the tree can still be removed).
fn set_readonly(dir: &Path, readonly: bool) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            set_readonly(&entry.path(), readonly)?;
        } else {
            let mut perms = entry.metadata()?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(readonly);
            std::fs::set_permissions(entry.path(), perms)?;
        }
    }
    Ok(())
}

fn unpack(zip: &Path, dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(zip)?;
    let archive = pkzip::Archive::new(FILE_NAME, &bytes)?;
    archive.unpack_to(dir)?;
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets)?;
    let vir = dir.join("vir");
    std::fs::create_dir_all(&vir)?;
    for e in archive.entries().iter().filter(|e| !e.is_dir()) {
        let name = e.file_name().to_ascii_uppercase();
        let ext = name.rsplit_once('.').map_or("", |(_, x)| x);
        if ext == "ZIP" {
            let inner = archive.read(e)?;
            pkzip::Archive::new(name.as_str(), &inner)?.unpack_to(&assets)?;
        } else if ext == "VIR" {
            std::fs::hard_link(dir.join(e.file_name()), vir.join(&name))
                .or_else(|_| std::fs::copy(dir.join(e.file_name()), vir.join(&name)).map(drop))?;
        } else if LOOSE_ASSETS.contains(&ext) && !name.starts_with("WCCMMPLS.")
            || TEXT_FILES.contains(&name.as_str())
        {
            std::fs::write(assets.join(&name), archive.read(e)?)?;
        } else if name == dll_text::DLL_NAME {
            let text = dll_text::extract(&archive.read(e)?).map_err(|e| e.to_string())?;
            std::fs::write(assets.join(dll_text::OUTPUT_NAME), text)?;
        }
    }
    Ok(())
}
