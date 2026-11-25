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
use options::{CHARSET, CHARSET_OFFER, GMCP, GMCP_OFFER, NAWS, OFFERS, Options};
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
    /// The client answered the CHARSET offer: `true` for `DO` (the request for the text's
    /// character set is in the reply), `false` for `DONT`/`WONT`.
    CharsetAnswered(bool),
    /// The client accepted a requested character set (`SB CHARSET ACCEPTED name`), or
    /// rejected every one offered (`None`).
    CharsetAccepted(Option<Vec<u8>>),
}

/// CHARSET subnegotiation codes (RFC 2066).
const CHARSET_REQUEST: u8 = 1;
const CHARSET_ACCEPTED: u8 = 2;
const CHARSET_REJECTED: u8 = 3;

/// Appends `IAC SB CHARSET REQUEST ";" name IAC SE` to `out`: asks the client to read the
/// text as `name` (`IBM437`, `UTF-8`).
pub fn frame_charset_request(name: &[u8], out: &mut impl Extend<u8>) {
    out.extend([IAC, SB, CHARSET, CHARSET_REQUEST, b';']);
    escape_into(name, out);
    out.extend([IAC, SE]);
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
    /// CHARSET is offered on this connection, and the character set requested when the
    /// client agrees.
    charset_offered: Option<Vec<u8>>,
    /// The client agreed to CHARSET: later requests may be sent.
    charset: bool,
}

impl TelnetCodec {
    pub fn new() -> Self {
        Self::default()
    }

    /// A codec that offers GMCP ([`Self::offers_with`] `true` must be what goes out).
    pub fn with_gmcp() -> Self {
        Self::with_offers(true, None)
    }

    /// A codec that offers GMCP when `gmcp`, and CHARSET when `charset` names the character
    /// set to request once the client agrees. What goes out on accept must be
    /// [`Self::offers_with`] with the same choices.
    pub fn with_offers(gmcp: bool, charset: Option<&[u8]>) -> Self {
        let mut options = Options::new();
        if gmcp {
            options.offer_gmcp();
        }
        if charset.is_some() {
            options.offer_charset();
        }
        Self {
            options,
            gmcp_offered: gmcp,
            charset_offered: charset.map(<[u8]>::to_vec),
            ..Self::default()
        }
    }

    /// The bytes to send on accept.
    pub fn offers() -> &'static [u8] {
        &OFFERS
    }

    /// The bytes to send on accept, then `IAC WILL GMCP` when `gmcp` and `IAC WILL CHARSET`
    /// when `charset`.
    pub fn offers_with(gmcp: bool, charset: bool) -> Vec<u8> {
        let mut out = OFFERS.to_vec();
        if gmcp {
            out.extend_from_slice(&GMCP_OFFER);
        }
        if charset {
            out.extend_from_slice(&CHARSET_OFFER);
        }
        out
    }

    /// The client agreed to CHARSET: [`frame_charset_request`] may be sent.
    pub fn charset_active(&self) -> bool {
        self.charset
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
                    if option == CHARSET
                        && let Some(name) = &self.charset_offered
                    {
                        match verb {
                            Verb::Do if !self.charset => {
                                self.charset = true;
                                frame_charset_request(name, reply);
                                events.push(TelnetEvent::CharsetAnswered(true));
                            }
                            Verb::Dont | Verb::Wont => {
                                self.charset = false;
                                events.push(TelnetEvent::CharsetAnswered(false));
                            }
                            _ => {}
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
                    } else if option == CHARSET && self.charset {
                        match payload.split_first() {
                            Some((&CHARSET_ACCEPTED, name)) => {
                                events.push(TelnetEvent::CharsetAccepted(Some(name.to_vec())));
                            }
                            Some((&CHARSET_REJECTED, _)) => {
                                events.push(TelnetEvent::CharsetAccepted(None));
                            }
                            // TTABLE messages: no translation tables are ever offered
                            _ => {}
                        }
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
    use super::options::CHARSET;
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
        assert_eq!(TelnetCodec::offers_with(false, false), OFFERS.to_vec());
        assert!(TelnetCodec::offers_with(true, false).ends_with(&[IAC, WILL, GMCP]));
        assert!(
            TelnetCodec::offers_with(true, true).ends_with(&[IAC, WILL, GMCP, IAC, WILL, CHARSET])
        );
    }

    #[test]
    fn charset_request_follows_do_and_acceptance_is_reported() {
        let mut c = TelnetCodec::with_offers(false, Some(b"IBM437"));
        let (mut data, mut reply, mut events) = (Vec::new(), Vec::new(), Vec::new());
        c.decode_events(&[IAC, DO, CHARSET], &mut data, &mut reply, &mut events);
        let mut want = vec![IAC, SB, CHARSET, 1, b';'];
        want.extend_from_slice(b"IBM437");
        want.extend([IAC, SE]);
        assert_eq!(reply, want, "no WILL again, just the request");
        assert_eq!(events, vec![TelnetEvent::CharsetAnswered(true)]);
        assert!(c.charset_active());
        events.clear();
        let mut accepted = vec![IAC, SB, CHARSET, 2];
        accepted.extend_from_slice(b"IBM437");
        accepted.extend([IAC, SE]);
        c.decode_events(&accepted, &mut data, &mut reply, &mut events);
        assert_eq!(
            events,
            vec![TelnetEvent::CharsetAccepted(Some(b"IBM437".to_vec()))]
        );
        assert!(data.is_empty());
    }

    #[test]
    fn charset_refusal_is_reported_and_nothing_is_requested() {
        // a terminal that refuses what it does not know (BSD telnet, the testkit client)
        let mut c = TelnetCodec::with_offers(false, Some(b"IBM437"));
        let (mut data, mut reply, mut events) = (Vec::new(), Vec::new(), Vec::new());
        c.decode_events(&[IAC, DONT, CHARSET], &mut data, &mut reply, &mut events);
        assert!(reply.is_empty());
        assert_eq!(events, vec![TelnetEvent::CharsetAnswered(false)]);
        assert!(!c.charset_active());
    }

    #[test]
    fn charset_is_refused_when_not_offered() {
        let mut c = TelnetCodec::new();
        let (mut data, mut reply, mut events) = (Vec::new(), Vec::new(), Vec::new());
        c.decode_events(&[IAC, DO, CHARSET], &mut data, &mut reply, &mut events);
        assert_eq!(reply, vec![IAC, WONT, CHARSET]);
        assert!(events.is_empty());
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
