//! The pattern registry (`patterns.json`, T-004) and a Rust line classifier.
//!
//! The static half of the harness's `tools/differential/vendor/play/patterns.py` in Rust: `Registry.match_line`,
//! `fields`, `iter_events`, `_join` and `PromptParser.peel` behave the same way. Game-data
//! patterns (monster texts, verbs, room titles) are not available here; the verb
//! placeholders expand to the same no-data fallbacks as `load_static`.
//!
//! The patterns are Python `re`. Most compile with the `regex` crate; the few that need
//! lookaround or backreferences go to `fancy-regex`, chosen per pattern at load.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};

use serde::Deserialize;

use crate::captures::{plain_to_lines, text_to_lines};

/// One compiled regex, `regex` where possible, `fancy-regex` otherwise.
#[derive(Debug, Clone)]
pub enum Rx {
    Std(regex::Regex),
    Fancy(fancy_regex::Regex),
}

/// Named groups that took part in a match, and where the match ended.
#[derive(Debug, Clone, Default)]
pub struct Caps {
    pub groups: BTreeMap<String, String>,
    pub end: usize,
}

impl Rx {
    /// A search regex (`re.search`).
    pub fn new(src: &str) -> Result<Self, PatternError> {
        match regex::Regex::new(src) {
            Ok(rx) => Ok(Self::Std(rx)),
            Err(std_err) => {
                fancy_regex::Regex::new(src)
                    .map(Self::Fancy)
                    .map_err(|_| PatternError::Regex {
                        src: src.to_owned(),
                        message: std_err.to_string(),
                    })
            }
        }
    }

    /// A whole-line regex (`re.fullmatch`).
    pub fn full(src: &str) -> Result<Self, PatternError> {
        Self::new(&format!("^(?:{src})$"))
    }

    /// A start-anchored regex (`re.match`).
    pub fn prefix(src: &str) -> Result<Self, PatternError> {
        Self::new(&format!("^(?:{src})"))
    }

    pub fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Std(rx) => rx.is_match(text),
            Self::Fancy(rx) => rx.is_match(text).unwrap_or(false),
        }
    }

    /// First match with its named groups.
    pub fn captures(&self, text: &str) -> Option<Caps> {
        match self {
            Self::Std(rx) => {
                let c = rx.captures(text)?;
                let groups = rx
                    .capture_names()
                    .flatten()
                    .filter_map(|n| c.name(n).map(|m| (n.to_owned(), m.as_str().to_owned())))
                    .collect();
                Some(Caps {
                    groups,
                    end: c.get(0)?.end(),
                })
            }
            Self::Fancy(rx) => {
                let c = rx.captures(text).ok()??;
                let groups = rx
                    .capture_names()
                    .flatten()
                    .filter_map(|n| c.name(n).map(|m| (n.to_owned(), m.as_str().to_owned())))
                    .collect();
                Some(Caps {
                    groups,
                    end: c.get(0)?.end(),
                })
            }
        }
    }

    /// Replaces every match with `with` (literal).
    pub fn replace_all(&self, text: &str, with: &str) -> String {
        match self {
            Self::Std(rx) => rx.replace_all(text, regex::NoExpand(with)).into_owned(),
            Self::Fancy(rx) => rx
                .replace_all(text, fancy_regex::NoExpand(with))
                .into_owned(),
        }
    }

    /// Group `n` of the last match in `text` (`finditer` then `[-1].group(1)`).
    pub fn last_group(&self, text: &str, n: usize) -> Option<String> {
        match self {
            Self::Std(rx) => rx
                .captures_iter(text)
                .last()
                .and_then(|c| c.get(n).map(|m| m.as_str().to_owned())),
            Self::Fancy(rx) => rx
                .captures_iter(text)
                .filter_map(Result::ok)
                .last()
                .and_then(|c| c.get(n).map(|m| m.as_str().to_owned())),
        }
    }
}

/// Errors loading the registry.
#[derive(Debug, thiserror::Error)]
pub enum PatternError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing patterns.json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("regex {src:?} does not compile: {message}")]
    Regex { src: String, message: String },
}

