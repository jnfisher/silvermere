# Published MegaMud hunting routes

Eight `.mp` route files (six closed loops, two plain paths) as published on megamud.net,
copied here unmodified. `bin/silvermere/tests/m1_walk.rs` walks them over telnet against the
in-process server.

Format: `[description][author]`, `[CODE:Region:Start name]`, `FROMID:TOID:steps:-1:0:item::`,
then `ROOMID:FLAGS:command` per step, ROOMID being the room the command is typed in:
`((title_hash & 0xFFF) << 20) | exit_bits`, `title_hash = sum((i+1) * ord(c))` over the room
title, `exit_bits` two bits per direction (N, S, E, W, NE, NW, SE, SW, U, D from bit 0):
01 open exit, 10 door or gate, 00 none or hidden.

| File | Start room (map:room) | Steps |
|---|---|---|
| `slmsloop.mp` | 1:2307 (Spell Shop) | 198 |
| `slm2loop.mp` | 1:1076 (Slum Street, Crossroads) | 82 |
| `cry1loop.mp` | 1:1028 (Crypt, Lower Level) | 66 |
| `madwloop.mp` | 1:2323 (Alchemy Laboratory, behind a hidden exit) | 201 |
| `narnloop.mp` | 1:2150 (Newhaven, Arena) | 2 |
| `labeloop.mp` | 1:1832 (Labyrinth, Entrance) | 118 |
| `rivpgnol.mp` | 1:2398 (Rocky Passage), a path | 20 |
| `rivpboat.mp` | 1:2398, a path whose step 1 `go passage` is broken in the data | 1 |
