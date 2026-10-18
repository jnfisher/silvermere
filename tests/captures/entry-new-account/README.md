# Capture: entry-new-account

A new BBS account through MBBSEmu's NEW dialogue (username, password twice, e-mail, gender), the BBS menu, the MajorMUD menu; then back out (`X`, `X`, `Y`) before any character exists. Everything before the MajorMUD menu is MBBSEmu's, not the module's.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:27 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level none yet (account only) |
| raw.bin | 4709 bytes, 0 `ESC[79D`, 0 `\x01` |
| Steps | 11 (10 sends) |

## Behaviours exercised

None: this scenario is MBBSEmu's account dialogue and the MajorMUD menu.

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `Build #\S+|\d{1,2}/\d{1,2}/\d{2,4}|\d{1,2}:\d{2}(?::\d{2})?`: MBBSEmu banner build and clock

## Unclassified lines

Lines of `plain.txt` the pattern registry reports as `unknown`. `tests/test_captures.py` allows exactly these.

With game data (`Registry.load()` with a staged `test/run/built`):

```
Are you sure (Y/N, or R to re-logon)? Y
Have a nice day...
Make your selection (X to exit): M
Make your selection (X to exit): X
Ok, thanks for calling!
Username: NEW
[MAJORMUD]: X
and will only use on this system.
so please select a strong, unique password that you have never used before
```

Additionally without game data (bare checkout, static patterns only):

```
You are about to terminate this connection!
```

## Known script artefact

The final `Y` step in `script.json` records `matched: false, waited: 0.0`. That comes from an earlier `capture.py` revision that waited a fixed 2 s without a pattern; the bytes in `raw.bin` are correct. Regenerating the script results would fix the field.
