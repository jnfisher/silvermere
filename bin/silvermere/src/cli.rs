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
        /// Print the server's JSON answer as it came.
        #[arg(long, global = true)]
        json: bool,
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
    /// The web pages (documentation, FAQ, settings) without a server.
    Web {
        #[command(subcommand)]
        action: crate::commands::web::WebCmd,
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
    /// List the players in the game.
    Who,
    /// Send a line to every player in the game.
    Broadcast { message: String },
    /// Read the static world tables again from the database.
    ReloadWorld,
    /// Stop the server after warning the players in the game; every character is saved.
    Shutdown {
        /// How long until the stop: `90s`, `5m`, `1h` or `1h30m`, or a number of seconds;
        /// at most 24h
        #[arg(long = "in", value_name = "DURATION", default_value = "5m", value_parser = parse_delay)]
        delay: u64,
        /// Call off the planned shutdown instead
        #[arg(long, conflicts_with = "message")]
        cancel: bool,
        /// Said to the players with each warning, such as `upgrading to 0.2`
        message: Option<String>,
    },
    /// Restart the server after warning the players in the game; every character is saved,
    /// a backup taken, and the server comes back in the same process.
    Restart {
        /// How long until the restart: `90s`, `5m`, `1h` or `1h30m`, or a number of seconds;
        /// at most 24h
        #[arg(long = "in", value_name = "DURATION", default_value = "5m", value_parser = parse_delay)]
        delay: u64,
        /// Call off the planned restart instead
        #[arg(long, conflicts_with = "message")]
        cancel: bool,
        /// Said to the players with each warning, such as `upgrading to 1.0`
        message: Option<String>,
    },
    /// Take a backup now on the running server, as a scheduled one is taken.
    Backup,
    /// Disconnect a player, saved, telling him why.
    Kick {
        /// A character's first name or an account's username
        player: String,
        /// Told to the player
        reason: Option<String>,
    },
    /// Send a line to one player.
    Tell {
        /// A character's first name or an account's username
        player: String,
        message: String,
    },
    /// Stop a player using gossip and auction.
    Mute(SanctionArgs),
    /// Lift a player's mute.
    Unmute {
        /// A character's first name or an account's username
        player: String,
    },
    /// Hold a player in the jail room.
    Jail(SanctionArgs),
    /// Let a player out of jail, to town.
    Release {
        /// A character's first name or an account's username
        player: String,
    },
    /// Refuse a player's logins; one in the game is disconnected. Not for BBS door accounts.
    Ban(SanctionArgs),
    /// Lift a player's ban.
    Unban {
        /// A character's first name or an account's username
        player: String,
    },
    /// Refuse telnet connections from an address or CIDR block (not the rlogin door).
    #[command(name = "ban-ip")]
    BanIp {
        /// An address or CIDR block: 203.0.113.7, 203.0.113.0/24, 2001:db8::/32
        network: String,
        /// How long: `30m`, `12h`, `7d`, or seconds; without it, until lifted
        #[arg(long = "for", value_name = "DURATION", value_parser = parse_for)]
        for_secs: Option<u64>,
        /// Kept for the sysops; the caller is not told
        reason: Option<String>,
    },
    /// Lift the ban of an address or CIDR block.
    #[command(name = "unban-ip")]
    UnbanIp {
        /// The address or CIDR block as it was banned
        network: String,
    },
    /// List the bans, jails and mutes in force, the IP bans, and whether the realm is closed.
    Moderation,
    /// Close the realm to all but sysops, or open it again.
    Realm {
        #[command(subcommand)]
        action: RealmCmd,
    },
}

/// One line of the web console's command line: the `sysop` subcommands, without the
/// program's name.
#[derive(Debug, Parser)]
#[command(
    name = "sysop",
    no_binary_name = true,
    disable_version_flag = true,
    about = "The sysop's commands. Type a command, or help COMMAND."
)]
pub struct ConsoleLine {
    /// Print the server's JSON answer as it came.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: SysopCmd,
}

/// `sysop mute|jail|ban` arguments.
#[derive(Debug, Args)]
pub struct SanctionArgs {
    /// A character's first name or an account's username
    pub player: String,
    /// How long: `30m`, `12h`, `7d`, or seconds; without it, until lifted
    #[arg(long = "for", value_name = "DURATION", value_parser = parse_for)]
    pub for_secs: Option<u64>,
    /// Told to the player
    pub reason: Option<String>,
}

/// `sysop realm`.
#[derive(Debug, Subcommand)]
pub enum RealmCmd {
    /// Disconnect every player but the sysops, saved, and refuse their logins.
    Close {
        /// Told to the players and to whoever is refused
        reason: Option<String>,
    },
    /// Let everyone log in again.
    Open,
}

/// [`parse_delay`] with days (`7d`) too, for the sanctions: at least a second.
pub fn parse_for(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let bad = || format!("`{text}` is not a duration: use 30m, 12h, 7d, 1d12h or seconds");
    let mut days: u64 = 0;
    let rest = match text.split_once(['d', 'D']) {
        Some((d, rest)) => {
            days = d.parse().map_err(|_| bad())?;
            rest
        }
        None => text,
    };
    let rest_secs = if rest.is_empty() {
        0
    } else {
        parse_delay(rest).map_err(|_| bad())?
    };
    let secs = days
        .checked_mul(86_400)
        .and_then(|d| d.checked_add(rest_secs))
        .ok_or_else(bad)?;
    if secs == 0 {
        return Err(bad());
    }
    Ok(secs)
}

/// `90s`, `5m`, `1h`, `1h30m` or a plain number of seconds, as seconds.
pub fn parse_delay(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if let Ok(secs) = text.parse::<u64>() {
        return Ok(secs);
    }
    let bad = || format!("`{text}` is not a duration: use 90s, 5m, 1h, 1h30m or seconds");
    let mut total: u64 = 0;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let n: u64 = digits.parse().map_err(|_| bad())?;
        digits.clear();
        let unit = match c.to_ascii_lowercase() {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return Err(bad()),
        };
        total = n
            .checked_mul(unit)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(bad)?;
    }
    if !digits.is_empty() || text.is_empty() {
        return Err(bad());
    }
    Ok(total)
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
