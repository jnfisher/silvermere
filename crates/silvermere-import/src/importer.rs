//! `silvermere import`: the legacy `.VIR` set into the schema.
//!
//! [`read_content`] opens every file with the Btrieve reader, checks its record length
//! against the layout and decodes every record ([`Content`]); [`Importer::run`] then
//! optionally writes `--json`, de-duplicates by each table's key, converts to the schema
//! types and writes one transaction per table through the `silvermere-db` repositories'
//! `put_many`. Every table gets a count line ([`Count`]).
//!
//! The content/state split: content tables are always written (upserted by key); the
//! state beside them (`room_state` and its floor items, `shop_stock`, `monster_state`,
//! `items.active`) and the state-only tables (active monsters, gangs, bank books, item
//! ownership, characters) follow [`StateWrite`]: created where missing by default,
//! untouched with `--content-only`, rewritten with `--reset-state`.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use silvermere_db::Db;
use silvermere_db::repo::{Account, StateWrite};
use silvermere_types::records::{BankBook, Character};

use crate::assets::{AssetCount, unpack_assets};
use crate::btrieve::{BtrieveError, BtrieveFile, OpenOptions, Version};
use crate::decode::{self, DecodeError, LegacyClock, Record};
use crate::json::{ExportRecord, JsonWriter};
use crate::patch::{PatchReport, apply_patches};

/// The default `--map-files`.
pub const DEFAULT_MAP_FILES: &[&str] = &["WCCMP001.VIR", "NEWMP001.VIR"];

/// The patch file (`--patches` overrides `DIR/WCCUPDAT.DAT`).
pub const UPDATE_FILE: &str = "WCCUPDAT.DAT";

/// What to import and how.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// The directory holding the legacy files (`--from`).
    pub from: PathBuf,
    /// The room files, in order (`--map-files`).
    pub map_files: Vec<String>,
    /// Decode and count only (`--dry-run`).
    pub dry_run: bool,
    /// Write the decoded records as JSON lines here (`--json`).
    pub json: Option<PathBuf>,
    /// Content tables only, state untouched (`--content-only`).
    pub content_only: bool,
    /// Recreate state from content (`--reset-state`).
    pub reset_state: bool,
    /// Import `WCCUSERS` rows as characters (`--with-users`).
    pub with_users: bool,
    /// Keep the first of records with the same key instead of failing
    /// (`--allow-duplicates`).
    pub allow_duplicates: bool,
    /// The zone the legacy server ran in, for the records' DOS dates (`--legacy-tz`,
    /// default UTC).
    pub legacy_tz: TimeZone,
    /// Skip the `WCCUPDAT.DAT` patches (`--no-patches`).
    pub no_patches: bool,
    /// The patch file (`--patches`); default `WCCUPDAT.DAT` in `from`.
    pub patches: Option<PathBuf>,
    /// Read a running realm's `.DAT` set (`--live`; see [`ImportOptions::live`]).
    pub live: bool,
    /// Unpack the runtime text files here (`<data_dir>/assets/`); `None` skips them.
    pub assets: Option<PathBuf>,
    /// Replace asset files that are already there (`--overwrite-assets`).
    pub overwrite_assets: bool,
}

impl ImportOptions {
    /// The defaults for a directory.
    pub fn new(from: impl Into<PathBuf>) -> Self {
        Self {
            from: from.into(),
            map_files: DEFAULT_MAP_FILES.iter().map(|s| s.to_string()).collect(),
            dry_run: false,
            json: None,
            content_only: false,
            reset_state: false,
            with_users: false,
            allow_duplicates: false,
            legacy_tz: TimeZone::UTC,
            no_patches: false,
            patches: None,
            live: false,
            assets: None,
            overwrite_assets: false,
        }
    }

    /// The defaults for a running realm's directory (`--live`): every file is read from its
    /// `.DAT`, the one the DLL opens (with the host's `opnbtv`), or from its `.VIR` only
    /// where the `.DAT` is missing. Inferred [I]: the MBBS/Worldgroup host creates a missing
    /// `.DAT` from its `.VIR`, and `WCCMMUD.RLN` warns against copying a `.VIR` over a `.DAT`
    /// that is there ("bad side-effects", `wccacmsr` alone excepted); rooms come from
    /// `WCCMP001` only (the DLL never opens `NEWMP001`); the players are imported
    /// (`with_users`).
    pub fn live(from: impl Into<PathBuf>) -> Self {
        Self {
            map_files: vec![DEFAULT_MAP_FILES[0].to_string()],
            with_users: true,
            live: true,
            ..Self::new(from)
        }
    }

    /// The state mode the flags select.
    pub fn state_write(&self) -> StateWrite {
        if self.content_only {
            StateWrite::Untouched
        } else if self.reset_state {
            StateWrite::Reset
        } else {
            StateWrite::CreateMissing
        }
    }
}

