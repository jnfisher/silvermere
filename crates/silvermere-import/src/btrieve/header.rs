//! The File Control Record (FCR): version sniff, active-copy choice and header fields.
//!
//! Port of wbtrv32 `BtrieveDatabase::validateDatabase` (BtrieveDatabase.cc:79). All
//! multi-byte fields are little-endian; 32-bit counts are stored as a high word then a low
//! word.

use super::error::Fault;

/// The smallest legal page, and the size of the first read used to sniff the version.
pub const MIN_PAGE_LENGTH: usize = 512;

/// The FCR must hold every field up to `usrflgs` at 0x106.
const FCR_MIN_LEN: usize = 0x108;

/// `usrflgs` bits (FCR +0x106).
pub mod usrflgs {
    pub const VARIABLE: u16 = 0x0001;
    pub const TRUNCATE_BLANKS: u16 = 0x0002;
    pub const COMPRESSED: u16 = 0x0008;
    pub const VAT: u16 = 0x0800;
}

/// Btrieve file format generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// Btrieve 5.x: no magic, version code at +7, logical page n is physical page n.
    V5,
    /// Btrieve 6.x: `FC\0\0` magic, two FCR copies, a page allocation table, and a 2-byte
    /// big-endian usage count in front of every record.
    V6,
}

/// How records are stored, from `usrflgs` and the variable-record byte at +0x38.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordType {
    Fixed,
    Variable,
    /// Variable, with trailing blanks stripped on write and re-appended on read.
    VariableTruncated,
    /// v6 variable-allocation tables; wbtrv32 reads these as fixed records.
    UsesVat,
}

/// The decoded header of the active FCR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: Version,
    /// Bytes per page (FCR +0x08); a multiple of 512. Never assumed, always read.
    pub page_length: u16,
    /// Pages in the file according to the FCR (+0x26 high, +0x28 low).
    pub page_count: u32,
    /// Logical record length (FCR +0x16): the fixed part a caller reads, without the v6
    /// usage count and without the variable-length pointer.
    pub record_length: u16,
    /// Bytes per record slot on a data page (FCR +0x18), overhead included.
    pub physical_record_length: u16,
    /// Live records according to the FCR (+0x1A high, +0x1C low).
    pub record_count: u32,
    /// Number of keys (not segments), FCR +0x14.
    pub key_count: u16,
    pub record_type: RecordType,
    /// Raw `usrflgs` word (FCR +0x106).
    pub usrflgs: u16,
    /// Byte length of the file on disk.
    pub file_length: u64,
    /// Byte offset of the active FCR copy (0, or one page in for the v6 second copy).
    pub fcr_offset: u64,
    /// Absolute byte offset of the v6 key attribute table (FCR +0x78 plus `fcr_offset`).
    pub(crate) kat_offset: u64,
    /// Head of the v5 deleted-record chain (FCR +0x10), `0xFFFF_FFFF` when empty.
    pub(crate) deleted_record_pointer: u32,
}

impl Header {
    /// True when records carry a variable-length tail (`usrflgs` bit 0 or a variable type).
    pub fn is_variable_length(&self) -> bool {
        matches!(
            self.record_type,
            RecordType::Variable | RecordType::VariableTruncated
        ) || self.usrflgs & usrflgs::VARIABLE != 0
    }

    /// Record slots per data page: `(page_length - 6) / physical_record_length`.
    pub fn records_per_page(&self) -> usize {
        (usize::from(self.page_length) - 6) / usize::from(self.physical_record_length)
    }

    /// Bytes in front of the payload in each slot (the v6 usage count).
    pub(crate) fn slot_prefix(&self) -> usize {
        match self.version {
            Version::V5 => 0,
            Version::V6 => 2,
        }
    }
}

pub(crate) fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

pub(crate) fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// A 32-bit count stored as a little-endian high word followed by a low word.
pub(crate) fn hi_lo_at(buf: &[u8], at: usize) -> u32 {
    u32::from(u16_at(buf, at)) << 16 | u32::from(u16_at(buf, at + 2))
}

