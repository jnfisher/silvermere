# Capture: creation-fsd

Character creation over ANSI at MBBSEmu's default 80x24: race list, class list, the lawful question, the full-screen stat editor (FSD) driven one key at a time with a cursor-position check after each field (family name, Strength 50, an up/down arrow pair, Health 50, hair length cycled once with Space, SAVE), the suicide-password notice, name validation, the `st` screen the game shows, then Enter for the first room and a clean logoff.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:28 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level 1 |
| raw.bin | 15356 bytes, 4 `ESC[79D`, 0 `\x01` |
| Steps | 33 (32 sends) |

## Notes

- Answers: race 1 (Human), class 1 (Warrior), lawful N, given name default (Aldric), family name Stonebrook, Str 50, Hea 50; 80 CP left unspent for John.

## Behaviours exercised

- FSD driven by `ESC[row;colf` positions, one key at a time
- an arrow-key pair inside the editor (moves between fields here, it does not exit)
- name validation runs in the background (Timer.Hertz 1000: about a second)
- FSD editor; line-mode fallback is the creation-line-mode capture
- `Please wait - Validating your name.` and the spinner
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
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! 100                  Aldric                                    Human                  Warrior    40    100    40    40    100    40    40    100    40    40    100    40    40    100    40    40    100    40               none               black               black    SAVE                                                                               Aldric                                                                               Aldric                  Stonebrook 100                                                                               Stonebrook    40    50  90                                                                 50    40  90                                                                 40    50  90                                                                 50    40  90                                                                 40    40  90                                                                 40    40  90                                                                 40    40    50  80                                                                 50    40  80                                                                 40               none               SHORT                                                                            SHORT               black                                                                            black               black                                                                            black    SAVE                                                                 SAVE
Are you sure (Y/N, or R to re-logon)? Y
Do you want to be Lawful?  [Yes/No] N
Have a nice day...
If you answer YES to this question then you will never be allowed to instigate
Make your selection (X to exit): M
Make your selection (X to exit): X
Ok, thanks for calling!
Password: *******
Please choose your class [ ? for help ] : 1
Please choose your race [ ? for help ] : 1
Remember, this is a very important choice. Once you have chosen Lawfulness,
Username: Aldric
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
```

Additionally without game data (bare checkout, static patterns only):

```
Newhaven, Village Entrance
You are about to terminate this connection!
```
