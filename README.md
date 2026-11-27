<p align="center"><img src="docs/images/silvermere-logo.png" alt="silvermere" width="100%"></p>

<p align="center"><img src="docs/images/silvermere.webp" alt="Silvermere from the air: the walled town on the Silver river, the island with the Labyrinth beyond it, and Darkwood Forest to the east" width="100%"></p>

<h3 align="center">📖 <a href="https://jnfisher.github.io/silvermere/">Documentation, FAQ and settings reference: jnfisher.github.io/silvermere</a></h3>

**silvermere is MajorMUD 1.11p, rebuilt in Rust as a single program you download and run.**
It plays like the original, and MegaMud and any telnet client connect to it unchanged. It needs
none of the old stack: no MajorBBS or Worldgroup, no DOS emulator, no Btrieve.

Start it and it sets itself up. The first start fetches the stock 1.11p world, builds its
database, asks you to pick a sysop name and password, and opens the realm on port 2323. A
couple of minutes later you are playing.

- **Plays like 1.11p.** Checked line by line against the original game running side by side,
  down to the text and the prompts. MegaMud's scripts run as they always did.
- **Runs anywhere.** Windows, Linux (including a 64-bit Raspberry Pi), macOS and Docker, from a
  laptop to a small VPS. Its data lives in one SQLite file, or in PostgreSQL if you prefer.
- **Easy to host.** One command installs it as a service. It backs itself up every day, and
  upgrades keep your players and your world.
- **A sysop console in the browser.** See who is on and where, watch a player's screen, kick,
  jail, mute or ban, close the realm, restart or upgrade, all from one page.
- **Your world, your way.** Bring your own BBS's MajorMUD files or a Nightmare Redux database
  of an edited realm such as MudRev, keep editing the world with the classic editors, or run it
  as a door behind your BBS.

## Quick start

