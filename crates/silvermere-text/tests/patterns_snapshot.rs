//! Layer 2: every seeded catalogue entry, rendered with
//! its example arguments and stripped of ANSI, matches the pattern registry's regex for
//! it (`tests/fixtures/patterns.json`, by name) and, where the registry gives a literal
//! example, equals it byte for byte. The test walks the registry, so entries later cards
//! add (T-011 and the display cards) are covered as soon as they name a `pattern`.
//! An `insta` snapshot pins the exact bytes of every example.
//!
//! The fixture is the export of `tools/differential/vendor/play/patterns.py`:
//! `{"version", "counts", "static": [{"name", "cat", "rx", "example", "seen",
//! "prio", "groups"}], "tables", "join", "prompt", ...}`. Only `static` is read here.

use std::collections::BTreeMap;

use serde::Deserialize;
use silvermere_text::colour::Palette;
use silvermere_text::cp437;
use silvermere_text::msg::Msg;
use silvermere_text::render::render;
use silvermere_text::strip::strip_ansi;

#[derive(Deserialize)]
struct Registry {
    version: u32,
    #[serde(rename = "static")]
    patterns: Vec<Pattern>,
}

/// One static pattern; the export's other fields (`cat`, `src`, `seen`, `prio`,
/// `groups`) are not needed here.
#[derive(Deserialize)]
struct Pattern {
    name: String,
    rx: String,
    example: String,
}

fn registry() -> Registry {
    let registry: Registry =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json"))
            .expect("patterns.json parses");
    assert_eq!(registry.version, 1, "patterns.json export version");
    registry
}

/// The export keeps the weapon-verb placeholders of `patterns.py` unexpanded in `rx`
/// (`{HITV}`, `{HITV3}`, `{MISSV}`, `{MISSV3}`); `Registry.load_static` fills them from
/// the game data, or with these fallbacks when no data is loaded.
fn expand_placeholders(rx: &str) -> String {
    rx.replace("{HITV}", "[a-z]+(?: [a-z]+)?")
        .replace("{HITV3}", "[a-z]+(?: [a-z]+)?")
        .replace("{MISSV}", "[a-z]+ at")
        .replace("{MISSV3}", "[a-z]+ at")
}

/// The rendered message as the registry sees a screen: ANSI stripped, CP437 decoded,
/// cut into lines at `\r\n`, trailing blanks removed, empty lines dropped.
fn screen_lines(msg: &Msg) -> Vec<String> {
    let bytes = render(msg, Palette::default()).bytes;
    cp437::decode(&strip_ansi(&bytes))
        .split("\r\n")
        .map(|l| l.trim_end_matches([' ', '\t', '\u{7}']).to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn every_seeded_message_matches_its_registry_pattern() {
    // the moved entries render the package's text
    crate::package();
    let registry = registry();
    let mut by_pattern: BTreeMap<&str, Vec<&Msg>> = BTreeMap::new();
    let examples = Msg::examples();
    for msg in &examples {
        if let Some(name) = msg.pattern() {
            by_pattern.entry(name).or_default().push(msg);
        }
    }
    let mut checked = 0;
    let mut names_seen = Vec::new();
    for pattern in &registry.patterns {
        names_seen.push(pattern.name.as_str());
        let Some(msgs) = by_pattern.get(pattern.name.as_str()) else {
            continue;
        };
        let rx = expand_placeholders(&pattern.rx);
        let regex = regex::Regex::new(&format!("^(?:{rx})$"))
            .unwrap_or_else(|e| panic!("pattern {}: {e}", pattern.name));
        // every entry for the pattern matches its regex; where the registry gives a
        // literal example, one of them reproduces it byte for byte
        let mut equal_to_example = pattern.example.contains("...");
        let mut rendered = Vec::new();
        for msg in msgs {
            let lines = screen_lines(msg);
            assert!(
                lines.iter().any(|l| regex.is_match(l)),
                "{} ({}): no line of {lines:?} matches /{rx}/",
                msg.key(),
                pattern.name
            );
            equal_to_example |= lines.contains(&pattern.example);
            rendered.push((msg.key(), lines));
            checked += 1;
        }
        assert!(
            equal_to_example,
            "{}: no entry renders the registry example {:?}: {rendered:?}",
            pattern.name, pattern.example
        );
    }
    // every catalogue entry that names a pattern names one the registry has
    for (name, msgs) in &by_pattern {
        assert!(
            names_seen.contains(name),
            "{}: pattern {name} is not in patterns.json",
            msgs[0].key()
        );
    }
    assert!(checked >= 30, "only {checked} seeded entries checked");
}

#[test]
fn examples_cover_every_key_and_render_without_panics() {
    let examples = Msg::examples();
    assert_eq!(examples.len(), Msg::KEYS.len());
    for msg in &examples {
        let r = render(msg, Palette::default());
        assert_eq!(r.wraps, msg.wrap().is_some(), "{}", msg.key());
        assert!(
            !r.bytes.contains(&0xFF),
            "{}: 0xFF is the transport's to double",
            msg.key()
        );
        // every line end on the wire is CR LF
        for (i, w) in r.bytes.windows(2).enumerate() {
            if w[0] == b'\r' {
                assert_eq!(w[1], b'\n', "{}: bare CR at {i}", msg.key());
            }
            if w[1] == b'\n' {
                assert_eq!(w[0], b'\r', "{}: bare LF at {i}", msg.key());
            }
        }
    }
}

/// The rendered bytes of every example in every palette, as escaped text. The entries whose
/// text is the package's are left out (tests/package_text.rs pins their bytes by hash).
#[test]
fn rendered_examples_snapshot() {
    let mut out = String::new();
    for msg in Msg::examples() {
        if silvermere_text::package::MOVED_KEYS.contains(&msg.key()) {
            continue;
        }
        out.push_str(&format!("{}\n", msg.key()));
        for p in 0..Palette::COUNT {
            let r = render(&msg, Palette::new(p).unwrap());
            out.push_str(&format!("  palette {p}: {}\n", escape(&r.bytes)));
        }
    }
    insta::assert_snapshot!("rendered_examples", out);
}

fn escape(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|&b| std::ascii::escape_default(b))
        .map(char::from)
        .collect()
}
