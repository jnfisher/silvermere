# silvermere-testkit

Test support for the layers above unit tests (layers 2 to 4 in
[testing.md](testing.md)). It is a dev-dependency only; `silvermere-testkit` is not linked into the
server.

## A scripted integration test in ten lines

```rust
use silvermere_testkit::{ScriptedSession, Step, TestServer};

#[tokio::test]
async fn login_then_echo() {
    let server = TestServer::echo().await.unwrap();           // real silvermere-net acceptor
    let mut client = server.client().await.unwrap();          // refuses every telnet option
    let t = ScriptedSession::new()
        .step(Step::connect().until("Name: $"))
        .send_until("alice", r"You typed: (\w+)")
        .step(Step::send("quit").quiet(0.3))
        .run(&mut client).await.unwrap();
    assert!(t.plain.contains("You typed: alice"));            // t.raw has the bytes
}
```

The same steps can come from a recorded `script.json` (`ScriptedSession::from_script_json`),
whose format is described in `tests/captures/README.md`. For a client-only check, use
`TelnetClient::connect(addr)`, `send_line`, `until(regex, timeout)` and `read(quiet)`.

## What is in the crate

| Item | What it is |
|---|---|
| `TelnetClient` | `bridge.py` in Rust: `DO x` answered `WONT x` (NAWS answered when `with_naws`), `WILL` ignored, `IAC IAC` is 0xFF, subnegotiations skipped; raw bytes kept; `plain()` is `bridge.plain`. Text that arrives between commands comes back as `skipped` on the next send. |
| `ScriptedSession`, `Step` | `capture.Runner.step` semantics: `send`/`raw`/`until`/`until_cursor`/`wait`/`timeout`/`nowait`/`set`/`if_unset`/`skip_if`/`required`, ops `connect`, `disconnect`, `sleep`. `quiet_cap` keeps replays of one-second quiet periods fast. |
| `TestClock` (`silvermere_rules::Clock`) | Moves only when advanced; `today()`/`wall()` run from a fixed 2026-01-01 start; `advance_ticks(n)` is `n` fast ticks of one second. Clones share a timeline. |
| `seeded_rng(seed)`, `silvermere_rules::Rng` | `ChaCha8Rng`; `assert_uniform(samples, bins, tolerance)` is a chi-square per degree of freedom check. |
| `PatternSet` | Loads `tests/fixtures/patterns.json` (`load_default()`), expands the verb placeholders as the Python does without game data, picks `regex` or `fancy-regex` per pattern, and classifies lines like `Registry.iter_events` (prompts, echo, tables, wrapped exits). |
| `Capture`, `raw_to_lines`, `plain` | The twelve T-005 scenarios under `tests/captures/` and the two text-preparation ports. |
| `normalise`, `diff_events`, `MaskSpec` | Masks random fields **by pattern field** from `tests/fixtures/masks.json` and compares event sequences by name and unmasked fields. |
| `shared_template::shared_file`, `shared_source_file` | A database file built by the first test process that asks for it and copied by the others (a lock file under `target/tmp`). `shared_file`'s directory is named for the test executable and the fixtures, so each test binary builds its own; `shared_source_file`'s for the sources it is given alone, so every test binary of a build shares one (the `silvermere` binary's stock, Aldric and team templates). |
| `TestServer::echo()` | The real `silvermere-net` acceptor with a stand-in engine, on a random port. The full server (engine, clock, RNG, temp SQLite, fixture world) joins when T-015 and the fixture import land. |

## Files

- `tests/fixtures/patterns.json`: the T-004 export, byte for byte (also read by `silvermere-text`).
- `tests/fixtures/masks.json`: which fields are masked; the Python harness (T-019) reads the same file.
- `tests/captures/`: the T-005 captures, copied verbatim. Do not edit; re-record with `tools/differential/vendor/play/capture.py` (see `tests/captures/README.md`).
