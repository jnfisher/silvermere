# Capture: shop

Newhaven shops from the Village Entrance: the weapon shop `list`, `buy quarterstaff` twice (duplicates allowed), `sell quarterstaff`, `appraise quarterstaff`, `wield` and `i` (Two handed); the armour shop `list` and the free padded set worn; the general store `list`, a purchase it cannot afford and one it does not sell; the healer, where `list` prints nothing (a Hospital shop sells only `healing` and `curing`; see fight-arena).

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:28 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level 1 |
| raw.bin | 13779 bytes, 78 `ESC[79D`, 0 `\x01` |
| Steps | 45 (44 sends) |

## Behaviours exercised

- a shop refuses what it does not stock (`... is not a known item.`) and what you cannot afford
- the quarterstaff shows as `(Two handed)` once wielded
- the free starter gear
- the second `buy quarterstaff` is accepted (duplicate buys)
- duplicate buys
- inventory line wrap

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
- `\[HP=-?\d+(?:/(?:MA|KAI)=-?\d+)?`: HP/mana in the prompt depend on earlier fights and regen
- `^\S.*?\s+\d+\s+(?:Free|\d+ \w+)$`: shop quantities change with other players' purchases

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
Behind a small wooden counter sits a large, bearded man, with a cheerful look
Newhaven, Armour Shop
Newhaven, General Store
Newhaven, Healer
Newhaven, Narrow Path
Newhaven, Narrow Road
Newhaven, Village Entrance
Newhaven, Weapons Shop
This dusky old shop carries a small assortment of weapons suitable for the
This large wooden shack carries all sorts of useful goods. A big,
This makeshift store carries a small assortment of clothes and armour.
This narrow path is fairly plain, save for the small lanterns hanging from
This narrow road is quite plain save for the various lanterns hanging from
This small, stark stone chamber is bereft of even the simple ornamentation,
Welcome to Newhaven! You are standing at the crude wooden gates of the
You are about to terminate this connection!
and its bare walls gleam in the bright light of the torches lining its walls. A
as you enter. The general store is a necessary shop for adventurers, as it
buildings lie to your north and west.
clash of steel below you, and cries of pain and triumph accompanying it. Stone
doorway to the south.
east, where faint sounds of metal hitting metal can be heard.
end of the room. There is an iron washbasin and some simple, utilitarian items
flickering glow. A stocky, mustached man stands behind a counter here, waiting
from here.
interest. You can exit through the doorway to the north.
long-haired man sits behind a counter whistling to himself, and smiles at you
on his face. Other than the assortment of items here, there is little of
raiders. You can see small buildings to the north and south, and a dusty path
sells light sources used to explore murky caverns. You can leave the shop
shack labeled "General Store" lies to your south. The path leads east and west
short, stocky man with a black beard sits behind a large stone table at the far
surrounds the village, to protect against creatures of the night and random
that leads to the west and southeast.
the trees around, and a large stone stairwell leading downwards. You hear the
the trees around. North of you lies a strangely decorated shop, and a beat-up
through the open doorway to the north.
to provide weaponry to novice adventurers. You can exit the shop through a
used in the healing profession lie at his side. You can exit the chamber to the
village entrance, in the middle of a dusty path. A low wooden palisade
young adventurer. Torches line the walls, illuminating the room with a smoky,
```
