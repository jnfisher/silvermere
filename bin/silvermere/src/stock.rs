//! The stock MajorMUD 1.11p distribution (T-120): the MBBSEmu package, which silvermere does
//! not ship. `silvermere import` with no source, and the first start, download it once into
//! `<data_dir>/cache/`, check its sha256 and import from it; a later run uses the cached copy
//! (checked again) and replaces it if it no longer matches. The URL, size and sha256 are
//! pinned in `silvermere_import::stock`.

use std::path::{Path, PathBuf};

use silvermere_import::stock::{self, Origin};

/// The distribution's name, for messages.
pub const NAME: &str = "MajorMUD 1.11p distribution (the MBBSEmu package)";

/// The cache directory under `data_dir`.
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cache")
}

/// The package in `<data_dir>/cache/`, checked; downloaded when missing, when the cached copy
/// does not match, or always with `refresh` (`import --download`).
pub fn fetch(data_dir: &Path, refresh: bool) -> anyhow::Result<PathBuf> {
    let dir = cache_dir(data_dir);
    let cached = dir.join(stock::FILE_NAME);
    if refresh || !cached.exists() {
        eprintln!(
            "downloading the stock {NAME} from {} into {}",
            stock::url(),
            dir.display()
        );
    }
    let (path, origin) = stock::cached(&dir, refresh)?;
    match origin {
        Origin::Cache => eprintln!("using the stock {NAME} in {}", path.display()),
        Origin::Downloaded => eprintln!("downloaded and checked (sha256 {})", stock::SHA256),
        Origin::Replaced => eprintln!(
            "the copy in {} did not match its sha256: downloaded it again and checked it",
            path.display()
        ),
    }
    Ok(path)
}
