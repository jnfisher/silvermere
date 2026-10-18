//! What flows between a session task and the engine.

use std::io;

use crate::line_editor::EchoMode;

/// Identifies one connection.
///
/// Placeholder: `silvermere-types` (T-002) defines the shared `SessionId`; once it lands this
/// becomes a re-export of that type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u32);

/// What the client told us about its terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Screen width (`usracc.scnwid`).
    pub columns: u16,
    /// Screen height (`usracc.scnbrk`).
    pub rows: u16,
}

impl Default for Capabilities {
    /// 80x24 until (unless) the client sends NAWS.
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
        }
    }
}

/// Why a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The client closed or dropped the connection.
    Eof,
    /// The client stopped reading and the output queue passed its bound.
    Backpressure,
    /// The engine asked for the close (`SessionCommand::Close`, or it dropped its sender).
    Closed,
    /// Any other socket error.
    Io(io::ErrorKind),
}

/// Session task -> engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// The connection is up; sent before negotiation completes so the login prompt never
    /// waits on it.
    Connected(Capabilities),
    /// One edited input line: no terminator, no telnet commands, CP437 bytes (decode with
    /// [`crate::cp437::decode`]).
    Line(Vec<u8>),
    /// The line passed the 254-byte limit and was discarded (GSBL status 251).
    LineTooLong,
    /// One keystroke in character mode (`SessionCommand::SetCharMode(true)`).
    Key(u8),
    /// The client reported its window size (columns, rows).
    Naws(u16, u16),
    /// The session is gone; no further events follow for this id.
    Disconnect(Reason),
}

/// Engine -> session task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    /// Output bytes, already CP437 with `\r\n` line ends; the transport doubles 0xFF.
    Write(Vec<u8>),
    /// Change how typed bytes are echoed (`echon`, `echsec`).
    SetEcho(EchoMode),
    /// Switch between line editing and per-key delivery.
    SetCharMode(bool),
    /// The engine is running the last `Line` now: the CR echo held back for it goes out
    /// ahead of its output ([`crate::SessionConfig::hold_line_echo`]).
    LineStarted,
    /// The engine has run the last `Line`: input typed behind it may be taken now (until
    /// then, or a short timeout, it waits unechoed; see `session_task::LINE_HOLD`).
    LineDone,
    /// Flush queued output, then close.
    Close,
}