/// The importer's errors.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// A legacy file is not in `--from`.
    #[error("{0}: not found")]
    Missing(PathBuf),
    /// The Btrieve reader failed.
    #[error(transparent)]
    Btrieve(#[from] BtrieveError),
    /// A record does not fit its layout.
    #[error("{file} record {ordinal}: {source}")]
    Decode {
        /// The file.
        file: String,
        /// The record's ordinal.
        ordinal: u32,
        /// What was wrong.
        source: DecodeError,
    },
    /// Records were skipped (duplicate keys or no owner) without `--allow-duplicates`.
    #[error("{0} record(s) skipped; see the count table (--allow-duplicates keeps the first)")]
    Skipped(u32),
    /// `--content-only` and `--reset-state` together.
    #[error("--content-only and --reset-state exclude each other")]
    Flags,
    /// A database write failed.
    #[error("{table}: {source}")]
    Db {
        /// The table being written.
        table: &'static str,
        /// The error.
        source: silvermere_db::DbError,
    },
    /// Writing `--json` failed.
    #[error("--json: {0}")]
    Json(#[from] std::io::Error),
    /// A record holds an invalid DOS date (the error names the record and member).
    #[error(transparent)]
    Date(DecodeError),
    /// `WCCUPDAT.DAT` holds a record the patch pass cannot apply.
    #[error("WCCUPDAT.DAT: {0}")]
    Patch(String),
    /// A pre-image file lies beside the data file: changes not yet committed.
    #[error(
        "{}: {} beside it holds uncommitted changes (the BBS is running or was not shut \
         down cleanly); stop the BBS cleanly, or open the file once with Btrieve to roll it \
         back, then import again",
        file.display(),
        pre
    )]
    PreImage {
        /// The data file.
        file: PathBuf,
        /// The pre-image file's name.
        pre: String,
    },
    /// The file continues in an extension file, or is cut short.
    #[error("{}: {reason}; extended (multi-file) and truncated Btrieve files are not supported", file.display())]
    Extended {
        /// The data file.
        file: PathBuf,
        /// What was found.
        reason: String,
    },
    /// Unpacking the runtime text files failed.
    #[error("assets: {0}")]
    Assets(#[from] crate::assets::AssetError),
}

/// A decoded record with where it came from.
#[derive(Debug, Clone)]
pub struct Decoded<R> {
    /// The file it was read from.
    pub file: String,
    /// The reader's 1-based ordinal in that file (T-006's `_id`).
    pub ordinal: u32,
    /// The record.
    pub record: R,
}

/// Every legacy file decoded: the content and the virgin state it carries.
#[derive(Debug, Clone, Default)]
pub struct Content {
    /// `--map-files` in order.
    pub rooms: Vec<Decoded<decode::RoomRecord>>,
    /// WCCITEMS.
    pub items: Vec<Decoded<decode::ItemRecord>>,
    /// WCCKNMSR.
    pub monsters: Vec<Decoded<decode::MonsterRecord>>,
    /// WCCACMSR.
    pub active_monsters: Vec<Decoded<decode::ActmonRecord>>,
    /// WCCSPELS.
    pub spells: Vec<Decoded<decode::SpellRecord>>,
    /// WCCCLASS.
    pub classes: Vec<Decoded<decode::ClassRecord>>,
    /// WCCRACE.
    pub races: Vec<Decoded<decode::RaceRecord>>,
    /// WCCSHOPS.
    pub shops: Vec<Decoded<decode::ShopRecord>>,
    /// WCCMSG.
    pub messages: Vec<Decoded<decode::MudmsgRecord>>,
    /// WCCTEXT.
    pub text_blocks: Vec<Decoded<decode::TextblkRecord>>,
    /// WCCACTS.
    pub actions: Vec<Decoded<decode::ActionRecord>>,
    /// WCCGANGS.
    pub gangs: Vec<Decoded<decode::GangRecord>>,
    /// WCCBANKS.
    pub bank_books: Vec<Decoded<decode::BankbookRecord>>,
    /// WCCITOWN.
    pub item_ownership: Vec<Decoded<decode::ItemownRecord>>,
    /// WCCUSERS.
    pub users: Vec<Decoded<decode::PlayerRecord>>,
    /// The file each table was read from, by its `.VIR` name, where it differs (a live
    /// realm's `.DAT`).
    pub files: std::collections::HashMap<String, String>,
    /// Records read that the schema has no table for (`WCCVACN.DAT`).
    pub uncarried: Vec<Note>,
}

impl Content {
    /// The file a table was read from: its `.DAT` in a live import, else `vir`.
    pub fn file(&self, vir: &str) -> String {
        self.files
            .get(vir)
            .cloned()
            .unwrap_or_else(|| vir.to_string())
    }
}

/// A record that was read and not written, reported on its own line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// The file.
    pub file: String,
    /// The reader's ordinal.
    pub ordinal: u32,
    /// The record's key, for the reader of the report.
    pub key: String,
    /// Why it was not written.
    pub reason: String,
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not carried [{}] record {}", self.file, self.ordinal)?;
        if !self.key.is_empty() {
            write!(f, " ({})", self.key)?;
        }
        write!(f, ": {}", self.reason)
    }
}

/// One count line: `table [file]: N records read, N written, N skipped (reason)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Count {
    /// The schema table (or `patches` for WCCUPDAT.DAT).
    pub table: &'static str,
    /// The legacy file.
    pub file: String,
    /// Records the reader returned.
    pub read: u32,
    /// Records written (in a dry run, to be written).
    pub written: u32,
    /// Records not written.
    pub skipped: u32,
    /// Why they were not written.
    pub reason: Option<String>,
    /// Whether the skips are an error (duplicates, missing owners) or by request.
    pub skip_is_error: bool,
}

