//! Transport trait, telnet transport, session line discipline, CP437.
//!
//! The network edge: the Transport trait, the hand-written telnet transport (IAC parsing and
//! option negotiation), the line editor, the CP437 codec and the per-connection session task
//! with its bounded output queue.

#![forbid(unsafe_code)]

pub mod cp437;
pub mod events;
pub mod line_editor;
pub mod session_task;
pub mod telnet;
pub mod transport;

pub use events::{Capabilities, Reason, SessionCommand, SessionEvent, SessionId};
pub use line_editor::{EchoMode, EditorEvent, LineEditor};
pub use session_task::{
    ServerConfig, SessionConfig, bind, serve, spawn_session, spawn_session_with,
};
pub use telnet::{TelnetCodec, TelnetTransport};
pub use transport::Transport;
