# Deploy checklist

The steps of a real deploy on a VPS, run top to bottom: (A) on SQLite, then (B) against a
hosted PostgreSQL (Neon). The runbook behind them is [install.md](install.md#vps-runbook).
Each step gives the command, what success looks like, and what to paste back into the pull
request.

`HOST` is the VPS's address; `you` your login on it. Commands marked **(VPS)** run on the
VPS over SSH, **(home)** on your own machine, **(Windows VM)** in MegaMud.

**Never paste a password, a token or a connection string.** silvermere prints database URLs
as `USER:****@HOST`, and the smoke test prints no address. Before each paste, replace the
VPS's name or address with `HOST`, any other address (yours, players') with `x.x.x.x`, and
the Neon endpoint (`ep-...` in a host name) with `ENDPOINT`; each step below says where they
show. Type secrets only into the prompts and files named here, never on a command line,
where they land in your shell history.

What you need: the VPS as provisioned (key-only SSH, ufw with OpenSSH and 2323/tcp, the repo
cloned at `~/silvermere`); Python 3 at home; MegaMud on the Windows VM; for (B), a Neon
account. Pick two passwords now, used nowhere else: one for your sysop account, one for the
`smoketest` account.

Each step starts with **Needs:**, the steps it builds on. Skipping ahead works only when
those are done. Logged in as root, `sudo` is harmless, so run the commands as written.

### Your layout

