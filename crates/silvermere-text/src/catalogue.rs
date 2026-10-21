//! The message catalogue format, `messages/catalogue.toml`.
//!
//! ```toml
//! [messages.exp_gain]
//! text = "{@ANSI_CLRLINE}You gain {exp} experience.\r"   # the bytes the original printed
//! wrap = false                                           # true: 79-column wrap (btutsw)
//! pattern = "exp_gain"                                   # name in tests/fixtures/patterns.json
//! args = { exp = "i64" }                                 # placeholder types (default String)
//! example = { exp = "13" }                               # values for the snapshot test
//! ```
//!
//! In `text`, `{name}` is an argument, `{@NAME}` a macro: an `ANSI_*` string (resolved
//! at build time) or a `COL_*` colour row (resolved at render time with the player's
//! palette). `{{` and `}}` are literal braces. printf conversions (`%d`,
//! `%ld`, `%s`, widths) are translated when an entry is written (by hand here, by
//! T-011's converter for the `.MSG` files); nothing is interpreted at render time.
//!
//! `build.rs` includes this module by path and generates the `Msg` enum from the
//! catalogue, so a key typo is a compile error. The module therefore uses only
//! `serde`, `toml`, `thiserror` and its sibling modules `ansi` and `colour`.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::ansi;
use super::colour::ColRow;

/// A parsed, validated catalogue.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    /// Entries by key, in key order.
    pub messages: BTreeMap<String, Entry>,
}

/// One message.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The template: the text the original printed, with `\r` line ends as written.
    pub text: String,
    /// Whether the original printed this with the host's 79-column word wrap on (`btutsw`).
    #[serde(default)]
    pub wrap: bool,
    /// The pattern registry name whose regex the rendered text matches.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Argument types; an argument not listed is a `String`.
    #[serde(default)]
    pub args: BTreeMap<String, ArgType>,
    /// Example argument values (as text) for `Msg::examples()`.
    #[serde(default)]
    pub example: BTreeMap<String, String>,
    /// Anything the reader should know.
    #[serde(default)]
    pub note: Option<String>,
}

/// The Rust type of an argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ArgType {
    String,
    #[serde(rename = "i64")]
    I64,
    #[serde(rename = "i32")]
    I32,
    #[serde(rename = "u64")]
    U64,
    #[serde(rename = "u32")]
    U32,
    #[serde(rename = "u16")]
    U16,
    #[serde(rename = "u8")]
    U8,
    #[serde(rename = "char")]
    Char,
}

impl ArgType {
    /// The type's name in generated code.
    pub const fn rust_name(self) -> &'static str {
        match self {
            ArgType::String => "String",
            ArgType::I64 => "i64",
            ArgType::I32 => "i32",
            ArgType::U64 => "u64",
            ArgType::U32 => "u32",
            ArgType::U16 => "u16",
            ArgType::U8 => "u8",
            ArgType::Char => "char",
        }
    }

    /// Whether `value` is a valid example for the type.
    pub fn accepts(self, value: &str) -> bool {
        match self {
            ArgType::String => true,
            ArgType::I64 => value.parse::<i64>().is_ok(),
            ArgType::I32 => value.parse::<i32>().is_ok(),
            ArgType::U64 => value.parse::<u64>().is_ok(),
            ArgType::U32 => value.parse::<u32>().is_ok(),
            ArgType::U16 => value.parse::<u16>().is_ok(),
            ArgType::U8 => value.parse::<u8>().is_ok(),
            ArgType::Char => value.chars().count() == 1,
        }
    }
}

/// One piece of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePiece {
    /// Literal text.
    Text(String),
    /// `{name}`: an argument.
    Arg(String),
    /// `{@ANSI_*}`: an ANSI string, by its macro name.
    Ansi(String),
    /// `{@COL_*}`: a colour-table row, by its `COL_*` name.
    Colour(ColRow),
}

/// Why a catalogue is rejected.
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("catalogue is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("message `{key}`: {problem}")]
    Entry { key: String, problem: String },
}

