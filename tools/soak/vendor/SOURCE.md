# Where vendor/ comes from

The party-soak bots, copied from the project's MBBSEmu test harness; edit here. Each Python file
starts with a provenance header.

| Here | Changes |
|---|---|
| `play/progress.py`, `party.py`, `partycheck.py`, `bridge.py` | none besides the header |
| `play/patterns.py` | the header; the same file as `tools/differential/vendor/play/patterns.py` |
| `play/mmud_data.py` | `DATA` honours `$MMUD_DATA`, the directory of the `WCC*.db` world files MBBSEmu makes from the distribution (`live-soak.sh` copies them out of the MBBSEmu container) |
| `play/plans/party-duo.json` | none |
| `play/parties.json` | none (reformatted JSON) |

`bridge.py` writes its logs to `play/logs/` and sockets next to itself (git-ignored).
