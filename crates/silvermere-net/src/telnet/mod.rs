//! The hand-written telnet layer: IAC parsing, option negotiation, 0xFF escaping.
//!
//! [`TelnetCodec`] is the pure core (bytes in, data and replies out) shared by the session
//! task and [`TelnetTransport`], the sequential [`Transport`] over a `TcpStream`.

pub mod options;
pub mod parser;

use std::collections::VecDeque;
use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::events::Capabilities;
use crate::transport::Transport;
use options::{NAWS, OFFERS, Options};
use parser::{Action, IAC, Parser};

/// Appends `bytes` to `out` with every 0xFF doubled (`IAC IAC`).
pub fn escape_into(bytes: &[u8], out: &mut impl Extend<u8>) {
    for &b in bytes {
        if b == IAC {
            out.extend([IAC, IAC]);
        } else {
            out.extend([b]);
        }
    }
}

/// Telnet state for one connection, with no I/O.
#[derive(Debug, Clone, Default)]
pub struct TelnetCodec {
    parser: Parser,
    options: Options,
    caps: Capabilities,
}

impl TelnetCodec {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes to send on accept.
    pub fn offers() -> &'static [u8] {
        &OFFERS
    }

    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// Decodes received bytes: data goes to `data`, negotiation answers to `reply`. Returns
    /// the new capabilities when a NAWS report changed them.
    pub fn decode(
        &mut self,
        input: &[u8],
        data: &mut impl Extend<u8>,
        reply: &mut impl Extend<u8>,
    ) -> Option<Capabilities> {
        let mut changed = None;
        for &byte in input {
            match self.parser.feed(byte) {
                None | Some(Action::Command(_)) => {}
                Some(Action::Data(b)) => data.extend([b]),
                Some(Action::Negotiate(verb, option)) => {
                    if let Some(answer) = self.options.respond(verb, option) {
                        reply.extend(answer);
                    }
                }
                Some(Action::Subnegotiation(option, payload)) => {
                    if option == NAWS
                        && let Some(caps) = parse_naws(&payload)
                    {
                        self.caps = caps;
                        changed = Some(caps);
                    }
                }
            }
        }
        changed
    }
}

/// `IAC SB NAWS <w16> <h16> IAC SE`, big-endian. A zero dimension means "unknown" (RFC
/// 1073) and keeps the default for that dimension.
fn parse_naws(payload: &[u8]) -> Option<Capabilities> {
    let [w1, w0, h1, h0] = payload.get(..4)?.try_into().ok()?;
    let default = Capabilities::default();
    let pick = |v: u16, d: u16| if v == 0 { d } else { v };
    Some(Capabilities {
        columns: pick(u16::from_be_bytes([w1, w0]), default.columns),
        rows: pick(u16::from_be_bytes([h1, h0]), default.rows),
    })
}

/// [`Transport`] over a `TcpStream`, for sequential use (tests, tools). The session task
/// drives the same [`TelnetCodec`] over split socket halves so it can read and write at once.
#[derive(Debug)]
pub struct TelnetTransport {
    stream: TcpStream,
    codec: TelnetCodec,
    data: VecDeque<u8>,
    raw: Vec<u8>,
}

impl TelnetTransport {
    /// Wraps an accepted stream and sends the option offers. Does not wait for answers.
    pub async fn accept(mut stream: TcpStream) -> io::Result<Self> {
        stream.write_all(TelnetCodec::offers()).await?;
        Ok(Self {
            stream,
            codec: TelnetCodec::new(),
            data: VecDeque::new(),
            raw: vec![0; 4096],
        })
    }
}

impl Transport for TelnetTransport {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.data.is_empty() {
            let n = self.stream.read(&mut self.raw).await?;
            if n == 0 {
                return Ok(0);
            }
            let mut reply = Vec::new();
            self.codec
                .decode(&self.raw[..n], &mut self.data, &mut reply);
            if !reply.is_empty() {
                self.stream.write_all(&reply).await?;
            }
        }
        let n = buf.len().min(self.data.len());
        for (dst, src) in buf.iter_mut().zip(self.data.drain(..n)) {
            *dst = src;
        }
        Ok(n)
    }

    async fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut escaped = Vec::with_capacity(bytes.len());
        escape_into(bytes, &mut escaped);
        self.stream.write_all(&escaped).await
    }

    fn capabilities(&self) -> Capabilities {
        self.codec.capabilities()
    }

    async fn close(&mut self) {
        // Best effort: the peer may already be gone.
        let _ = self.stream.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::parser::{DO, SB, SE, WILL, WONT};
    use super::*;

    #[test]
    fn escape_doubles_iac_only() {
        let mut out = Vec::new();
        escape_into(&[1, 0xFF, 2], &mut out);
        assert_eq!(out, vec![1, 0xFF, 0xFF, 2]);
    }

    #[test]
    fn naws_updates_capabilities() {
        let mut c = TelnetCodec::new();
        let (mut data, mut reply) = (Vec::new(), Vec::new());
        let caps = c.decode(
            &[IAC, WILL, NAWS, IAC, SB, NAWS, 0, 100, 0, 40, IAC, SE],
            &mut data,
            &mut reply,
        );
        assert_eq!(
            caps,
            Some(Capabilities {
                columns: 100,
                rows: 40
            })
        );
        assert_eq!(reply, vec![IAC, DO, NAWS]);
        assert!(data.is_empty());
    }

    #[test]
    fn zero_naws_dimension_keeps_default() {
        assert_eq!(
            parse_naws(&[0, 0, 0, 50]),
            Some(Capabilities {
                columns: 80,
                rows: 50
            })
        );
    }

    #[test]
    fn data_and_replies_are_separated() {
        let mut c = TelnetCodec::new();
        let (mut data, mut reply) = (Vec::new(), Vec::new());
        c.decode(&[b'h', IAC, DO, 24, b'i'], &mut data, &mut reply);
        assert_eq!(data, b"hi");
        assert_eq!(reply, vec![IAC, WONT, 24]);
    }
}
