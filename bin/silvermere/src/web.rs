//! The web pages' two doors into the server (`silvermere_web`): the settings file, read
//! and written as `silvermere config` does, and the sysop's sign-in against the accounts.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use silvermere_auth::{AccountStore as _, Clock as _, Lockout, LockoutConfig, Role, has_role};
use silvermere_db::Db;
use silvermere_web::{
    Change, Kind, LoginOutcome, SettingValue, Settings, SettingsView, Source, SysopLogin,
};

use crate::commands::config::{self as file, Row};
use crate::config::{Config, Overrides};

/// The settings file of this server.
pub struct FileSettings {
    data_dir_flag: Option<PathBuf>,
    ov: Overrides,
    /// The settings as they loaded when the server started, as `--print-config` shows
    /// them.
    running: String,
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
            .and_then(|c| c.to_toml())
            .unwrap_or_default();
        FileSettings {
            data_dir_flag,
            ov,
            running,
        }
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
        Some(
            "Holds a secret, so it is neither shown nor changed here. Change it on the \
             server with the silvermere command (see Configuration)."
                .to_owned(),
        )
    } else if row.key == "data_dir" {
        Some(
            "Where the settings file itself lives. Change it with --data-dir or \
             SILVERMERE_DATA_DIR."
                .to_owned(),
        )
    } else {
        match &row.source {
            file::Source::Env(var) => Some(format!(
                "Set by the environment variable {var}, which wins over the file. Remove \
                 it to change the value here."
            )),
            file::Source::Flag => Some(
                "Set by a flag on the server's command line, which wins over the file.".to_owned(),
            ),
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
            .and_then(|c| c.to_toml())
            .map_err(|e| format!("{e:#}"))?;
        Ok(SettingsView {
            file: target.display().to_string(),
            values: rows.into_iter().map(setting).collect(),
            pending: now != self.running,
        })
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
