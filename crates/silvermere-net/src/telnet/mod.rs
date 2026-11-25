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
use options::{GMCP, GMCP_OFFER, NAWS, OFFERS, Options};
use parser::{Action, IAC, Parser, SB, SE, Verb};

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

/// Appends one GMCP message (`Package.Name json`, UTF-8) to `out` as `IAC SB GMCP <message>
/// IAC SE`, with every 0xFF in it doubled.
pub fn frame_gmcp(message: &[u8], out: &mut impl Extend<u8>) {
    out.extend([IAC, SB, GMCP]);
    escape_into(message, out);
    out.extend([IAC, SE]);
}

/// What a received chunk told the session besides data and replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelnetEvent {
    /// The client reported its window size; the codec's capabilities are updated.
    Naws(Capabilities),
    /// The client agreed to GMCP (`DO GMCP`, `true`) or turned it off (`DONT GMCP`).
    Gmcp(bool),
    /// A GMCP message from the client, `Package.Name` and optional JSON, as sent.
    GmcpMessage(Vec<u8>),
}

/// Telnet state for one connection, with no I/O.
#[derive(Debug, Clone, Default)]
pub struct TelnetCodec {
    parser: Parser,
    options: Options,
    caps: Capabilities,
    /// GMCP is offered on this connection.
    gmcp_offered: bool,
    /// The client agreed to GMCP and has not turned it off.
    gmcp: bool,
}

impl TelnetCodec {
    pub fn new() -> Self {
        Self::default()
    }

    /// A codec that offers GMCP ([`Self::offers_with`] `true` must be what goes out).
    pub fn with_gmcp() -> Self {
        Self {
            options: Options::with_gmcp(),
            gmcp_offered: true,
            ..Self::default()
        }
    }

    /// The bytes to send on accept.
    pub fn offers() -> &'static [u8] {
        &OFFERS
    }

    /// The bytes to send on accept, with `IAC WILL GMCP` last when `gmcp`.
    pub fn offers_with(gmcp: bool) -> Vec<u8> {
        let mut out = OFFERS.to_vec();
        if gmcp {
            out.extend_from_slice(&GMCP_OFFER);
        }
        out
    }

    /// The client agreed to GMCP: messages may be sent to it.
    pub fn gmcp_active(&self) -> bool {
        self.gmcp
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
        let mut events = Vec::new();
        self.decode_events(input, data, reply, &mut events);
        events.into_iter().rev().find_map(|e| match e {
            TelnetEvent::Naws(caps) => Some(caps),
            _ => None,
        })
    }

    /// [`Self::decode`], with everything else the client said (window size, GMCP) in
    /// `events`, in the order it arrived.
    pub fn decode_events(
        &mut self,
        input: &[u8],
        data: &mut impl Extend<u8>,
        reply: &mut impl Extend<u8>,
        events: &mut Vec<TelnetEvent>,
    ) {
        for &byte in input {
            match self.parser.feed(byte) {
                None | Some(Action::Command(_)) => {}
                Some(Action::Data(b)) => data.extend([b]),
                Some(Action::Negotiate(verb, option)) => {
                    if let Some(answer) = self.options.respond(verb, option) {
                        reply.extend(answer);
                    }
                    if option == GMCP && self.gmcp_offered {
                        let on = match verb {
                            Verb::Do => true,
                            Verb::Dont => false,
                            Verb::Will | Verb::Wont => self.gmcp,
                        };
                        if on != self.gmcp {
                            self.gmcp = on;
                            events.push(TelnetEvent::Gmcp(on));
                        }
                    }
                }
                Some(Action::Subnegotiation(option, payload)) => {
                    if option == NAWS
                        && let Some(caps) = parse_naws(&payload)
                    {
                        self.caps = caps;
                        events.push(TelnetEvent::Naws(caps));
                    } else if option == GMCP && self.gmcp {
                        events.push(TelnetEvent::GmcpMessage(payload));
                    }
                }
            }
        }
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
    use super::parser::{DO, DONT, WILL, WONT};
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
    fn gmcp_is_agreed_and_messages_come_through() {
        let mut c = TelnetCodec::with_gmcp();
        let (mut data, mut reply, mut events) = (Vec::new(), Vec::new(), Vec::new());
        let mut input = vec![IAC, DO, GMCP, IAC, SB, GMCP];
        input.extend_from_slice(b"Core.Hello {\"client\":\"Mudlet\"}");
        input.extend([IAC, SE, b'x']);
        c.decode_events(&input, &mut data, &mut reply, &mut events);
        assert!(reply.is_empty(), "the DO acknowledges our offer");
        assert_eq!(data, b"x");
        assert!(c.gmcp_active());
        assert_eq!(
            events,
            vec![
                TelnetEvent::Gmcp(true),
                TelnetEvent::GmcpMessage(b"Core.Hello {\"client\":\"Mudlet\"}".to_vec()),
            ]
        );
        events.clear();
        c.decode_events(&[IAC, DONT, GMCP], &mut data, &mut reply, &mut events);
        assert_eq!(events, vec![TelnetEvent::Gmcp(false)]);
        assert!(!c.gmcp_active());
    }

    #[test]
    fn gmcp_is_refused_when_not_offered() {
        let mut c = TelnetCodec::new();
        let (mut data, mut reply, mut events) = (Vec::new(), Vec::new(), Vec::new());
        c.decode_events(
            &[IAC, DO, GMCP, IAC, SB, GMCP, b'a', IAC, SE],
            &mut data,
            &mut reply,
            &mut events,
        );
        assert_eq!(reply, vec![IAC, WONT, GMCP]);
        assert!(events.is_empty());
        assert!(!c.gmcp_active());
    }

    #[test]
    fn gmcp_frame_doubles_iac() {
        let mut out = Vec::new();
        frame_gmcp(b"A \xff", &mut out);
        assert_eq!(out, vec![IAC, SB, GMCP, b'A', b' ', 0xFF, 0xFF, IAC, SE]);
    }

    #[test]
    fn offers_end_with_gmcp_when_on() {
        assert_eq!(TelnetCodec::offers_with(false), OFFERS.to_vec());
        assert!(TelnetCodec::offers_with(true).ends_with(&[IAC, WILL, GMCP]));
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
