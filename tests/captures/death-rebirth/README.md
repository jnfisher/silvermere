# Capture: death-rebirth

A level-1 Elf Mage (20 HP, no armour, no weapon) goes down into the arena and rests (accepted); then fights each spawn with bare hands until a monster knocks it below 0 HP (negative HP in the prompt; `rest` refused while mortally wounded), lies there until `You have been killed!`, then the miracle and lives lines, the rebirth room, `i` (empty) and `st`.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:57 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Corvin, name Corvin Ashdown, race Elf, class Mage, level 1, start Newhaven, Narrow Road |
| raw.bin | 30735 bytes, 478 `ESC[79D`, 0 `\x01` |
| Steps | 50 (24 sends) |

## Notes

- No sysop commands were used: Corvin rested, then fought back with fists until killed.
- A first attempt that only rested was stopped after ten minutes: the arena's giant rat never hurt Corvin (`The fat giant rat lunges at you!` only). Its broken connection was cleared by a login (not captured) before this run, so no punishment lines appear here.
- Up to eight fight rounds are scripted; the rounds after the knockout are skipped (`skip_if`).

## Behaviours exercised

- a life is lost (lives count shown)
- resting in a spawn room
- gear and coins stay in the death room (`i` empty)
- negative HP in the prompt
- DEATHHP -200: the unconscious window before death
- death text
- rebirth room
- mortally wounded restrictions

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
- `\[HP=-?\d+(?:/(?:MA|KAI)=-?\d+)?`: HP/mana in the prompt depend on earlier fights and regen
- `for \d+ damage|You gain \d+ experience|\d+ (?:copper farthings?|silver nobles?|gold crowns?|platinum pieces?|runic coins?)`: RNG: damage, experience share, coin drops
- `giant rat|lashworm|filthbug|carrion beast|acid slime|kobold thief`: which monster spawned
- `\w+ just (?:entered|left) the Realm\.`: other players coming and going

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
Username: Corvin
[MAJORMUD]: E
[MAJORMUD]: X
```

Additionally without game data (bare checkout, static patterns only):

```
A acid slime oozes into the room from nowhere.
A carrion beast creeps in the room from nowhere.
A large carrion beast creeps in the room from nowhere.
A thin giant rat creeps into the room from nowhere.
Newhaven, Arena
Newhaven, Healer
Newhaven, Narrow Road
The acid slime lashes at you, but you dodge out of the way!
The blood-soaked floor is covered in broken, rusted weaponry and some things
The carrion beast falls to the ground with a yelp, and is still.
The carrion beast snaps at you with its teeth!
The fat giant rat lunges at you!
The giant rat falls to the ground with a tortured squeak.
The large carrion beast snaps at you with its teeth!
The large carrion beast snaps at you, but you dodge out of the way!
The thin giant rat lunges at you!
The thin giant rat lunges at you, but you dodge out of the way!
This huge room is has been carved into the earth with crude work- manship.
This narrow road is quite plain save for the various lanterns hanging from
You are about to terminate this connection!
better left unmentioned. A huge iron gate sits in the west wall, and behind it
buildings lie to your north and west.
clash of steel below you, and cries of pain and triumph accompanying it. Stone
in the northern wall.
the trees around, and a large stone stairwell leading downwards. You hear the
you can hear the growling of some unfathomable beast. A large black door is set
```