| On | Do this |
|---|---|
| **Windows** | Download the `windows` zip from the [latest release](https://github.com/jnfisher/silvermere/releases/latest), unzip it to a folder such as `C:\Silvermere`, and double-click `silvermere.exe`. |
| **Linux** | `curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh \| sh` |
| **macOS** | Download the `apple-darwin` archive from the [latest release](https://github.com/jnfisher/silvermere/releases/latest), unpack it, and run `./silvermere` in Terminal. |
| **Docker** | Three commands with the compose file in `docker/`: [docs/install.md](docs/install.md#docker-compose). |

Then connect MegaMud or a telnet client to `localhost`, port `2323`, and log in with your sysop
name. The sections below cover each platform in a little more detail, then hosting for other
players.

## Play it locally

### Windows

1. Download `silvermere-VERSION-x86_64-pc-windows-msvc.zip` from the
   [release page](https://github.com/jnfisher/silvermere/releases/latest).
2. Unzip it to a folder you can write to, such as `C:\Silvermere`. Not `C:\Program Files`.
3. Double-click `silvermere.exe`.

The first start builds the world (a minute or two), asks for a sysop name and password, and
serves. If Windows Firewall asks, click **Cancel** to play on this PC only, or allow
**Private networks** for friends on your network. SmartScreen may say the publisher is
unknown: click **More info**, then **Run anyway**.

In MegaMud, add a BBS entry with address `localhost`, port `2323`, terminal type ANSI.
[docs/windows.md](docs/windows.md) covers the rest: telnet clients, where files are kept,
backups, hosting and running as a service.

### Linux

The installer downloads the latest release for your machine (x86_64 or aarch64), checks it,
puts it in `~/silvermere` and runs it as your own user's service. It says what it will do and
asks first ([packaging/install.sh](packaging/install.sh)):

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
```

Or unpack a release archive yourself and run it in the foreground:

```sh
sha256sum -c SHA256SUMS --ignore-missing
tar xzf silvermere-VERSION-x86_64-unknown-linux-musl.tar.gz
cd silvermere-VERSION-x86_64-unknown-linux-musl
./silvermere
```

The first start builds the world into `./data/silvermere.db`, asks for a sysop name and
password, and serves; later starts go straight to serving. Then `telnet localhost 2323`.

### macOS

Download `silvermere-VERSION-universal-apple-darwin.tar.gz` from the
[release page](https://github.com/jnfisher/silvermere/releases/latest) (it runs on Apple
silicon and Intel), then in Terminal:

```sh
tar xzf silvermere-VERSION-universal-apple-darwin.tar.gz
cd silvermere-VERSION-universal-apple-darwin
xattr -d com.apple.quarantine silvermere 2>/dev/null   # a browser download is quarantined; this lets it run
./silvermere
```

The first start is the same as on Linux. Then `telnet localhost 2323`, or point MegaMud on
another machine at the Mac's address. To build it yourself instead, see
[Building from source](#building-from-source).

### Logging in

Log in with the sysop name. At `Make your selection (X to exit):` press `M`, then `E` to enter
the realm. Players type `NEW` at `Username:` to make their own accounts.

**First steps as the sysop.** The [sysop console](#running-the-realm) is at
`http://127.0.0.1:8088/console`: sign in with the same name and password. In the game,
`SYSOP` alone lists the original's sysop commands, and typing `sysop` at the `[MAJORMUD]:` menu
(the word is hidden) opens the maintenance menu. [docs/sysop.md](docs/sysop.md) has the details
and [docs/cli.md](docs/cli.md) lists every command-line tool.

## Hosting for other players

A Linux VPS with systemd runs silvermere as a service under its own user, in `/opt/silvermere`.
Players connect on port 2323; the admin API stays on loopback. The data is a SQLite file in
`/opt/silvermere/data`, or a hosted PostgreSQL database such as Neon (use its direct
connection, not the pooled one: the server holds an advisory lock for its lifetime).

<details>
<summary>The easy way: install.sh and <code>silvermere service install</code></summary>

As root, or with sudo:

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sudo sh -s -- --system
```

It downloads the latest release, checks its SHA-256, installs it in `/opt/silvermere`, builds
the world, asks for the sysop's name and password, and runs `silvermere service install
--system`. That creates the `silvermere` user, writes the systemd unit and starts it. Read
[packaging/install.sh](packaging/install.sh) first if you like; it asks before it does anything.
Without root, `curl -fsSL …/install.sh | sh` installs into `~/silvermere` and runs it as a user
service ([docs/install.md](docs/install.md#installsh-linux)).

For a hosted PostgreSQL, add `--hosted`. It asks for the database URL (hidden as you paste it)
and keeps it in `/etc/silvermere/env`, mode 600:

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sudo sh -s -- --system --hosted
```

To change the URL later:

```sh
cd /opt/silvermere && sudo ./silvermere service install --system --database-url-from-stdin
```

Then open the telnet port and check the log:

```sh
sudo ufw allow OpenSSH && sudo ufw allow 2323/tcp && sudo ufw --force enable
journalctl -u silvermere -n 50 --no-pager
```

</details>

<details>
<summary>By hand: the user, the unit file, the firewall</summary>

Start with the `silvermere` binary from the release archive (above) or from
`target/release/` after building. Make its user and folder:

```sh
sudo useradd --system --home-dir /opt/silvermere --shell /usr/sbin/nologin silvermere
sudo install -d -o silvermere -g silvermere /opt/silvermere
sudo install -m 0755 silvermere /opt/silvermere/silvermere
```

Build the world and make the sysop. On SQLite:

```sh
cd /opt/silvermere
sudo -u silvermere ./silvermere import
sudo -u silvermere ./silvermere account create NAME --sysop   # prints Password: and echoes what you type
```

On a hosted PostgreSQL, put the URL in an env file only root can read, in single quotes (the
shell would cut it at the first `&`), and load it for the same two commands:

```sh
sudo install -d -m 700 /etc/silvermere
sudo install -m 600 /dev/null /etc/silvermere/env
sudoedit /etc/silvermere/env      # one line: SILVERMERE_DATABASE_URL='postgres://USER:PASSWORD@HOST/DBNAME?sslmode=require'
cd /opt/silvermere
sudo sh -c 'set -a; . /etc/silvermere/env; exec runuser -u silvermere -- ./silvermere import'
sudo sh -c 'set -a; . /etc/silvermere/env; exec runuser -u silvermere -- ./silvermere account create NAME --sysop'
```

Write the unit. On PostgreSQL, uncomment the `EnvironmentFile` line:

```sh
sudo tee /etc/systemd/system/silvermere.service >/dev/null <<'EOF'
[Unit]
Description=silvermere
Wants=network-online.target
After=network-online.target
StartLimitIntervalSec=0

[Service]
User=silvermere
Group=silvermere
WorkingDirectory=/opt/silvermere
ExecStart=/opt/silvermere/silvermere serve
#EnvironmentFile=/etc/silvermere/env
Restart=on-failure
RestartSec=5
TimeoutStopSec=60
UMask=0027

[Install]
WantedBy=multi-user.target
EOF
```

`TimeoutStopSec=60` gives the server time to save every character when it stops.
`Restart=on-failure` with no start limit keeps it trying while a hosted database is away.

Start it, open the telnet port, and check it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now silvermere      # starts now and after every reboot
sudo ufw allow OpenSSH && sudo ufw allow 2323/tcp && sudo ufw --force enable
systemctl status silvermere --no-pager
journalctl -u silvermere -n 50 --no-pager
```

If the provider has its own firewall, allow `22/tcp` and `2323/tcp` there too. Keep the admin
port `8080` closed.

</details>

The server backs itself up every day at 04:30 and keeps the last 30 (`silvermere backup --list`
shows them; an after-backup command can copy each one off the box, with rclone, restic or rsync
recipes in [docs/install.md](docs/install.md#backup-and-restore)). The full runbook, with upgrades, TLS
for PostgreSQL and troubleshooting: [docs/install.md](docs/install.md) and
[docs/deploy-checklist.md](docs/deploy-checklist.md).

### Behind a BBS, as a door

Set `rlogin_bind = "127.0.0.1:2327"` under `[server]` and point the BBS's rlogin door at it with
the caller's name as the user. Callers skip silvermere's login. Only accounts with the `door`
flag can be reached this way: the ones the door creates on a first visit, an import's players
with `--door-users`, and any you grant it to (`silvermere account grant NAME door`).
[docs/auth.md](docs/auth.md#rlogin-a-bbs-door) covers door, telnet or both, sysops on the door,
and moving a realm from a BBS.

### Upgrading

On Linux, installed with install.sh: run the same one-liner again, as you did the first time
(`curl -fsSL …/install.sh | sudo sh -s -- --system`, or `| sh` for your own user). Over an
install it upgrades: it backs up the database, keeps the old binary as `silvermere.previous`,
installs the new one, keeps `data/` and `silvermere.toml`, and restarts the service. At the
version already installed it does nothing (`--force` reinstalls).

Elsewhere (by hand, Docker compose, macOS, Windows) the steps are the same idea: back up, stop,
replace the program, start. The new version migrates the database on its first start.
[docs/install.md](docs/install.md#upgrading) has them per platform, and the way back.

## Settings

silvermere runs with no settings file. When you want one, it is `silvermere.toml` in the folder
the server runs from:

| Install | File |
|---|---|
| `install.sh --system`, `service install --system` | `/opt/silvermere/silvermere.toml` |
| `install.sh` (your user) | `~/silvermere/silvermere.toml` |
| Windows | beside `silvermere.exe`, such as `C:\Silvermere\silvermere.toml` |
| Docker | `/data/silvermere.toml`, in the volume |

The easiest way to see and change settings is in a browser: the server has web pages of its
own on `http://127.0.0.1:8088` (this documentation, a FAQ, a settings editor and the
[sysop console](#running-the-realm), which a sysop signs in to with their game account;
[docs/web.md](docs/web.md)).

`silvermere config` reads and writes the same file with the same checks as startup, run from that folder:

```sh
silvermere config path                      # which file, and whether it exists
silvermere config show                      # every setting, and where each value comes from
silvermere config show game                 # one key or section
silvermere config set game.max_top 20       # write one key; a bad value is refused
silvermere config unset game.max_top        # back to the default
silvermere config edit                      # open it in $VISUAL or $EDITOR; checked on save
```

For the `--system` install, run them as `cd /opt/silvermere && sudo ./silvermere config ...`.
In Docker: `docker compose exec silvermere /silvermere config show`; the image has no
editor, so use `config set` there, or edit `/data/silvermere.toml` in the volume. Admin tokens and the
database URL are secrets, so `config set` will not write them and says how instead.

The server reads its settings when it starts, so restart it after a change:
`sudo systemctl restart silvermere` (`systemctl --user restart silvermere` for your user's
service). Command-line flags and `SILVERMERE_*` environment variables
override the file; `config show` names the one in effect. Every key, its default and its range:
[docs/config.md](docs/config.md).

## Running the realm

<p align="center"><img src=".github/media/console-dark.gif" alt="The sysop console in dark mode with 64 players online: the status bar, the players and the rooms they stand in, and a player's panel with Kick, Jail, Mute and Ban" width="880"></p>

The **sysop console** is a page of the server's own web site, `http://127.0.0.1:8088/console`:
sign in with a sysop's game account. It shows, live:

- who is in the realm, the room each stands in (`map:room` and its name), level, class, race,
  gang, idle time and the address they connect from;
- a panel for each player: a line to them, **watch** their screen live and type as them
  (they are not told), **kick**, **jail** (held in the jail room until a set time, then let
  out to town), **mute** (no gossip or auction), **ban** the account or the address, each for
  a time you choose and with a reason they are told;
- the bans, jails and mutes in force, each with a button to lift it;
- logins, deaths, levels, sign-ins and every staff action as they happen;
- the server's health, the last backup, and buttons to **close the realm** to all but sysops,
  **back up now**, **restart** or **shut down**;
- a command line taking the same commands as `silvermere sysop` below, the game's own `SYSOP`
  commands among them. Every button runs one of them there, so you always see what was done.

A **restart** warns the players on a countdown, saves everyone, takes a backup and starts the
server again in the same process, so systemd and Docker see it keep running and the console
reconnects by itself. Replace the program file first and the restart is an upgrade (Linux and
macOS). Light and dark follow your system; it works on a phone too.

The same from a shell on the server:

```sh
silvermere sysop who                                  # who is on, where, from which address
silvermere sysop tell Lars "please keep gossip civil"
silvermere sysop jail Lars --for 1h "fighting in town" # release Lars to let him out early
silvermere sysop mute Lars --for 1d "spamming gossip"
silvermere sysop ban Lars --for 7d "cheating"          # refuses their logins; unban to lift
silvermere sysop ban-ip 203.0.113.0/24 "bots"          # disconnects them too; the BBS door is never checked
silvermere sysop spectate Lars --type                  # their screen, live; your lines go in as theirs
silvermere sysop game god Lars info                    # the game's own SYSOP commands
silvermere sysop realm close "upgrading"               # only sysops log in; realm open to end it
silvermere sysop restart --in 5m "upgrading to 1.0"    # --cancel calls it off
silvermere sysop moderation                            # what is in force
```

They call the [admin API](#admin-api) on this computer and need an `admin` token in
`silvermere.toml` (`SILVERMERE_ADMIN_TOKEN` also works). Bans, jails, mutes and a closed realm
are stored and hold across restarts.

The console is plain HTTP on the server's own loopback address. To use it from elsewhere, use
an SSH tunnel (`ssh -L 8088:127.0.0.1:8088 you@server`), a private network such as Tailscale,
or a reverse proxy with TLS; [docs/web.md](docs/web.md) covers each, and how sessions, password
re-entry for what cannot be undone and the host names it answers to keep it safe. Every
command: [docs/cli.md](docs/cli.md#silvermere-sysop). In the game, `SYSOP` still has the
original's own commands ([docs/sysop.md](docs/sysop.md)).

## Editing the world

silvermere has no world editor of its own. Edit the world with an editor that writes the
MajorMUD 1.11p Btrieve files, then import the result. The import replaces the content (rooms,
items, monsters, spells, shops, text) and leaves play alone: characters, room state, shop stock,
gangs and bank books are not touched.

```sh
silvermere backup                                        # a copy of the database first
sudo systemctl stop silvermere                           # an import needs the server stopped
silvermere import --from DIR --live --content-only --dry-run   # read and check; writes nothing
silvermere import --from DIR --live --content-only
sudo systemctl start silvermere
```

`DIR` holds the edited files. `--live` reads the `.DAT` set, which is what the game and its
editors work on; leave it out to read `.VIR` files instead. `--content-only` is what keeps the
players and the state as they are, and running it twice gives the same result. The update file
`WCCUPDAT.DAT` in `DIR` is applied over the content, as the original applied it at every start;
`--no-patches` skips it. [docs/import.md](docs/import.md#content-and-state) has the details.

## How we know it's faithful

We ran the original `WCCMMUD.DLL` under [MBBSEmu](https://github.com/mbbsemu/MBBSEmu) and used
it as the reference: to work out how the game behaves, and to check that silvermere behaves the
same way. Where the two disagree, the original is right.

- **Side by side.** A differential harness runs the original and silvermere next to each
  other, plays the same scripted sessions against both and compares the output line by line:
  49 scenarios and 31 scripts under [tools/differential](tools/differential/README.md).
  Deterministic steps must match byte for byte. Steps that roll dice are compared
  statistically, and the known gaps are listed in `known_gaps.json`.
- **Dice and combat.** Hit rates, damage and other random outcomes pass statistical tests
  against the original across 11 sessions (22 gated measurements, Holm-corrected at 0.01).
- **Tests.** Nearly 2,000 tests across the workspace, with snapshot files and recorded output
  of the original. `cargo test --workspace` runs them.
- **Real clients.** We played with MegaMud on Windows against a silvermere server over the
  internet, three characters at once with scripting: deaths and resurrection, shops, `top`,
  `who`, `st`, `quiet`, parties and reconnects. MegaMud reads colour codes, prompts and room
  text byte by byte, so this is a strict check of the text.
- **Review.** Changes to game behaviour are reviewed independently and checked with a live
  side-by-side run before they merge.

Details: [docs/testing.md](docs/testing.md) and [docs/differences.md](docs/differences.md).

## Building from source

You need [rustup](https://rustup.rs) and a C linker: `build-essential` on Debian and Ubuntu,
`xcode-select --install` on macOS (with Homebrew, `brew install rustup`, then put
`$(brew --prefix rustup)/bin` on your `PATH`). The toolchain is pinned in
`rust-toolchain.toml`; rustup installs it on the first `cargo` command. Use rustup, not a
distribution's `cargo` package, which is usually older.

```sh
git clone https://github.com/jnfisher/silvermere.git && cd silvermere
cargo build --release
./target/release/silvermere                                 # the first start, as with a release download
```

Or the same steps one at a time:

```sh
./target/release/silvermere import                          # the stock 1.11p data, downloaded from MBBSEmu
./target/release/silvermere account create NAME --sysop     # prints Password: and reads one line
./target/release/silvermere serve                           # then: telnet localhost 2323
```

`import --from DIR` imports your own
1.11p files, or a running BBS's live `.DAT` files with `--live`
([docs/import.md](docs/import.md)).

`cargo test --workspace` runs the unit, rules, snapshot and end-to-end tests; database tests run on SQLite
and use PostgreSQL when `SILVERMERE_DATABASE_URL` points at one. A differential comparison against the
original runs on the maintainers' runner against an installed copy (harness in
[tools/differential](tools/differential/README.md)). The docs are checked by
`python3 tools/docs/check_links.py` and `cargo test -p silvermere --test all docs_in_sync::`.
How to contribute: [CONTRIBUTING.md](CONTRIBUTING.md).

| Path | What it is |
|---|---|
| `crates/silvermere-types` | record types, enums, ids, config structs (no I/O) |
| `crates/silvermere-rules` | pure game rules: combat, spells, regen, experience, prices |
| `crates/silvermere-text` | message catalogue, prompt and statline, ANSI |
| `crates/silvermere-engine` | the world: rooms, players, monsters, timers, commands |
| `crates/silvermere-db` | schema, migrations, repositories, persistence worker |
| `crates/silvermere-auth` | accounts, passwords, roles, login |
| `crates/silvermere-net` | telnet transport, line discipline, CP437 |
| `crates/silvermere-api` | admin HTTP API: health, metrics, who, moderation, restart, events |
| `crates/silvermere-web` | the web pages: documentation, FAQ, settings, the sysop console |
| `crates/silvermere-import` | one-time importer for the legacy world files |
| `crates/silvermere-testkit` | test client, scripted sessions, fixtures |
| `bin/silvermere` | the server binary |
| `tools/` | schema-doc generator, message converter, docs checker, differential harness, demo and stress run |

## Admin API

HTTP and JSON on `127.0.0.1:8080`. `/healthz` and `/metrics` are open; every `/v1/*` route
needs a bearer token.

```sh
curl -s localhost:8080/healthz                    # {"status":"ok",...}
curl -s localhost:8080/metrics                    # Prometheus text
silvermere admin token new NAME --scopes read     # prints the token once, and a block for silvermere.toml
curl -s -H "Authorization: Bearer TOKEN" localhost:8080/v1/who
```

Paste the printed `[[admin.tokens]]` block into `silvermere.toml` and restart. Scopes are
`read`, `write` (broadcast, a line to a player, reload the world) and `admin` (kick, ban, jail,
mute, IP bans, closing the realm, watching and typing for a player, the game's `SYSOP`
commands, restart and shutdown). Keep it on loopback: listening
anywhere else needs `admin.allow_remote = true` and logs a warning at every start. Routes, scopes and events:
[docs/admin-api.md](docs/admin-api.md); metrics: [docs/metrics.md](docs/metrics.md).

## Documentation

| Read | For |
|---|---|
| [docs/install.md](docs/install.md) | Docker, release binary, VPS and cloud, firewall, backup, upgrade |
| [docs/windows.md](docs/windows.md) | Windows: first start, connecting, hosting, running as a service |
| [docs/deploy-checklist.md](docs/deploy-checklist.md) | a checked VPS deployment, SQLite then hosted PostgreSQL |
| [docs/sysop.md](docs/sysop.md) | first steps for a new sysop, and the sysop commands |
| [docs/web.md](docs/web.md) | the server's own web pages: documentation, FAQ, settings editor and sysop console |
| [docs/cli.md](docs/cli.md) | every `silvermere` command and flag |
| [docs/config.md](docs/config.md) | every setting, environment variable and default |
| [docs/faq.md](docs/faq.md) | short answers to what sysops ask first |
| [docs/differences.md](docs/differences.md) | how this differs from the original |
| [docs/import.md](docs/import.md) | importing the legacy world files |
| [docs/auth.md](docs/auth.md) | accounts, login dialogue, lockout, BBS door (rlogin) |
| [docs/admin-api.md](docs/admin-api.md) | admin HTTP API and tokens |
| [docs/metrics.md](docs/metrics.md) | metrics, shutdown and backups |
| [docs/schema.md](docs/schema.md) | database schema (generated; CI fails if it is stale) |
| [docs/README.md](docs/README.md) | the full index |

## Contributing

This repository aims to stay faithful to MajorMUD 1.11p. Contributions that keep the game as it
was are welcome: fixes, new platforms, hosting, tooling, documentation and client
compatibility. A place where silvermere behaves differently from the original is a bug, and a
fix for it belongs here; say how you checked it against the original running under MBBSEmu. A
change to gameplay that departs from 1.11p does not.

To change the world, see [Editing the world](#editing-the-world) above; that needs no fork. To
change the rules, fork it. They are readable Rust, far easier to change than patching
`WCCMMUD.DLL`, as many sysops once had to.

The practical steps (building, tests, the faithfulness checks) are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## Credits

- **[MBBSEmu](https://github.com/mbbsemu/MBBSEmu)**, the MajorBBS/Worldgroup emulator that the original server
  runs under in silvermere's differential tests, and the source of the stock game data silvermere downloads.
  Working with it led to a fix upstream: `farcoreleft` now returns a positive free-memory figure.

## Game files

silvermere contains none of MajorMUD's game files (rooms, items, monsters, spells, text) and
none of its original code, and neither do its release downloads. The first start downloads the
stock 1.11p package published by MBBSEmu
([WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip](https://download.mbbsemu.com/modules/WCCMMUD/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip)),
checks its sha256, and builds the world on your machine; `silvermere import --from DIR` imports
your own BBS's files instead ([docs/install.md](docs/install.md#game-data),
[docs/import.md](docs/import.md)). The test suite holds short recordings of the original's
output under MBBSEmu (`tests/captures/`), used to check that silvermere behaves the same. The
differential harness (`tools/differential/`) runs the original next to silvermere, so it needs
an installed copy.

## License and trademark

silvermere is released under the MIT license. See [LICENSE](LICENSE).

MajorMUD® is a registered trademark. In November 2026 the MajorMUD trademark, software and
copyrights were acquired by Richard A. "Rick" Hadsall Jr. silvermere is an independent project,
not affiliated with or endorsed by the owner of MajorMUD.
