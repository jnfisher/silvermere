//! `silvermere config` (T-124): the settings file `serve` reads, shown, changed and edited
//! with the same lookup and checks as startup.

use std::collections::BTreeMap;
use std::io::{BufRead, Write as _};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use clap::Subcommand;

use crate::config::{
    Config, Overrides, config_file, config_target, lookup_data_dir, toml_syntax_error,
};

/// `config` subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Print the settings file this install reads, and whether it exists.
    Path,
    /// Print the effective settings, or one key or section, and where each value comes from.
    Show {
        /// A key such as game.death_hp, or a section such as game.
        key: Option<String>,
    },
    /// Write one setting into the file (VALUE is TOML; a bare word is a string).
    Set {
        key: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Comment a setting out of the file, so it is back to its default.
    Unset { key: String },
    /// Open the file in $VISUAL or $EDITOR and check it when the editor closes.
    Edit,
}

/// Keys that are absent by default.
const OPTIONAL: [&str; 1] = ["game.rng_seed"];

/// What restarting looks like, after every change.
const RESTART: &str =
    "restart the server to apply (for the service: sudo systemctl restart silvermere)";

pub fn run(
    cmd: &ConfigCmd,
    data_dir_flag: Option<&Path>,
    ov: &Overrides,
) -> anyhow::Result<ExitCode> {
    let target = config_target(data_dir_flag, ov);
    match cmd {
        ConfigCmd::Path => {
            let state = if target.exists() {
                "exists"
            } else {
                "does not exist yet; `silvermere config set` or `config edit` creates it"
            };
            println!("{} ({state})", shown_path(&target));
        }
        ConfigCmd::Show { key } => show(data_dir_flag, ov, key.as_deref())?,
        ConfigCmd::Set { key, value } => set(data_dir_flag, ov, &target, key, Some(value))?,
        ConfigCmd::Unset { key } => set(data_dir_flag, ov, &target, key, None)?,
        ConfigCmd::Edit => return edit(&target, &lookup_data_dir(data_dir_flag)),
    }
    Ok(ExitCode::SUCCESS)
}

/// Where a value comes from, highest first.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Flag,
    Env(String),
    File,
    Default,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Flag => f.write_str("flag"),
            Source::Env(var) => write!(f, "env {var}"),
            Source::File => f.write_str("file"),
            Source::Default => f.write_str("default"),
        }
    }
}

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

fn flat(text: &str) -> BTreeMap<String, toml::Value> {
    let mut out = BTreeMap::new();
    if let Ok(t) = text.parse::<toml::Table>() {
        flatten("", &toml::Value::Table(t), &mut out);
    }
    out
}

/// Every key with its default (`--print-config` of the defaults).
fn defaults() -> BTreeMap<String, toml::Value> {
    flat(&toml::to_string(&Config::default()).unwrap_or_default())
}

fn is_known(key: &str) -> bool {
    OPTIONAL.contains(&key) || defaults().contains_key(key)
}

/// The key a `SILVERMERE_*` variable sets, as `Config::load` reads it.
fn env_key(var: &str) -> Option<String> {
    let rest = var.strip_prefix("SILVERMERE_")?;
    Some(match rest {
        "DATABASE_URL" => "database.url".into(),
        "ADMIN_TOKENS" | "ADMIN_TOKEN" => "admin.tokens".into(),
        _ => rest.to_ascii_lowercase().replace("__", "."),
    })
}

/// Which layer above the defaults sets each key.
fn sources(
    data_dir_flag: Option<&Path>,
    ov: &Overrides,
    file: Option<&Path>,
) -> BTreeMap<String, Source> {
    let mut out = BTreeMap::new();
    if let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) {
        for k in flat(&text).into_keys() {
            out.insert(k, Source::File);
        }
    }
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, v)| k.starts_with("SILVERMERE_") && !v.is_empty())
        .collect();
    vars.sort();
    for (var, _) in vars {
        if let Some(k) = env_key(&var) {
            out.insert(k, Source::Env(var));
        }
    }
    for k in flat(&ov.flag_lines.join("\n")).into_keys() {
        out.insert(k, Source::Flag);
    }
    if data_dir_flag.is_some() {
        out.insert("data_dir".into(), Source::Flag);
    }
    out
}

