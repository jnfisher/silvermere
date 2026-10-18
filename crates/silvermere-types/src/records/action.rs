//! The `action` record (WCCACTS).

use serde::Serialize;

use crate::flags::ActionTargets;
use crate::records::layout::{self, Layout};
use crate::sizes::NACTTEXTS;

/// A social action: the `action` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Action {
    /// +0x000 verb (key 0).
    pub name: String,
    /// +0x01E texts: 0/1 single, 2-4 user, 5/6 monster, 7/8 inventory, 9/10 floor.
    pub texts: [String; NACTTEXTS],
    /// +0x38E what the action can target.
    pub targets: ActionTargets,
}

impl Action {
    /// Byte layout of the `action` record.
    pub const LAYOUT: &'static Layout = &layout::ACTION;
    /// The `action` record length: 1010 bytes.
    pub const RECORD_LEN: usize = 1010;
}
