//! The clap tree. Subcommand bodies live in `commands/`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::{LogFormat, Overrides};

/// silvermere: a MajorMUD 1.11p server.
#[derive(Debug, Parser)]
#[command(
    name = "silvermere",
    version,
    after_help = "With no subcommand (as when double-clicked): builds the stock world into an empty \
                  database, asks for a sysop account if there is none, then serves."
)]
pub struct Cli {
    /// Config file (default ./silvermere.toml, then <data-dir>/silvermere.toml).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Data directory (default ./data).
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    /// Log format: pretty or json.
    #[arg(long, global = true, value_parser = parse_format)]
    pub log_format: Option<LogFormat>,
    /// Log filter, e.g. info or silvermere=debug.
    #[arg(long, global = true)]
    pub log_level: Option<String>,
    /// None: the first start a double-click runs (`commands::first_start`, T-109).
    #[command(subcommand)]
    pub command: Option<Command>,
}

fn parse_format(s: &str) -> Result<LogFormat, String> {
    match s {
        "pretty" => Ok(LogFormat::Pretty),
        "json" => Ok(LogFormat::Json),
        _ => Err("expected pretty or json".into()),
    }
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server.
    Serve(ServeArgs),
    /// Import legacy world files into the database.
    Import(ImportArgs),
    /// Apply schema migrations.
    Migrate,
    /// Probe the admin API's /healthz (for container health checks).
    Healthcheck(crate::commands::healthcheck::HealthcheckArgs),
    /// Manage accounts.
    Account {
        #[command(subcommand)]
        action: AccountCmd,
    },
    /// Write a consistent copy of the database (safe while the server runs).
    Backup(BackupArgs),
    /// Put a backup back (stop the server first).
    Restore(RestoreArgs),
    /// Admin API bearer tokens.
    Admin {
        #[command(subcommand)]
        action: AdminCmd,
    },
    /// Admin commands over the admin API.
    Sysop {
        #[command(subcommand)]
        action: SysopCmd,
    },
    /// The first start without serving: builds the stock world into an empty database and
    /// asks for a sysop account if there is none, and on a first install how to back up
    /// (install.sh runs it).
    Setup(SetupArgs),
    /// Show and change the settings file (silvermere.toml).
    #[command(name = "config")]
    Settings {
        #[command(subcommand)]
        action: crate::commands::config::ConfigCmd,
    },
    /// The systemd service (Linux): install or uninstall.
    Service {
        #[command(subcommand)]
        action: crate::commands::service::ServiceCmd,
    },
}