fn show(data_dir_flag: Option<&Path>, ov: &Overrides, key: Option<&str>) -> anyhow::Result<()> {
    if let Some(k) = key {
        let prefix = format!("{k}.");
        anyhow::ensure!(
            is_known(k) || defaults().keys().any(|d| d.starts_with(&prefix)),
            "unknown key `{k}`; `silvermere config show` lists them all"
        );
    }
    let cfg = Config::load(data_dir_flag, ov)?;
    let file = config_file(data_dir_flag, ov)?;
    let from = sources(data_dir_flag, ov, file.as_deref());
    // `to_toml` is `--print-config`: the secrets are redacted.
    let mut values = flat(&cfg.to_toml()?);
    for k in OPTIONAL {
        values
            .entry(k.into())
            .or_insert(toml::Value::String("(not set)".into()));
    }
    match &file {
        Some(f) => println!("# file: {}", shown_path(f)),
        None => println!(
            "# file: none (would be {})",
            shown_path(&config_target(data_dir_flag, ov))
        ),
    }
    let rows: Vec<(String, String)> = values
        .iter()
        .filter(|(k, _)| key.is_none_or(|want| *k == want || k.starts_with(&format!("{want}."))))
        .map(|(k, v)| {
            let v = if OPTIONAL.contains(&k.as_str()) && v.as_str() == Some("(not set)") {
                "(not set)".to_owned()
            } else {
                v.to_string()
            };
            (format!("{k} = {v}"), k.clone())
        })
        .collect();
    let width = rows.iter().map(|(l, _)| l.len()).max().unwrap_or(0).min(60);
    for (line, k) in rows {
        let src = from.get(&k).cloned().unwrap_or(Source::Default);
        println!("{line:width$}  # {src}");
    }
    Ok(())
}

/// Why `config set` and `config unset` leave a key that holds a secret alone, and what to
/// do instead.
fn secret_refusal(key: &str, unset: bool) -> Option<String> {
    let tokens = key == "admin.tokens" || key.starts_with("admin.");
    Some(match (tokens, key == "database.url", unset) {
        (true, _, false) => format!(
            "{key} holds secrets, so `config set` does not write it: make a token with \
             `silvermere admin token new NAME` and paste the [[admin.tokens]] block it prints \
             with `silvermere config edit`"
        ),
        (true, _, true) => format!(
            "{key} holds secrets, so `config unset` does not touch it: remove a token's \
             [[admin.tokens]] block with `silvermere config edit` (`silvermere admin token list` \
             shows them)"
        ),
        (false, true, false) => "database.url can hold a password, so `config set` does not \
             write it: on the service install run `sudo ./silvermere service install --system \
             --database-url-from-stdin` from /opt/silvermere, or put SILVERMERE_DATABASE_URL in an \
             env file only root can read (docs/install.md)"
            .to_owned(),
        (false, true, true) => "database.url can hold a password, so `config unset` does not \
             touch it: remove its line with `silvermere config edit`, or the \
             SILVERMERE_DATABASE_URL line from the env file (/etc/silvermere/env for the \
             service); with neither, the server uses SQLite in the data directory"
            .to_owned(),
        (false, false, _) => return None,
    })
}

/// A value as `serve --set` reads it: TOML, else a plain string.
fn parse_value(raw: &str) -> toml_edit::Value {
    let raw = raw.trim();
    raw.parse::<toml_edit::Value>()
        .unwrap_or_else(|_| toml_edit::Value::from(raw))
}