impl Count {
    /// The count line; a dry run says what would be written.
    pub fn line(&self, dry_run: bool) -> String {
        let verb = if dry_run { "to write" } else { "written" };
        let mut line = format!(
            "{} [{}]: {} records read, {} {verb}, {} skipped",
            self.table, self.file, self.read, self.written, self.skipped
        );
        if let Some(reason) = &self.reason {
            line.push_str(&format!(" ({reason})"));
        }
        line
    }
}

impl fmt::Display for Count {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.line(false))
    }
}

/// What a run did.
#[derive(Debug, Clone)]
pub struct Report {
    /// One line per file, in reading order.
    pub counts: Vec<Count>,
    /// Whether nothing was written.
    pub dry_run: bool,
    /// Time to read and decode.
    pub read_time: Duration,
    /// Time to write.
    pub write_time: Duration,
    /// What the `WCCUPDAT.DAT` pass did, when it ran.
    pub patches: Option<PatchReport>,
    /// The runtime text files unpacked, when `assets` was given.
    pub assets: Vec<AssetCount>,
    /// Every record read and not written that is not a duplicate or a skip by request:
    /// one line each.
    pub uncarried: Vec<Note>,
}

impl Report {
    /// The count for `file`.
    pub fn count(&self, file: &str) -> Option<&Count> {
        self.counts
            .iter()
            .find(|c| c.file.eq_ignore_ascii_case(file))
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.counts {
            writeln!(f, "{}", c.line(self.dry_run))?;
        }
        if let Some(p) = &self.patches {
            write!(f, "{p}")?;
        }
        for a in &self.assets {
            writeln!(f, "{}", a.line(self.dry_run))?;
        }
        for n in &self.uncarried {
            writeln!(f, "{n}")?;
        }
        if self.dry_run {
            writeln!(f, "dry run: nothing written")?;
        }
        Ok(())
    }
}

/// `name` in `dir`, matched case-insensitively (the distribution's names are upper case;
/// an unpacked copy may not be).
pub fn find_file(dir: &Path, name: &str) -> Result<PathBuf, ImportError> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Ok(exact);
    }
    let entries = std::fs::read_dir(dir).map_err(|_| ImportError::Missing(exact.clone()))?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Ok(entry.path());
        }
    }
    Err(ImportError::Missing(exact))
}

