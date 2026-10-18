//! Alternate collating sequence (ACS) pages.
//!
//! Port of wbtrv32 `loadACSAtPhysicalOffset` (BtrieveDatabase.cc:425). The table is only
//! exposed: the importer uses its presence to know which keys compare case-insensitively,
//! and the database's `COLLATE NOCASE` replaces the collation itself.

use super::error::Fault;
use super::header::Version;

/// Bytes in a collating table: one weight per input byte.
pub const ACS_LENGTH: usize = 256;

/// The v5 ACS page signature.
const V5_ACS_HEADER: [u8; 7] = [0, 0, 1, 0, 0, 0, 0xAC];

/// One collating sequence: its 8-character name and the byte-to-weight table.
#[derive(Clone, PartialEq, Eq)]
pub struct Acs {
    pub name: String,
    pub table: [u8; ACS_LENGTH],
}

impl Acs {
    /// The weight of `byte` under this sequence.
    pub fn weight(&self, byte: u8) -> u8 {
        self.table[usize::from(byte)]
    }

    /// True when every ASCII lower-case letter weighs the same as its upper-case form,
    /// which is what makes a key case-insensitive.
    pub fn folds_case(&self) -> bool {
        (b'a'..=b'z').all(|c| self.weight(c) == self.weight(c.to_ascii_uppercase()))
    }
}

impl std::fmt::Debug for Acs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Acs")
            .field("name", &self.name)
            .field("folds_case", &self.folds_case())
            .finish_non_exhaustive()
    }
}

/// Decodes an ACS page. A v5 page without the signature is "no ACS" (`Ok(None)`), as in
/// wbtrv32; a v6 page must have `'A'` at byte 1 or 0xAC at byte 6 (wbtrv32 tests the two
/// with `&&`, so either one passes).
pub(crate) fn parse_acs_page(page: &[u8], version: Version) -> Result<Option<Acs>, Fault> {
    if page.len() < 0x0F + ACS_LENGTH {
        return Err(Fault::InvalidAcs("ACS page shorter than its table".into()));
    }
    match version {
        Version::V6 => {
            if page[1] != b'A' && page[6] != 0xAC {
                return Err(Fault::InvalidAcs("bad v6 ACS page header".into()));
            }
        }
        Version::V5 => {
            if page[..V5_ACS_HEADER.len()] != V5_ACS_HEADER {
                return Ok(None);
            }
        }
    }
    // The name is a C string in 9 bytes at +7 with trailing blanks trimmed.
    let raw = &page[7..16];
    let raw = &raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())];
    let name: String = raw.iter().map(|&b| char::from(b)).collect();
    let mut table = [0u8; ACS_LENGTH];
    table.copy_from_slice(&page[0x0F..0x0F + ACS_LENGTH]);
    Ok(Some(Acs {
        name: name.trim_end_matches(' ').to_owned(),
        table,
    }))
}
