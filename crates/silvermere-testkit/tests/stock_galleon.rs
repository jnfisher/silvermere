//! T-145: the Ancient Galleon end to end, over telnet on the stock world: the Crystal Lake
//! room spells sweep a player with a raft aboard, `jump ship` drops him back in the lake,
//! and Justice Darkbane's `arlysia` takes him back to town.
//!
//! Stock records (map 17): room 580 Arlysia, City Docks, west of it 1124 Crystal Lake, Near
//! Docks; the galleon is rooms 980 to 1008 and 1032, and its Ship Bow (980) carries the
//! `jump ship` command text 9397; 1641 White Forest, Landing, whose `say tree` leads into
//! the White Forest (1642); 1746 Pristine Shrine, Justice Darkbane's room (monster 849);
//! 515 Arlysia, Outer Wall, where his `arlysia` answer leads. Items 690 log raft and 691
//! wooden skiff. The lake rooms cast "sea 1", "sea 2" or "sea 3" (spells 1076 to 1078)
//! every other medium pass: with no boat, 30% nothing and 70% a teleport to a random lake
//! room; with a boat below level 50, half the casts sweep the player aboard; at 50 or
//! above, 2.1% of "sea 1" casts do, no "sea 2" cast and 0.19% of "sea 3" casts, and the
//! rest summon the lake's monsters or do nothing, never drifting him. Three of the rooms a
//! drift or `jump ship` lands in (1012, 1297 and 1641) have no room spell: the lake's spells
//! stop there.
//!
//! The server's own fast tick runs beside the ticks the scenarios send, so the rolls differ
//! from run to run: each scenario holds whatever they are.

use crate::common;

use common::*;

const WARRIOR: u16 = 1;
const LOG_RAFT: i32 = 690;
const DOCKS: u32 = 580;
const NEAR_DOCKS: u32 = 1124;
const SHIP_BOW: u32 = 980;
const LANDING: u32 = 1641;
const SHRINE: u32 = 1746;
const DARKBANE: u16 = 849;
const OUTER_WALL: u32 = 515;
/// Enough hit points to outlast the lake's monsters between two passes.
const HP: i16 = 30000;
const ABOARD: &str = "You suddenly find your self aboard an ancient sea vessel!";

/// The galleon's rooms.
fn on_galleon(room: u32) -> bool {
    (980..=1008).contains(&room) || room == 1032
}

/// The rooms of `rooms` with no room spell: a drift or a jump that lands there ends the
/// lake's spells.
async fn dead_ends(r: &silvermere::Running, rooms: &[u32]) -> Vec<u32> {
    let rooms = rooms.to_vec();
    on_world(r, move |w| {
        let mut dead: Vec<u32> = rooms
            .into_iter()
            .filter(|&n| {
                w.room(silvermere_types::ids::RoomKey::new(17, n))
                    .is_some_and(|rm| rm.roomspell == 0)
            })
            .collect();
        dead.sort_unstable();
        dead.dedup();
        dead
    })
    .await
}

/// The rooms `jump ship` may land in: one `teleport <room> 17` line each of text block 9383,
/// read from the world.
async fn jump_rooms(r: &silvermere::Running) -> Vec<u32> {
    on_world(r, |w| {
        let block = silvermere_types::ids::TextBlockNo(9383);
        let mut rooms = Vec::new();
        for part in 0.. {
            let Some(t) = w.text_blocks.get(&(part, block)) else {
                break;
            };
            for line in t.text.lines() {
                let w: Vec<&str> = line
                    .split(|c: char| c == ':' || c.is_whitespace())
                    .filter(|w| !w.is_empty())
                    .collect();
                if let Some(i) = w.iter().position(|&x| x == "teleport") {
                    rooms.push(w[i + 1].parse().unwrap());
                }
            }
        }
        rooms
    })
    .await
}

/// A Warrior `name` of `level` with [`HP`] hit points, carrying a log raft when `raft`,
/// standing on the docks.
async fn sailor(
    r: &silvermere::Running,
    db: &StockDb,
    name: &str,
    level: u16,
    raft: bool,
) -> silvermere_testkit::TelnetClient {
    let c = new_character(r, &db.url, name, WARRIOR, &[]).await;
    edit_player(r, name, move |p| {
        p.level = level;
        p.maxhp = HP;
        p.hp = HP;
    })
    .await;
    if raft {
        give(r, name, LOG_RAFT, 1).await;
    }
    place(r, name, 17, DOCKS).await;
    c
}

