//! Run-time session structures: they become engine types, not tables.
//!
//! Linked lists become owned values (the queued command and forget lists become `Vec`s); the channel
//! numbers and `unk_*` members are not carried.

use serde::{Deserialize, Serialize};

use crate::enums::{DebugLevel, EscState, InGame};
use crate::flags::SessionFlags;
use crate::ids::ClassNo;

/// A queued command line. The original queues at most 40 per session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCommand {
    /// +0x000 command text (at most 254 bytes).
    pub text: String,
}

/// A forget-list entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetEntry {
    /// +0x00 forgotten first name.
    pub name: String,
}

/// Per-session state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserState {
    /// +0x00 session flags.
    pub flags: SessionFlags,
    /// +0x04 paged list: limit/step.
    pub listmax: i16,
    /// +0x06 paged list: position.
    pub listidx: i16,
    /// +0x08 TOPTEN class filter, 0 = all.
    pub topclass: i16,
    /// +0x0A rank of the next filtered line.
    pub toprank: i16,
    /// +0x0C saved list position.
    pub listkey: i32,
    /// +0x10 user report: name filter.
    pub rptname: String,
    /// +0x24 first name of the inviting user.
    pub inviter: String,
    /// +0x44 character-mode input line (text and length).
    pub inpbuf: String,
    /// +0x48 input echo column.
    pub echocol: i16,
    /// +0x4A custom status line template (kept across deaths in [`crate::records::PermanentRecord`]).
    pub statline: String,
    /// +0x87 escape sequence parser state.
    pub escstate: EscState,
    /// +0x8C queued command lines.
    pub pending: Vec<PendingCommand>,
    /// +0x90 forget list.
    pub forgets: Vec<ForgetEntry>,
    /// +0x94 copy of the last input.
    pub lastinput: String,
    /// +0x98 kept in permrec, same day only.
    pub daybyte: u8,
}

impl Default for UserState {
    fn default() -> Self {
        Self {
            flags: SessionFlags::empty(),
            listmax: 0,
            listidx: 0,
            topclass: 0,
            toprank: 0,
            listkey: 0,
            rptname: String::new(),
            inviter: String::new(),
            inpbuf: String::new(),
            echocol: 0,
            statline: String::new(),
            escstate: EscState::None,
            pending: Vec::new(),
            forgets: Vec::new(),
            lastinput: String::new(),
            daybyte: 0,
        }
    }
}

/// Per-session flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFlags {
    /// +0x00 sysop debug level.
    pub debug: DebugLevel,
    /// +0x01 has the sysop key.
    pub sysop: bool,
    /// +0x02 has the game operator key.
    pub gameop: bool,
    /// +0x03 may gossip (not sysop/gameop).
    pub hasgoss: bool,
    /// +0x04 may auction (not sysop/gameop).
    pub hasauct: bool,
    /// +0x05 user list rank, or [`crate::enums::uls`] `NOTPLAYING`/`UNRANKED`.
    pub listslot: i16,
    /// +0x07 progress into the game.
    pub ingame: InGame,
}

/// A top-list entry; the list holds `NTOPLIST`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopEntry {
    /// +0x00 user-id.
    pub userid: String,
    /// +0x1E first name.
    pub name: String,
    /// +0x29 last name.
    pub lastname: String,
    /// +0x3C class number, 0 = empty.
    pub classnum: ClassNo,
    /// +0x3E gang name.
    pub gang: String,
    /// +0x52 experience, billions.
    pub exphi: u32,
    /// +0x56 experience, remainder.
    pub explo: u32,
    /// +0x5A "EDITED" or "".
    pub edited: String,
    /// +0x61 WCCUSERS records (entry 0).
    pub nrecs: i16,
}

/// Experience split at a billion, printed as the billions then nine digits of remainder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BigExp {
    /// +0x00 billions.
    pub expbil: u32,
    /// +0x04 remainder below a billion.
    pub exprem: u32,
}
