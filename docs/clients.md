# Clients

Players connect to Silvermere over telnet, port 2323 unless the sysop changed
`server.telnet_bind`. Any telnet client works; this page says what each kind gets and how
to set it up.

| Client | What it gets |
|---|---|
| [Plain telnet](#plain-telnet-and-terminals) in an ANSI terminal | the original game, byte for byte: CP437 text, colour, the full-screen stat editor |
| [MegaMud](#megamud) and other terminal clients of the time | the same; scripts see the original prompts and text |
| [Mudlet](#mudlet) | an automatic map (with the mapper package), health, mana and status as data, the encoding set by itself |
| [Other MUD clients](#other-mud-clients) (TinTin++, MUSHclient, ...) | the same data over GMCP, for scripts |
| A BBS door | rlogin, with no second login: see [auth.md](auth.md#rlogin-a-bbs-door) |

There is no browser client yet.

## Plain telnet and terminals

`telnet HOST 2323` in a terminal set to ANSI colour and the CP437 character set (the DOS
character set the maps and logos are drawn in). For a UTF-8 terminal, type `set utf8 on` in
the game, or the sysop can make UTF-8 the default with `compat.charset = "utf8"`.

Character creation and `TRAIN STATS` open the original's full-screen form when the window is
at least 80 columns by 23 rows with ANSI on, and the line-mode editor (one question per
line) otherwise. Its keys are in [legacy-clients.md](legacy-clients.md).

## MegaMud

Add a BBS entry: the host, port `2323`, terminal type "ANSI". The login script sees these
prompts, in this order:

1. `Username:` then `Password:`. Type `NEW` at the username prompt to make an account (when
   the sysop allows open signup).
2. `Make your selection (X to exit):`. `M` enters MajorMUD, `X` logs off
   (`compat.bbs_menu = true`).
3. The `[MAJORMUD]:` menu. `[E] . Enter the Realm`.

MegaMud declines the newer options below (GMCP, CHARSET), so it sees the original text,
including the hidden letter in each direction name. Where the server differs from the
original on the wire, on purpose, is in [legacy-clients.md](legacy-clients.md) and
[differences.md](differences.md).

## Mudlet

[Mudlet](https://www.mudlet.org/) is a free MUD client with an automatic mapper. Silvermere
sends it room and character data over GMCP, so Mudlet can draw the map as you walk and show
health and mana.

### Connecting

Make a new profile with the server's address and port 2323. Mudlet agrees to GMCP and to
the character set by itself; there is nothing to turn on.

The server tells Mudlet its text is CP437 (telnet CHARSET), so Mudlet switches its encoding
and keeps it in the profile. Only the very first login screen of a new profile comes before
the switch and may show `�`. If the sysop turned `net.charset_negotiation` off, set
**Preferences**, **General**, **Server data encoding** to CP437 yourself. `set utf8 on`
switches the game to UTF-8, and Mudlet with it.

The original game hides a random letter and a backspace in each direction name (`nX`,
backspace, `orth`) so that client scripts can't match "north". Mudlet doesn't apply
backspaces, so once a client has agreed to GMCP the server leaves the letters out; it has
the exits as data anyway.

Character creation and `TRAIN STATS` use the line-mode editor in Mudlet (one question per
line): Mudlet shows a scrolling window with its own input line, so it can't draw the
full-screen form.

### The map

Mudlet draws maps from GMCP on its own only for a few games it knows. For Silvermere,
install the mapper package once per profile:

1. Download [`silvermere-mapper.mpackage`](https://raw.githubusercontent.com/jnfisher/silvermere/main/tools/mudlet/silvermere-mapper.mpackage)
   (it is also in the source, `tools/mudlet/`).
2. In Mudlet: **Package Manager**, **Install new package**, pick the file. The Package
   Manager then lists it as `silvermere-mapper` with its version and a description.
3. Open the map (**Map** on the toolbar, or the map panel of the Mudlet UI) and walk.

To update it, uninstall `silvermere-mapper` in the Package Manager and install the new
file; the map you have drawn stays. An earlier copy installed from `silvermere-mapper.xml`
has the same name, so uninstall that first too.

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

### Health and mana

`gmcp.Char.Vitals` holds `hp`, `maxhp`, `mp` and `maxmp`, and `gmcp.Char.Status` holds
`name`, `fullname`, `level`, `class`, `race` and `exp`, each sent again whenever it changes.
A gauge script can read them; the event names are `gmcp.Char.Vitals` and
`gmcp.Char.Status`. For example, a script with a `gmcp.Char.Vitals` event handler:

```lua
local v = gmcp.Char.Vitals
echo(string.format("HP %d/%d  MA %d/%d\n", v.hp, v.maxhp, v.mp, v.maxmp))
```

## Other MUD clients

Any client that speaks GMCP (TinTin++, MUSHclient with a GMCP plugin, Blightmud, ...) gets
the same messages as Mudlet. They are UTF-8 text, `Package.Name` and then JSON:

| Package | When | Data |
|---|---|---|
| `Room.Info` | each room display (moving, `LOOK`) | `num` (`map * 1000000 + room`), `name`, `area` (`Map <n>`), `environment` (`normal`, `shop`, `hotel`, `library`, `arena`, `colliseum`, `lair`, `jail`), `map`, `room`, `exits` (`{"n": num, ...}`) |
| `Char.Name` | entering the realm | `name`, `fullname` |
| `Char.Status` | entering the realm, and on change | `name`, `fullname`, `level`, `class`, `race`, `exp` |
| `Char.Vitals` | entering the realm, and on change | `hp`, `maxhp`, `mp`, `maxmp` |
| `Core.Ping` | answering the client's `Core.Ping` | none |

Exits use the short names `n`, `s`, `e`, `w`, `ne`, `nw`, `se`, `sw`, `u` and `d`, and are
the `Obvious exits:` ones. Until the client sends `Core.Supports.Set`, every package goes
out; after it, only the modules it lists (`Core` always). The wire details are in
[net.md](net.md#gmcp).

A client that agrees to GMCP is taken to be a MUD client: it gets the direction names
without their hidden letter and the line-mode stat editor, as Mudlet does.

## What the server negotiates

On connecting the server offers these telnet options; a client that ignores or refuses one
gets the original behaviour.

| Option | What it is for | Switch |
|---|---|---|
| ECHO, SGA, BINARY | the server echoes typing and sends 8-bit text, as the original host did | always |
| NAWS | the window size, which picks the stat editor | always |
| GMCP (201) | room and character data for mappers and gauges | `net.gmcp` |
| CHARSET (42) | tells the client the text is CP437 (or UTF-8 after `set utf8 on`) | `net.charset_negotiation` |

Both switches are on by default; see [config.md](config.md). How each option is answered is in
[net.md](net.md#negotiation).
