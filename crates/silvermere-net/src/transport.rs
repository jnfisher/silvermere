//! The `Transport` abstraction the session layer is written against.

use std::future::Future;
use std::io;

use crate::events::Capabilities;

/// A byte stream to one client with the protocol framing (telnet now, WebSocket later)
/// already removed from reads and added to writes.
///
/// Written with return-position `impl Future` rather than `#[async_trait]`: the workspace
/// does not carry `async-trait`, and stable Rust expresses the `Send` bound directly.
pub trait Transport: Send {
    /// Reads application data (negotiation answered and stripped). `Ok(0)` is end of stream.
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> + Send;
    /// Writes application data, escaping it for the protocol.
    fn write_all(&mut self, bytes: &[u8]) -> impl Future<Output = io::Result<()>> + Send;
    /// The terminal capabilities learned so far.
    fn capabilities(&self) -> Capabilities;
    /// Flushes and closes the connection.
    fn close(&mut self) -> impl Future<Output = ()> + Send;
}
