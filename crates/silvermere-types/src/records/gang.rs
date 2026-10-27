//! The `gang` record (WCCGANGS).

use serde::{Deserialize, Serialize};

use crate::flags::GangFlags;
use crate::records::layout::{self, Layout};

/// A gang: the `gang` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gang {
    /// +0x000 name in upper case (key 0).
    pub key: String,
    /// +0x014 name as entered.
    pub name: String,
    /// +0x028 gang experience (key 1).
    pub exp: u32,
    /// +0x02C leader's character name.
    pub leader: String,
    /// +0x04A date founded (a DOS date in the record), Unix seconds UTC.
    pub created: i64,
    /// +0x04E number of members.
    pub members: i16,
    /// +0x050 gang flags.
    pub flags: GangFlags,
    /// +0x055 experience past the cap.
    pub overexp: u32,
    /// +0x059 extra wraps of `overexp`.
    pub overflows: i16,
}

impl Gang {
    /// Byte layout of the `gang` record.
    pub const LAYOUT: &'static Layout = &layout::GANG;
    /// The `gang` record length: 253 bytes.
    pub const RECORD_LEN: usize = 253;
}
