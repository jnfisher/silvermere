# Web pages (`silvermere-web`)

The server has a small web site of its own: this documentation, the [FAQ](faq.md) and an
editor for the settings. It listens on `server.web_bind`, `127.0.0.1:8088` by default: open
`http://127.0.0.1:8088` in a browser on the computer the server runs on.

| Page | What | Who |
|---|---|---|
| `/` | where to start | anyone who can reach the port |
| `/docs` | the documentation of the version that is running | anyone |
| `/faq` | the FAQ | anyone |
| `/settings` | every setting, with its description, default and where its value comes from | a sysop, signed in |

The pages are part of the program: they are built from `docs/` when the server is compiled, so
they always describe the version that is running, and they need no network.

## Settings

Sign in with the name and password of a game account that has the sysop role
(`silvermere account create NAME --sysop`, or `silvermere account grant NAME sysop`). Signing
in here does not take the account's place in the game. Ten wrong passwords for an account
within 15 minutes lock this sign-in for 15 minutes.

The page lists every setting by section, with the description from [config.md](config.md), and
a filter box. Change values and press **Save** for the section:

- It writes `silvermere.toml`, the file `silvermere config set` writes, with the same checks as
  startup. A value the server would refuse is shown with startup's message, and nothing is
  written.
- A value put back to its default is taken out of the file.
- A saved change does not reach the running server by itself. The page lists what is waiting
  and marks each field: *not loaded yet* or *needs a restart* ([below](#applying-a-change)).
- A setting that an environment variable or a flag sets cannot be changed here (they win over
  the file), and the database URL and the admin tokens are never shown or written. The page
  says which is which.

A session lasts 12 hours and ends when the server stops.

### Applying a change

Most of the game options (`[game]`) can go into the running game at once: press **Apply
now** in the banner. It does what the in-game `SYSOP RELOAD` does: the game options are read
from the file again (with the environment variables and the server's flags still on top)
and the running game uses them from its next command. Nobody is disconnected. A value a
sysop changed in the game since (with a `SYSOP` command) goes back to the file's.

Everything else is read only when the server starts: restart it to apply those. That is
every key outside `[game]`, and these game options, which set up the game's timers and
caches at startup:

`game.inactive_secs`, `game.rng_seed`, `game.characters_per_account`, `game.open_signup`,
`game.addons`, `game.sec_create`, `game.sec_oth`, `game.sec_slow`, `game.sec_med`,
`game.sec_fast`, `game.sec_energy`, `game.cleanup.at`, `game.cleanup.tz` and
`game.buffers.monsbuf`.

**Apply now** leaves them as they were. Each loaded change is logged on the `audit` target
with the sysop's name.

## Reaching it from another computer

The default bind is loopback, so only the server's own computer reaches the pages. Three ways to
open them from elsewhere:

- **An SSH tunnel**, with nothing to change: `ssh -L 8088:127.0.0.1:8088 you@server`, then
  open `http://127.0.0.1:8088` on your own computer.
- **A private network such as Tailscale or WireGuard**: bind the pages to the server's address
  on that network (`silvermere config set server.web_bind 100.x.y.z:8088`, the address
  `tailscale ip -4` prints) and restart. The network encrypts the traffic, and only its members
  reach the port.
- **A wider bind** (`server.web_bind = "0.0.0.0:8088"`). The pages are plain HTTP, so the sysop
  password would cross the network unprotected: put a reverse proxy with TLS in front, and
  open the port only to it. The server logs a warning when the bind is not loopback.

In Docker the server is inside the container, so the compose file binds
`0.0.0.0:8088` there and publishes it on the host's loopback.

`server.web_bind = ""` turns the pages off. If the address is taken when the server starts, it
logs a warning and runs without them.

## The public copy

`silvermere web export DIR` writes the open pages as static files, for a public copy of the
documentation: there the settings page is a reference of every key and its default, and nobody
signs in. `--base /path` is the path the copy is served under. The repository publishes one
from `main` on every push (`.github/workflows/pages.yml`).

## For contributors

- A Markdown file added to `docs/` becomes a page with no other change; to place it in the
  sidebar, add its name to `GROUPS` in `crates/silvermere-web/src/site.rs`.
- A new `[game]` key that the engine reads only at startup goes in `GAME_KEYS_AT_STARTUP`
  and `keep_startup_values` in `bin/silvermere/src/web.rs`, and in the list above.
- The settings page takes its descriptions from the key tables of [config.md](config.md) and
  its values from the same code as `silvermere config show`, so a new setting appears once it
  is in the config and in that table (`docs_in_sync` fails until it is).
- `/play` and `/ws` are kept for playing in the browser, which is not built.
