//! The web pages' two doors into the server (`silvermere_web`): the settings file, read
//! and written as `silvermere config` does and loaded into the running game as the
//! in-game `SYSOP RELOAD` does, and the sysop's sign-in against the accounts.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use silvermere_auth::{AccountStore as _, Clock as _, Lockout, LockoutConfig, Role, has_role};
use silvermere_db::Db;
use silvermere_web::{
    Change, Kind, Lock, LoginOutcome, SettingValue, Settings, SettingsView, Source, SysopLogin,
};

use crate::commands::config::{self as file, Row};
use crate::config::{Config, Overrides};

/// The `[game]` keys the running game reads only at startup: the cycle periods and the
/// inactivity check (the scheduler's), the monster cache's size, the RNG seed, the gate's
/// copy of `open_signup`, and the cleanup and backup schedule's time zone. Loading the
/// game options leaves these as they were; a new value needs a restart.
pub const GAME_KEYS_AT_STARTUP: &[&str] = &[
    "game.inactive_secs",
    "game.rng_seed",
    "game.characters_per_account",
    "game.open_signup",
    "game.addons",
    "game.sec_create",
    "game.sec_oth",
    "game.sec_slow",
    "game.sec_med",
    "game.sec_fast",
    "game.sec_energy",
    "game.cleanup.at",
    "game.cleanup.tz",
    "game.buffers.monsbuf",
];

/// Whether a saved change to `key` can reach the running game without a restart.
pub fn loads_live(key: &str) -> bool {
    key.starts_with("game.") && !GAME_KEYS_AT_STARTUP.contains(&key)
}

/// `new` with every startup-only key ([`GAME_KEYS_AT_STARTUP`]) as `running` has it, so
/// the engine's copy of the options says what is in force.
pub fn keep_startup_values(
    mut new: silvermere_types::config::GameConfig,
    running: &silvermere_types::config::GameConfig,
) -> silvermere_types::config::GameConfig {
    new.inactive_secs = running.inactive_secs;
    new.rng_seed = running.rng_seed;
    new.characters_per_account = running.characters_per_account;
    new.open_signup = running.open_signup;
    new.addons = running.addons.clone();
    new.sec_create = running.sec_create;
    new.sec_oth = running.sec_oth;
    new.sec_slow = running.sec_slow;
    new.sec_med = running.sec_med;
    new.sec_fast = running.sec_fast;
    new.sec_energy = running.sec_energy;
    new.cleanup = running.cleanup.clone();
    new.buffers = running.buffers.clone();
    new
}

/// The settings file of this server.
pub struct FileSettings {
    data_dir_flag: Option<PathBuf>,
    ov: Overrides,
    /// Every key the running server has loaded, with its value as `--print-config` shows
    /// it: what the server started with, and the game options loaded since.
    running: Mutex<Vec<(String, toml::Value)>>,
    /// The engine's input, for loading the game options; `None` loads nothing.
    engine: Option<std::sync::mpsc::Sender<silvermere_engine::EngineInput>>,
}

/// A config as dotted keys and values, in the order `--print-config` writes them. A table
/// of tables is walked; anything else (an array of tables too) is one value.
fn flatten(cfg: &Config) -> anyhow::Result<Vec<(String, toml::Value)>> {
    fn walk(prefix: &str, table: &toml::Table, out: &mut Vec<(String, toml::Value)>) {
        for (k, v) in table {
            let key = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match v {
                toml::Value::Table(t) => walk(&key, t, out),
                other => out.push((key, other.clone())),
            }
        }
    }
    let table: toml::Table = toml::from_str(&cfg.to_toml()?)?;
    let mut out = Vec::new();
    walk("", &table, &mut out);
    Ok(out)
}

/// The keys whose value differs between `then` and `now`, one present in only one of them
/// included.
fn changed(then: &[(String, toml::Value)], now: &[(String, toml::Value)]) -> Vec<String> {
    let old: std::collections::HashMap<&str, &toml::Value> =
        then.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut keys: Vec<String> = now
        .iter()
        .filter(|(k, v)| old.get(k.as_str()) != Some(&v))
        .map(|(k, _)| k.clone())
        .collect();
    let new: std::collections::HashSet<&str> = now.iter().map(|(k, _)| k.as_str()).collect();
    keys.extend(
        then.iter()
            .filter(|(k, _)| !new.contains(k.as_str()))
            .map(|(k, _)| k.clone()),
    );
    keys
}

