<p align="center"><img src="docs/images/silvermere-logo.png" alt="silvermere" width="100%"></p>

<p align="center"><img src="docs/images/silvermere.webp" alt="Silvermere from the air: the walled town on the Silver river, the island with the Labyrinth beyond it, and Darkwood Forest to the east" width="100%"></p>

silvermere is a faithful recreation of the MajorMUD 1.11p engine, written in Rust.

It is one binary. The stock 1.11p game data is downloaded from MBBSEmu
([WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip](https://download.mbbsemu.com/modules/WCCMMUD/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip)) and imported into the
server's own database. There are no legacy dependencies: no Worldgroup or MajorBBS, no Btrieve,
no DOS emulator, no `WCCMMUD.DLL`.

It keeps its data in a local SQLite file out of the box, or in PostgreSQL: your own server or a
hosted service such as Neon.

MegaMud and ordinary telnet clients connect on port 2323 and play unmodified.

## Play it locally

### Windows

1. Download `silvermere-VERSION-x86_64-pc-windows-msvc.zip` from the release page.
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

From the release archive:

```sh
sha256sum -c SHA256SUMS --ignore-missing
tar xzf silvermere-VERSION-x86_64-unknown-linux-musl.tar.gz
cd silvermere-VERSION-x86_64-unknown-linux-musl
./silvermere
```

Or with cargo, from a clone ([rustup](https://rustup.rs) and a C linker, `build-essential` on
Debian and Ubuntu):

```sh
git clone https://github.com/jnfisher/silvermere.git
cd silvermere
cargo build --release              # rustup fetches the toolchain pinned in rust-toolchain.toml
./target/release/silvermere
```

With no subcommand, the first start downloads the stock game data from MBBSEmu, builds the
world into `./data/silvermere.db`, asks
for a sysop name and password, and serves. Later starts go straight to serving. Then
`telnet localhost 2323`.

### macOS

With [Homebrew](https://brew.sh):

```sh
xcode-select --install             # the linker, if the command line tools are not installed
brew install rustup
export PATH="$(brew --prefix rustup)/bin:$PATH"   # Homebrew keeps rustup off the PATH; add this to ~/.zshrc
git clone https://github.com/jnfisher/silvermere.git
cd silvermere
cargo build --release              # rustup fetches the toolchain pinned in rust-toolchain.toml
./target/release/silvermere
```

The first start is the same as on Linux. Then `telnet localhost 2323`, or point MegaMud on
another machine at the Mac's address.

### Logging in

Log in with the sysop name. At `Make your selection (X to exit):` press `M`, then `E` to enter
the realm. Type `NEW` at `Username:` to create more accounts.

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

Back up nightly with `silvermere backup --keep 14`, run from `/opt/silvermere` as the
`silvermere` user; it is safe while the server runs. The full runbook, with upgrades, TLS
for PostgreSQL and troubleshooting: [docs/install.md](docs/install.md) and
[docs/deploy-checklist.md](docs/deploy-checklist.md).

## Settings

silvermere runs with no settings file. When you want one, it is `silvermere.toml` in the folder
the server runs from:

| Install | File |
|---|---|
| `install.sh --system`, `service install --system` | `/opt/silvermere/silvermere.toml` |
| `install.sh` (your user) | `~/silvermere/silvermere.toml` |
| Windows | beside `silvermere.exe`, such as `C:\Silvermere\silvermere.toml` |
| Docker | `/data/silvermere.toml`, in the volume |

`silvermere config` reads and writes it with the same checks as startup, run from that folder:

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

## Building from source

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it on the first `cargo`
command. Use rustup, not a distribution's `cargo` package, which is usually older.

```sh
cargo build --release
./target/release/silvermere import                          # the stock 1.11p data, downloaded from MBBSEmu
./target/release/silvermere account create NAME --sysop     # prints Password: and reads one line
./target/release/silvermere serve                           # then: telnet localhost 2323
```

These are the steps the first start runs, one at a time. `import --from DIR` imports your own
1.11p files, or a running BBS's live `.DAT` files with `--live`
([docs/import.md](docs/import.md)).

`cargo test --workspace` runs the unit, rules, snapshot and end-to-end tests; database tests run on SQLite
and use PostgreSQL when `SILVERMERE_DATABASE_URL` points at one. A differential comparison against the
original runs on the maintainers' runner against an installed copy (harness in
[tools/differential](tools/differential/README.md)). The docs are checked by
`python3 tools/docs/check_links.py` and `cargo test -p silvermere --test docs_in_sync`.
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
| `crates/silvermere-api` | admin HTTP API: health, metrics, who, broadcast, events |
| `crates/silvermere-import` | one-time importer for the legacy world files |
| `crates/silvermere-testkit` | test client, scripted sessions, fixtures |
| `bin/silvermere` | the server binary |
| `tools/` | schema-doc generator, message converter, docs checker, differential harness |

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
`read`, `write` (broadcast, reload the world) and `admin`. Routes, scopes and events:
[docs/admin-api.md](docs/admin-api.md); metrics: [docs/metrics.md](docs/metrics.md).

## How we know it's faithful

We ran the original `WCCMMUD.DLL` under MBBSEmu and used it as the reference: to work out how
the game behaves, and to check that silvermere behaves the same way. Where the two disagree,
the original is right.

- **Tests.** We wrote nearly 2,000 `#[test]` functions across the workspace (about 250 files), with
  over 100 snapshot files in the engine and text crates and fixture directories of expected output in
  five crates (engine, text, import, types, rules). `cargo test --workspace` runs them.
- **Differential harness (m1-m4).** We ran the original `WCCMMUD.DLL` under MBBSEmu next to
  silvermere, replayed the same scripted sessions against both, and compared the output line by
  line. Steps marked random we compared statistically; masks hide values only. There are 49
  scenarios and 31 scripts under [tools/differential](tools/differential/README.md). In the m4
  tier, deterministic steps are byte-exact and random steps are checked structurally or
  statistically. We list the known gaps in `known_gaps.json`.
- **Statistical gates (m2).** For fights and other dice rolls (T-040, #72), the full run passed
  22 gated rows over 11 sessions, with Holm correction at family alpha 0.01. Player hit-rate rows
  use a 2-point bound plus the test, monster rows the test only. We treat one row (monster swings
  per round) as advisory because of an MBBSEmu artifact: its cache floor, since fixed upstream by
  the `farcoreleft` change credited below, and not yet re-measured.
- **Real clients.** We played with MegaMud on Windows against a silvermere VPS over
  the internet, three characters at once, scripting with no problems: deaths and resurrection,
  shops, `top`, `who`, `st`, `quiet`, parties and reconnects. MegaMud parses colour codes, prompts
  and room text byte by byte, so this is a strict check of text fidelity.
- **Review.**
  Behaviour PRs are independently reviewed and, since the m4 tier (T-057), checked with a live m4
  run before merge.

Details: [docs/testing.md](docs/testing.md) and [docs/differences.md](docs/differences.md).

## Documentation

| Read | For |
|---|---|
| [docs/install.md](docs/install.md) | Docker, release binary, VPS and cloud, firewall, backup, upgrade |
| [docs/windows.md](docs/windows.md) | Windows: first start, connecting, hosting, running as a service |
| [docs/deploy-checklist.md](docs/deploy-checklist.md) | a checked VPS deployment, SQLite then hosted PostgreSQL |
| [docs/config.md](docs/config.md) | every setting, environment variable and default |
| [docs/differences.md](docs/differences.md) | how this differs from the original |
| [docs/import.md](docs/import.md) | importing the legacy world files |
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

To make your own game, fork it. Rules, text, items and areas are readable Rust and data files,
so changing them is far easier than patching `WCCMMUD.DLL`, as many sysops once had to.

The practical steps (building, tests, the faithfulness checks) are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## Credits

- **[MBBSEmu](https://github.com/mbbsemu/MBBSEmu)**, the MajorBBS/Worldgroup emulator that the original server
  runs under in silvermere's differential tests, and the source of the stock game data silvermere downloads.
  Working with it led to a fix upstream: `farcoreleft` now returns a positive free-memory figure.

## License

MIT. See [LICENSE](LICENSE).

## Game files

No game data is in this repository: the import and the tests download the stock MajorMUD
1.11p distribution (the MBBSEmu package) on first use and check its sha256
([docs/install.md](docs/install.md#game-data)). Recordings of the original's output
under MBBSEmu (`tests/captures/`) are here. The differential harness (`tools/differential/`)
runs the original under MBBSEmu next to silvermere, so it needs an installed copy.
