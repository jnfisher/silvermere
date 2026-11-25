# Playing with Mudlet

[Mudlet](https://www.mudlet.org/) is a free MUD client with an automatic mapper. Silvermere
sends it room and character data over GMCP (`net.gmcp`, on by default; see
[net.md](net.md#gmcp)), so Mudlet can draw the map as you walk and show health and mana.

## Connecting

Make a new profile in Mudlet with the server's address and telnet port (2323 unless the
sysop changed `server.telnet_bind`). Mudlet agrees to GMCP by itself.

## Text

The server sends the game's text in CP437, the DOS character set its maps and logos are
drawn in. Set Mudlet's **Preferences**, **General**, **Server data encoding** to CP437, or
the box characters show as `�`.

The original game hides a random letter and a backspace in each direction name (`nX`,
backspace, `orth`) so that client scripts can't match "north". Mudlet doesn't apply
backspaces, so once a client has agreed to GMCP the server leaves the letters out; it has
the exits as data anyway. Clients without GMCP (MegaMud, plain telnet) get them as before.

## The map

Mudlet only draws maps from GMCP on its own for a few games it knows. For Silvermere,
install the mapper package once per profile:

1. Download [`silvermere-mapper.xml`](https://raw.githubusercontent.com/jnfisher/silvermere/main/tools/mudlet/silvermere-mapper.xml)
   (it is also in the source, `tools/mudlet/`).
2. In Mudlet: **Package Manager**, **Install new package**, pick the file.
3. Open the map (**Map** on the toolbar) and walk.

Each room you see is added to the map, in an area per game map (`Map 1`, `Map 2`, ...), and
linked to the rooms next to it. Exits to rooms you have not been to yet show as stubs.
Shops, hotels, libraries, arenas, lairs and the jail get their own colours.

- The map shows only what the game shows you: no rooms in the dark, and hidden exits only
  once you have found them.
- New profiles also come with Mudlet's "generic mapper", which guesses rooms from the text.
  Don't turn it on (`start mapping`) as well.
- Rooms are placed one step from a room next to them. MajorMUD's maps don't always fit a
  grid, so two rooms can land on the same spot; drag one aside in the map editor.
- Room numbers are `map * 1000000 + room` (room 1 of map 1 is 1000001), the same from one
  session or server restart to the next, so a saved map stays good.

## Health and mana

`gmcp.Char.Vitals` holds `hp`, `maxhp`, `mp` and `maxmp`, and `gmcp.Char.Status` holds
`name`, `level`, `class`, `race` and `exp`, both updated whenever they change. A gauge
script can read them; the event names are `gmcp.Char.Vitals` and `gmcp.Char.Status`.
