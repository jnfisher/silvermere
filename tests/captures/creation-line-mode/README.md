# Capture: creation-line-mode

Character creation with the terminal reported as 79x24 by NAWS, so the module takes the non-full-screen path (the full-screen editor needs at least 80 columns): no form background, the text Point Cost Chart and race ranges (`Strength  35 - 0`: the maximum prints 0), and `You have N CP to distribute.` after every accepted stat. MBBSEmu's FSD engine still positions every field with `ESC[row;colf`, so the keystrokes are the same as in creation-fsd.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:44 |
| Terminal | NAWS 79x24 |
| Character | user Corvin, name Corvin Ashdown, race Elf, class Mage, level 1 |
| raw.bin | 13129 bytes, 4 `ESC[79D`, 0 `\x01` |
| Steps | 30 (29 sends) |

## Notes

- The BBS account was created beforehand (same NEW dialogue as entry-new-account; not captured).
- Answers: race 5 (Elf), class 12 (Mage), lawful N, family name Ashdown, Int 60, Wil 50; 80 CP left. Corvin is the death-rebirth character.

## Behaviours exercised

- the line-mode editor (terminal not ANSI >= 80x23)
- line-mode fallback
- the lawful question

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
- `Build #\S+|\d{1,2}/\d{1,2}/\d{2,4}|\d{1,2}:\d{2}(?::\d{2})?`: MBBSEmu banner build and clock
- `[-\\|/]\x08`: name-validation spinner (one frame per poll; the count depends on timing)

## Unclassified lines

Lines of `plain.txt` the pattern registry reports as `unknown`. `tests/test_captures.py` allows exactly these.

With game data (`Registry.load()` with a staged `test/run/built`):

```
+-    ... and so on ...   -+
+-    Point Cost Chart    -+
Agility   50 - 0
Are you sure (Y/N, or R to re-logon)? Y
Ashdown    35
Character Generator
Charm     50 - 0
Corvin                                    Elf                  Mage    35    90    35    50    120    50    40    100    40    50    120    50    30    80    30    50    120    50               none               black               black    SAVE                                                                               Corvin                                                                               Corvin                  Ashdown
Do you want to be Lawful?  [Yes/No] N
Have a nice day...
Health    30 - 0
If you answer YES to this question then you will never be allowed to instigate
Intellect 50 - 0
M A J O R   M U D
Make your selection (X to exit): M
Make your selection (X to exit): X
Ok, thanks for calling!
Password: *******
Please choose your class [ ? for help ] : 12
Please choose your race [ ? for help ] : 5
Remember, this is a very important choice. Once you have chosen Lawfulness,
Strength  35 - 0
Username: Corvin
Willpower 40 - 0
You must now choose if you want to be a truly 'lawful' citizen of the realm.
[MAJORMUD]: E
[MAJORMUD]: X
any action which would give you evil points, and any player that attacks or
can still attack those with a bad reputation. This option is designed
game, and choosing it for any other reason (Item storage, etc...) is strictly
prohibited.
robs from you will receive three times the regular evil points in return. You
solely for those who want to stay away from the player combat aspects of the
you may not remove this title, unless you start a new character.
|     ... and so on ...    |
| +10 to base stat:  10 CP |
| +20 to base stat:  30 CP |
| +30 to base stat:  60 CP |
| +40 to base stat: 100 CP |
| +50 to base stat: 150 CP |
| 1st 10 points: 1 CP each |
| 2nd 10 points: 2 CP each |
| 3rd 10 points: 3 CP each |
```

Additionally without game data (bare checkout, static patterns only):

```
Newhaven, Village Entrance
You are about to terminate this connection!
```
