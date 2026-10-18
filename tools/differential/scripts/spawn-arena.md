# Scenario: spawn-arena

Newhaven Arena (1:2150): log in, walk down into the arena, stand there five minutes, look,
go back up and log off. Every `monster_arrives` event on each side is a spawn
(`generate_monsters_near_users`, `generate_monster`).

| | |
|---|---|
| Written by | T-035, for T-040's spawn-rate comparison (layer 4) |
| Captured | not yet: the first differential run records both sides live |
| Character | user Aldric (the `creation-fsd` setup), level 1 |

Expected on both sides: no more monsters in the arena than players (one), the first
arrival within a few passes of entering (each 5 s pass spawns on 89 rolls of 99 while the
arena is empty), arrival lines matching `monster_arrives` / `monster_arrives_generic`.
