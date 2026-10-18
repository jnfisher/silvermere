//! A minimal telnet client that behaves like the harness bridge (`tools/differential/vendor/play/bridge.py`).
//!
//! Every `DO x` is answered `WONT x` (the one exception is NAWS when a window size was
//! given, as `capture.py` does), every `WILL x` is ignored (bridge.py does not reply to
//! it; kept), `IAC IAC` is one 0xFF byte and subnegotiations are skipped. What is left is
//! recorded as raw bytes and decoded as CP437. Waiting follows `Runner.wait`
//! (the harness's `capture.py`): until a regex matches the ANSI-stripped text since a point, or a
//! quiet period, or the timeout, or the server hangs up.
//!
//! The client has no reader task: bytes are read when a call waits for them, and bytes
//! that arrived between commands come back as `skipped` on the next send (the telepath or
//! late line `bridge.Session.send` reports separately).

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::captures::plain;
use crate::patterns::Rx;

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const NAWS: u8 = 31;

/// Default `timeout` of `until` in scripts (`bridge.py` `--timeout`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default quiet period of `read` (`bridge.py` `--wait`).
pub const DEFAULT_QUIET: Duration = Duration::from_secs(1);

/// What a wait collected.
#[derive(Debug, Clone, Default)]
pub struct Captured {
    /// Bytes after telnet stripping, since the previous collection.
    pub raw: Vec<u8>,
    /// The same, decoded and passed through `bridge.plain`.
    pub plain: String,
    /// Text that arrived between the previous collection and the last send.
    pub skipped: String,
    /// The `until` regex (or cursor position) matched; `true` for a quiet wait that ended
    /// normally.
    pub matched: bool,
    /// How long the wait took.
    pub waited: Duration,
}

/// Client errors.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    #[error("bad regex {0:?}")]
    Regex(String),
    #[error("timed out after {timeout:?} waiting for {pattern:?}; got {got:?}")]
    Timeout {
        pattern: String,
        timeout: Duration,
        got: String,
    },
    #[error("not connected")]
    NotConnected,
}

/// What to wait for after a point in the stream (`Runner.wait`).
#[derive(Debug, Clone)]
pub struct Wait {
    /// Regex searched in the plain text since the start.
    pub until: Option<String>,
    /// `"row;col"`: the last `ESC[row;colf` since the start is this, and 0.3 s pass quiet.
    pub until_cursor: Option<String>,
    /// With neither of the above: end after this long without output.
    pub quiet: Duration,
    pub timeout: Duration,
}

