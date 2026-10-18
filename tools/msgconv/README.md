# msgconv

Converts the MajorMUD message files into the checked-in data of `silvermere-text`
(T-011). A development tool: standard library only, not shipped.

```
D=/path/to/unzipped/WCCMMUD_1.11p_DOS_MBBSEmu_v2     # WCCMMUD.MSG, WCCTEXT.MSG, WCCMMHLP.MSG
M=crates/silvermere-text/messages
python3 -I tools/msgconv/msgconv.py --msg $D/WCCMMUD.MSG --catalogue $M/wccmmud.toml --options docs/config-defaults.md
python3 -I tools/msgconv/msgconv.py --msg $D/WCCTEXT.MSG --catalogue $M/wcctext.toml --assets $M/assets
python3 -I tools/msgconv/msgconv.py --msg $D/WCCMMHLP.MSG --help-topics $M/help_topics.toml
python3 -I tools/msgconv/msgconv.py --selftest
```

- `--options` renders `config-defaults.md.in` with the option tables in `msgconv.py` (module
  defaults, what survives, notes) merged with the ranges and shipped
  values parsed from `WCCMMUD.MSG`. Edit the tables or the `.in` file, never the output.
- After regenerating, update the `SHA_*` constants in `crates/silvermere-text/tests/msg_catalogue.rs`
  (`sha256sum` of the files). With `MMUD_DIST_ZIP=/path/to/WCCMMUD_1.11p_DOS_MBBSEmu_v2.zip` the
  test runs the converter itself (needs `unzip` and `python3`) and compares byte for byte.
- The `.MSG` rules and the line-end decision are in `docs/text.md`, "The converted `.MSG` files".