/// `serve` flags.
#[derive(Debug, Default, Args)]
pub struct ServeArgs {
    /// Telnet listen address.
    #[arg(long)]
    pub telnet_bind: Option<String>,
    /// Admin API listen address.
    #[arg(long)]
    pub admin_bind: Option<String>,
    /// Database URL (sqlite:// or postgres://).
    #[arg(long)]
    pub database_url: Option<String>,
    /// Print the effective configuration as TOML and exit.
    #[arg(long)]
    pub print_config: bool,
    /// Override game.inactive_secs.
    #[arg(long = "game.inactive-secs")]
    pub game_inactive_secs: Option<u32>,
    /// Override game.death_hp.
    #[arg(long = "game.death-hp", allow_negative_numbers = true)]
    pub game_death_hp: Option<i32>,
    /// Seed the game RNG (game.rng_seed); without it the seed comes from the OS.
    #[arg(long, value_name = "N")]
    pub seed: Option<u64>,
    /// Override any key: `--set game.max_top=20` (value is TOML).
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

/// `setup` flags.
#[derive(Debug, Default, Args)]
pub struct SetupArgs {
    /// Ask nothing about backups: the defaults (daily at 04:30, keeping 30) stand.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// `backup` flags.
#[derive(Debug, Args)]
pub struct BackupArgs {
    /// Where to write (default <data dir>/backups/silvermere-YYYYMMDD-HHMMSS.db, UTC).
    #[arg(long)]
    pub to: Option<PathBuf>,
    /// Delete the oldest backups beyond N in the destination folder (default backup.keep
    /// when writing to the backups folder).
    #[arg(long, value_name = "N", value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..))]
    pub keep: Option<usize>,
    /// Show the backups, their sizes, the schedule and its last run; makes none.
    #[arg(long, conflicts_with_all = ["to", "keep"])]
    pub list: bool,
}

/// `restore` flags.
#[derive(Debug, Args)]
pub struct RestoreArgs {
    /// The backup to restore.
    pub file: PathBuf,
    /// Required for PostgreSQL, where the restore replaces objects in place.
    #[arg(long)]
    pub force: bool,
}

/// `import` flags (T-008/T-009): defined with their body in [`crate::cmd_import`].
pub use crate::cmd_import::ImportCmd as ImportArgs;

/// `account` subcommands (T-013).
#[derive(Debug, Subcommand)]
pub enum AccountCmd {
    /// Create an account.
    Create {
        name: String,
        #[arg(long)]
        sysop: bool,
        #[arg(long)]
        gameop: bool,
        #[arg(long)]
        password_stdin: bool,
    },
    /// Change a password (read from stdin), or with --missing give every account that has
    /// none a random one.
    Passwd {
        #[arg(required_unless_present = "missing")]
        name: Option<String>,
        /// Every account with no password (as an import creates them) gets a random one;
        /// prints `name<TAB>password` lines.
        #[arg(long, conflicts_with = "name")]
        missing: bool,
        /// With --missing: list the accounts that would get one, change nothing.
        #[arg(long, requires = "missing")]
        dry_run: bool,
        /// With --missing: write the lines to FILE (created with mode 600; refused if it
        /// exists) instead of stdout.
        #[arg(
            long,
            requires = "missing",
            conflicts_with = "dry_run",
            value_name = "FILE"
        )]
        out: Option<PathBuf>,
    },
    /// Grant a role.
    Grant { name: String, role: String },
    /// Revoke a role.
    Revoke { name: String, role: String },
    /// List accounts.
    List,
    /// Delete an account.
    Delete {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

/// `admin` subcommands (T-060).
#[derive(Debug, Subcommand)]
pub enum AdminCmd {
    /// Manage admin API tokens.
    Token {
        #[command(subcommand)]
        action: TokenCmd,
    },
}

/// `admin token` subcommands.
#[derive(Debug, Subcommand)]
pub enum TokenCmd {
    /// Make a token: prints it once, with the `[[admin.tokens]]` block to paste.
    New {
        name: String,
        /// Comma-separated: read, write, admin.
        #[arg(long, value_delimiter = ',', default_value = "read")]
        scopes: Vec<String>,
    },
    /// List the configured tokens (names, scopes, hash prefixes; never a token).
    List,
}

/// `sysop` subcommands (T-018).
#[derive(Debug, Subcommand)]
pub enum SysopCmd {
    /// Who is online.
    Who,
    /// Broadcast a message.
    Broadcast { message: String },
    /// Reload static world tables.
    ReloadWorld,
}

impl Cli {
    /// Flag-layer overrides for the config.
    pub fn overrides(&self) -> Overrides {
        let mut lines = Vec::new();
        if let Some(f) = self.log_format {
            let v = if f == LogFormat::Json {
                "json"
            } else {
                "pretty"
            };
            lines.push(format!("logging.format = \"{v}\""));
        }
        if let Some(l) = &self.log_level {
            lines.push(format!("logging.level = {}", quote(l)));
        }
        if let Some(Command::Import(i)) = &self.command
            && let Some(v) = &i.database_url
        {
            lines.push(format!("database.url = {}", quote(v)));
        }
        if let Some(Command::Serve(s)) = &self.command {
            if let Some(v) = &s.telnet_bind {
                lines.push(format!("server.telnet_bind = {}", quote(v)));
            }
            if let Some(v) = &s.admin_bind {
                lines.push(format!("server.admin_bind = {}", quote(v)));
            }
            if let Some(v) = &s.database_url {
                lines.push(format!("database.url = {}", quote(v)));
            }
            if let Some(v) = s.game_inactive_secs {
                lines.push(format!("game.inactive_secs = {v}"));
            }
            if let Some(v) = s.game_death_hp {
                lines.push(format!("game.death_hp = {v}"));
            }
            if let Some(v) = s.seed {
                lines.push(format!("game.rng_seed = {v}"));
            }
            for kv in &s.set {
                lines.push(match kv.split_once('=') {
                    Some((k, v)) if toml::from_str::<toml::Table>(kv).is_ok() => {
                        format!("{} = {}", k.trim(), v.trim())
                    }
                    Some((k, v)) => format!("{} = {}", k.trim(), quote(v.trim())),
                    None => kv.clone(),
                });
            }
        }
        Overrides {
            config_file: self.config.clone(),
            flag_lines: lines,
        }
    }
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}