The server runs one of two ways, both set up by [install.sh](install.md#installsh-linux) and
`silvermere service install`:

- **User unit** (the default): your own user, the binary and data in `~/silvermere`, the unit at
  `~/.config/systemd/user/silvermere.service`, the database URL (B) in
  `~/.config/silvermere/env`.
- **System unit** (`--system`): a `silvermere` user, the binary and data in `/opt/silvermere`,
  `/etc/systemd/system/silvermere.service`, the database URL in `/etc/silvermere/env`.

Your **working directory** is `~/silvermere` or `/opt/silvermere`. The VPS commands below
use four shell variables for the difference. Paste your layout's line at the start of every
SSH session (in bash, Ubuntu's default shell; zsh would not split `$SM` into words):

```sh
# user unit
D=~/silvermere; SM="./silvermere"; SC="systemctl --user"; JC="journalctl --user"
# system unit
D=/opt/silvermere; SM="sudo -u silvermere ./silvermere"; SC="sudo systemctl"; JC="journalctl"
```

`cd $D && $SM backup` then runs the right binary as the right user in the right place,
`$SC restart silvermere` restarts the right unit, and `$JC -u silvermere` reads its journal.
With a hosted database (B), commands run by hand need the URL loaded first: user unit
`set -a; . ~/.config/silvermere/env; set +a`; system unit `sudo -u silvermere sh -c 'set -a;
. /etc/silvermere/env; set +a; ./silvermere ...'`.

A unit written by hand before `service install` existed (deploys A and B) is replaced by
`service install`'s, at the same path. Delete any drop-in beside it first
(`rm -r ~/.config/systemd/user/silvermere.service.d`, or the `/etc/systemd/system/` one):
its settings are in the new unit, and a stale one would override them.

## A. SQLite

### A0. Coming from a hand-run server (VPS)

Needs: nothing. Skip this step if `silvermere serve` has never run by hand on the box.

A server started by hand (in tmux or a terminal) holds port 2323, and the service in A5
needs that port. This applies to either layout. Stop the hand-run server first: attach (`tmux attach`) and press Ctrl-C
once, then wait for `shutdown complete: N characters saved`. A second Ctrl-C aborts the
save. Without tmux, `pkill -INT -x silvermere` does the same. When it has stopped,
`ss -ltn | grep 2323` prints nothing.

With the user unit, the hand-run server's data in `~/silvermere/data` is already where the
unit looks: A3 and A4 find the world and the sysop there and skip them.

With the system unit, to keep that world (accounts, characters, room state) instead of
importing a fresh one, copy the data directory before A2 (A3 and A4 then skip themselves).
`useradd` first, so the copy can be given to the `silvermere` user:

```sh
sudo useradd --system --user-group --home-dir /opt/silvermere --shell /usr/sbin/nologin silvermere
```

The copy is safe to run again: it fills the directory rather than nesting a second `data/`
inside it. The `chmod` keeps the database and logs away from other users, as the unit's
`UMask` does for files the service writes later.

```sh
sudo install -d -o silvermere -g silvermere /opt/silvermere/data
sudo cp -a ~/silvermere/data/. /opt/silvermere/data/
sudo chown -R silvermere:silvermere /opt/silvermere/data
sudo chmod -R o-rwx /opt/silvermere/data
```

`~/silvermere/data` is the hand-run server's data directory if it ran from `~/silvermere`
with no `--data-dir`. If it ran with `--data-dir DIR`, copy `DIR/.` instead. If a
`silvermere.toml` sat beside it (in `~/silvermere` or in the data directory), copy that to
`/opt/silvermere/silvermere.toml` with the same owner and `chmod 600`, and check its
`database.url` and `data_dir`: they must not point back into `~/silvermere`.

Then skip A4 if the sysop account already exists in that world.

Paste: your layout, whether you stopped a hand-run server, and whether you copied its data,
kept it in place or imported.

### A1. The box (VPS)

Needs: the VPS as provisioned (see above).

```sh
sudo sshd -T | grep -E '^(passwordauthentication|kbdinteractiveauthentication|permitrootlogin) '
sudo ufw status verbose
lsb_release -ds; nproc; free -h | head -2
```

Success: `passwordauthentication no`, `kbdinteractiveauthentication no`, `permitrootlogin no`;
ufw `Status: active` allowing only `OpenSSH` and `2323/tcp`. If the Vultr instance is in a
firewall group, the group allows 22 and 2323/tcp inbound and nothing else.

Paste: all of the output, and whether a Vultr firewall group is in use.

### A2. Install (VPS)

Needs: A1, and A0 if a server was run by hand. No Rust toolchain, no build.

```sh
curl -fsSL https://raw.githubusercontent.com/jnfisher/silvermere/main/packaging/install.sh | sh
```

(For the system unit: `| sudo sh -s -- --system`.) It prints what it will do and asks; answer
yes. At `Database: SQLite ... or a hosted PostgreSQL?` press Enter: A runs on SQLite, and B
moves it to Neon with `service install` later. Success: `SHA-256 checked: ...` and `Installed silvermere VERSION in ...`. Paste: from
`This will:` to the `Installed` line.

### A3. The world (VPS)

Needs: A2. install.sh goes straight on to `silvermere setup`.

Success: `First start: building the world in ...` then `The world is built.` If A0 kept or
copied a world, setup finds it and builds nothing. Paste: those lines.

### A4. The sysop (VPS)

Needs: A3. setup asks next (skipped when the world already has a sysop).

Type the sysop's name, then the password twice; the password does not show. Success:
`Created the sysop account NAME.` Paste: that line.

### A5. The service (VPS)

Needs: A4 (A0 first if a server was run by hand: port 2323 must be free).

install.sh asks `Install it as a systemd service, started now and at boot?`: answer yes. (Or
later, `cd $D && ./silvermere service install`, with `sudo` and `--system` for the system unit.)
It writes the unit, turns on lingering for the user unit, starts it and waits for
`telnet listening on`. If it prints a `sudo loginctl enable-linger` line, run that. Then:

```sh
sudo ss -ltnp | grep silvermere
curl -s localhost:8080/healthz; echo
loginctl show-user "$USER" -p Linger        # user unit
```

Success: `silvermere: telnet listening on 0.0.0.0:2323`, the status `active (running)`;
`ss` shows `0.0.0.0:2323` and `127.0.0.1:8080` only; `healthz` answers; `Linger=yes`.
Paste: everything `service install` printed and the three checks.

### A6. Reachable from outside (home)

Needs: A5.

```sh
nc -vz -w 5 HOST 2323      # succeeds
nc -vz -w 5 HOST 8080      # times out or is refused: the admin API is loopback only
telnet HOST 2323           # Username: appears; Ctrl-] then quit to leave
```

Paste: the two `nc` lines and whether `Username:` appeared. `nc -v` prints the address:
mask it as `HOST`.

### A7. Smoke test (home)

Needs: A5; a clone of the repository at home.

From a clone of the repository at home (same branch):

```sh
read -rsp 'smoketest password: ' SMOKE_PASSWORD && export SMOKE_PASSWORD && echo
python3 tools/smoke/smoke.py HOST --state ~/smoke-A.state
```

`read -s` takes the password without echoing it or writing it to the shell history; it lasts
for this terminal, so the later steps reuse it (leave `SMOKE_PASSWORD` unset and the script
asks each time instead).

Success: the last line is `PASS`. The first run signs up `smoketest` with `NEW` and creates
its character; later runs log in, and with the state file a missing account fails rather
than signing up again. The time on each line is a fair measure of lag from home.
Paste: the whole output. It names no address; if a `FAIL` shows the last screen, check it
for anything you typed before pasting.

### A8. MegaMud (Windows VM)

Needs: A5.

Add a BBS entry: address `HOST`, port `2323`, ANSI. Log in with the sysop account (or a new
one through `NEW`) and let the login script run: `Username:`, `Password:`, `M` at `Make your
selection (X to exit):`, `E` at `[MAJORMUD]:`. Walk a few rooms, `stat`, look at the
statistics panel, let it map a room or two, then quit.

Paste: whether the script logged in without help, what showed wrong (colours, box drawing,
prompts, the HP/MA bar, the map), and how the lag felt. A screenshot helps if anything is off.

### A9. Restart keeps state (VPS, then home)

Needs: A5, and A7 (its `~/smoke-A.state`).

```sh
$SC restart silvermere                                             # VPS
python3 tools/smoke/smoke.py HOST --state ~/smoke-A.state           # home
```

Success: a line `still in '...', where the last run left (--state)` and `PASS`. Paste: the
output, and `$JC -u silvermere -n 8 --no-pager` (VPS), which should show
`shutdown complete: N characters saved`. The journal lines start with the VPS's host name:
mask it as `HOST`.

### A10. Reboot keeps state (VPS, then home)

Needs: A5, and A7 (its `~/smoke-A.state`).

```sh
sudo reboot                                                        # VPS; wait a minute
python3 tools/smoke/smoke.py HOST --state ~/smoke-A.state           # home
$SC status silvermere --no-pager | head -5                          # VPS, after logging back in
```

Success: `PASS` with the `still in` line; the service `active (running)` since the boot (for
the user unit, without anyone logging in first: run the smoke test before you SSH in).
The script keeps trying to connect for a minute (`--wait`) while the box boots.
Paste: both, the host name in the status lines masked as `HOST`.

### A11. Backup, then restore into a fresh data directory (VPS)

Needs: A3–A5 in either layout (your working directory and the service running from it), and
A7 for the `--state` check.

```sh
cd $D
$SM backup                                                         # prints the file it wrote
$SC stop silvermere
mv data data.before-restore                                        # sudo mv for the system unit
$SM restore data.before-restore/backups/silvermere-YYYYMMDD-HHMMSS.db   # and its .assets/
$SC start silvermere
```

Then at home: `python3 tools/smoke/smoke.py HOST --state ~/smoke-A.state`.

Success: `backup` prints the file and `... .db.assets (93 files)` (the assets are copied
first, the database last); `restore` prints
`restored ...` and `restored 93 asset files to ./data/assets`; then `PASS` with the `still in`
line, and the room descriptions show. Paste: the backup and restore lines and the smoke output. These show paths, no
addresses. Keep `data.before-restore`
until you are happy, then remove it: `rm -r $D/data.before-restore` (with `sudo` for the
system unit).

Also add the nightly backup: the line `service install` printed at A5 (`Backups:`), with
`crontab -e` for the user unit or `sudo crontab -u silvermere -e` for the system unit. Run
`service install` again after B3 to get the line that loads the env file (cron does not
read the unit's).

### A12. Logs (VPS)

Needs: A5.

```sh
$JC -u silvermere -n 30 --no-pager
ls $D/data/logs/
sudo tail -n 20 $D/data/logs/silvermere.log.$(date -u +%F)
```

Success: readable start, stop and login lines; no `ERROR`. Paste: the output, with the host
name masked as `HOST` and players' addresses as `x.x.x.x`.

## B. Hosted PostgreSQL (Neon)

### B1. The database (Neon console)

Needs: a Neon account.

Create a project in the region nearest the VPS (AWS US East 2, Ohio, for Chicago). In
**Connect**, turn **Connection pooling off**: the host must not contain `-pooler`. Copy the
string; you'll paste it on the VPS in B3, nowhere else.

Then in Neon's SQL Editor run `SHOW server_version;` (the dashboard shows it too). Paste:
the region and the version, for example `17.5`. Not the string.

### B2. Stop the SQLite server (VPS)

Needs: A5 (the service, either layout) and B1 (the server's version).

```sh
$SC stop silvermere
sudo apt-get install -y postgresql-client
pg_dump --version
```

Success: `pg_dump --version` prints B1's major version or a newer one. `pg_dump` refuses a
newer server, which B8 would show. Ubuntu 26.04's own client is newer than Neon's server, so
this is usually enough. Only if `pg_dump` is older than B1's version, add the PGDG repository
and install the server's major version from it (`NN` is B1's major version, for example 17):

```sh
sudo apt-get install -y postgresql-common
sudo /usr/share/postgresql-common/pgdg/apt.postgresql.org.sh -y    # the PGDG repository
sudo apt-get install -y postgresql-client-NN
pg_dump --version
```

The SQLite database stays in `data/` and is used again if you remove the `[database]` table
later. Paste: the `pg_dump --version` line next to B1's version, and whether you needed PGDG.

### B3. The URL, the world and the sysop (VPS)

Needs: B2.

First the **pooled** string (Neon's Connect with pooling on: the host with `-pooler`), to see
it refused. `service install` prompts for it without showing it:

```sh
cd $D && ./silvermere service install --database-url-from-stdin     # sudo ... --system for the system unit
```

Success: it fails with `goes through a transaction-mode connection pooler` and writes
nothing. Paste: the error.

Then the same command with the **direct** string. Keep `sslmode=require`;
`&channel_binding=require` can stay. It writes the URL to your layout's env file (mode 600,
single-quoted, checked through `sh`), builds the world in Neon, asks for the sysop as in A4,
then restarts the service on Neon and waits for `telnet listening on`. To time the import,
put `time` in front.

Success: `Wrote the database URL to ...`, `First start: building the world in ...`,
`The world is built.`, `Created the sysop account NAME.`, the listening line and `active
(running)`. No `ignoring unrecognized connect parameter` warning. Paste: all of it, with
the `time` output, the Neon endpoint masked as `ENDPOINT`.

### B4. On Neon (VPS)

Needs: B3.

```sh
$JC -u silvermere -n 20 --no-pager | grep -E 'database|listening'
```

Success: `silvermere: database postgres://USER:****@ENDPOINT...`. Paste: those lines, masked.

### B5. Smoke, MegaMud

Needs: B4.

```sh
python3 tools/smoke/smoke.py HOST --state ~/smoke-B.state                         # home
```

Then MegaMud as in A8. Success: `PASS`. Paste: the smoke output, and how the game feels
against A (lag on moving, on `stat`, on combat).

### B6. TLS (VPS)

Needs: B5 (the server running against Neon), and `psql` (installed with B2's client).

Neon's proxy ends TLS, so the database cannot see whether a client used it:
`pg_stat_ssl` shows `ssl = f` for every connection there. Check from the client instead.
The URL goes to `psql` without its password, which goes in `PGPASSWORD`, so `ps` shows
none. The user layout has the URL loaded (see Your layout); the first line loads it for the
system layout:

```sh
SILVERMERE_DATABASE_URL=$(sudo sh -c '. /etc/silvermere/env; printf %s "$SILVERMERE_DATABASE_URL"')   # system layout only
rest=${SILVERMERE_DATABASE_URL#*://}; cred=${rest%@*}
export PGPASSWORD=${cred#*:}                        # the password (as written in the URL)
url="postgres://${cred%%:*}@${rest##*@}"            # the URL without it
psql "$url" -c '\conninfo'
nossl=${url/sslmode=require/sslmode=disable}; nossl=${nossl/channel_binding=require&/}; psql "${nossl/&channel_binding=require/}" -c 'SELECT 1'
unset PGPASSWORD rest cred url nossl
```

A password with `%`-escapes in the URL needs them decoded for `PGPASSWORD`; Neon's
generated ones have none.

Success: the first prints `SSL connection (protocol: TLSv1.3, cipher: ...)`, or with psql 18
a table with `SSL Connection | true` and `SSL Protocol | TLSv1.3`. The second is refused by
Neon: `connection is insecure (try using sslmode=require)`. Paste: the SSL line or rows and
the refusal, with the host, user and database in them masked (`ENDPOINT`).

On a self-hosted PostgreSQL (no proxy), the server's view is valid too:
`SELECT s.ssl, s.version FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid) WHERE
a.application_name = 'silvermere writer guard';` shows `t` and the TLS version.

### B7. Dropped connections (Neon console and VPS)

Needs: B5; its `~/smoke-B.state`, or a MegaMud login as the check when the smoke test is not
set up; and a unit that restarts the server (step 3 relies on it). Check that first:

```sh
$SC show silvermere -p Restart -p RestartUSec -p StartLimitIntervalUSec -p TimeoutStopUSec -p DropInPaths
```

Success: `Restart=on-failure`, `RestartUSec=5s`, `StartLimitIntervalUSec=0` (shown as
`infinity` by some versions), `TimeoutStopUSec=1min`. A unit written by hand may lack
some of these: run `service install` again (see Your layout) and check again.

Keep `$JC -u silvermere -f` running in one SSH window. A build of this PR writes the guard's
lines to the journal; an older one wrote them only to `data/logs/`.

1. **Restart the compute** (Neon console: the branch's compute, then Restart). Wait a minute.
   Success: `writer guard connection failed; re-taking the lock`, then `writer guard re-taken`.
   Run the smoke test again (`--state ~/smoke-B.state`), or log in with MegaMud.
2. **A short block** (VPS, second window), about 30 seconds:

   ```sh
   sudo iptables -I OUTPUT -p tcp --dport 5432 -j REJECT; sleep 30; sudo iptables -D OUTPUT -p tcp --dport 5432 -j REJECT
   ```

   Success: `writer guard not re-taken yet` lines, then `writer guard re-taken`; the service
   keeps running.
3. **A long block**, about 90 seconds (same command with `sleep 90`). Success: `could not be
   re-taken within 60 s ... flushing and stopping`, the service exits and systemd restarts it
   every 5 s until the block goes, then it runs again. Run the smoke test (or a MegaMud
   login) once more.

Paste: the journal lines of each, and the smoke outputs. Mask the host name as `HOST`; an
error may name the Neon endpoint or its address, so mask those as `ENDPOINT` and `x.x.x.x`.

### B8. Backup (VPS)

Needs: B2 (`pg_dump` of the right version) and B4. In the user layout, the URL loaded into
this session (`set -a; . ~/.config/silvermere/env; set +a`): without it `backup` backs up the
SQLite file in `data/` instead.

```sh
cd $D && time $SM backup
```

Success: `data/backups/silvermere-YYYYMMDD-HHMMSS.dump` (a `pg_dump` archive: its first
bytes are `PGDMP`) and its `.dump.assets/` beside it. A `server version mismatch`
means `pg_dump` is older than the server: back to B2. Paste: the line and the time.

### B9. Afterwards

Needs: B3.

Stay on Neon, or go back to SQLite: delete your layout's env file
(`~/.config/silvermere/env` or `/etc/silvermere/env`), then run `service install` again (it
drops the unit's `EnvironmentFile` and restarts the service).

## What to paste back

One comment on the pull request, by step:

```
A0  hand-run:       layout (system or user unit); stopped? data copied, kept or imported?
A1  box:            ...
A2  install:        install.sh's plan, SHA-256 and Installed lines
A3  world:          setup's lines
A4  sysop:          Created line
A5  service:        service install's output, ss, healthz (Linger=yes for a user unit)
A6  outside:        nc 2323 / nc 8080 / telnet
A7  smoke:          output
A8  MegaMud:        login script ok?, display issues, lag
A9  restart:        smoke output, journal
A10 reboot:         smoke output, status
A11 backup/restore: lines, smoke output
A12 logs:           excerpt
B1  Neon:           region, version
B2  pg_dump:        version vs B1's; PGDG needed?
B3  URL:            pooler refusal; service install's output with the direct URL, time
B4  on Neon:        journal lines
B5  smoke/MegaMud:  smoke output, feel
B6  TLS:            SSL connection line; sslmode=disable refusal
B7  drops:          unit settings (show); journal lines for restart, 30 s, 90 s; smoke or MegaMud
B8  backup:         line, time
Anything else odd:
```
