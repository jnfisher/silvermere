//! The CLI's client for the admin API of a running server: one std-only HTTP request per
//! call (the image has no curl), and the token the config holds for it.

use std::io::{Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

use crate::config::{Config, TokenScope};

/// The address a client reaches `bind` on: an unspecified address is reached on loopback.
pub fn reach(bind: &str) -> Option<SocketAddr> {
    let mut addr: SocketAddr = bind.parse().ok()?;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    Some(addr)
}

/// One HTTP/1.1 request with `Connection: close`, with a JSON `body` if given; the status
/// and the body of the answer.
pub fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
    timeout: Duration,
) -> std::io::Result<(u16, String)> {
    let mut s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let ctype = body.map_or("", |_| "Content-Type: application/json\r\n");
    let body = body.unwrap_or("");
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{auth}{ctype}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    let mut raw = Vec::new();
    s.take(1 << 20).read_to_end(&mut raw)?;
    let raw = String::from_utf8_lossy(&raw);
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| std::io::Error::other("not an HTTP answer"))?;
    let body = raw.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    Ok((status, body.to_string()))
}

/// The secret of a configured token that may do what `scope` allows: an `admin` token
/// first (it sees the most), else one with `scope` itself. Only a token whose secret the
/// config holds counts (`SILVERMERE_ADMIN_TOKEN`, or a `token` in `[[admin.tokens]]`); one
/// stored as a hash cannot be sent.
pub fn token_for(cfg: &Config, scope: TokenScope) -> Option<String> {
    let with = |s: TokenScope| {
        cfg.admin
            .tokens
            .iter()
            .filter(|t| t.scopes.contains(&s))
            .find_map(|t| t.token.clone())
    };
    with(TokenScope::Admin).or_else(|| with(scope))
}
