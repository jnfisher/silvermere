//! The line discipline: what the Worldgroup GSBL did with each typed byte.
//!
//! Per byte: BS and DEL erase, CR ends the line (a following LF or NUL is swallowed), a bare
//! LF also ends it, XOFF and other control bytes are dropped, everything else (high-bit CP437
//! bytes included) is kept. Echo happens here, at receive time, so type-ahead splices into
//! output as it did on the original host.

use std::collections::VecDeque;

/// Longest line: the game sets the host's line limit (`btumil`) to 254 on realm entry. The
/// BBS menus used the host default (255); one limit everywhere is fine.
pub const LINE_LIMIT: usize = 254;

/// Completed events held for the engine. The original held two (the GSBL "deferred CR");
/// the session task stops reading the socket when this many are waiting, so the cap only
/// drops input when [`LineEditor::feed`] is driven past it directly.
pub const PENDING_CAP: usize = 64;

const BS: u8 = 0x08;
const LF: u8 = 0x0A;
const CR: u8 = 0x0D;
const DEL: u8 = 0x7F;
const NUL: u8 = 0x00;
/// Erase echo: back, blank, back.
const RUBOUT: [u8; 3] = [BS, b' ', BS];

/// How typed bytes are echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EchoMode {
    /// Echo each byte as typed (`echon`).
    #[default]
    On,
    /// Echo nothing, not even the terminator.
    Off,
    /// Echo `ch` per byte up to `width` bytes, nothing beyond (the host's `echsec`;
    /// the password prompt uses `*` and 8). Lasts for one line, then echo returns to `On`.
    Secure { ch: u8, width: u8 },
}

/// One thing the editor has for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEvent {
    Line(Vec<u8>),
    LineTooLong,
    Key(u8),
}

/// Per-session line editor.
#[derive(Debug, Clone)]
pub struct LineEditor {
    echo: EchoMode,
    limit: usize,
    buf: Vec<u8>,
    pending: VecDeque<EditorEvent>,
    char_mode: bool,
    after_cr: bool,
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl LineEditor {
    pub fn new() -> Self {
        Self::with_limit(LINE_LIMIT)
    }

    pub fn with_limit(limit: usize) -> Self {
        Self {
            echo: EchoMode::On,
            limit,
            buf: Vec::new(),
            pending: VecDeque::new(),
            char_mode: false,
            after_cr: false,
        }
    }

    pub fn echo(&self) -> EchoMode {
        self.echo
    }

    pub fn set_echo(&mut self, echo: EchoMode) {
        self.echo = echo;
    }

    /// Character mode (the game's single-key input): each byte becomes a `Key` with
    /// no editing and no echo. Escape-sequence folding stays with the engine (`escstate`).
    pub fn set_char_mode(&mut self, on: bool) {
        self.char_mode = on;
    }

    /// The partial line typed so far.
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    /// Whether the pending queue is at its cap.
    pub fn is_full(&self) -> bool {
        self.pending.len() >= PENDING_CAP
    }

    /// Whether a completed event is waiting.
    pub fn has_event(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Whether the oldest completed event is a line.
    pub fn line_waiting(&self) -> bool {
        matches!(self.pending.front(), Some(EditorEvent::Line(_)))
    }

    /// The oldest completed event, if any.
    pub fn next_event(&mut self) -> Option<EditorEvent> {
        self.pending.pop_front()
    }

    /// Feeds a run of bytes; see [`LineEditor::feed`].
    pub fn feed_all(&mut self, bytes: &[u8], echo: &mut Vec<u8>) {
        for &b in bytes {
            self.feed(b, echo);
        }
    }

    /// Feeds one data byte (telnet already removed); echo bytes are appended to `echo`.
    pub fn feed(&mut self, byte: u8, echo: &mut Vec<u8>) {
        // CR LF and CR NUL are one Enter.
        if std::mem::take(&mut self.after_cr) && (byte == LF || byte == NUL) {
            return;
        }
        if self.char_mode {
            match byte {
                CR => {
                    self.after_cr = true;
                    self.push(EditorEvent::Key(CR));
                }
                LF => self.push(EditorEvent::Key(CR)),
                // XOFF is swallowed by the host before anything else.
                0x13 => {}
                _ => self.push(EditorEvent::Key(byte)),
            }
            return;
        }
        match byte {
            CR => {
                self.after_cr = true;
                self.end_line(echo);
            }
            LF => self.end_line(echo),
            BS | DEL => self.erase(echo),
            // Other control bytes (XOFF 0x13 among them) are dropped.
            0x00..=0x1F => {}
            _ => self.insert(byte, echo),
        }
    }

    fn insert(&mut self, byte: u8, echo: &mut Vec<u8>) {
        if self.buf.len() >= self.limit {
            // btumil overflow, GSBL status 251: the line is discarded.
            self.buf.clear();
            self.push(EditorEvent::LineTooLong);
            return;
        }
        self.buf.push(byte);
        match self.echo {
            EchoMode::On => echo.push(byte),
            EchoMode::Off => {}
            EchoMode::Secure { ch, width } => {
                if self.buf.len() <= usize::from(width) {
                    echo.push(ch);
                }
            }
        }
    }

    fn erase(&mut self, echo: &mut Vec<u8>) {
        let before = self.buf.len();
        if self.buf.pop().is_none() {
            return;
        }
        let shown = match self.echo {
            EchoMode::On => true,
            EchoMode::Off => false,
            EchoMode::Secure { width, .. } => before <= usize::from(width),
        };
        if shown {
            echo.extend_from_slice(&RUBOUT);
        }
    }

    fn end_line(&mut self, echo: &mut Vec<u8>) {
        match self.echo {
            EchoMode::Off => {}
            EchoMode::On => echo.extend_from_slice(b"\r\n"),
            EchoMode::Secure { .. } => {
                echo.extend_from_slice(b"\r\n");
                self.echo = EchoMode::On;
            }
        }
        let line = std::mem::take(&mut self.buf);
        self.push(EditorEvent::Line(line));
    }

    fn push(&mut self, event: EditorEvent) {
        if self.is_full() {
            tracing::warn!(?event, "input queue full; newest input dropped");
            return;
        }
        self.pending.push_back(event);
    }
}
