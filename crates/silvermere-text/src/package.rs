//! The game's long text, read from the assets directory when the server starts (T-138).
//!
//! The help topics (`WCCMMHLP.MSG`), the text blocks of `WCCMMUD.MSG` and `WCCTEXT.MSG`, the
//! two character-creation forms (`EDITCHA1`, `EDTCHAN2` of `WCCTEXT.MSG`) and the long
//! catalogue screens of [`MOVED_KEYS`] are not compiled in: the import copies the three
//! message files into `<data_dir>/assets/` and takes the screens out of `WCCMMUD.DLL` into
//! [`DLL_TEXT_FILE`]. [`PackageText::load`] parses them, [`install`] makes them the
//! process's text, and the HELP command, the full-screen form and [`crate::render`] read them
//! through [`installed`]. Until they are installed, the moved catalogue entries render their
//! catalogue text, [`NOT_INSTALLED`], and HELP prints the same.
//!
//! The `.MSG` format: one entry per `KEY {value} TYPE ...`, `~}` a literal `}` and `~~` a
//! literal `~`, the prose between entries describing the next one. CRs are dropped and each LF
//! becomes the `\r` line end the catalogue uses (docs/text.md). A text block becomes a
//! catalogue template: `{` and `}` doubled, `%s` and `%c` the named arguments in order, `%%` a
//! `%`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::OnceLock;

use crate::catalogue::{Catalogue, Entry, HelpTopic, HelpTopics, TemplatePiece};
use crate::cp437;
use crate::editor_screen::EditorScreen;

/// The help file.
pub const HELP_FILE: &str = "WCCMMHLP.MSG";
/// The game's message file: the text blocks of [`WCCMMUD_TEXTS`].
pub const WCCMMUD_FILE: &str = "WCCMMUD.MSG";
/// The text file: the text blocks of [`WCCTEXT_TEXTS`] and the creation forms.
pub const WCCTEXT_FILE: &str = "WCCTEXT.MSG";
/// The long screens the import takes out of `WCCMMUD.DLL` (silvermere-import's `dll_text`).
pub const DLL_TEXT_FILE: &str = "DLLTEXT.TXT";
/// The first line of [`DLL_TEXT_FILE`].
const DLL_TEXT_HEADER: &str = "silvermere dll text 1";

/// What a moved text prints while the game's text is not installed: the catalogue text of
/// every key of [`MOVED_KEYS`], and HELP's answer.
pub const NOT_INSTALLED: &str = "\r(This text is not installed: run `silvermere import`.)\r";

/// The catalogue entries whose text comes from the package: the DLL's screens, then
/// `goodtext` (`WCCTEXT.MSG`). Their keys and arguments stay in `catalogue.toml`.
pub const MOVED_KEYS: &[&str] = &[
    "menu_about_ansi",
    "menu_about_plain",
    "menu_info_ansi",
    "menu_info_plain",
    "menu_counts_ansi",
    "menu_counts_plain",
    "sysop_help",
    "sysop_help_sysop",
    "sysop_menu",
    "stat_editor_preamble",
    "enter_stats_reset",
    "save_refused",
    "suicide_pw_notice",
    "sysop_god_syntax",
    "sysop_god_valid_options",
    "sysop_name_changed",
    "sysop_valid_edits",
    "sysop_xref_another",
    "race_change_done",
    "class_change_done",
    "cast_silly",
    "goodtext",
];

/// One text block of a message file as a catalogue entry: the `.MSG` key, the pattern
/// registry name, the argument names of its `%s`/`%c` in order, and example values.
pub struct TextBlock {
    pub key: &'static str,
    pub pattern: Option<&'static str>,
    pub args: &'static [&'static str],
    pub example: &'static [(&'static str, &'static str)],
}