fn entry_error(key: &str, problem: impl Into<String>) -> CatalogueError {
    CatalogueError::Entry {
        key: key.to_string(),
        problem: problem.into(),
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Splits a template into pieces. `key` names the entry in errors.
pub fn parse_template(key: &str, text: &str) -> Result<Vec<TemplatePiece>, CatalogueError> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        match c {
            '{' if rest.starts_with("{{") => {
                literal.push('{');
                rest = &rest[2..];
            }
            '}' if rest.starts_with("}}") => {
                literal.push('}');
                rest = &rest[2..];
            }
            '}' => return Err(entry_error(key, "unmatched `}` (write `}}` for a brace)")),
            '{' => {
                let Some(end) = rest.find('}') else {
                    return Err(entry_error(key, "unterminated `{`"));
                };
                let name = &rest[1..end];
                if !literal.is_empty() {
                    pieces.push(TemplatePiece::Text(std::mem::take(&mut literal)));
                }
                if let Some(macro_name) = name.strip_prefix('@') {
                    if ansi::by_name(macro_name).is_some() {
                        pieces.push(TemplatePiece::Ansi(macro_name.to_string()));
                    } else if let Some(row) = ColRow::from_macro_name(macro_name) {
                        pieces.push(TemplatePiece::Colour(row));
                    } else {
                        return Err(entry_error(
                            key,
                            format!("unknown macro `{{@{macro_name}}}`"),
                        ));
                    }
                } else if is_identifier(name) {
                    pieces.push(TemplatePiece::Arg(name.to_string()));
                } else {
                    return Err(entry_error(
                        key,
                        format!("`{{{name}}}` is not an argument name"),
                    ));
                }
                rest = &rest[end + 1..];
            }
            _ => {
                literal.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !literal.is_empty() {
        pieces.push(TemplatePiece::Text(literal));
    }
    Ok(pieces)
}

impl Entry {
    /// The template's pieces.
    pub fn pieces(&self, key: &str) -> Result<Vec<TemplatePiece>, CatalogueError> {
        parse_template(key, &self.text)
    }

    /// The argument names in order of first appearance, each with its type.
    pub fn arg_list(&self, key: &str) -> Result<Vec<(String, ArgType)>, CatalogueError> {
        let mut args: Vec<(String, ArgType)> = Vec::new();
        for piece in self.pieces(key)? {
            if let TemplatePiece::Arg(name) = piece
                && !args.iter().any(|(n, _)| *n == name)
            {
                let ty = self.args.get(&name).copied().unwrap_or(ArgType::String);
                args.push((name, ty));
            }
        }
        Ok(args)
    }

    fn validate(&self, key: &str) -> Result<(), CatalogueError> {
        let args = self.arg_list(key)?;
        for name in self.args.keys() {
            if !args.iter().any(|(n, _)| n == name) {
                return Err(entry_error(
                    key,
                    format!("`args` names `{name}`, which the text does not use"),
                ));
            }
        }
        for (name, value) in &self.example {
            let Some((_, ty)) = args.iter().find(|(n, _)| n == name) else {
                return Err(entry_error(
                    key,
                    format!("`example` names `{name}`, which the text does not use"),
                ));
            };
            if !ty.accepts(value) {
                return Err(entry_error(
                    key,
                    format!("example `{name} = {value:?}` is not a {}", ty.rust_name()),
                ));
            }
        }
        Ok(())
    }
}

/// Parses and validates a catalogue.
pub fn parse(toml_text: &str) -> Result<Catalogue, CatalogueError> {
    let catalogue: Catalogue = toml::from_str(toml_text)?;
    catalogue.validate()?;
    Ok(catalogue)
}

impl Catalogue {
    /// Checks every key and entry, as [`parse`] does.
    pub fn validate(&self) -> Result<(), CatalogueError> {
        for (key, entry) in &self.messages {
            if !is_identifier(key) {
                return Err(entry_error(key, "keys are snake_case identifiers"));
            }
            entry.validate(key)?;
        }
        Ok(())
    }
}

/// The words of the last help topic, the end marker.
pub const HELP_END_MARK: &str = "Help ?";

/// The arguments a help text may use: the say and yell characters, the two `%c` of the
/// help texts.
const HELP_ARGS: [&str; 2] = ["talk_char", "yell_char"];

/// The help topics of `WCCMMHLP.MSG` (read by [`crate::package`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpTopics {
    /// `HL1B0`, the overview `HELP` alone prints.
    pub overview: String,
    /// The topics in file order; the last is the `Help ?` end marker.
    pub topics: Vec<HelpTopic>,
}

/// One help topic: the topic string and the text printed for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpTopic {
    /// The message name of the topic string, `HL1T..`.
    pub key: String,
    /// The message name of the text, `HL1B..`.
    pub body_key: String,
    /// The topic string: the words a player's topic is matched against.
    pub words: String,
    /// The text, a catalogue template with `{talk_char}` and `{yell_char}`.
    pub text: String,
}

impl HelpTopics {
    /// Checks the topics: every text is a valid template using only `talk_char` and
    /// `yell_char`, every topic has words, the keys are unique and the last topic is the
    /// `Help ?` end marker.
    pub fn validated(self) -> Result<HelpTopics, CatalogueError> {
        let help = self;
        let check = |key: &str, text: &str| -> Result<(), CatalogueError> {
            for piece in parse_template(key, text)? {
                match piece {
                    TemplatePiece::Arg(name) if HELP_ARGS.contains(&name.as_str()) => {}
                    TemplatePiece::Text(_) | TemplatePiece::Ansi(_) => {}
                    other => {
                        return Err(entry_error(
                            key,
                            format!(
                                "help texts take only {{talk_char}} and {{yell_char}}, not {other:?}"
                            ),
                        ));
                    }
                }
            }
            Ok(())
        };
        check("overview", &help.overview)?;
        let mut seen = std::collections::BTreeSet::new();
        for topic in &help.topics {
            if topic.words.trim().is_empty() {
                return Err(entry_error(&topic.key, "empty topic words"));
            }
            if !seen.insert(topic.key.as_str()) {
                return Err(entry_error(&topic.key, "duplicate topic key"));
            }
            check(&topic.key, &topic.text)?;
        }
        match help.topics.last() {
            Some(last) if last.words == HELP_END_MARK => Ok(help),
            _ => Err(entry_error(
                "topic",
                format!("the last topic must be the end marker {HELP_END_MARK:?}"),
            )),
        }
    }
}

/// The `Msg` variant name for a key: `exp_gain` becomes `ExpGain`.
pub fn variant_name(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            let first = chars
                .next()
                .map(|c| c.to_ascii_uppercase())
                .unwrap_or_default();
            std::iter::once(first).chain(chars).collect::<String>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_pieces() {
        let p = parse_template("k", "{@ANSI_CLRLINE}{who} gains {{x}} {@COL_EXITS}!").unwrap();
        assert_eq!(
            p,
            [
                TemplatePiece::Ansi("ANSI_CLRLINE".into()),
                TemplatePiece::Arg("who".into()),
                TemplatePiece::Text(" gains {x} ".into()),
                TemplatePiece::Colour(ColRow::Exits),
                TemplatePiece::Text("!".into()),
            ]
        );
        assert!(parse_template("k", "{@NOPE}").is_err());
        assert!(parse_template("k", "{Bad}").is_err());
        assert!(parse_template("k", "a}").is_err());
        assert!(parse_template("k", "{a").is_err());
    }

    #[test]
    fn validation() {
        let good =
            "[messages.a_b]\ntext = \"{n}\"\nargs = { n = \"i32\" }\nexample = { n = \"4\" }\n";
        let c = parse(good).unwrap();
        assert_eq!(
            c.messages["a_b"].arg_list("a_b").unwrap(),
            [("n".to_string(), ArgType::I32)]
        );
        let bad_example = good.replace("\"4\"", "\"x\"");
        assert!(parse(&bad_example).is_err());
        let unused_arg = good.replace("{n}", "n");
        assert!(parse(&unused_arg).is_err());
        let bad_key = good.replace("a_b", "AB");
        assert!(parse(&bad_key).is_err());
        let unknown_field = format!("{good}warp = true\n");
        assert!(parse(&unknown_field).is_err());
    }

    #[test]
    fn variant_names() {
        assert_eq!(variant_name("exp_gain"), "ExpGain");
        assert_eq!(variant_name("clrline"), "Clrline");
        assert_eq!(variant_name("room_exits_none"), "RoomExitsNone");
    }
}
