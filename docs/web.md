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
- Settings are read once, at startup: restart the server to apply a change. Until then the page
  says the file has changes the running server has not loaded.
- A setting that an environment variable or a flag sets cannot be changed here (they win over
  the file), and the database URL and the admin tokens are never shown or written. The page
  says which is which.

A session lasts 12 hours and ends when the server stops.

## Reaching it from another computer

The default bind is loopback, so only the server's own computer reaches the pages. Two ways to
open them from elsewhere:

- **An SSH tunnel**, with nothing to change: `ssh -L 8088:127.0.0.1:8088 you@server`, then
  open `http://127.0.0.1:8088` on your own computer.
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
- The settings page takes its descriptions from the key tables of [config.md](config.md) and
  its values from the same code as `silvermere config show`, so a new setting appears once it
  is in the config and in that table (`docs_in_sync` fails until it is).
- `/play` and `/ws` are kept for playing in the browser, which is not built.