fn bare(v: &toml_edit::Value) -> String {
    v.clone().decorated("", "").to_string()
}

fn read_doc(target: &Path) -> anyhow::Result<(String, toml_edit::DocumentMut)> {
    let text = match std::fs::read_to_string(target) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(write_error(target, e)),
    };
    let doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| {
        let msg = e.to_string();
        let msg = toml_syntax_error(&msg).unwrap_or(msg);
        anyhow::anyhow!(
            "{} is not valid TOML ({msg}); fix it with `silvermere config edit`",
            target.display()
        )
    })?;
    Ok((text, doc))
}

fn get<'a>(doc: &'a toml_edit::DocumentMut, key: &str) -> Option<&'a toml_edit::Value> {
    let mut item = doc.as_item();
    for part in key.split('.') {
        item = item.as_table_like()?.get(part)?;
    }
    item.as_value()
}

fn table_at<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    path: &[&str],
) -> Option<&'a mut dyn toml_edit::TableLike> {
    let mut table = doc.as_table_mut() as &mut dyn toml_edit::TableLike;
    for p in path {
        table = table.get_mut(p)?.as_table_like_mut()?;
    }
    Some(table)
}

/// What `unset` leaves in place of the key.
const UNSET_MARK: &str = "# unset by silvermere config";

/// The `[table]` at `path` itself (not an inline table).
fn table_mut<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    path: &[&str],
) -> Option<&'a mut toml_edit::Table> {
    let mut t = doc.as_table_mut();
    for p in path {
        t = t.get_mut(p)?.as_table_mut()?;
    }
    Some(t)
}

fn text_of(d: &toml_edit::Decor) -> String {
    d.prefix().and_then(|p| p.as_str()).unwrap_or("").to_owned()
}

/// Where the comment lines after a table's last key are kept: in the next header's
/// prefix (`[name]` or the n-th `[[name]]`), or at the end of the file.
enum After {
    Header(Vec<String>, Option<usize>),
    End,
}

fn headers(
    t: &toml_edit::Table,
    path: &mut Vec<String>,
    out: &mut Vec<(isize, Vec<String>, Option<usize>)>,
) {
    for (k, item) in t.iter() {
        path.push(k.to_owned());
        match item {
            toml_edit::Item::Table(sub) => {
                if !sub.is_implicit()
                    && let Some(p) = sub.position()
                {
                    out.push((p, path.clone(), None));
                }
                headers(sub, path, out);
            }
            toml_edit::Item::ArrayOfTables(a) => {
                for (i, sub) in a.iter().enumerate() {
                    if let Some(p) = sub.position() {
                        out.push((p, path.clone(), Some(i)));
                    }
                }
            }
            _ => {}
        }
        path.pop();
    }
}

fn after(doc: &mut toml_edit::DocumentMut, parents: &[&str]) -> After {
    let here = if parents.is_empty() {
        Some(isize::MIN)
    } else {
        table_mut(doc, parents).and_then(|t| t.position())
    };
    let Some(here) = here else {
        return After::End;
    };
    let mut all = Vec::new();
    headers(doc.as_table(), &mut Vec::new(), &mut all);
    all.into_iter()
        .filter(|(p, ..)| *p > here)
        .min_by_key(|(p, ..)| *p)
        .map_or(After::End, |(_, path, i)| After::Header(path, i))
}

fn header_decor<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    path: &[String],
    i: Option<usize>,
) -> Option<&'a mut toml_edit::Decor> {
    let mut item = doc.as_item_mut();
    for p in path {
        item = item.as_table_like_mut()?.get_mut(p)?;
    }
    match (item, i) {
        (toml_edit::Item::Table(t), None) => Some(t.decor_mut()),
        (toml_edit::Item::ArrayOfTables(a), Some(i)) => Some(a.get_mut(i)?.decor_mut()),
        _ => None,
    }
}