/// Opens `name` in `dir` and decodes every record with `decode`.
pub fn read_file<R: Record>(
    dir: &Path,
    name: &str,
    decode: impl Fn(&[u8]) -> Result<R, DecodeError>,
) -> Result<Vec<Decoded<R>>, ImportError> {
    let path = find_file(dir, name)?;
    let variable =
        name.eq_ignore_ascii_case("WCCTEXT.VIR") || name.eq_ignore_ascii_case("WCCTEXT.DAT");
    preflight_siblings(&path)?;
    let mut file: BtrieveFile = OpenOptions::new().variable_length(variable).open(&path)?;
    preflight_pages(&file)?;
    if let Err(e) = file.expect_record_len(R::LAYOUT.len) {
        // An empty file decodes to nothing whatever its record length: the stock
        // NEWMP001.VIR is a v6 file of 1544-byte records with none in it (T-007).
        if file.header.record_count != 0 {
            return Err(e.into());
        }
        tracing::warn!(file = name, "{e}; the file is empty, so nothing is decoded");
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(file.header.record_count as usize);
    let mut records = file.records();
    while let Some(record) = records.next_record() {
        let record = record?;
        let decoded = decode(record.data).map_err(|source| ImportError::Decode {
            file: name.to_string(),
            ordinal: record.ordinal,
            source,
        })?;
        out.push(Decoded {
            file: name.to_string(),
            ordinal: record.ordinal,
            record: decoded,
        });
    }
    Ok(out)
}

/// A sibling of `path` with extension `ext`, any case.
fn sibling(path: &Path, ext: &str) -> Option<PathBuf> {
    let dir = path.parent()?;
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    find_file(dir, &format!("{stem}.{ext}")).ok()
}

/// Files beside a data file that say its content is not all in it: a Btrieve 5 pre-image
/// file (`.PRE`, written while changes are uncommitted; the DLL opens its files in the
/// host's pre-image mode) and an extension file
/// (`.^01`, the continuation of an extended file).
fn preflight_siblings(path: &Path) -> Result<(), ImportError> {
    if let Some(pre) = sibling(path, "PRE") {
        return Err(ImportError::PreImage {
            file: path.to_path_buf(),
            pre: pre
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        });
    }
    if let Some(ext) = sibling(path, "^01") {
        return Err(ImportError::Extended {
            file: path.to_path_buf(),
            reason: format!(
                "it continues in {}",
                ext.file_name().unwrap_or_default().to_string_lossy()
            ),
        });
    }
    Ok(())
}

/// A v5 file holds every page its header counts (each stock file holds exactly that many);
/// fewer means it continues elsewhere or was cut short. (A v6 header counts differently.)
fn preflight_pages(file: &BtrieveFile) -> Result<(), ImportError> {
    let h = &file.header;
    let held = h.file_length / u64::from(h.page_length);
    if h.version == Version::V5 && u64::from(h.page_count) > held {
        return Err(ImportError::Extended {
            file: file.path().to_path_buf(),
            reason: format!(
                "the header counts {} pages of {} bytes, the file holds {held}",
                h.page_count, h.page_length
            ),
        });
    }
    Ok(())
}

/// The file a table is read from: with `live`, `X.DAT` when there is one, else `vir`.
pub fn table_file(dir: &Path, vir: &str, live: bool) -> String {
    let stem = vir
        .len()
        .checked_sub(4)
        .filter(|&i| vir.is_char_boundary(i) && vir[i..].eq_ignore_ascii_case(".VIR"))
        .map(|i| &vir[..i]);
    if live && let Some(stem) = stem {
        let dat = format!("{}.DAT", stem.to_ascii_uppercase());
        if find_file(dir, &dat).is_ok() {
            return dat;
        }
    }
    vir.to_string()
}

/// The vacation file the DLL opens when it is there; the feature is a stub offered only on
/// the developers' own systems, so silvermere has no table for it.
pub const VACATION_FILE: &str = "WCCVACN.DAT";

/// Reads and decodes every file of the set.
pub fn read_content(opts: &ImportOptions) -> Result<Content, ImportError> {
    let dir = opts.from.as_path();
    let mut files = std::collections::HashMap::new();
    let mut name = |vir: &str| {
        let actual = table_file(dir, vir, opts.live);
        if actual != vir {
            files.insert(vir.to_string(), actual.clone());
        }
        actual
    };
    let mut rooms = Vec::new();
    for map in &opts.map_files {
        rooms.extend(read_file(dir, &name(map), decode::RoomRecord::decode)?);
    }
    let mut content = Content {
        rooms,
        items: read_file(dir, &name("WCCITEMS.VIR"), decode::ItemRecord::decode)?,
        monsters: read_file(dir, &name("WCCKNMSR.VIR"), decode::MonsterRecord::decode)?,
        active_monsters: read_file(dir, &name("WCCACMSR.VIR"), decode::ActmonRecord::decode)?,
        spells: read_file(dir, &name("WCCSPELS.VIR"), decode::SpellRecord::decode)?,
        classes: read_file(dir, &name("WCCCLASS.VIR"), decode::ClassRecord::decode)?,
        races: read_file(dir, &name("WCCRACE.VIR"), decode::RaceRecord::decode)?,
        shops: read_file(dir, &name("WCCSHOPS.VIR"), decode::ShopRecord::decode)?,
        messages: read_file(dir, &name("WCCMSG.VIR"), decode::MudmsgRecord::decode)?,
        text_blocks: read_file(dir, &name("WCCTEXT.VIR"), decode::decode_textblk)?,
        actions: read_file(dir, &name("WCCACTS.VIR"), decode::ActionRecord::decode)?,
        gangs: read_file(dir, &name("WCCGANGS.VIR"), decode::GangRecord::decode)?,
        bank_books: read_file(dir, &name("WCCBANKS.VIR"), decode::BankbookRecord::decode)?,
        item_ownership: read_file(dir, &name("WCCITOWN.VIR"), decode::ItemownRecord::decode)?,
        users: read_file(dir, &name("WCCUSERS.VIR"), decode::PlayerRecord::decode)?,
        ..Content::default()
    };
    content.files = files;
    if opts.live
        && let Ok(path) = find_file(dir, VACATION_FILE)
    {
        preflight_siblings(&path)?;
        let mut file = BtrieveFile::open(&path)?;
        let mut records = file.records();
        while let Some(record) = records.next_record() {
            let record = record?;
            content.uncarried.push(Note {
                file: VACATION_FILE.to_string(),
                ordinal: record.ordinal,
                key: String::new(),
                reason: "silvermere has no vacation table (the feature is a stub)".to_string(),
            });
        }
    }
    Ok(content)
}

/// Writes `--json OUT`: one file per Btrieve file.
pub fn write_json(content: &Content, out: &Path) -> Result<(), ImportError> {
    let w = JsonWriter::new(out)?;
    fn rows<'a, R>(v: &'a [Decoded<R>], file: &'a str) -> impl Iterator<Item = (u32, &'a R)> {
        v.iter()
            .filter(move |d| d.file.eq_ignore_ascii_case(file))
            .map(|d| (d.ordinal, &d.record))
    }
    fn table(file: &str) -> &str {
        file.split('.').next().unwrap_or(file)
    }
    let maps: Vec<&str> = {
        let mut seen = Vec::new();
        for d in &content.rooms {
            if !seen.contains(&d.file.as_str()) {
                seen.push(d.file.as_str());
            }
        }
        seen
    };
    for map in maps {
        w.write(table(map), rows(&content.rooms, map))?;
    }
    write_all(&w, "WCCITEMS", &content.items)?;
    write_all(&w, "WCCKNMSR", &content.monsters)?;
    write_all(&w, "WCCACMSR", &content.active_monsters)?;
    write_all(&w, "WCCSPELS", &content.spells)?;
    write_all(&w, "WCCCLASS", &content.classes)?;
    write_all(&w, "WCCRACE", &content.races)?;
    write_all(&w, "WCCSHOPS", &content.shops)?;
    write_all(&w, "WCCMSG", &content.messages)?;
    write_all(&w, "WCCTEXT", &content.text_blocks)?;
    write_all(&w, "WCCACTS", &content.actions)?;
    write_all(&w, "WCCGANGS", &content.gangs)?;
    write_all(&w, "WCCBANKS", &content.bank_books)?;
    write_all(&w, "WCCITOWN", &content.item_ownership)?;
    write_all(&w, "WCCUSERS", &content.users)?;
    Ok(())
}

