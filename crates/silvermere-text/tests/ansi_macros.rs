//! The ANSI macros: each raw if-ANSI literal of the game resolves to the crate's constant,
//! and the colour table has the original's fifteen rows. The literals are copied below. Every
//! string macro and the colour-table rows are also committed as `tests/fixtures/ansi_facts.json`,
//! and the crate is checked against that.

use serde::Deserialize;
use silvermere_text::ansi;
use silvermere_text::colour::{CLRTBL, ColRow, Palette};

/// The if-ANSI literals of the game, with `\x1B` and `\b` unescaped.
mod raw {
    pub const COMMON: [(&str, &[u8]); 20] = [
        (
            "ANSI_CLRLINE",
            b"\x1B[[\x1B[79D\x1B[K| \x08 \x08 \x08 \x08]",
        ),
        (
            "ANSI_NORMAL",
            b"\x1B[[\x1B[0;37;40m| \x08 \x08 \x08 \x08 \x08]",
        ),
        ("ANSI_LTRED", b"\x1B[[\x1B[1;31m| \x08 \x08 \x08 \x08]"),
        (
            "ANSI_LTREDBLK",
            b"\x1B[[\x1B[1;31;40m| \x08 \x08 \x08 \x08 \x08]",
        ),
        ("ANSI_GREY", b"\x1B[[\x1B[0;37m| \x08 \x08 \x08 \x08]"),
        ("ANSI_BROWN", b"\x1B[[\x1B[0;33m| \x08 \x08 \x08 \x08]"),
        ("ANSI_WHITE", b"\x1B[[\x1B[1;37m| \x08 \x08 \x08 \x08]"),
        ("ANSI_GREEN", b"\x1B[[\x1B[0;32m| \x08 \x08 \x08 \x08]"),
        ("ANSI_CYAN", b"\x1B[[\x1B[0;36m| \x08 \x08 \x08 \x08]"),
        ("ANSI_YELLOW", b"\x1B[[\x1B[1;33m| \x08 \x08 \x08 \x08]"),
        ("ANSI_LTBLUE", b"\x1B[[\x1B[1;34m| \x08 \x08 \x08 \x08]"),
        ("ANSI_MAGENTA", b"\x1B[[\x1B[0;35m| \x08 \x08 \x08 \x08]"),
        ("ANSI_LTCYAN", b"\x1B[[\x1B[1;36m| \x08 \x08 \x08 \x08]"),
        ("ANSI_RED", b"\x1B[[\x1B[0;31m| \x08 \x08 \x08 \x08]"),
        ("ANSI_BOLD", b"\x1B[[\x1B[1m| \x08 \x08 \x08]"),
        ("ANSI_FGRED", b"\x1B[[\x1B[31m| \x08 \x08 \x08]"),
        ("ANSI_FGBROWN", b"\x1B[[\x1B[33m| \x08 \x08 \x08]"),
        ("ANSI_FGCYAN", b"\x1B[[\x1B[36m| \x08 \x08 \x08]"),
        ("ANSI_FGGREY", b"\x1B[[\x1B[37m| \x08 \x08 \x08]"),
        ("ANSI_FGGREEN", b"\x1B[[\x1B[32m| \x08 \x08 \x08]"),
    ];

    pub const TABLE_OWN: [(&str, &[u8]); 4] = [
        ("ANSI_LTMAGENTA", b"\x1B[[\x1B[1;35m| \x08 \x08 \x08 \x08]"),
        // two if-ANSI sequences back to back
        (
            "ANSI_BLKONWHT",
            b"\x1B[[\x1B[0;30m| \x08 \x08 \x08 \x08]\x1B[[\x1B[47m| \x08 \x08 \x08]",
        ),
        ("ANSI_BLKBACK", b"\x1B[[\x1B[40m| \x08 \x08 \x08]"),
        // empty: no colour at all
        ("NOCOLOR", b""),
    ];
}

