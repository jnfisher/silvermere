//! `silvermere import`: the legacy `.VIR` set into the database (T-008).
//!
//! [`ImportCmd`] is the `import` subcommand of T-017's clap tree (`cli::ImportArgs`), and
//! `commands::import` calls [`run`]. The database comes from the configuration
//! (`database.url`, which `--database-url` overrides); logging is set up by the CLI.
//!
//! The source (T-107, T-120): `--from DIR`, `FILE.zip` or `--from FILE.zip` (unpacked into a
//! temporary directory first), or nothing, which imports the stock 1.11p distribution,
//! downloaded once into `<data_dir>/cache/` and checked ([`crate::stock`]); `--download`
//! fetches it again. `--live` reads a running realm's `.DAT` set. The runtime text files go
//! into `<data_dir>/assets/`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use jiff::tz::TimeZone;
use silvermere_db::Db;
use silvermere_import::{ImportOptions, Importer};

use crate::config::Config;

/// `import` flags (`docs/import.md`).
#[derive(Debug, Clone, clap::Args)]
pub struct ImportCmd {
    /// A .zip (or directory) holding the WCC*.VIR files and WCCUPDAT.DAT, such as the
    /// MBBSEmu package; works offline. Default: the stock 1.11p distribution, downloaded once
    /// into <data_dir>/cache/ and checked.
    #[arg(value_name = "ZIP|DIR", conflicts_with = "from")]
    pub source: Option<PathBuf>,
    /// Directory (or .zip) holding the WCC*.VIR files and WCCUPDAT.DAT.
    #[arg(long, value_name = "DIR|ZIP")]
    pub from: Option<PathBuf>,
    /// Download the stock distribution again, replacing the cached copy, and import it.
    #[arg(long, conflicts_with_all = ["source", "from"])]
    pub download: bool,
    /// Read a running realm's WCC*.DAT files (a missing .DAT falls back to its .VIR) and
    /// import its players; needs --from.
    #[arg(long, requires = "from")]
    pub live: bool,
    /// The room files, in order (comma separated); default WCCMP001.VIR,NEWMP001.VIR, or
    /// WCCMP001 alone with --live.
    #[arg(long, value_delimiter = ',')]
    pub map_files: Option<Vec<String>>,
    /// Replace the files already in <data_dir>/assets/ (by default they are kept).
    #[arg(long)]
    pub overwrite_assets: bool,
    /// Decode and count only; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Also write the decoded records as OUT/<TABLE>.jsonl (T-006's shape).
    #[arg(long, value_name = "OUT")]
    pub json: Option<PathBuf>,
    /// Write content tables only; leave state untouched.
    #[arg(long, conflicts_with = "reset_state")]
    pub content_only: bool,
    /// Recreate state (room floors, shop stock, monster state, ...) from the content.
    #[arg(long)]
    pub reset_state: bool,
    /// Import WCCUSERS records as characters (for differential tests).
    #[arg(long)]
    pub with_users: bool,
    /// Keep the first of records with the same key instead of failing.
    #[arg(long)]
    pub allow_duplicates: bool,
    /// Skip the WCCUPDAT.DAT patches.
    #[arg(long, conflicts_with = "patches")]
    pub no_patches: bool,
    /// The patch file to apply (default: WCCUPDAT.DAT in --from); missing = a warning.
    #[arg(long, value_name = "PATH")]
    pub patches: Option<PathBuf>,
    /// The zone the legacy server ran in (IANA name), for the records' DOS dates.
    #[arg(long, value_name = "ZONE", default_value = "UTC", value_parser = parse_zone)]
    pub legacy_tz: TimeZone,
    /// Database URL (sqlite:// or postgres://); overrides `database.url`.
    #[arg(long)]
    pub database_url: Option<String>,
}

impl ImportCmd {
    fn options(&self, from: &Path, assets: PathBuf) -> ImportOptions {
        let base = if self.live {
            ImportOptions::live(from)
        } else {
            ImportOptions::new(from)
        };
        ImportOptions {
            map_files: self.map_files.clone().unwrap_or(base.map_files.clone()),
            dry_run: self.dry_run,
            json: self.json.clone(),
            content_only: self.content_only,
            reset_state: self.reset_state,
            with_users: self.with_users || base.with_users,
            allow_duplicates: self.allow_duplicates,
            legacy_tz: self.legacy_tz.clone(),
            no_patches: self.no_patches,
            patches: self.patches.clone(),
            assets: Some(assets),
            overwrite_assets: self.overwrite_assets,
            ..base
        }
    }
}

