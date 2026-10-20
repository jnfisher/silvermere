//! `docs/config.md` lists exactly the keys `silvermere serve --print-config` prints, with the real defaults.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Keys whose printed value depends on the data directory, or that are absent by default.
const RESOLVED: [&str; 2] = ["data_dir", "database.url"];
const ABSENT: [&str; 1] = ["game.rng_seed"];

fn flatten(prefix: &str, v: &toml::Value, out: &mut BTreeMap<String, toml::Value>) {
    match v {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        other => {
            out.insert(prefix.to_owned(), other.clone());
        }
    }
}

/// Computed once: both tests share it, so there is one folder and one process.
fn printed_defaults() -> &'static BTreeMap<String, toml::Value> {
    static DEFAULTS: OnceLock<BTreeMap<String, toml::Value>> = OnceLock::new();
    DEFAULTS.get_or_init(run_print_config)
}

fn run_print_config() -> BTreeMap<String, toml::Value> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("docs_in_sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(&dir).args(["serve", "--print-config"]);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    let o = c.output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v = toml::Value::Table(
        String::from_utf8(o.stdout)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap(),
    );
    let mut m = BTreeMap::new();
    flatten("", &v, &mut m);
    m
}

/// (key, default cell) for each table row between "## Keys" and "## Original options".
fn documented() -> Vec<(String, String)> {
    let md = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/config.md"))
        .unwrap();
    let keys = md.split("\n## Keys").nth(1).expect("## Keys section");
    let keys = keys.split("\n## Original options").next().unwrap();
    let mut rows = Vec::new();
    for line in keys.lines().filter(|l| l.starts_with("| `")) {
        let cells: Vec<&str> = line.split(" | ").collect();
        let key = cells[0]
            .trim_start_matches("| ")
            .trim()
            .trim_matches('`')
            .to_owned();
        let default = cells[2].trim().trim_matches('`').to_owned();
        rows.push((key, default));
    }
    rows
}

#[test]
fn config_md_keys_match_print_config() {
    let mut want: Vec<String> = printed_defaults().keys().cloned().collect();
    want.extend(ABSENT.iter().map(|s| s.to_string()));
    want.sort();
    let mut have: Vec<String> = documented().into_iter().map(|(k, _)| k).collect();
    have.sort();
    assert_eq!(
        have, want,
        "docs/config.md first column differs from `serve --print-config`"
    );
}

#[test]
fn config_md_defaults_match() {
    let real = printed_defaults();
    for (key, cell) in documented() {
        if ABSENT.contains(&key.as_str()) {
            assert_eq!(cell, "absent", "{key}");
            assert!(!real.contains_key(&key), "{key} is now printed by default");
            continue;
        }
        if RESOLVED.contains(&key.as_str()) {
            continue;
        }
        let doc: toml::Table = format!("v = {cell}")
            .parse()
            .unwrap_or_else(|e| panic!("{key}: default `{cell}` is not TOML: {e}"));
        assert_eq!(doc["v"], real[&key], "{key}: documented default `{cell}`");
    }
}

/// The `config` subcommands, from `silvermere config --help`.
fn config_subcommands() -> Vec<String> {
    let o = Command::new(env!("CARGO_BIN_EXE_silvermere"))
        .args(["config", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(o.stdout).unwrap();
    let list = help.split("Commands:").nth(1).expect("a Commands: list");
    let list = list.split("\n\n").next().unwrap();
    list.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|w| *w != "help")
        .map(str::to_owned)
        .collect()
}

/// T-124: the README's Settings section and docs/config.md name exactly the real
/// `silvermere config` subcommands.
#[test]
fn config_subcommands_are_documented() {
    let real = config_subcommands();
    assert!(real.len() >= 5, "{real:?}");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let readme = std::fs::read_to_string(format!("{root}README.md")).unwrap();
    let settings = readme
        .split("\n## Settings")
        .nth(1)
        .expect("## Settings in README.md");
    let settings = settings.split("\n## ").next().unwrap().to_owned();
    let config_md = std::fs::read_to_string(format!("{root}docs/config.md")).unwrap();
    for (name, text) in [
        ("README.md Settings", settings),
        ("docs/config.md", config_md),
    ] {
        let mut named: Vec<String> = text
            .split("silvermere config ")
            .skip(1)
            .filter_map(|rest| {
                let word: String = rest.chars().take_while(char::is_ascii_lowercase).collect();
                (!word.is_empty()).then_some(word)
            })
            .collect();
        named.sort();
        named.dedup();
        let mut want = real.clone();
        want.sort();
        assert_eq!(
            named, want,
            "{name} lists other `silvermere config` subcommands"
        );
    }
}

/// `silvermere <path> --help`.
fn help(path: &[String]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_silvermere"))
        .args(path)
        .arg("--help")
        .output()
        .unwrap();
    assert!(o.status.success(), "{path:?} --help");
    String::from_utf8(o.stdout).unwrap()
}

/// The subcommands in a help text's `Commands:` list.
fn children(help: &str) -> Vec<String> {
    let Some(list) = help.split("Commands:\n").nth(1) else {
        return Vec::new();
    };
    let list = list.split("\n\n").next().unwrap();
    list.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|w| *w != "help")
        .map(str::to_owned)
        .collect()
}