impl FileSettings {
    /// The file the command line of this process names; without one (a server started
    /// in-process), the file in `cfg`'s data directory.
    pub fn new(cfg: &Config) -> Self {
        let (data_dir_flag, ov) = match crate::config::invocation() {
            Some((dir, ov)) => (dir.clone(), ov.clone()),
            None => (Some(cfg.data_dir.clone()), Overrides::default()),
        };
        // what the file, the environment and the flags give now, at startup: a later
        // difference is a change the running server has not loaded
        let running = Config::load(data_dir_flag.as_deref(), &ov)
            .and_then(|c| flatten(&c))
            .unwrap_or_default();
        FileSettings {
            data_dir_flag,
            ov,
            running: Mutex::new(running),
            engine: None,
        }
    }

    /// Loads the game options into the engine on `load_now`.
    pub fn with_engine(
        mut self,
        engine: std::sync::mpsc::Sender<silvermere_engine::EngineInput>,
    ) -> Self {
        self.engine = Some(engine);
        self
    }

    fn running(&self) -> std::sync::MutexGuard<'_, Vec<(String, toml::Value)>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A value as its field shows it: a string without its quotes, anything else as TOML.
fn shown(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn kind(v: Option<&toml::Value>) -> Kind {
    match v {
        Some(toml::Value::Boolean(_)) => Kind::Bool,
        // a key that is absent by default is a number (`game.rng_seed`)
        Some(toml::Value::Integer(_)) | None => Kind::Integer,
        Some(toml::Value::String(_)) => Kind::Text,
        Some(_) => Kind::Toml,
    }
}

fn setting(row: Row) -> SettingValue {
    let locked = if row.secret {
        Some(Lock::Secret)
    } else if row.key == "data_dir" {
        Some(Lock::DataDir)
    } else {
        match &row.source {
            file::Source::Env(var) => Some(Lock::Env(var.clone())),
            file::Source::Flag => Some(Lock::Flag),
            _ => None,
        }
    };
    SettingValue {
        value: row.value.as_ref().map_or(String::new(), shown),
        default: row.default.as_ref().map_or(String::new(), shown),
        kind: kind(row.default.as_ref().or(row.value.as_ref())),
        source: match row.source {
            file::Source::Default => Source::Default,
            file::Source::File => Source::File,
            file::Source::Env(var) => Source::Env(var),
            file::Source::Flag => Source::Flag,
        },
        locked,
        key: row.key,
    }
}

impl Settings for FileSettings {
    fn view(&self) -> Result<SettingsView, String> {
        let dir = self.data_dir_flag.as_deref();
        let (target, rows) = file::rows(dir, &self.ov).map_err(|e| format!("{e:#}"))?;
        let now = Config::load(dir, &self.ov)
            .and_then(|c| flatten(&c))
            .map_err(|e| format!("{e:#}"))?;
        let (pending_live, pending_restart) = changed(&self.running(), &now)
            .into_iter()
            .partition(|k| self.engine.is_some() && loads_live(k));
        Ok(SettingsView {
            file: target.display().to_string(),
            values: rows.into_iter().map(setting).collect(),
            pending_live,
            pending_restart,
        })
    }

    fn load_now(&self) -> Result<Vec<String>, String> {
        let Some(engine) = &self.engine else {
            return Err("This server cannot load settings while it runs: restart it.".into());
        };
        let cfg =
            Config::load(self.data_dir_flag.as_deref(), &self.ov).map_err(|e| format!("{e:#}"))?;
        let now = flatten(&cfg).map_err(|e| format!("{e:#}"))?;
        let mut running = self.running();
        let loaded: Vec<String> = changed(&running, &now)
            .into_iter()
            .filter(|k| loads_live(k))
            .collect();
        let game = cfg.game;
        engine
            .send(silvermere_engine::EngineInput::Custom {
                session: None,
                run: Box::new(move |w| {
                    let game = keep_startup_values(game, &w.config);
                    w.config = game;
                    tracing::info!("game options loaded from the settings file (web)");
                }),
            })
            .map_err(|_| "The game engine is not running.".to_string())?;
        // the loaded keys now run with their saved values
        for key in &loaded {
            let value = now.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            running.retain(|(k, _)| k != key);
            if let Some(v) = value {
                running.push((key.clone(), v));
            }
        }
        Ok(loaded)
    }

    fn apply(&self, changes: &[Change]) -> Result<(), String> {
        let changes: Vec<(String, Option<String>)> = changes
            .iter()
            .map(|c| (c.key.clone(), c.value.clone()))
            .collect();
        file::apply(self.data_dir_flag.as_deref(), &self.ov, &changes).map_err(|e| format!("{e:#}"))
    }
}

/// Signs a sysop in with his game account. Wrong passwords count towards a lockout of
/// this door only; signing in here never takes the account's place in the game.
pub struct AccountLogin {
    db: Arc<Db>,
    lockout: Lockout,
}

impl AccountLogin {
    pub fn new(db: Arc<Db>) -> Self {
        AccountLogin {
            db,
            lockout: Lockout::new(LockoutConfig::default()),
        }
    }

    async fn check(&self, username: String, password: String) -> LoginOutcome {
        const WRONG: &str = "Unknown account or wrong password.";
        let now = silvermere_auth::SystemClock.now();
        let account = match self.db.by_username(&username).await {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(%e, "web sign-in: reading the account");
                return LoginOutcome::Refused("The accounts cannot be read just now.".into());
            }
        };
        // an unknown name spends the same time as a wrong password
        let hash = account
            .as_ref()
            .map(|a| a.password_hash.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| silvermere_auth::password::dummy_hash().to_owned());
        if let Some(a) = &account
            && let Some(until) = self.lockout.locked_until(a.id, now)
        {
            tracing::info!(target: "audit", account = a.id.0, username = %a.username, "web sign-in refused: locked");
            return LoginOutcome::Refused(format!(
                "Too many wrong passwords. Try again in {} minutes.",
                Lockout::minutes_left(until, now)
            ));
        }
        let right = tokio::task::spawn_blocking(move || {
            silvermere_auth::password::verify(&password, &hash)
        })
        .await
        .unwrap_or(false);
        let Some(a) = account.filter(|a| !a.password_hash.is_empty()) else {
            tracing::info!(target: "audit", username = %username, "web sign-in failed: unknown user or no password");
            return LoginOutcome::Refused(WRONG.into());
        };
        if !right {
            self.lockout.record_failure(a.id, now);
            tracing::info!(target: "audit", account = a.id.0, username = %a.username, "web sign-in failed: wrong password");
            return LoginOutcome::Refused(WRONG.into());
        }
        self.lockout.record_success(a.id);
        if !has_role(&a, Role::Sysop) {
            tracing::info!(target: "audit", account = a.id.0, username = %a.username, "web sign-in refused: not a sysop");
            return LoginOutcome::Refused(
                "That account is not a sysop. The settings are the sysop's.".into(),
            );
        }
        tracing::info!(target: "audit", account = a.id.0, username = %a.username, "web sign-in");
        LoginOutcome::Sysop(a.username)
    }
}

impl SysopLogin for AccountLogin {
    fn login(
        &self,
        username: String,
        password: String,
    ) -> Pin<Box<dyn Future<Output = LoginOutcome> + Send + '_>> {
        Box::pin(self.check(username, password))
    }
}

