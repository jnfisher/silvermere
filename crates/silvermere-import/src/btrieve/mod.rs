//! Btrieve 5.x/6.x reader for `.DAT` and virgin `.VIR` files.
//!
//! This is the only code in the project that knows the Btrieve file format; the record
//! decoders take the byte slices it yields. It follows the parsing half of MBBSEmu wbtrv32's
//! `btrieve/BtrieveDatabase.cc` (no index, no cursor emulation, no writes).
//!
//! # What it reads
//!
//! - The File Control Record ([`header`]): v6 is `FC\0\0` with two FCR copies, the higher
//!   usage count active; v5 has a version code of 3, 4 or 5 at +7. Wrong page length,
//!   accel flags, the recovery flag and compression are hard errors.
//! - The ACS ([`acs`]): v5 at logical page 1, v6 wherever the PAT says `'A'`, plus a page
//!   of its own for each MultipleACS key.
//! - Key definitions ([`keys`]): 0x1E bytes per segment, v5 consecutive from FCR+0x110,
//!   v6 through the key attribute table.
//! - Data pages (`pages`): v5 logical page n is physical page n; v6 maps through the
//!   PAT (`pat`) and only `'D'`/`'V'` pages count.
//!
//! # Record order (the ordinal contract)
//!
//! Records come out in wbtrv32's order: page by page (logical page order; for v5 that is
//! physical order), and within a page in slot order. Enumeration stops after the first
//! page at which the count reaches the header's record count. The 1-based
//! [`Record::ordinal`] therefore equals the `data_t.id` MBBSEmu assigns when it converts
//! the same file, as long as MBBSEmu did not drop a row for violating a unique key; this
//! reader drops nothing, so its count can exceed an MBBSEmu `.db` row count.
//!
//! # Deleted and unused slots
//!
//! v5: slots on the deleted-record chain (FCR+0x10) are skipped, and a slot holding a
//! free-space link followed by zeros ends the page. v6: a slot whose big-endian usage
//! count is 0 is a hole and is skipped.
//!
//! # Variable-length records
//!
//! [`BtrieveFile::open`] refuses them with [`BtrieveError::VariableLength`]. MajorMUD's
//! `WCCTEXT` is variable-length (fixed part 22 bytes, records up to 2022), so
//! [`OpenOptions::variable_length`] opts in to the fragment walk of wbtrv32's
//! `getVariableLengthData`.

pub mod acs;
pub mod error;
pub mod header;
pub mod keys;
mod pages;
mod pat;

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

pub use acs::Acs;
pub use error::BtrieveError;
pub use header::{Header, RecordType, Version};
pub use keys::{Key, KeyAttributes, KeyDataType, KeyDef};

use error::{Fault, ReadError};
use header::MIN_PAGE_LENGTH;
use keys::{KEY_DEF_LENGTH, KeyAttributes as Attr, V5_KEY_DEF_BASE};
use pages::{PageHeader, PageReader, Slot, Source};
use pat::{Pat, PatEntry};

/// Read buffer for the page walk; several pages per read on every MajorMUD file.
const READ_BUFFER: usize = 64 * 1024;

/// A key definition chain longer than this is corrupt (Btrieve allows 119 segments).
const MAX_SEGMENTS: usize = 1024;

/// How to open a file.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    variable_length: bool,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept variable-length files and append each record's fragment chain to its fixed
    /// part. Off by default: a variable-length file is then an error.
    pub fn variable_length(mut self, yes: bool) -> Self {
        self.variable_length = yes;
        self
    }

    pub fn open(&self, path: &Path) -> Result<BtrieveFile, BtrieveError> {
        let io = |source| BtrieveError::Io {
            file: path.to_path_buf(),
            source,
        };
        let file = File::open(path).map_err(io)?;
        let file_length = file.metadata().map_err(io)?.len();
        let source = BufReader::with_capacity(READ_BUFFER, file);
        BtrieveFile::load(path.to_path_buf(), Box::new(source), file_length, self)
    }

    /// Opens an in-memory image; `name` is used in errors.
    pub fn open_bytes(
        &self,
        name: impl Into<PathBuf>,
        bytes: Vec<u8>,
    ) -> Result<BtrieveFile, BtrieveError> {
        let len = bytes.len() as u64;
        BtrieveFile::load(name.into(), Box::new(Cursor::new(bytes)), len, self)
    }
}