/// The text blocks of `WCCMMUD.MSG`.
pub const WCCMMUD_TEXTS: &[TextBlock] = &[
    TextBlock {
        key: "USRLOGIN",
        pattern: Some("login_greeting"),
        args: &["version"],
        example: &[("version", "1.11p-WG")],
    },
    TextBlock {
        key: "USRLOGOF",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "USRFOLED",
        pattern: Some("enter_banner"),
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "USREXIT",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "LEAVEMNU",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "INACHUP",
        pattern: Some("inactive_hangup"),
        args: &[],
        example: &[],
    },
];

/// The text blocks of `WCCTEXT.MSG`.
pub const WCCTEXT_TEXTS: &[TextBlock] = &[
    TextBlock {
        key: "GOODTEXT",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "GOODPMPT",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "CHSRACE",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "CHSCLASS",
        pattern: None,
        args: &[],
        example: &[],
    },
    TextBlock {
        key: "MENUTEXT",
        pattern: Some("mud_title"),
        args: &["version"],
        example: &[("version", "1.11p-WG (Dec 30 2005 14:20:05)")],
    },
    TextBlock {
        key: "MENUPMPT",
        pattern: Some("mud_menu_prompt"),
        args: &[],
        example: &[],
    },
];

/// The arguments of the help texts' two `%c`: the say and yell characters.
const HELP_ARGS: [&str; 2] = ["talk_char", "yell_char"];

/// Why the text could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {problem}")]
pub struct PackageError {
    /// The file (or file and key).
    pub file: String,
    /// What is wrong.
    pub problem: String,
}

fn error(file: &str, problem: impl Into<String>) -> PackageError {
    PackageError {
        file: file.to_string(),
        problem: problem.into(),
    }
}

/// One `KEY {value} ...` entry of a `.MSG` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgEntry {
    /// The key.
    pub key: String,
    /// The value: unescaped, CRs dropped, LFs kept.
    pub value: String,
    /// The type letter after the value (`T` for a text block), empty for none.
    pub kind: String,
}

/// Whether `line` starts like an entry: `KEY`, blanks, `{`. Returns the index after the `{`.
fn entry_start(line: &[char]) -> Option<usize> {
    if !line.first()?.is_ascii_uppercase() {
        return None;
    }
    let mut i = 1;
    while i < line.len()
        && (line[i].is_ascii_uppercase() || line[i].is_ascii_digit() || line[i] == '_')
    {
        i += 1;
    }
    let key_end = i;
    while i < line.len() && (line[i] == ' ' || line[i] == '\t') {
        i += 1;
    }
    (line.get(i) == Some(&'{')).then_some(key_end)
}

/// CP437 bytes as text: ASCII and the C0 controls as themselves, the upper half as the
/// CP437 characters.
fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii() {
                char::from(b)
            } else {
                cp437::decode_byte(b)
            }
        })
        .collect()
}