#[derive(Debug, Deserialize)]
struct RawPattern {
    name: String,
    cat: String,
    rx: String,
    #[serde(default)]
    example: String,
    #[serde(default)]
    seen: String,
    prio: u32,
    #[serde(default)]
    groups: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawTable {
    opener: String,
    row_name: String,
    cat: String,
    rx: String,
    max_rows: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawJoin {
    prefix: String,
    rule: String,
}

#[derive(Debug, Deserialize)]
struct RawPrompt {
    std: String,
    generic: String,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    counts: HashMap<String, serde_json::Value>,
    #[serde(rename = "static")]
    statics: Vec<RawPattern>,
    tables: Vec<RawTable>,
    join: Vec<RawJoin>,
    exit_tail: Vec<String>,
    prompt: RawPrompt,
}

/// One static pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub name: String,
    pub cat: String,
    /// The regex source after placeholder expansion.
    pub rx_src: String,
    pub example: String,
    /// `C` confirmed, `F` forum capture only, `I` inferred.
    pub seen: String,
    pub prio: u32,
    /// Literal characters in the regex: the tie-break between equal priorities.
    pub spec: usize,
    pub groups: Vec<String>,
    rx: Rx,
}

impl Pattern {
    /// Whole-line match of this pattern alone.
    pub fn matches(&self, line: &str) -> Option<BTreeMap<String, String>> {
        self.rx.captures(line).map(|c| c.groups)
    }
}

/// A table-row pattern, armed by the opener pattern of the same name.
#[derive(Debug, Clone)]
pub struct Table {
    pub opener: String,
    pub row_name: String,
    pub cat: String,
    pub max_rows: usize,
    rx: Rx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoinRule {
    Exits,
    DotHere,
    Dot,
    UntilHave,
}

/// A classified line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub name: String,
    pub cat: String,
    pub fields: BTreeMap<String, String>,
    /// Fields whose value `normalise` replaced; they compare equal whatever they held.
    pub masked: BTreeSet<String>,
    pub line: String,
    pub lineno: usize,
}

impl Event {
    fn new(
        name: &str,
        cat: &str,
        fields: BTreeMap<String, String>,
        line: &str,
        lineno: usize,
    ) -> Self {
        Self {
            name: name.to_owned(),
            cat: cat.to_owned(),
            fields,
            masked: BTreeSet::new(),
            line: line.to_owned(),
            lineno,
        }
    }
}

/// How the prompt at the start of a line is read (`PromptParser.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    /// `[HP=..]:` forms only (the default).
    Std,
    /// `>` only (statline OFF).
    Off,
    /// The standard form, then any bracketed `[..hp=..]:` (the generic form).
    Auto,
}

struct PromptParser {
    mode: PromptMode,
    std: Rx,
    generic_src: String,
    generic: OnceLock<Rx>,
}