/// An open Btrieve file: its header, keys and collating sequences, ready to enumerate.
pub struct BtrieveFile {
    pub header: Header,
    pub keys: Vec<Key>,
    /// Every distinct ACS the file uses: the file's default first, then per-key ones.
    pub acs: Vec<Acs>,
    path: PathBuf,
    reader: PageReader,
    pat: Option<Pat>,
    deleted: HashSet<u64>,
}

impl std::fmt::Debug for BtrieveFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BtrieveFile")
            .field("path", &self.path)
            .field("header", &self.header)
            .field("keys", &self.keys)
            .field("acs", &self.acs)
            .finish_non_exhaustive()
    }
}

/// One live record. `data` is the payload: after the v6 usage count, `record_length`
/// bytes for a fixed-length file, the fixed part plus its fragments for a variable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    /// 1-based position in enumeration order (see the module docs).
    pub ordinal: u32,
    /// Logical page the record sits on.
    pub page: u32,
    /// Slot index on that page.
    pub slot: u16,
    pub data: &'a [u8],
}

impl BtrieveFile {
    /// Opens a fixed-length Btrieve file; variable-length files are refused.
    pub fn open(path: &Path) -> Result<Self, BtrieveError> {
        OpenOptions::new().open(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Checks the header's record length against the caller's record layout: equal for a
    /// fixed-length file; for a variable-length file `len` is the largest record (the
    /// `opnbtv` buffer) and the fixed part must fit in it.
    pub fn expect_record_len(&self, len: usize) -> Result<(), BtrieveError> {
        let found = usize::from(self.header.record_length);
        let fits = if self.header.is_variable_length() {
            found <= len
        } else {
            found == len
        };
        if fits {
            Ok(())
        } else {
            Err(BtrieveError::RecordLengthMismatch {
                file: self.path.clone(),
                expected: len,
                found,
            })
        }
    }

    /// A cursor over the live records, in the order described in the module docs. It
    /// reads one page at a time; call [`Records::next_record`] until it returns `None`.
    pub fn records(&mut self) -> Records<'_> {
        let page_length = usize::from(self.header.page_length);
        Records {
            page: vec![0; page_length],
            scratch: Vec::new(),
            out: Vec::new(),
            next_page: match self.header.version {
                Version::V5 => 0,
                Version::V6 => 1,
            },
            current_page: 0,
            page_offset: 0,
            slot: None,
            loaded: 0,
            done: self.header.record_count == 0,
            file: self,
        }
    }

    fn load(
        path: PathBuf,
        source: Box<dyn Source>,
        file_length: u64,
        options: &OpenOptions,
    ) -> Result<Self, BtrieveError> {
        let mut reader = PageReader::new(source);
        let header =
            Self::read_header(&mut reader, file_length, options).map_err(|e| e.at(&path))?;
        let mut file = Self {
            pat: (header.version == Version::V6).then(|| Pat::new(header.page_length)),
            header,
            keys: Vec::new(),
            acs: Vec::new(),
            path,
            reader,
            deleted: HashSet::new(),
        };
        file.read_tables().map_err(|e| e.at(&file.path))?;
        tracing::debug!(
            file = %file.path.display(),
            version = ?file.header.version,
            page_length = file.header.page_length,
            record_length = file.header.record_length,
            records = file.header.record_count,
            keys = file.keys.len(),
            "opened Btrieve file"
        );
        Ok(file)
    }

    /// The version sniff and active FCR (`validateDatabase`, BtrieveDatabase.cc:79).
    fn read_header(
        reader: &mut PageReader,
        file_length: u64,
        options: &OpenOptions,
    ) -> Result<Header, ReadError> {
        if file_length < MIN_PAGE_LENGTH as u64 {
            return Err(Fault::not_btrieve("shorter than one 512-byte page").into());
        }
        let mut first = [0u8; MIN_PAGE_LENGTH];
        reader.read_at(0, &mut first)?;
        let version = header::sniff(&first)?;
        let page_length = header::page_length(&first)?;
        let len = u64::from(page_length);
        let need_pages = match version {
            Version::V5 => 1,
            Version::V6 => 4, // two FCR copies and the first PAT pair
        };
        if file_length < need_pages * len {
            return Err(Fault::not_btrieve(format!(
                "{file_length} bytes is too short for {need_pages} pages of {page_length}"
            ))
            .into());
        }

        let mut fcr = vec![0u8; usize::from(page_length)];
        let fcr_offset = match version {
            Version::V5 => {
                reader.read_at(0, &mut fcr)?;
                0
            }
            Version::V6 => {
                reader.read_at(len, &mut fcr)?;
                if header::fcr_usage_count(&first) > header::fcr_usage_count(&fcr) {
                    reader.read_at(0, &mut fcr)?;
                    0
                } else {
                    len
                }
            }
        };
        let header = header::parse_fcr(&fcr, version, fcr_offset, file_length)?;
        if header.is_variable_length() {
            if !options.variable_length {
                return Err(Fault::VariableLength.into());
            }
            if header.record_type == RecordType::UsesVat {
                return Err(
                    Fault::not_btrieve("variable-length VAT files are not supported").into(),
                );
            }
        }
        Ok(header)
    }

    /// Default ACS, deleted chain and key definitions.
    fn read_tables(&mut self) -> Result<(), ReadError> {
        let page_length = u64::from(self.header.page_length);
        let default_acs = match self.header.version {
            Version::V6 => {
                // loadPAT: every 'A' page in the first PAT pair; the last one wins.
                let mut pair = vec![0u8; 2 * usize::from(self.header.page_length)];
                self.reader.read_at(2 * page_length, &mut pair)?;
                let mut acs = None;
                for page in pat::scan_first_pat(&pair, usize::from(self.header.page_length))? {
                    acs = self.acs_at_physical(u64::from(page) * page_length)?;
                }
                acs
            }
            Version::V5 => {
                self.deleted = pages::deleted_chain(&mut self.reader, &self.header)?;
                // The v5 ACS is always logical page 1, when the file has one.
                if 2 * page_length <= self.header.file_length {
                    self.acs_at_physical(page_length)?
                } else {
                    None
                }
            }
        };
        if let Some(acs) = &default_acs {
            self.acs.push(acs.clone());
        }
        self.read_keys(default_acs.as_ref())
    }

    fn acs_at_physical(&mut self, at: u64) -> Result<Option<Acs>, ReadError> {
        let mut page = vec![0u8; usize::from(self.header.page_length)];
        self.reader.read_at(at, &mut page)?;
        Ok(acs::parse_acs_page(&page, self.header.version)?)
    }

    /// `loadKeyDefinitions` (BtrieveDatabase.cc:468). A segment with `SegmentedKey` set is
    /// followed directly by the next segment of the same key.
    fn read_keys(&mut self, default_acs: Option<&Acs>) -> Result<(), ReadError> {
        let h = &self.header;
        let key_count = usize::from(h.key_count);
        let fcr = h.fcr_offset;
        let starts: Vec<u64> = match h.version {
            Version::V6 => {
                let mut kat = vec![0u8; key_count * 2];
                self.reader.read_at(h.kat_offset, &mut kat)?;
                kat.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&w| fcr + u64::from(u16::from_le_bytes(w)))
                    .collect()
            }
            // wbtrv32 starts key i at 0x110 + i * 0x1E, which lands inside the previous
            // key when it has segments; the definitions are consecutive, so v5 just reads
            // on from the previous segment.
            Version::V5 => vec![fcr + V5_KEY_DEF_BASE],
        };

        let mut keys: Vec<Key> = Vec::with_capacity(key_count);
        let mut current: Option<Key> = None;
        let mut at = starts.first().copied().unwrap_or(0);
        let mut def = [0u8; KEY_DEF_LENGTH];
        for _ in 0..=MAX_SEGMENTS {
            if keys.len() == key_count {
                self.keys = keys;
                return Ok(());
            }
            let number = keys.len() as u16;
            self.reader.read_at(at, &mut def)?;
            let raw =
                keys::parse_key_def(&def, number, self.header.version, self.header.record_length)?;
            let acs = if raw.attributes.has(Attr::MULTIPLE_ACS) {
                Some(self.key_acs(number, raw.acs_page)?)
            } else if raw.attributes.has(Attr::NUMBERED_ACS) {
                Some(default_acs.cloned().ok_or_else(|| {
                    Fault::InvalidAcs(format!(
                        "key {number} requires an ACS but the file has none"
                    ))
                })?)
            } else {
                None
            };
            if let Some(acs) = &acs
                && !self.acs.contains(acs)
            {
                self.acs.push(acs.clone());
            }
            let continues = raw.attributes.has(Attr::SEGMENTED);
            let key = current.get_or_insert_with(|| Key {
                number,
                segments: Vec::new(),
            });
            let segment = key.segments.len() as u16;
            key.segments
                .push(keys::key_def(raw, number, segment, acs.as_ref()));
            at += KEY_DEF_LENGTH as u64;
            if !continues {
                keys.extend(current.take());
                if let Some(&next) = starts.get(keys.len()) {
                    at = next;
                }
            }
        }
        Err(Fault::not_btrieve("key definitions do not terminate").into())
    }