impl Default for Wait {
    fn default() -> Self {
        Self {
            until: None,
            until_cursor: None,
            quiet: DEFAULT_QUIET,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Iac {
    Data,
    Cmd,
    Option(u8),
    Sub,
    SubEnd,
}

/// A telnet connection to a server under test.
pub struct TelnetClient {
    addr: SocketAddr,
    stream: Option<TcpStream>,
    naws: Option<(u16, u16)>,
    iac: Iac,
    /// Every byte received over all connections, telnet stripped.
    raw: Vec<u8>,
    /// Start of the unread part of `raw`.
    read_pos: usize,
    last_rx: Instant,
    skipped: String,
}

impl TelnetClient {
    /// Opens a connection and refuses every option the server offers.
    pub async fn connect(addr: SocketAddr) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self {
            addr,
            stream: Some(stream),
            naws: None,
            iac: Iac::Data,
            raw: Vec::new(),
            read_pos: 0,
            last_rx: Instant::now(),
            skipped: String::new(),
        })
    }

    /// Answers `DO NAWS` with this window size instead of refusing it.
    pub fn with_naws(mut self, columns: u16, rows: u16) -> Self {
        self.naws = Some((columns, rows));
        self
    }

    /// Closes the connection (if open) and opens a new one to the same address. The raw
    /// stream keeps growing, as `Runner` concatenates connections.
    pub async fn reconnect(&mut self) -> io::Result<()> {
        self.disconnect().await;
        self.stream = Some(TcpStream::connect(self.addr).await?);
        self.iac = Iac::Data;
        self.last_rx = Instant::now();
        Ok(())
    }

    /// Drops the connection.
    pub async fn disconnect(&mut self) {
        if let Some(mut s) = self.stream.take() {
            let _ = s.shutdown().await;
        }
    }

    pub fn is_open(&self) -> bool {
        self.stream.is_some()
    }

    /// Every byte received so far (telnet stripped, CP437, ANSI kept).
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// `raw()` decoded and passed through `bridge.plain`.
    pub fn plain(&self) -> String {
        plain(&silvermere_net::cp437::decode(&self.raw))
    }

    /// Length of the stream so far: the `offset` a script step records.
    pub fn position(&self) -> usize {
        self.raw.len()
    }

    /// `bridge.plain` of the stream from `offset`.
    pub fn plain_from(&self, offset: usize) -> String {
        plain(&silvermere_net::cp437::decode(
            &self.raw[offset.min(self.raw.len())..],
        ))
    }

    /// Sends `text` and a CR. Returns what arrived unread before the send.
    pub async fn send_line(&mut self, text: &str) -> Result<String, ClientError> {
        let mut bytes = encode(text);
        bytes.push(b'\r');
        self.send_bytes(&bytes).await
    }

    /// Sends `text` with no CR (the creation editor is driven one key at a time).
    pub async fn send_text(&mut self, text: &str) -> Result<String, ClientError> {
        self.send_bytes(&encode(text)).await
    }

    /// Sends bytes exactly. Returns what arrived unread before the send.
    pub async fn send_raw(&mut self, bytes: &[u8]) -> Result<String, ClientError> {
        self.send_bytes(bytes).await
    }

    async fn send_bytes(&mut self, bytes: &[u8]) -> Result<String, ClientError> {
        self.drain_available().await?;
        let skipped = plain(&silvermere_net::cp437::decode(&self.raw[self.read_pos..]));
        self.read_pos = self.raw.len();
        self.skipped = skipped.clone();
        let stream = self.stream.as_mut().ok_or(ClientError::NotConnected)?;
        stream.write_all(bytes).await?;
        Ok(skipped)
    }

    /// Waits until `pattern` matches the plain text since the last collection.
    pub async fn until(
        &mut self,
        pattern: &str,
        timeout: Duration,
    ) -> Result<Captured, ClientError> {
        let start = self.read_pos;
        let got = self
            .wait_from(
                start,
                &Wait {
                    until: Some(pattern.to_owned()),
                    timeout,
                    ..Wait::default()
                },
            )
            .await?;
        if got.matched {
            Ok(got)
        } else {
            Err(ClientError::Timeout {
                pattern: pattern.to_owned(),
                timeout,
                got: got.plain,
            })
        }
    }

    /// Collects what arrives until `quiet` passes without output (`bridge.py read`).
    pub async fn read(&mut self, quiet: Duration) -> Result<Captured, ClientError> {
        let start = self.read_pos;
        self.wait_from(
            start,
            &Wait {
                quiet,
                ..Wait::default()
            },
        )
        .await
    }

    /// Waits as `Runner.wait` does for output after `start` (a [`position`](Self::position)),
    /// then returns everything since then and marks it read.
    pub async fn wait_from(&mut self, start: usize, wait: &Wait) -> Result<Captured, ClientError> {
        let rx = match &wait.until {
            Some(src) => Some(Rx::new(src).map_err(|_| ClientError::Regex(src.clone()))?),
            None => None,
        };
        let t0 = Instant::now();
        let matched = loop {
            let now = Instant::now();
            let new_plain = self.plain_from(start);
            if rx.as_ref().is_some_and(|rx| rx.is_match(&new_plain)) {
                break true;
            }
            if let Some(want) = &wait.until_cursor {
                let text = silvermere_net::cp437::decode(&self.raw[start.min(self.raw.len())..]);
                if cursor_scan(&text).as_deref() == Some(want.as_str())
                    && now.duration_since(self.last_rx) >= Duration::from_millis(300)
                {
                    break true;
                }
            }
            if rx.is_none()
                && wait.until_cursor.is_none()
                && now.duration_since(self.last_rx.max(t0)) >= wait.quiet
            {
                break true;
            }
            if self.stream.is_none() {
                break rx.is_none() && wait.until_cursor.is_none();
            }
            if now.duration_since(t0) >= wait.timeout {
                break false;
            }
            self.pump(Duration::from_millis(50)).await?;
        };
        let end = self.raw.len();
        let raw = self.raw[start.min(end)..end].to_vec();
        self.read_pos = end;
        Ok(Captured {
            plain: plain(&silvermere_net::cp437::decode(&raw)),
            raw,
            skipped: std::mem::take(&mut self.skipped),
            matched,
            waited: t0.elapsed(),
        })
    }

    /// Waits for the server to hang up; `true` if it did within `timeout`. Output that
    /// arrives meanwhile is kept in the stream.
    pub async fn wait_closed(&mut self, timeout: Duration) -> Result<bool, ClientError> {
        let t0 = Instant::now();
        while self.stream.is_some() {
            if t0.elapsed() >= timeout {
                return Ok(false);
            }
            self.pump(Duration::from_millis(50)).await?;
        }
        Ok(true)
    }

    /// Reads whatever is already waiting, without blocking.
    async fn drain_available(&mut self) -> Result<(), ClientError> {
        while self.stream.is_some() {
            if !self.pump(Duration::from_millis(1)).await? {
                break;
            }
        }
        Ok(())
    }

    /// Reads one chunk if one arrives within `limit`. `true` if bytes were read.
    async fn pump(&mut self, limit: Duration) -> Result<bool, ClientError> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(false);
        };
        let mut buf = [0u8; 8192];
        let n = match tokio::time::timeout(limit, stream.read(&mut buf)).await {
            Err(_) => return Ok(false),
            Ok(Ok(0)) => 0,
            Ok(Ok(n)) => n,
            Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionReset => 0,
            Ok(Err(e)) => return Err(e.into()),
        };
        if n == 0 {
            self.stream = None;
            return Ok(false);
        }
        let (data, replies) = self.strip_telnet(&buf[..n]);
        if !replies.is_empty()
            && let Some(stream) = self.stream.as_mut()
        {
            stream.write_all(&replies).await?;
        }
        if !data.is_empty() {
            self.raw.extend_from_slice(&data);
            self.last_rx = Instant::now();
        }
        Ok(true)
    }

    /// `bridge.Session.strip_telnet`: `(data, replies)`, state kept across chunks.
    fn strip_telnet(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut out = Vec::with_capacity(chunk.len());
        let mut replies = Vec::new();
        for &b in chunk {
            self.iac = match self.iac {
                Iac::Data if b == IAC => Iac::Cmd,
                Iac::Data => {
                    out.push(b);
                    Iac::Data
                }
                Iac::Cmd => match b {
                    IAC => {
                        out.push(IAC);
                        Iac::Data
                    }
                    DO | DONT | WILL | WONT => Iac::Option(b),
                    SB => Iac::Sub,
                    _ => Iac::Data,
                },
                Iac::Option(DO) => {
                    match (b, self.naws) {
                        (NAWS, Some((w, h))) => {
                            replies.extend_from_slice(&[IAC, WILL, NAWS, IAC, SB, NAWS]);
                            replies.extend_from_slice(&w.to_be_bytes());
                            replies.extend_from_slice(&h.to_be_bytes());
                            replies.extend_from_slice(&[IAC, SE]);
                        }
                        _ => replies.extend_from_slice(&[IAC, WONT, b]),
                    }
                    Iac::Data
                }
                Iac::Option(_) => Iac::Data,
                Iac::Sub if b == IAC => Iac::SubEnd,
                Iac::Sub => Iac::Sub,
                Iac::SubEnd if b == SE => Iac::Data,
                Iac::SubEnd => Iac::Sub,
            };
        }
        (out, replies)
    }
}

/// `row;col` of the last `ESC [ row ; col f|H`.
fn cursor_scan(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut last = None;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            let row_start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > row_start && j < bytes.len() && bytes[j] == b';' {
                let row = &text[row_start..j];
                j += 1;
                let col_start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > col_start && j < bytes.len() && matches!(bytes[j], b'f' | b'H') {
                    last = Some(format!("{row};{}", &text[col_start..j]));
                    i = j;
                }
            }
        }
        i += 1;
    }
    last
}

/// `text.encode("cp437")`; characters with no CP437 byte become `?`.
pub fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                c as u8
            } else {
                (0x80..=0xffu8)
                    .find(|&b| silvermere_net::cp437::decode_byte(b) == c)
                    .unwrap_or(b'?')
            }
        })
        .collect()
}