/// Parses a `.MSG` file (CP437 bytes).
pub fn parse_msg(file: &str, data: &[u8]) -> Result<Vec<MsgEntry>, PackageError> {
    let text: Vec<char> = decode(data).chars().filter(|&c| c != '\r').collect();
    let mut entries = Vec::new();
    let mut pos = 0;
    loop {
        // the next line start at or after pos where an entry begins
        let mut start = pos;
        let found = loop {
            if start >= text.len() {
                break None;
            }
            if (start == 0 || text[start - 1] == '\n')
                && let Some(key_end) = entry_start(&text[start..])
            {
                break Some((start, start + key_end));
            }
            match text[start..].iter().position(|&c| c == '\n') {
                Some(nl) => start += nl + 1,
                None => break None,
            }
        };
        let Some((start, key_end)) = found else {
            break;
        };
        let key: String = text[start..key_end].iter().collect();
        let mut i = key_end;
        while text[i] != '{' {
            i += 1;
        }
        i += 1;
        let mut value = String::new();
        loop {
            match text.get(i) {
                None => return Err(error(file, format!("{key}: unterminated value"))),
                Some('~') if matches!(text.get(i + 1), Some('}' | '~')) => {
                    value.push(text[i + 1]);
                    i += 2;
                }
                Some('}') => break,
                Some(&c) => {
                    value.push(c);
                    i += 1;
                }
            }
        }
        let eol = text[i..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(text.len(), |n| i + n);
        let spec: String = text[i + 1..eol].iter().collect();
        let kind = spec.split_whitespace().next().unwrap_or("").to_string();
        entries.push(MsgEntry { key, value, kind });
        pos = eol;
    }
    Ok(entries)
}

/// A `.MSG` value as a catalogue template: LF to `\r`, braces doubled, `%s`/`%c` to the
/// next of `args`, `%%` to `%`. Any other `%` conversion is an error.
pub fn to_template(value: &str, args: &[&str]) -> Result<String, String> {
    let mut args = args.iter();
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push('\r'),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '%' => match chars.next() {
                Some('%') => out.push('%'),
                Some(d @ ('s' | 'c')) => match args.next() {
                    Some(name) => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                    None => return Err(format!("`%{d}` with no argument left")),
                },
                other => {
                    return Err(format!(
                        "unhandled printf directive `%{}`",
                        other.unwrap_or(' ')
                    ));
                }
            },
            _ => out.push(c),
        }
    }
    Ok(out)
}

/// The help topics of `WCCMMHLP.MSG`: `HL1B0` the overview, then pairs of a topic string
/// (`HL1Tn`) and the text after it (`HL1Bn`), paired by position as the game pairs them.
pub fn help_topics(entries: &[MsgEntry]) -> Result<HelpTopics, PackageError> {
    let is_help = |k: &str| {
        k.strip_prefix("HL1T")
            .or_else(|| k.strip_prefix("HL1B"))
            .is_some_and(|rest| {
                !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
    };
    let heads: Vec<&MsgEntry> = entries.iter().filter(|e| is_help(&e.key)).collect();
    let fail = |p: String| error(HELP_FILE, p);
    let Some((first, rest)) = heads.split_first().filter(|(f, _)| f.key == "HL1B0") else {
        return Err(fail("HL1B0 (the overview) must come first".into()));
    };
    if rest.len() % 2 != 0 {
        return Err(fail("topic and text messages do not pair up".into()));
    }
    let overview = to_template(&first.value, &[]).map_err(|p| fail(format!("HL1B0: {p}")))?;
    let mut topics = Vec::with_capacity(rest.len() / 2);
    for pair in rest.chunks(2) {
        let (t, b) = (pair[0], pair[1]);
        if !t.key.starts_with("HL1T") || !b.key.starts_with("HL1B") {
            return Err(fail(format!(
                "{}/{} is not a topic/text pair",
                t.key, b.key
            )));
        }
        if b.value.matches("%c").count() > HELP_ARGS.len() {
            return Err(fail(format!("{} has too many %c", b.key)));
        }
        topics.push(HelpTopic {
            key: t.key.clone(),
            body_key: b.key.clone(),
            words: t.value.clone(),
            text: to_template(&b.value, &HELP_ARGS).map_err(|p| fail(format!("{}: {p}", b.key)))?,
        });
    }
    HelpTopics { overview, topics }
        .validated()
        .map_err(|e| fail(e.to_string()))
}

/// The text blocks `blocks` of a message file as a catalogue, keyed by the lower-case key.
pub fn text_blocks(
    file: &str,
    entries: &[MsgEntry],
    blocks: &[TextBlock],
) -> Result<Catalogue, PackageError> {
    let mut messages = BTreeMap::new();
    for block in blocks {
        let e = entries
            .iter()
            .find(|e| e.key == block.key)
            .ok_or_else(|| error(file, format!("no {}", block.key)))?;
        if e.kind != "T" {
            return Err(error(file, format!("{} is not a text block", block.key)));
        }
        let text = to_template(&e.value, block.args)
            .map_err(|p| error(file, format!("{}: {p}", block.key)))?;
        messages.insert(
            block.key.to_ascii_lowercase(),
            Entry {
                text,
                wrap: false,
                pattern: block.pattern.map(str::to_string),
                args: BTreeMap::new(),
                example: block
                    .example
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                note: None,
            },
        );
    }
    let catalogue = Catalogue { messages };
    catalogue
        .validate()
        .map_err(|e| error(file, e.to_string()))?;
    Ok(catalogue)
}

/// The full-screen form `key` of `WCCTEXT.MSG` as text, `\r` line ends.
fn form(entries: &[MsgEntry], key: &str) -> Result<String, PackageError> {
    entries
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.value.replace('\n', "\r"))
        .ok_or_else(|| error(WCCTEXT_FILE, format!("no {key}")))
}

/// Parses [`DLL_TEXT_FILE`]: key to template.
pub fn parse_dll_text(data: &[u8]) -> Result<BTreeMap<String, String>, PackageError> {
    let fail = |p: &str| error(DLL_TEXT_FILE, p);
    let mut rest = data
        .strip_prefix(DLL_TEXT_HEADER.as_bytes())
        .and_then(|r| r.strip_prefix(b"\n"))
        .ok_or_else(|| fail("not a text file the import wrote (no header)"))?;
    let mut out = BTreeMap::new();
    while !rest.is_empty() {
        let nl = rest
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| fail("a key line has no end"))?;
        let line = std::str::from_utf8(&rest[..nl]).map_err(|_| fail("a key line is not text"))?;
        let (key, len) = line
            .split_once(' ')
            .and_then(|(k, l)| Some((k, l.parse::<usize>().ok()?)))
            .ok_or_else(|| fail(&format!("`{line}` is not `KEY LENGTH`")))?;
        let body = &rest[nl + 1..];
        if body.len() < len + 1 || body[len] != b'\n' {
            return Err(fail(&format!("the text of `{key}` is cut short")));
        }
        out.insert(key.to_string(), decode(&body[..len]));
        rest = &body[len + 1..];
    }
    Ok(out)
}

