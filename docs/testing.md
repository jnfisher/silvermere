# Testing

`cargo test --workspace` runs every test in this repository; the tests use SQLite unless
`SILVERMERE_DATABASE_URL` names a PostgreSQL database. The repository holds no game data: the
importer's suites and the end-to-end tests read the stock MajorMUD 1.11p distribution, which the
first run downloads once (6.7 MB, from download.mbbsemu.com) into `~/.cache/silvermere/stock`
(`SILVERMERE_STOCK_CACHE` moves it), checks against the sha256 pinned in
`crates/silvermere-import/src/stock.rs` and unpacks there. Later runs need no network. Offline,
save the zip there by hand (`tools/differential/fetch-stock.sh` prints the path and does the
download); a test with no copy fails saying so, it never skips.

| Command | What it runs |
|---|---|
| `cargo test --workspace` | unit and integration tests on SQLite |
| `SILVERMERE_DATABASE_URL=postgres://… cargo test -p silvermere-db` | the database crate on PostgreSQL |
| `cargo clippy --workspace --all-targets -- -D warnings` | lints, as CI runs them |
| `cargo run --manifest-path tools/schema-doc/Cargo.toml -- --check` | `docs/schema.md` matches the migrations |

CI (`.github/workflows/ci.yml`) runs all four on GitHub-hosted runners, with the stock
distribution kept by `actions/cache`. It runs the tests with `cargo-nextest` (settings in
`.config/nextest.toml`) plus the doctests, and skips changes that touch only prose no test reads.
CI builds without debug info and without crate-local ThinLTO (`CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_DEV_LTO=off`), which cuts the test build by about a quarter; overflow checks and
debug assertions stay on, and a local build keeps the line tables of `Cargo.toml`.

Each crate's integration tests are modules of one test binary, `tests/all.rs`: a new test file is
a `mod` line there, and a test's name starts with its file's name, so
`cargo test -p silvermere --test all m1_walk::` runs one file. The one exception is the
binary's `metrics_recorder`, a test binary of its own because it installs the process-wide
metrics recorder.

## Tests that need the original game

The differential harness (`tools/differential/`) runs the original module under MBBSEmu next to
silvermere and compares their output. It needs an installed copy of the game with its program
files, which this repository does not hold, so it is not part of `cargo test` or CI here.
`docs/import.md` describes importing from an installed copy.

## The demo and stress run

`tools/demo/run.sh` puts players on a server: it imports the stock world, makes 64 characters
and lets them play, then records the web console or measures the server.

```sh
tools/demo/run.sh stress demo-run 5   # 64 characters for 5 minutes: demo-run/report.json
tools/demo/run.sh media demo-run      # the console's screenshots, GIF and MP4 scenes
tools/demo/run.sh all demo-run 5      # both
```

It needs cargo, node 18 or later and ffmpeg (it installs Playwright's Chromium for the
recording). GitHub runs the same on demand: Actions, "Demo and stress run", Run workflow; the
media and the report are the run's artifact. Nothing it makes is committed.

The characters come from `populate`, an example of the binary that is not shipped:

```sh
cargo run --release -p silvermere --example populate -- seed --database-url sqlite://world.db
cargo run --release -p silvermere --example populate -- run --telnet 127.0.0.1:2323 --minutes 10
```

- **seed** runs the server in its own process on the database and makes each character as a
  player would, through an account and the creation dialogue; then the levels a trainer gives
  (the game's own gains), the spells the class may learn at that level, the best armour and
  weapon the class may use (worn with WEAR and WIELD), and a start in an area whose monsters
  suit the level. The levels come in brackets: 24 characters at 1-5, 20 at 6-15, 12 at 16-30
  and 8 at 31-50. The areas come from the world: monster types are ranked by the median
  experience their monsters give and split among the brackets. `--seed` makes the same
  population again; `population.json` holds the names, the passwords and the areas' rooms.
- **run** plays them against any server, even one on another computer: each connects from an
  address of its own (`--from`, on Linux any `127.x.y.z`), walks its area, fights what it
  meets, rests when hurt and gossips now and then. It reports the commands per second, the time
  from a command to its prompt, the fights, kills, deaths and disconnects, and with
  `--server-pid` the server's CPU and memory every 10 seconds.

On the development container, 64 characters made about 10 commands a second for five minutes
with no disconnects; the server used 1.1% of one core on average (2.8% at most) and 176 MB.
