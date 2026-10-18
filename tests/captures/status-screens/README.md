# Capture: status-screens

The character screens: `st`, `exp`, `i`, `set`, `spells`, `health`, `wealth`, `keys`, `who`, `par`, `top`.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:28 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level 1 |
| raw.bin | 6730 bytes, 23 `ESC[79D`, 0 `\x01` |
| Steps | 20 (19 sends) |

## Behaviours exercised

- the inventory line (`You are carrying ...`) with worn items
- 79-column wrap of the inventory line

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
- `\[HP=-?\d+(?:/(?:MA|KAI)=-?\d+)?`: HP/mana in the prompt depend on earlier fights and regen
- `Exp: \d+|Exp needed for next level: .*|Lives/CP:\s+\d+/\d+`: experience totals
- `(?m)^ *\S.*(?:Online|minutes|Level).*$`: who/top rows depend on who is on

## Unclassified lines

Lines of `plain.txt` the pattern registry reports as `unknown`. `tests/test_captures.py` allows exactly these.

With game data (`Registry.load()` with a staged `test/run/built`):

```
-=-=-=-=-=-=-=-=-=-=-=-
Aldric Stonebrook              (Warrior)             [H:100%]   - Midrank
Are you sure (Y/N, or R to re-logon)? Y
Have a nice day...
Make your selection (X to exit): M
Make your selection (X to exit): X
Ok, thanks for calling!
Password: *******
Rank Name                  Class      Gang/Guild          Experience
Username: Aldric
You have No money!
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
