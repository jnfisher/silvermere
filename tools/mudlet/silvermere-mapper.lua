-- Silvermere mapper for Mudlet: draws the map from the server's GMCP `Room.Info`.
--
-- Mudlet builds maps from GMCP by itself only for a few games it knows; for others it
-- installs a "generic mapper" that guesses rooms from the text. This script does the GMCP
-- part for Silvermere instead. Don't also run the generic mapper's `start mapping`.
--
-- What it does each time `Room.Info` arrives (each room display, so moving and LOOK):
--   * creates the room the first time (id = the server's `num`, map * 1000000 + room), in
--     an area named after the server's `area` ("Map 1", ...), placed one step from a
--     neighbour already on the map in the direction of the exit between them;
--   * links its exits to rooms already mapped, both ways, and leaves stubs for the rest;
--   * keeps the room's exits in its user data, so a room found later can be linked back;
--   * colours shops, hotels and the like, and centres the map on the room.
--
-- Mudlet runs Lua 5.1: no `goto`, no `//`.

silvermere = silvermere or {}
local M = silvermere

-- The package's version and date, which build.py copies into the package's details: bump
-- both with every change, so the Package Manager shows which one a player has.
M.version = "1.0.0"
M.released = "2026-11-25"

-- Mudlet's exit numbers and the step on the grid for each GMCP direction.
M.dirs = {
  n  = { id = 1,  dx = 0,  dy = 1,  dz = 0 },
  ne = { id = 2,  dx = 1,  dy = 1,  dz = 0 },
  nw = { id = 3,  dx = -1, dy = 1,  dz = 0 },
  e  = { id = 4,  dx = 1,  dy = 0,  dz = 0 },
  w  = { id = 5,  dx = -1, dy = 0,  dz = 0 },
  s  = { id = 6,  dx = 0,  dy = -1, dz = 0 },
  se = { id = 7,  dx = 1,  dy = -1, dz = 0 },
  sw = { id = 8,  dx = -1, dy = -1, dz = 0 },
  u  = { id = 9,  dx = 0,  dy = 0,  dz = 1 },
  d  = { id = 10, dx = 0,  dy = 0,  dz = -1 },
}

-- Room colours by GMCP `environment` (custom environment ids start above Mudlet's 16).
M.envs = {
  shop      = { id = 301, rgb = { 220, 180, 40 } },
  hotel     = { id = 302, rgb = { 80, 160, 255 } },
  library   = { id = 303, rgb = { 170, 120, 255 } },
  arena     = { id = 304, rgb = { 230, 60, 60 } },
  colliseum = { id = 304, rgb = { 230, 60, 60 } },
  lair      = { id = 305, rgb = { 150, 40, 40 } },
  jail      = { id = 306, rgb = { 120, 120, 120 } },
}

local EXITS_KEY = "silvermere.exits"

-- The area's id, made the first time its name is seen.
function M.area(name)
  local id = getAreaTable()[name]
  if id then
    return id
  end
  return addAreaName(name)
end

-- The exits a room had when it was last seen: { dir = num }.
function M.storedExits(num)
  local text = getRoomUserData(num, EXITS_KEY)
  if not text or text == "" then
    return {}
  end
  local ok, value = pcall(yajl.to_value, text)
  if ok and type(value) == "table" then
    return value
  end
  return {}
end

-- Where a new room goes: one step from a mapped neighbour in the same area, either by the
-- neighbour's exit to it or by its own exit to the neighbour; else the area's origin.
function M.place(num, areaId, exits)
  for dir, target in pairs(exits) do
    local d = M.dirs[dir]
    target = tonumber(target)
    if d and target and roomExists(target) and getRoomArea(target) == areaId then
      local x, y, z = getRoomCoordinates(target)
      return x - d.dx, y - d.dy, z - d.dz
    end
  end
  return 0, 0, 0
end

-- An exit from `from` to `to`; a stub there goes (Mudlet keeps it otherwise).
function M.setExit(from, to, dirId)
  setExitStub(from, dirId, false)
  setExit(from, to, dirId)
end

-- Links `num`'s exits that lead to mapped rooms, and the mapped rooms' exits back to it.
function M.link(num, exits)
  for dir, target in pairs(exits) do
    local d = M.dirs[dir]
    target = tonumber(target)
    if d and target then
      if roomExists(target) then
        M.setExit(num, target, d.id)
        for backDir, backTarget in pairs(M.storedExits(target)) do
          local b = M.dirs[backDir]
          if b and tonumber(backTarget) == num then
            M.setExit(target, num, b.id)
          end
        end
      else
        setExitStub(num, d.id, true)
      end
    end
  end
end

function M.onRoomInfo()
  local info = gmcp and gmcp.Room and gmcp.Room.Info
  if not info then
    return
  end
  local num = tonumber(info.num)
  if not num then
    return
  end
  local exits = info.exits or {}
  local areaId = M.area(info.area or "Silvermere")
  if not roomExists(num) then
    addRoom(num)
    setRoomArea(num, areaId)
    -- placed before linking, so the neighbour found is the one it is drawn next to
    local x, y, z = M.place(num, areaId, exits)
    setRoomCoordinates(num, x, y, z)
  end
  if info.name then
    setRoomName(num, info.name)
  end
  local env = M.envs[info.environment or ""]
  if env then
    setCustomEnvColor(env.id, env.rgb[1], env.rgb[2], env.rgb[3], 255)
    setRoomEnv(num, env.id)
  end
  setRoomUserData(num, EXITS_KEY, yajl.to_string(exits))
  M.link(num, exits)
  M.current = num
  centerview(num)
end

if M.handler then
  killAnonymousEventHandler(M.handler)
end
M.handler = registerAnonymousEventHandler("gmcp.Room.Info", function()
  silvermere.onRoomInfo()
end)
