//! The rlogin door login (RFC 1282): a BBS that already knows who its caller is opens the
//! game with that name, and the caller is not asked to log in a second time.
//!
//! The client sends `NUL client-user NUL server-user NUL terminal/speed NUL`; the server
//! answers one `NUL` and from then on the connection carries plain bytes both ways, with no
//! telnet commands. The name is taken on trust, so only the addresses in
//! [`RloginConfig::trusted`] (loopback when the list is empty) are served: anyone else who
//! could reach the port could be anyone.

use std::io;
use std::net::IpAddr;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// How long a caller has to send the whole handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest handshake read; each field is short (RFC 1282 bounds the names at 16
/// bytes, BBSes send longer ones, nothing sends kilobytes).
const HANDSHAKE_MAX: usize = 1024;

/// rlogin listener settings.
#[derive(Debug, Clone, Default)]
pub struct RloginConfig {
    /// The addresses allowed to connect. Empty: loopback only (the BBS on this computer).
    pub trusted: Vec<IpAddr>,
}

impl RloginConfig {
    /// Whether `peer` may say who its caller is.
    pub fn trusts(&self, peer: IpAddr) -> bool {
        let peer = canonical(peer);
        if self.trusted.is_empty() {
            return peer.is_loopback();
        }
        self.trusted.iter().any(|t| canonical(*t) == peer)
    }
}

/// An IPv4 address that came in on an IPv6 socket, as the IPv4 address.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// What the rlogin client said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RloginHello {
    /// The user on the calling system (the BBS's name for the caller).
    pub client_user: String,
    /// The account asked for on this server.
    pub server_user: String,
    /// The terminal type, without the `/speed`.
    pub terminal: String,
}

impl RloginHello {
    /// The account to log in: the server user, or the client user when that is blank
    /// (BBSes differ in which field they fill).
    pub fn user(&self) -> &str {
        let server = self.server_user.trim();
        if server.is_empty() {
            self.client_user.trim()
        } else {
            server
        }
    }
}

/// Reads the handshake from `stream`. Returns the hello and any bytes the client sent
/// after it (typed ahead), or an error for a malformed or too long handshake.
pub async fn read_hello(stream: &mut TcpStream) -> io::Result<(RloginHello, Vec<u8>)> {
    let mut buf = Vec::with_capacity(128);
    let mut chunk = [0u8; 256];
    loop {
        if let Some((hello, used)) = parse_hello(&buf)? {
            return Ok((hello, buf.split_off(used)));
        }
        if buf.len() >= HANDSHAKE_MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rlogin handshake too long",
            ));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Parses a complete handshake at the start of `buf`: the hello and the bytes it used, or
/// `None` while it is still incomplete.
pub fn parse_hello(buf: &[u8]) -> io::Result<Option<(RloginHello, usize)>> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let Some(&first) = buf.first() else {
        return Ok(None);
    };
    if first != 0 {
        return Err(bad("rlogin handshake does not start with NUL"));
    }
    let mut fields = Vec::with_capacity(3);
    let mut start = 1;
    for (i, &b) in buf.iter().enumerate().skip(1) {
        if b == 0 {
            fields.push(&buf[start..i]);
            start = i + 1;
            if fields.len() == 3 {
                let text = |f: &[u8]| -> io::Result<String> {
                    if f.iter().all(|c| (0x20..0x7f).contains(c)) {
                        Ok(String::from_utf8_lossy(f).into_owned())
                    } else {
                        Err(bad("rlogin handshake field is not printable ASCII"))
                    }
                };
                let terminal = text(fields[2])?;
                let terminal = terminal
                    .split_once('/')
                    .map_or(terminal.as_str(), |(t, _)| t)
                    .to_string();
                return Ok(Some((
                    RloginHello {
                        client_user: text(fields[0])?,
                        server_user: text(fields[1])?,
                        terminal,
                    },
                    start,
                )));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn parses_the_three_fields_and_keeps_what_follows() {
        let (hello, used) = parse_hello(b"\0Alice Able\0alice\0ansi/38400\0look")
            .unwrap()
            .unwrap();
        assert_eq!(hello.client_user, "Alice Able");
        assert_eq!(hello.server_user, "alice");
        assert_eq!(hello.terminal, "ansi");
        assert_eq!(hello.user(), "alice");
        assert_eq!(&b"\0Alice Able\0alice\0ansi/38400\0look"[used..], b"look");
    }

    #[test]
    fn a_blank_server_user_means_the_client_user() {
        let (hello, _) = parse_hello(b"\0Bob\0\0vt100/9600\0").unwrap().unwrap();
        assert_eq!(hello.user(), "Bob");
    }

    #[test]
    fn incomplete_waits_and_garbage_is_refused() {
        assert!(parse_hello(b"").unwrap().is_none());
        assert!(parse_hello(b"\0Alice\0al").unwrap().is_none());
        assert!(parse_hello(b"GET / HTTP/1.1\r\n").is_err());
        assert!(parse_hello(b"\0Al\x1bce\0a\0ansi\0").is_err());
    }

    #[test]
    fn only_loopback_is_trusted_by_default() {
        let open = RloginConfig::default();
        assert!(open.trusts(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(open.trusts(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(open.trusts(IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped())));
        assert!(!open.trusts(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))));
        let listed = RloginConfig {
            trusted: vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))],
        };
        assert!(listed.trusts(IpAddr::V6(Ipv4Addr::new(192, 168, 1, 5).to_ipv6_mapped())));
        assert!(!listed.trusts(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }
}
