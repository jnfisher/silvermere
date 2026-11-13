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

Settings are read once, at startup. Restart the server: `sudo systemctl restart silvermere`
for the service. The settings page says when the file has changes the running server has not
loaded.

### Why can't I change a setting on the settings page?

Either it holds a secret (the database URL, the admin tokens), or an environment variable or a
command-line flag sets it, and those win over the file. The page names which. Remove the
variable or the flag to edit the value in the file. See [Precedence](config.md#precedence).

### How do I sign in to the settings page?

With the name and password of a game account that has the sysop role. The page is on
`http://127.0.0.1:8088` on the server itself; see [web.md](web.md) to reach it from another
computer.

### How do I stop strangers making accounts?

Set `game.open_signup` to `false`; then only `silvermere account create` makes accounts.

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
See [Upgrading](install.md#upgrading).

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