/// The game's long text, parsed and checked.
#[derive(Debug, Clone)]
pub struct PackageText {
    help: HelpTopics,
    wccmmud: Catalogue,
    wcctext: Catalogue,
    editcha1: String,
    edtchan2: String,
    editcha1_screen: EditorScreen,
    /// The moved catalogue entries: key to template and its pieces.
    moved: HashMap<String, (String, Vec<TemplatePiece>)>,
    /// [`PackageText::redact`]'s fragments, longest first, made on first use.
    fragments: OnceLock<Vec<String>>,
}

impl PackageText {
    /// Reads and checks the files from the assets directory `dir`.
    pub fn load(dir: &Path) -> Result<PackageText, PackageError> {
        let files = files_by_upper_name(dir);
        let read = |name: &str| -> Result<Vec<u8>, PackageError> {
            let path = files
                .get(name)
                .ok_or_else(|| error(name, format!("not in {}", dir.display())))?;
            std::fs::read(path).map_err(|e| error(name, e.to_string()))
        };
        Self::from_files(
            &read(HELP_FILE)?,
            &read(WCCMMUD_FILE)?,
            &read(WCCTEXT_FILE)?,
            &read(DLL_TEXT_FILE)?,
        )
    }

    /// Parses the four files' contents.
    pub fn from_files(
        help: &[u8],
        wccmmud: &[u8],
        wcctext: &[u8],
        dll_text: &[u8],
    ) -> Result<PackageText, PackageError> {
        let help = help_topics(&parse_msg(HELP_FILE, help)?)?;
        let wccmmud = text_blocks(
            WCCMMUD_FILE,
            &parse_msg(WCCMMUD_FILE, wccmmud)?,
            WCCMMUD_TEXTS,
        )?;
        let text_entries = parse_msg(WCCTEXT_FILE, wcctext)?;
        let wcctext = text_blocks(WCCTEXT_FILE, &text_entries, WCCTEXT_TEXTS)?;
        let editcha1 = form(&text_entries, "EDITCHA1")?;
        let edtchan2 = form(&text_entries, "EDTCHAN2")?;
        let editcha1_screen = EditorScreen::parse(&editcha1)
            .map_err(|e| error(WCCTEXT_FILE, format!("EDITCHA1: {e}")))?;
        let mut templates = parse_dll_text(dll_text)?;
        templates.insert("goodtext".into(), wcctext.messages["goodtext"].text.clone());
        let mut moved = HashMap::new();
        for key in MOVED_KEYS {
            let fail = |p: String| error(DLL_TEXT_FILE, format!("{key}: {p}"));
            let template = templates
                .remove(*key)
                .ok_or_else(|| fail("missing (import again)".into()))?;
            let pieces = crate::catalogue::parse_template(key, &template)
                .map_err(|e| fail(e.to_string()))?;
            if pieces.iter().any(|p| matches!(p, TemplatePiece::Arg(_))) {
                return Err(fail("takes no argument".into()));
            }
            moved.insert(key.to_string(), (template, pieces));
        }
        if let Some(extra) = templates.keys().next() {
            return Err(error(
                DLL_TEXT_FILE,
                format!("{extra}: not a moved catalogue key"),
            ));
        }
        Ok(PackageText {
            help,
            wccmmud,
            wcctext,
            editcha1,
            edtchan2,
            editcha1_screen,
            moved,
            fragments: OnceLock::new(),
        })
    }

