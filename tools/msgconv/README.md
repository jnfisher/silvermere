# msgconv

Writes `docs/config-defaults.md` from the options of `WCCMMUD.MSG` (T-011). A development tool:
standard library only, not shipped. The game's text in the `.MSG` files is not converted into
the repository: the server reads the files from the assets directory at start-up
(`crates/silvermere-text/src/package.rs`, which reads them the same way; docs/text.md).

```
D=/path/to/unzipped/WCCMMUD_1.11p_DOS_MBBSEmu_v2     # WCCMMUD.MSG
python3 -I tools/msgconv/msgconv.py --msg $D/WCCMMUD.MSG --options docs/config-defaults.md
python3 -I tools/msgconv/msgconv.py --selftest
```

- `--options` renders `config-defaults.md.in` with the option tables in `msgconv.py` (module
  defaults, what survives, notes) merged with the ranges and shipped
  values parsed from `WCCMMUD.MSG`. Edit the tables or the `.in` file, never the output.
- After regenerating, update `SHA_CONFIG_DEFAULTS` in `crates/silvermere-text/tests/msg_catalogue.rs`
  (`sha256sum` of the document). With `MMUD_DIST_ZIP=/path/to/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip`
  the test runs the converter itself (needs `unzip` and `python3`) and compares byte for byte.
- The `.MSG` rules and the line-end decision are in `docs/text.md`.
