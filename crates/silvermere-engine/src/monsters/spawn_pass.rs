//! [`generate_monsters_near_users`]: the 5-second spawn pass, step by step.
//!
//! 1. The 512-entry occupied-room table is cleared (room numbers only).
//! 2. Pass 1, players from `gennextusr` in channel order: each player's room is looked up
//!    in the table by room number only (sic); an Arena or Normal room not yet counted
//!    gets its monsters and players counted and an entry; then a roll of 1-100 against 90
//!    (Arena) or 5 (Normal) while the players outnumber the monsters. A Lair is filled
//!    while `generate_monster` succeeds.
//! 3. Pass 2, the occupied rooms from `gennextocc`: each exit with about 5% (a roll of
//!    1-100 below 6) looks at the room beyond, loaded with the map of the last player of
//!    pass 1 (sic), and spawns one there while it has fewer monsters than the
//!    occupied room has players, then tries no more exits of that room; Lairs fill.
//! 4. A complete run starts both passes from the beginning next time.
//!
//! The pass returns once `made > MAXGENERATE` (the ninth monster), remembering where to
//! resume. A Lair's fill only stops at the cap and does not return: the generation comes
//! before the `made` test, so a fill at the cap still makes one more monster, and every
//! later Lair of the pass makes one each.

use silvermere_rules::spawn::{self, MAXGENERATE, NOCCROOMS};
use silvermere_types::enums::RoomType;
use silvermere_types::ids::{MapId, RoomId, RoomKey, SessionId};
use silvermere_types::sizes::{NDIRS, NROOMMONS};

use super::generate::{Request, generate_monster};
use crate::scheduler::Rng;
use crate::world::{OccRoom, World};

/// A monster generated in the room `at` from the spawn fields (group, specific monster,
/// level range) of `fields` (a room record).
fn spawn_one(world: &mut World, at: RoomKey, fields: RoomKey) -> bool {
    let Some(req) = Request::for_room(world, fields) else {
        return false;
    };
    generate_monster(world, at, req).is_some()
}

fn room_type(world: &World, key: RoomKey) -> Option<RoomType> {
    world.room(key).map(|r| r.room_type)
}

fn monsters_in(world: &World, key: RoomKey) -> u32 {
    world.room_state.get(&key).map_or(0, |s| {
        s.monsters
            .iter()
            .take(NROOMMONS)
            .filter(|m| m.0 != 0)
            .count() as u32
    })
}