fn after_text(doc: &mut toml_edit::DocumentMut, a: &After) -> String {
    match a {
        After::End => doc.trailing().as_str().unwrap_or("").to_owned(),
        After::Header(path, i) => header_decor(doc, path, *i)
            .map(|d| text_of(d))
            .unwrap_or_default(),
    }
}

fn set_after_text(doc: &mut toml_edit::DocumentMut, a: &After, text: String) {
    match a {
        After::End => doc.set_trailing(text),
        After::Header(path, i) => {
            if let Some(d) = header_decor(doc, path, *i) {
                d.set_prefix(text);
            }
        }
    }
}

/// The byte range of a commented-out `# leaf = ...` line in `text`, newline included.
fn commented_line(text: &str, leaf: &str) -> Option<(usize, usize)> {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        if let Some(rest) = line.trim_start().strip_prefix('#')
            && let Some(rest) = rest.trim_start().strip_prefix(leaf)
            && rest.trim_start().starts_with('=')
        {
            return Some((start, end));
        }
        start = end;
    }
    None
}

/// Comments `parents.leaf` out, as `# leaf = value  # unset by silvermere config`; the comment
/// lines above it stay where they are.
fn comment_out(doc: &mut toml_edit::DocumentMut, parents: &[&str], leaf: &str) {
    let Some(table) = table_mut(doc, parents) else {
        // An inline table: no room for a comment.
        if let Some(t) = table_at(doc, parents) {
            t.remove(leaf);
        }
        return;
    };
    let next = table
        .iter()
        .map(|(k, _)| k)
        .skip_while(|k| *k != leaf)
        .nth(1)
        .map(str::to_owned);
    let Some((key, item)) = table.remove_entry(leaf) else {
        return;
    };
    let value = item.as_value().map(bare).unwrap_or_default();
    let line = format!(
        "{}# {} = {value}  {UNSET_MARK}\n",
        text_of(key.leaf_decor()),
        key.display_repr()
    );
    if let Some(next) = next
        && let Some(mut k) = table.key_mut(&next)
    {
        let rest = text_of(k.leaf_decor());
        k.leaf_decor_mut().set_prefix(line + &rest);
        return;
    }
    let a = after(doc, parents);
    let rest = after_text(doc, &a);
    set_after_text(doc, &a, line + &rest);
}

/// Adds `parents.leaf = value`. A commented-out `# leaf = ...` line in that table is replaced
/// by the key, so `unset` then `set` leaves one line.
fn insert_new(
    doc: &mut toml_edit::DocumentMut,
    parents: &[&str],
    leaf: &str,
    value: toml_edit::Value,
) {
    let item = toml_edit::Item::Value(value.decorated(" ", ""));
    let Some(table) = table_mut(doc, parents) else {
        if let Some(t) = table_at(doc, parents) {
            t.insert(leaf, item);
        }
        return;
    };
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_owned()).collect();
    for (n, k) in keys.iter().enumerate() {
        let text = table
            .key(k)
            .map(|k| text_of(k.leaf_decor()))
            .unwrap_or_default();
        if let Some((start, end)) = commented_line(&text, leaf) {
            // Re-insert the keys from here on so the new one takes the commented line's place.
            let tail: Vec<_> = keys[n..]
                .iter()
                .filter_map(|k| table.remove_entry(k))
                .collect();
            let mut new_key = toml_edit::Key::new(leaf);
            new_key.leaf_decor_mut().set_prefix(&text[..start]);
            table.insert_formatted(&new_key, item);
            for (i, (mut k, item)) in tail.into_iter().enumerate() {
                if i == 0 {
                    k.leaf_decor_mut().set_prefix(&text[end..]);
                }
                table.insert_formatted(&k, item);
            }
            return;
        }
    }
    let a = after(doc, parents);
    let text = after_text(doc, &a);
    let mut new_key = toml_edit::Key::new(leaf);
    if let Some((start, end)) = commented_line(&text, leaf) {
        set_after_text(doc, &a, text[end..].to_owned());
        new_key.leaf_decor_mut().set_prefix(&text[..start]);
    }
    if let Some(t) = table_mut(doc, parents) {
        t.insert_formatted(&new_key, item);
    }
}