fn write_all<R: ExportRecord>(
    w: &JsonWriter,
    table: &str,
    v: &[Decoded<R>],
) -> Result<(), ImportError> {
    w.write(table, v.iter().map(|d| (d.ordinal, &d.record)))?;
    Ok(())
}

/// Keeps the first record of each key; the count of the rest per file.
struct Kept<'a, R> {
    records: Vec<&'a Decoded<R>>,
    /// `(file, read, duplicates)` in first-seen file order.
    files: Vec<(String, u32, u32)>,
}

fn dedupe<'a, R, K: std::hash::Hash + Eq>(
    v: &'a [Decoded<R>],
    files: &[&str],
    key: impl Fn(&R) -> K,
) -> Kept<'a, R> {
    let mut seen = HashSet::new();
    let mut kept = Kept {
        records: Vec::with_capacity(v.len()),
        files: files.iter().map(|f| (f.to_string(), 0, 0)).collect(),
    };
    for d in v {
        let slot = match kept.files.iter().position(|f| f.0 == d.file) {
            Some(i) => i,
            None => {
                kept.files.push((d.file.clone(), 0, 0));
                kept.files.len() - 1
            }
        };
        kept.files[slot].1 += 1;
        if seen.insert(key(&d.record)) {
            kept.records.push(d);
        } else {
            kept.files[slot].2 += 1;
        }
    }
    kept
}

/// The importer: options plus the database it writes to (none for a dry run).
pub struct Importer<'a> {
    opts: ImportOptions,
    db: Option<&'a Db>,
}

impl<'a> Importer<'a> {
    /// An importer writing to `db` (pass `None` with `dry_run`).
    pub fn new(opts: ImportOptions, db: Option<&'a Db>) -> Self {
        Self { opts, db }
    }

    /// Reads, decodes, exports and writes; the count table on success. Skips are an
    /// error unless `--allow-duplicates`; the error is returned after the counts are
    /// complete and before anything is written.
    pub async fn run(&self) -> Result<Report, ImportError> {
        if self.opts.content_only && self.opts.reset_state {
            return Err(ImportError::Flags);
        }
        let start = Instant::now();
        let mut content = read_content(&self.opts)?;
        let patches = self.patch(&mut content)?;
        let mut report = self.run_content(&content, start).await?;
        report.patches = patches;
        if let Some(dest) = &self.opts.assets {
            report.assets = unpack_assets(
                &self.opts.from,
                dest,
                self.opts.overwrite_assets,
                self.opts.dry_run,
            )?;
        }
        Ok(report)
    }

    /// The `WCCUPDAT.DAT` pass between decode and write (plan `04` "Importer"): skipped
    /// with `--no-patches`; a missing file is a warning, not an error.
    pub fn patch(&self, content: &mut Content) -> Result<Option<PatchReport>, ImportError> {
        if self.opts.no_patches {
            return Ok(None);
        }
        let path = match &self.opts.patches {
            Some(path) if path.is_file() => path.clone(),
            Some(path) => {
                tracing::warn!("{}: not found; no patches applied", path.display());
                return Ok(None);
            }
            None => match find_file(&self.opts.from, UPDATE_FILE) {
                Ok(path) => path,
                Err(e) => {
                    tracing::warn!("{e}; no patches applied");
                    return Ok(None);
                }
            },
        };
        apply_patches(content, &path).map(Some)
    }

    /// [`Importer::run`] on records already read (`read_started` is when reading began,
    /// for the report's timing). Every check runs before anything is written.
    pub async fn run_content(
        &self,
        content: &Content,
        read_started: Instant,
    ) -> Result<Report, ImportError> {
        if self.opts.content_only && self.opts.reset_state {
            return Err(ImportError::Flags);
        }
        if let Some(out) = &self.opts.json {
            write_json(content, out)?;
        }
        let mut plan = Plan::new(content, &self.opts);
        let clock = LegacyClock::new(self.opts.legacy_tz.clone(), jiff::Timestamp::now());
        plan.check_dates(&clock)?;
        // a dry run has no database: it checks the books against this import's characters
        plan.check_bank_book_accounts(self.db.filter(|_| !self.opts.dry_run), self.opts.live)
            .await?;
        let read_time = read_started.elapsed();
        let errors: u32 = plan
            .counts
            .iter()
            .filter(|c| c.skip_is_error)
            .map(|c| c.skipped)
            .sum();
        if errors > 0 && !self.opts.allow_duplicates {
            for c in &plan.counts {
                tracing::error!("{c}");
            }
            return Err(ImportError::Skipped(errors));
        }
        let start = Instant::now();
        if !self.opts.dry_run {
            let db = self.db.expect("a database unless --dry-run");
            // T-109: marked before the first table and after the last, so a start can tell
            // a world this import left half written
            let marker = |source| ImportError::Db {
                table: "world_import",
                source,
            };
            silvermere_db::world_import::begin(db)
                .await
                .map_err(marker)?;
            let kept = plan.write(db, &self.opts, &clock).await?;
            silvermere_db::world_import::finish(db)
                .await
                .map_err(marker)?;
            plan.notes.extend(kept);
        }
        let mut uncarried = content.uncarried.clone();
        uncarried.append(&mut plan.notes);
        Ok(Report {
            counts: plan.counts,
            dry_run: self.opts.dry_run,
            read_time,
            write_time: start.elapsed(),
            patches: None,
            assets: Vec::new(),
            uncarried,
        })
    }
}