/// The resolved escape every macro must give, by name.
fn expected(name: &str) -> &'static [u8] {
    match name {
        "ANSI_CLRLINE" => b"\x1B[79D\x1B[K",
        "ANSI_NORMAL" => b"\x1B[0;37;40m",
        "ANSI_LTRED" => b"\x1B[1;31m",
        "ANSI_LTREDBLK" => b"\x1B[1;31;40m",
        "ANSI_GREY" => b"\x1B[0;37m",
        "ANSI_BROWN" => b"\x1B[0;33m",
        "ANSI_WHITE" => b"\x1B[1;37m",
        "ANSI_GREEN" => b"\x1B[0;32m",
        "ANSI_CYAN" => b"\x1B[0;36m",
        "ANSI_YELLOW" => b"\x1B[1;33m",
        "ANSI_LTBLUE" => b"\x1B[1;34m",
        "ANSI_MAGENTA" => b"\x1B[0;35m",
        "ANSI_LTCYAN" => b"\x1B[1;36m",
        "ANSI_RED" => b"\x1B[0;31m",
        "ANSI_BOLD" => b"\x1B[1m",
        "ANSI_FGRED" => b"\x1B[31m",
        "ANSI_FGBROWN" => b"\x1B[33m",
        "ANSI_FGCYAN" => b"\x1B[36m",
        "ANSI_FGGREY" => b"\x1B[37m",
        "ANSI_FGGREEN" => b"\x1B[32m",
        "ANSI_LTMAGENTA" => b"\x1B[1;35m",
        "ANSI_BLKONWHT" => b"\x1B[0;30m\x1B[47m",
        "ANSI_BLKBACK" => b"\x1B[40m",
        "NOCOLOR" => b"",
        other => panic!("no expectation for {other}"),
    }
}

// the twenty common macros
#[test]
fn common_macros_resolve_to_their_escapes() {
    for (name, literal) in raw::COMMON {
        let resolved = ansi::resolve_if_ansi(literal);
        assert_eq!(resolved, expected(name), "{name}");
        assert_eq!(ansi::by_name(name), Some(expected(name)), "{name} constant");
    }
}

// the colour table's own four
#[test]
fn table_macros_resolve_to_their_escapes() {
    for (name, literal) in raw::TABLE_OWN {
        let resolved = ansi::resolve_if_ansi(literal);
        assert_eq!(resolved, expected(name), "{name}");
        assert_eq!(ansi::by_name(name), Some(expected(name)), "{name} constant");
    }
}

// PROMPT_SYSOP is plain ANSI, so resolving changes nothing
#[test]
fn prompt_sysop_is_not_if_ansi() {
    assert_eq!(
        ansi::resolve_if_ansi(ansi::PROMPT_SYSOP),
        ansi::PROMPT_SYSOP
    );
}

