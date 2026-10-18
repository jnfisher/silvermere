//! Text preparation shared by the client and the classifier, and the T-005 capture format.
//!
//! `plain` does what `bridge.plain` does and `raw_to_lines` what `patterns.raw_to_lines` does
//! (`tools/differential/vendor/play/bridge.py`, `tools/differential/vendor/play/patterns.py`), so the Rust
//! test client and the Python harness read the same bytes the same way.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

/// `ANSI_CLRLINE`: starts nearly every message the game sends.
pub const CLRLINE: &str = "\x1b[79D\x1b[K";

static ANSI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-Z0-9]|\x1b.").expect("ansi"));
static BACKSPACE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("[^\n\x08]\x08").expect("backspace"));
static NEWLINES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new("\n{3,}").expect("newlines"));
static IAC_RE: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(r"(?s-u)\xff[\xfb-\xfe].|\xff\xfa.*?\xff\xf0|\xff[\xf0-\xfa]")
        .expect("iac")
});

/// Removes ANSI escape sequences.
pub fn strip_ansi(text: &str) -> String {
    ANSI_RE.replace_all(text, "").into_owned()
}

fn apply_backspaces(mut text: String) -> String {
    loop {
        let next = BACKSPACE_RE.replace_all(&text, "");
        if next == text {
            break;
        }
        text = next.into_owned();
    }
    text.replace('\x08', "")
}

/// As the harness's `bridge.plain`: ANSI and the spinner's backspace noise removed, line ends
/// normalised to LF, runs of three or more blank lines folded to one.
pub fn plain(text: &str) -> String {
    let text = apply_backspaces(strip_ansi(text));
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    NEWLINES_RE.replace_all(&text, "\n\n").into_owned()
}

/// As the harness's `patterns.raw_to_lines`: CLRLINE and `\x01` end a line, other ANSI is dropped,
/// backspaces are applied, the bell is dropped, CR LF and bare CR end a line.
pub fn raw_to_lines(data: &[u8]) -> Vec<String> {
    let stripped = IAC_RE.replace_all(data, &b""[..]);
    let text = silvermere_net::cp437::decode(&stripped);
    text_to_lines(&text)
}

/// [`raw_to_lines`] for text that is already decoded (CP437 as one char per byte).
pub fn text_to_lines(text: &str) -> Vec<String> {
    let text = text.replace(CLRLINE, "\n").replace('\x01', "\n");
    let text = apply_backspaces(strip_ansi(&text));
    let text = text.replace('\x07', "");
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.split('\n').map(str::to_owned).collect()
}

/// As the harness's `patterns.plain_to_lines`: `bridge.plain` output (or a transcript) -> lines.
pub fn plain_to_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\x07', "")
        .split('\n')
        .map(str::to_owned)
        .collect()
}

/// `bridge.unescape`: `\e \t \r \n` in a script's `send`.
pub fn unescape(s: &str) -> String {
    s.replace("\\e", "\x1b")
        .replace("\\t", "\t")
        .replace("\\r", "\r")
        .replace("\\n", "\n")
}

/// Python's `repr(str)` for the characters scripts use (the `>>> 'cmd'` markers in
/// `plain.txt` are written with it).
pub fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Errors reading a capture directory.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

fn read(path: &Path) -> Result<Vec<u8>, CaptureError> {
    std::fs::read(path).map_err(|source| CaptureError::Io {
        path: path.to_owned(),
        source,
    })
}

/// A `mask` entry of `script.json`: a regex over the decoded text to blank before a byte
/// comparison (RNG fields, the build stamp, direction-name letter slots, clocks).
#[derive(Debug, Clone)]
pub struct ScriptMask {
    pub regex: String,
    pub why: String,
}

/// One T-005 scenario: `script.json`, `raw.bin`, `plain.txt`, `README.md`.
#[derive(Debug, Clone)]
pub struct Capture {
    pub name: String,
    pub dir: PathBuf,
    pub script: Value,
    pub raw: Vec<u8>,
    /// Lines of `plain.txt` including the `>>> 'cmd'` markers.
    pub plain: Vec<String>,
    pub readme: String,
}