/// Sends `line` (a move, or a room or monster word that moves him) and returns the room he
/// reaches first. The engine takes the line before any tick is sent (he has moved, or a
/// move delay or a queued line shows it), then ticks go one at a time until he has moved. (A room spell may move him on before any
/// prompt: its lines come with none.)
async fn step(
    r: &silvermere::Running,
    c: &mut silvermere_testkit::TelnetClient,
    name: &str,
    line: &str,
) -> u32 {
    let before = room_of(r, name).await;
    c.send_line(line).await.unwrap();
    let who = name.to_owned();
    let mut taken = false;
    for _ in 0..400 {
        pump(c, 5).await;
        let who = who.clone();
        taken = on_world(r, move |w| {
            let s = w.session(session_of(w, &who)).unwrap();
            let p = s.character.as_ref().unwrap();
            p.room.0 != before || p.delay > 0 || !s.ust.pending.is_empty()
        })
        .await;
        if taken {
            break;
        }
    }
    assert!(taken, "`{line}` in {before} was not taken:\n{}", c.plain());
    for _ in 0..200 {
        let now = room_of(r, name).await;
        if now != before {
            pump(c, 50).await;
            return now;
        }
        tick(r, 1).await;
    }
    panic!("`{line}` from {before} did not move:\n{}", c.plain());
}

/// Every monster in `map:room` goes (the player fought them off).
async fn clear_room(r: &silvermere::Running, room: u32) {
    on_world(r, move |w| {
        let gone: Vec<_> = w
            .active_monsters
            .values()
            .filter(|a| a.room.0 == room && a.map.0 == 17)
            .map(|a| a.number)
            .collect();
        for n in gone {
            silvermere_engine::monsters::active::remove_active_monster(w, n, false);
        }
    })
    .await;
}

/// How many monsters stand in `map 17:room`.
async fn monsters_in(r: &silvermere::Running, room: u32) -> usize {
    on_world(r, move |w| {
        w.active_monsters
            .values()
            .filter(|a| a.room.0 == room && a.map.0 == 17)
            .count()
    })
    .await
}

/// The medium passes run so far.
async fn medium_runs(r: &silvermere::Running) -> u64 {
    on_world(r, |w| w.cycles.medium_runs).await
}

/// Runs fast ticks, one at a time, until `name` is in a room `done` accepts or `passes`
/// medium passes have gone by; the monsters the room spells summon are cleared after each
/// pass when `clear`, and his hit points are put back to [`HP`] after each pass (the lake's
/// monsters bite, his maximum is worked out again as the game goes on, and a death would
/// end the scenario in the death room). Returns the rooms `name` passed through (the start first), the
/// passes it took and how many monsters were cleared.
async fn drift(
    r: &silvermere::Running,
    c: &mut silvermere_testkit::TelnetClient,
    name: &str,
    passes: u64,
    clear: bool,
    done: impl Fn(u32) -> bool,
) -> (Vec<u32>, u64, usize) {
    let start = medium_runs(r).await;
    let mut cleared = 0;
    let mut rooms = vec![room_of(r, name).await];
    let mut last = start;
    for n in 0u64.. {
        tick(r, 1).await;
        if n % 16 == 0 {
            pump(c, 1).await;
        }
        let room = room_of(r, name).await;
        if room != *rooms.last().unwrap() {
            rooms.push(room);
        }
        let now = medium_runs(r).await;
        if done(room) || now - start >= passes {
            pump(c, 100).await;
            return (rooms, now - start, cleared);
        }
        if now != last {
            last = now;
            if clear {
                cleared += monsters_in(r, room).await;
                clear_room(r, room).await;
            }
            edit_player(r, name, |p| {
                p.maxhp = HP;
                p.hp = HP;
            })
            .await;
        }
    }
    unreachable!()
}

