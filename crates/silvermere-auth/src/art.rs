//! The login art (T-118): the ANSI screen a connection sees before the first prompt, the
//! way a BBS showed its logon screen.
//!
//! The default is the SILVERMERE logo in `art/LOGIN.ANS` (made by `tools/art/make_logos.py`,
//! candidate 02). A sysop replaces it by putting a `LOGIN.ANS` in the assets directory; an
//! empty file shows no art. A file that breaks the MegaMud-safe rules ([`check`]) is not
//! sent: the default is, with a warning in the log. Described in `docs/sysop.md`.

use std::path::Path;

/// The built-in login art.
pub const DEFAULT: &[u8] = include_bytes!("../art/LOGIN.ANS");

/// The replacement file's name in the assets directory.
pub const FILE_NAME: &str = "LOGIN.ANS";

/// The most lines the art may have, so the prompt below it fits on a 25-line screen.
pub const MAX_LINES: usize = 20;

/// The most columns a line may use; the 80th is left free so no terminal wraps.
pub const MAX_COLUMNS: usize = 79;

const ESC: u8 = 0x1B;
const RESET: &[u8] = b"\x1b[0m";

/// Why an art file may not be sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtError {
    /// More than [`MAX_LINES`] lines.
    #[error("{0} lines (at most {MAX_LINES})")]
    TooManyLines(usize),
    /// A line wider than [`MAX_COLUMNS`].
    #[error("line {line}: {columns} columns (at most {MAX_COLUMNS})")]
    TooWide {
        /// The line, from 1.
        line: usize,
        /// Its width.
        columns: usize,
    },
    /// A CR or LF that is not part of a CR LF pair.
    #[error("line {0}: a CR or LF that is not CR LF")]
    BareLineEnd(usize),
    /// A control byte other than ESC, CR and LF.
    #[error("line {line}: control byte {byte:#04x}")]
    Control {
        /// The line, from 1.
        line: usize,
        /// The byte.
        byte: u8,
    },
    /// An escape sequence other than `ESC[...m`.
    #[error("line {0}: an escape sequence other than ESC[...m")]
    Escape(usize),
    /// An SGR parameter outside 0, 1, 5, 30-37 and 40-47.
    #[error("line {line}: SGR parameter {param}")]
    Sgr {
        /// The line, from 1.
        line: usize,
        /// The parameter.
        param: String,
    },
    /// The art does not end with `ESC[0m` (optionally followed by CR LF).
    #[error("does not end with ESC[0m")]
    NoReset,
}

/// Checks the MegaMud-safe rules: at most [`MAX_LINES`] lines of at most [`MAX_COLUMNS`]
/// columns, CR LF line ends, CP437 text with no control bytes but ESC, CR and LF, only
/// `ESC[...m` with the 16 standard colours (0, 1, 5, 30-37, 40-47), and `ESC[0m` at the end.
pub fn check(art: &[u8]) -> Result<(), ArtError> {
    let body = art.strip_suffix(b"\r\n").unwrap_or(art);
    if !body.ends_with(RESET) {
        return Err(ArtError::NoReset);
    }
    let lines: Vec<&[u8]> = split_crlf(body);
    if lines.len() > MAX_LINES {
        return Err(ArtError::TooManyLines(lines.len()));
    }
    for (n, line) in lines.iter().enumerate() {
        let n = n + 1;
        let mut columns = 0;
        let mut i = 0;
        while i < line.len() {
            match line[i] {
                ESC => i = sgr(line, i, n)?,
                b'\r' | b'\n' => return Err(ArtError::BareLineEnd(n)),
                b if b < 0x20 || b == 0x7F => return Err(ArtError::Control { line: n, byte: b }),
                _ => {
                    columns += 1;
                    i += 1;
                }
            }
        }
        if columns > MAX_COLUMNS {
            return Err(ArtError::TooWide { line: n, columns });
        }
    }
    Ok(())
}

fn split_crlf(body: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.windows(2).position(|w| w == b"\r\n") {
        lines.push(&rest[..at]);
        rest = &rest[at + 2..];
    }
    lines.push(rest);
    lines
}

/// Checks the `ESC[...m` at `i`; returns the index after it.
fn sgr(line: &[u8], i: usize, n: usize) -> Result<usize, ArtError> {
    if line.get(i + 1) != Some(&b'[') {
        return Err(ArtError::Escape(n));
    }
    let start = i + 2;
    let len = line[start..]
        .iter()
        .position(|&b| !(b.is_ascii_digit() || b == b';'))
        .ok_or(ArtError::Escape(n))?;
    if line[start + len] != b'm' {
        return Err(ArtError::Escape(n));
    }
    for p in line[start..start + len].split(|&b| b == b';') {
        let v: u32 = if p.is_empty() {
            0
        } else {
            std::str::from_utf8(p)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(u32::MAX)
        };
        if !matches!(v, 0 | 1 | 5 | 30..=37 | 40..=47) {
            return Err(ArtError::Sgr {
                line: n,
                param: String::from_utf8_lossy(p).into_owned(),
            });
        }
    }
    Ok(start + len + 1)
}

/// Drops a SAUCE record and anything else from the first `0x1A` (DOS end of file) on, as
/// ANSI editors append them; the terminal never showed those bytes.
pub fn strip_sauce(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|&b| b == 0x1A) {
        Some(at) => &bytes[..at],
        None => bytes,
    }
}

/// The art a server sends, and why it is not the file asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginArt {
    /// The bytes to send; empty sends nothing.
    pub bytes: Vec<u8>,
    /// Set when a replacement file was there but could not be used.
    pub warning: Option<String>,
}

/// The art for the assets directory `dir`: its [`FILE_NAME`] when present and valid (empty
/// means no art), else [`DEFAULT`]. A file that cannot be read or breaks the rules gives
/// the default and a warning, which is also logged.
pub fn load(dir: &Path) -> LoginArt {
    let path = dir.join(FILE_NAME);
    let fallback = |warning: String| {
        tracing::warn!(file = %path.display(), "login art not used, sending the default: {warning}");
        LoginArt {
            bytes: DEFAULT.to_vec(),
            warning: Some(warning),
        }
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return LoginArt {
                bytes: DEFAULT.to_vec(),
                warning: None,
            };
        }
        Err(e) => return fallback(e.to_string()),
    };
    let art = strip_sauce(&bytes);
    if art.is_empty() {
        return LoginArt {
            bytes: Vec::new(),
            warning: None,
        };
    }
    match check(art) {
        Ok(()) => LoginArt {
            bytes: art.to_vec(),
            warning: None,
        },
        Err(e) => fallback(e.to_string()),
    }
}
