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

/// T-138: the game's long text (help, the creation forms, the long screens) is read from
/// `<data_dir>/assets/`. An import writes it; a data directory imported by 0.1.3 or earlier
/// has none of it, so `serve` and `setup` call this: [`complete_text_assets`], downloading the
/// package first if it is not cached.
pub fn ensure_text_assets(data_dir: &Path) {
    complete_text_assets(data_dir, true);
}

/// Makes the text files of `<data_dir>/assets/` whole from the cached package (downloaded
/// first only with `download`): copies the missing ones, then replaces any that does not
/// parse (a realm's own edited or differently formatted file) with the package's, keeping
/// the realm's copy beside it as `NAME.REALM`. Each step logs one line; without a package it
/// warns once, and the game runs with HELP and the long screens saying the text is not
/// installed. After an import this runs without downloading.
pub fn complete_text_assets(data_dir: &Path, download: bool) {
    let assets = data_dir.join("assets");
    let say = |line: String, warn: bool| {
        if warn {
            tracing::warn!("{line}");
        } else {
            tracing::info!("{line}");
        }
        eprintln!("silvermere: {line}");
    };
    let missing = match silvermere_import::assets::missing_text_assets(&assets) {
        Ok(m) => m,
        Err(e) => return say(format!("the game's text is not installed: {e}"), true),
    };
    let unparsed = if missing.is_empty() {
        unparsed_text_file(&assets)
    } else {
        None
    };
    if missing.is_empty() && unparsed.is_none() {
        return;
    }
    let cached = cache_dir(data_dir).join(stock::FILE_NAME);
    let zip = if download || cached.exists() {
        stock::cached(&cache_dir(data_dir), false)
            .map(|(zip, _)| zip)
            .map_err(anyhow::Error::from)
    } else {
        Err(anyhow::anyhow!(
            "the stock package is not in {}",
            cache_dir(data_dir).display()
        ))
    };
    let zip = match zip {
        Ok(zip) => zip,
        Err(e) => {
            let what = match &unparsed {
                Some((file, problem)) => format!("{file} does not parse ({problem})"),
                None => format!("{} missing", missing.join(", ")),
            };
            return say(
                format!(
                    "the game's text is not complete in {} ({what}) and the stock package is not to hand ({e:#}): HELP and the long screens say the text is not installed until `silvermere import` or a start with the package cached",
                    assets.display()
                ),
                true,
            );
        }
    };
    if !missing.is_empty() {
        match silvermere_import::assets::text_assets_from_package(&zip, &assets) {
            Ok(names) => say(
                format!(
                    "the game's text was not in {}: copied {} from the stock {NAME}",
                    assets.display(),
                    names.join(", ")
                ),
                false,
            ),
            Err(e) => {
                return say(format!("the game's text is not installed ({e})"), true);
            }
        }
    }
    // each file that does not parse, one at a time (parsing stops at the first)
    let mut replaced = Vec::new();
    while let Some((file, problem)) = unparsed_text_file(&assets) {
        if replaced.contains(&file) {
            return say(
                format!("{file} from the stock {NAME} does not parse either ({problem})"),
                true,
            );
        }
        let stock_bytes = match silvermere_import::assets::stock_text_asset(&zip, &file) {
            Ok(b) => b,
            Err(e) => return say(format!("{file} does not parse ({problem}), and {e}"), true),
        };
        let path = existing_path(&assets, &file).unwrap_or_else(|| assets.join(&file));
        let kept = assets.join(format!("{file}.REALM"));
        let moved = std::fs::rename(&path, &kept)
            .and_then(|()| std::fs::write(assets.join(&file), &stock_bytes));
        match moved {
            Ok(()) => say(
                format!(
                    "{} does not parse ({problem}): using the stock {NAME}'s, the realm's kept as {}",
                    path.display(),
                    kept.display()
                ),
                true,
            ),
            Err(e) => return say(format!("{}: {e}", path.display()), true),
        }
        replaced.push(file);
    }
}

/// The text file of `assets` (as named in `TEXT_ASSETS`) that stops the text loading, and why.
fn unparsed_text_file(assets: &Path) -> Option<(String, String)> {
    use silvermere_text::package as p;
    let read = |n: &str| existing_path(assets, n).and_then(|path| std::fs::read(path).ok());
    let (Some(h), Some(m), Some(t), Some(d)) = (
        read(p::HELP_FILE),
        read(p::WCCMMUD_FILE),
        read(p::WCCTEXT_FILE),
        read(p::DLL_TEXT_FILE),
    ) else {
        return None;
    };
    match p::PackageText::from_files(&h, &m, &t, &d) {
        Ok(_) => None,
        Err(e) => Some((e.file, e.problem)),
    }
}

/// The file of `dir` named `name` in any case.
fn existing_path(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

/// Installs the game's long text from `<data_dir>/assets/` for this process (the first
/// install wins); logs a warning when it cannot be read.
pub fn install_text(data_dir: &Path) {
    let assets = data_dir.join("assets");
    if let Err(e) = silvermere_text::package::load_and_install(&assets) {
        tracing::warn!("the game's text is not installed: {e}");
    }
}
