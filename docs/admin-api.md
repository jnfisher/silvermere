# Admin API (`silvermere-api`)

## Overview and auth

HTTP and JSON on the admin port (`server.admin_bind`, `127.0.0.1:8080` by default; this is
never widened by default), built with axum. It is the seam external tools attach to:
monitoring, an operator CLI, the future editor and web client. The engine is never read from the API task: each call that needs game data
sends an `AdminRequest` to the engine thread and waits for the reply (5 s, then `504`).

`/healthz` and `/metrics` are open (the admin bind is loopback by default). Every `/v1/*`
route needs `Authorization: Bearer <token>`:

- a missing or unknown token is `401 {"error":"unauthorized","message":"..."}` with a
  `WWW-Authenticate: Bearer` header;
- a valid token without the scope the route needs is `403 {"error":"forbidden",...}`.

Every error body has the shape `{"error": "<code>", "message": "<human text>"}`; the codes
are `unauthorized`, `forbidden`, `bad_text`, `engine_unavailable` (503), `engine_timeout`
(504), `reload_failed` (500), `bad_id` (400), `no_such_account` (404), `delete_failed` (500),
`bad_delay` (400), `bad_body` (400), `shutdown_unavailable` (503), `restart_refused` (503,
with the reason as the `message`), `backup_unavailable` (503), `reserved` (501), and for
the [moderation routes](#moderation) `no_such_player` (404), `not_connected` (409),
`door_account` (409), `bad_network` (400), `no_such_ban` (404), `not_sanctioned` (404),
`bad_duration` (400), `store_failed` (500) and `moderation_unavailable` (503).

| Scope | Allows |
|---|---|
| `read` | the `GET` routes: `/v1/who` (hidden sysops omitted), `/v1/events`, `/v1/backup`, `/v1/moderation` |
| `write` | routes that change the running game: `/v1/broadcast`, `/v1/reload-world`, `/v1/players/{name}/tell` |
| `admin` | everything, including `DELETE /v1/accounts/{id}`, `/v1/shutdown`, the moderation routes (kick, ban, jail, mute, IP bans, closing the realm) and the reserved world routes; `/v1/who` includes hidden sysops |

`write` does not include `read`: give an operator tool both. (The T-018 names `broadcast`
and `sysop` are still accepted in the config and mean `write` and `admin`.)

Binding anything but a loopback address refuses to start unless `admin.allow_remote = true`
and at least one token is configured (`silvermere_api::check_bind`, [below](#remote)).

## Remote

The admin API is for the computer the server runs on: the CLI, scripts and monitoring there.
It is plain HTTP, its tokens are long-lived, and an `admin` token can delete accounts and shut
the server down, so it is not made to face the internet.

`server.admin_bind` on an address other computers reach (`0.0.0.0:8080`, a LAN address) is
refused at start:

```
server.admin_bind is 0.0.0.0:8080, which other computers can reach. The admin API is meant for
this computer: anyone who can reach it can try tokens, and an admin token can delete accounts
and shut the server down. To allow it anyway, set admin.allow_remote = true. See
docs/admin-api.md#remote
```

With `admin.allow_remote = true` (`SILVERMERE_ADMIN__ALLOW_REMOTE=true`) it starts if a token is
configured, and logs this warning at every start:

```
The admin API is reachable from other computers (0.0.0.0:8080, admin.allow_remote = true). It
is plain HTTP: tokens and everything sent to it can be read on the network. Keep it behind an
SSH tunnel, a private network such as Tailscale, or a reverse proxy with TLS. In Docker this is
expected: it is safe while the port is published on 127.0.0.1 only.
```

To reach it from another computer without opening it: `ssh -L 8080:127.0.0.1:8080
you@server`, then use `http://127.0.0.1:8080` there. The Docker image sets
`admin.allow_remote`, since inside the container the API must listen on `0.0.0.0`; the compose
file publishes the port on the host's `127.0.0.1` only.

## Built routes

<!-- built-routes:start -->
| Verb | Path | Scope | Request | Response |
|---|---|---|---|---|
| `GET` | `/healthz` | `none` | none | `200 {"status":"ok",...}` |
| `GET` | `/metrics` | `none` | none | Prometheus text |
| `GET` | `/v1/who` | `read` | none | `{"players":[...]}` |
| `GET` | `/v1/events` | `read` | `?kinds=a,b`, `Last-Event-ID` | SSE stream |
| `GET` | `/v1/backup` | `read` | none | `{"enabled":true,"next":"...","last":{...},"backups":[...]}` |
| `POST` | `/v1/backup` | `admin` | none | `{"at":"...","ok":true,"file":"...","bytes":n,...}` |
| `POST` | `/v1/broadcast` | `write` | `{"text":"..."}` | `{"delivered":n}` |
| `POST` | `/v1/reload-world` | `write` | none | `{"reloaded":{...},"ms":t}` |
| `DELETE` | `/v1/accounts/{id}` | `admin` | none | `{"deleted":{"account":id,"character":"..."}}` |
| `GET` | `/v1/shutdown` | `read` | none | `{"planned":null}` or `{"planned":{"at":"...","in_secs":n,"restart":false}}` |
| `POST` | `/v1/shutdown` | `admin` | `{"in_secs":n,"message":"..."}` (both optional) | `{"at":"...","in_secs":n,"restart":false}` |
| `DELETE` | `/v1/shutdown` | `admin` | none | `{"cancelled":true}` |
| `POST` | `/v1/restart` | `admin` | `{"in_secs":n,"message":"..."}` (both optional) | `{"at":"...","in_secs":n,"restart":true}` |
| `DELETE` | `/v1/restart` | `admin` | none | `{"cancelled":true}` |
| `GET` | `/v1/moderation` | `read` | none | `{"realm":{...},"sanctions":[...],"ip_bans":[...]}` |
| `POST` | `/v1/players/{name}/kick` | `admin` | `{"reason":"..."}` (optional) | `{"kicked":"Lars"}` |
| `POST` | `/v1/players/{name}/tell` | `write` | `{"text":"..."}` | `{"told":"Lars"}` |
| `PUT` | `/v1/players/{name}/{kind}` | `admin` | `{"for_secs":n,"reason":"..."}` (both optional) | `{"player":"Lars","kind":"jail","until":"...","online":true,...}` |
| `DELETE` | `/v1/players/{name}/{kind}` | `admin` | none | `{"player":"Lars","kind":"jail","lifted":true,...}` |
| `POST` | `/v1/ip-bans` | `admin` | `{"network":"203.0.113.0/24","for_secs":n,"reason":"..."}` | `{"id":1,"network":"203.0.113.0/24","until":null}` |
| `DELETE` | `/v1/ip-bans` | `admin` | `{"network":"..."}` | `{"network":"...","lifted":true}` |
| `POST` | `/v1/realm/close` | `admin` | `{"reason":"..."}` (optional) | `{"closed":true,"disconnected":n}` |
| `POST` | `/v1/realm/open` | `admin` | none | `{"closed":false,"was_closed":true}` |
<!-- built-routes:end -->

Examples (`$T` is a token):

```sh
curl -s localhost:8080/healthz
curl -s -H "Authorization: Bearer $T" localhost:8080/v1/who
curl -sN -H "Authorization: Bearer $T" 'localhost:8080/v1/events?kinds=connected,closed'
curl -s -X POST -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
     -d '{"text":"Reboot in 5 minutes"}' localhost:8080/v1/broadcast
curl -s -X POST -H "Authorization: Bearer $T" localhost:8080/v1/reload-world
curl -s -X DELETE -H "Authorization: Bearer $T" localhost:8080/v1/accounts/42
curl -s -X POST -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
     -d '{"in_secs":300,"message":"upgrading to 0.2"}' localhost:8080/v1/shutdown
curl -s -X DELETE -H "Authorization: Bearer $T" localhost:8080/v1/shutdown
```

### `GET /healthz`

`200 {"status":"ok","engine_tick":n,"persist":"healthy","engine":"running","backup":"ok"}`.
`503` with `"status":"unavailable"` when the engine thread is dead (`"engine":"dead"`) or the
persistence worker is unhealthy (`"persist":"unhealthy"`). The Docker healthcheck hits it.
`"backup"` is the last scheduled backup: `"ok"`, `"failed"` or `"none"` (none yet); a failed
backup does not make the server unhealthy. [`GET /v1/backup`](#get-v1backup) has the details.

### `GET /v1/backup`

The scheduled backup (T-150), as `silvermere backup --list` shows it:

```json
{
  "enabled": true, "at": "04:30", "tz": "America/Detroit", "keep": 30, "after": true,
  "next": "2026-10-28T08:30:00Z",
  "last": {
    "at": "2026-10-27T08:30:00Z", "ok": true,
    "file": "./data/backups/silvermere-20261027-083000.db", "bytes": 1843200,
    "pruned": ["./data/backups/silvermere-20260927-083000.db"],
    "hook": {"ok": false, "exit": 1, "timed_out": false, "tail": ["...403 Forbidden"]}
  },
  "backups": [{"name": "silvermere-20261027-083000.db", "bytes": 1843200, "assets": 412}]
}
```

`after` says whether `backup.after` is set (the command itself is not shown). `next` is `null`
when `backup.enabled` is false. `last` is `null` before the first scheduled run; a failed run
has `"ok": false` and an `error`, and no `file`. `hook` is absent when there is no command;
`tail` is the last lines of its output. `backups` lists `data/backups/`, newest first.

### `GET /metrics`

Prometheus text (`text/plain; version=0.0.4`). The names are fixed (T-061's soak reads them):

| Metric | Type | Emitted by |
|---|---|---|
| `silvermere_sessions` | gauge | engine loop, after every input |
| `silvermere_tick_seconds` | histogram | engine loop, per fast tick |
| `silvermere_commands_total` | counter | command dispatch (T-016) |
| `silvermere_persist_queue_depth` | gauge | persistence worker (T-023) |
| `silvermere_db_latency_seconds` | histogram | persistence worker (T-023) |

### `GET /v1/who`

The players the in-game user list shows: only those in the realm, in the same order (first
names, case-blind alphabetical).

```json
{"players":[{"session_id":1,"first_name":"Abe","last_name":"Smith","title":"Apprentice",
  "class":"Warrior","race":"Human","level":3,"alignment":-5,"gang":"","idle_secs":7,
  "hidden":false,"ip":"203.0.113.7","account":12,"muted":false,"jailed":false,
  "room":"1:2140","room_name":"Town Square"}]}
```

`alignment` is the player's evil points; `gang` is empty for none;
`idle_secs` is seconds since the last input; `ip` is the address the connection comes from
(an rlogin door player's is the BBS's); `account` is the account id; `muted` and `jailed` say
whether a sysop's mute or jail is in force ([below](#moderation)); `room` is where the
character stands (`map:room`) and `room_name` that room's name. A sysop who made himself invisible is listed only
for an `admin` token (the in-game list hides him from players).

### `POST /v1/broadcast`

`{"text":"Reboot in 5 minutes"}` (1 to 1000 characters) answers `{"delivered":n}`, `n` being
the players told. It is the in-game `SYSOP SPEAK`: the line is sent to every
player in the game, preceded by `ANSI_CLRLINE`, with no colour prefix, followed by each
player's prompt. A text with a control character (C0 such as ESC, CR, LF or tab, DEL, or C1
such as U+009B) is rejected with `400 bad_text`, so the API sends only what the line editor
could type. The in-game command leaves the speaker out; the API has none.

### `POST /v1/reload-world`

Re-reads the static world tables (rooms, items, known monsters, spells, classes, races,
shops, messages, text blocks, actions) from the database into the running engine. The engine
swaps them in one step between two events, so a command never sees a half-loaded world.
Players are never disconnected, and live state (characters, inventories, room state, active
monsters, gangs, bank books) is not touched. Answers

```json
{"reloaded":{"rooms":131,"items":600,"monsters":300,"spells":200,"classes":12,"races":8,
  "shops":40,"messages":900,"text_blocks":500,"actions":60},"ms":42}
```

(counts are the rows read.) If the database read or the engine's checks fail (a special room
is missing from the new tables), nothing is replaced and the answer is
`500 {"error":"reload_failed","message":"..."}`; the cause is in the server log.

### Moderation

A sysop's tools for the players (silvermere's own; the original had none of them).
`silvermere sysop kick|tell|mute|unmute|jail|release|ban|unban|ban-ip|unban-ip|moderation|realm`
call these routes ([cli.md](cli.md#silvermere-sysop)).

`{name}` is a character's first name or an account's username, ignoring case (the account's
username is tried first). Each sanction is stored before the running game is told, so it
holds for a player who is not connected from his next login, and across restarts.
`for_secs` is how long from now (1 to 315360000, ten years); absent or `null` is until lifted.
A reason or text is at most 200 characters with no control character (`400 bad_text`).
Each call is audited (`admin.players.<name>.<action>`, `admin.ip-bans`, `admin.realm.close`)
with what it set, and stored rows name the token in `by_whom`.

| Route | What it does |
|---|---|
| `POST /v1/players/{name}/kick` | The player is told `You have been disconnected by the sysop.`, with the reason, saved as a logoff saves (no hang-up penalty) and disconnected. `409 not_connected` when he is not connected. |
| `POST /v1/players/{name}/tell` | `The sysop tells you: <text>` to the player, in the game or at the menus. `409 not_connected` when he is not connected. |
| `PUT /v1/players/{name}/mute` | No gossip or auction (with text) until it ends: `You are muted until ...` |
| `PUT /v1/players/{name}/jail` | The player is moved to the jail room (`game.jail_room`) and held there: walking, following, being dragged, recall, teleports and summons out are refused. When the jail ends (or is lifted) he is told and moved to the reincarnation room, the town `SYSOP GOTO SILVERMERE` goes to. A jail that ended while he was away lets him out at his next entry to the realm. |
| `PUT /v1/players/{name}/ban` | His logins are refused after the right password with `This account is banned until ...` and the reason, and the connection closes; a player in the game is disconnected, saved. A BBS door account (`door` flag) cannot be banned: `409 door_account`. |
| `DELETE /v1/players/{name}/{kind}` | Lifts the mute, jail or ban. `404 not_sanctioned` when there was none. |
| `POST /v1/ip-bans` | Telnet connections from the address or CIDR block are closed before the login screen; the rlogin door is never checked. Players already in the game stay. `network` is stored in its canonical form (`203.0.113.77/24` is `203.0.113.0/24`); banning it again replaces the ban. `400 bad_network` for anything else. |
| `DELETE /v1/ip-bans` | Lifts the ban of `network`. `404 no_such_ban` when there is none. |
| `POST /v1/realm/close` | Only sysops may log in (the rlogin door included, and no door signups); every player but the sysops is told `The realm is closed`, with the reason, saved and disconnected. It stays closed across restarts. |
| `POST /v1/realm/open` | Everyone may log in again. |
| `GET /v1/moderation` | The realm's state, the sanctions in force and the IP bans in force. |

```json
{"realm":{"closed":true,"reason":"upgrading to 1.0","by":"ops","at":"2026-11-20T18:00:00Z"},
 "sanctions":[{"account":12,"username":"lars","character":"Lars","kind":"jail",
   "until":"2026-11-21T18:00:00Z","reason":"player killing in town","by":"ops",
   "at":"2026-11-20T18:00:00Z"}],
 "ip_bans":[{"id":1,"network":"203.0.113.0/24","until":null,"reason":"bots","by":"ops",
   "at":"2026-11-20T18:05:00Z"}]}
```

A `PUT` answers `{"player":"Lars","account":12,"kind":"jail","until":"...","online":true}`:
`until` is `null` for until lifted, and `online` says whether the running game was told (the
player was connected). The rows are `account_sanctions`, `ip_bans` and `realm_closed`
([schema.md](schema.md)).

### `DELETE /v1/accounts/{id}`

Deletes an account and its character through the running engine (T-069), so that nothing the
engine holds for it (the gang's member count, bank books, evil points, limited-item
ownership) is written back after the rows are gone. `{id}` is the account id (`silvermere
account list` prints it). In order:

1. A character in the game on that account is logged off by the normal path (the connection
   is closed and the character saved).
2. The store writes everything queued so far and reads the account and its stored character.
3. The engine runs `session::on_account_deleted` (the module's `dlarou` hook):
   every carried item is handed back (a limited copy is released), the gang counts one member
   fewer (flagged disbanded if he led it), and the engine's bank books and evil points for the
   account are dropped.
4. The engine queues the character row's and then the account row's deletion behind its own
   writes; the account's roles, bank books, evil points and profile go with it (`ON DELETE
   CASCADE`). The store audits it as `auth.account_deleted`, as the CLI does.

The answer comes once step 4 is queued:

```json
{"deleted":{"account":42,"character":"Lars"}}
```

(`character` is `null` for an account with no character.) An unknown account is
`404 {"error":"no_such_account",...}`; an id that is not a positive integer is
`400 {"error":"bad_id",...}`; a store read that fails is `500 {"error":"delete_failed",...}`
and nothing is deleted. The call is audited as `admin.accounts.<id>`, with the `character`
in the payload.

`silvermere account delete <name> --yes` uses this route when a server is running. It probes
`/healthz` on `server.admin_bind` (`0.0.0.0` and `::` are reached on loopback), and when that
does not answer `200` it checks the store's single-writer guard (T-072, below):

| Probe | Guard | What the CLI does |
|---|---|---|
| `200` | (not checked) | looks the account up and calls this route with an `admin` token from the config (a plain `token`, e.g. `SILVERMERE_ADMIN_TOKEN`); refuses if it has none |
| anything else (refused, a timeout, a non-`200`, a bad answer) | free | no server holds the store: takes the guard and deletes the rows directly |
| anything else | held | refuses: a server is running on the store but its admin API cannot be used, and deleting behind its back could be undone by its write-behind |

A `504 {"error":"engine_timeout",...}` from the route means the engine did not answer in
time, not that it refused: the CLI says the deletion was queued and may still complete, and
exits 1. Check with `silvermere account list` before trying again.

### `POST /v1/shutdown`

Plans a shutdown: the server warns the players in the game, then stops as it does on
SIGTERM (every character saved, `The server is shutting down. Your character has been
saved.` to everyone, exit 0). `silvermere sysop shutdown` calls it.

`{"in_secs": 300, "message": "upgrading to 0.2"}`: `in_secs` is 0 to 86400 (a day; `400
bad_delay` beyond), 300 when absent; `message` is optional, up to 200 characters with no
control character (`400 bad_text`). A body that is not that JSON is `400 bad_body`; an empty
body takes both defaults. The answer is `{"at":"2026-11-18T20:05:00Z","in_secs":300}`, `at`
in UTC.

The warnings are broadcasts, as [`POST /v1/broadcast`](#post-v1broadcast) sends them: one
when the plan is made, then at 1 hour, 30, 15, 10, 5 and 2 minutes, 1 minute, 30 and 10
seconds before the stop, those still ahead. Each says
`The server will shut down in 5 minutes: upgrading to 0.2` (with no message, `The server
will shut down in 5 minutes.`). `in_secs: 0` stops at once with no warning.

A second call replaces the plan, its warnings counted from the new time. The plan lives in
the running server only: a restart forgets it.

What happens after the exit is up to what runs the server. Under the systemd service
(`Restart=on-failure`) the server stays stopped until `systemctl start silvermere`; under
Docker's `restart: unless-stopped` (the compose file) it starts again at once. So for an
upgrade with the service: plan the shutdown, replace the program once it has stopped, start
it.

Audited as `admin.shutdown` with `in_secs`, `at` and `text`. A server that cannot stop
itself (one run in-process by the tests) answers `503 shutdown_unavailable`.

### `POST /v1/restart`

Plans a restart: the same countdown and warnings as [`POST /v1/shutdown`](#post-v1shutdown)
(`The server will restart in 5 minutes: ...`), then the server stops as it does on SIGTERM,
every character saved, takes a backup (as a scheduled one, when `backup.enabled`), and starts
again **in the same process**:

- on Linux and macOS the process replaces itself with its program file, same arguments and
  environment. The process id does not change, so systemd keeps the service `active
  (running)` and Docker's main process never exits: neither restarts anything. A program file
  replaced before the restart is the one that starts, so replacing the file and restarting is
  an upgrade.
- on Windows the server starts again inside the process, with the settings read anew. A
  running `.exe` cannot be replaced there, so an upgrade is still stop, replace, start.

The settings change that needs a restart ([web.md](web.md#applying-a-change)) takes effect.
Before the countdown is accepted, and again when it comes due, the server checks that the
settings file loads and that the program file runs (`--version`); if not, the restart is
refused with `503 {"error":"restart_refused","message":"<why>"}` (or, when due, called off:
the players are told `The planned restart is cancelled.` and the server keeps running).
A restart replaces a planned shutdown and the other way round; `DELETE /v1/restart` (or
`DELETE /v1/shutdown`) cancels whichever is planned. Audited as `admin.restart`.

`GET /v1/shutdown` says what is planned: `{"planned": null}`, or the plan with `in_secs`
counted from now and `restart`.

### `POST /v1/backup`

Takes a backup now, as a scheduled one is taken (into the backups folder, pruned to
`backup.keep`, the after-backup command run), and answers the run as `last` in
[`GET /v1/backup`](#get-v1backup) shows it. A failed backup is still a `200`, with `"ok":
false` and the `error`. Audited as `admin.backup`.

### `DELETE /v1/shutdown`

Cancels the plan: `{"cancelled": true}`, and the players are told `The planned shutdown is
cancelled.`; `{"cancelled": false}` when none was planned. Audited as `admin.shutdown` with
`cancelled`.

### Single-writer guard

`serve` takes a store-level guard at start, before the migrations, and holds it until it
exits (T-072, `silvermere_db::WriterGuard`):

- SQLite: an exclusive lock on `<database>.lock` beside the database file (for example
  `data/silvermere.db.lock`). The file holds the holder's pid and stays after it exits; the
  lock goes with the process.
- PostgreSQL: the session advisory lock `pg_try_advisory_lock(1397577554,
  hashtext(current_schema()))` on a connection of its own, so the store is the database plus
  the schema in `search_path`. It goes when that connection closes.
- An in-memory SQLite database has no guard: no other process can open it.

A second `serve` on the same store exits 1 with `another silvermere is already serving ...`
and the holder (the pid, or the PostgreSQL backend pid). `account delete` deletes directly only
when it can take the guard itself, as above.

## SSE and event kinds

`GET /v1/events` sends one frame per event: `id:` the `events` row id, `event:` the kind,
`data:` JSON

```json
{"id":17,"at":"2026-10-14T20:15:00Z","kind":"connected","session":3,"character":null,
 "room":null,"payload":{}}
```

(`session`, `character` and `room` are `null` when not applicable.) A `: keepalive` comment
is sent every 15 seconds. The stream is the live tail of the `events` table: every event is
also a row there, with the same id and payload.

- `?plain=1` sends the frames without the `event:` name, so a browser's `EventSource` hears
  every kind with one handler (the kind is in the data); the web console uses it.
- `?kinds=login,death` sends only those kinds. A word without a dot also covers its
  family: `auth` sends `auth.login_failed`, `auth.lockout` and the rest; `auth.lockout`
  sends only that one.
- `Last-Event-ID: n` (EventSource sends it on reconnect) first replays every stored event
  with a larger id, then continues live. The filter applies to the replay too.
- The stream reads the `events` table in id order (a new row is sent within a second, at
  once for rows the API itself writes), so a slow client cannot lose an event and rows
  written by any part of the server are streamed. A stream without `Last-Event-ID` starts
  after the newest stored row.

Event kinds (generated from `EventKind::ALL`; a test fails when this table is stale). A kind
marked "not yet" is a reserved name: its card has not landed, the table never holds it, and
a client may already subscribe to it.

<!-- event-kinds:start -->
| Kind | Name in `events.kind` | Emitted | From |
|---|---|---|---|
| `connected` | `connected` | yes | engine: a session opened or closed |
| `closed` | `closed` | yes | engine: a session opened or closed |
| `login` | `login` | yes | engine: a character entered the realm |
| `logoff` | `logoff` | yes | engine: a character left the realm |
| `death` | `death` | yes | engine: a character died |
| `level` | `level` | yes | engine: a character trained a level (train_level) |
| `auth` | `auth.*` | yes | silvermere-auth: login failures, lockouts, account changes |
| `admin` | `admin.*` | yes | this API: the audit row of each authenticated call |
<!-- event-kinds:end -->

Engine rows (`connected`, `closed`, `login`, `logoff`, `death`, `level`) carry the
`session`. Their payloads:

| Kind | Payload |
|---|---|
| `login` | `{"name": "Lars", "ip": "203.0.113.7"}`: the character entered the realm, from that address (`null` when unknown) |
| `logoff` | `{"name": "Lars", "how": "exit"}`: the character left the realm; `how` is `exit` (QUIT or X), `hangup` (the connection dropped) or `closed` (the server closed it: a kick, a ban, the realm closing, the shutdown) |
| `death` | `{"name": "Lars"}`, with the `room` where he died |
| `level` | `{"level": N}`, the level the character trained to |

`connected` and `closed` have `{}`. Tokens with the `read` scope see the players' addresses
here and in `/v1/who`.

The web console ([web.md](web.md#the-console)) calls these routes inside the server with a
token made at start and held in memory only, naming the signed-in sysop in the
`X-Silvermere-Sysop` header: its rows name `web:<sysop>` as the token, and its `GET`s (it
refreshes every few seconds) are not audited.

Audit rows (`admin.*`): every authenticated call to a `/v1` route (including a `403` and the
`501`s) is logged as `tracing::info!(target = "audit", route, token_name, status)` and
appended to the `events` table as `kind = "admin.<route>"`, `<route>` being the path after
`/v1/` with `/` as `.` (`admin.who`, `admin.broadcast`, `admin.world.rooms`; an item route
carries the id: `admin.world.rooms.2140`). The payload is `{"token", "method", "status"}`,
plus `text` and `delivered` for a broadcast and `reloaded` and `ms` for a reload. Only
authenticated calls are audited: a `401` writes no row. The token's name is recorded, never
the token or the `Authorization` header.

## Reserved routes

The world-editing routes exist so that a client can be written against the final paths. They
need an `admin` token and answer `501`:

```json
{"error":"reserved","message":"world editing is not built yet","docs":"docs/admin-api.md#reserved-routes"}
```

<!-- reserved-routes:start -->
| Table | Collection | Item | `docs/schema.md` table |
|---|---|---|---|
| `rooms` | `GET POST` | `GET PUT DELETE` | `rooms` |
| `monsters` | `GET POST` | `GET PUT DELETE` | `monsters` |
| `items` | `GET POST` | `GET PUT DELETE` | `items` |
| `shops` | `GET POST` | `GET PUT DELETE` | `shops` |
| `spells` | `GET POST` | `GET PUT DELETE` | `spells` |
| `classes` | `GET POST` | `GET PUT DELETE` | `classes` |
| `races` | `GET POST` | `GET PUT DELETE` | `races` |
| `texts` | `GET POST` | `GET PUT DELETE` | `text_blocks` |
<!-- reserved-routes:end -->

Planned contract (the editor project implements it):

- **Verbs.** `GET /v1/world/<table>` lists, `POST` creates; `GET|PUT|DELETE
  /v1/world/<table>/{id}` read, replace and delete one row. Other methods answer `405`.
  An unknown `<table>` is `404`.
- **Ids.** `{id}` is the record's number (`rooms` use `map:room`, e.g. `1:2140`).
- **Request body.** The row as in the `docs/schema.md` table named above, without the
  server-managed columns (generated ids and timestamps). Child tables of a row (a room's
  exits and placed items, a monster's attacks and abilities) travel inside the row.
- **Content versus state.** These are *content* tables. The state tables (characters,
  accounts, bank books, item ownership, room state) are not part of `/v1/world`; they change
  through the game.
- **Taking effect.** A write changes the database only. It reaches the running game after
  `POST /v1/reload-world`.

## Tokens

Tokens are named and scoped in `silvermere.toml`:

```toml
[[admin.tokens]]
name = "editor"
sha256 = "<64 hex digits>"
scopes = ["read", "write"]      # read | write | admin; default ["read"]
```

Only the SHA-256 is configured and kept in memory; the presented token is hashed and compared
in constant time. A token is never logged, printed in `--print-config` or echoed in a config
error.

- **Issue.** `silvermere admin token new editor --scopes read,write` prints the token once
  (`sm_` and 32 random bytes as base64url) and the `[[admin.tokens]]` block to paste. The
  token is not stored anywhere else.
- **List.** `silvermere admin token list` shows each configured token's name, scopes and the
  first 8 hex digits of its hash (never a token).
- **Revoke.** Delete the block and restart the server (there is no hot reload of the token
  list).
- **Legacy.** `SILVERMERE_ADMIN_TOKEN=<secret>` still works as a token named `default`
  with all three scopes. A start-up `warn` is logged when it is used while `server.admin_bind`
  is not a loopback address. `SILVERMERE_ADMIN_TOKENS` (a JSON array of
  `{"name","sha256"|"token","scopes"}`) and a plain `token = "..."` entry (hashed on load)
  are also read.

## Compatibility promises

Within `/v1`: routes and verbs are only added, never removed or changed; existing response
fields keep their names and meaning (new fields may appear, so clients ignore unknown
fields); error `error` codes keep their meaning; new event kinds may appear on the stream
(clients ignore kinds they do not know) and the `events` row id stays ascending in time;
scopes keep what they allow, a new scope or route never widens an existing token; the
reserved routes keep their paths. A change that breaks any of this is a new prefix (`/v2`).

## Not carried

The in-game `SYSOP` command set (T-055), MBBSEmu's `/SYS` console globals, the local console
and its sysop status display: `shocst` becomes the audit rows and `tracing`, and what an
operator used the console for is these routes.
