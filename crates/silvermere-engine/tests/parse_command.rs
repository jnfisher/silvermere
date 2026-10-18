//! `parse_command` against the generated oracle (T-024): every letter sequence in
//! `tests/fixtures/command_words.json` (from `tools/parsegen/parsegen.py`)
//! maps to the code of the command that word runs on the original, in upper and lower
//! case, and every `CMD_` code but `CMD_NONE` is reachable by some word.

use std::collections::BTreeSet;

use serde::Deserialize;
use silvermere_engine::commands::table::{CODES_WITHOUT_WORD, TABLE};
use silvermere_engine::commands::{Cmd, parse_command};

#[derive(Deserialize)]
struct Fixture {
    generator: String,
    codes_reachable: Vec<String>,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    word: String,
    code: String,
    kind: String,
}

fn fixture() -> Fixture {
    let f: Fixture = serde_json::from_str(include_str!("fixtures/command_words.json"))
        .expect("command_words.json parses");
    assert_eq!(f.generator, "tools/parsegen/parsegen.py");
    f
}

fn code_by_name(name: &str) -> Cmd {
    Cmd::ALL
        .iter()
        .copied()
        .find(|c| c.code_name() == Some(name))
        .unwrap_or_else(|| panic!("unknown code {name}"))
}

fn expected(e: &Entry) -> Option<Cmd> {
    match e.code.as_str() {
        "CMD_NONE" => None,
        name => Some(code_by_name(name)),
    }
}

// Every word in the fixture parses to the code of the command it runs on the original.
#[test]
fn every_fixture_entry_maps_to_its_code() {
    let f = fixture();
    assert!(
        f.entries.len() > 1000,
        "the oracle has every reachable sequence"
    );
    let mut failures = Vec::new();
    for e in &f.entries {
        let want = expected(e);
        let got = parse_command(&e.word);
        if got != want {
            failures.push(format!(
                "{:?} ({}): want {:?}, got {:?}",
                e.word, e.kind, want, got
            ));
        }
        // every letter is upper-cased: the lower-case spelling gives the same code.
        let lower = e.word.to_ascii_lowercase();
        if parse_command(&lower) != want {
            failures.push(format!("{lower:?} (lower): want {want:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// 138 codes; every one but CMD_NONE has a word.
#[test]
fn every_code_is_reachable() {
    let f = fixture();
    assert_eq!(Cmd::ALL.len(), 138);
    let reachable: BTreeSet<u16> = f
        .entries
        .iter()
        .filter_map(expected)
        .map(Cmd::code)
        .collect();
    let from_header: BTreeSet<u16> = f
        .codes_reachable
        .iter()
        .filter(|n| n.as_str() != "CMD_NONE")
        .map(|n| code_by_name(n).code())
        .collect();
    assert_eq!(reachable, from_header);
    let without: BTreeSet<u16> = CODES_WITHOUT_WORD.iter().map(|c| c.code()).collect();
    let all: BTreeSet<u16> = Cmd::ALL
        .iter()
        .copied()
        .filter(|c| *c != Cmd::None)
        .map(Cmd::code)
        .collect();
    assert_eq!(reachable.len(), 138 - 1 - without.len());
    assert_eq!(&all - &without, reachable);
}

// Every table entry is itself accepted at its minimum and at full length, so a typo in
// the table that the oracle happens not to cover still fails here.
#[test]
fn table_entries_accept_themselves() {
    for e in TABLE {
        let min = &e.word[..usize::from(e.min)];
        assert_eq!(parse_command(min), Some(e.code), "{}", e.word);
        assert_eq!(parse_command(e.word), Some(e.code), "{}", e.word);
        assert!(
            e.min >= 1 && usize::from(e.min) <= e.word.len(),
            "{}",
            e.word
        );
    }
}

// The first name is checked as a command word.
#[test]
fn single_letters_and_punctuation() {
    assert_eq!(parse_command("a"), Some(Cmd::Attack));
    assert_eq!(parse_command("?"), Some(Cmd::Help));
    assert_eq!(parse_command("?x"), None);
    assert_eq!(parse_command("'hello"), Some(Cmd::Broadcast));
    assert_eq!(parse_command("-hello"), Some(Cmd::Broadcast));
    assert_eq!(parse_command(">bob"), Some(Cmd::DirectMsg));
    assert_eq!(parse_command("/bob"), Some(Cmd::Whisper));
    assert_eq!(parse_command("who"), Some(Cmd::Scan));
    assert_eq!(parse_command("x"), None);
    for (w, c) in [
        ("n", Cmd::North),
        ("s", Cmd::South),
        ("e", Cmd::East),
        ("w", Cmd::West),
        ("u", Cmd::Up),
        ("d", Cmd::Down),
        ("ne", Cmd::NorthEast),
        ("nw", Cmd::NorthWest),
        ("se", Cmd::SouthEast),
        ("sw", Cmd::SouthWest),
    ] {
        assert_eq!(parse_command(w), Some(c), "{w}");
    }
}
