//! The runtime text files: `--from` into `<data_dir>/assets/` (T-107).
//!
//! The engine reads a few text files by name from the assets directory
//! (`display::Assets::load_dir`, silvermere-engine): the `FILE DESCRIPTION` room files (the
//! gang houses' `WCCnnnnn.HSE`, and the emblem, banner, statue and stone files the same
//! rooms can name), the room map files (`WCCMAP01.ANS`, the MAP command), the release notes
//! `WCCMMUD.NOT` (main menu R) and the sysop menu's `WCCMMUD.RLN`, `WCCMMSYS.REL` and
//! `WCCMMSYS.NOT` (docs/sysop.md). The distribution ships the description files packed:
//! its install step `WCCMISC.BAT` runs `pkunzip -o` on `WCCHSE.ZIP`, `WCCEMB.ZIP`,
//! `WCCBAN.ZIP`, `WCCSTA.ZIP` and `WCCSTO.ZIP` into the BBS directory, where the DLL opens
//! them by name.
//!
//! [`unpack_assets`] takes the members of those archives, then every loose file of `--from`
//! with one of [`ASSET_EXTENSIONS`] (a running BBS's own copy, possibly edited by the sysop,
//! wins over the archive's), and writes each into the assets directory. Names match in any
//! case, as on DOS; a file already there is kept unless `overwrite` is set.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::zip::{Archive, ZipError};

/// The extensions of the text files the engine reads from the assets directory.
pub const ASSET_EXTENSIONS: &[&str] = &[
    "ANS", "HSE", "BAN", "EMB", "STA", "STO", "NOT", "REL", "RLN",
];

/// The archives `WCCMISC.BAT` unpacks into the BBS directory, in its order.
pub const ASSET_ARCHIVES: &[&str] = &[
    "WCCHSE.ZIP",
    "WCCEMB.ZIP",
    "WCCBAN.ZIP",
    "WCCSTA.ZIP",
    "WCCSTO.ZIP",
];

/// What unpacking the assets can fail on.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// An archive could not be read.
    #[error(transparent)]
    Zip(#[from] ZipError),
    /// Reading `--from` or writing the assets directory failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> AssetError + '_ {
    move |source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// One count line: `assets [SOURCE]: N files, N written, N kept (...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetCount {
    /// An archive's name, or `text files` for the loose files of `--from`.
    pub source: String,
    /// Files taken from this source.
    pub files: u32,
    /// Files written (in a dry run, to be written).
    pub written: u32,
    /// Files already in the assets directory and kept.
    pub kept: u32,
}

/// The `source` of the loose files' count line.
pub const LOOSE: &str = "text files";

impl AssetCount {
    /// The count line; a dry run says what would be written.
    pub fn line(&self, dry_run: bool) -> String {
        let verb = if dry_run { "to write" } else { "written" };
        let mut line = format!(
            "assets [{}]: {} files, {} {verb}, {} kept",
            self.source, self.files, self.written, self.kept
        );
        if self.kept > 0 {
            line.push_str(" (already there; --overwrite-assets replaces them)");
        }
        line
    }
}

impl fmt::Display for AssetCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line(false))
    }
}

fn has_asset_extension(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ASSET_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// The regular files of `dir` by upper-case name (the first of names equal in any case).
fn files_by_name(dir: &Path) -> Result<HashMap<String, PathBuf>, AssetError> {
    let mut out = HashMap::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(io(dir)(e)),
    };
    let mut entries: Vec<_> = rd.collect::<Result<_, _>>().map_err(io(dir))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.file_type().map_err(io(&e.path()))?.is_file() {
            let name = e.file_name().to_string_lossy().to_ascii_uppercase();
            out.entry(name).or_insert_with(|| e.path());
        }
    }
    Ok(out)
}

/// The files of MajorMUD Plus (`WCCMMPLS.*`), a separate add-on the MBBSEmu package carries
/// beside the game: the engine reads none of them, so they are not assets.
fn is_plus_file(upper_name: &str) -> bool {
    upper_name.starts_with("WCCMMPLS.")
}

/// Unpacks the assets of `from` into `dest` (created when anything is written); with
/// `dry_run` only counts. One [`AssetCount`] per archive found, then one for the loose
/// files.
pub fn unpack_assets(
    from: &Path,
    dest: &Path,
    overwrite: bool,
    dry_run: bool,
) -> Result<Vec<AssetCount>, AssetError> {
    let sources = files_by_name(from)?;
    // upper-case name -> (count slot, bytes); a later source replaces an earlier one
    let mut chosen: BTreeMap<String, (usize, Vec<u8>)> = BTreeMap::new();
    let mut counts = Vec::new();
    for archive in ASSET_ARCHIVES {
        let Some(path) = sources.get(*archive) else {
            continue;
        };
        let bytes = std::fs::read(path).map_err(io(path))?;
        let zip = Archive::new(*archive, &bytes)?;
        counts.push(AssetCount {
            source: archive.to_string(),
            files: 0,
            written: 0,
            kept: 0,
        });
        for e in zip.entries().iter().filter(|e| !e.is_dir()) {
            let name = e.file_name().to_ascii_uppercase();
            chosen.insert(name, (counts.len() - 1, zip.read(e)?));
        }
    }
    counts.push(AssetCount {
        source: LOOSE.to_string(),
        files: 0,
        written: 0,
        kept: 0,
    });
    let loose = counts.len() - 1;
    let mut names: Vec<_> = sources
        .iter()
        .filter(|(n, _)| has_asset_extension(n) && !is_plus_file(n))
        .collect();
    names.sort();
    for (name, path) in names {
        let bytes = std::fs::read(path).map_err(io(path))?;
        chosen.insert(name.clone(), (loose, bytes));
    }

    let existing = files_by_name(dest)?;
    if !dry_run
        && chosen
            .keys()
            .any(|n| overwrite || !existing.contains_key(n))
    {
        std::fs::create_dir_all(dest).map_err(io(dest))?;
    }
    for (name, (slot, bytes)) in chosen {
        let c = &mut counts[slot];
        c.files += 1;
        // an existing file keeps its own spelling, so no two names differ only in case
        let target = match existing.get(&name) {
            Some(_) if !overwrite => {
                c.kept += 1;
                continue;
            }
            Some(path) => path.clone(),
            None => dest.join(&name),
        };
        if !dry_run {
            std::fs::write(&target, bytes).map_err(io(&target))?;
        }
        c.written += 1;
    }
    Ok(counts)
}
