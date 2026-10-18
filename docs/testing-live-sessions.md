# Testing the server

How the test layers are run today. Layers 1 and 2 (unit
and per-crate tests) run with `cargo test --workspace`. This page covers the live-server layers.

## Layer 3: live sessions (`bin/silvermere/tests`)

Each file starts the real server in the test process (`silvermere::serve_in_process`) on the
imported stock world, connects `TelnetClient`s and drives them.

| File | What it pins |
|---|---|
| `m1_walk.rs` | M1: entry, rooms, exits, status screens against the T-005 captures |
| `m3_shops.rs` | the `shop` capture replayed byte for byte; Spell Shop `list`; refusals; Healer; stock rows |
| `m3_bank.rs` | `deposit`, `bankbook`, `withdraw`, the rate card; the book and the purse after a restart |
| `m3_party.rs` | two sessions: `invite`, `follow`, `party`, the march, rank change, level-exit drop, `uninvite` |
| `m3_comms.rs` | telepath allowance and `set maxpages`, say / yell / gossip, gang lifecycle, 100 telepaths |
| `m3_social_training.rs` | social actions with and without a target; the `train stats` refusal |

`cargo test -p silvermere --test m3_shops` runs one file. The database follows
`SILVERMERE_DATABASE_URL` for the workspace run in CI (`sqlite://` and a PostgreSQL 16 service);
the live tests build their own SQLite copy of the stock world under `target/tmp/`.

Helpers: `tests/common/stock.rs` (stock world, `start`, `create_aldric`), `tests/common/m3.rs`
(a template with Aldric, Brynja and Isolde created once per test binary; `Player` with `cmd`,
`wait_for`, `leave`; `place`, `edit_char`, `edit_live`; `classify` and `assert_classified` against
the pattern registry; `stored*` readers).

Two rules the M3 tests keep:

- **No fixed sleeps.** A command waits for its echo and the prompt behind it, and an effect on
  another session is waited for by the text it prints (`wait_for`). A command of the observing
  player is a barrier for everything queued for it before.
- **Cycles by hand where a count depends on one.** `start_manual` sets `SECFAST` to a day, so the
  real-time ticker never fires; `tick(&r, n)` sends `n` fast ticks. Three ticks are one medium
  cycle (`SECMED`), which is what resets the telepath allowance. Queued movement runs in the fast
  cycle, so use `Player::walk_ticking`.

## Layer 4: the differential harness

`tools/differential/README.md`. The M3 group is `tools/differential/scenarios/m3/`:

```
python3 -I tools/differential/differential.py run --scenarios m3 \
    --orig 127.0.0.1:2323 --new 127.0.0.1:2325 --report report
```

`shops`, `bank` (Isolde), `party` and `comms` (Aldric and Brynja on two connections: steps carry
`"as": "B"`, and `{"op": "sleep", "since": "last", "until": ...}` waits for a message another
connection's command caused). The older single-connection scenarios `shop`, `gang`,
`social-actions`, `train-stats-refusals` and `groups-solo` already cover the shop capture, the gang
refusals, the action verbs, the `train stats` refusals and the lone `par` screen.

## Layer 5: the party soak

Not part of CI. The bots are vendored in `tools/soak/vendor/play` (`SOURCE.md`); they read the
world from `$MMUD_DATA`, the directory of the `WCC*.db` files MBBSEmu makes from the
distribution. `tools/soak/soak.sh` runs in segments, so a machine restart costs at most the
segment in progress: the server's database and the characters survive, and `compare.py` adds the
finished segments together (an unfinished `seg-NN` has no `end` file and is skipped).

On the runner, in one `live.yml` run (the servers last only that long), 150 minutes:

```
"$CI_SCRIPTS/start-servers.sh" && tools/soak/live-soak.sh
```

It copies the world files out of the original's container, creates Brynja and Isolde on both
servers (`ORIG_PORT`/`NEW_PORT`, default 2323/2325; the `m3/party` and `m3/bank` setups; both Human Warriors), runs partycheck and 2 x 25 min
on each, and prints `compare.py` for both (`soak-compare.log`). By hand, with your own servers:

```
MMUD_DATA=<dir of the .db files> tools/soak/soak.sh check   RUN        # partycheck, about 1 min
MMUD_DATA=...                    tools/soak/soak.sh segment RUN 25     # repeat; SOAK_PORT=<port> picks the server (default 2325)
python3 -I tools/soak/compare.py RUN [ORIG_RUN] [--serve-log silvermere-serve.log]
```

Priority: the text must be the same, so the gate that matters is an unknown line only on silvermere (check it against the original's text first; only then call it a classifier gap) and, outside the soak, the byte-equal steps of the m3 scenarios. Gates (server side only, exit status 1 on failure): no disconnect, no crash (`panicked` in the
serve log), no unknown line on silvermere that the original's run does not also have. The bots
are early tools, so `exp/hr`, deaths, position-ambiguous notes, unfollowed lines and the
follower's credit are advisory: printed, and flagged only when the two sides differ by more than a
factor of two. Echoes of the commands the bot typed (bridge `events.jsonl`) are not unknown lines;
`check` logs both characters in before partycheck (it drives sessions already in the realm) and
both steps log them off cleanly.

`m3_comms.rs::partycheck_hundred_telepaths` is the same telepath exchange over the in-process
server. `compare.py`, `soak.sh` and `live-soak.sh` were written without the original or the
bots running: `test_compare.py` checks the summing and the gates on synthetic logs only.
