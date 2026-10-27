//! The single `Config`: defaults < `silvermere.toml` < `SILVERMERE_*` env < flags.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};
use silvermere_types::config::GameConfig;

/// The whole configuration. Scalars precede tables so the TOML dump is valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Data directory (`silvermere.db`, `logs/`, `backups/`, `legacy/`).
    pub data_dir: PathBuf,
    /// `[server]`.
    pub server: ServerCfg,
    /// `[database]`.
    pub database: DatabaseCfg,
    /// `[net]`.
    pub net: NetCfg,
    /// `[compat]`.
    pub compat: CompatCfg,
    /// `[game]`: the WCCMMUD.MSG and WCCMMUD.INI options that survive.
    pub game: GameConfig,
    /// `[logging]`.
    pub logging: LoggingCfg,
    /// `[backup]`: the scheduled backup and the after-backup command (T-150).
    pub backup: BackupCfg,
    /// `[admin]`: the admin API's tokens (`[[admin.tokens]]`).
    pub admin: AdminCfg,
}

/// `[admin]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdminCfg {
    /// `[[admin.tokens]]`; none means no `/v1` call can authenticate, and a
    /// non-loopback `server.admin_bind` refuses to start.
    pub tokens: Vec<TokenCfg>,
    /// Set by `Config::load` when `SILVERMERE_ADMIN_TOKEN` supplied the `default` token.
    #[serde(skip)]
    pub legacy_env_token: bool,
}

/// The name of the token made from `SILVERMERE_ADMIN_TOKEN`.
pub const LEGACY_TOKEN_NAME: &str = "default";

/// One `[[admin.tokens]]` entry: `sha256` (the hex SHA-256 of the token, what
/// `silvermere admin token new` prints) or, for a hand-written config, the plain `token`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCfg {
    /// Shown in the audit rows.
    pub name: String,
    /// The hex SHA-256 of the bearer secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The bearer secret itself (the `SILVERMERE_ADMIN_TOKEN` token and the T-066 form).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// `read`, `write`, `admin`; absent means `["read"]`.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<TokenScope>,
}

// The secret stays out of `{:?}` (and so out of logs).
impl std::fmt::Debug for TokenCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenCfg")
            .field("name", &self.name)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

fn default_scopes() -> Vec<TokenScope> {
    vec![TokenScope::Read]
}

/// What a token may do (`docs/admin-api.md`); `broadcast` and `sysop` are the T-018 names
/// of `write` and `admin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenScope {
    /// GET routes and the event stream.
    Read,
    /// `/v1/broadcast`, `/v1/reload-world`.
    #[serde(alias = "broadcast")]
    Write,
    /// Everything, the reserved routes included.
    #[serde(alias = "sysop")]
    Admin,
}

impl TokenScope {
    pub fn to_scope(self) -> silvermere_api::Scope {
        match self {
            TokenScope::Read => silvermere_api::Scope::Read,
            TokenScope::Write => silvermere_api::Scope::Write,
            TokenScope::Admin => silvermere_api::Scope::Admin,
        }
    }
}

impl TokenCfg {
    /// The hash of the secret: `sha256` when given, else the hash of `token`.
    pub fn hash(&self) -> Option<[u8; 32]> {
        match (&self.sha256, &self.token) {
            (Some(hex), _) => silvermere_api::auth::parse_sha256_hex(hex),
            (None, Some(secret)) => Some(silvermere_api::auth::sha256(secret)),
            (None, None) => None,
        }
    }

    /// The API's in-memory form (only the SHA-256 of the secret is kept).
    pub fn to_token(&self) -> silvermere_api::Token {
        let scopes: Vec<_> = self.scopes.iter().map(|s| s.to_scope()).collect();
        // `validate` has checked the hash parses.
        silvermere_api::Token::from_hash(&self.name, self.hash().unwrap_or([0; 32]), &scopes)
    }
}

/// `[server]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerCfg {
    /// Telnet listen address.
    pub telnet_bind: String,
    /// Admin API listen address.
    pub admin_bind: String,
    /// Connections beyond this are refused. 1..=100000.
    pub max_sessions: u32,
}

/// `[net]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetCfg {
    /// Seconds a connection may sit in the login dialogue (before it has an account)
    /// without typing a line before it is closed; 0 turns the timeout off. Once logged
    /// in, `game.inactive_secs` applies instead.
    pub login_timeout_secs: u32,
}