static KV_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"([A-Za-z]+)=(-?\w+)(?:/(\w+))?").expect("kv"));
static SPEC_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\(\?P<\w+>|\\.|[()?*+.|\[\]{}^$]").expect("spec"));

impl PromptParser {
    /// `PromptParser.peel`: `(fields, byte offset where the rest of the line starts)`.
    fn peel(&self, line: &str) -> Option<(BTreeMap<String, String>, usize)> {
        if matches!(self.mode, PromptMode::Std | PromptMode::Auto)
            && let Some(c) = self.std.captures(line)
        {
            let mut d = c.groups;
            let state1 = d.remove("state1").unwrap_or_default();
            let state2 = d.remove("state2").unwrap_or_default();
            let state = if state1.is_empty() { state2 } else { state1 };
            let state = state.trim_matches(|c| c == ' ' || c == '(' || c == ')');
            if !state.is_empty() {
                d.insert("state".into(), state.to_owned());
            }
            d.insert("form".into(), "std".into());
            return Some((d, c.end));
        }
        if self.mode == PromptMode::Auto {
            let generic = self.generic.get_or_init(|| {
                Rx::prefix(&self.generic_src).expect("generic prompt compiles with fancy-regex")
            });
            if let Some(c) = generic.captures(line) {
                let mut d = BTreeMap::new();
                d.insert("form".to_owned(), "generic".to_owned());
                if let Some(s) = c.groups.get("state") {
                    let s = s.trim_matches(|c| c == ' ' || c == '(' || c == ')');
                    if !s.is_empty() {
                        d.insert("state".into(), s.to_owned());
                    }
                }
                let body = c.groups.get("body").cloned().unwrap_or_default();
                for kv in KV_RE.captures_iter(&body) {
                    let k = kv[1].to_lowercase();
                    let k = match k.as_str() {
                        "ma" | "kai" => "mana".to_owned(),
                        "cash" => "wealth".to_owned(),
                        _ => k,
                    };
                    let v = &kv[2];
                    if v.trim_start_matches('-')
                        .chars()
                        .all(|c| c.is_ascii_digit())
                        && !v.trim_start_matches('-').is_empty()
                    {
                        d.insert(k.clone(), v.to_owned());
                    }
                    if let Some(mx) = kv.get(3)
                        && !mx.as_str().is_empty()
                        && mx.as_str().chars().all(|c| c.is_ascii_digit())
                    {
                        d.insert(format!("max{k}"), mx.as_str().to_owned());
                    }
                }
                return Some((d, c.end));
            }
        }
        if self.mode == PromptMode::Off && line.starts_with('>') {
            let d = BTreeMap::from([("form".to_owned(), "off".to_owned())]);
            return Some((d, 1));
        }
        None
    }
}

/// A line the client typed, spliced into a line list so its echo is not misread
/// (`patterns.SENT`).
const SENT: &str = "\0>";

/// `patterns.sent_marker`.
pub fn sent_marker(cmd: &str) -> String {
    format!("{SENT}{cmd}")
}

/// The static registry.
pub struct PatternSet {
    patterns: Vec<Pattern>,
    /// Indexes into `patterns` in matching order: `(prio, -spec)`, ties in file order.
    order: Vec<usize>,
    by_name: HashMap<String, usize>,
    tables: HashMap<String, Table>,
    join: Vec<(String, JoinRule)>,
    exit_tail: HashSet<String>,
    prompt: PromptParser,
    room_names: HashSet<String>,
    static_count: usize,
}

/// `{HITV}` etc. with no game data, as `Registry.load_static`.
fn expand_verbs(rx: &str) -> String {
    const HIT: &str = "[a-z]+(?: [a-z]+)?";
    const MISS: &str = "[a-z]+ at";
    rx.replace("{HITV}", HIT)
        .replace("{HITV3}", HIT)
        .replace("{MISSV3}", MISS)
        .replace("{MISSV}", MISS)
}

impl PatternSet {
    /// The registry vendored at `tests/fixtures/patterns.json`.
    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/patterns.json")
    }

    /// Loads `patterns.json` from `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatternError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| PatternError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::from_json(&bytes)
    }

    /// Loads the vendored registry.
    pub fn load_default() -> Result<Self, PatternError> {
        Self::load(Self::default_path())
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, PatternError> {
        let file: RawFile = serde_json::from_slice(bytes)?;
        let mut patterns = Vec::with_capacity(file.statics.len());
        for p in file.statics {
            let rx_src = expand_verbs(&p.rx);
            let spec = SPEC_RE.replace_all(&rx_src, "").chars().count();
            patterns.push(Pattern {
                rx: Rx::full(&rx_src)?,
                name: p.name,
                cat: p.cat,
                rx_src,
                example: p.example,
                seen: p.seen,
                prio: p.prio,
                spec,
                groups: p.groups,
            });
        }
        let mut order: Vec<usize> = (0..patterns.len()).collect();
        order.sort_by_key(|&i| (patterns[i].prio, std::cmp::Reverse(patterns[i].spec)));
        let mut by_name = HashMap::new();
        for (i, p) in patterns.iter().enumerate() {
            by_name.entry(p.name.clone()).or_insert(i);
        }
        let mut tables = HashMap::new();
        for t in file.tables {
            tables.insert(
                t.opener.clone(),
                Table {
                    rx: Rx::full(&t.rx)?,
                    opener: t.opener,
                    row_name: t.row_name,
                    cat: t.cat,
                    max_rows: t.max_rows.unwrap_or(999),
                },
            );
        }
        let join = file
            .join
            .into_iter()
            .filter_map(|j| {
                let rule = match j.rule.as_str() {
                    "exits" => JoinRule::Exits,
                    "dot_here" => JoinRule::DotHere,
                    "dot" => JoinRule::Dot,
                    "until_have" => JoinRule::UntilHave,
                    _ => return None,
                };
                Some((j.prefix, rule))
            })
            .collect();
        let static_count = file
            .counts
            .get("static")
            .and_then(serde_json::Value::as_u64)
            .map_or(patterns.len(), |n| n as usize);
        Ok(Self {
            patterns,
            order,
            by_name,
            tables,
            join,
            exit_tail: file.exit_tail.into_iter().collect(),
            prompt: PromptParser {
                mode: PromptMode::Std,
                std: Rx::prefix(&file.prompt.std)?,
                generic_src: file.prompt.generic,
                generic: OnceLock::new(),
            },
            room_names: HashSet::new(),
            static_count,
        })
    }

    /// Every static pattern in file order.
    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }

    /// `counts.static` from the file.
    pub fn declared_static_count(&self) -> usize {
        self.static_count
    }

    pub fn by_name(&self, name: &str) -> Option<&Pattern> {
        self.by_name.get(name).map(|&i| &self.patterns[i])
    }

    /// Table-row patterns by opener name.
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.tables.values()
    }

    /// Sets how prompts are read; the generic form (lookahead, `fancy-regex`) is compiled
    /// the first time `Auto` needs it.
    pub fn set_prompt_mode(&mut self, mode: PromptMode) {
        self.prompt.mode = mode;
    }

    /// Room titles from game data; none by default, so no `room_title`/`room_desc` events.
    pub fn set_room_names<I: IntoIterator<Item = String>>(&mut self, names: I) {
        self.room_names = names.into_iter().collect();
    }

    /// `Registry.match_line`: the first pattern by `(prio, -spec)` whose regex matches the
    /// whole (stripped) line.
    pub fn match_line(&self, line: &str) -> Option<(&Pattern, BTreeMap<String, String>)> {
        self.order.iter().find_map(|&i| {
            let p = &self.patterns[i];
            p.rx.captures(line).map(|c| (p, c.groups))
        })
    }

    /// Classifies decoded text. `raw` selects `raw_to_lines` (ANSI, CLRLINE) over
    /// `plain_to_lines`; `sent` are recent commands whose echo is reported as `echo`.
    pub fn classify_text(&self, text: &str, raw: bool, sent: &[String]) -> Vec<Event> {
        let lines = if raw {
            text_to_lines(text)
        } else {
            plain_to_lines(text)
        };
        self.classify_lines_sent(&lines, sent)
    }

    /// `iter_events` over `lines`; lines starting with the sent marker splice commands in.
    pub fn classify_lines<S: AsRef<str>>(&self, lines: &[S]) -> Vec<Event> {
        self.classify_lines_sent(lines, &[])
    }

    /// Classifies a `plain.txt` transcript: its `>>> 'cmd'` lines become sent markers
    /// (`capture.classify`).
    pub fn classify_transcript<S: AsRef<str>>(&self, lines: &[S]) -> Vec<Event> {
        let converted: Vec<String> = lines
            .iter()
            .map(|l| {
                let l = l.as_ref();
                match l.strip_prefix(">>> ") {
                    Some(rest)
                        if rest.len() >= 2
                            && (rest.starts_with('\'') || rest.starts_with('"'))
                            && rest.chars().next() == rest.chars().last() =>
                    {
                        sent_marker(&rest[1..rest.len() - 1])
                    }
                    _ => l.to_owned(),
                }
            })
            .collect();
        self.classify_lines(&converted)
    }

    fn take_echo(pending: &mut std::collections::VecDeque<String>, s: &str) -> bool {
        if let Some(k) = pending.iter().position(|c| c == s) {
            pending.drain(..=k);
            true
        } else {
            false
        }
    }

    /// As the harness's `Registry.iter_events`.
    pub fn classify_lines_sent<S: AsRef<str>>(&self, lines: &[S], sent: &[String]) -> Vec<Event> {
        let mut out = Vec::new();
        let mut pending: std::collections::VecDeque<String> =
            sent.iter().map(|s| s.trim().to_owned()).collect();
        let mut table: Option<(&Table, usize)> = None;
        let mut after_title = false;
        let n = lines.len();
        let mut i = 0;
        while i < n {
            let mut line = lines[i].as_ref().replace('\x07', "");
            let lineno = i;
            i += 1;
            if let Some(cmd) = line.strip_prefix(SENT) {
                pending.push_back(cmd.trim().to_owned());
                while pending.len() > 4 {
                    pending.pop_front();
                }
                continue;
            }
            // prompts at the start of the line (several can pile up on one line)
            while let Some((pf, end)) = self.prompt.peel(&line) {
                out.push(Event::new("prompt", "prompt", pf, &line, lineno));
                after_title = false;
                table = None;
                let rest = line[end..].to_owned();
                let echoed = !rest.trim().is_empty() && Self::take_echo(&mut pending, rest.trim());
                if echoed {
                    out.push(Event::new(
                        "echo",
                        "echo",
                        BTreeMap::from([("cmd".to_owned(), rest.trim().to_owned())]),
                        &rest,
                        lineno,
                    ));
                    line = String::new();
                    break;
                }
                line = rest;
            }
            let mut s = line.trim().to_owned();
            if s.is_empty() {
                continue;
            }
            if Self::take_echo(&mut pending, &s) {
                out.push(Event::new(
                    "echo",
                    "echo",
                    BTreeMap::from([("cmd".to_owned(), s.clone())]),
                    &line,
                    lineno,
                ));
                continue;
            }
            // wrapped displays
            if let Some((_, rule)) = self.join.iter().find(|(prefix, _)| s.starts_with(prefix)) {
                (s, i) = self.join_lines(s, lines, i, *rule);
            }
            // table rows
            if let Some((t, rows)) = table {
                let m = if rows < t.max_rows {
                    t.rx.captures(&s)
                } else {
                    None
                };
                if let Some(c) = m {
                    table = Some((t, rows + 1));
                    out.push(Event::new(&t.row_name, &t.cat, c.groups, &s, lineno));
                    continue;
                }
                table = None;
            }
            // room title (exact name from the room data)
            if self.room_names.contains(&s) {
                after_title = true;
                out.push(Event::new(
                    "room_title",
                    "room",
                    BTreeMap::from([("name".to_owned(), s.clone())]),
                    &s,
                    lineno,
                ));
                continue;
            }
            if let Some((p, fields)) = self.match_line(&s) {
                if let Some(t) = self.tables.get(&p.name) {
                    table = Some((t, 0));
                }
                if p.cat == "room" && p.name.starts_with("room_") {
                    after_title = false;
                }
                out.push(Event::new(&p.name, &p.cat, fields, &s, lineno));
                continue;
            }
            if after_title {
                out.push(Event::new("room_desc", "room", BTreeMap::new(), &s, lineno));
                continue;
            }
            out.push(Event::new(
                "unknown",
                "unclassified",
                BTreeMap::new(),
                &s,
                lineno,
            ));
        }
        out
    }

    /// As the harness's `Registry._join`: wrapped displays.
    fn join_lines<S: AsRef<str>>(
        &self,
        mut s: String,
        lines: &[S],
        mut i: usize,
        rule: JoinRule,
    ) -> (String, usize) {
        for _ in 0..12 {
            if i >= lines.len() {
                break;
            }
            let nxt = lines[i].as_ref().trim();
            let stop = match rule {
                JoinRule::Exits => {
                    let last = s.rsplit(' ').next().unwrap_or("");
                    !(s.ends_with(',') || self.exit_tail.contains(last))
                        || nxt.is_empty()
                        || nxt.starts_with('[')
                }
                JoinRule::DotHere => {
                    s.ends_with(" here.") || nxt.is_empty() || nxt.starts_with('[')
                }
                JoinRule::Dot => s.ends_with('.') || nxt.is_empty() || nxt.starts_with('['),
                JoinRule::UntilHave => {
                    nxt.is_empty()
                        || nxt.starts_with("You have")
                        || nxt.starts_with("Wealth:")
                        || nxt.starts_with('[')
                }
            };
            if stop {
                break;
            }
            s.push(' ');
            s.push_str(nxt);
            i += 1;
        }
        (s, i)
    }
}
