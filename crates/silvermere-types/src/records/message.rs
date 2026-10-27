//! The `mudmsg` record (WCCMSG).

use serde::{Deserialize, Serialize};

use crate::ids::MessageNo;
use crate::records::layout::{self, Layout};

/// A three-part message: the `mudmsg` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// +0x000 message number (key 0).
    pub number: MessageNo,
    /// +0x004 texts to the actor, the target and the room.
    pub msgs: [String; 3],
}

impl Message {
    /// Byte layout of the `mudmsg` record.
    pub const LAYOUT: &'static Layout = &layout::MUDMSG;
    /// The `mudmsg` record length: 253 bytes.
    pub const RECORD_LEN: usize = 253;
}
