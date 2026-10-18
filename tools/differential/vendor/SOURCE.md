# Where vendor/ comes from

Copied from the project's MBBSEmu test harness; edit here. The table lists the changes made on
copying.

| Here | Changes |
|---|---|
| `play/bridge.py` | none besides the provenance header |
| `play/patterns.py` | the provenance header; patterns carry no source field (`tests/fixtures/patterns.json` is its export) |
| `play/mmud_data.py` | the provenance header; `DIST_ZIP` is the stock zip in the download cache (T-120); for T-040's record-level comparison (`WCCUSERS`, `WCCACMSR` decoded from MBBSEmu's `.db` files) |
| `play/capture.py` | provenance header; `Runner.step` takes the stream offset of a `connect` step before connecting (a banner sent at once by a fast server was missed) |
| `test/stage.sh` | provenance header; stages `orig` only (no `built` instance); opt-in `STAGE_CODEC_OF_TWINKS=N` writes `WCCMMUD.INI` with `CODEC_OF_TWINKS=N` (T-057, the m4 group's developer sysop commands); with no folder given, takes the distribution zip from `../fetch-stock.sh` (T-120) |
| `test/run.sh`, `test/keygen.py` | none besides the provenance header |
| `test/keys/*` | none: `keygen.py` output for BTURNO 12345678 (ACTIVATE code and `WCCADDON.SYS`); `stage.sh` uses it when `keygen.py` is not available |
| `test/config/*.json` | none |

`vendor/` is laid out like the repository root the scripts expect (`dist/`, `test/`, `play`).
Staged output goes to `vendor/test/run/` and the MBBSEmu binary to `vendor/tools/mbbsemu/`
(both git-ignored).

`SILVERMERE_RESEARCH_DIR` (a local copy of the harness tree) is an optional override only: CI and the
tests never clone, fetch or need a token for anything outside this repository.

The distribution zip is not vendored (T-120): `tools/differential/fetch-stock.sh` downloads
`WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` from download.mbbsemu.com into the cache the tests share and
checks it against the sha256 pinned in `crates/silvermere-import/src/stock.rs`.