/// Tells v6 from v5 on the first page: `FC\0\0` is v6; otherwise the version code at +7
/// must be 3, 4 or 5 (wbtrv32 computes `fcr[6] << 16 | fcr[7]` into a u16, which keeps
/// only byte 7). Anything else is not a Btrieve file.
pub(crate) fn sniff(first: &[u8]) -> Result<Version, Fault> {
    if first.len() < MIN_PAGE_LENGTH {
        return Err(Fault::not_btrieve("shorter than one 512-byte page"));
    }
    if first[..4] == *b"FC\0\0" {
        return Ok(Version::V6);
    }
    match first[7] {
        3..=5 => Ok(Version::V5),
        code => Err(Fault::not_btrieve(format!(
            "no v6 magic and v5 version code {code} is not 3, 4 or 5"
        ))),
    }
}

/// Page length at +0x08: at least 512 and a multiple of 512.
pub(crate) fn page_length(first: &[u8]) -> Result<u16, Fault> {
    if first.len() < 0x0A {
        return Err(Fault::not_btrieve("header too short"));
    }
    let page_length = u16_at(first, 0x08);
    if page_length < 512 || page_length & 0x1FF != 0 {
        return Err(Fault::not_btrieve(format!(
            "page length {page_length} is not a multiple of 512"
        )));
    }
    Ok(page_length)
}

/// The usage count of a v6 FCR copy (u32 at +4); the higher one is active.
pub(crate) fn fcr_usage_count(page: &[u8]) -> u32 {
    u32_at(page, 4)
}

/// Decodes and validates the active FCR. `fcr` is that copy's page.
pub(crate) fn parse_fcr(
    fcr: &[u8],
    version: Version,
    fcr_offset: u64,
    file_length: u64,
) -> Result<Header, Fault> {
    if fcr.len() < FCR_MIN_LEN {
        return Err(Fault::not_btrieve("FCR shorter than 0x108 bytes"));
    }
    let page_length = page_length(fcr)?;

    if version == Version::V5 {
        // BtrieveDatabase.cc:136: 0xFFFF at 0x22 means "inconsistent, needs recovery".
        if fcr[0x22] == 0xFF && fcr[0x23] == 0xFF {
            return Err(Fault::NeedsRecovery);
        }
    }

    let accel_flags = u16_at(fcr, 0x0A);
    if accel_flags != 0 {
        return Err(Fault::not_btrieve(format!(
            "accel flags {accel_flags:#06x}, expected 0"
        )));
    }

    let flags = u16_at(fcr, 0x106);
    if flags & usrflgs::COMPRESSED != 0 {
        return Err(Fault::Compressed);
    }

    let variable_byte = fcr[0x38];
    let record_type = if version == Version::V6 && flags & usrflgs::VAT != 0 {
        RecordType::UsesVat
    } else if variable_byte != 0 || flags & usrflgs::VARIABLE != 0 {
        if variable_byte == 0xFD || flags & usrflgs::TRUNCATE_BLANKS != 0 {
            RecordType::VariableTruncated
        } else {
            RecordType::Variable
        }
    } else {
        RecordType::Fixed
    };

    if version == Version::V6 && fcr[0x76] != fcr[0x14] {
        return Err(Fault::not_btrieve("key count and KAT key count differ"));
    }

    let header = Header {
        version,
        page_length,
        page_count: hi_lo_at(fcr, 0x26),
        record_length: u16_at(fcr, 0x16),
        physical_record_length: u16_at(fcr, 0x18),
        record_count: hi_lo_at(fcr, 0x1A),
        key_count: u16_at(fcr, 0x14),
        record_type,
        usrflgs: flags,
        file_length,
        fcr_offset,
        kat_offset: fcr_offset + u64::from(u16_at(fcr, 0x78)),
        deleted_record_pointer: hi_lo_at(fcr, 0x10),
    };
    check_record_geometry(&header)?;
    Ok(header)
}

/// Not in wbtrv32, which trusts these fields: the slot must fit on a page and hold the
/// payload plus its overhead, or slicing a record would run off the page.
fn check_record_geometry(h: &Header) -> Result<(), Fault> {
    let physical = usize::from(h.physical_record_length);
    if physical == 0 || physical > usize::from(h.page_length) - 6 {
        return Err(Fault::not_btrieve(format!(
            "physical record length {physical} does not fit a {}-byte page",
            h.page_length
        )));
    }
    let mut needed = usize::from(h.record_length) + h.slot_prefix();
    if h.is_variable_length() {
        needed += 4; // VRECPTR
        if h.record_type == RecordType::VariableTruncated {
            needed += 2; // count of stripped blanks
        }
    }
    if h.record_length == 0 || needed > physical {
        return Err(Fault::not_btrieve(format!(
            "record length {} does not fit physical record length {physical}",
            h.record_length
        )));
    }
    Ok(())
}
