# Mudlet mapper package

`silvermere-mapper.lua` builds Mudlet's map from Silvermere's GMCP `Room.Info`
(players' guide: [docs/mudlet.md](../../docs/mudlet.md); the GMCP messages:
[docs/net.md](../../docs/net.md#gmcp)). `silvermere-mapper.xml` is the Mudlet package
players install; it is generated from the Lua:

```
python3 tools/mudlet/build.py          # after editing the Lua
python3 tools/mudlet/build.py --check  # CI: the package matches the Lua
```

Mudlet runs Lua 5.1, so keep to it (no `goto`, no `//`). The mapper API it uses is
[Mudlet's](https://wiki.mudlet.org/w/Manual:Mapper_Functions); setting an exit does not
remove a stub in the same direction, hence `silvermere.setExit`.
