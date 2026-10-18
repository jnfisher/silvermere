# Capture: room-walk

Rooms: `look` at the Village Entrance, `n` and `s` (shop and back), the arena's closed door (`n`, `open n`: locked), the skiff to Silvermere, two consecutive `Docks` rooms and a run of `River Street`, the manhole into the dark sewer (`The room is very dark - you can't see anything`), `exits`, `search`, and the walk back to Newhaven.

| | |
|---|---|
| Server | original `WCCMMUD.DLL` (sha256 `e308533696e1e64cf52715921d94d4245f4708f56ea048e4f7c6ec9b564b725f`) under MBBSEmu v1.0-alpha-090826, telnet 2323 |
| Captured | 2026-10-14 14:42 |
| Terminal | no NAWS answer (MBBSEmu default 80x24, ANSI on) |
| Character | user Aldric, name Aldric Stonebrook, race Human, class Warrior, level 1, start Newhaven, Narrow Road |
| raw.bin | 26544 bytes, 153 `ESC[79D`, 0 `\x01` |
| Steps | 61 (60 sends) |

## Behaviours exercised

- titles are not unique (`Docks`, `River Street`)
- identical consecutive rooms
- dark rooms show nothing but the darkness line
- the arena's closed north door: `The door is closed!`, `The door is locked.`
- duplicate titles
- dark room text
- 79-column wrap of descriptions
- door refusals

## Mask hints

Fields the comparer masks before a byte comparison (also in `script.json` `mask`):

- `\(Dec 30 2005 14:20:05\)`: build stamp of the DLL (mud_title)
- ` \[\d+ game minutes left\]| \[Last chance\]`: menu time-left suffix
- `[A-Z ]\x08`: every direction name carries a random capital letter and a backspace, rewritten each slow cycle: `nJ\borth`
- `\[HP=-?\d+(?:/(?:MA|KAI)=-?\d+)?`: HP/mana in the prompt depend on earlier fights and regen
- `Also here: .*|.* (?:creeps|wanders|scurries|crawls|slithers|walks|sneaks) into the room.*`: monsters and players that happen to be around

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
e
n
nw
w
x
```

Additionally without game data (bare checkout, static patterns only):

```
Docks
Intersection of Brass St. & River St.
Intersection of Guild St. & River St.
Intersection of River St. & Mystic Alley
Newhaven, Arena
Newhaven, Docks
Newhaven, Forest Path
Newhaven, Narrow Path
Newhaven, Narrow Road
Newhaven, Village Entrance
Newhaven, Weapons Shop
Off to the north, a rickety boat launch juts out into the fast-flowing waters
River Street
Small Pier
Tavern".
The blood-soaked floor is covered in broken, rusted weaponry and some things
The trees become very thick around you now, and you can barely see where
This dusky old shop carries a small assortment of weapons suitable for the
This huge room is has been carved into the earth with crude work- manship.
This is a cobblestoned street. It is dimly lit by guttering lanterns hung
This narrow path is fairly plain, save for the small lanterns hanging from
This narrow road is quite plain save for the various lanterns hanging from
Welcome to Newhaven! You are standing at the crude wooden gates of the
You are about to terminate this connection!
You can make out the entrance to a low wooden building off to the south.
You climb into one of the skiffs, and row to Silvermere.
You hop in one of the skiffs, and row to Newhaven.
You pull open the manhole cover, and slip inside the hole.
You stand at the southernmost edge of Newhaven. The view from here is
a small, run-down shack. From the look of it, it looks to be a caretaker's
amazing. As you stare out across the Silver river you behold such sights as the
and from Silvermere.
and from Silvermere. A small wooden pier pokes out into the river to the north.
be a fishing rod, and there are a number of small skiffs tied to the dock for
berths are empty also, echoing the effect of the recent decrease in traffic to
better left unmentioned. A huge iron gate sits in the west wall, and behind it
buildings lie to your north and west.
clash of steel below you, and cries of pain and triumph accompanying it. Stone
continues to the east and west.
doorway to the south.
east.
flickering glow. A stocky, mustached man stands behind a counter here, waiting
from here.
from tall posts. The city wall is to the north, towering over twenty feet above
from tall posts. There are large open crates piled here, all empty. The boat
from tall posts. To the north is the city wall, and to the south there is a
from tall posts. To the north you can see the entrance to the Wharf district,
from tall posts. To the west you can see Wharf street, and to the east there is
go manhole
goes off to the south here, heading towards, and ending in, the town square.
half-dozen skiffs of varying size.
house of some sort. River street runs parallel to Wharf street to the south.
in the northern wall.
notorious Port Blackwater to the west, and the fabled town of Silvermere
of here.
of the Silver river.
one of the lamp-posts, reading "Magic Shoppe" in a thin, spidery script.
pillars over the swift waters of the river. Tied to the far end are a
raiders. You can see small buildings to the north and south, and a dusty path
seems to be no-one home, as there are no light lit. River street continues to
shack labeled "General Store" lies to your south. The path leads east and west
south here. There is a manhole set in the ground here.
southwards. A large half-shaven man sits off in the corner, with what looks to
streets. It continues to the east and west.
streets. It continues to the east and west. An ornate metal sign is attached to
streets. It continues to the east and west. Brass street arrows off to the
streets. It continues to the east and west. The broad expanse of Guild street
streets. It continues to the east and west. The side of a seedy inn stands to
streets. It continues to the east and west. There is a seedy-looking wooden
streets. It continues to the east and west. You can see a junction off to the
structure to the south, and a sign over the open door reads "The Wailing Hound
surrounds the village, to protect against creatures of the night and random
tall, rickety house, whose door is set with a tarnished brass knocker. There
that leads to the west and southeast.
the east and west.
the light shining through the trees. You hear the sound of rippling water south
the trees around, and a large stone stairwell leading downwards. You hear the
the trees around. North of you lies a strangely decorated shop, and a beat-up
to provide weaponry to novice adventurers. You can exit the shop through a
use in reaching the docks of Silvermere.
village entrance, in the middle of a dusty path. A low wooden palisade
which leads to the city's docks. To the south is a small alleyway. River street
you can hear the growling of some unfathomable beast. A large black door is set
you're going due to the only light source being from the village behind you and
you. River street is fairly quiet, but is well-used as it intersects many main
young adventurer. Torches line the walls, illuminating the room with a smoky,
your south, and an intersection is to the west.
```
