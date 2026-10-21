//! The long catalogue screens whose text is in the package's `WCCMMUD.DLL` (T-138): the main
//! menu's About, Information and Counts screens, the SYSOP help and menu, and a few long
//! notices. The repository keeps only [`TABLE`] (`dll_text.tsv`): for each catalogue key, the
//! byte ranges of the DLL its text is made of, the SHA-256 of those bytes and the ANSI macros
//! between them.
//!
//! [`extract`] reads the ranges out of a DLL, checks each key's hash and writes the texts as
//! [`OUTPUT_NAME`] in the assets directory, where silvermere-text's `package` module reads
//! them. The format is plain: a header line, then per key a `KEY LENGTH` line, that many bytes
//! of catalogue template (CP437, `\r` line ends, `{{` and `}}` for braces) and a line feed.
//!
//! This file names no other module of its crate: silvermere-testkit includes it as its own
//! module (`#[path]`), so the table lives here alone.

use sha2::{Digest, Sha256};

/// The table, `dll_text.tsv`.
pub const TABLE: &str = include_str!("dll_text.tsv");

/// The package file the ranges are read from.
pub const DLL_NAME: &str = "WCCMMUD.DLL";

/// The file [`extract`]'s output is written to, in the assets directory.
pub const OUTPUT_NAME: &str = "DLLTEXT.TXT";

/// The first line of [`OUTPUT_NAME`].
pub const HEADER: &str = "silvermere dll text 1";

/// One part of a key's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part<'a> {
    /// `length` bytes of the DLL from `offset`.
    Dll { offset: usize, length: usize },
    /// Template text kept as written (an ANSI macro).
    Template(&'a str),
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    /// The catalogue key.
    pub key: &'a str,
    /// The SHA-256 of the DLL bytes of the parts, in order, lowercase hex.
    pub sha256: &'a str,
    /// The parts in order.
    pub parts: Vec<Part<'a>>,
}

/// Why the texts could not be taken from a DLL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DllTextError {
    /// A row of the table does not parse.
    Table { line: usize, problem: String },
    /// A key's range runs past the end of the DLL.
    Short { key: String },
    /// A key's bytes do not have the table's hash: another build of the DLL.
    Mismatch { key: String, got: String },
}

impl std::fmt::Display for DllTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DllTextError::Table { line, problem } => {
                write!(f, "dll_text.tsv line {line}: {problem}")
            }
            DllTextError::Short { key } => write!(
                f,
                "{DLL_NAME}: the text of `{key}` runs past the end of the file (not the 1.11p DLL)"
            ),
            DllTextError::Mismatch { key, got } => write!(
                f,
                "{DLL_NAME}: the text of `{key}` does not match its SHA-256 (got {got}; not the 1.11p DLL)"
            ),
        }
    }
}

impl std::error::Error for DllTextError {}

/// The rows of [`TABLE`].
pub fn rows() -> Result<Vec<Row<'static>>, DllTextError> {
    parse_table(TABLE)
}

/// Parses a table in the format of [`TABLE`].
pub fn parse_table(table: &str) -> Result<Vec<Row<'_>>, DllTextError> {
    let mut rows = Vec::new();
    for (i, line) in table.lines().enumerate() {
        let problem = |p: &str| DllTextError::Table {
            line: i + 1,
            problem: p.to_string(),
        };
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(key), Some(sha256), Some(parts), None) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            return Err(problem("not three tab-separated columns"));
        };
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(problem("the hash is not 64 hex digits"));
        }
        let mut list = Vec::new();
        for part in parts.split(' ') {
            if part.starts_with('{') {
                list.push(Part::Template(part));
                continue;
            }
            let (offset, length) = part
                .split_once('+')
                .and_then(|(o, l)| Some((o.parse().ok()?, l.parse().ok()?)))
                .ok_or_else(|| problem(&format!("`{part}` is not OFFSET+LENGTH or a macro")))?;
            list.push(Part::Dll { offset, length });
        }
        rows.push(Row {
            key,
            sha256,
            parts: list,
        });
    }
    Ok(rows)
}

/// The text of every key of the table from `dll`, as the bytes of [`OUTPUT_NAME`].
pub fn extract(dll: &[u8]) -> Result<Vec<u8>, DllTextError> {
    let mut out = format!("{HEADER}\n").into_bytes();
    for row in rows()? {
        let mut hash = Sha256::new();
        let mut text = Vec::new();
        for part in &row.parts {
            match *part {
                Part::Dll { offset, length } => {
                    let bytes = offset
                        .checked_add(length)
                        .and_then(|end| dll.get(offset..end))
                        .ok_or_else(|| DllTextError::Short {
                            key: row.key.to_string(),
                        })?;
                    hash.update(bytes);
                    for &b in bytes {
                        text.push(b);
                        if b == b'{' || b == b'}' {
                            text.push(b);
                        }
                    }
                }
                Part::Template(t) => text.extend_from_slice(t.as_bytes()),
            }
        }
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != row.sha256 {
            return Err(DllTextError::Mismatch {
                key: row.key.to_string(),
                got,
            });
        }
        out.extend_from_slice(format!("{} {}\n", row.key, text.len()).as_bytes());
        out.extend_from_slice(&text);
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses() {
        let rows = rows().unwrap();
        assert!(rows.len() >= 20);
        let mut keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), rows.len(), "a key is listed twice");
        assert!(parse_table("a\tb\n").is_err());
        assert!(parse_table(&format!("a\t{}\tx+1\n", "0".repeat(64))).is_err());
    }

    #[test]
    fn a_wrong_dll_names_the_key() {
        let first = rows().unwrap()[0].key;
        match extract(b"too short") {
            Err(DllTextError::Short { key }) => assert_eq!(key, first),
            other => panic!("{other:?}"),
        }
        let zeros = vec![0u8; 2 << 20];
        let err = extract(&zeros).unwrap_err();
        assert!(matches!(&err, DllTextError::Mismatch { key, .. } if key == first));
        assert!(err.to_string().contains(first));
    }
}
