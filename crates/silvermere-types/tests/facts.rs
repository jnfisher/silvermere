//! The committed facts the layout, code and name-table tests check silvermere against.
//!
//! `tests/fixtures/rules_facts.json` holds the stored codes and array sizes, the byte layout of
//! every record, the record length of each data file, the fixed name tables and the reverse
//! direction table, so every test runs from the committed snapshot.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::Deserialize;

/// The committed fixture.
pub const FACTS_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/rules_facts.json"
);

/// Everything the tests take from the committed facts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Facts {
    /// The named codes and sizes of the records and their fields, in order.
    pub constants: Vec<Define>,
    /// The named codes of the command layer (status line modes, search scopes, ...), in order.
    pub command_constants: Vec<Define>,
    /// The members of each checked record.
    pub structs: BTreeMap<String, Vec<Member>>,
    /// The record length the game opens each data file with.
    pub record_lengths: Vec<RecordLength>,
    pub string_tables: Vec<StringTable>,
    /// The opposite of each direction, as `DIR_` constant names in direction order.
    pub reverse_directions: Vec<String>,
}

impl Facts {
    /// The record constants by name.
    pub fn consts(&self) -> BTreeMap<&str, i64> {
        by_name(&self.constants)
    }

    /// The command constants by name.
    pub fn command_consts(&self) -> BTreeMap<&str, i64> {
        by_name(&self.command_constants)
    }
}

fn by_name(defines: &[Define]) -> BTreeMap<&str, i64> {
    defines.iter().map(|d| (d.name.as_str(), d.value)).collect()
}

/// The record length of one data file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RecordLength {
    pub file: String,
    pub record_len: usize,
}

/// A fixed name table, named by its Rust static.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StringTable {
    pub name: String,
    /// The declared size, as a number or a constant name (`10`, `NDIRS`).
    pub size: String,
    /// The entries in order; `None` is the end marker.
    pub entries: Vec<Option<String>>,
}

/// The committed facts.
pub fn facts() -> Facts {
    let text = std::fs::read_to_string(FACTS_PATH).unwrap_or_else(|e| panic!("{FACTS_PATH}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{FACTS_PATH}: {e}"))
}

/// One named integer constant.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Define {
    pub name: String,
    pub value: i64,
}

impl Define {
    /// The family prefix: `PF` for `PF_NOGOSSIP`, `ST2` for `ST2_HELD`.
    pub fn family(&self) -> &str {
        self.name.split('_').next().unwrap_or("")
    }
}

/// One member of a record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Member {
    pub name: String,
    /// Byte offset in the record.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// Size of one element.
    pub elem: usize,
}
