# Metrics

`GET /metrics` on the admin port serves these in the Prometheus text format. Every name
starts with `silvermere_`. No label takes an unbounded value: there are no character names
and no raw command text in any label.

| Metric | Type | Labels | How to read it |
|---|---|---|---|
| `silvermere_sessions` | gauge | none | Connections the engine holds a slot for, logged in or not; it falls to 0 after a clean shutdown. |
| `silvermere_commands_total` | counter | `family`: `movement`, `combat`, `info`, `comms`, `shop`, `sysop`, `other` | Lines typed in the realm, by the family of the first word; a family that never moves while players are online points at a stuck dispatcher. |
| `silvermere_tick_seconds` | histogram | none | Wall time of one fast tick (the engine's one-second cycle); a p99 near the tick period means the world thread is saturated. |
| `silvermere_persist_queue_depth` | gauge | none | Writes waiting for the persistence worker; it should return to 0 between batches. Thousands right after a start are the start-up pass (every monster it generates is a write) and should drain within a minute; a value that stays high or only grows means the database is slower than the game or unreachable, and the server logs `persistence queue backlog` at WARN with the depth and drain rate once it has stayed above 1,000 for 30 s. |
| `silvermere_db_latency_seconds` | histogram | `op`: `load`, `save_character`, `save_state`, `event_insert` | Time the database took for the writes of one kind inside a batch, which go as a few multi-row statements; `load` is reserved for reads. |
| `silvermere_logins_total` | counter | none | Characters that entered the realm. |
| `silvermere_deaths_total` | counter | none | Player deaths. Declared; T-033 (combat orchestration) raises it where a death is applied. |
| `silvermere_levelups_total` | counter | none | Player level gains. Declared; T-039 (experience and levels) raises it. |
| `silvermere_handler_panics_total` | counter | none | Event handlers that panicked and were contained; any increase is a bug to report with the log line that names the input. |
| `silvermere_output_dropped_total` | counter | none | Writes shed because a session's bounded output queue was full; a slow client shows up here instead of stalling the game. |
| `silvermere_persist_errors_total` | counter | none | Persistence batches that failed on connectivity and were retried; no write is dropped for it. |
| `silvermere_build_info` | gauge | `version`, `backend` (`SQLite` or `PostgreSQL`) | Always 1; join on it to see which build and database a series came from. |

The names of the tick and database-latency metrics are `silvermere_tick_seconds` and
`silvermere_db_latency_seconds`; they are fixed because T-061's soak reads them (the card
text says `tick_duration_seconds` and `db_query_seconds`). `commands_total` families follow
the module of the command's handler in `commands::dispatch`; banks count as `shop`.
Every family is exported from start-up at 0, so a scrape never lacks a series because
no command of that family has run yet.

## Shutdown and backups

`SIGTERM` or Ctrl-C stops `silvermere serve` cleanly: new connections are refused, players
are told, every character is saved, the persistence queue is flushed and SQLite is
checkpointed (no `-wal` file is left). A second signal during shutdown exits at once with
code 130. Exit code 1 means the database could not be reached in time and characters may
not be saved.

`silvermere backup [--to PATH] [--keep N]` writes a consistent copy while the server runs
(default `<data dir>/backups/silvermere-YYYYMMDD-HHMMSS.db`, UTC); the server also runs it daily
at `backup.at` ([config.md](config.md#backup)). `silvermere restore FILE` needs the server stopped.
