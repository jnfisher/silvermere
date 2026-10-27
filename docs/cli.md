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
| [`sysop`](#silvermere-sysop) | sysop commands over the admin API | needs one running |
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
| `account grant <NAME> <ROLE>` | grant a role (`player`, `sysop`, `gameop`) or a permission such as `SYS_GOTO_KEY` ([sysop.md](sysop.md#roles-and-permissions)) |
| `account revoke <NAME> <ROLE>` | revoke a role or permission |
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

Sysop commands over the admin API of a running server: `sysop who`, `sysop broadcast <MESSAGE>`
and `sysop reload-world`. **Not implemented yet:** each prints `not implemented yet` and exits
with status 2. The same three actions are admin API routes (`GET /v1/who`, `POST /v1/broadcast`,
`POST /v1/reload-world`; see [admin-api.md](admin-api.md)), and in the game `SYSOP` lists the
sysop commands ([sysop.md](sysop.md)).

```
silvermere sysop who
silvermere sysop broadcast <MESSAGE>
silvermere sysop reload-world
```

## `silvermere healthcheck`

Probe the admin API's `/healthz`: exit 0 on an HTTP 200, else 1 with one line on stderr. For
container health checks. Details: [admin-api.md](admin-api.md).

```
silvermere healthcheck [--url <URL>]
```

| Flag | Meaning |
|---|---|
| `--url <URL>` | health endpoint to probe, plain http only (default `http://127.0.0.1:8080/healthz`) |