// fifteen rows, four palettes
#[test]
fn clrtbl_rows_by_name() {
    let rows: [[&str; 4]; 15] = [
        ["ANSI_GREEN", "ANSI_BROWN", "ANSI_GREEN", "ANSI_GREEN"],
        [
            "ANSI_MAGENTA",
            "ANSI_GREEN",
            "ANSI_LTMAGENTA",
            "ANSI_MAGENTA",
        ],
        ["ANSI_GREY", "ANSI_GREY", "ANSI_CYAN", "ANSI_CYAN"],
        ["NOCOLOR", "NOCOLOR", "ANSI_LTCYAN", "ANSI_LTCYAN"],
        ["ANSI_WHITE", "ANSI_WHITE", "ANSI_GREY", "ANSI_GREY"],
        ["ANSI_LTRED", "ANSI_LTRED", "ANSI_LTRED", "ANSI_LTRED"],
        ["ANSI_LTBLUE", "ANSI_LTBLUE", "ANSI_LTCYAN", "ANSI_LTCYAN"],
        ["ANSI_LTRED", "ANSI_LTRED", "ANSI_LTRED", "ANSI_LTRED"],
        ["ANSI_LTMAGENTA", "NOCOLOR", "NOCOLOR", "ANSI_LTMAGENTA"],
        ["ANSI_LTRED", "ANSI_RED", "ANSI_BLKONWHT", "ANSI_LTRED"],
        ["NOCOLOR", "NOCOLOR", "ANSI_BLKBACK", "NOCOLOR"],
        ["ANSI_GREEN", "ANSI_GREEN", "NOCOLOR", "ANSI_GREEN"],
        ["ANSI_GREEN", "ANSI_GREEN", "ANSI_GREEN", "ANSI_GREEN"],
        ["ANSI_GREY", "ANSI_GREY", "ANSI_GREY", "ANSI_LTCYAN"],
        ["ANSI_MAGENTA", "ANSI_CYAN", "ANSI_LTMAGENTA", "ANSI_LTCYAN"],
    ];
    assert_eq!(CLRTBL.len(), ColRow::ALL.len());
    for (r, row) in rows.iter().enumerate() {
        for (p, name) in row.iter().enumerate() {
            assert_eq!(CLRTBL[r][p], expected(name), "row {r} palette {p}");
            let palette = Palette::new(p as u8).unwrap();
            assert_eq!(ColRow::ALL[r].index() as usize, r);
            assert_eq!(
                silvermere_text::Colour::new(ColRow::ALL[r], palette).prefix(),
                expected(name)
            );
        }
    }
}

// --- the committed facts -------------------------------------------------------------------

const FACTS_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/ansi_facts.json"
);

/// The committed macro and colour-table facts.
#[derive(Debug, Deserialize)]
struct Facts {
    /// Every string macro, ANSI or not, in a fixed order.
    macros: Vec<Macro>,
    /// The macro names of each populated colour-table row.
    clrtbl: Vec<Vec<String>>,
    /// The highest colour-table row in use.
    maxclr: u32,
}

#[derive(Debug, Deserialize)]
struct Macro {
    name: String,
    /// The unescaped literal, one char per byte (Latin-1), cut at the first NUL.
    text: String,
}

impl Macro {
    fn bytes(&self) -> Vec<u8> {
        self.text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).expect("one byte per char"))
            .collect()
    }
}

fn facts() -> Facts {
    let text = std::fs::read_to_string(FACTS_PATH).unwrap_or_else(|e| panic!("{FACTS_PATH}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{FACTS_PATH}: {e}"))
}

#[test]
fn every_macro_matches_the_facts() {
    let mut seen = std::collections::BTreeSet::new();
    for m in facts().macros {
        let (name, literal) = (&m.name, m.bytes());
        let crate_name = if name == "ANSI_DKGRAY" {
            "ANSI_DKGREY"
        } else {
            name.as_str()
        };
        let Some(ours) = ansi::by_name(crate_name) else {
            assert!(
                !name.starts_with("ANSI_") && name != "NOCOLOR",
                "{name} has no constant in silvermere_text::ansi"
            );
            continue;
        };
        assert_eq!(ansi::resolve_if_ansi(&literal), ours, "{name}");
        seen.insert(crate_name.to_string());
    }
    let all: std::collections::BTreeSet<String> =
        ansi::BY_NAME.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(seen, all, "every constant was checked against the facts");
}

#[test]
fn clrtbl_matches_the_facts() {
    let facts = facts();
    let rows = &facts.clrtbl;
    assert_eq!(rows.len(), 15, "fifteen populated rows");
    for (r, row) in rows.iter().enumerate() {
        assert_eq!(row.len(), 4, "row {r} has four palettes");
        for (p, name) in row.iter().enumerate() {
            let want =
                ansi::by_name(name).unwrap_or_else(|| panic!("row {r}: unknown macro {name}"));
            assert_eq!(CLRTBL[r][p], want, "row {r} palette {p} ({name})");
        }
    }
    assert_eq!(facts.maxclr, 14, "maxclr");
    assert_eq!(silvermere_text::MAXCLR, 14);
}
