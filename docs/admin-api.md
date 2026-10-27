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
(504), `reload_failed` (500), `bad_id` (400), `no_such_account` (404), `delete_failed` (500)
and `reserved` (501).

| Scope | Allows |
|---|---|
| `read` | the `GET` routes: `/v1/who` (hidden sysops omitted), `/v1/events` |
| `write` | routes that change the running game: `/v1/broadcast`, `/v1/reload-world` |
| `admin` | everything, including `DELETE /v1/accounts/{id}` and the reserved world routes; `/v1/who` includes hidden sysops |

`write` does not include `read`: give an operator tool both. (The T-018 names `broadcast`
and `sysop` are still accepted in the config and mean `write` and `admin`.)

Binding anything but a loopback address with no token configured refuses to start
(`silvermere_api::check_bind`).

## Built routes

<!-- built-routes:start -->
| Verb | Path | Scope | Request | Response |
|---|---|---|---|---|
| `GET` | `/healthz` | `none` | none | `200 {"status":"ok",...}` |
| `GET` | `/metrics` | `none` | none | Prometheus text |
| `GET` | `/v1/who` | `read` | none | `{"players":[...]}` |
| `GET` | `/v1/events` | `read` | `?kinds=a,b`, `Last-Event-ID` | SSE stream |
| `GET` | `/v1/backup` | `read` | none | `{"enabled":true,"next":"...","last":{...},"backups":[...]}` |
| `POST` | `/v1/broadcast` | `write` | `{"text":"..."}` | `{"delivered":n}` |
| `POST` | `/v1/reload-world` | `write` | none | `{"reloaded":{...},"ms":t}` |
| `DELETE` | `/v1/accounts/{id}` | `admin` | none | `{"deleted":{"account":id,"character":"..."}}` |
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
  "hidden":false}]}
```

`alignment` is the player's evil points; `gang` is empty for none;
`idle_secs` is seconds since the last input. A sysop who made himself invisible is listed only
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
| `login` | `login` | not yet | engine: reserved, not emitted yet |
| `logoff` | `logoff` | not yet | engine: reserved, not emitted yet |
| `death` | `death` | not yet | engine: reserved, not emitted yet |
| `level` | `level` | yes | engine: a character trained a level (train_level) |
| `auth` | `auth.*` | yes | silvermere-auth: login failures, lockouts, account changes |
| `admin` | `admin.*` | yes | this API: the audit row of each authenticated call |
<!-- event-kinds:end -->

Engine rows (`connected`, `closed`, `level`) carry the `session`. The payload of `level` is
`{"level": N}`, the level the character trained to; the others have `{}`.

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
