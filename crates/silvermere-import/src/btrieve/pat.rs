//! The v6 page allocation table (PAT): logical page to physical page.
//!
//! Port of wbtrv32 `loadPAT` (BtrieveDatabase.cc:367), `logicalPageToPhysicalOffset`
//! (:666) and `lookupPATEntry` (:687). PAT pages come in pairs that both start with `PP`;
//! the copy with the higher usage count at +4 is active. Each 4-byte entry is
//! `[page high][type][page low][page mid]`.

use super::error::{Fault, ReadError};
use super::header::{Header, u16_at, u32_at};
use super::pages::PageReader;

/// PAT entry type codes.
pub(crate) mod kind {
    pub const ACS: u8 = b'A';
    pub const DATA: u8 = b'D';
    pub const EXTRA: u8 = b'E';
    pub const VARIABLE: u8 = b'V';
    /// High bit: an index page.
    pub const INDEX_BIT: u8 = 0x80;
    /// Not a PAT code: the lookup ran off the table or the file.
    pub const NONE: u8 = 0xFF;
}

/// One PAT lookup result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PatEntry {
    pub kind: u8,
    /// Physical byte offset, `None` when unmapped.
    pub offset: Option<u64>,
}

impl PatEntry {
    const UNMAPPED: Self = Self {
        kind: kind::NONE,
        offset: None,
    };
}

/// Decodes a 4-byte PAT entry into (type, physical page).
pub(crate) fn decode_entry(entry: &[u8]) -> (u8, u32) {
    let page = u32::from(entry[0]) << 16 | u32::from(entry[3]) << 8 | u32::from(entry[2]);
    (entry[1], page)
}

fn has_pat_magic(page: &[u8]) -> bool {
    page[..2] == *b"PP"
}

/// The active copy of a PAT pair, by usage count (`lookupPATEntry` compares u32s).
fn active_copy(pair: &[u8], page_length: usize) -> &[u8] {
    let (first, second) = pair.split_at(page_length);
    if u32_at(first, 4) > u32_at(second, 4) {
        first
    } else {
        second
    }
}

/// Scans the first PAT pair (physical pages 2 and 3) as `loadPAT` does: both copies must
/// carry `PP`, the active one (u16 usage count) must hold only known type codes, and every
/// `'A'` entry names an ACS page. Returns the physical pages of the ACS entries in order.
pub(crate) fn scan_first_pat(pair: &[u8], page_length: usize) -> Result<Vec<u32>, Fault> {
    let (first, second) = pair.split_at(page_length);
    if !has_pat_magic(first) {
        return Err(Fault::not_btrieve("PAT1 table is invalid"));
    }
    if !has_pat_magic(second) {
        return Err(Fault::not_btrieve("PAT2 table is invalid"));
    }
    let active = if u16_at(first, 4) > u16_at(second, 4) {
        first
    } else {
        second
    };
    let mut acs_pages = Vec::new();
    for entry in active[8..].as_chunks::<4>().0 {
        let (code, page) = decode_entry(entry);
        if code & kind::INDEX_BIT != 0 {
            continue;
        }
        match code {
            kind::ACS => acs_pages.push(page),
            0 | kind::DATA | kind::EXTRA | kind::VARIABLE => {}
            other => {
                return Err(Fault::not_btrieve(format!(
                    "bad PAT entry type {other:#04x}"
                )));
            }
        }
    }
    Ok(acs_pages)
}

/// Logical-to-physical mapping with the most recently read PAT pair cached; a page walk
/// touches each pair many times in a row.
pub(crate) struct Pat {
    cached_at: Option<u64>,
    pair: Vec<u8>,
}

impl Pat {
    pub(crate) fn new(page_length: u16) -> Self {
        Self {
            cached_at: None,
            pair: vec![0; usize::from(page_length) * 2],
        }
    }

    /// `lookupPATEntry`: the type and physical offset of `logical`.
    pub(crate) fn lookup(
        &mut self,
        reader: &mut PageReader,
        header: &Header,
        logical: u32,
    ) -> Result<PatEntry, ReadError> {
        if logical >= header.page_count {
            return Ok(PatEntry::UNMAPPED);
        }
        let page_length = u64::from(header.page_length);
        let entries_per_page = u32::from(header.page_length) / 4;
        let pages_per_pat = entries_per_page - 2;

        // wbtrv32's paging: each further PAT pair is `page_length / 4` pages on.
        let mut logical = logical;
        let mut pat_page: u64 = 2;
        while logical > pages_per_pat {
            logical -= pages_per_pat;
            pat_page += u64::from(entries_per_page);
        }
        let at = pat_page * page_length;
        // `physicalOffset >= fileLength - pageLength * 2` is "overflowed, junk".
        if at + 2 * page_length >= header.file_length {
            return Ok(PatEntry::UNMAPPED);
        }
        if self.cached_at != Some(at) {
            self.cached_at = None;
            reader.read_at(at, &mut self.pair)?;
            self.cached_at = Some(at);
        }
        let len = usize::from(header.page_length);
        if !has_pat_magic(&self.pair[..len]) && !has_pat_magic(&self.pair[len..]) {
            return Ok(PatEntry::UNMAPPED);
        }
        let active = active_copy(&self.pair, len);
        let index = logical as usize * 4 + 4;
        let (code, page) = decode_entry(&active[index..index + 4]);
        let offset = (page != 0x00FF_FFFF).then(|| u64::from(page) * page_length);
        Ok(PatEntry { kind: code, offset })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // BtrieveDatabase.cc:742 lookupPATEntry: "high low mid" byte order.
    #[test]
    fn entry_bytes_are_high_type_low_mid() {
        assert_eq!(decode_entry(&[0x01, b'D', 0x03, 0x02]), (b'D', 0x01_02_03));
    }

    #[test]
    fn scan_rejects_missing_magic_and_unknown_codes() {
        let mut pair = vec![0u8; 1024];
        assert!(scan_first_pat(&pair, 512).is_err());
        pair[..2].copy_from_slice(b"PP");
        pair[512..514].copy_from_slice(b"PP");
        assert_eq!(scan_first_pat(&pair, 512), Ok(vec![]));
        pair[512 + 8..512 + 12].copy_from_slice(&[0, b'A', 5, 0]);
        assert_eq!(scan_first_pat(&pair, 512), Ok(vec![5]));
        pair[512 + 13] = b'Z';
        assert!(scan_first_pat(&pair, 512).is_err());
    }
}