impl Capture {
    /// Loads one scenario directory.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, CaptureError> {
        let dir = dir.as_ref();
        let script_path = dir.join("script.json");
        let script =
            serde_json::from_slice(&read(&script_path)?).map_err(|source| CaptureError::Json {
                path: script_path,
                source,
            })?;
        let text = |file: &str| -> Result<String, CaptureError> {
            Ok(String::from_utf8_lossy(&read(&dir.join(file))?).into_owned())
        };
        Ok(Self {
            name: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            dir: dir.to_owned(),
            script,
            raw: read(&dir.join("raw.bin"))?,
            plain: text("plain.txt")?.split('\n').map(str::to_owned).collect(),
            readme: text("README.md")?,
        })
    }

    /// Every scenario under `root` (directories holding a `script.json`), sorted by name.
    /// An absent `root` gives an empty list.
    pub fn all(root: impl AsRef<Path>) -> Result<Vec<Self>, CaptureError> {
        let Ok(entries) = std::fs::read_dir(root.as_ref()) else {
            return Ok(Vec::new());
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("script.json").is_file())
            .collect();
        dirs.sort();
        dirs.iter().map(Self::load).collect()
    }

    /// `(raw offset, text sent)` of every send step that ran, as `capture.sends`.
    pub fn sends(&self) -> Vec<(usize, String)> {
        let steps = self.script["steps"].as_array().cloned().unwrap_or_default();
        steps
            .iter()
            .filter(|st| st["op"].as_str().unwrap_or("send") == "send")
            .filter_map(|st| {
                let result = &st["result"];
                let offset = usize::try_from(result["offset"].as_u64()?).ok()?;
                let text = result["sent"].as_str().map_or_else(
                    || unescape(st["send"].as_str().unwrap_or_default()),
                    str::to_owned,
                );
                Some((offset, text))
            })
            .collect()
    }

    /// `plain.txt` minus the `>>>` markers (the k-th marker line of each send, in order)
    /// and how many markers were found: `capture.strip_markers`.
    pub fn plain_without_markers(&self) -> (Vec<String>, usize) {
        let want: Vec<String> = self
            .sends()
            .iter()
            .map(|(_, t)| format!(">>> {}", py_repr(t)))
            .collect();
        let mut found = 0;
        let mut out = Vec::with_capacity(self.plain.len());
        for line in &self.plain {
            if found < want.len() && *line == want[found] {
                found += 1;
            } else {
                out.push(line.clone());
            }
        }
        (out, found)
    }

    /// `raw_to_lines(raw.bin)`.
    pub fn raw_lines(&self) -> Vec<String> {
        raw_to_lines(&self.raw)
    }

    /// The `mask` list of `script.json`.
    pub fn script_masks(&self) -> Vec<ScriptMask> {
        self.script["mask"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some(ScriptMask {
                            regex: m["regex"].as_str()?.to_owned(),
                            why: m["why"].as_str().unwrap_or_default().to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The lines the README lists as `unknown` to the registry: `(with game data, extra
    /// lines without it)` from the two code blocks under "## Unclassified lines".
    pub fn listed_unknown(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        let Some((_, section)) = self.readme.split_once("## Unclassified lines") else {
            return Default::default();
        };
        let mut blocks = Vec::new();
        let mut rest = section;
        while let Some((_, after)) = rest.split_once("```\n") {
            let Some((body, tail)) = after.split_once("```") else {
                break;
            };
            blocks.push(
                body.split('\n')
                    .filter(|l| !l.is_empty() && *l != "(none)")
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>(),
            );
            rest = tail;
        }
        let mut blocks = blocks.into_iter();
        (
            blocks.next().unwrap_or_default(),
            blocks.next().unwrap_or_default(),
        )
    }
}

/// Blanks every match of the script's mask regexes in `text` with `<masked>`. Patterns
/// the `regex` crate cannot compile fall back to `fancy-regex` (see `patterns::Rx`).
pub fn apply_script_masks(text: &str, masks: &[ScriptMask]) -> String {
    let mut out = text.to_owned();
    for m in masks {
        if let Ok(rx) = crate::patterns::Rx::new(&m.regex) {
            out = rx.replace_all(&out, "<masked>");
        }
    }
    out
}

/// The workspace `tests/captures/` directory.
pub fn captures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/captures")
}