    /// The ACS of a MultipleACS key, on its own logical page (`loadACS`, :413).
    fn key_acs(&mut self, key: u16, logical: u32) -> Result<Acs, ReadError> {
        let at = self.map_page(logical)?.ok_or_else(|| {
            Fault::InvalidAcs(format!("key {key}: ACS page {logical} is not mapped"))
        })?;
        self.acs_at_physical(at)?
            .ok_or_else(|| Fault::InvalidAcs(format!("key {key}: no ACS on page {logical}")).into())
    }

    /// `logicalPageToPhysicalOffset` (:666).
    fn map_page(&mut self, logical: u32) -> Result<Option<u64>, ReadError> {
        Ok(self.lookup(logical)?.offset)
    }

    fn lookup(&mut self, logical: u32) -> Result<PatEntry, ReadError> {
        match &mut self.pat {
            None => Ok(PatEntry {
                kind: pat::kind::DATA,
                offset: Some(u64::from(logical) * u64::from(self.header.page_length)),
            }),
            Some(pat) => pat.lookup(&mut self.reader, &self.header, logical),
        }
    }
}

/// A cursor over a file's live records (`loadRecords`, BtrieveDatabase.cc:229). The
/// record borrows the cursor's page buffer, so this is a lending cursor rather than an
/// [`Iterator`]: `while let Some(record) = records.next_record() { .. }`.
pub struct Records<'f> {
    file: &'f mut BtrieveFile,
    page: Vec<u8>,
    scratch: Vec<u8>,
    out: Vec<u8>,
    next_page: u32,
    current_page: u32,
    /// `logical page * page_length`, the base of v5 deleted-chain offsets.
    page_offset: u64,
    /// Next slot on the current page; `None` between pages.
    slot: Option<usize>,
    loaded: u32,
    done: bool,
}

