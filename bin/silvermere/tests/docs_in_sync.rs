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
