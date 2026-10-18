# Capture: entry-existing

Login as an existing character: username, password, BBS menu `M`, the MajorMUD menu, `E` into the realm (the first room), Enter (statline FULL redisplays the room), then `x` (silent meditation, USREXIT), BBS `X`, `Y`.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:28 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level 1 |
| raw.bin | 4375 bytes, 5 `ESC[79D`, 0 `\x01` |
| Steps | 10 (9 sends) |

## Behaviours exercised

- the clean logoff that avoids the disconnect penalty
- the prompt has no newline; the next message starts with ESC[79D ESC[K
- Enter redisplays the room under statline FULL

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
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
Password: *******
Username: Aldric
[MAJORMUD]: E
[MAJORMUD]: X
```

Additionally without game data (bare checkout, static patterns only):

```
Newhaven, Village Entrance
Welcome to Newhaven! You are standing at the crude wooden gates of the
You are about to terminate this connection!
raiders. You can see small buildings to the north and south, and a dusty path
surrounds the village, to protect against creatures of the night and random
that leads to the west and southeast.
village entrance, in the middle of a dusty path. A low wooden palisade
```