/// `config set` (`value` given) and `config unset`.
fn set(
    data_dir_flag: Option<&Path>,
    ov: &Overrides,
    target: &Path,
    key: &str,
    value: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(why) = secret_refusal(key, value.is_none()) {
        anyhow::bail!(why);
    }
    anyhow::ensure!(
        is_known(key),
        "unknown key `{key}`; `silvermere config show` lists them all"
    );
    let (_, mut doc) = read_doc(target)?;
    let default = defaults()
        .get(key)
        .map_or_else(|| "(not set)".to_owned(), toml::Value::to_string);
    let old = get(&doc, key).map(bare);
    let parts: Vec<&str> = key.split('.').collect();
    let (leaf, parents) = parts.split_last().expect("a key has a name");
    let new_shown = match value {
        Some(raw) => {
            let mut table = doc.as_table_mut() as &mut dyn toml_edit::TableLike;
            for p in parents {
                let entry = table.entry(p).or_insert_with(|| {
                    let mut t = toml_edit::Table::new();
                    t.set_implicit(true);
                    toml_edit::Item::Table(t)
                });
                table = entry.as_table_like_mut().with_context(|| {
                    format!("{key}: `{p}` in {} is not a table", target.display())
                })?;
            }
            let mut new = parse_value(raw);
            let shown = bare(&new);
            // In place, so the key keeps the comments above it and the value the one after.
            match table.get_mut(leaf) {
                Some(item) => {
                    if let Some(old) = item.as_value() {
                        *new.decor_mut() = old.decor().clone();
                    }
                    *item = toml_edit::Item::Value(new);
                }
                None => insert_new(&mut doc, parents, leaf, new),
            }
            shown
        }
        None => {
            if old.is_none() {
                println!(
                    "{key} is not set in {}; it is already {default} (default)",
                    shown_path(target)
                );
                return Ok(());
            }
            comment_out(&mut doc, parents, leaf);
            format!("{default} (default)")
        }
    };
    let text = doc.to_string();
    Config::check_file_text(&text).context("not written")?;
    write_file(target, &text, &lookup_data_dir(data_dir_flag))?;
    println!(
        "{key}: {} -> {new_shown}",
        old.unwrap_or_else(|| format!("{default} (default)"))
    );
    println!("wrote {}", shown_path(target));
    let file = Some(target).filter(|t| t.exists());
    match sources(data_dir_flag, ov, file).get(key) {
        Some(Source::Env(var)) => println!(
            "note: {var} is set and overrides the file; the server keeps its value until you remove it"
        ),
        Some(Source::Flag) => println!("note: a flag on this command line overrides the file"),
        _ => {}
    }
    println!("{RESTART}");
    warn_outside_service(target);
    Ok(())
}

/// The path as the sysop would type it.
fn shown_path(p: &Path) -> String {
    std::path::absolute(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .display()
        .to_string()
}

fn write_error(target: &Path, e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        let how = if cfg!(windows) {
            "run it as Administrator"
        } else {
            "run with sudo"
        };
        anyhow::anyhow!(
            "cannot write {}: permission denied; {how}",
            target.display()
        )
    } else {
        anyhow::anyhow!("cannot write {}: {e}", target.display())
    }
}

/// The owner a new settings file gets: the data directory's, which the `--system` install
/// gives the service's user (the folder above it may be root's), else the folder's.
#[cfg(unix)]
fn new_file_owner(data_dir: Option<(u32, u32)>, folder: Option<(u32, u32)>) -> Option<(u32, u32)> {
    data_dir.or(folder)
}

