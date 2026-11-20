# Command-line tools

Every `silvermere` command. The usage lines and flags are the output of `silvermere <command>
--help`; `cargo test -p silvermere --test all docs_in_sync::` fails when a command or flag is
missing from this page. Run `silvermere --help` for the same list from the binary.

With no command (as when double-clicked), `silvermere` builds the stock world into an empty
database, asks for a sysop account if there is none, then serves.

| Command | What it does | While a server runs on the same store |
|---|---|---|
| [`serve`](#silvermere-serve) | run the server | (is the server) |
| [`setup`](#silvermere-setup) | first start without serving | no |
| [`service`](#silvermere-service) | install or remove the systemd service | n/a |
| [`config`](#silvermere-config) | show and change `silvermere.toml` | yes (a running server reads it at start) |
| [`account`](#silvermere-account) | create accounts, change passwords, grant roles | yes |
| [`import`](#silvermere-import) | import legacy world files | no, it refuses |
| [`backup`](#silvermere-backup) | write a consistent copy of the database | yes |
| [`restore`](#silvermere-restore) | put a backup back | no, it refuses |
| [`migrate`](#silvermere-migrate) | apply schema migrations | no, it refuses |
| [`admin token`](#silvermere-admin-token) | make admin API tokens | yes |
| [`sysop`](#silvermere-sysop) | sysop commands over the admin API: who, broadcast, kick, ban, jail, mute, close the realm, shutdown | needs one running |
| [`healthcheck`](#silvermere-healthcheck) | probe the admin API's `/healthz` | needs one running |

## Global flags

Every command takes these, and they go before or after the command name.

| Flag | Meaning |
|---|---|
| `--config <CONFIG>` | config file (default `./silvermere.toml`, then `<data-dir>/silvermere.toml`) |
| `--data-dir <DATA_DIR>` | data directory (default `./data`) |
| `--log-format <LOG_FORMAT>` | log format: `pretty` or `json` |
| `--log-level <LOG_LEVEL>` | log filter, for example `info` or `silvermere=debug` |
| `-h`, `--help` | print help |
| `-V`, `--version` | print the version (on `silvermere` alone) |

[config.md](config.md) has the settings these override and their environment variables.

## `silvermere serve`

Run the server. Details: [install.md](install.md), [config.md](config.md).

```
silvermere serve [OPTIONS]
```

| Flag | Meaning |
|---|---|
| `--telnet-bind <TELNET_BIND>` | telnet listen address |
| `--admin-bind <ADMIN_BIND>` | admin API listen address |
| `--database-url <DATABASE_URL>` | database URL (`sqlite://` or `postgres://`) |
| `--print-config` | print the effective configuration as TOML and exit |
| `--game.inactive-secs <GAME_INACTIVE_SECS>` | override `game.inactive_secs` |
| `--game.death-hp <GAME_DEATH_HP>` | override `game.death_hp` |
| `--seed <N>` | seed the game RNG (`game.rng_seed`); without it the seed comes from the OS |
| `--set <KEY=VALUE>` | override any key, for example `--set game.max_top=20` (the value is TOML) |

Only one server may run on a store: a second `serve` refuses.

## `silvermere setup`

The first start without serving: builds the stock world into an empty database and asks for a
sysop account if there is none. When it has just built the world and a person answers on a
terminal, it asks how to back up (daily at 04:30, keeping 30, by default) and writes the answer
to `silvermere.toml`. `install.sh` runs it. It prints how to connect and where the sysop
commands are when it finishes. Details: [install.md](install.md).

```
silvermere setup [--yes]
```

| Flag | Meaning |
|---|---|
| `--yes`, `-y` | ask nothing about backups: the defaults stand (on, daily at 04:30, keeping 30) |

Refuses while a server runs on the store (it writes the world).

## `silvermere service`

The systemd service (Linux): `install` or `uninstall`. Details:
[install.md](install.md#silvermere-service-install).

```
silvermere service install [--user | --system] [--database-url-from-stdin]
silvermere service uninstall [--user | --system]
```

### `silvermere service install`

Write and start the systemd unit. Run it from the folder that holds `data/` and
`silvermere.toml`: that folder becomes the unit's working directory. It runs `setup` first, and
prints how to connect and where the sysop commands are when it finishes.

| Flag | Meaning |
|---|---|
| `--user` | a user unit in `~/.config/systemd/user` (the default when not root) |
| `--system` | a system unit in `/etc/systemd/system`, run as the `silvermere` user (the default as root) |
| `--database-url-from-stdin` | read a database URL (a hosted PostgreSQL) from stdin and write it to the unit's env file; never on the command line |

### `silvermere service uninstall`

Stop the service and remove its unit; the data and the env file stay. Takes `--user` or
`--system` as above.

## `silvermere config`

Show and change the settings file (`silvermere.toml`). It is called `settings` in the README.
Details: [config.md](config.md).

```
silvermere config path
silvermere config show [KEY]
silvermere config set <KEY> <VALUE>
silvermere config unset <KEY>
silvermere config edit
```

| Subcommand | What it does |
|---|---|
| `config path` | print the settings file this install reads, and whether it exists |
| `config show [KEY]` | print the effective settings, or one key or section, and where each value comes from |
| `config set <KEY> <VALUE>` | write one setting into the file (the value is TOML; a bare word is a string) |
| `config unset <KEY>` | comment a setting out of the file, so it is back to its default |
| `config edit` | open the file in `$VISUAL` or `$EDITOR` and check it when the editor closes |

None take flags beyond the global ones.

## `silvermere account`

Manage accounts. Details: [auth.md](auth.md), [admin-api.md](admin-api.md). Role changes take
effect at the account's next login. Passwords are read as one line from stdin or the terminal,
never from the command line.

```
silvermere account create <NAME> [--sysop] [--gameop] [--password-stdin]
silvermere account passwd <NAME>
silvermere account passwd --missing [--dry-run] [--out <FILE>]
silvermere account grant <NAME> <ROLE>
silvermere account revoke <NAME> <ROLE>
silvermere account list
silvermere account delete <NAME> [--yes]
```

| Subcommand | What it does |
|---|---|
| `account create <NAME>` | create an account; `--sysop` and `--gameop` give it that role; `--password-stdin` reads the password from stdin |
| `account passwd <NAME>` | change a password (read from stdin) |
| `account passwd --missing` | give every account that has no password (as an import creates them) a random one, and print `name<TAB>password` lines; accounts with a password are never touched ([auth.md](auth.md), [import.md](import.md)) |
| `--dry-run` | with `--missing`: list the accounts that would get a password, change nothing |
| `--out <FILE>` | with `--missing`: write the lines to FILE (created with mode 600; refused if it exists) instead of stdout |
| `account grant <NAME> <ROLE>` | grant a role (`player`, `sysop`, `gameop`), a permission such as `SYS_GOTO_KEY` ([sysop.md](sysop.md#roles-and-permissions)) or a flag such as `door` (the BBS rlogin door may log the account in, [auth.md](auth.md#rlogin-a-bbs-door)) |
| `account revoke <NAME> <ROLE>` | revoke a role, permission or flag |
| `account list` | list accounts |
| `account delete <NAME>` | delete an account; `--yes` skips the question. With a server running it goes through the admin API, which needs an `admin` token in the config |

## `silvermere import`

Import legacy world files into the database. With no argument it downloads the stock 1.11p
distribution once and imports it. Details: [import.md](import.md).

```
silvermere import [OPTIONS] [ZIP|DIR]
```

| Flag | Meaning |
|---|---|
| `[ZIP\|DIR]` | a `.zip` or directory holding the `WCC*.VIR` files and `WCCUPDAT.DAT`, such as the MBBSEmu package; works offline |
| `--from <DIR\|ZIP>` | directory or `.zip` holding the same files |
| `--download` | download the stock distribution again, replacing the cached copy, and import it |
| `--live` | read a running realm's `WCC*.DAT` files and import its players; needs `--from` |
| `--map-files <MAP_FILES>` | the room files, in order (comma separated) |
| `--overwrite-assets` | replace the files already in `<data_dir>/assets/` |
| `--dry-run` | decode and count only; write nothing |
| `--json <OUT>` | also write the decoded records as `OUT/<TABLE>.jsonl` |
| `--content-only` | write content tables only; leave state untouched |
| `--reset-state` | recreate state (room floors, shop stock, monster state) from the content |
| `--with-users` | import `WCCUSERS` records as characters (for differential tests) |
| `--door-users` | as `--with-users`, and give each player's account the `door` flag so a BBS rlogin door logs them in without a password ([import.md](import.md), [auth.md](auth.md#rlogin-a-bbs-door)) |
| `--allow-duplicates` | keep the first of records with the same key instead of failing |
| `--no-patches` | skip the `WCCUPDAT.DAT` patches |
| `--patches <PATH>` | the patch file to apply (default: `WCCUPDAT.DAT` in `--from`) |
| `--legacy-tz <ZONE>` | the zone the legacy server ran in (IANA name) for the records' DOS dates (default `UTC`) |
| `--database-url <DATABASE_URL>` | database URL (`sqlite://` or `postgres://`); overrides `database.url` |

A run that writes refuses while a server runs on the store.

## `silvermere backup`

Write a consistent copy of the database; safe while the server runs. Details:
[install.md](install.md#backup-and-restore), [metrics.md](metrics.md).

```
silvermere backup [--to <TO>] [--keep <N>]
silvermere backup --list
```

| Flag | Meaning |
|---|---|
| `--to <TO>` | where to write (default `<data dir>/backups/silvermere-YYYYMMDD-HHMMSS.db`, UTC) |
| `--keep <N>` | delete the oldest backups beyond N (at least 1) in the destination folder; without `--to` it defaults to `backup.keep` (30) |
| `--list` | show the backups with their sizes, the schedule, and the last scheduled run with the after-backup command's result; makes no backup |

After a backup it runs the after-backup command (`backup.after`), if one is set, and says how
it ended. A failing command leaves the backup in place and the exit status 0. The server also
backs up on its own, daily at `backup.at` ([config.md](config.md#backup)).

## `silvermere restore`

Put a backup back. Stop the server first: it refuses while one runs. Details:
[install.md](install.md#backup-and-restore).

```
silvermere restore [--force] <FILE>
```

| Flag | Meaning |
|---|---|
| `<FILE>` | the backup to restore |
| `--force` | required for PostgreSQL, where the restore replaces objects in place |

## `silvermere migrate`

Apply schema migrations. Refuses while a server runs; `serve` migrates on its own at start.
Details: [schema.md](schema.md), [install.md](install.md#upgrading).

```
silvermere migrate
```

## `silvermere admin token`

Make and list admin API bearer tokens. Details: [admin-api.md](admin-api.md#tokens).

```
silvermere admin token new <NAME> [--scopes <SCOPES>]
silvermere admin token list
```

| Subcommand | What it does |
|---|---|
| `admin token new <NAME>` | make a token: prints it once, with the `[[admin.tokens]]` block to paste into the config; `--scopes` is comma-separated `read`, `write`, `admin` (default `read`) |
| `admin token list` | list the configured tokens (names, scopes, hash prefixes; never a token) |

## `silvermere sysop`

Sysop commands on a running server, over its admin API on `server.admin_bind`. Details:
[admin-api.md](admin-api.md). In the game, `SYSOP` lists the sysop commands
([sysop.md](sysop.md)).

```
silvermere sysop who
silvermere sysop broadcast <MESSAGE>
silvermere sysop reload-world
silvermere sysop shutdown [--in <DURATION>] [MESSAGE]
silvermere sysop shutdown --cancel
silvermere sysop restart [--in <DURATION>] [MESSAGE]
silvermere sysop restart --cancel
silvermere sysop kick <PLAYER> [REASON]
silvermere sysop tell <PLAYER> <MESSAGE>
silvermere sysop mute <PLAYER> [--for <DURATION>] [REASON]
silvermere sysop unmute <PLAYER>
silvermere sysop jail <PLAYER> [--for <DURATION>] [REASON]
silvermere sysop release <PLAYER>
silvermere sysop ban <PLAYER> [--for <DURATION>] [REASON]
silvermere sysop unban <PLAYER>
silvermere sysop ban-ip <NETWORK> [--for <DURATION>] [REASON]
silvermere sysop unban-ip <NETWORK>
silvermere sysop moderation
silvermere sysop backup
silvermere sysop realm close [REASON]
silvermere sysop realm open
silvermere sysop spectate <PLAYER> [--type]
silvermere sysop type <PLAYER> [LINE]...
silvermere sysop game <WORDS>...
silvermere sysop --json <SUBCOMMAND>
```

| Subcommand | What it does | Route | Scope |
|---|---|---|---|
| `sysop who` | lists the players in the game: name, class, race, level, gang and idle time | `GET /v1/who` | `read` |
| `sysop broadcast <MESSAGE>` | sends one line to every player in the game and says how many were told | `POST /v1/broadcast` | `write` |
| `sysop reload-world` | reads the static world tables again from the database, with nobody disconnected | `POST /v1/reload-world` | `write` |
| `sysop shutdown [MESSAGE]` | stops the server after warning the players in the game (when it is planned, then at 1 hour, 30, 15, 10, 5, 2 and 1 minutes, 30 and 10 seconds before), every character saved; `MESSAGE` goes with each warning. A second call replaces the plan | `POST /v1/shutdown` | `admin` |
| `sysop shutdown --cancel` | calls the planned shutdown off and tells the players | `DELETE /v1/shutdown` | `admin` |
| `sysop restart [MESSAGE]` | as `shutdown`, then a backup, and the server starts again in the same process: systemd and Docker see it keep running. Refused when the settings would not load or the program file does not run ([admin-api.md](admin-api.md#post-v1restart)) | `POST /v1/restart` | `admin` |
| `sysop restart --cancel` | calls the planned restart off and tells the players | `DELETE /v1/restart` | `admin` |
| `sysop kick <PLAYER> [REASON]` | disconnects the player, saved as a logoff saves (no hang-up penalty), telling him why | `POST /v1/players/{name}/kick` | `admin` |
| `sysop tell <PLAYER> <MESSAGE>` | `The sysop tells you: ...` to one player, in the game or at the menus | `POST /v1/players/{name}/tell` | `write` |
| `sysop mute <PLAYER>` | no gossip or auction until it ends or is lifted | `PUT /v1/players/{name}/mute` | `admin` |
| `sysop unmute <PLAYER>` | lifts the mute | `DELETE /v1/players/{name}/mute` | `admin` |
| `sysop jail <PLAYER>` | moves the player to the jail room and holds him there: no walking, recall or teleport out. When it ends he is moved to town | `PUT /v1/players/{name}/jail` | `admin` |
| `sysop release <PLAYER>` | lets the player out of jail, to town | `DELETE /v1/players/{name}/jail` | `admin` |
| `sysop ban <PLAYER>` | refuses his logins with the reason and the end; a player in the game is disconnected, saved. Not for BBS door accounts | `PUT /v1/players/{name}/ban` | `admin` |
| `sysop unban <PLAYER>` | lifts the ban | `DELETE /v1/players/{name}/ban` | `admin` |
| `sysop ban-ip <NETWORK>` | refuses telnet connections from an address or CIDR block (`203.0.113.7`, `203.0.113.0/24`, `2001:db8::/32`) before the login screen, and disconnects the players in the realm who came from it, saved; the rlogin door is never checked | `POST /v1/ip-bans` | `admin` |
| `sysop unban-ip <NETWORK>` | lifts the ban of the address or block | `DELETE /v1/ip-bans` | `admin` |
| `sysop moderation` | lists the bans, jails and mutes in force, the IP bans, and whether the realm is closed | `GET /v1/moderation` | `read` |
| `sysop backup` | takes a backup now on the running server, as a scheduled one (pruned to `backup.keep`, the after-backup command run); `silvermere backup` does the same from the outside | `POST /v1/backup` | `admin` |
| `sysop realm close [REASON]` | only sysops may log in; every other player is told, saved and disconnected. It stays closed across restarts | `POST /v1/realm/close` | `admin` |
| `sysop realm open` | everyone may log in again | `POST /v1/realm/open` | `admin` |
| `sysop spectate <PLAYER>` | shows the player's screen as they see it, live, until they leave (Ctrl-C to stop); they are not told. With `--type`, each line you type goes to the game as theirs, and nothing on their screen shows it | `GET /v1/players/{name}/screen` | `admin` |
| `sysop type <PLAYER> [LINE]...` | types one line for the player (none: Enter) | `POST /v1/players/{name}/input` | `admin` |
| `sysop game <WORDS>...` | runs one of the game's own `SYSOP` commands outside the game and prints its answer: `game list users`, `game god Lars info`, `game speak "back in 5"` (`SYSOP` may be left out). Those that need your character in the realm are refused: `GOTO`, `SUMMON`, `EXPRESS_TRAIN`, `MAP`, `RESET`, `REPORT USERS`, `REPORT NOTOPTEN`, `LIST GANG` ([sysop.md](sysop.md#sub-commands)) | `POST /v1/sysop` | `admin` |

| Flag | Meaning |
|---|---|
| `--json` | print the server's JSON answer as it came, for scripts |
| `--in <DURATION>` | with `shutdown` and `restart`: how long until the stop: `90s`, `5m`, `1h`, `1h30m` or a number of seconds, at most a day (default `5m`) |
| `--cancel` | with `shutdown` and `restart`: call off the planned shutdown or restart instead |
| `--for <DURATION>` | with `mute`, `jail`, `ban` and `ban-ip`: how long: `30m`, `12h`, `7d`, `1d12h` or a number of seconds; without it, until lifted |
| `--type` | with `spectate`: each line read from the terminal is typed for the player |

`<PLAYER>` is a character's first name or an account's username, ignoring case; the sanction
is stored, so it holds for a player who is not connected from his next login. The reason is
told to the player (an IP ban's is kept for the sysops only). Details:
[admin-api.md](admin-api.md#moderation).

After the stop, the systemd service stays stopped until `systemctl start silvermere`; Docker's
`restart: unless-stopped` starts it again at once. To upgrade under the service: plan the
shutdown, replace the program once it has stopped, start it
([admin-api.md](admin-api.md#post-v1shutdown)).

The command sends a token whose secret the config holds: `SILVERMERE_ADMIN_TOKEN` (the same
value the server was started with), or a `token` in `[[admin.tokens]]`. A token stored only as
its `sha256` cannot be sent. An `admin` token is used when there is one (`sysop who` then lists
hidden sysops too), else one with the scope in the table.

Exit 0 with the answer on stdout; exit 1 with one line on stderr when no server is running on
`server.admin_bind`, the config has no usable token, or the server refuses the call.

## `silvermere web`

The web pages without a server. Details: [web.md](web.md).

```
silvermere web export <DIR> [--base <BASE>]
```

| Subcommand | What it does |
|---|---|
| `web export <DIR>` | write the documentation pages as static files into `DIR` (made if missing), for a public copy; the settings page is the reference of every key |

| Flag | Meaning |
|---|---|
| `--base <BASE>` | the path the copy is served under, such as `/silvermere` for a project site (default: the root) |

## `silvermere healthcheck`

Probe the admin API's `/healthz`: exit 0 on an HTTP 200, else 1 with one line on stderr. For
container health checks. Details: [admin-api.md](admin-api.md).

```
silvermere healthcheck [--url <URL>]
```

| Flag | Meaning |
|---|---|
| `--url <URL>` | health endpoint to probe, plain http only (default `http://127.0.0.1:8080/healthz`) |
