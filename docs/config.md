# Configuration

silvermere runs with no configuration. Change only what you need.

## Precedence

Highest first:

1. command-line flags
2. `SILVERMERE_*` environment variables
3. `silvermere.toml`
4. built-in defaults

The file is `--config PATH`, else `./silvermere.toml`, else `<data-dir>/silvermere.toml`.
`silvermere.example.toml` is the commented template with every key at its default. Copy it and edit.
`silvermere serve --print-config` prints the effective configuration, so you can check what a flag,
variable or file change did.

Environment variables use a double underscore between section and key:
`SILVERMERE_GAME__DEATH_HP=-200` sets `game.death_hp`. Two flat names are also accepted:
`SILVERMERE_DATABASE_URL` (`database.url`) and `SILVERMERE_DATA_DIR`.

Flags on `serve`: `--telnet-bind`, `--admin-bind`, `--database-url`, `--data-dir`, `--log-format`,
`--log-level`, `--game.inactive-secs`, `--game.death-hp`, `--seed N` (`game.rng_seed`), and
`--set KEY=VALUE` for any other key (the value is TOML, e.g. `--set game.max_top=20`).

A value outside its range stops startup with exit code 1 and names the key and the range. Nothing is
clamped silently.

## The config command

`silvermere config` works on the file `serve` would read, with the same `--config` and
`--data-dir` lookup and the same checks as startup:

