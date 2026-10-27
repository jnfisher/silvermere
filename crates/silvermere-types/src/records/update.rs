//! The `updrec` record (WCCUPDAT).

use serde::{Deserialize, Serialize};

use crate::enums::UpdateRecordType;
use crate::records::layout::{self, Layout};

/// A database update record: the `updrec` record. `data` is the raw image of
/// the record that follows, decoded by T-009 according to `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRecord {
    /// +0x000 sequence number (key 0).
    pub seqnum: i32,
    /// +0x004 type of the record that follows (`type`).
    pub kind: UpdateRecordType,
    /// +0x005 image of that record (2022 bytes).
    pub data: Vec<u8>,
}

impl UpdateRecord {
    /// Byte layout of the `updrec` record.
    pub const LAYOUT: &'static Layout = &layout::UPDREC;
    /// The `updrec` record length: 2027 bytes.
    pub const RECORD_LEN: usize = 2027;
}