enum Hit {
    Fixed { start: usize, slot: usize },
    Variable { slot: usize },
}

impl Records<'_> {
    /// Records yielded so far.
    pub fn loaded(&self) -> u32 {
        self.loaded
    }

    /// The next live record, `None` at the end, or the error that ended the walk.
    pub fn next_record(&mut self) -> Option<Result<Record<'_>, BtrieveError>> {
        if self.done {
            return None;
        }
        match self.advance() {
            Ok(Some(hit)) => {
                self.loaded += 1;
                let (data, slot) = match hit {
                    Hit::Fixed { start, slot } => (
                        &self.page[start..start + usize::from(self.file.header.record_length)],
                        slot,
                    ),
                    Hit::Variable { slot } => (&self.out[..], slot),
                };
                Some(Ok(Record {
                    ordinal: self.loaded,
                    page: self.current_page,
                    slot: slot as u16,
                    data,
                }))
            }
            Ok(None) => {
                self.finish();
                None
            }
            Err(err) => {
                self.done = true;
                Some(Err(err.at(&self.file.path)))
            }
        }
    }

    /// Counts the remaining records without looking at them.
    pub fn count_remaining(&mut self) -> Result<u32, BtrieveError> {
        let mut n = 0;
        while let Some(record) = self.next_record() {
            record?;
            n += 1;
        }
        Ok(n)
    }

    fn finish(&mut self) {
        self.done = true;
        let h = &self.file.header;
        if self.loaded != h.record_count {
            tracing::warn!(
                file = %self.file.path.display(),
                header = h.record_count,
                read = self.loaded,
                "record count differs from the header"
            );
        } else {
            tracing::debug!(file = %self.file.path.display(), records = self.loaded, "enumerated");
        }
    }

    fn advance(&mut self) -> Result<Option<Hit>, ReadError> {
        let h = self.file.header.clone();
        let page_length = u64::from(h.page_length);
        let physical = usize::from(h.physical_record_length);
        let per_page = h.records_per_page();
        // wbtrv32 walks to the FCR page count; past the end of the file it would fail a
        // read, so the walk is also capped at the pages the file can hold.
        let last_page = h.page_count.min(h.file_length.div_ceil(page_length) as u32);
        loop {
            let Some(slot) = self.slot else {
                if self.next_page >= last_page {
                    return Ok(None);
                }
                let logical = self.next_page;
                self.next_page += 1;
                let entry = self.file.lookup(logical)?;
                if h.version == Version::V6
                    && entry.kind != pat::kind::DATA
                    && entry.kind != pat::kind::VARIABLE
                {
                    continue;
                }
                let Some(offset) = entry.offset else { continue };
                self.file.reader.read_at(offset, &mut self.page)?;
                if !PageHeader::parse(&self.page)?.is_data() {
                    continue;
                }
                self.current_page = logical;
                self.page_offset = u64::from(logical) * page_length;
                self.slot = Some(0);
                continue;
            };

            if slot >= per_page {
                self.slot = None;
                if self.loaded == h.record_count {
                    return Ok(None);
                }
                continue;
            }
            self.slot = Some(slot + 1);
            let at = pages::PAGE_HEADER_LENGTH + slot * physical;
            if self.file.deleted.contains(&(self.page_offset + at as u64)) {
                continue;
            }
            let logical = &self.page[at..at + usize::from(h.record_length)];
            match pages::classify_slot(h.version, logical, h.file_length) {
                Slot::Live => {}
                Slot::Deleted => continue,
                Slot::Unused => {
                    self.slot = Some(per_page);
                    continue;
                }
            }
            let start = at + h.slot_prefix();
            if !h.is_variable_length() {
                return Ok(Some(Hit::Fixed { start, slot }));
            }
            let fixed_end = start + usize::from(h.record_length);
            self.out.clear();
            self.out.extend_from_slice(&self.page[start..fixed_end]);
            let tail = self.page[fixed_end..at + physical].to_vec();
            self.scratch.resize(usize::from(h.page_length), 0);
            let file = &mut *self.file;
            let pat = &mut file.pat;
            pages::read_variable_tail(
                &mut file.reader,
                &h,
                |reader, page| match pat {
                    None => Ok(Some(u64::from(page) * page_length)),
                    Some(pat) => Ok(pat.lookup(reader, &h, page)?.offset),
                },
                &tail,
                &mut self.scratch,
                &mut self.out,
            )?;
            return Ok(Some(Hit::Variable { slot }));
        }
    }
}