/// Writes `text` to `target` through a file beside it, synced and then renamed over it, so a
/// crash leaves the old file or the new one. An existing file keeps its mode and (when we
/// may) its owner; a new one is mode 600 and gets `new_file_owner`, so a file made with
/// sudo stays readable by the service's user.
fn write_file(target: &Path, text: &str, data_dir: &Path) -> anyhow::Result<()> {
    let fail = |e| write_error(target, e);
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let existing = std::fs::metadata(target).ok();
    let name = target.file_name().map_or("silvermere.toml".into(), |n| {
        n.to_string_lossy().into_owned()
    });
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        opts.mode(
            existing
                .as_ref()
                .map_or(0o600, |m| m.permissions().mode() & 0o7777),
        );
    }
    let written = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        if let Some(m) = &existing {
            // The umask may have narrowed the mode given at open.
            std::fs::set_permissions(&tmp, m.permissions())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let owner = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.uid(), m.gid()));
            let want = match &existing {
                Some(m) => Some((m.uid(), m.gid())),
                None => new_file_owner(owner(data_dir), owner(dir)),
            };
            if let (Some((uid, gid)), Ok(m)) = (want, f.metadata())
                && (uid, gid) != (m.uid(), m.gid())
            {
                // Only root may give a file away; anyone else keeps it.
                let _ = std::os::unix::fs::chown(&tmp, Some(uid), Some(gid));
            }
        }
        #[cfg(not(unix))]
        let _ = data_dir;
        f.sync_all()?;
        std::fs::rename(&tmp, target)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}

/// The `WorkingDirectory=` of a systemd unit.
fn unit_workdir(unit: &str) -> Option<std::path::PathBuf> {
    unit.lines()
        .find_map(|l| l.trim().strip_prefix("WorkingDirectory="))
        .map(|d| std::path::PathBuf::from(d.trim().trim_matches('"')))
}

/// Under sudo, a file outside the system service's folder is not the one the service reads.
fn service_warning(target: &Path, unit: Option<&str>) -> Option<String> {
    let workdir = unit_workdir(unit?)?;
    let target = std::path::absolute(target).ok()?;
    (!target.starts_with(&workdir)).then(|| {
        format!(
            "warning: the silvermere service runs in {} and does not read {}; run this from {0}",
            workdir.display(),
            target.display()
        )
    })
}

fn warn_outside_service(target: &Path) {
    if std::env::var_os("SUDO_USER").is_none() {
        return;
    }
    let unit = std::fs::read_to_string("/etc/systemd/system/silvermere.service").ok();
    if let Some(w) = service_warning(target, unit.as_deref()) {
        eprintln!("{w}");
    }
}

/// `$VISUAL`, else `$EDITOR`, else notepad on Windows and vi elsewhere.
fn editor() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.into())
}

const NEW_FILE: &str = "# silvermere settings: only the keys you change.\n\
                        # `silvermere config show` lists every key; docs/config.md describes them.\n";

