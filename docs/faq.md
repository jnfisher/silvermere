# FAQ

Short answers, each with the page that has the rest.

## Starting out

### How do I connect?

With a telnet client, to port 2323 of the computer the server runs on: `telnet localhost 2323`
on the same machine. The port is `server.telnet_bind`. See
[Connecting](install.md#connecting), or [Connecting on Windows](windows.md#connecting).

### How do I log in the first time?

The first start asks for a sysop name and password. Log in with that name; at
`Make your selection (X to exit):` press `M`, then `E` to enter the realm. Other players type
`NEW` at `Username:` to make an account. See [sysop.md](sysop.md).

### I have no sysop account, or lost its password.

On the server, make one or set a new password:

```sh
silvermere account create NAME --sysop
silvermere account passwd NAME
```

An existing account becomes a sysop with `silvermere account grant NAME sysop`. See
[Command line](auth.md#command-line).

### Where is the game data from?

The first start downloads the stock 1.11p package and builds the world from it, or imports the
files of your own BBS. See [Game data](install.md#game-data) and [import.md](import.md).

## Settings

### Where are the settings?

The server runs with no settings file. Change one in the browser on the
[settings page](web.md#settings), or with `silvermere config set KEY VALUE`; both write
`silvermere.toml` with the same checks as startup. See [config.md](config.md) for every key.

### I changed a setting and nothing happened.

A saved setting does not reach a running server by itself. On the settings page, **Apply
now** loads most game options (`[game]`) into the running game with nobody disconnected; the
page marks the ones that need a restart. For those, restart the server:
`sudo systemctl restart silvermere` for the service. See
[Applying a change](web.md#applying-a-change).

### Why can't I change a setting on the settings page?

Either it holds a secret (the database URL, the admin tokens), or an environment variable or a
command-line flag sets it, and those win over the file. The page names which. Remove the
variable or the flag to edit the value in the file. See [Precedence](config.md#precedence).

### How do I sign in to the settings page?

With the name and password of a game account that has the sysop role. The page is on
`http://127.0.0.1:8088` on the server itself; see [web.md](web.md) to reach it from another
computer. Signing in opens the [console](web.md#the-console); the settings are a click away.

### How do I stop strangers making accounts?

Set `game.open_signup` to `false`; then only `silvermere account create` makes accounts.

## Running the realm

### Where do I see who is playing?

In the [console](web.md#the-console), `http://127.0.0.1:8088/console`: every player in the
realm, the room they stand in, their level, class, race, gang, idle time and address, kept up
to date as they move. From a shell: `silvermere sysop who`. In the game: `SYSOP LIST USERS`.

### How do I kick, mute, jail or ban a player?

Click the player in the console and choose, with a time and a reason the player is told; or
from a shell, `silvermere sysop kick|mute|jail|ban NAME [--for 1h] [REASON]`, and
`unmute`, `release`, `unban` to lift it. A jailed player is held in the jail room (no walking,
recall or teleport out) until the time is up, then let out to town. A ban refuses the account's
logins and disconnects the player if on. Each holds for a player who is offline from the next
login, and across restarts. See [admin-api.md](admin-api.md#moderation).

### How do I ban an address?

`silvermere sysop ban-ip 203.0.113.7` (or a block, `203.0.113.0/24`), or the console's **Ban
address**. Telnet connections from it are closed before the login screen, and players in the
realm who came from it are told, saved and disconnected. The BBS door is never checked: its
callers all come from the BBS.

### Can I watch what a player is doing?

Yes. Click the player in the console, then **Watch**: their screen as they see it, live. Type
a line under it and the game takes it as theirs; nothing on their screen shows it, and they
are not told they are watched. From a shell: `silvermere sysop spectate NAME` (add `--type` to
type as them). Watching and every line typed are audited. See
[web.md](web.md#the-console).

### Can I use the SYSOP commands without logging in to the game?

Yes, most of them: `silvermere sysop game list users`, `game god Lars info`, `game speak
"back in 5"`, or the same after `game` on the console's command line. The ones that need your
character in the realm (`GOTO`, `SUMMON`, `MAP`, `RESET` and the paged reports) are refused
there. See [sysop.md](sysop.md#sub-commands).

### How do I close the game for maintenance?

**Close realm** in the console, or `silvermere sysop realm close "reason"`. Everyone but the
sysops is told, saved and disconnected, and only sysops can log in until **Open realm**
(`silvermere sysop realm open`). It stays closed across restarts.

### Can I restart the server from the console?

Yes: **Restart** (or `silvermere sysop restart --in 5m "reason"`) warns the players, saves
everyone, takes a backup and starts the server again in the same process. systemd and Docker
see it keep running, and the console reconnects by itself, still signed in. It is refused if
the settings would not load or the program file does not run. See
[POST /v1/restart](admin-api.md#post-v1restart).

### Can I open the console to the internet?

Not directly: it is plain HTTP. Use an SSH tunnel, a private network such as Tailscale, or a
reverse proxy with TLS, and add the proxy's name to `server.web_hosts`. See
[Reaching it from another computer](web.md#reaching-it-from-another-computer). The admin API
stays on this computer unless you set `admin.allow_remote`.

### How do I test my server with many players?

`tools/demo/run.sh stress` imports the stock world, makes 64 characters of every class and
level and lets them play, then reports how the server kept up. See
[The demo and stress run](testing.md#the-demo-and-stress-run).

## Hosting

### Which ports does it use?

| Port | Setting | What |
|---|---|---|
| 2323 | `server.telnet_bind` | the game, over telnet; open it to your players |
| 8080 | `server.admin_bind` | the admin API; loopback only by default |
| 8088 | `server.web_bind` | these pages; loopback only by default |
| none | `server.rlogin_bind` | the BBS door, off by default |

See [Firewall](install.md#firewall).

### Can a BBS run it as a door?

Yes, over rlogin: the BBS's callers go straight into the game with no second login. See
[rlogin (a BBS door)](auth.md#rlogin-a-bbs-door).

### Can people play in a browser?

Not yet. Players connect with a telnet client.

### Is it backed up?

Yes: every day at 04:30, keeping the last 30. `silvermere backup --list` shows them. See
[Backup and restore](install.md#backup-and-restore).

### How do I upgrade?

Replace the program and start it; the new version migrates the database on its first start.
See [Upgrading](install.md#upgrading). On Linux and macOS you can replace the program file and
then **Restart** from the console: the new version starts in the same process.

### How do I bring over an existing game?

`silvermere import --from FOLDER` reads the original's files; `--live` takes a running realm's
players too. See [import.md](import.md).

## The game

### How does it differ from the original?

See [differences.md](differences.md).

### How do I change the world?

silvermere has no world editor. Edit the original's files with an editor that writes them and
import them again; `silvermere sysop reload-world` then loads the new world into the running
server. See [Content and state](import.md#content-and-state).