impl Default for NetCfg {
    fn default() -> Self {
        Self {
            login_timeout_secs: 300,
        }
    }
}

/// `[database]`.
#[derive(Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseCfg {
    /// `sqlite://...` or `postgres://...`; empty means `sqlite://<data_dir>/silvermere.db`.
    pub url: String,
}

// The password stays out of `{:?}` (and so out of logs), as with the admin tokens.
impl std::fmt::Debug for DatabaseCfg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseCfg")
            .field("url", &silvermere_db::redact_url(&self.url))
            .finish()
    }
}

/// `[compat]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompatCfg {
    /// The `Make your selection (X to exit):` BBS menu shim.
    pub bbs_menu: bool,
    /// `cp437` or `utf8`.
    pub charset: Charset,
    /// Version line shown in menus.
    pub version_string: String,
    /// Sysop bulletin text (was WCCSYSOP.BUL).
    pub bulletin: String,
}

/// Wire character set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Charset {
    /// Code page 437.
    Cp437,
    /// UTF-8.
    Utf8,
}

/// `[logging]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingCfg {
    /// `pretty` or `json`.
    pub format: LogFormat,
    /// `tracing` filter directive, e.g. `info`.
    pub level: String,
}

/// `[backup]` (T-150): a daily `silvermere backup` in the server, and an optional command
/// run after each backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupCfg {
    /// Whether the server backs up daily at `at`.
    pub enabled: bool,
    /// Local wall-clock time of the daily backup, `"HH:MM"` (24-hour), in `game.cleanup.tz`.
    pub at: String,
    /// The backups kept in `data/backups/`: older ones are deleted after each backup there.
    pub keep: u32,
    /// A command run after each successful backup, without a shell: `{file}` is the backup's
    /// path, `{dir}` the backups folder. Empty: none.
    pub after: String,
    /// Seconds `after` may run before it is killed.
    pub after_timeout_secs: u32,
}

impl Default for BackupCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            at: "04:30".into(),
            keep: 30,
            after: String::new(),
            after_timeout_secs: 1800,
        }
    }
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human readable.
    Pretty,
    /// One JSON object per line.
    Json,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            server: ServerCfg::default(),
            database: DatabaseCfg::default(),
            net: NetCfg::default(),
            compat: CompatCfg::default(),
            game: GameConfig::default(),
            logging: LoggingCfg::default(),
            backup: BackupCfg::default(),
            admin: AdminCfg::default(),
        }
    }
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            telnet_bind: "0.0.0.0:2323".into(),
            admin_bind: "127.0.0.1:8080".into(),
            max_sessions: 256,
        }
    }
}

impl Default for CompatCfg {
    fn default() -> Self {
        Self {
            bbs_menu: true,
            charset: Charset::Cp437,
            version_string: "1.11p-WG (Dec 30 2005 14:20:05)".into(),
            bulletin: String::new(),
        }
    }
}

impl Default for LoggingCfg {
    fn default() -> Self {
        Self {
            format: LogFormat::Pretty,
            level: "info".into(),
        }
    }
}

/// What the command line contributes before the config is assembled.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    /// `--config`.
    pub config_file: Option<PathBuf>,
    /// Flag-layer TOML lines such as `game.death_hp = -5` (highest precedence).
    pub flag_lines: Vec<String>,
}

/// Parses `SILVERMERE_ADMIN_TOKENS`. serde_json quotes the offending value, which may be a
/// secret, so the error carries the category and position only.
fn parse_admin_tokens(json: &str) -> anyhow::Result<Vec<TokenCfg>> {
    serde_json::from_str(json).map_err(|e| {
        anyhow::anyhow!(
            "SILVERMERE_ADMIN_TOKENS is not a JSON array of {{\"name\", \"sha256\" or \"token\", \"scopes\"}} objects ({:?} error at line {} column {})",
            e.classify(),
            e.line(),
            e.column()
        )
    })
}

/// A TOML syntax error keeps its line and column but not the source line, which may be
/// a secret (`token = s3cr3t` without quotes).
pub(crate) fn toml_syntax_error(msg: &str) -> Option<String> {
    let first = msg
        .lines()
        .next()
        .filter(|l| l.starts_with("TOML parse error at line"))?;
    let place = msg
        .lines()
        .last()
        .map(str::trim)
        .filter(|l| l.starts_with("in "));
    Some(match place {
        Some(place) => format!("{first} {place}"),
        None => first.to_string(),
    })
}

