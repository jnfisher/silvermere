# Capture: fight-arena

Newhaven Arena (1:2150): wait for a monster, `a <monster>` (*Combat Engaged*), the same attack re-sent while engaged, the fight to `*Combat Off*`, `get copper`, `rest` until healed (or interrupted), `exp`, back up the stairs, `buy healing` at the Newhaven healer (free at full HP), and log off.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:29 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level 1 |
| raw.bin | 9215 bytes, 51 `ESC[79D`, 0 `\x01` |
| Steps | 25 (21 sends) |

## Notes

- The `rest` step matched instantly (`waited` about 0): the fight cost no HP, so there was nothing to rest off. T-019 must not treat this scenario as exercising a real rest; use `death-rebirth` or a wounded character for that.
- Monster chosen at run time: the last word of `Also here:` (script `set`).

## Behaviours exercised

- monster names in `Also here:` carry their alignment colour
- recovery is rest and healers: `buy healing` (2 copper per missing HP) at full HP it costs nothing: `You hand over nothing and all your wounds are healed.`
- the attack re-sent while engaged (what it does to the combat state)
- re-sent attack
- the bell after `drops to the ground!` (if a player falls)
- hit and miss verbs

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
- `\[HP=-?\d+(?:/(?:MA|KAI)=-?\d+)?`: HP/mana in the prompt depend on earlier fights and regen
- `for \d+ damage|You gain \d+ experience|\d+ (?:copper farthings?|silver nobles?|gold crowns?|platinum pieces?|runic coins?)`: RNG: damage, experience share, coin drops
- `(?:large|small|big|thin|fat|nasty|angry|fierce|short|tall|happy|slimy) `: random monster adjective
- `giant rat|lashworm|filthbug|carrion beast|acid slime|kobold thief`: which monster spawned

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
Newhaven, Arena
Newhaven, Healer
Newhaven, Narrow Path
Newhaven, Narrow Road
Newhaven, Village Entrance
The blood-soaked floor is covered in broken, rusted weaponry and some things
The giant rat falls to the ground with a tortured squeak.
The large giant rat lunges at you!
This huge room is has been carved into the earth with crude work- manship.
This narrow path is fairly plain, save for the small lanterns hanging from
This narrow road is quite plain save for the various lanterns hanging from
This small, stark stone chamber is bereft of even the simple ornamentation,
Welcome to Newhaven! You are standing at the crude wooden gates of the
You are about to terminate this connection!
and its bare walls gleam in the bright light of the torches lining its walls. A
better left unmentioned. A huge iron gate sits in the west wall, and behind it
buildings lie to your north and west.
clash of steel below you, and cries of pain and triumph accompanying it. Stone
east, where faint sounds of metal hitting metal can be heard.
end of the room. There is an iron washbasin and some simple, utilitarian items
from here.
in the northern wall.
raiders. You can see small buildings to the north and south, and a dusty path
shack labeled "General Store" lies to your south. The path leads east and west
short, stocky man with a black beard sits behind a large stone table at the far
surrounds the village, to protect against creatures of the night and random
that leads to the west and southeast.
the trees around, and a large stone stairwell leading downwards. You hear the
the trees around. North of you lies a strangely decorated shop, and a beat-up
used in the healing profession lie at his side. You can exit the chamber to the
village entrance, in the middle of a dusty path. A low wooden palisade
you can hear the growling of some unfathomable beast. A large black door is set
```
