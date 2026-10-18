# Stock world export digests

`DIGEST.json` is what `tests/equivalence.rs` and `tests/post_update_equivalence.rs` compare the
import against: digests of the export MBBSEmu's database gave for the stock 1.11p world
(post-update, after one start; the distribution zip's sha256 is in the file). The export is the
game's records, so it is not kept here (T-120). Per table, the file holds:

- `rows`: the records the export holds;
- `compared`: the records it shares with the virgin import, and `ids_sha256` over their ids;
- `fields`: per compared field (run-time fields left out, rooms masked), the first 16 hex digits
  of a sha256 over every compared record's value.

No value is stored. `tests/support/world_digest.rs` gives the canonical form. To rebuild it from
an export directory (`MANIFEST.json` and `<TABLE>.jsonl.gz`):
`MMUD_WORLD_JSON=DIR UPDATE_WORLD_DIGEST=1 cargo test -p silvermere-import --test equivalence`.