/// Extracts the config, with errors that never quote a secret.
fn extract(fig: &Figment) -> anyhow::Result<Config> {
    fig.extract().map_err(|e| {
        // A wrong-typed `admin.tokens` value is quoted in figment's message; name the
        // key instead.
        if let Some(bad) = e.clone().into_iter().find(|e| e.path.first().is_some_and(|p| p == "admin")) {
            return anyhow::anyhow!(
                "{} has the wrong shape; expected [[admin.tokens]] with string name, sha256 and scopes read/write/admin",
                bad.path.join(".")
            );
        }
        let text = e.to_string();
        toml_syntax_error(&text).map_or_else(|| anyhow::anyhow!("{text}"), anyhow::Error::msg)
    })
}

/// The data directory the file lookup uses: `--data-dir`, else `SILVERMERE_DATA_DIR`, else
/// `./data`.
pub fn lookup_data_dir(data_dir_flag: Option<&Path>) -> PathBuf {
    match data_dir_flag {
        Some(d) => d.to_path_buf(),
        None => std::env::var_os("SILVERMERE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./data")),
    }
}

/// The file [`Config::load`] reads: `--config` (which must exist), else the first of
/// `./silvermere.toml` and `<data-dir>/silvermere.toml` that exists, else none.
pub fn config_file(
    data_dir_flag: Option<&Path>,
    ov: &Overrides,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(p) = &ov.config_file {
        anyhow::ensure!(p.exists(), "config file {} not found", p.display());
        return Ok(Some(p.clone()));
    }
    Ok(lookup_candidates(data_dir_flag)
        .into_iter()
        .find(|p| p.exists()))
}

fn lookup_candidates(data_dir_flag: Option<&Path>) -> [PathBuf; 2] {
    [
        PathBuf::from("./silvermere.toml"),
        lookup_data_dir(data_dir_flag).join("silvermere.toml"),
    ]
}

/// The file `silvermere config` reads and writes: the one [`config_file`] finds, else where
/// to create it. That is `--config`; else, when no file exists, `<data-dir>/silvermere.toml`
/// if the data directory was given (`--data-dir` or `SILVERMERE_DATA_DIR`, as in Docker),
/// else `./silvermere.toml` (the service's working directory, `/opt/silvermere` for the
/// `--system` install).
pub fn config_target(data_dir_flag: Option<&Path>, ov: &Overrides) -> PathBuf {
    if let Some(p) = &ov.config_file {
        return p.clone();
    }
    let [here, in_data] = lookup_candidates(data_dir_flag);
    if here.exists() {
        here
    } else if in_data.exists()
        || data_dir_flag.is_some()
        || std::env::var_os("SILVERMERE_DATA_DIR").is_some()
    {
        in_data
    } else {
        here
    }
}

impl Config {
    /// Assembles the effective configuration and validates it.
    pub fn load(data_dir_flag: Option<&Path>, ov: &Overrides) -> anyhow::Result<Config> {
        let env = || Env::prefixed("SILVERMERE_").split("__");
        let mut fig = Figment::from(Serialized::defaults(Config::default()));
        if let Some(f) = config_file(data_dir_flag, ov)? {
            fig = fig.merge(Toml::file(f));
        }
        fig = fig.merge(env().ignore(&["database_url", "admin_tokens"]));
        // The deployment docs write the flat SILVERMERE_DATABASE_URL for the PG switch.
        if let Ok(url) = std::env::var("SILVERMERE_DATABASE_URL") {
            fig = fig.merge(Serialized::default("database.url", url));
        }
        // SILVERMERE_ADMIN_TOKENS is a JSON array of {name, token, scopes}.
        if let Ok(json) = std::env::var("SILVERMERE_ADMIN_TOKENS") {
            let tokens = parse_admin_tokens(&json)?;
            fig = fig.merge(Serialized::default("admin.tokens", tokens));
        }
        if let Some(d) = data_dir_flag {
            fig = fig.merge(Serialized::default("data_dir", d));
        }
        if !ov.flag_lines.is_empty() {
            fig = fig.merge(Toml::string(&ov.flag_lines.join("\n")));
        }
        let mut cfg = extract(&fig)?;
        // The legacy single token: a token named `default` with every scope.
        if let Ok(secret) = std::env::var("SILVERMERE_ADMIN_TOKEN")
            && !secret.is_empty()
            && !cfg.admin.tokens.iter().any(|t| t.name == LEGACY_TOKEN_NAME)
        {
            cfg.admin.tokens.push(TokenCfg {
                name: LEGACY_TOKEN_NAME.into(),
                sha256: None,
                token: Some(secret),
                scopes: vec![TokenScope::Read, TokenScope::Write, TokenScope::Admin],
            });
            cfg.admin.legacy_env_token = true;
        }
        cfg.finish()?;
        Ok(cfg)
    }

