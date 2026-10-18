//! [`parse_command`] over the table, and the typed line as words.
//!
//! [`parse_command`] takes the first word of a line (or, at character creation, the first name)
//! and gives the command's `CMD_` code, or `None` for `CMD_NONE`. [`CommandLine`] is the host's
//! word split: words at space boundaries, each with its offsets in the raw line so a handler can
//! take "the rest of the line" from a word.
//! [`eq_ignore_case`] and [`starts_with_ignore_case`] are the host's case-insensitive equality and
//! prefix tests with ASCII folding only.

use silvermere_types::enums::CommandCode as Cmd;

use super::table::{CommandWord, TABLE};

/// The code for one typed word, `None` when the word is no command. Case does not matter
/// (the word is upper-cased). An empty word is no command.
pub fn parse_command(word: &str) -> Option<Cmd> {
    if word.is_empty() {
        return None;
    }
    let upper = word.to_ascii_uppercase();
    // The one branch the table cannot express: after the full word the tree tests word[6] and then
    // word[8], never word[7].
    if let Some(tail) = upper.strip_prefix("BORROW") {
        return if tail.len() <= 2 {
            Some(Cmd::Borrow)
        } else {
            None
        };
    }
    TABLE.iter().find_map(|e| matches(e, &upper))
}

fn matches(e: &CommandWord, upper: &str) -> Option<Cmd> {
    if upper.len() < usize::from(e.min) {
        return None;
    }
    if e.open {
        return upper.starts_with(e.word).then_some(e.code);
    }
    e.word.starts_with(upper).then_some(e.code)
}

/// `a` equals `b`, ignoring ASCII case.
pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `word` begins with `prefix`, ignoring ASCII case. An empty prefix matches, as the
/// host's does.
pub fn starts_with_ignore_case(prefix: &str, word: &str) -> bool {
    word.len() >= prefix.len()
        && word.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// One word of the line with its byte offsets in [`CommandLine::raw`] (and
/// `margn[i]` as `end`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// The typed line as the host's `parsin()` split it: words separated by spaces, with the
/// raw line kept so `rstrin()` (the spaces put back) is a slice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandLine {
    pub raw: String,
    pub words: Vec<Word>,
}

impl CommandLine {
    /// Splits `line` at spaces (`parsin`: runs of spaces are one boundary, leading and
    /// trailing spaces give no word).
    pub fn parse(line: &str) -> Self {
        let mut words = Vec::new();
        let mut start = None;
        for (i, b) in line.bytes().enumerate() {
            match (b == b' ', start) {
                (false, None) => start = Some(i),
                (true, Some(s)) => {
                    words.push(Word {
                        text: line[s..i].to_string(),
                        start: s,
                        end: i,
                    });
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            words.push(Word {
                text: line[s..].to_string(),
                start: s,
                end: line.len(),
            });
        }
        Self {
            raw: line.to_string(),
            words,
        }
    }

    /// The number of words.
    pub fn argc(&self) -> usize {
        self.words.len()
    }

    /// Word `i`, or `""` past the end.
    pub fn arg(&self, i: usize) -> &str {
        self.words.get(i).map_or("", |w| w.text.as_str())
    }

    /// The line from word `i` to its end with the typed spacing (trailing blanks trimmed);
    /// `""` past the end.
    pub fn rest_from(&self, i: usize) -> &str {
        self.words.get(i).map_or("", |w| &self.raw[w.start..])
    }

    /// The first word, or `""`.
    pub fn first(&self) -> &str {
        self.arg(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // the abbreviation rule
    #[test]
    fn attack_prefixes() {
        for w in ["A", "at", "ATT", "atta", "ATTAC", "attack"] {
            assert_eq!(parse_command(w), Some(Cmd::Attack), "{w}");
        }
        assert_eq!(parse_command("ATX"), None);
        assert_eq!(parse_command("ATTACKS"), None);
        assert_eq!(parse_command(""), None);
    }

    // the BORROW word[8] test
    #[test]
    fn borrow_quirk() {
        assert_eq!(parse_command("BORROW"), Some(Cmd::Borrow));
        assert_eq!(parse_command("BORROWxy"), Some(Cmd::Borrow));
        assert_eq!(parse_command("BORROWxyz"), None);
        assert_eq!(parse_command("BO"), Some(Cmd::Borrow));
    }

    // Words and the rest of the line.
    #[test]
    fn command_line_words() {
        let l = CommandLine::parse("  say  hello   there ");
        assert_eq!(l.argc(), 3);
        assert_eq!(l.arg(0), "say");
        assert_eq!(l.arg(2), "there");
        assert_eq!(l.arg(3), "");
        assert_eq!(l.rest_from(1), "hello   there ");
        assert_eq!(l.words[1].start, 7);
        assert_eq!(l.words[1].end, 12);
        assert_eq!(CommandLine::parse("").argc(), 0);
    }

    #[test]
    fn host_string_tests() {
        assert!(eq_ignore_case("Break", "BREAK"));
        assert!(!eq_ignore_case("Break", "BREAKS"));
        assert!(starts_with_ignore_case("sys", "SYSOP"));
        assert!(starts_with_ignore_case("", "x"));
        assert!(!starts_with_ignore_case("sysop", "sys"));
    }
}
