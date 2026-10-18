# Contributing to silvermere

Open an issue first for anything larger than a fix, then a pull request against `main`
filling in the PR template. These rules hold for every change.

## Checks (all required on `main`)

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
SILVERMERE_DATABASE_URL=sqlite:// cargo test --workspace
SILVERMERE_DATABASE_URL=postgres://user:pass@localhost/silvermere cargo test --workspace
cargo deny check
docker build -f docker/Dockerfile .
```

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml); rustup picks it
up. Stable only, no nightly features. Fix a clippy lint rather than `#[allow]`ing it; an
`#[allow]` needs a comment saying why.

## Crate dependency rule

Dependencies point strictly downward:

```
types <- rules <- text <- engine <- {db, auth, net, api} <- bin
import -> {types, db}
```

- `silvermere-engine` never depends on `-net` or `-db`: it consumes input events and emits
  output events and persistence requests over channels.
- `silvermere-import` is the only crate that knows Btrieve layouts.
- `silvermere-testkit` is a dev-dependency only.
- Every crate's `Cargo.toml` already lists the lower crates it may use. Adding an upward or
  sideways dependency fails review by diff; if a card seems to need one, the design is
  wrong: raise it in the PR instead.
- Third-party versions are pinned once in `[workspace.dependencies]` in the root
  `Cargo.toml`; crates use `name.workspace = true`. Use the libraries already there; a change
  that needs a different one raises it first.
- Library crates use `thiserror`; binaries use `anyhow`. `unsafe` is forbidden workspace-wide.

New crates start from [`crates/_template/`](crates/_template/) (not a workspace member):
copy it, replace `NAME`, add it to the workspace members in dependency order and to
`[workspace.dependencies]`.

## Tests

- Every crate has a `tests/` directory; extend it. Unit tests for private helpers go in a
  `#[cfg(test)] mod tests` next to the code.
- **Validated tests.** A test that pins the original's behaviour says on the line above how
  that behaviour was established: the capture, MBBSEmu run or differential scenario it matches.

  ```rust
  // the creation-line-mode capture: the stat editor's minimums for a new character
  #[test]
  fn creation_minimums_match_the_capture() { ... }
  ```

- `proptest` for rules with numeric domains, `insta` snapshots for rendered screens,
  `silvermere-testkit` for integration tests over telnet. Tests are deterministic: use the
  `TestClock` and a seeded RNG, never wall-clock time or `thread_rng`.
- Database tests read `SILVERMERE_DATABASE_URL` and skip when it is unset; CI runs them on
  SQLite and PostgreSQL. SQL stays in the portable subset; where dialects differ, the
  `Dialect` enum in `silvermere-db` picks the statement.

## Style

- `cargo fmt` defaults; edition 2024.
- Each crate's `lib.rs` opens with a doc comment naming its responsibility.
- Comments say why, not what. Describe behaviour, never the original's internals: no source
  file names or line numbers, no names of its functions or variables, no quoted code.
  `tools/release/wording-guard.pl` (run by CI) fails on them.