/// `config edit`: the edit happens on a copy, which replaces the file only once it is valid.
fn edit(target: &Path, data_dir: &Path) -> anyhow::Result<ExitCode> {
    let original = match std::fs::read_to_string(target) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(write_error(target, e)),
    };
    let name = target.file_name().map_or("silvermere.toml".into(), |n| {
        n.to_string_lossy().into_owned()
    });
    let copy = target.with_file_name(format!(".{name}.edit.toml"));
    let _ = std::fs::remove_file(&copy);
    write_file(&copy, original.as_deref().unwrap_or(NEW_FILE), data_dir)?;
    let editor = editor();
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or("vi");
    let args: Vec<&str> = words.collect();
    let stdin = std::io::stdin();
    let mut answers = stdin.lock();
    loop {
        let status = std::process::Command::new(program)
            .args(&args)
            .arg(&copy)
            .status()
            .with_context(|| {
                format!("could not start the editor `{editor}`; set VISUAL or EDITOR")
            })?;
        if !status.success() {
            let _ = std::fs::remove_file(&copy);
            anyhow::bail!(
                "the editor `{editor}` failed ({status}); {} is unchanged",
                shown_path(target)
            );
        }
        let text = std::fs::read_to_string(&copy).map_err(|e| write_error(&copy, e))?;
        match Config::check_file_text(&text) {
            Ok(_) => {
                let _ = std::fs::remove_file(&copy);
                if original.as_deref() == Some(text.as_str()) {
                    println!("no change to {}", shown_path(target));
                } else {
                    write_file(target, &text, data_dir)?;
                    println!("wrote {}", shown_path(target));
                    println!("{RESTART}");
                    warn_outside_service(target);
                }
                return Ok(ExitCode::SUCCESS);
            }
            Err(e) => {
                eprintln!("the file is not valid: {e:#}");
                eprint!("edit it again? [Y/n] ");
                let _ = std::io::stderr().flush();
                let mut answer = String::new();
                let again = answers.read_line(&mut answer).is_ok_and(|n| n > 0)
                    && !answer.trim().eq_ignore_ascii_case("n");
                if !again {
                    eprintln!(
                        "not saved: {} is unchanged; your edits are in {}",
                        shown_path(target),
                        shown_path(&copy)
                    );
                    return Ok(ExitCode::from(1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new file goes to the data directory's owner (the service's user on the `--system`
    /// install, where the folder above it is root's), else the folder's.
    #[cfg(unix)]
    #[test]
    fn a_new_file_gets_the_data_directorys_owner() {
        assert_eq!(
            new_file_owner(Some((990, 990)), Some((0, 0))),
            Some((990, 990))
        );
        assert_eq!(new_file_owner(None, Some((1000, 1000))), Some((1000, 1000)));
        assert_eq!(new_file_owner(None, None), None);
    }

    #[test]
    fn warns_when_the_file_is_not_the_services() {
        let unit = "[Service]\nUser=silvermere\nWorkingDirectory=/opt/silvermere\nExecStart=x\n";
        assert_eq!(unit_workdir(unit), Some("/opt/silvermere".into()));
        assert!(
            service_warning(Path::new("/opt/silvermere/silvermere.toml"), Some(unit)).is_none()
        );
        assert!(
            service_warning(
                Path::new("/opt/silvermere/data/silvermere.toml"),
                Some(unit)
            )
            .is_none()
        );
        let w = service_warning(Path::new("/root/silvermere.toml"), Some(unit)).unwrap();
        assert!(w.contains("runs in /opt/silvermere"), "{w}");
        assert!(service_warning(Path::new("/root/silvermere.toml"), None).is_none());
    }

    #[test]
    fn env_names_map_to_keys() {
        assert_eq!(
            env_key("SILVERMERE_GAME__DEATH_HP").as_deref(),
            Some("game.death_hp")
        );
        assert_eq!(env_key("SILVERMERE_DATA_DIR").as_deref(), Some("data_dir"));
        assert_eq!(
            env_key("SILVERMERE_DATABASE_URL").as_deref(),
            Some("database.url")
        );
        assert_eq!(
            env_key("SILVERMERE_ADMIN_TOKEN").as_deref(),
            Some("admin.tokens")
        );
        assert_eq!(env_key("OTHER"), None);
    }

    #[test]
    fn values_read_as_serve_set_reads_them() {
        assert_eq!(bare(&parse_value("-5")), "-5");
        assert_eq!(bare(&parse_value("0.0.0.0:23")), "\"0.0.0.0:23\"");
        assert_eq!(bare(&parse_value("[\"all\"]")), "[\"all\"]");
    }

    #[test]
    fn known_keys_come_from_the_defaults() {
        assert!(is_known("game.death_hp") && is_known("game.rng_seed") && is_known("data_dir"));
        assert!(!is_known("game.nope") && !is_known("game"));
    }
}