/// Below level 50 with a log raft, the room spell west of the docks soon sweeps him aboard:
/// half the casts do (text block 9445's last line), the rest drift him to another lake room
/// or do nothing, and a cast comes every other medium pass. A drift into one of the rooms
/// with no room spell ends it there.
#[tokio::test(flavor = "multi_thread")]
async fn a_level_20_sailor_is_swept_aboard() {
    let db = stock_db();
    let r = start(&db.url, 145).await;
    let mut c = sailor(&r, &db, "Wren", 20, true).await;
    let at = c.position();
    let lake = step(&r, &mut c, "Wren", "w").await;
    assert_eq!(lake, NEAR_DOCKS, "{}", c.plain_from(at));
    let dead = dead_ends(&r, &jump_rooms(&r).await).await;
    let end = |n: u32| on_galleon(n) || dead.contains(&n);
    let (rooms, passes, _) = drift(&r, &mut c, "Wren", 40, false, end).await;
    let out = c.plain_from(at);
    let aboard = *rooms.last().unwrap();
    if dead.contains(&aboard) {
        // drifted into a room with no room spell: nothing more happens there
        r.stop().await.unwrap();
        return;
    }
    assert!(
        on_galleon(aboard),
        "not aboard after {passes} passes: {rooms:?}\n{out}"
    );
    assert!(out.contains(ABOARD), "{out}");
    // 9396: the first five rooms of the deck
    assert!((981..=985).contains(&aboard), "{aboard}");
    r.stop().await.unwrap();
}

/// At level 50 or above the Near Docks' "sea 1" sweeps him aboard on 2.1% of casts (text
/// block 9361: 70% go on to 9388, whose 50-53 line is the sweep); the rest do nothing or
/// summon crimson mists or severed heads, and while they stand in the room the next casts
/// do nothing (`nomonsters`). Nothing at this level drifts him to another lake room. The
/// summoned monsters are cleared after each pass, as if fought off.
#[tokio::test(flavor = "multi_thread")]
async fn a_level_60_sailor_reaches_the_galleon_in_the_end() {
    let db = stock_db();
    let r = start(&db.url, 1460).await;
    let mut c = sailor(&r, &db, "Osric", 60, true).await;
    assert_eq!(step(&r, &mut c, "Osric", "w").await, NEAR_DOCKS);
    let at = c.position();
    let (rooms, passes, cleared) = drift(&r, &mut c, "Osric", 2000, true, on_galleon).await;
    let out = c.plain_from(at);
    assert!(
        rooms.len() == 2 && on_galleon(rooms[1]),
        "straight from the Near Docks to the deck, after {passes} passes: {rooms:?}"
    );
    assert!(out.contains(ABOARD), "{out}");
    // the lake's monsters come with their lines, unless he was swept aboard first
    let lines = out.matches("A cruel wind blows in").count()
        + out.matches("A horde of severed heads").count();
    assert_eq!(
        lines > 0,
        cleared > 0,
        "{lines} lines, {cleared} monsters:\n{out}"
    );
    r.stop().await.unwrap();
}

/// With no raft or skiff, a lake room's spell runs 9357: 30% nothing, 70% a teleport to one
/// of 9383's rooms; never the galleon.
#[tokio::test(flavor = "multi_thread")]
async fn without_a_boat_the_lake_drifts_him() {
    let db = stock_db();
    let r = start(&db.url, 1451).await;
    let mut c = sailor(&r, &db, "Pell", 20, false).await;
    assert_eq!(step(&r, &mut c, "Pell", "w").await, NEAR_DOCKS);
    let lake = jump_rooms(&r).await;
    let dead = dead_ends(&r, &lake).await;
    let (rooms, passes, _) = drift(&r, &mut c, "Pell", 40, false, |n| dead.contains(&n)).await;
    assert!(
        rooms.len() > 2 || dead.contains(rooms.last().unwrap()),
        "drifted in {passes} passes: {rooms:?}"
    );
    for room in &rooms[1..] {
        assert!(
            lake.contains(room),
            "{room} is not one of 9383's: {rooms:?}"
        );
    }
    r.stop().await.unwrap();
}

