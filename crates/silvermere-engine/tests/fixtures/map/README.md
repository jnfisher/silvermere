# MAP fixtures

- The stock town map, `WCCMAP01.ANS` (3803 bytes, sha256 `b17c6be4…182b455`), is not kept here:
  the test reads it from the stock distribution (the testkit's `stock` helper), unpacked unchanged
  from the MBBSEmu package `WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` (sha256
  `815f7b35…2d3583`). Room 1:164, the Sysop Support Chamber, names it as its `mapfile`.
- `orig-map.bin` (3823 bytes, sha256 `e3345aea…fec0e5`): what the original `WCCMMUD.DLL` sent for
  `map` in that room, from its dispatcher's `ANSI_NORMAL` up to the prompt. Cut from `orig.raw`
  of the `m4/stealth` scenario in the live run 39042397672 (live.yml,
  MBBSEmu v1.0-alpha-090826, 2026-10-17).