#[cfg(test)]
mod startup_keys {
    use super::*;

    /// Every startup-only key is a real key, `keep_startup_values` keeps each one, and
    /// docs/web.md lists each one.
    #[test]
    fn the_startup_only_keys_are_kept_and_documented() {
        let running = Config::default();
        let keys: Vec<String> = flatten(&running)
            .unwrap()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        // a file that changes every game option
        let mut new = running.game.clone();
        new.inactive_secs += 1;
        new.rng_seed = Some(42);
        new.characters_per_account += 1;
        new.open_signup = !new.open_signup;
        new.addons = vec!["none".into()];
        new.sec_create += 1;
        new.sec_oth += 1;
        new.sec_slow += 1;
        new.sec_med += 1;
        new.sec_fast += 1;
        new.sec_energy += 1;
        new.cleanup.at = "04:00".into();
        new.cleanup.tz = "UTC".into();
        new.buffers.monsbuf += 1;
        new.max_top += 1;
        let kept = keep_startup_values(new, &running.game);
        let mut after = running.clone();
        after.game = kept;
        let changed = changed(&flatten(&running).unwrap(), &flatten(&after).unwrap());
        assert_eq!(
            changed,
            vec!["game.max_top".to_string()],
            "only the live key moves"
        );
        let doc = include_str!("../../../docs/web.md");
        for key in GAME_KEYS_AT_STARTUP {
            // rng_seed is absent by default, so not among the flattened keys
            assert!(
                keys.iter().any(|k| k == key) || *key == "game.rng_seed",
                "{key} is not a setting"
            );
            assert!(!loads_live(key));
            assert!(
                doc.contains(&format!("`{key}`")),
                "docs/web.md does not list {key}"
            );
        }
        assert!(loads_live("game.max_top"));
        assert!(!loads_live("server.max_sessions"));
    }
}