    /// The defaults with `text` (a whole `silvermere.toml`) on top, checked as startup checks
    /// it, with no environment variable or flag: what `config set` and `config edit` write
    /// must stand on its own.
    pub fn check_file_text(text: &str) -> anyhow::Result<Config> {
        let fig = Figment::from(Serialized::defaults(Config::default())).merge(Toml::string(text));
        let mut cfg = extract(&fig)?;
        cfg.finish()?;
        Ok(cfg)
    }

    /// Fills the database URL from the data directory and validates.
    fn finish(&mut self) -> anyhow::Result<()> {
        if self.database.url.is_empty() {
            self.database.url =
                format!("sqlite://{}", self.data_dir.join("silvermere.db").display());
        }
        // T-108: Neon's `channel_binding=require`, which the driver would warn about each time
        self.database.url = silvermere_db::drop_ignored_params(&self.database.url);
        self.validate().map_err(anyhow::Error::msg)
    }

    /// Range checks for every key.
    pub fn validate(&self) -> Result<(), String> {
        for (key, v) in [
            ("server.telnet_bind", &self.server.telnet_bind),
            ("server.admin_bind", &self.server.admin_bind),
        ] {
            v.parse::<SocketAddr>()
                .map_err(|e| format!("{key} = {v:?} is not a host:port address ({e})"))?;
        }
        if !(1..=100_000).contains(&self.server.max_sessions) {
            return Err(format!(
                "server.max_sessions = {} is out of range; allowed 1..=100000",
                self.server.max_sessions
            ));
        }
        let b = &self.backup;
        if silvermere_types::config::hour_minute(&b.at).is_none() {
            return Err(format!(
                "backup.at = {:?} is not a time; allowed \"HH:MM\", 00:00..=23:59",
                b.at
            ));
        }
        if !(1..=10_000).contains(&b.keep) {
            return Err(format!(
                "backup.keep = {} is out of range; allowed 1..=10000",
                b.keep
            ));
        }
        if !(1..=86_400).contains(&b.after_timeout_secs) {
            return Err(format!(
                "backup.after_timeout_secs = {} is out of range; allowed 1..=86400",
                b.after_timeout_secs
            ));
        }
        if let Err(e) = crate::backups::hook::split(&b.after) {
            return Err(format!("backup.after: {e}"));
        }
        for (i, t) in self.admin.tokens.iter().enumerate() {
            if t.name.trim().is_empty() || t.scopes.is_empty() {
                return Err(format!(
                    "admin.tokens[{i}] needs a name and at least one scope"
                ));
            }
            match (&t.sha256, &t.token) {
                (None, None) => {
                    return Err(format!("admin.tokens[{i}] needs a sha256 (or a token)"));
                }
                (Some(hex), _) if silvermere_api::auth::parse_sha256_hex(hex).is_none() => {
                    return Err(format!("admin.tokens[{i}].sha256 must be 64 hex digits"));
                }
                (None, Some(secret)) if secret.is_empty() => {
                    return Err(format!("admin.tokens[{i}].token is empty"));
                }
                _ => {}
            }
        }
        self.game.validate()
    }

    /// The effective configuration as TOML (`--print-config`).
    pub fn to_toml(&self) -> anyhow::Result<String> {
        // Secrets never print.
        let mut shown = self.clone();
        for t in &mut shown.admin.tokens {
            if t.token.is_some() {
                t.token = Some("<redacted>".into());
            }
        }
        shown.database.url = self.database_shown();
        Ok(toml::to_string_pretty(&shown)?)
    }

    /// `database.url` with its password replaced by `****`, for messages (T-108).
    pub fn database_shown(&self) -> String {
        silvermere_db::redact_url(&self.database.url)
    }