/// The records to write, de-duplicated and converted, with their counts.
struct Plan<'a> {
    counts: Vec<Count>,
    /// Records read and not written, one line each.
    notes: Vec<Note>,
    rooms: Vec<&'a Decoded<decode::RoomRecord>>,
    items: Vec<&'a Decoded<decode::ItemRecord>>,
    monsters: Vec<&'a Decoded<decode::MonsterRecord>>,
    active_monsters: Vec<&'a Decoded<decode::ActmonRecord>>,
    spells: Vec<&'a Decoded<decode::SpellRecord>>,
    classes: Vec<&'a Decoded<decode::ClassRecord>>,
    races: Vec<&'a Decoded<decode::RaceRecord>>,
    shops: Vec<&'a Decoded<decode::ShopRecord>>,
    messages: Vec<&'a Decoded<decode::MudmsgRecord>>,
    text_blocks: Vec<&'a Decoded<decode::TextblkRecord>>,
    actions: Vec<&'a Decoded<decode::ActionRecord>>,
    gangs: Vec<&'a Decoded<decode::GangRecord>>,
    bank_books: Vec<&'a Decoded<decode::BankbookRecord>>,
    item_ownership: Vec<&'a Decoded<decode::ItemownRecord>>,
    users: Vec<&'a Decoded<decode::PlayerRecord>>,
}