/// The flags in a help text's `Options:` list: each long name, and a short name that has no
/// long one.
fn flags(help: &str) -> Vec<String> {
    let opts = help.split("Options:\n").nth(1).unwrap_or("");
    let mut out = Vec::new();
    for l in opts.lines() {
        let Some(l) = l.strip_prefix("  ") else {
            continue;
        };
        let mut words = l.split_whitespace();
        let Some(first) = words.next() else { continue };
        let first = first.trim_end_matches(',');
        let is_short = first.len() == 2 && first.starts_with('-') && !first.starts_with("--");
        if is_short
            && l.trim_start().starts_with(&format!("{first}, "))
            && let Some(long) = words.next().filter(|w| w.starts_with("--"))
        {
            out.push(long.to_owned());
            continue;
        }
        if first.starts_with('-') {
            out.push(first.to_owned());
        }
    }
    out
}

/// True when `text` has `needle` as a whole word: not part of a longer flag or command name
/// (`--out` is not in `--output`, `account pass` is not in `account passwd`).
fn mentions(text: &str, needle: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    text.match_indices(needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// Every command path below `path`, `path` itself first (the root is the empty path).
fn command_paths(path: &[String], out: &mut Vec<Vec<String>>) {
    out.push(path.to_vec());
    for c in children(&help(path)) {
        let mut p = path.to_vec();
        p.push(c);
        command_paths(&p, out);
    }
}

/// T-133: `docs/cli.md` has a section for each top-level command, names every subcommand
/// and every flag of it, and lists the global flags.
#[test]
fn cli_md_names_every_command_and_flag() {
    let md =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli.md")).unwrap();
    let mut paths = Vec::new();
    command_paths(&[], &mut paths);
    let root_help = help(&[]);
    let globals = flags(&root_help);
    assert!(globals.len() >= 4, "{globals:?}");
    let global_section = md
        .split("\n## Global flags")
        .nth(1)
        .expect("## Global flags in docs/cli.md")
        .split("\n## ")
        .next()
        .unwrap();
    for g in &globals {
        assert!(
            mentions(global_section, g),
            "docs/cli.md ## Global flags lacks {g}"
        );
    }
    let sections: Vec<&str> = md.split("\n## `silvermere ").skip(1).collect();
    for p in paths.iter().filter(|p| p.len() == 1) {
        // `admin` has one subcommand, `token`, and the page has one section for both.
        let heading = if p[0] == "admin" {
            "admin token"
        } else {
            p[0].as_str()
        };
        let section = sections
            .iter()
            .find(|s| s.starts_with(&format!("{heading}`")))
            .unwrap_or_else(|| panic!("docs/cli.md lacks a `## silvermere {heading}` section"));
        let section = section.split("\n## ").next().unwrap();
        for sub in paths.iter().filter(|q| q.first() == p.first()) {
            let name = sub.join(" ");
            assert!(
                mentions(section, &name),
                "docs/cli.md section `{heading}` does not name `{name}`"
            );
            for f in flags(&help(sub)).iter().filter(|f| !globals.contains(f)) {
                assert!(
                    mentions(section, f),
                    "docs/cli.md section `{heading}` lacks {f} of `{name}`"
                );
            }
        }
    }
}
