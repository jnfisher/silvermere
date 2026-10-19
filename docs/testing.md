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
