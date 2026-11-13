//! The settings as the editor sees them. The binary owns the settings file and its checks;
//! it hands the pages a [`SettingsView`] and takes [`Change`]s back through [`Settings`].

/// What kind of field a value is edited in, from the type of its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Integer,
    /// A string, shown and typed without its quotes.
    Text,
    /// Anything else (a list), typed as TOML.
    Toml,
}

/// Which layer sets a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    /// The `SILVERMERE_*` variable named.
    Env(String),
    Flag,
}

/// One setting as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingValue {
    /// The dotted key, e.g. `game.death_hp`.
    pub key: String,
    /// The value in force, as the field shows it: a string without quotes, anything else as
    /// TOML, empty for a key that is not set.
    pub value: String,
    /// The default, in the same form.
    pub default: String,
    pub kind: Kind,
    pub source: Source,
    /// Why the value cannot be changed here (a secret, or set above the file), if so.
    pub locked: Option<String>,
}

/// Every setting, and the file they are saved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    /// The settings file's path, as the sysop would type it.
    pub file: String,
    /// The settings, in the order the documentation lists them.
    pub values: Vec<SettingValue>,
    /// Whether the file holds changes the running server has not loaded.
    pub pending: bool,
}

/// One change to the settings file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub key: String,
    /// The new value as TOML; `None` takes the key out of the file (back to its default).
    pub value: Option<String>,
}

/// The settings file, as the binary reads, checks and writes it.
pub trait Settings: Send + Sync + 'static {
    /// The settings as they stand.
    fn view(&self) -> Result<SettingsView, String>;
    /// Writes `changes` to the file, all or none: a value the server would refuse at
    /// startup is an error with startup's message, and nothing is written.
    fn apply(&self, changes: &[Change]) -> Result<(), String>;
}

/// A string as a TOML basic string.
fn toml_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What a submitted form of `section` changes: each posted field of an unlocked setting
/// of that section whose text differs from the value in force. A field put back to its
/// default takes the key out of the file.
pub fn changes(view: &SettingsView, section: &str, posted: &[(String, String)]) -> Vec<Change> {
    let mut out = Vec::new();
    for v in &view.values {
        if crate::page::section(&v.key) != section || v.locked.is_some() {
            continue;
        }
        let Some((_, text)) = posted.iter().find(|(k, _)| *k == v.key) else {
            continue;
        };
        let text = text.trim();
        if text == v.value {
            continue;
        }
        let value = if text == v.default || text.is_empty() && v.kind != Kind::Text {
            None
        } else {
            Some(match v.kind {
                Kind::Text => toml_string(text),
                _ => text.to_owned(),
            })
        };
        // back to a default that was never in the file: nothing to take out
        if value.is_none() && v.source == Source::Default {
            continue;
        }
        out.push(Change {
            key: v.key.clone(),
            value,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(key: &str, value: &str, default: &str, kind: Kind, source: Source) -> SettingValue {
        SettingValue {
            key: key.into(),
            value: value.into(),
            default: default.into(),
            kind,
            source,
            locked: None,
        }
    }

    fn view() -> SettingsView {
        let mut secret = value("database.url", "<redacted>", "", Kind::Text, Source::File);
        secret.locked = Some("a secret".into());
        SettingsView {
            file: "silvermere.toml".into(),
            pending: false,
            values: vec![
                value(
                    "game.death_hp",
                    "-200",
                    "-200",
                    Kind::Integer,
                    Source::Default,
                ),
                value("game.max_top", "20", "10", Kind::Integer, Source::File),
                value(
                    "game.open_signup",
                    "true",
                    "true",
                    Kind::Bool,
                    Source::Default,
                ),
                value(
                    "game.usr_cur_name",
                    "runic coins",
                    "runic coins",
                    Kind::Text,
                    Source::Default,
                ),
                value(
                    "server.max_sessions",
                    "256",
                    "256",
                    Kind::Integer,
                    Source::Default,
                ),
                secret,
            ],
        }
    }

    fn post(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn only_what_differs_in_the_posted_section_changes() {
        let posted = post(&[
            ("game.death_hp", "-50"),
            ("game.max_top", "10"),
            ("game.open_signup", "true"),
            ("game.usr_cur_name", "say \"gems\""),
            ("server.max_sessions", "9"),
            ("database.url", "postgres://x"),
        ]);
        assert_eq!(
            changes(&view(), "game", &posted),
            [
                Change {
                    key: "game.death_hp".into(),
                    value: Some("-50".into())
                },
                // back to the default: out of the file
                Change {
                    key: "game.max_top".into(),
                    value: None
                },
                Change {
                    key: "game.usr_cur_name".into(),
                    value: Some("\"say \\\"gems\\\"\"".into())
                },
            ]
        );
        // a locked setting never changes, even when posted
        assert!(changes(&view(), "database", &posted).is_empty());
    }
}
