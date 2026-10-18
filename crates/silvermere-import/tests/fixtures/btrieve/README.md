# Btrieve reader fixtures

Three synthetic test files from the MBBSEmu project's Btrieve library, wbtrv32 (its `assets/`
directory), copied unchanged. They hold test records (counting bytes, test keys and ACS tables),
no game content. `tests/btrieve_fixtures.rs` checks the reader against them, with the expected
numbers of wbtrv32's own tests. They are the only fixtures for v5 variable-length records and
per-key ACS. The real files, v5 and v6, are the stock distribution's, which the tests download
(T-120).

| File | What it exercises | sha256 |
|---|---|---|
| `MBBSEMU.DAT` | v5, fixed-length 74-byte records, four keys | `b0539351c3bd37abf0e1ffb5031aa12b99d32af84808f091f15059de906251ff` |
| `MULTIACS.DAT` | v6, variable-length, three keys each with its own ACS | `a59d406f7431e58dd14904f4779fdfb348cf3f0d632173257fa1dd13e66bb167` |
| `VARIABLE.DAT` | v5, variable-length records (fragments) | `ce13b5e268d4d619f56b5165f4078814e8efe6eb4ff8b0eacc3436933ff5d0ce` |

`tools/release/allowed-data.txt` lists them as the tree's only Btrieve files.