/// `jump ship` on the Ship Bow: message 2941, then one of 9383's rooms; there the next room
/// spell takes over (a lake room's), unless he landed in a room with none.
#[tokio::test(flavor = "multi_thread")]
async fn jump_ship_lands_in_the_lake() {
    let db = stock_db();
    let r = start(&db.url, 1452).await;
    let mut c = sailor(&r, &db, "Tamsin", 20, true).await;
    place(&r, "Tamsin", 17, SHIP_BOW).await;
    let lake = jump_rooms(&r).await;
    let at = c.position();
    let landed = step(&r, &mut c, "Tamsin", "jump ship").await;
    pump(&mut c, 200).await;
    let out = c.plain_from(at);
    assert!(
        out.contains("You climb down the mast of the ship."),
        "{out}"
    );
    assert!(lake.contains(&landed), "{landed}:\n{out}");
    let spell = on_world(&r, move |w| {
        w.room(silvermere_types::ids::RoomKey::new(17, landed))
            .map_or(0, |rm| rm.roomspell)
    })
    .await;
    let (rooms, passes, _) = drift(&r, &mut c, "Tamsin", 40, false, |n| n != landed).await;
    if spell == 0 {
        assert_eq!(rooms, [landed], "no room spell in {landed}");
    } else {
        assert!(
            rooms.len() == 2,
            "{landed}'s spell in {passes} passes: {rooms:?}"
        );
        let next = rooms[1];
        assert!(on_galleon(next) || lake.contains(&next), "{rooms:?}");
    }
    r.stop().await.unwrap();
}

/// `jump ship` with a monster in the room: message 503 and nothing else.
#[tokio::test(flavor = "multi_thread")]
async fn jump_ship_with_a_monster_aboard_is_refused() {
    let db = stock_db();
    let r = start(&db.url, 1453).await;
    let mut c = sailor(&r, &db, "Ulric", 20, true).await;
    place(&r, "Ulric", 17, SHIP_BOW).await;
    spawn(&r, 17, SHIP_BOW, 904).await;
    assert_eq!(monsters_in(&r, SHIP_BOW).await, 1);
    pump(&mut c, 200).await;
    let out = cmd(&mut c, "jump ship").await;
    assert_eq!(room_of(&r, "Ulric").await, SHIP_BOW, "{out}");
    let reply: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "jump ship" && !l.starts_with("[HP="))
        .collect();
    assert_eq!(
        reply,
        ["You cannot do that right now!"],
        "the whole reply:\n{out}"
    );
    r.stop().await.unwrap();
}

/// From "White Forest, Landing" (one of 9383's rooms) `say tree` leads into the White
/// Forest; Justice Darkbane, in the Pristine Shrine deep in it, answers `arlysia` by
/// sending him to Arlysia's Outer Wall.
#[tokio::test(flavor = "multi_thread")]
async fn the_landing_and_justice_darkbane() {
    let db = stock_db();
    let r = start(&db.url, 1454).await;
    let mut c = sailor(&r, &db, "Vala", 60, false).await;
    assert!(jump_rooms(&r).await.contains(&LANDING));
    place(&r, "Vala", 17, LANDING).await;
    let at = c.position();
    let forest = step(&r, &mut c, "Vala", "say tree").await;
    assert_eq!(forest, 1642, "{}", c.plain_from(at));
    // the walk on from 1642 (35 moves, two with room words) is the White Forest's, not the
    // galleon's; he is put in the shrine
    place(&r, "Vala", 17, SHRINE).await;
    spawn(&r, 17, SHRINE, DARKBANE).await;
    let at = c.position();
    let town = step(&r, &mut c, "Vala", "ask darkbane arlysia").await;
    pump(&mut c, 200).await;
    let out = c.plain_from(at);
    assert_eq!(town, OUTER_WALL, "{out}");
    assert!(out.contains("The ghost bows to you"), "{out}");
    r.stop().await.unwrap();
}

/// What one lake room spell did to him.
#[derive(Debug, Default)]
struct Odds {
    aboard: u32,
    drifted: u32,
    summoned: u32,
    stayed: u32,
}