impl<'a> Plan<'a> {
    fn new(content: &'a Content, opts: &ImportOptions) -> Self {
        let mut counts = Vec::new();
        let state = opts.state_write();
        // `count` records the lines of one table; a table not written is skipped by request.
        fn count<'k, R>(
            counts: &mut Vec<Count>,
            table: &'static str,
            kept: Kept<'k, R>,
            write: Option<&str>,
            key: &str,
        ) -> Vec<&'k Decoded<R>> {
            for (file, read, dups) in &kept.files {
                let (written, skipped, reason, skip_is_error) = match write {
                    None if *dups > 0 => {
                        (read - dups, *dups, Some(format!("duplicate {key}")), true)
                    }
                    None => (*read, 0, None, false),
                    Some(why) => (0, *read, (*read > 0).then(|| why.to_string()), false),
                };
                counts.push(Count {
                    table,
                    file: file.clone(),
                    read: *read,
                    written,
                    skipped,
                    reason,
                    skip_is_error,
                });
            }
            if write.is_some() {
                Vec::new()
            } else {
                kept.records
            }
        }

        let maps: Vec<String> = opts.map_files.iter().map(|m| content.file(m)).collect();
        let maps: Vec<&str> = maps.iter().map(String::as_str).collect();
        let state_skip = (state == StateWrite::Untouched).then_some("state: --content-only");
        let users_skip = if !opts.with_users {
            Some("--with-users not given")
        } else {
            state_skip
        };
        let c = content;
        let rooms = count(
            &mut counts,
            "rooms",
            dedupe(&c.rooms, &maps, |r| r.key()),
            None,
            "(map, room)",
        );
        macro_rules! table {
            ($field:ident, $table:literal, $file:literal, $skip:expr, $keyname:literal, $key:expr) => {
                count(
                    &mut counts,
                    $table,
                    dedupe(&c.$field, &[c.file($file).as_str()], $key),
                    $skip,
                    $keyname,
                )
            };
        }
        let items = table!(items, "items", "WCCITEMS.VIR", None, "number", |r| r.number);
        let monsters = table!(monsters, "monsters", "WCCKNMSR.VIR", None, "number", |r| r
            .number);
        let active_monsters = table!(
            active_monsters,
            "active_monsters",
            "WCCACMSR.VIR",
            state_skip,
            "number",
            |r| r.number
        );
        let spells = table!(spells, "spells", "WCCSPELS.VIR", None, "number", |r| r
            .number);
        let classes = table!(classes, "classes", "WCCCLASS.VIR", None, "number", |r| r
            .number);
        let races = table!(races, "races", "WCCRACE.VIR", None, "number", |r| r.number);
        let shops = table!(shops, "shops", "WCCSHOPS.VIR", None, "number", |r| r.number);
        let messages = table!(messages, "messages", "WCCMSG.VIR", None, "number", |r| r
            .number);
        let text_blocks = table!(
            text_blocks,
            "text_blocks",
            "WCCTEXT.VIR",
            None,
            "(part, number)",
            |r| (r.part, r.number)
        );
        let actions = table!(actions, "actions", "WCCACTS.VIR", None, "name", |r| r
            .name
            .trimmed()
            .to_lowercase());
        let gangs = table!(gangs, "gangs", "WCCGANGS.VIR", state_skip, "key", |r| r
            .key
            .trimmed()
            .to_lowercase());
        let bank_books = table!(
            bank_books,
            "bank_books",
            "WCCBANKS.VIR",
            users_skip,
            "(userid, bank)",
            |r| (r.userid.trimmed().to_lowercase(), r.bank)
        );
        let item_ownership = table!(
            item_ownership,
            "item_ownership",
            "WCCITOWN.VIR",
            state_skip,
            "id",
            |r| r.id
        );
        let users = table!(
            users,
            "characters",
            "WCCUSERS.VIR",
            users_skip,
            "userid",
            |r| r.userid.trimmed().to_lowercase()
        );
        Self {
            counts,
            notes: Vec::new(),
            rooms,
            items,
            monsters,
            active_monsters,
            spells,
            classes,
            races,
            shops,
            messages,
            text_blocks,
            actions,
            gangs,
            bank_books,
            item_ownership,
            users,
        }
    }

    /// Converts the records that hold DOS dates, so an invalid one fails the run (a dry
    /// run too) before anything is written.
    fn check_dates(&self, clock: &LegacyClock) -> Result<(), ImportError> {
        try_conv(&self.monsters, |r| decode::split_monster(r, clock))?;
        try_conv(&self.active_monsters, |r| {
            decode::to_active_monster(r, clock)
        })?;
        try_conv(&self.gangs, |r| decode::to_gang(r, clock))?;
        try_conv(&self.users, |r| decode::to_character(r, clock))?;
        Ok(())
    }

    /// Bank books are written under the account of their userid (`--with-users`). A book
    /// whose userid has no account, and is not a character of this import (which creates
    /// one), is a skip, found here before anything is written, and reported per record. It
    /// is an error unless `--allow-duplicates`, except with `--live` (a realm keeps the books
    /// of players since deleted; John, 2026-10-17). A dry run opens no database, so `db` is
    /// `None` and a book counts as orphaned unless its owner is a character of the import.
    async fn check_bank_book_accounts(
        &mut self,
        db: Option<&Db>,
        live: bool,
    ) -> Result<(), ImportError> {
        if self.bank_books.is_empty() {
            return Ok(());
        }
        let users: HashSet<String> = self
            .users
            .iter()
            .map(|d| d.record.userid.trimmed().to_lowercase())
            .collect();
        let mut missing = 0;
        for d in &self.bank_books {
            let userid = d.record.userid.trimmed();
            if users.contains(&userid.to_lowercase()) {
                continue;
            }
            let found = match db {
                Some(db) => db
                    .accounts()
                    .by_username(&userid)
                    .await
                    .map_err(|source| ImportError::Db {
                        table: "accounts",
                        source,
                    })?
                    .is_some(),
                None => false,
            };
            if !found {
                missing += 1;
                self.notes.push(Note {
                    file: d.file.clone(),
                    ordinal: d.ordinal,
                    key: format!("userid {userid}, bank {}", d.record.bank),
                    reason: format!("its owner {userid} has no account and no character"),
                });
            }
        }
        if missing > 0
            && let Some(c) = self.counts.iter_mut().find(|c| c.table == "bank_books")
        {
            c.written -= missing;
            c.skipped += missing;
            // A live realm keeps the books of players since deleted: skipped and reported
            // (John, 2026-10-17); in the virgin set any such book is an error.
            c.skip_is_error = !live;
            c.reason = Some("no account for the userid".to_string());
        }
        Ok(())
    }

    /// Writes every table, one transaction each.
    async fn write(
        &self,
        db: &Db,
        opts: &ImportOptions,
        clock: &LegacyClock,
    ) -> Result<Vec<Note>, ImportError> {
        let state = opts.state_write();
        fn conv<R, T>(v: &[&Decoded<R>], f: impl Fn(&R) -> T) -> Vec<T> {
            v.iter().map(|d| f(&d.record)).collect()
        }

        let rooms = conv(&self.rooms, |r| decode::split_room(r).0);
        timed("rooms", rooms.len(), db.rooms().put_many(&rooms, state)).await?;
        let items = conv(&self.items, decode::to_item);
        timed("items", items.len(), db.items().put_many(&items, state)).await?;
        let monsters: Vec<_> = try_conv(&self.monsters, |r| decode::split_monster(r, clock))?
            .into_iter()
            .map(|(m, _)| m)
            .collect();
        timed(
            "monsters",
            monsters.len(),
            db.monsters().put_many(&monsters, state),
        )
        .await?;
        let spells = conv(&self.spells, decode::to_spell);
        timed("spells", spells.len(), db.spells().put_many(&spells)).await?;
        let classes = conv(&self.classes, decode::to_class);
        timed("classes", classes.len(), db.classes().put_many(&classes)).await?;
        let races = conv(&self.races, decode::to_race);
        timed("races", races.len(), db.races().put_many(&races)).await?;
        let shops = conv(&self.shops, |r| decode::split_shop(r).0);
        timed("shops", shops.len(), db.shops().put_many(&shops, state)).await?;
        let messages = conv(&self.messages, decode::to_message);
        timed(
            "messages",
            messages.len(),
            db.messages().put_many(&messages),
        )
        .await?;
        let texts = conv(&self.text_blocks, decode::to_text_block);
        timed(
            "text_blocks",
            texts.len(),
            db.text_blocks().put_many(&texts),
        )
        .await?;
        let actions = conv(&self.actions, decode::to_action);
        timed("actions", actions.len(), db.actions().put_many(&actions)).await?;
        let active = try_conv(&self.active_monsters, |r| {
            decode::to_active_monster(r, clock)
        })?;
        timed(
            "active_monsters",
            active.len(),
            db.active_monsters().put_many(&active, state),
        )
        .await?;
        let gangs = try_conv(&self.gangs, |r| decode::to_gang(r, clock))?;
        timed("gangs", gangs.len(), db.gangs().put_many(&gangs, state)).await?;
        let owns = conv(&self.item_ownership, decode::to_item_ownership);
        timed(
            "item_ownership",
            owns.len(),
            db.item_ownership().put_many(&owns, state),
        )
        .await?;
        let characters = try_conv(&self.users, |r| decode::to_character(r, clock))?;
        let kept = write_characters(db, &characters, state).await?;
        let notes = kept
            .into_iter()
            .map(|i| {
                let d = self.users[i];
                Note {
                    file: d.file.clone(),
                    ordinal: d.ordinal,
                    key: format!("userid {}", d.record.userid.trimmed()),
                    reason: "the account already has a character, kept (--reset-state \
                             rewrites it)"
                        .to_string(),
                }
            })
            .collect();
        let books = conv(&self.bank_books, decode::to_bank_book);
        // Books without an account were counted (and refused unless --allow-duplicates)
        // by `check_bank_book_accounts`; they are skipped here.
        write_bank_books(db, &books, state).await?;
        Ok(notes)
    }
}