| Command | What it does |
|---|---|
| `silvermere config path` | Prints the file and whether it exists. With no file yet, a new one goes in `<data-dir>` when `--data-dir` or `SILVERMERE_DATA_DIR` is given (Docker: `/data`), else in the current folder. |
| `silvermere config show [KEY]` | Prints the effective settings, or one key or section, each marked `flag`, `env VARIABLE`, `file` or `default`. Tokens and the database password are redacted, as with `--print-config`. |
| `silvermere config set KEY VALUE` | Writes one key into the file, keeping its comments and order. VALUE is TOML, and a bare word is a string. A bad key or value is refused with startup's message and nothing is written. A new file is made mode 600 and owned by the data directory's owner (the service's user on the `--system` install); an existing one keeps its mode and owner. Every write goes through a temporary file renamed over the old one. |
| `silvermere config unset KEY` | Comments the key out (`# KEY = VALUE  # unset by silvermere config`), so it is back to its default; the comment lines above it stay. A later `set` of a commented-out key replaces that line. |
| `silvermere config edit` | Opens a copy of the file in `$VISUAL`, `$EDITOR`, or vi (notepad on Windows). The file is replaced only when the copy is valid; an invalid copy is shown and offered for another edit. |

`set` and `unset` print the old and new values. When a flag or environment variable overrides the
key, they say so: the server keeps that value until it is removed. `admin.tokens` and
`database.url` hold secrets and are refused by `set`: make a token with `silvermere admin token
new NAME` and paste its block with `config edit`; give the database URL with `service install
--database-url-from-stdin` or in `SILVERMERE_DATABASE_URL`. A file that needs root to write says
"run with sudo".

Settings are read once, at startup. Restart the server after a change (`sudo systemctl restart
silvermere` for the service).

## Product decisions

| Setting | Value |
|---|---|
| `game.death_hp` | `-200` |
| `game.inactive_secs` | `3600` (one hour idle, then the player is logged off) |
| `game.characters_per_account` | `1` |
| `game.open_signup` | `true` (anyone can create an account with `NEW`) |

## Example

```toml
# silvermere.toml: only the keys you change.
[server]
telnet_bind = "0.0.0.0:2323"
admin_bind  = "127.0.0.1:8080"    # keep on loopback; a non-loopback bind needs a token

[database]
url = "postgres://silvermere:secret@localhost/silvermere"   # empty = SQLite in the data directory

[compat]
bbs_menu = true                   # the "Make your selection (X to exit):" menu
charset  = "cp437"

[game]
death_hp = -200
inactive_secs = 3600
characters_per_account = 1
open_signup = true
addons = ["all"]

[logging]
format = "json"
level  = "info"
```

## Keys

The first column is checked against `silvermere serve --print-config` by
`bin/silvermere/tests/docs_in_sync.rs`, so this list and the defaults stay true.
"Original name" is the `WCCMMUD.MSG` or `WCCMMUD.INI` option the key replaces. Ranges, sources and the
options that were dropped are in [config-defaults.md](config-defaults.md).

### Top level

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `data_dir` | `SILVERMERE_DATA_DIR` | `"./data"` | Data directory: database, `backups/`, `silvermere.toml`. Also `--data-dir`. | — |

### [server]

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `server.telnet_bind` | `SILVERMERE_SERVER__TELNET_BIND` | `"0.0.0.0:2323"` | Telnet listen address, host:port. | — |
| `server.admin_bind` | `SILVERMERE_SERVER__ADMIN_BIND` | `"127.0.0.1:8080"` | Admin API listen address. Keep it on loopback unless you set a token. | — |
| `server.max_sessions` | `SILVERMERE_SERVER__MAX_SESSIONS` | `256` | Most simultaneous connections, 1..=100000. | — |

### [admin]

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `admin.tokens` | `SILVERMERE_ADMIN_TOKENS` | `[]` | Admin API bearer tokens: `[[admin.tokens]]` with `name`, `sha256` (what `silvermere admin token new` prints) or a plain `token`, and `scopes` (`read`, `write`, `admin`; `broadcast` and `sysop` are accepted aliases). The older `SILVERMERE_ADMIN_TOKEN` also works. See [admin-api.md](admin-api.md). `--print-config` shows tokens as `<redacted>`. | — |

### [database]

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `database.url` | `SILVERMERE_DATABASE_URL` | `""` | `sqlite://...` or `postgres://...`. Empty means `sqlite://<data_dir>/silvermere.db`. | — |

### [net]

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `net.login_timeout_secs` | `SILVERMERE_NET__LOGIN_TIMEOUT_SECS` | `300` | Seconds a connection may sit in the login dialogue without typing before it is closed. 0 = off. | — |

### [compat]

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `compat.bbs_menu` | `SILVERMERE_COMPAT__BBS_MENU` | `true` | Show the `Make your selection (X to exit):` menu before the game, as MegaMud scripts expect. | — |
| `compat.charset` | `SILVERMERE_COMPAT__CHARSET` | `"cp437"` | `"cp437"` or `"utf8"`. A player can switch with `set utf8 on`. | — |
| `compat.version_string` | `SILVERMERE_COMPAT__VERSION_STRING` | `"1.11p-WG (Dec 30 2005 14:20:05)"` | Shown in menus. | — |
| `compat.bulletin` | `SILVERMERE_COMPAT__BULLETIN` | `""` | Bulletin text shown after login. | — |

### [game]

Defaults are the values the original distribution shipped. `inactive_secs = 3600` replaces the shipped 900. `characters_per_account` and `open_signup` are new options. Every ranged key is rejected, not clamped, when out of range. See [config-defaults.md](config-defaults.md) for ranges, sources and the options that were dropped.

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `game.inactive_secs` | `SILVERMERE_GAME__INACTIVE_SECS` | `3600` | Seconds idle before a kick. 60..=32000. | `INACTIVE` |
| `game.death_hp` | `SILVERMERE_GAME__DEATH_HP` | `-200` | Hit points at which a player dies. -20000..=0. | `DEATHHP` |
| `game.rng_seed` | `SILVERMERE_GAME__RNG_SEED` | `absent` | Seed for the game RNG (`serve --seed N`). Absent means seeded from the OS. | — |
| `game.blank_lastname_allowed` | `SILVERMERE_GAME__BLANK_LASTNAME_ALLOWED` | `false` | Allow characters with no last name. | `BLANK_LASTNAME_ALLOWED` |
| `game.characters_per_account` | `SILVERMERE_GAME__CHARACTERS_PER_ACCOUNT` | `1` | Characters per account (1). 1..=255. | — |
| `game.open_signup` | `SILVERMERE_GAME__OPEN_SIGNUP` | `true` | Anyone may create an account with NEW (true). | — |
| `game.addons` | `SILVERMERE_GAME__ADDONS` | `["all"]` | Add-on packs to load; `["all"]`. | — |
| `game.sec_create` | `SILVERMERE_GAME__SEC_CREATE` | `5` | Seconds between monster creation. | `SECCREATE` |
| `game.sec_oth` | `SILVERMERE_GAME__SEC_OTH` | `160` | Seconds between player saves. 1..=20000. | `SECOTH` |
| `game.sec_slow` | `SILVERMERE_GAME__SEC_SLOW` | `30` | Seconds in the slow cycle. | `SECSLOW` |
| `game.sec_med` | `SILVERMERE_GAME__SEC_MED` | `3` | Seconds in the medium cycle. | `SECMED` |
| `game.sec_fast` | `SILVERMERE_GAME__SEC_FAST` | `1` | Seconds in the fast cycle. | `SECFAST` |
| `game.sec_energy` | `SILVERMERE_GAME__SEC_ENERGY` | `5` | Seconds in the energy cycle. | `SECENERGY` |
| `game.start_room` | `SILVERMERE_GAME__START_ROOM` | `2140` | (map 1): where new characters start. | `STARTROOM` |
| `game.reinc_room` | `SILVERMERE_GAME__REINC_ROOM` | `2189` | (map 1): where a dead player is reborn. | `REINCROOM` |
| `game.evil_reinc_room` | `SILVERMERE_GAME__EVIL_REINC_ROOM` | `142` | (map 1): rebirth room for evil players. | `EVILREINCROOM` |
| `game.jail_room` | `SILVERMERE_GAME__JAIL_ROOM` | `376` | Room (map 1) where jailed players are held. | `JAILROOM` |
| `game.xtra_lvl` | `SILVERMERE_GAME__XTRA_LVL` | `2` | Extra levels before training. 0..=16. | `XTRALVL` |
| `game.regen_mins` | `SILVERMERE_GAME__REGEN_MINS` | `5` | Minutes before monsters regenerate. | `REGENMINS` |
| `game.gang_exp` | `SILVERMERE_GAME__GANG_EXP` | `0` | Experience (x1000) for a gang house. 0..=32767. | `GANGEXP` |
| `game.talk_char` | `SILVERMERE_GAME__TALK_CHAR` | `"."` | Line prefix that makes a line speech. | `TALKCHAR` |
| `game.yell_char` | `SILVERMERE_GAME__YELL_CHAR` | `'"'` | Line prefix that makes a line a yell. | `YELLCHAR` |
| `game.smash_rounds` | `SILVERMERE_GAME__SMASH_ROUNDS` | `15` | Rounds a smash attack lasts. | `SMASHROUNDS` |
| `game.smash_penalty` | `SILVERMERE_GAME__SMASH_PENALTY` | `35` | Penalty a smash attack applies. | `SMASHPENALTY` |
| `game.def_chat_rate` | `SILVERMERE_GAME__DEF_CHAT_RATE` | `1` | Pages, gossips and auctions allowed per slow cycle. | `DEFCHATRATE` |
| `game.max_top` | `SILVERMERE_GAME__MAX_TOP` | `10` | Entries in the top lists. 10..=3000. | `MAXTOP` |
| `game.inv_glo` | `SILVERMERE_GAME__INV_GLO` | `[";x", ";-", ";re"]` | Disallowed globals (at most 10). | `INVGLO1`..`INVGLO10` |
| `game.pun_hup` | `SILVERMERE_GAME__PUN_HUP` | `"none"` | Punishment for hanging up: `"high"`, `"medium"`, `"low"` or `"none"`. | `PUNHUP` |
| `game.evil_keep` | `SILVERMERE_GAME__EVIL_KEEP` | `0` | Percentage of evil points kept. 0..=100. | `EVILKEEP` |
| `game.lvl_diff` | `SILVERMERE_GAME__LVL_DIFF` | `9` | Largest level gap for PvP. -1..=100. | `LVLDIFF` |
| `game.min_hup_hp` | `SILVERMERE_GAME__MIN_HUP_HP` | `0` | Least percentage lost on hangup. 0..=100. | `MINHUPHP` |
| `game.max_hup_hp` | `SILVERMERE_GAME__MAX_HUP_HP` | `0` | Most percentage lost on hangup. 0..=100. | `MAXHUPHP` |
| `game.max_hup_items` | `SILVERMERE_GAME__MAX_HUP_ITEMS` | `0` | Most items lost on hangup. 0..=100. | `MAXHUPIT` |
| `game.keep_exp` | `SILVERMERE_GAME__KEEP_EXP` | `false` | Keep some experience after the final death. | `KEEPEXP` |
| `game.light_evy` | `SILVERMERE_GAME__LIGHT_EVY` | `true` | SYSOP LIGHTNING may hit any player. | `LIGHTEVY` |
| `game.gender_gold` | `SILVERMERE_GAME__GENDER_GOLD` | `0` | Gold to change gender. -1..=32000. | `GENDERGO` |
| `game.enable_lawful` | `SILVERMERE_GAME__ENABLE_LAWFUL` | `true` | Players may choose the Lawful alignment. | `ENABLAWF` |
| `game.ep_cycle` | `SILVERMERE_GAME__EP_CYCLE` | `60` | Minutes per evil-point forgiveness cycle. 15..=360. | `EPCYCLE` |
| `game.ep_amount` | `SILVERMERE_GAME__EP_AMOUNT` | `1` | Evil points forgiven per cycle. 1..=100. | `EPAMOUNT` |
| `game.max_ep_day` | `SILVERMERE_GAME__MAX_EP_DAY` | `12` | Most evil points forgiven a day. 4..=100. | `MAXEPDAY` |
| `game.usr_cur_name` | `SILVERMERE_GAME__USR_CUR_NAME` | `"runic coins"` | Name of the top currency, at most 28 characters. | `USRCURNM` |
| `game.plat_per_runic` | `SILVERMERE_GAME__PLAT_PER_RUNIC` | `100` | Platinum pieces per top coin. | — |
| `game.gold_per_plat` | `SILVERMERE_GAME__GOLD_PER_PLAT` | `100` | Gold pieces per platinum piece. | — |
| `game.silv_per_gold` | `SILVERMERE_GAME__SILV_PER_GOLD` | `10` | Silver pieces per gold piece. | — |
| `game.copper_per_silv` | `SILVERMERE_GAME__COPPER_PER_SILV` | `10` | Copper pieces per silver piece. | — |
| `game.track_limited` | `SILVERMERE_GAME__TRACK_LIMITED` | `false` | Log every move of a limited item. | `TRACK_LIMITED` |
| `game.exp_in_top_gang` | `SILVERMERE_GAME__EXP_IN_TOP_GANG` | `false` | Show each gang's experience in TOP GANGS. | `EXP_IN_TOPGANG` |
| `game.log_sysop_commands` | `SILVERMERE_GAME__LOG_SYSOP_COMMANDS` | `true` | Log sysop commands (on by default). | `LOG_SYSOP_COMMANDS` |
| `game.race_change_duration` | `SILVERMERE_GAME__RACE_CHANGE_DURATION` | `300` | Minutes a race change lasts. 1..=720. | `RACE_CHANGE_DURATION` |
| `game.enable_class_change` | `SILVERMERE_GAME__ENABLE_CLASS_CHANGE` | `false` | Let characters from before 1.1u change class as well as race. | `ENABLE_CLASS_CHANGE` |
| `game.tournament_play` | `SILVERMERE_GAME__TOURNAMENT_PLAY` | `false` | Tournament mode: forces `enable_lawful` off and `lvl_diff` to 100. | `TOURNAMENT_PLAY` |
| `game.maximum_bank_balance` | `SILVERMERE_GAME__MAXIMUM_BANK_BALANCE` | `0` | In copper; 0 means no limit. | `MAXIMUM_BANK_BALANCE` |
| `game.kid_gloves` | `SILVERMERE_GAME__KID_GLOVES` | `false` | Refund wrong payments. | `KID_GLOVES` |
| `game.sys_log_deaths` | `SILVERMERE_GAME__SYS_LOG_DEATHS` | `false` | Log player deaths, rerolls and suicides. | `SYS_LOG_DEATHS` |
| `game.profanity_check` | `SILVERMERE_GAME__PROFANITY_CHECK` | `false` | Filter profanity in say, yell and gossip-class lines. | `PROFCHEK` |
| `game.exp_in_topten` | `SILVERMERE_GAME__EXP_IN_TOPTEN` | `true` | Show experience in the TOPTEN list. | `EXPINTEN` |
| `game.codec_of_twinks` | `SILVERMERE_GAME__CODEC_OF_TWINKS` | `0` | Developer code; 78 unlocks the sysop developer tools. | `CODEC_OF_TWINKS` |
| `game.cleanup.at` | `SILVERMERE_GAME__CLEANUP__AT` | `"03:00"` | Local time (`"HH:MM"`, 24-hour) of the daily cleanup (`mcurou`: gang-house tax, floors swept, evil forgiven, empty gangs deleted). It runs with the players connected; nobody is logged off. | — |
| `game.cleanup.tz` | `SILVERMERE_GAME__CLEANUP__TZ` | `"America/Detroit"` | IANA time zone `game.cleanup.at` is read in (silvermere addition; an unknown name is a config error). | — |
| `game.buffers.monsbuf` | `SILVERMERE_GAME__BUFFERS__MONSBUF` | `300` | The active-monster cache holds `monsbuf + 1` slots; the background passes walk it, so it decides which monsters move, regenerate and refill their energy. 10..=32767. The original shrank it to fit its memory (to 20 under MBBSEmu); the differential sets that value. | `MONSBUF` |

### [logging]

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `logging.format` | `SILVERMERE_LOGGING__FORMAT` | `"pretty"` | `"pretty"` or `"json"`. | — |
| `logging.level` | `SILVERMERE_LOGGING__LEVEL` | `"info"` | A `tracing` filter, e.g. `info` or `silvermere=debug`. Under `serve` it sets the log file in `data/logs/`; stderr (the journal under systemd) always gets warnings and errors and the writer guard's and persistence's info lines, whatever this says. | — |

### [backup]

The daily backup the server takes, and the command run after each backup
([install.md](install.md#backup-and-restore) has the recipes).

| Key | Environment variable | Default | Meaning | Original name |
|---|---|---|---|---|
| `backup.enabled` | `SILVERMERE_BACKUP__ENABLED` | `true` | Back up daily at `backup.at` while the server runs, into `data/backups/`. A stopped server takes none. | — |
| `backup.at` | `SILVERMERE_BACKUP__AT` | `"04:30"` | Local time (`"HH:MM"`, 24-hour) of the daily backup, in `game.cleanup.tz` (after the 03:00 cleanup by default). | — |
| `backup.keep` | `SILVERMERE_BACKUP__KEEP` | `30` | Backups kept in `data/backups/`: after each one there (scheduled, by hand, or before an upgrade) the oldest beyond this are deleted. 1..=10000. | — |
| `backup.after` | `SILVERMERE_BACKUP__AFTER` | `""` | A command run after each successful backup, without a shell: `{file}` is the backup's path, `{dir}` the backups folder (also in `SILVERMERE_BACKUP_FILE` and `SILVERMERE_BACKUP_DIR`). Quote with `'...'` or `"..."`; for a shell, `sh -c '...'`. It is not given the database URL, its password or the admin tokens (it can still read what the server's user can). Its failure is logged and shown; the backup stays. Empty: none. | — |
| `backup.after_timeout_secs` | `SILVERMERE_BACKUP__AFTER_TIMEOUT_SECS` | `1800` | Seconds `backup.after` may run before it is killed, with what it started. 1..=86400. | — |

## Original options with no key

| Original | What happened |
|---|---|
| `COINSWP`, `GCOINSW` | Defined in the original but never read. No key. |
| `SYS_RESET_ROOMS`, `SYS_DISABLE_GANGS` and the other `SYS_*_KEY` options | Became account roles instead of game settings. |
| `SECBUFF`, `DELUSER`, `ACTIVATE`, `DEMOLVL` | Removed. See [differences.md](differences.md). |

The full list, with ranges and the reason for each, is [config-defaults.md](config-defaults.md).
