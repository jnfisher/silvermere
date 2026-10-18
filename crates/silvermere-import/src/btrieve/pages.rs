//! Page I/O, data-page and slot decoding, the v5 deleted chain and variable-length
//! fragments.
//!
//! Ports of wbtrv32 `getRecordPointer`/`getRecordPointerList` (BtrieveDatabase.cc:42-63),
//! `isUnusedRecord` (:207), the slot loop of `loadRecords` (:229) and
//! `getVariableLengthData` (:599).

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom};

use super::error::{Fault, ReadError};
use super::header::{Header, RecordType, Version, u16_at};

/// Bytes of page header in front of the first record slot.
pub(crate) const PAGE_HEADER_LENGTH: usize = 6;

/// The empty-chain marker of record pointers.
pub(crate) const NO_RECORD: u32 = 0xFFFF_FFFF;

/// Anything the reader can pull pages from: a file, or bytes in memory for tests.
pub(crate) trait Source: Read + Seek + Send {}
impl<T: Read + Seek + Send> Source for T {}

/// Positioned page reads over a buffered source. Sequential reads skip the seek, so a v5
/// page walk streams the file once.
pub(crate) struct PageReader {
    inner: Box<dyn Source>,
    pos: Option<u64>,
}

impl PageReader {
    pub(crate) fn new(inner: Box<dyn Source>) -> Self {
        Self { inner, pos: None }
    }

    pub(crate) fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if self.pos != Some(offset) {
            self.pos = None;
            self.inner.seek(SeekFrom::Start(offset))?;
        }
        self.pos = None;
        self.inner.read_exact(buf)?;
        self.pos = Some(offset + buf.len() as u64);
        Ok(())
    }
}

/// The 6-byte header of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageHeader {
    pub raw: [u8; PAGE_HEADER_LENGTH],
}

impl PageHeader {
    pub(crate) fn parse(page: &[u8]) -> Result<Self, Fault> {
        let raw = page
            .get(..PAGE_HEADER_LENGTH)
            .and_then(|b| <[u8; PAGE_HEADER_LENGTH]>::try_from(b).ok())
            .ok_or_else(|| Fault::not_btrieve("page shorter than its 6-byte header"))?;
        Ok(Self { raw })
    }

    /// A data page has the high bit of byte 5 (the usage count's top byte) set.
    pub(crate) fn is_data(&self) -> bool {
        self.raw[5] & 0x80 != 0
    }
}

/// A record pointer: a little-endian high word then a low word. `None` if short.
pub(crate) fn record_pointer(bytes: &[u8]) -> Option<u32> {
    let b = bytes.get(..4)?;
    Some(u32::from(u16_at(b, 0)) << 16 | u32::from(u16_at(b, 2)))
}

/// What a record slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Live,
    /// v6: usage count 0, a hole; later slots on the page may be live.
    Deleted,
    /// v5: a free-space link followed by zeros; the rest of the page is unused.
    Unused,
}

/// `isUnusedRecord`: v6 looks at the big-endian usage count; v5 treats a slot whose
/// bytes after the first four are all zero, and whose first four are a pointer inside the
/// file, as unused. `logical` is the slot's first `record_length` bytes.
pub(crate) fn classify_slot(version: Version, logical: &[u8], file_length: u64) -> Slot {
    match version {
        Version::V6 => match logical {
            [hi, lo, ..] if u16::from_be_bytes([*hi, *lo]) != 0 => Slot::Live,
            _ => Slot::Deleted,
        },
        Version::V5 => {
            let free_link = logical.len() >= 4
                && logical[4..].iter().all(|&b| b == 0)
                && record_pointer(logical).is_some_and(|p| u64::from(p) < file_length);
            if free_link { Slot::Unused } else { Slot::Live }
        }
    }
}

/// `getRecordPointerList`: the absolute offsets of every slot on the v5 deleted chain.
/// wbtrv32 follows the chain blindly; this stops on a loop or a pointer past the file.
pub(crate) fn deleted_chain(
    reader: &mut PageReader,
    header: &Header,
) -> Result<HashSet<u64>, ReadError> {
    let mut seen = HashSet::new();
    let mut next = header.deleted_record_pointer;
    while next != NO_RECORD {
        let at = u64::from(next);
        let page = (at / u64::from(header.page_length)) as u32;
        if at + 4 > header.file_length {
            return Err(Fault::corrupt(
                page,
                format!("deleted-record pointer {at:#x} past end of file"),
            )
            .into());
        }
        if !seen.insert(at) {
            return Err(Fault::corrupt(page, "deleted-record chain loops").into());
        }
        let mut link = [0u8; 4];
        reader.read_at(at, &mut link)?;
        next = record_pointer(&link).unwrap_or(NO_RECORD);
    }
    Ok(seen)
}

/// The variable-length tail pointer in a physical record: `[high][low][mid][fragment]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VrecPtr {
    pub page: u32,
    pub fragment: u8,
}

impl VrecPtr {
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        let b = bytes.get(..4)?;
        Some(Self {
            page: u32::from(b[0]) << 16 | u32::from(b[2]) << 8 | u32::from(b[1]),
            fragment: b[3],
        })
    }
}

/// One fragment located on a variable page: where its bytes are and the next pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fragment {
    pub start: usize,
    pub len: usize,
    pub next: Option<VrecPtr>,
}