/// Converts every record with a fallible conversion (the ones holding DOS dates).
fn try_conv<R, T>(
    v: &[&Decoded<R>],
    f: impl Fn(&R) -> Result<T, DecodeError>,
) -> Result<Vec<T>, ImportError> {
    v.iter()
        .map(|d| f(&d.record).map_err(ImportError::Date))
        .collect()
}

/// Awaits one table's write, logging when it starts (a slow database shows progress, not a
/// hang, T-108) and how long it took.
async fn timed(
    table: &'static str,
    records: usize,
    write: impl std::future::Future<Output = silvermere_db::Result<()>>,
) -> Result<(), ImportError> {
    tracing::info!(table, records, "writing");
    let start = Instant::now();
    write
        .await
        .map_err(|source| ImportError::Db { table, source })?;
    tracing::info!(table, records, elapsed = ?start.elapsed(), "written");
    Ok(())
}

/// The account a character or bank book belongs to: the one named by the userid,
/// created (with no password) for a character that has none.
async fn account_for(
    db: &Db,
    userid: &str,
    create: bool,
) -> Result<Option<silvermere_types::ids::AccountId>, ImportError> {
    let accounts = db.accounts();
    let e = |source| ImportError::Db {
        table: "accounts",
        source,
    };
    if let Some(row) = accounts.by_username(userid).await.map_err(e)? {
        return Ok(Some(row.id));
    }
    if !create {
        return Ok(None);
    }
    let account = Account::new(userid, "", jiff::Timestamp::now());
    Ok(Some(accounts.insert(&account).await.map_err(e)?))
}

/// `--with-users`: each character under the account of its userid. A character is
/// inserted when its account has none, rewritten with `--reset-state`. Returns the indexes
/// of the characters not written because their account already has one.
async fn write_characters(
    db: &Db,
    characters: &[Character],
    state: StateWrite,
) -> Result<Vec<usize>, ImportError> {
    let e = |source| ImportError::Db {
        table: "characters",
        source,
    };
    let mut kept = Vec::new();
    for (i, c) in characters.iter().enumerate() {
        let Some(account) = account_for(db, &c.userid, true).await? else {
            continue;
        };
        match db.characters().by_account(account).await.map_err(e)? {
            Some(row) if state == StateWrite::Reset => {
                db.characters().update(row.id, c).await.map_err(e)?;
            }
            Some(_) => kept.push(i),
            None => {
                db.characters().insert(account, c).await.map_err(e)?;
            }
        }
    }
    Ok(kept)
}

/// Bank books under the account of their userid; the number with no such account.
async fn write_bank_books(
    db: &Db,
    books: &[BankBook],
    state: StateWrite,
) -> Result<u32, ImportError> {
    let e = |source| ImportError::Db {
        table: "bank_books",
        source,
    };
    let mut missing = 0;
    for book in books {
        let Some(account) = account_for(db, &book.userid, false).await? else {
            missing += 1;
            continue;
        };
        let exists = db
            .bank_books()
            .get(account, book.bank)
            .await
            .map_err(e)?
            .is_some();
        if state.writes(exists) {
            db.bank_books().put(account, book).await.map_err(e)?;
        }
    }
    Ok(missing)
}