/// A directory removed when dropped: an unpacked zip.
struct Unpacked(PathBuf);

impl Drop for Unpacked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Unpacks `bytes` (the zip `name`) into a new directory under `base` (the data directory,
/// which is writable where the root file system may not be).
fn unpack(base: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<Unpacked> {
    remove_stale_copies(base);
    let dir = base.join(format!(".import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir = Unpacked(dir);
    silvermere_import::zip::Archive::new(name, bytes)?.unpack_to(&dir.0)?;
    Ok(dir)
}

/// Removes the `.import-<pid>` copies an interrupted import left in `base` (SIGINT or
/// SIGKILL skip the guard): those whose process is gone (where `/proc` says so), or else
/// those more than a day old.
fn remove_stale_copies(base: &Path) {
    let Ok(rd) = std::fs::read_dir(base) else {
        return;
    };
    let proc = Path::new("/proc/self").exists();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix(".import-")
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let stale = if proc {
            !Path::new("/proc").join(pid.to_string()).exists()
        } else {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 24 * 3600)
        };
        if stale {
            tracing::info!(dir = %e.path().display(), "removing an interrupted import's copy");
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// The directory to import from: a directory as it is, a zip (given, or the stock one from
/// the cache) unpacked into `<data_dir>/.import-<pid>/` (the guard removes the copy).
fn source(cmd: &ImportCmd, base: &Path) -> anyhow::Result<(PathBuf, Option<Unpacked>)> {
    match cmd.source.as_ref().or(cmd.from.as_ref()) {
        Some(dir) if dir.is_dir() => Ok((dir.clone(), None)),
        Some(file) if file.is_file() => {
            let bytes =
                std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
            let dir = unpack(base, &file.display().to_string(), &bytes)?;
            Ok((dir.0.clone(), Some(dir)))
        }
        Some(path) => anyhow::bail!("{}: no such directory or file", path.display()),
        None => {
            let zip = crate::stock::fetch(base, cmd.download)?;
            let bytes =
                std::fs::read(&zip).with_context(|| format!("reading {}", zip.display()))?;
            eprintln!("importing the stock {}", crate::stock::NAME);
            let dir = unpack(base, crate::stock::NAME, &bytes)?;
            Ok((dir.0.clone(), Some(dir)))
        }
    }
}

fn parse_zone(name: &str) -> Result<TimeZone, String> {
    TimeZone::get(name).map_err(|e| format!("unknown time zone {name:?}: {e}"))
}

/// Runs the import and prints the count table. Exit 0 on success, 1 on any import error.
pub fn run(cfg: &Config, cmd: &ImportCmd) -> anyhow::Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let db = if cmd.dry_run {
            None
        } else {
            crate::commands::ensure_sqlite_dir(cfg)?;
            Some(
                Db::connect_and_migrate(&cfg.database.url)
                    .await
                    .with_context(|| format!("opening {}", cfg.database_shown()))?,
            )
        };
        // the database first: an unreachable one fails before any download
        let (from, _unpacked) = match tokio::task::block_in_place(|| source(cmd, &cfg.data_dir)) {
            Ok(s) => s,
            Err(e) => {
                if let Some(db) = &db {
                    db.close().await;
                }
                eprintln!("import failed: {e:#}");
                return Ok(ExitCode::from(1));
            }
        };
        let mut opts = cmd.options(&from, cfg.data_dir.join("assets"));
        // the stock package's own DLL must give the long screens (T-138)
        opts.stock_package = cmd.source.is_none() && cmd.from.is_none();
        let result = Importer::new(opts, db.as_ref()).run().await;
        if let Some(db) = &db {
            db.close().await;
        }
        match result {
            Ok(report) => {
                print!("{report}");
                println!(
                    "read and decoded in {:.1?}, written in {:.1?}",
                    report.read_time, report.write_time
                );
                // T-138: the text files whole (a realm's DLL or .MSG may not give them),
                // from the cached package, without downloading
                if !cmd.dry_run {
                    crate::stock::complete_text_assets(&cfg.data_dir, false);
                }
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => {
                eprintln!("import failed: {e}");
                Ok(ExitCode::from(1))
            }
        }
    })
}