/// `casts` casts of `spell` on `name` standing in the first room of map 17 that has it,
/// with the room cleared of monsters and him put back before each: what each did.
async fn cast_odds(r: &silvermere::Running, name: &str, spell: i32, casts: u32) -> Odds {
    let name = name.to_owned();
    on_world(r, move |w| {
        use silvermere_types::ids::{RoomKey, SpellNo};
        let mut lake: Vec<u32> = w
            .rooms
            .iter()
            .filter(|(k, rm)| k.map.0 == 17 && rm.roomspell == spell)
            .map(|(k, _)| k.room.0)
            .collect();
        lake.sort_unstable();
        let home = RoomKey::new(17, lake[0]);
        let id = session_of(w, &name);
        let mut odds = Odds::default();
        for _ in 0..casts {
            let gone: Vec<_> = w
                .active_monsters
                .values()
                .filter(|a| a.room.0 == home.room.0 && a.map.0 == 17)
                .map(|a| a.number)
                .collect();
            for n in gone {
                silvermere_engine::monsters::active::remove_active_monster(w, n, false);
            }
            silvermere_engine::movement::move_user::set_user_location(w, id, home, false);
            silvermere_engine::spells::room::room_cast_on_user(w, id, SpellNo(spell as _));
            let room = w.session(id).unwrap().character.as_ref().unwrap().room.0;
            let here = w
                .active_monsters
                .values()
                .any(|a| a.room.0 == home.room.0 && a.map.0 == 17);
            if on_galleon(room) {
                odds.aboard += 1;
            } else if room != home.room.0 {
                odds.drifted += 1;
            } else if here {
                odds.summoned += 1;
            } else {
                odds.stayed += 1;
            }
            // what he was told is not read here
            w.session_mut(id).unwrap().output.clear();
        }
        odds
    })
    .await
}

/// `got` of `n` is within four standard deviations of `p`.
fn near(got: u32, n: u32, p: f64) -> bool {
    let (n, got) = (f64::from(n), f64::from(got));
    let sd = (n * p * (1.0 - p)).sqrt().max(1.0);
    (got - n * p).abs() <= 4.0 * sd
}

/// The odds of one cast of each lake spell, from text blocks 9357 to 9445 (see the module
/// docs), over 4,000 seeded casts each: no boat, a boat below level 50, and a boat at 50
/// or above on each of "sea 1", "sea 2" and "sea 3": how often he is swept aboard or
/// drifted, and (`summons`) how often the lake's monsters come.
async fn odds_of_one_cast(seed: u64, summons: bool) {
    const N: u32 = 4000;
    let db = stock_db();
    let r = start(&db.url, seed).await;
    let _c = sailor(&r, &db, "Yorick", 20, false).await;
    // no boat: 70% drift (9357), at any level
    let o = cast_odds(&r, "Yorick", 1076, N).await;
    assert!(o.aboard == 0 && near(o.drifted, N, 0.70), "{o:?}");
    give(&r, "Yorick", LOG_RAFT, 1).await;
    // below 50 (9445): 50% aboard, 25% x 70% drift (9357), and a quarter of "sea 3"'s
    // odds at 50 and above (9363): its 0.19% more aboard, its 43.6% summons
    for spell in [1076, 1077, 1078] {
        let o = cast_odds(&r, "Yorick", spell, N).await;
        assert!(near(o.aboard, N, 0.5 + 0.25 * 0.00188), "{spell}: {o:?}");
        assert!(near(o.drifted, N, 0.175), "{spell}: {o:?}");
        if summons {
            assert!(near(o.summoned, N, 0.25 * 0.436), "{spell}: {o:?}");
        }
    }
    edit_player(&r, "Yorick", |p| p.level = 60).await;
    // 50 and above: never drifted; aboard 70% x 3% on "sea 1" (9361, 9388), never on
    // "sea 2" (9362, 9389), 0.19% on "sea 3" (9363 to 9390, 9390's 3% to 9394, then 9394's
    // own sweep or its way back through 9390 or 9388)
    for (spell, aboard, summoned) in [
        (1076, 0.021, 0.7 * 0.47),
        (1077, 0.0, 0.6 * 0.6),
        (1078, 0.00188, 0.436),
    ] {
        let o = cast_odds(&r, "Yorick", spell, N).await;
        assert_eq!(o.drifted, 0, "{spell}: {o:?}");
        assert!(near(o.aboard, N, aboard), "{spell}: {o:?}");
        if summons {
            assert!(near(o.summoned, N, summoned), "{spell}: {o:?}");
        }
    }
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_odds_of_one_cast() {
    odds_of_one_cast(1455, false).await;
}

/// The lake's `summon` lines bring their monsters whatever the room's spawn rules: two of
/// the three rooms that control the lake (1010 and 1416) allow no monsters at all (area
/// maximum 0), so in their rooms a summon that keeps the rules brings none.
#[tokio::test(flavor = "multi_thread")]
async fn the_lake_summons_its_monsters() {
    odds_of_one_cast(1456, true).await;
}