/// Locates fragment `number` on a variable page. The page ends in an array of u16 offsets
/// counted back from the end; the next non-0xFFFF entry below gives the end. The high bit
/// (always, in v6) means the fragment opens with a pointer to the next one.
pub(crate) fn locate_fragment(
    page: &[u8],
    number: u8,
    version: Version,
    page_no: u32,
) -> Result<Fragment, Fault> {
    let bad =
        |reason: &str| Fault::corrupt(page_no, format!("variable fragment {number}: {reason}"));
    let words = page.len() / 2;
    let word = |i: usize| u16_at(page, i * 2);
    let index = ((page.len() - 1) >> 1)
        .checked_sub(usize::from(number))
        .filter(|&i| i < words)
        .ok_or_else(|| bad("index outside the page"))?;
    let entry = word(index);
    let start = usize::from(entry & 0x7FFF);
    let end = (1..=index)
        .map(|back| word(index - back))
        .find(|&w| w != 0xFFFF)
        .map(|w| usize::from(w & 0x7FFF))
        .ok_or_else(|| bad("invalid previous fragment marker"))?;
    if end < start || end > page.len() {
        return Err(bad("fragment bounds outside the page"));
    }
    if version == Version::V6 || entry & 0x8000 != 0 {
        if end - start < 4 {
            return Err(bad("fragment shorter than its next pointer"));
        }
        Ok(Fragment {
            start: start + 4,
            len: end - start - 4,
            next: VrecPtr::parse(&page[start..start + 4]),
        })
    } else {
        Ok(Fragment {
            start,
            len: end - start,
            next: None,
        })
    }
}

/// Upper bound on the fragments one record may chain through.
const MAX_FRAGMENTS: usize = 1 << 16;

/// `getVariableLengthData`: appends the fragment chain that starts at `tail` (the bytes
/// after the fixed part of the physical record) to `out`.
pub(crate) fn read_variable_tail(
    reader: &mut PageReader,
    header: &Header,
    mut map: impl FnMut(&mut PageReader, u32) -> Result<Option<u64>, ReadError>,
    tail: &[u8],
    scratch: &mut [u8],
    out: &mut Vec<u8>,
) -> Result<(), ReadError> {
    let mut ptr = VrecPtr::parse(tail);
    let truncated = match header.record_type {
        RecordType::VariableTruncated => tail.get(4..6).map_or(0, |b| u16_at(b, 0)),
        _ => 0,
    };
    let mut visited = HashSet::new();
    for _ in 0..MAX_FRAGMENTS {
        let Some(current) = ptr.filter(|p| p.fragment <= 254) else {
            out.resize(out.len() + usize::from(truncated), b' ');
            return Ok(());
        };
        let Some(offset) = map(reader, current.page)? else {
            out.resize(out.len() + usize::from(truncated), b' ');
            return Ok(());
        };
        if !visited.insert((current.page, current.fragment)) {
            return Err(Fault::corrupt(current.page, "variable fragment chain loops").into());
        }
        reader.read_at(offset, scratch)?;
        let fragment = locate_fragment(scratch, current.fragment, header.version, current.page)?;
        out.extend_from_slice(&scratch[fragment.start..fragment.start + fragment.len]);
        if out.len() as u64 > header.file_length {
            return Err(
                Fault::corrupt(current.page, "variable record longer than the file").into(),
            );
        }
        ptr = fragment.next;
    }
    Err(Fault::corrupt(0, "variable fragment chain does not end").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // BtrieveDatabase.cc:42 getRecordPointer: high word first, each word little-endian.
    #[test]
    fn record_pointer_is_high_word_then_low_word() {
        assert_eq!(record_pointer(&[0x01, 0x00, 0x34, 0x12]), Some(0x0001_1234));
        assert_eq!(record_pointer(&[0xFF; 4]), Some(NO_RECORD));
        assert_eq!(record_pointer(&[0; 3]), None);
    }

    // BtrieveDatabase.cc:207 isUnusedRecord
    #[test]
    fn slot_classification() {
        assert_eq!(
            classify_slot(Version::V6, &[0, 0, 9, 9], 100),
            Slot::Deleted
        );
        assert_eq!(classify_slot(Version::V6, &[0, 1, 0, 0], 100), Slot::Live);
        assert_eq!(
            classify_slot(Version::V5, &[0, 0, 0x10, 0, 0, 0], 100),
            Slot::Unused
        );
        // A link past the end of the file is data, not a free slot.
        assert_eq!(
            classify_slot(Version::V5, &[0, 0, 0x10, 0, 0, 0], 8),
            Slot::Live
        );
        assert_eq!(
            classify_slot(Version::V5, &[0, 0, 0x10, 0, 1, 0], 100),
            Slot::Live
        );
    }

    // BtrieveDatabase.cc:261 loadRecords: data page iff byte 5 has its high bit set.
    #[test]
    fn data_page_flag() {
        assert!(PageHeader::parse(&[0, 0, 0, 0, 0, 0x80]).unwrap().is_data());
        assert!(!PageHeader::parse(&[0, 0, 0, 0, 0, 0x7F]).unwrap().is_data());
        assert!(PageHeader::parse(&[0; 5]).is_err());
    }

    proptest! {
        #[test]
        fn page_header_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..16)) {
            let parsed = PageHeader::parse(&bytes);
            prop_assert_eq!(parsed.is_ok(), bytes.len() >= PAGE_HEADER_LENGTH);
        }

        #[test]
        fn record_pointer_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..8)) {
            prop_assert_eq!(record_pointer(&bytes).is_some(), bytes.len() >= 4);
            let _ = VrecPtr::parse(&bytes);
            let _ = classify_slot(Version::V5, &bytes, 1 << 20);
            let _ = classify_slot(Version::V6, &bytes, 1 << 20);
        }

        #[test]
        fn fragment_locator_never_panics(
            page in proptest::collection::vec(any::<u8>(), 512..=512),
            number in any::<u8>(),
            v6 in any::<bool>(),
        ) {
            let version = if v6 { Version::V6 } else { Version::V5 };
            if let Ok(f) = locate_fragment(&page, number, version, 1) {
                prop_assert!(f.start + f.len <= page.len());
            }
        }
    }
}