    /// The help topics.
    pub fn help(&self) -> &HelpTopics {
        &self.help
    }

    /// The text blocks of `WCCMMUD.MSG`.
    pub fn wccmmud(&self) -> &Catalogue {
        &self.wccmmud
    }

    /// The text blocks of `WCCTEXT.MSG`.
    pub fn wcctext(&self) -> &Catalogue {
        &self.wcctext
    }

    /// The `EDITCHA1` form as text.
    pub fn editcha1(&self) -> &str {
        &self.editcha1
    }

    /// The `EDTCHAN2` form as text.
    pub fn edtchan2(&self) -> &str {
        &self.edtchan2
    }

    /// The `EDITCHA1` form, parsed.
    pub fn editcha1_screen(&self) -> &EditorScreen {
        &self.editcha1_screen
    }

    /// The template of a moved catalogue entry.
    pub fn catalogue_text(&self, key: &str) -> Option<&str> {
        self.moved.get(key).map(|(t, _)| t.as_str())
    }

    /// `text` with every fragment of the package's long text replaced by `[package text]`,
    /// for snapshots and test output that must not hold that text. A fragment is what the
    /// release guard looks for: a run of 12 or more printable ASCII characters (no quote or
    /// backslash, ANSI sequences removed) with three or more letters, cut from one line of a
    /// help text, a form or a moved screen, and not part of the text that stays in the
    /// catalogue or the prompts.
    pub fn redact(&self, text: &str) -> String {
        let fragments = self.fragments.get_or_init(|| self.make_fragments());
        let mut out = text.to_string();
        for f in fragments {
            if out.contains(f.as_str()) {
                out = out.replace(f.as_str(), "[package text]");
            }
        }
        out
    }