/// The 5-second spawn pass.
pub fn generate_monsters_near_users(world: &mut World) {
    let mut made: u32 = 0;
    if world.occupied.len() != NOCCROOMS {
        world.occupied.resize(NOCCROOMS, OccRoom::default());
    }
    // empty the table: a zero room number marks a free entry
    for e in &mut world.occupied {
        e.room.room = RoomId(0);
    }
    let nterms = world.ids.max();
    let nocc = usize::from(nterms).min(NOCCROOMS);
    // the room record `rm` holds; it survives from one player to the next
    let mut rm: Option<RoomKey> = None;
    // --- pass 1 -----------------------------------------------------------------------
    let mut i = world.spawn.gennextusr;
    while i < nterms {
        let id = SessionId(i);
        let player = world
            .is_in_game(id)
            .then(|| {
                world
                    .session(id)
                    .and_then(|s| s.player())
                    .map(|p| (p.room, p.map))
            })
            .flatten();
        let Some((roomnum, mapnum)) = player else {
            i += 1;
            continue;
        };
        world.spawn.last_map = Some(mapnum);
        let here = RoomKey {
            map: mapnum,
            room: roomnum,
        };
        let mut nmons: u32 = 0;
        let mut nplayers: u32 = 0;
        let mut slot = 0;
        // (sic) the room number alone is compared, and the search goes on past
        // a match to the first free entry
        for j in 0..nocc {
            let e = world.occupied[j];
            if e.room.room == roomnum {
                nmons = u32::from(e.nmons);
                nplayers = u32::from(e.nplayers);
                rm = world.room(e.room).map(|_| e.room);
                slot = j;
            } else if e.room.room.0 == 0 {
                rm = world.room(here).map(|_| here);
                slot = j;
                break;
            }
        }
        let rtype = rm.and_then(|k| room_type(world, k));
        match (rm, rtype) {
            (Some(rmk), Some(t @ (RoomType::Arena | RoomType::Normal))) => {
                // a room not counted yet
                if nplayers == 0 {
                    nmons += monsters_in(world, rmk);
                    nplayers = world
                        .session_ids()
                        .into_iter()
                        .filter(|k| world.is_in_game(*k))
                        .filter_map(|k| world.session(k).and_then(|s| s.player()))
                        .filter(|p| p.map == mapnum && p.room == roomnum)
                        .count() as u32;
                    world.occupied[slot] = OccRoom {
                        room: here,
                        nplayers: nplayers.min(255) as u8,
                        nmons: nmons.min(255) as u8,
                    };
                }
                let roll = world.rng.range(1, 100);
                if spawn::pass1_spawns(nmons, nplayers, spawn::spawn_chance(t), roll)
                    && spawn_one(world, here, rmk)
                {
                    made += 1;
                    if made > MAXGENERATE {
                        world.spawn.gennextusr = i;
                        return;
                    }
                }
            }
            (Some(rmk), Some(RoomType::Lair)) => {
                // generate first, then test the count (sic)
                while spawn_one(world, here, rmk) && made <= MAXGENERATE {
                    made += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    world.spawn.gennextusr = i;
    // --- pass 2 -----------------------------------------------------------------------
    let mapnum = world.spawn.last_map.unwrap_or(MapId(1));
    let mut rm: Option<RoomKey> = None;
    let mut i = world.spawn.gennextocc;
    while i < nocc {
        let occ = world.occupied[i];
        if occ.room.room.0 == 0 {
            i += 1;
            continue;
        }
        let mut k = 0;
        while k < NDIRS {
            // (re)load the occupied room
            if rm != Some(occ.room) {
                rm = world.room(occ.room).map(|_| occ.room);
            }
            let Some(rmk) = rm else {
                k += 1;
                continue;
            };
            let Some(roomnum) = world.room(rmk).map(|r| r.exits[k].target) else {
                k += 1;
                continue;
            };
            if roomnum.0 == 0 {
                k += 1;
                continue;
            }
            // about 5% per exit (rolls 1-5)
            if world.rng.range(1, 100) >= 6 {
                k += 1;
                continue;
            }
            // (sic) a match only marks the neighbour as counted until a free
            // entry turns up after it, which then becomes the slot: with free entries
            // left, a neighbour in the table is not skipped
            let mut slot: i32 = 1000;
            for j in 0..NOCCROOMS {
                let e = world.occupied[j].room.room;
                if e.0 == 0 {
                    slot = j as i32;
                    break;
                } else if e == roomnum {
                    slot = -1;
                }
            }
            if slot != -1 {
                // sic: the map of the last player of pass 1
                let nkey = RoomKey {
                    map: mapnum,
                    room: roomnum,
                };
                rm = world.room(nkey).map(|_| nkey);
                match rm.and_then(|n| room_type(world, n)) {
                    Some(RoomType::Arena | RoomType::Normal) => {
                        let cnt = monsters_in(world, nkey);
                        if u32::from(occ.nplayers) > cnt {
                            // the last exit tried for this room
                            k = NDIRS;
                            if spawn_one(world, nkey, nkey) {
                                made += 1;
                                if made > MAXGENERATE {
                                    world.spawn.gennextocc = i;
                                    return;
                                }
                            }
                        }
                    }
                    Some(RoomType::Lair) => {
                        while spawn_one(world, nkey, nkey) && made <= MAXGENERATE {
                            made += 1;
                        }
                    }
                    _ => {}
                }
                // the neighbour counts as handled; the entry keeps the counts
                // of its last use
                if slot != 1000 {
                    world.occupied[slot as usize].room = nkey;
                }
            }
            k += 1;
        }
        i += 1;
    }
    world.spawn.gennextusr = 0;
    world.spawn.gennextocc = 0;
}
