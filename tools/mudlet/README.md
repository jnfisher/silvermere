# Mudlet mapper package

`silvermere-mapper.lua` builds Mudlet's map from Silvermere's GMCP `Room.Info`
(players' guide: [docs/clients.md](../../docs/clients.md#mudlet); the GMCP messages:
[docs/net.md](../../docs/net.md#gmcp)). Both packages are generated from the Lua:

- `silvermere-mapper.mpackage`, the one players install: the script plus a `config.lua`
  with the title, author, version and description Mudlet's Package Manager shows;
- `silvermere-mapper.xml`, the bare script.

```
python3 tools/mudlet/build.py          # after editing the Lua
python3 tools/mudlet/build.py --check  # CI: the packages match the Lua
```

Bump `M.version` and `M.released` at the top of the Lua with every change, so players can
tell which one they have. The rest of the details (title, description) are in `build.py`.

Mudlet runs Lua 5.1, so keep to it (no `goto`, no `//`). The mapper API it uses is
[Mudlet's](https://wiki.mudlet.org/w/Manual:Mapper_Functions); setting an exit does not
remove a stub in the same direction, hence `silvermere.setExit`.