    /// The SQLite file URL with create-if-missing, or the PostgreSQL URL unchanged.
    pub fn sqlite_path(&self) -> Option<PathBuf> {
        let rest = self.database.url.strip_prefix("sqlite://")?;
        let rest = rest.split('?').next().unwrap_or(rest);
        (!rest.is_empty()).then(|| PathBuf::from(rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-108: `{:?}` of the config shows the database URL without its password.
    #[test]
    fn debug_hides_the_database_password() {
        let db = DatabaseCfg {
            url: "postgres://sm:hunter2@h/db".into(),
        };
        let shown = format!("{db:?}");
        assert!(
            shown.contains("sm:****@h/db") && !shown.contains("hunter2"),
            "{shown}"
        );
    }

    /// A bad `SILVERMERE_ADMIN_TOKENS` never echoes the secret it was given.
    #[test]
    fn bad_admin_tokens_env_does_not_echo_the_value() {
        for bad in [r#"["s3cret-value"]"#, r#"[{"name":"a","token":12345}]"#] {
            let msg = format!("{:#}", parse_admin_tokens(bad).unwrap_err());
            assert!(
                !msg.contains("s3cret-value") && !msg.contains("12345"),
                "{msg}"
            );
            assert!(msg.contains("SILVERMERE_ADMIN_TOKENS"), "{msg}");
        }
        assert_eq!(
            parse_admin_tokens(r#"[{"name":"a","token":"t"}]"#)
                .unwrap()
                .len(),
            1
        );
    }

    /// A TOML syntax error names line and column but never prints the source line.
    #[test]
    fn toml_syntax_error_does_not_echo_the_source_line() {
        let dir = std::env::temp_dir().join(format!("sm-toml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("silvermere.toml");
        std::fs::write(
            &file,
            "[[admin.tokens]]\nname = \"a\"\ntoken = s3cretvalue\n",
        )
        .unwrap();
        let ov = Overrides {
            config_file: Some(file),
            flag_lines: vec![],
        };
        let msg = format!("{:#}", Config::load(Some(&dir), &ov).unwrap_err());
        assert!(!msg.contains("s3cretvalue"), "{msg}");
        assert!(msg.contains("line 3, column 9"), "{msg}");
    }

    /// A wrong-typed token in the TOML file does not echo it either.
    #[test]
    fn bad_admin_tokens_toml_does_not_echo_the_value() {
        let dir = std::env::temp_dir().join(format!("silvermere-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.toml");
        std::fs::write(&file, "[[admin.tokens]]\nname = \"a\"\ntoken = 98765\n").unwrap();
        let ov = Overrides {
            config_file: Some(file),
            ..Default::default()
        };
        let err = Config::load(Some(&dir), &ov).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            !msg.contains("98765") && msg.contains("admin.tokens"),
            "{msg}"
        );
    }

    /// T-150: a settings file from before `[backup]` existed (an upgraded install) gets the
    /// defaults: backups on, daily at 04:30, keeping 30, no command.
    #[test]
    fn an_older_settings_file_gets_the_backup_defaults() {
        let cfg =
            Config::check_file_text("[game]\nmax_top = 20\n[server]\nmax_sessions = 10\n").unwrap();
        assert_eq!(cfg.backup, BackupCfg::default());
        assert!(cfg.backup.enabled);
        assert_eq!((cfg.backup.at.as_str(), cfg.backup.keep), ("04:30", 30));
        assert_eq!(
            (cfg.backup.after.as_str(), cfg.backup.after_timeout_secs),
            ("", 1800)
        );
    }

    #[test]
    fn backup_settings_are_checked() {
        for (text, want) in [
            ("[backup]\nat = \"4:3\"\n", "backup.at"),
            ("[backup]\nkeep = 0\n", "backup.keep"),
            (
                "[backup]\nafter_timeout_secs = 0\n",
                "backup.after_timeout_secs",
            ),
            (
                "[backup]\nafter = \"rclone copy '{file}\"\n",
                "backup.after",
            ),
        ] {
            let e = format!("{:#}", Config::check_file_text(text).unwrap_err());
            assert!(e.contains(want), "{text}: {e}");
        }
        assert!(Config::check_file_text("[backup]\nat = \"23:59\"\nkeep = 1\n").is_ok());
    }
}