    fn make_fragments(&self) -> Vec<String> {
        let unbrace = |t: &str| t.replace("{{", "{").replace("}}", "}");
        let mut texts: Vec<String> = vec![unbrace(&self.help.overview)];
        texts.extend(self.help.topics.iter().map(|t| unbrace(&t.text)));
        texts.push(self.editcha1.clone());
        texts.push(self.edtchan2.clone());
        for (_, pieces) in self.moved.values() {
            let mut t = String::new();
            for p in pieces {
                match p {
                    TemplatePiece::Text(x) => t.push_str(x),
                    _ => t.push('\r'),
                }
            }
            texts.push(t);
        }
        let kept = format!(
            "{}\n{}",
            crate::msg::CATALOGUE_TOML,
            crate::prompt::PROMPT_TOML
        );
        let mut set = std::collections::BTreeSet::new();
        for t in &texts {
            for line in strip_escapes(t).split(['\r', '\n']) {
                for run in line.split(|c: char| !(' '..='~').contains(&c) || c == '"' || c == '\\')
                {
                    let run = run.trim();
                    if run.len() >= 12
                        && run.chars().filter(char::is_ascii_alphabetic).count() >= 3
                        && !kept.contains(run)
                    {
                        set.insert(run.to_string());
                    }
                }
            }
        }
        let mut out: Vec<String> = set.into_iter().collect();
        out.sort_by_key(|f| std::cmp::Reverse(f.len()));
        out
    }

    /// The pieces of a moved catalogue entry's template.
    pub fn catalogue_pieces(&self, key: &str) -> Option<&[TemplatePiece]> {
        self.moved.get(key).map(|(_, p)| p.as_slice())
    }
}

/// `text` without its ANSI sequences (`ESC [` parameters and a final letter).
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The regular files of `dir` by upper-case name; an unreadable directory gives none.
fn files_by_upper_name(dir: &Path) -> HashMap<String, std::path::PathBuf> {
    let mut out = HashMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.file_type().is_ok_and(|t| t.is_file()) {
            out.entry(e.file_name().to_string_lossy().to_ascii_uppercase())
                .or_insert_with(|| e.path());
        }
    }
    out
}

static INSTALLED: OnceLock<PackageText> = OnceLock::new();

/// Makes `text` the process's text. The first install wins: a later one is refused
/// (`false`) and changes nothing.
pub fn install(text: PackageText) -> bool {
    INSTALLED.set(text).is_ok()
}

/// Loads the text from the assets directory `dir` and installs it, unless text is already
/// installed, which is returned as it is.
pub fn load_and_install(dir: &Path) -> Result<&'static PackageText, PackageError> {
    if let Some(text) = INSTALLED.get() {
        return Ok(text);
    }
    let text = PackageText::load(dir)?;
    install(text);
    Ok(INSTALLED.get().expect("installed above"))
}

/// The installed text, if any.
pub fn installed() -> Option<&'static PackageText> {
    INSTALLED.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_entries_and_escapes() {
        let data = b"prose\r\nK1 {a~}b~~c} T desc\r\nK2 {x\r\n y} T\r\nnot {a key}\r\nK3{z}\r\n";
        let got = parse_msg("t", data).unwrap();
        let keys: Vec<&str> = got.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["K1", "K2", "K3"]);
        assert_eq!(got[0].value, "a}b~c");
        assert_eq!(got[0].kind, "T");
        assert_eq!(got[1].value, "x\n y");
        assert_eq!(got[2].kind, "");
        assert!(parse_msg("t", b"K {open").is_err());
    }

    #[test]
    fn templates() {
        assert_eq!(
            to_template("v%s {w}\nx", &["version"]).unwrap(),
            "v{version} {{w}}\rx"
        );
        assert_eq!(to_template("100%%", &[]).unwrap(), "100%");
        assert!(to_template("%d", &[]).is_err());
        assert!(to_template("%s", &[]).is_err());
    }

    #[test]
    fn dll_text_file() {
        let data = b"silvermere dll text 1\na 3\nx{{\nb 0\n\n";
        let got = parse_dll_text(data).unwrap();
        assert_eq!(got["a"], "x{{");
        assert_eq!(got["b"], "");
        assert!(parse_dll_text(b"a 3\nxyz\n").is_err());
        assert!(parse_dll_text(b"silvermere dll text 1\na 9\nxyz\n").is_err());
    }
}
