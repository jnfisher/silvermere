# Install and run

Everything silvermere needs is in the image or the release archive except the game data: the
stock 1.11p world is downloaded once, on the first import, and checked (see
[Game data](#game-data)).

From nothing to logged in on Linux (x86_64 or aarch64), with no Rust toolchain:

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
```

It downloads the latest release, checks its SHA-256, installs it in `~/silvermere`, downloads
the stock game data and checks it, builds the world, asks for the sysop and offers `silvermere service install`
([Release binary](#release-binary) has the details). From a clone:

```sh
cargo build --release
./target/release/silvermere import                          # downloads the stock world once
./target/release/silvermere account create NAME --sysop     # prints Password: and reads one line
./target/release/silvermere serve                           # then: telnet localhost 2323
```

Type `account create` on its own, not pasted with the lines after it: it takes the next line
of input as the password, and a pasted block's next line is already waiting.

- [Game data](#game-data)
- [Docker compose](#docker-compose)
- [Release binary](#release-binary)
- [VPS runbook](#vps-runbook)
- [Hosted PostgreSQL](#hosted-postgresql)
- [VPS and cloud](#vps-and-cloud)
- [Firewall](#firewall)
- [Connecting](#connecting)
- [Backup and restore](#backup-and-restore)
- [Upgrading](#upgrading)
- [Troubleshooting](#troubleshooting)

The image and release archives are published from the first tagged release. Until then, build locally (`docker compose -f docker/compose.yaml build`, or `cargo build --release`); for Fly.io or Kubernetes, push your own build to your own registry and use that image name below.

Ports: telnet `2323`, admin API `8080` (loopback by default). Image: `ghcr.io/jnfisher/silvermere`
(`latest` and version tags). Settings are in [config.md](config.md); `silvermere config show`
prints them and `silvermere config set KEY VALUE` changes one.

## Game data

silvermere ships no game data. The stock MajorMUD 1.11p world is the MBBSEmu package
`WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` (6,673,244 bytes, sha256
`815f7b35cdbc6c4de8deb0b686031e34905da147f360655858058a1f642d3583`), published at
<https://download.mbbsemu.com/modules/WCCMMUD/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip>.
`silvermere import` with no arguments (and the first start, `silvermere` or `silvermere setup`)
downloads it once into `data/cache/`, checks its size and sha256, and builds a playable stock
world from it: the content, its initial state, and the room description, map and menu text files
in `data/assets/`. A later import uses the cached copy, checked again; a copy that no longer
matches is downloaded again, and a download that does not match is never kept. `import --download`
fetches it again regardless. A server needs outbound HTTPS for the first import only.

Offline, or behind a proxy that stops the download: fetch the zip by hand (any machine, any
browser), check its sha256 (`sha256sum`), and either save it as `data/cache/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip`
or import it directly with `silvermere import WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip`. `HTTPS_PROXY`
and `NO_PROXY` are honoured; `SSL_CERT_FILE` names a PEM bundle of extra root certificates for a
proxy with its own CA. `SILVERMERE_STOCK_URL` points the download at a mirror (the sha256 is still
checked).

Two other routes ([import.md](import.md) has the details):

- **Your own copy of the files**: `silvermere import FILE.zip`, or `--from DIR`, for a
  modified world. Works offline.
- **A running realm**: `silvermere import --from DIR --live` reads a BBS's live `WCC*.DAT`
  files, players, gangs, bank books and room state included. Stop the BBS first. Every
  player gets an account with no password; set one with `silvermere account passwd NAME`.

Import is one time; a re-import keeps the state already in the database (and the files already
in `data/assets/`) unless told otherwise.

## Docker compose

From a clone of the repository:

```sh
cp docker/.env.example docker/.env          # then set SILVERMERE_ADMIN_TOKEN: openssl rand -hex 32
docker compose -f docker/compose.yaml build # until a release image is published; after that, pull works
docker compose -f docker/compose.yaml --profile import run --rm import   # downloads the stock world once
docker compose -f docker/compose.yaml up -d --wait
docker compose -f docker/compose.yaml exec silvermere /silvermere account create NAME --sysop
telnet localhost 2323
```

`account create` prints `Password: ` on stderr, reads one line from standard input and echoes what you type; `--password-stdin` is accepted and ignored. Run it on its own line: pasted together with the next command, it reads that command as the password (and fails, or worse, succeeds). To script it, pipe it in: `printf '%s\n' "$PW" | docker compose ... exec -T silvermere /silvermere account create NAME --sysop`.

The image holds no game data: the first import downloads the stock distribution into the data
volume (`/data/cache`) and checks it; later imports use that copy. Offline, put the zip in
`docker/legacy/` and run `... run --rm import import /data/legacy/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip`.
To import your own files instead, put them in `docker/legacy/` (mounted at `/data/legacy`) and run
`docker compose -f docker/compose.yaml --profile import run --rm import import --from /data/legacy`
(add `--live` for a running realm's `.DAT` files).

The data lives in the `silvermere-data` volume. Logs: `docker compose -f docker/compose.yaml logs -f silvermere`.
The image has no shell, so run the binary directly: `exec silvermere /silvermere <command>`.

### Switch to PostgreSQL

In `docker/compose.yaml`:

1. Uncomment the `postgres` service and the `pg-data` volume.
2. Uncomment the `depends_on` block in `silvermere` and the `SILVERMERE_DATABASE_URL` line in both `silvermere` and `import`.
3. Set `POSTGRES_PASSWORD` in `docker/.env`.
4. Start the database (`up -d --wait postgres`), run the import (`--profile import run --rm import`), then `up -d --wait`. The new database is empty and the server exits on an empty database.

`SILVERMERE_DATABASE_URL` is `postgres://silvermere:PASSWORD@postgres:5432/silvermere`.

## Release binary

Each tagged release has archives for Linux (x86_64, aarch64), macOS (universal) and Windows (x86_64),
plus `SHA256SUMS`.

### install.sh (Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
# a given release, or another folder:
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh -s -- --version 0.7.0 --prefix ~/sm
```

[packaging/install.sh](../packaging/install.sh) is plain POSIX sh; read it first if you like.
It says what it is about to do and asks before doing it. Then it:

1. picks the archive for this machine (`x86_64` or `aarch64`, Linux only);
2. downloads it and checks its SHA-256 against the release's `SHA256SUMS` (a mismatch stops it
   with nothing installed);
3. installs `silvermere` in `~/silvermere` (or `--prefix`), keeping `data/` and any
   `silvermere.toml` already there, so a re-run upgrades;
4. downloads the stock game data with curl (or wget) into `data/cache/` and checks its SHA-256
   against the one pinned in the script; a mismatch stops it and keeps nothing (a copy already
   there that matches is kept; `SILVERMERE_STOCK_URL` names a mirror);
5. asks whether the server uses SQLite (in `data/`) or a hosted PostgreSQL (`--hosted` answers
   it up front);
6. on SQLite, runs `silvermere setup`: builds the stock world if the database is empty and asks
   for a sysop name and password if there is no sysop (an empty name skips it). On a hosted
   database it runs nothing on SQLite: the service install does the first start there;
7. offers `silvermere service install` ([below](#silvermere-service-install)), with
   `--database-url-from-stdin` for a hosted database, so it asks for the URL.

The checksum shows the archive arrived intact and matches what the release lists. It comes from
the same place as the archive, so it does not prove who published it: the trust is in the
`jnfisher/silvermere` GitHub releases, over HTTPS. To check by hand first, download the archive
and `SHA256SUMS` from the release page and follow [By hand](#by-hand).

It refuses to run as root. For a system service run by its own `silvermere` user, run it as
root with `--system` (`curl ... | sudo sh -s -- --system`): it installs in `/opt/silvermere`
and runs `service install --system`. With no terminal to answer on (or `--yes`), it installs
the binary and prints the remaining commands.

### silvermere service install

On Linux with systemd, run it from the folder that holds `data/` (`~/silvermere` after
install.sh). That folder becomes the unit's working directory:

```sh
cd ~/silvermere && ./silvermere service install
```

- As your own user (the default when not root, `--user`): writes
  `~/.config/systemd/user/silvermere.service` and turns on lingering
  (`loginctl enable-linger`), so the server starts at boot without a login. If that needs
  root, it prints the `sudo loginctl enable-linger` line to run.
- As root (the default as root, `--system`): creates the `silvermere` system user, writes
  `/etc/systemd/system/silvermere.service` running as that user, and hands it the data
  directory. The binary and its folder must be readable by every user: `/opt/silvermere`,
  not a home directory.
- `--database-url-from-stdin`: a [hosted PostgreSQL](#hosted-postgresql). It prompts for the
  URL (hidden) or reads one line piped in, never from the command line, so it stays out of
  `ps` and the shell history. A pooled URL is refused before anything is written. The URL
  goes single-quoted into `~/.config/silvermere/env` (`/etc/silvermere/env` for `--system`),
  mode 600 in a 700 folder, and it checks the shell reads it back unchanged.
- Before starting, it does what `setup` does against the database the unit will use: the
  world if it is empty, the sysop if there is none.
- Then `daemon-reload`, `enable`, `restart`, and it waits for `telnet listening on`. It prints
  the status, the `journalctl` command and the nightly backup line for your crontab.

A re-run rewrites the unit and keeps the data and the env file (`--database-url-from-stdin`
again replaces the URL). `silvermere service uninstall` stops the service and removes the
unit; the data and the env file stay. `--data-dir` and `--config`, given to `service
install`, go into the unit's `ExecStart`.

The unit it writes is the one shown below.

### By hand

Download the archive and the sums into one folder, then:

```sh
sha256sum -c SHA256SUMS --ignore-missing
tar xzf silvermere-VERSION-x86_64-unknown-linux-musl.tar.gz
cd silvermere-VERSION-x86_64-unknown-linux-musl
```

Layout, all relative to the folder you run from:

```
silvermere             the binary
silvermere.toml        optional config (the archive ships the commented template)
data/                  database (silvermere.db), created on first run
data/backups/          written by `silvermere backup`
data/logs/             server logs
data/assets/           room description, map and menu text files, written by import
data/cache/            the stock distribution zip, downloaded by the first import and checked
legacy/                your own 1.11p files, if you import them (--from ./legacy)
```

First run:

```sh
./silvermere import                          # downloads the stock world once; --from ./legacy for your own files
./silvermere account create NAME --sysop     # prints Password: , reads one line, echoes what you type
./silvermere serve
```

A systemd unit (what `service install --system` writes, with `Group=silvermere` and, given a
URL, `EnvironmentFile=/etc/silvermere/env`):

```ini
# /etc/systemd/system/silvermere.service
[Unit]
Description=silvermere
Wants=network-online.target
After=network-online.target
StartLimitIntervalSec=0

[Service]
User=silvermere
WorkingDirectory=/opt/silvermere
ExecStart=/opt/silvermere/silvermere serve
Restart=on-failure
RestartSec=5
TimeoutStopSec=60
UMask=0027

[Install]
WantedBy=multi-user.target
```

`TimeoutStopSec=60` gives the server time to save every character on SIGTERM: with a slow
or unreachable database the shutdown's own waits add up to at most about 57 s (the save
barrier waits up to 30 s while writes are still being saved, T-113), and systemd's kill
after the timeout would lose what was still queued (compose's `stop_grace_period` is 60 s
for the same reason). Keep both at 60 s or more. `UMask=0027` keeps the database, backups and logs it writes unreadable
to other users on the box.
`network-online.target` matters for a hosted database, which must be reachable at start.
`Restart=on-failure` with `RestartSec=5` and `StartLimitIntervalSec=0` (deliberately: no
limit on restarts) keeps trying while
the database is away: on PostgreSQL, `serve` stops after about a minute without its
database (see [Troubleshooting](#troubleshooting)), and systemd starts it again until the
database answers.

## VPS runbook

From a fresh Ubuntu LTS VPS to MegaMud logged in. A small box is plenty (1-2 vCPU, 2 GB or
more; building from source wants about 4 GB of memory and a few GB of disk). `HOST` is the
VPS's address and `you` your login. [deploy-checklist.md](deploy-checklist.md) walks the
same steps with what to check at each one.

**Telnet is plaintext.** Passwords and everything typed cross the internet unencrypted, as
they did on a BBS. Give players' accounts passwords they use nowhere else. To play encrypted,
tunnel over SSH (`ssh -L 2323:localhost:2323 you@HOST`, then connect to `localhost:2323`;
PuTTY's Connection > SSH > Tunnels does the same for MegaMud on Windows).

### 1. The box

Once, as root (skip what your provider already did):

```sh
adduser you && usermod -aG sudo you
install -d -m 700 -o you -g you /home/you/.ssh
cp ~/.ssh/authorized_keys /home/you/.ssh/ && chown you:you /home/you/.ssh/authorized_keys
# SSH keys only, no root login. sshd keeps the first value it reads, and the files in
# sshd_config.d/ are read in name order, so 00- wins over cloud-init's 50-cloud-init.conf.
cat > /etc/ssh/sshd_config.d/00-hardening.conf <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
EOF
sshd -t && systemctl reload ssh
ufw allow OpenSSH && ufw allow 2323/tcp && ufw --force enable
apt-get update && apt-get -y upgrade
```

Before closing the root session, check from your own machine that `ssh you@HOST` works and
that `ssh -o PubkeyAuthentication=no you@HOST` is refused. `sudo sshd -T | grep -E
'passwordauth|permitroot'` shows the values in force.

If the provider has its own firewall (a Vultr firewall group, a DigitalOcean cloud firewall, a
Hetzner firewall), allow `22/tcp` and `2323/tcp` inbound there too. Nothing else: the admin
API (`8080`) stays on loopback.

### 2a. Run it: the binary (recommended)

As `you`, with no Rust toolchain:

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
```

Answer its questions: go ahead, Enter for SQLite, the sysop's name and password, then yes to
the service. That leaves a user unit running `~/silvermere/silvermere` from `~/silvermere`,
with the data in `~/silvermere/data`.

For a hosted PostgreSQL, answer `p` at the database question (or pass `--hosted`), then yes to
the service. It asks for the direct URL (it does not show as you paste it), refuses a pooled
one, builds the world and asks for the sysop in that database, then starts. Nothing is built
in SQLite. Later, or to change the URL:

```sh
cd ~/silvermere && ./silvermere service install --database-url-from-stdin   # paste the direct URL at the prompt
```

Check it:

```sh
systemctl --user status silvermere --no-pager
sudo ss -ltnp | grep silvermere             # 0.0.0.0:2323 and 127.0.0.1:8080, nothing else
```

For a system unit under its own `silvermere` user in `/opt/silvermere` instead:
`curl -fsSL .../install.sh | sudo sh -s -- --system`. [Release binary](#release-binary) has what
each step does. An upgrade is the same `curl ... | sh` (it keeps the data), a
[backup](#4-backups), then `systemctl --user restart silvermere`.

#### By hand: build from source, write the unit yourself

Build from a clone, as `you`:

```sh
sudo apt-get install -y build-essential pkg-config git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. ~/.cargo/env
git clone https://github.com/jnfisher/silvermere.git ~/silvermere   # or your deploy key's URL
cd ~/silvermere && cargo build --release --locked                    # rustup fetches the pinned toolchain
```

Use rustup, not the distribution's `cargo` package: Ubuntu's is older than the toolchain
pinned in `rust-toolchain.toml` and cannot build it. Or use a
[release archive](#release-binary) instead of building.

Install it under its own user, import the stock world (downloaded once) and create the first sysop:

```sh
sudo useradd --system --home-dir /opt/silvermere --shell /usr/sbin/nologin silvermere
sudo install -d -o silvermere -g silvermere /opt/silvermere
sudo install -m 0755 ~/silvermere/target/release/silvermere /opt/silvermere/silvermere
cd /opt/silvermere                                  # data/ is relative to where it runs
sudo -u silvermere ./silvermere import
sudo -u silvermere ./silvermere account create NAME --sysop   # prints Password: and echoes what you type
```

Save the [systemd unit](#by-hand) as `/etc/systemd/system/silvermere.service`, then:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now silvermere      # enabled: it starts again after a reboot
systemctl status silvermere --no-pager
sudo ss -ltnp | grep silvermere             # 0.0.0.0:2323 and 127.0.0.1:8080, nothing else
```

Logs: `journalctl -u silvermere -f` has the start and stop lines (with the database in use),
warnings and errors, and the writer guard's and persistence's own lines;
`/opt/silvermere/data/logs/silvermere.log.YYYY-MM-DD` (one file a day) has everything.

#### A user unit instead (no root, runs from the clone)

The server can also run as your own user, straight from the clone, under systemd's user
manager: no `silvermere` user, no `/opt`, no `sudo` for the service. Its working
directory is `~/silvermere`, so the data is in `~/silvermere/data`. Import and create the
sysop from there. For a hosted PostgreSQL, set up its URL first ([Hosted
PostgreSQL](#hosted-postgresql), the user unit's env file) and load it into the shell
(`set -a; . ~/.config/silvermere/env; set +a`), or these write to SQLite:

```sh
cd ~/silvermere
./target/release/silvermere import
./target/release/silvermere account create NAME --sysop
```

Save this as `~/.config/systemd/user/silvermere.service`:

```ini
# ~/.config/systemd/user/silvermere.service
[Unit]
Description=silvermere (user unit)
StartLimitIntervalSec=0

[Service]
WorkingDirectory=%h/silvermere
ExecStart=%h/silvermere/target/release/silvermere serve
# the database URL for a hosted PostgreSQL, if any (see Hosted PostgreSQL); "-" makes it optional
EnvironmentFile=-%h/.config/silvermere/env
Restart=on-failure
RestartSec=5
TimeoutStopSec=60
UMask=0027

[Install]
WantedBy=default.target
```

For a unit you already have, put these settings in a drop-in rather than editing it. Write
it directly; `systemctl --user edit` can leave nothing saved:

```sh
mkdir -p ~/.config/systemd/user/silvermere.service.d
cat > ~/.config/systemd/user/silvermere.service.d/override.conf <<'EOF'
[Unit]
StartLimitIntervalSec=0

[Service]
EnvironmentFile=-%h/.config/silvermere/env
Restart=on-failure
RestartSec=5
TimeoutStopSec=60
UMask=0027
EOF
systemctl --user daemon-reload
systemctl --user show silvermere -p DropInPaths -p EnvironmentFiles
```

`DropInPaths` and `EnvironmentFiles` must both list the files. `StartLimitIntervalSec` goes
in `[Unit]`: in `[Service]` systemd ignores it (`Unknown key`). The system unit takes the same
drop-in at `/etc/systemd/system/silvermere.service.d/override.conf`, without the
`EnvironmentFile` line (its URL is in `silvermere.toml`).

Then:

```sh
sudo loginctl enable-linger "$USER"         # start your user manager at boot, without a login
systemctl --user daemon-reload
systemctl --user enable --now silvermere
systemctl --user status silvermere --no-pager
journalctl --user -u silvermere -f
```

The settings mean what they do in the system unit. SIGTERM is systemd's default stop signal,
and `serve` saves every character on it. A user unit cannot wait for the system's
`network-online.target`, so at boot `serve` may start before a hosted database is reachable.
`Restart=on-failure` then tries again every 5 s. `cargo build --release` replaces the binary
the unit runs, so an upgrade is `git pull --ff-only`, the build, a backup, then
`systemctl --user restart silvermere`. The nightly backup goes in your own crontab
(`crontab -e`): `0 4 * * * cd ~/silvermere && set -a && . ~/.config/silvermere/env && set +a && ./target/release/silvermere backup --keep 14`. cron does not read the unit's `EnvironmentFile`, so the line
loads it; without that, on a hosted database the backup would quietly be of the SQLite
default in `data/`. (Drop the `set -a ... set +a` part on SQLite.)

### 2b. Run it: Docker compose

With Docker installed ([get.docker.com](https://get.docker.com)), from the clone:

```sh
cd ~/silvermere
cp docker/.env.example docker/.env
sed -i.bak "s/^SILVERMERE_ADMIN_TOKEN=.*/SILVERMERE_ADMIN_TOKEN=$(openssl rand -hex 32)/" docker/.env
rm docker/.env.bak                          # the backup sed left still holds the placeholder
chmod 600 docker/.env
docker compose -f docker/compose.yaml build
docker compose -f docker/compose.yaml --profile import run --rm import
docker compose -f docker/compose.yaml up -d --wait
docker compose -f docker/compose.yaml exec silvermere /silvermere account create NAME --sysop
```

`restart: unless-stopped` and Docker's own service (enabled by the installer) bring it back
after a reboot. Docker publishes ports past ufw, so `2323` is reachable whatever ufw says;
the admin port is published on `127.0.0.1` only. Logs:
`docker compose -f docker/compose.yaml logs -f silvermere`. In the commands below, run
`silvermere` as `docker compose -f docker/compose.yaml exec silvermere /silvermere`.

### 3. Connect

From another machine: `telnet HOST 2323`, or MegaMud with a BBS entry for `HOST` port `2323`
([Connecting](#connecting)). `tools/smoke/smoke.py HOST` checks login, a walk and a re-login
from outside ([tools/smoke/README.md](../tools/smoke/README.md)).

### 4. Backups

A nightly backup, kept 14 days, from the silvermere user's crontab
(`sudo crontab -u silvermere -e`):

```
0 4 * * * cd /opt/silvermere && ./silvermere backup --keep 14
```

Copy them off the box now and then, from your own machine:
`rsync -a you@HOST:/opt/silvermere/data/backups/ ./silvermere-backups/`.

A backup is two things side by side: the database copy (`silvermere-YYYYMMDD-HHMMSS.db` on
SQLite, `.dump` on PostgreSQL, a `pg_dump` archive) and `<that name>.assets/`, a copy of the
room description, map and menu text files in `data/assets/`. Restore puts both back, so a
new box needs no fresh `import`. `--keep` removes the two together.

To restore:

```sh
sudo systemctl stop silvermere
cd /opt/silvermere && sudo -u silvermere ./silvermere restore data/backups/silvermere-YYYYMMDD-HHMMSS.db
sudo systemctl start silvermere
```

### 5. Upgrade to a new commit

```sh
cd ~/silvermere && git pull --ff-only && cargo build --release --locked
cd /opt/silvermere && sudo -u silvermere ./silvermere backup
sudo install -m 0755 ~/silvermere/target/release/silvermere /opt/silvermere/silvermere
sudo systemctl restart silvermere           # migrations apply on start
```

With compose: `git pull --ff-only`, `docker compose -f docker/compose.yaml build`, the
backup, then `docker compose -f docker/compose.yaml up -d --wait`.

## Hosted PostgreSQL

The same server against a database someone else runs, set by `database.url` (or
`SILVERMERE_DATABASE_URL`). Written for Neon and checked against a TLS-only PostgreSQL 16;
any hosted PostgreSQL that allows a direct connection should work the same way.

**Use a direct connection, not a pooled one.** The single-writer guard holds a session
advisory lock for the server's lifetime, which a transaction-mode pooler (PgBouncer, Neon's
`-pooler` host, Supabase's port 6543) cannot keep: the lock would stay on a pooled backend
and guard nothing. silvermere refuses such a URL with `goes through a transaction-mode
connection pooler`: by name for Neon's `-pooler` hosts (and a self-hosted one named that
way) and Supabase's port 6543, and otherwise by a probe before it takes the lock, which
opens a second connection and compares backends. The probe is best effort: it never
refuses a direct connection, but a pooler with backends to spare can get past it, so do not
count on it. A session-mode pooler works.

**Neon** (the free tier is enough to try it):

1. Create a project in the region nearest the VPS (for a Chicago VPS, AWS US East 2, Ohio).
2. In Connect, turn **Connection pooling off** and copy the connection string: its host has
   no `-pooler`. Keep `sslmode=require`. The string ends in `&channel_binding=require`;
   silvermere's driver has never done channel binding, so the parameter was never in effect.
   silvermere drops it (and the driver's warning about it); TLS is not weakened, and it can
   stay in the URL.
3. The guard pings the database every 15 seconds, which keeps Neon's compute from scaling
   to zero while the server runs. Check that your plan's compute hours cover that.

**The URL.** Keep it out of shell history and out of the unit file, in a config file only
the silvermere user reads. Every command run from `/opt/silvermere` picks it up:

```sh
sudo install -m 600 -o silvermere -g silvermere /dev/null /opt/silvermere/silvermere.toml
sudoedit /opt/silvermere/silvermere.toml
```

```toml
[database]
url = "postgres://USER:PASSWORD@HOST/DBNAME?sslmode=require"
```

Then the same steps as [the binary](#2a-run-it-the-binary-recommended): `import` (it runs the
migrations and builds the world in the hosted database), `account create`, start the unit.
Messages and `--print-config` show the URL as `USER:****@HOST`.

`silvermere service install --database-url-from-stdin` does all of this for you: it refuses a
pooled URL, writes the env file quoted and checked, builds the world in the hosted database,
asks for the sysop and starts the unit ([silvermere service install](#silvermere-service-install)).
By hand:

With the [user unit](#a-user-unit-instead-no-root-runs-from-the-clone), put the URL in the
unit's `EnvironmentFile`, readable by you alone. Single quotes keep `&` and `$` literal, both
for systemd and for the shell that reads the file:

```sh
install -d -m 700 ~/.config/silvermere
install -m 600 /dev/null ~/.config/silvermere/env
${EDITOR:-nano} ~/.config/silvermere/env
```

```sh
SILVERMERE_DATABASE_URL='postgres://USER:PASSWORD@HOST/DBNAME?sslmode=require'
```

**The single quotes are required.** Without them the shell that loads the file takes the
`&` in the URL as "run in the background" and cuts the URL short. systemd reads the file
either way, so the unit can run while commands run by hand fail. This prints `quoted` when
the line is right:

```sh
grep -Eq "^SILVERMERE_DATABASE_URL='[^']+'\$" ~/.config/silvermere/env && echo quoted || echo "NOT QUOTED: put the URL in single quotes"
```

Commands run by hand (`import`, `backup`, `account create`) need the same variable. Load it
into the shell first; this puts nothing in your shell history:
`set -a; . ~/.config/silvermere/env; set +a`. A `[database]` table in
`~/silvermere/silvermere.toml` (mode 600) works too, but the environment variable takes
precedence over it.

With compose, put `SILVERMERE_DATABASE_URL=...` in `docker/.env` and uncomment the
`SILVERMERE_DATABASE_URL` lines in `docker/compose.yaml` for `silvermere` and `import`,
with the value `${SILVERMERE_DATABASE_URL}` (the `postgres` service stays commented out).

**TLS.** `sslmode=require` encrypts the connection without checking the certificate, as
libpq does. `sslmode=verify-full` also checks it against the public roots built into
silvermere; `pg_dump` then needs `sslrootcert=system` too, which silvermere does not read, so
keep `require` if you use `silvermere backup`. To check TLS, look from the client:
`psql` with the URL (its password in `PGPASSWORD`, not on the command line; see checklist
B6) and `-c '\conninfo'` prints `SSL connection (protocol: TLSv1.3, ...)`, or with psql 18
a table with `SSL Connection | true`. The same URL with `sslmode=disable` (and without
`channel_binding`) is refused by a server that takes TLS only; Neon answers `connection is
insecure`. The server's `pg_stat_ssl` view is valid on a self-hosted PostgreSQL. Behind a proxy
that ends TLS, such as Neon's, it shows `ssl = f` for every connection.

**Which database.** `serve` names its database at start, in the journal:
`silvermere: database postgres://USER:****@HOST/DB?sslmode=require` (or
`sqlite://./data/silvermere.db`). If it says `sqlite` after a switch to PostgreSQL, the URL
did not reach it: for a unit, check `systemctl show silvermere -p EnvironmentFiles -p
DropInPaths` (with `--user` for a user unit).

**Dropped connections.** When the guard's connection drops (a compute restart, a network
blip), `serve` logs `writer guard connection failed; re-taking the lock` and, a moment later,
`writer guard re-taken`; play goes on. After about a minute without the database it logs
`could not be re-taken within 60 s`, saves what it can and exits, and the unit's
`Restart=on-failure` starts it again until the database is back.

**Backups.** `silvermere backup` runs `pg_dump` (and `restore`, `pg_restore`), which must be
on `PATH` and of the server's major version or newer: `pg_dump` refuses a newer server. Find
the server's major version first: Neon's dashboard shows it, or run `SHOW server_version;`
in its SQL Editor. Then install the distribution's client and compare:

```sh
sudo apt-get install -y postgresql-client
pg_dump --version                       # its major version must be the server's or newer
```

Only when the distribution's client is older than the server, add the PostgreSQL project's
apt repository (PGDG) and install the server's major version from it, with `NN` standing for
that major version:

```sh
sudo apt-get install -y postgresql-common
sudo /usr/share/postgresql-common/pgdg/apt.postgresql.org.sh -y   # adds the PGDG repository
sudo apt-get install -y postgresql-client-NN                      # NN: the server's major version
pg_dump --version
```

`/usr/bin/pg_dump` runs the newest client installed. The password goes to `pg_dump` in
`PGPASSWORD`, not on its command line. Neon's own point-in-time restore is a second line.
Restore with `silvermere restore --force FILE`.

### Latency: put it in the server's region

Every statement is a network round trip, so a database in another region costs more than one on the
same host. Put a hosted database (Neon, RDS, Supabase and the like) in the same region as the server,
and check the round trip from the server with `psql "$URL" -c '\timing' -c 'select 1' -c 'select 1'`
(the second `select` is the round trip).

What T-113 measured with the database 30 ms away (after: through a test proxy; before: a deploy in
Chicago against a database in us-east-2):

| | before T-113 | after |
|---|---|---|
| `silvermere import` of the stock world | 7 min 38 s | about 20 s (9 s on the same host) |
| the first `serve` after an import: the start-up pass queues about 11,000 writes (`silvermere_persist_queue_depth`) | 15,847 queued, draining at 30 writes/s | about 10 s to drain |
| a new character's name check ("Validating your name") | minutes, behind the whole queue | about 2 s, whatever is queued |

The server writes in batches of a few statements per table, so the round trip sets how often a batch
commits rather than how long each row takes. Most of the times above are round trips, so they grow
about in step with the distance. When the database cannot keep up, `serve` logs `persistence queue backlog` at WARN with the queue depth and the
drain rate when the queue stays above 1,000 writes for 30 s; `silvermere_persist_queue_depth` in
`/metrics` (docs/metrics.md) shows the same. On shutdown the server waits for its queue while writes
are still being saved (up to 30 s, inside the 60 s stop timeout above; 15 s after a lost writer
guard, since another server may hold the database) and gives up after 15 s with none saved,
naming the writes not saved by kind, players' records first.

## VPS and cloud

The image runs unchanged anywhere that runs containers. It runs as UID 65532, so a mounted volume
must be writable by that user.

**Hetzner, DigitalOcean, Vultr or any VPS.** Follow the [VPS runbook](#vps-runbook).

**Fly.io.** One volume at `/data`, one TCP service on 2323:

```toml
# fly.toml
app = "my-silvermere"

[mounts]
  source = "silvermere_data"
  destination = "/data"

[env]
  SILVERMERE_SERVER__ADMIN_BIND = "127.0.0.1:8080"   # the image default needs a token

[[services]]
  protocol = "tcp"
  internal_port = 2323
  [[services.ports]]
    port = 2323
```

Create the volume with `fly volumes create silvermere_data`. The server exits on an empty database, so import first, through a temporary machine:

```sh
fly machine run ghcr.io/jnfisher/silvermere:latest --volume silvermere_data:/data --rm -- import
fly deploy --image ghcr.io/jnfisher/silvermere:latest
```

To import your own files instead, copy them in first. The silvermere image has no shell, and a new volume is owned by root, so use a small image for the copy:

```sh
fly machine run busybox --volume silvermere_data:/data -- sleep 3600     # note the machine id
fly ssh sftp shell                                                      # put -r legacy /data/legacy
fly ssh console -C "chown -R 65532:65532 /data"
fly machine destroy MACHINE_ID --force
fly machine run ghcr.io/jnfisher/silvermere:latest --volume silvermere_data:/data --rm -- import --from /data/legacy
fly deploy --image ghcr.io/jnfisher/silvermere:latest
```

**Kubernetes.** One Deployment, one PVC, one Service. No Helm chart.

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata: { name: silvermere-data }
spec: { accessModes: [ReadWriteOnce], resources: { requests: { storage: 2Gi } } }
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: silvermere }
spec:
  replicas: 1
  strategy: { type: Recreate }
  selector: { matchLabels: { app: silvermere } }
  template:
    metadata: { labels: { app: silvermere } }
    spec:
      securityContext: { fsGroup: 65532 }
      containers:
        - name: silvermere
          image: ghcr.io/jnfisher/silvermere:latest
          env: [{ name: SILVERMERE_SERVER__ADMIN_BIND, value: "127.0.0.1:8080" }]   # the image default needs a token
          ports: [{ containerPort: 2323 }]
          volumeMounts: [{ name: data, mountPath: /data }]
      volumes: [{ name: data, persistentVolumeClaim: { claimName: silvermere-data } }]
---
apiVersion: v1
kind: Service
metadata: { name: silvermere }
spec: { type: LoadBalancer, selector: { app: silvermere }, ports: [{ port: 2323, targetPort: 2323 }] }
```

Run one replica only: the server owns the world in memory.

A new volume is empty and the server exits on an empty database, so import before the Deployment
starts. Save the PVC document as `pvc.yaml` and the other two as `app.yaml`, then (for the
stock world, downloaded into the volume, skip step 1 and pass `"args":["import"]` in step 2):

```sh
kubectl apply -f pvc.yaml
# 1. a temporary pod that mounts the volume, to copy the legacy files in
kubectl run loader --image=busybox --restart=Never --overrides='{"spec":{"volumes":[{"name":"d","persistentVolumeClaim":{"claimName":"silvermere-data"}}],"containers":[{"name":"loader","image":"busybox","command":["sleep","3600"],"volumeMounts":[{"name":"d","mountPath":"/data"}]}]}}'
kubectl wait --for=condition=Ready pod/loader
kubectl cp ./legacy loader:/data/legacy
kubectl exec loader -- chown -R 65532:65532 /data
kubectl delete pod loader
# 2. import (the image has no shell; run the binary directly)
kubectl run importer --image=ghcr.io/jnfisher/silvermere:latest --restart=Never --overrides='{"spec":{"volumes":[{"name":"d","persistentVolumeClaim":{"claimName":"silvermere-data"}}],"containers":[{"name":"importer","image":"ghcr.io/jnfisher/silvermere:latest","args":["import","--from","/data/legacy"],"volumeMounts":[{"name":"d","mountPath":"/data"}]}]}}'
kubectl wait --for=jsonpath='{.status.phase}'=Succeeded pod/importer --timeout=10m
kubectl logs importer
kubectl delete pod importer
# 3. start the server
kubectl apply -f app.yaml
```

## Firewall

- Publish `2323` only.
- Keep `8080` on loopback, or put a reverse proxy with TLS in front of it. A non-loopback
  `server.admin_bind` refuses to start without an admin token (`silvermere admin token new`).
  See [admin-api.md](admin-api.md).

## Connecting

**Telnet.** `telnet HOST 2323`. Use a terminal set to ANSI colour and the CP437 character set
(or run `set utf8 on` in the game for a UTF-8 terminal, or set `compat.charset = "utf8"`).

**MegaMud.** Add a BBS entry: host, port `2323`, terminal type "ANSI". The login script sees these prompts,
in this order:

1. `Username:` then `Password:`. Type `NEW` at the username prompt to create an account (open signup is on).
2. `Make your selection (X to exit):`. `M` enters MajorMUD, `X` logs off (`compat.bbs_menu = true`).
3. The `[MAJORMUD]:` menu. `[E] . Enter the Realm`.

More on old clients: `docs/legacy-clients.md` (TODO(T-062)).

## Backup and restore

```bash
silvermere backup --keep 14
silvermere backup --to /srv/backups/silvermere.db
```

`backup` is safe while the server runs. Without `--to` it writes
`<data dir>/backups/silvermere-YYYYMMDD-HHMMSS.db` (UTC; `.dump` on PostgreSQL, a `pg_dump`
archive), with the asset files of `data/assets/` in `<that file>.assets/` beside it.
`--keep N` deletes the oldest beyond N, each with its `.assets`, and any `.assets` whose
backup file was removed. The assets are copied first and the database last, so a database
copy always has its assets beside it; a failed copy leaves neither. An existing destination
is refused.

A nightly cron line:

```
0 4 * * * cd /opt/silvermere && ./silvermere backup --keep 14
```

For compose: `0 4 * * * docker compose -f /opt/silvermere/docker/compose.yaml exec -T silvermere /silvermere backup --keep 14`.

To restore, stop the server first:

```bash
silvermere restore data/backups/silvermere-20261015-040000.db
```

On PostgreSQL, restore replaces objects in place and needs `--force`. `restore` reads the
file's kind from its first bytes, not its name, and refuses a SQLite file for a PostgreSQL
database and the other way round. When `<file>.assets/` is beside it, it replaces
`data/assets/`; the old directory is kept as `assets.pre-restore-<time>`. The assets are
staged before the database is touched. If they cannot be put in place after the database
swap, a SQLite restore is undone (the restored copy is kept as
`silvermere.db.failed-restore-<time>`); for PostgreSQL the message says what is where. The
`.pre-restore-*` copies are the undo of a restore and are not pruned: remove them when
satisfied.

## Upgrading

1. `silvermere backup`.
2. Replace the binary, or pull the new image (`docker compose -f docker/compose.yaml pull`).
3. Start it. `serve` applies migrations itself; `silvermere migrate` does the same by hand.

A database created before the first release has to be recreated: its migrations were recorded under earlier checksums, and `serve` refuses to start on it.

## Troubleshooting

| You see | Cause and fix |
|---|---|
| `serve` exits with `another silvermere is already serving` | A second server on the same database refuses to start (`docs/admin-api.md`, single-writer guard). Stop the first one (the message names its pid); a stale `.lock` file left by a crash holds nothing and needs no cleanup. |
| `import`, `restore` or `migrate` exits with `a silvermere server is running on ...` | These write the store directly and take the same single-writer guard as `serve`, so they refuse while a server runs: its saves would overwrite what they write. Stop the server, run the command, start the server again. `account` commands and `backup` work alongside a running server. |
| `serve` logs `writer guard connection failed; re-taking the lock`, then `writer guard re-taken` | PostgreSQL only. The connection holding the guard's advisory lock dropped (a restart, a failover, a killed backend, a network blip); `serve` checks it every 15 s and took the lock again on a new connection. Nothing to do. |
| `serve` exits 1 with `the writer guard was lost and is now held elsewhere` or `could not be re-taken within 60 s` | PostgreSQL only. The guard's connection dropped and another process took the lock, or the database stayed unreachable for about a minute. `serve` saved what was queued and stopped rather than write beside another server. Find the other holder (`pg_locks`, the message names its backend pid), stop it or this one, and restart. |
| `... goes through a transaction-mode connection pooler` | The database URL is a pooled one (Neon's `-pooler` host, PgBouncer in transaction mode, Supabase's port 6543), which cannot hold the guard's session lock. Use the direct connection string ([Hosted PostgreSQL](#hosted-postgresql)). A self-hosted server whose host name ends in `-pooler` is treated the same way; connect by IP address or another name. |
| `backup` fails with `server version mismatch` | `pg_dump` is older than the server. Install a client of the server's major version or newer: the distribution's `postgresql-client` when it is new enough, otherwise PGDG's ([Hosted PostgreSQL](#hosted-postgresql)). |
| `serve` logs `ignoring unrecognized connect parameter` | A parameter in the URL that silvermere's driver does not know. Harmless; remove it from the URL to quiet the warning. (Neon's `channel_binding` is dropped by silvermere itself; a build that still warns about it predates T-108.) |
| Sourcing the env file prints `[1] 12345`, or commands then say `command not found` | The URL in `~/.config/silvermere/env` is not quoted: the shell took its `&` as "run in the background". Put the value in single quotes, `SILVERMERE_DATABASE_URL='postgres://...'`. |
| `taking the writer guard ... search_path names no schema that exists` | The `search_path` in the database URL's `options` (or the role's settings) names a schema that does not exist. Create it, or fix the setting. |
| `serve` prints that there is no world data and exits (under compose it keeps restarting) | Run `silvermere import --from DIR` on the 1.11p files first, then start the server. |
| Port in use | Something else holds `2323` or `8080`. Change `server.telnet_bind` or `server.admin_bind`, or the published port in compose. |
| Box-drawing and accents look wrong | The terminal is not CP437. Switch it to CP437, or use `set utf8 on` with a UTF-8 terminal. |
| PostgreSQL connection refused | Check `SILVERMERE_DATABASE_URL`, that the `postgres` service is up, and that the password matches `POSTGRES_PASSWORD`. |
| `no pg_hba.conf entry ... no encryption` | The database accepts TLS only: add `?sslmode=require` to the URL. |
| `room description files not loaded` after a restore | The backup has no `.assets` directory beside it (one made before T-108, or copied without it). Run the same `import` again with the server stopped; it writes `data/assets/` and keeps the restored state. |
| Admin bind refuses to start | A non-loopback `server.admin_bind` needs a token. See [Firewall](#firewall). |
| Permission denied on `/data` | The image runs as UID 65532. Make the volume writable by that user. |
