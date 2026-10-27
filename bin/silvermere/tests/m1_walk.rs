//! M1 ("walk") integration tests (T-029): scripted sessions against the stock world, byte-
//! compared with the T-005 captures of the original after normalisation
//! (`silvermere_testkit::capture_compare`). The scenario files shared with the differential
//! harness live in `tools/differential/scenarios/m1/`.

use crate::common;

use std::time::Duration;

use common::stock::{
    WAIT, aldric_db, fresh_db, login_to_menu, on_world, start, start_ticked_by_test, start_with,
    walker_ticker,
};
use silvermere_testkit::capture_compare::{assert_clean, assert_matches_capture, replay_steps};
use silvermere_testkit::{Capture, TelnetClient, captures};

fn capture(name: &str) -> Capture {
    Capture::load(captures::captures_dir().join(name)).expect("capture")
}

/// The `X\b` direction bytes and the Enter redisplay: login -> menu -> `E` -> banner ->
/// room -> `[HP=` glued to the next line; the bytes after `E` and after Enter equal
/// `entry-existing` once the build stamp and direction bytes are masked.
#[tokio::test]
async fn entry_existing_matches_capture() {
    let (_dir, url) = aldric_db();
    let cap = capture("entry-existing");
    let r = start(&url).await;
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    let at = c.position();
    c.send_line("E").await.unwrap();
    c.until(r"\[HP=-?\d+[^\]\n]*\]:", WAIT).await.unwrap();
    let entered = c.raw()[at..].to_vec();
    assert_matches_capture("E into the realm", &cap, 4, &entered, &[]);
    let at = c.position();
    c.send_line("").await.unwrap();
    c.until(r"\[HP=-?\d+[^\]\n]*\]:", WAIT).await.unwrap();
    let again = c.raw()[at..].to_vec();
    assert_matches_capture("Enter redisplay", &cap, 5, &again, &[]);
    r.stop().await.unwrap();
}

/// Logs Aldric in and enters the realm; returns the client and the running server.
async fn in_realm(url: &str) -> (TelnetClient, silvermere::Running) {
    let r = start(url).await;
    in_realm_on(r).await
}

/// [`in_realm`] on a server whose ticks come from [`walker_ticker`] alone.
async fn in_realm_ticked_by_test(url: &str) -> (TelnetClient, silvermere::Running) {
    in_realm_on(start_ticked_by_test(url).await).await
}

async fn in_realm_on(r: silvermere::Running) -> (TelnetClient, silvermere::Running) {
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    c.send_line("E").await.unwrap();
    c.until(r"\[HP=-?\d+[^\]\n]*\]:", WAIT).await.unwrap();
    (c, r)
}

/// `st`/`exp`/`i`/`set`/`keys` (`status-screens`): each screen equals
/// the capture's bytes after masking.
#[tokio::test]
async fn status_exp_inventory_set_match_captures() {
    let (_dir, url) = aldric_db();
    let cap = capture("status-screens");
    let (mut c, r) = in_realm(&url).await;
    // The capture was recorded after the `shop` scenario had dressed Aldric: padded armour
    // and a quarterstaff, bought and worn with the commands below.
    on_world(&r, |w| {
        use silvermere_engine::items::inventory::add_item_to_inventory;
        for id in w.session_ids() {
            if w.session(id).is_some_and(|s| s.character.is_some()) {
                for n in [332, 333, 334, 335, 336, 100] {
                    assert!(add_item_to_inventory(
                        w,
                        id,
                        silvermere_types::ids::ItemNo(n),
                        -1
                    ));
                }
            }
        }
    })
    .await;
    for line in [
        "wear padded vest",
        "wear padded pants",
        "wear padded helm",
        "wear padded gloves",
        "wear padded boots",
        "wield quarterstaff",
    ] {
        c.send_line(line).await.unwrap();
        c.until(&format!(r"(?s){line}\s.*\[HP="), WAIT)
            .await
            .unwrap();
    }
    // st, exp, i, set, spells, health, wealth, keys
    let reports = replay_steps(&mut c, &cap, 5..13, &[], WAIT).await;
    assert_clean("status-screens", &reports);
    r.stop().await.unwrap();
}

/// Movement (`move_user`) and the Enter redisplay: every room screen of the
/// `room-walk` capture (Newhaven Arena, dark slum rooms, `NONE!!!`, trap door, locked door,
/// the skiff, duplicate titles) is byte-equal to the original after masking.
#[tokio::test]
async fn room_screens_match_captures() {
    let (_dir, url) = aldric_db();
    let cap = capture("room-walk");
    let (mut c, r) = in_realm_ticked_by_test(&url).await;
    let ticker = walker_ticker(&r);
    for _ in 0..2 {
        c.send_line("w").await.unwrap();
        c.until(r"\[HP=-?\d+[^\]\n]*\]:", WAIT).await.unwrap();
    }
    assert!(c.plain().contains("Newhaven, Narrow Road"));
    let reports = replay_steps(&mut c, &cap, 5..29, &[], WAIT).await;
    ticker.abort();
    assert_clean("room-walk", &reports);
    r.stop().await.unwrap();
}

/// Text exits: `borrow skiff` (step 22 of `room-walk`) crosses the water
/// and the Action exit `go manhole` (step 36) comes back; both screens equal the capture.
#[tokio::test]
async fn text_exit_borrow_skiff() {
    let (_dir, url) = aldric_db();
    let cap = capture("room-walk");
    // the moves' ticks come from the test, as in `room_screens_match_captures`, instead of
    // waiting a second of the server's own for each
    let (mut c, r) = in_realm_ticked_by_test(&url).await;
    let ticker = walker_ticker(&r);
    for _ in 0..2 {
        c.send_line("w").await.unwrap();
        c.until(PROMPT, WAIT).await.unwrap();
    }
    assert!(c.plain().contains("Newhaven, Narrow Road"));
    // steps 5-22 reach the docks and cross (the capture goes on south at step 23); from step 29 on the capture holds a wandering
    // guardsman's line that splits the output, and monsters do not exist before T-035
    let reports = replay_steps(&mut c, &cap, 5..23, &[], WAIT).await;
    assert_clean("room-walk to the Small Pier", &reports);
    assert!(
        c.plain()
            .contains("You climb into one of the skiffs, and row to Silvermere.")
    );
    // Action exit: north from the Small Pier hops back to Newhaven
    let at = c.position();
    c.send_line("n").await.unwrap();
    c.until(PROMPT, WAIT).await.unwrap();
    let back = c.plain_from(at);
    assert!(back.contains("You hop in one of the skiffs"), "{back}");
    assert!(back.contains("Newhaven, Docks"), "{back}");
    ticker.abort();
    r.stop().await.unwrap();
}

const PROMPT: &str = r"\[HP=-?\d+[^\]\n]*\]:";

/// After a dropped connection in the realm the next entry prints the two
/// punishment lines, in order, before the room (`disconnect-punish` steps 12-13).
#[tokio::test]
async fn entry_after_disconnect_shows_punishment() {
    let (_dir, url) = aldric_db();
    let cap = capture("disconnect-punish");
    let (mut c, r) = in_realm(&url).await;
    c.disconnect().await;
    // the engine sees the hang-up and saves the character with the HUNGUP mark
    let db = silvermere_db::Db::connect_and_migrate(&url).await.unwrap();
    loop {
        let row = db
            .characters()
            .by_first_name("Aldric")
            .await
            .unwrap()
            .unwrap();
        if row
            .character
            .status
            .contains(silvermere_types::flags::PlayerStatus::HUNGUP)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    db.close().await;
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    let at = c.position();
    c.send_line("E").await.unwrap();
    c.until(PROMPT, WAIT).await.unwrap();
    let text = c.plain_from(at);
    let a = text
        .find("Last time you were on, you disconnected while playing.")
        .expect("first punishment line");
    let b = text
        .find("The gods have punished you appropriately.")
        .expect("second punishment line");
    assert!(a < b);
    assert_matches_capture("entry after the drop", &cap, 12, &c.raw()[at..], &[]);
    r.stop().await.unwrap();
}

/// A given name that parses as a command (`sysop`) bounces back with
/// the "already in use" text and creates no character.
#[tokio::test]
async fn creation_first_name_command_bounces() {
    let (_dir, url) = fresh_db();
    common::stock::add_account(&url, "Sam", "sam123").await;
    let r = start(&url).await;
    let mut c = TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    login_to_menu(&mut c, "Sam", "sam123").await;
    for (line, next) in [
        ("E", "choose your race"),
        ("1", "choose your class"),
        ("1", r"Lawful\?"),
        ("N", "Given name"),
        ("sysop", "Family name"),
        ("Test", "Strength"),
        ("", "Intellect"),
        ("", "Willpower"),
        ("", "Agility"),
        ("", "Health"),
        ("", "Charm"),
        ("", "Hair length"),
        ("", "Hair colour"),
        ("", "Eye colour"),
        ("", r"SAVE or QUIT"),
        ("", "The name you have chosen is already in use!"),
    ] {
        c.send_line(line).await.unwrap();
        c.until(next, WAIT).await.unwrap();
    }
    assert!(c.plain().contains("You will have to start over again."));
    r.stop().await.unwrap();
    let db = silvermere_db::Db::connect_and_migrate(&url).await.unwrap();
    assert!(
        db.characters()
            .by_first_name("Sysop")
            .await
            .unwrap()
            .is_none()
    );
    db.close().await;
}

/// A burst of 15 lines in one write is warned at the 8th to 12th and
/// the rest dropped (`flood-and-refusals`: five warnings, then "command ignored").
///
/// The queue only fills while `search`'s delay runs, and the delay counts down on fast ticks.
/// With the server's own tick, a loaded runner could let the delay run out before the engine
/// had taken the rest of the burst, and the lines ran as they came, unwarned. So the ticks
/// come from the test, and only once the last line has been dropped.
#[tokio::test]
async fn flood_control_warns_at_8_drops_at_12() {
    let (_dir, url) = aldric_db();
    let (mut c, r) = in_realm_ticked_by_test(&url).await;
    let burst = "search\r\n".to_owned() + &"exp\r\n".repeat(14);
    let at = c.position();
    c.send_raw(burst.as_bytes()).await.unwrap();
    c.until("command ignored", WAIT).await.unwrap();
    let ticker = walker_ticker(&r);
    c.until("Exp needed for next level", WAIT).await.unwrap();
    ticker.abort();
    let text = c.plain_from(at);
    let warns = text
        .matches("Why don't you slow down for a few seconds?")
        .count();
    assert_eq!(warns, 5, "{text}");
    assert_eq!(
        text.matches("You are typing too quickly - command ignored")
            .count(),
        1
    );
    // The host handled one line before it echoed the next: `search` set the delay (echo
    // off), so the lines typed ahead are shown when they run, after its result and the
    // warnings, not all at once ahead of them (capture: `search`, the result, the
    // warnings, then `exp`).
    let result = text
        .find("Your search revealed nothing.")
        .expect("search result");
    let first_exp_echo = text.find("\nexp\n").expect("an exp echo");
    assert!(result < first_exp_echo, "{text}");
    r.stop().await.unwrap();
}

/// Inactivity: marked after one period, hung up after the
/// second (two strikes), driven by `Tick` inputs rather than waiting.
#[tokio::test]
async fn inactivity_two_strikes() {
    let (_dir, url) = aldric_db();
    let r = start_with(&url, |cfg| cfg.game.inactive_secs = 60).await;
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    for _ in 0..60 {
        r.to_engine
            .send(silvermere_engine::EngineInput::Tick)
            .unwrap();
    }
    on_world(&r, |_| ()).await;
    assert!(c.is_open(), "one period: marked only");
    for _ in 0..60 {
        r.to_engine
            .send(silvermere_engine::EngineInput::Tick)
            .unwrap();
    }
    c.until("You have been inactive for too long.   Bye!", WAIT)
        .await
        .unwrap();
    assert!(c.wait_closed(WAIT).await.unwrap());
    r.stop().await.unwrap();
}

/// With hit points at -5 only the listed commands run; a move is
/// refused with the mortally-wounded line and the room is unchanged.
#[tokio::test]
async fn mortally_wounded_gate_allows_only_listed() {
    let (_dir, url) = aldric_db();
    let (mut c, r) = in_realm(&url).await;
    on_world(&r, |w| {
        for id in w.session_ids() {
            if let Some(ch) = w.session_mut(id).and_then(|s| s.character.as_mut()) {
                ch.hp = -5;
            }
        }
    })
    .await;
    c.send_line("e").await.unwrap();
    c.until("You may not do that while you are mortally wounded!", WAIT)
        .await
        .unwrap();
    for allowed in ["look", "exp", "i", "health"] {
        let at = c.position();
        c.send_line(allowed).await.unwrap();
        c.until(r"\[HP=-5[^\]\n]*\]:", WAIT).await.unwrap();
        assert!(
            !c.plain_from(at).contains("mortally wounded"),
            "{allowed} is on the allowed list"
        );
    }
    r.stop().await.unwrap();
}

/// `Failure model` (03-architecture): a handler that panics gives that session `Command
/// interrupted.`, other sessions go on and the world thread is alive.
#[tokio::test]
async fn handler_panic_isolated() {
    let (_dir, url) = aldric_db();
    common::stock::add_account(&url, "Bob", "bob123").await;
    let (mut a, r) = in_realm(&url).await;
    let mut b = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut b, "Bob", "bob123").await;
    let ids = on_world(&r, |w| w.session_ids()).await;
    assert_eq!(ids.len(), 2);
    let victim = on_world(&r, |w| {
        w.session_ids()
            .into_iter()
            .find(|id| w.session(*id).is_some_and(|s| s.character.is_some()))
    })
    .await
    .expect("Aldric's session");
    // Keep the expected panic's report out of the output, and only it: other tests of this binary
    // share the process (and its panic hook) under `cargo test`.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let expected = info
            .payload()
            .downcast_ref::<&str>()
            .is_some_and(|m| *m == "handler bug");
        if !expected {
            default_hook(info);
        }
    }));
    r.to_engine
        .send(silvermere_engine::EngineInput::Custom {
            session: Some(victim),
            run: Box::new(|_| panic!("handler bug")),
        })
        .unwrap();
    a.until("Command interrupted.", WAIT).await.unwrap();
    // the world thread answers, and both sessions still work
    assert_eq!(on_world(&r, |w| w.session_ids().len()).await, 2);
    a.send_line("set").await.unwrap();
    a.until(r"\[HP=35[^\]\n]*\]:\s*$|SET", WAIT).await.unwrap();
    b.send_line("").await.unwrap();
    b.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    r.stop().await.unwrap();
}

/// Level exits (`EXIT_LEVEL`): the Silvermere Docks exit north (1:33) is limited to levels
/// 0-3; a level 4 character is refused with the "progressed too far" line and stays in the
/// room.
#[tokio::test]
async fn level_exit_1_33_refuses_level_4() {
    let (_dir, url) = aldric_db();
    common::stock::edit_aldric(&url, |c| {
        c.map = silvermere_types::ids::MapId(1);
        c.room = silvermere_types::ids::RoomId(33);
    })
    .await;
    let (mut c, r) = in_realm(&url).await;
    assert!(c.plain().contains("Docks"), "{}", c.plain());
    // level 4 in memory (a stored level 4 without its CPs is reset on entry)
    on_world(&r, |w| {
        for id in w.session_ids() {
            if let Some(ch) = w.session_mut(id).and_then(|s| s.character.as_mut()) {
                ch.level = 4;
            }
        }
    })
    .await;
    let at = c.position();
    c.send_line("n").await.unwrap();
    c.until(PROMPT, WAIT).await.unwrap();
    let text = c.plain_from(at);
    assert!(
        text.contains("You have progressed too far to go through this exit!"),
        "{text}"
    );
    let room = on_world(&r, |w| {
        w.session_ids()
            .into_iter()
            .find_map(|id| w.session(id)?.character.as_ref().map(|c| c.room.0))
    })
    .await;
    assert_eq!(room, Some(33));
    r.stop().await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Walking the published MegaMud routes over telnet

/// `sum((i+1) * ord(c))` over the title: the part of a MegaMud room id that names the room.
fn title_hash(title: &str) -> u32 {
    title
        .chars()
        .enumerate()
        .map(|(i, c)| (i as u32 + 1) * c as u32)
        .sum()
}

const DIR_WORDS: [&str; 10] = [
    "north",
    "south",
    "east",
    "west",
    "northeast",
    "northwest",
    "southeast",
    "southwest",
    "up",
    "down",
];

/// The title and the MegaMud exit bits of the room a command's output shows, if it shows one.
fn parse_room(plain: &str) -> Option<(String, u32)> {
    let end = plain.rfind("Obvious exits:")?;
    let (head, tail) = plain.split_at(end);
    let line = tail.lines().next()?;
    let mut bits = 0u32;
    for part in line.trim_start_matches("Obvious exits:").split(',') {
        let part = part.trim().trim_end_matches('.');
        let door = part.contains("door") || part.contains("gate");
        let word = part.rsplit(' ').next()?;
        let d = DIR_WORDS.iter().position(|w| *w == word)?;
        bits |= (if door { 2 } else { 1 }) << (2 * d);
    }
    // the paragraph the exits end; the title is its first line when the description follows
    // it directly (a room without the blank line), else the last line before the paragraph
    let head = head.trim_end();
    let mut blocks: Vec<&str> = head.split("\n\n").collect();
    let block = blocks.pop()?;
    let lines: Vec<&str> = block.lines().collect();
    let direct = (0..lines.len().saturating_sub(1))
        .find(|&k| !lines[k].starts_with(' ') && lines[k + 1].starts_with("    "));
    let title = match direct {
        Some(k) => lines[k].trim().to_owned(),
        None => blocks.last()?.lines().last()?.trim().to_owned(),
    };
    // the prompt has no newline: the next message can start on the same line
    let mut title = title.as_str();
    while title.starts_with("[HP=") {
        match title.find("]:") {
            Some(i) => title = &title[i + 2..],
            None => break,
        }
    }
    Some((title.to_owned(), bits))
}

struct Route {
    steps: Vec<(u32, String)>,
}

fn load_route(name: &str) -> Route {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/routes")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    let text: String = bytes.iter().map(|b| *b as char).collect();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let mut header = lines.next().unwrap();
    while header.starts_with('[') {
        header = lines.next().unwrap();
    }
    let _ = header; // FROMID:TOID:steps:...
    let steps = lines
        .map(|l| {
            let mut p = l.splitn(3, ':');
            let id = u32::from_str_radix(p.next().unwrap(), 16).unwrap();
            let _flags = p.next();
            let cmd = p.next().unwrap_or_default();
            let cmd = cmd.split('[').next().unwrap().trim().to_owned();
            (id, cmd)
        })
        .collect();
    Route { steps }
}

/// A locked Door or Gate exit becomes a shut one.
fn unlock_exit(w: &mut silvermere_engine::World, map: u32, room: u32, d: usize) {
    use silvermere_engine::movement::exits::{LOCK_CLOSED, LOCK_LOCKED, live_exit, set_par1};
    use silvermere_types::enums::{Direction, ExitType};
    let key = silvermere_types::ids::RoomKey {
        map: silvermere_types::ids::MapId(map),
        room: silvermere_types::ids::RoomId(room),
    };
    let dir = Direction::COMPASS[d];
    if let Some(e) = live_exit(w, key, dir)
        && matches!(e.kind, ExitType::Door | ExitType::Gate)
        && e.par1 == LOCK_LOCKED
    {
        set_par1(w, key, dir, LOCK_CLOSED);
    }
}

/// The room number the logged-in character is in, on map 1.
async fn current_room(r: &silvermere::Running) -> Option<(u32, u32)> {
    on_world(r, |w| {
        w.session_ids().into_iter().find_map(|id| {
            w.session(id)?
                .character
                .as_ref()
                .map(|c| (c.map.0, c.room.0))
        })
    })
    .await
}

/// What a walk found: the first room whose title differs from the route's, and how many
/// rooms matched the route's id exactly (title and exit bits).
struct WalkReport {
    steps: usize,
    exact: usize,
    title_only: usize,
    broken: Vec<String>,
}

/// Types the route's steps, checking before each one that the room shown is the room the
/// route says the command is typed in.
async fn walk_route(
    c: &mut TelnetClient,
    r: &silvermere::Running,
    route: &Route,
    unlock: bool,
) -> WalkReport {
    let mut report = WalkReport {
        steps: 0,
        exact: 0,
        title_only: 0,
        broken: Vec::new(),
    };
    let mut here = parse_room(&c.plain());
    let mut trace: Vec<String> = Vec::new();
    // The data's own idea of the walk: room -> (title, destination by direction).
    let table = on_world(r, |w| {
        w.rooms
            .iter()
            .map(|(k, room)| {
                (
                    (k.map.0, k.room.0),
                    (room.name.clone(), room.exits.map(|e| e.target.0)),
                )
            })
            .collect::<std::collections::HashMap<(u32, u32), (String, [u32; 10])>>()
    })
    .await;
    let mut believed = current_room(r).await;
    for (n, (rid, cmd)) in route.steps.iter().enumerate() {
        if let Some((title, bits)) = &here {
            report.steps += 1;
            if title_hash(title) & 0xFFF == rid >> 20 {
                if *bits == rid & 0xF_FFFF {
                    report.exact += 1;
                } else {
                    report.title_only += 1;
                }
            } else {
                report.broken.push(format!(
                    "step {} ({cmd}): in {title:?}, route expects id {rid:08X}; trace {:?}",
                    n + 1,
                    &trace[trace.len().saturating_sub(4)..]
                ));
                break;
            }
        }
        trace.push(format!("{} {cmd} @ {:?}", n + 1, here));
        let at = c.position();
        // A move is over when the room, a refusal or the dark shows; anything else when the
        // prompt does. (Waiting for the echo or the first prompt misfires when an event, a
        // timed gate locking say, prints a prompt of its own.)
        let is_move = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"]
            .contains(&cmd.as_str())
            || cmd.starts_with("go ");
        // The prompt behind the reply ends it: what the step printed has all arrived.
        let done = if is_move {
            format!(
                r"(?s)(Obvious exits:|very dark|[Cc]losed|locked|no exit|can.t|cannot|not allowed|You say).*?{PROMPT}"
            )
        } else {
            PROMPT.to_owned()
        };
        c.send_line(cmd).await.unwrap();
        c.until(&done, WAIT).await.unwrap();
        // MegaMud opens a closed door and goes on; a timed gate may close again meanwhile
        for _ in 0..4 {
            let out = c.plain_from(at);
            let last = out.rsplit(&format!("{cmd}\n")).next().unwrap_or_default();
            if last.contains("Obvious exits:")
                || last.contains("very dark")
                || !last.to_lowercase().contains("closed")
            {
                break;
            }
            let retry = c.position();
            c.send_line(&format!("open {cmd}")).await.unwrap();
            c.until(PROMPT, WAIT).await.unwrap();
            if unlock && c.plain_from(retry).contains("is locked") {
                // a timed door went back to locked while the fast ticks ran; keys are not
                // what this test is about
                if let (Some(d), Some((map, room))) = (
                    ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"]
                        .iter()
                        .position(|d| *d == cmd.as_str()),
                    current_room(r).await,
                ) {
                    on_world(r, move |w| unlock_exit(w, map, room, d)).await;
                }
                c.send_line(&format!("open {cmd}")).await.unwrap();
                c.until(PROMPT, WAIT).await.unwrap();
            }
            c.send_line(cmd).await.unwrap();
            c.until(&done, WAIT).await.unwrap();
            let _ = retry;
        }
        let out = c.plain_from(at);
        if out.contains("There is no exit in that direction!") {
            report
                .broken
                .push(format!("step {} ({cmd}): no exit in that direction", n + 1));
            break;
        }
        // the room the data says the command leads to (a text exit is taken from the server)
        let dir = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"]
            .iter()
            .position(|d| *d == cmd.as_str());
        let actual = current_room(r).await;
        if let (Some(d), Some(from)) = (dir, believed) {
            let to = table
                .get(&from)
                .map(|t| (from.0, t.1[d]))
                .filter(|t| t.1 != 0);
            // a map change leaves the map the data names; only same-map exits are checked
            if let Some(to) = to
                && Some(to) != actual
                && actual.is_some_and(|a| a.0 == from.0)
            {
                let name = |k: Option<(u32, u32)>| {
                    k.and_then(|k| table.get(&k))
                        .map_or("?".to_owned(), |t| t.0.clone())
                };
                report.broken.push(format!(
                    "step {} ({cmd}): from room {from:?} ({}) the data leads to {to:?} ({}) but the server is in {actual:?} ({}): {out:?}",
                    n + 1,
                    name(Some(from)),
                    name(Some(to)),
                    name(actual),
                ));
                break;
            }
        }
        believed = actual;
        if out.contains("The room is very dark") {
            here = None;
        } else if let Some(room) = parse_room(&out) {
            here = Some(room);
        }
    }
    report
}

async fn walk_loop(file: &str, map_room: u32) -> WalkReport {
    walk_file(file, map_room, false).await.0
}

/// [`walk_loop`] that also returns the start and the end room.
async fn walk_file(
    file: &str,
    map_room: u32,
    unlock: bool,
) -> (WalkReport, Option<(u32, u32)>, Option<(u32, u32)>) {
    let (_dir, url) = aldric_db();
    common::stock::edit_aldric(&url, |ch| {
        ch.map = silvermere_types::ids::MapId(1);
        ch.room = silvermere_types::ids::RoomId(map_room);
    })
    .await;
    let (mut c, r) = in_realm_ticked_by_test(&url).await;
    if unlock {
        // The published routes are walked with keys and picklocks; a locked door becomes a
        // shut one, which the walker opens like MegaMud does.
        on_world(&r, |w| {
            let keys: Vec<_> = w.rooms.keys().copied().collect();
            for k in keys {
                for d in 0..10 {
                    unlock_exit(w, k.map.0, k.room.0, d);
                }
            }
        })
        .await;
    }
    // Moves wait for the fast tick (the original's delay); feed ticks quickly instead of
    // sleeping for them.
    let ticker = walker_ticker(&r);
    let route = load_route(file);
    let start = current_room(&r).await;
    let report = walk_route(&mut c, &r, &route, unlock).await;
    let end = current_room(&r).await;
    ticker.abort();
    r.stop().await.unwrap();
    (report, start, end)
}

/// `move_user` over the 198-step slum loop (`slmsloop.mp`, Spell Shop 1:2307): every step
/// is typed over telnet, every room's title is the one the route's id names, and the exit
/// bits agree for all but a few rooms.
#[tokio::test]
async fn walk_slum_loop_titles_and_exits() {
    let report = walk_loop("slmsloop.mp", 2307).await;
    assert!(report.broken.is_empty(), "{:?}", report.broken);
    assert!(report.steps >= 180, "{} rooms checked", report.steps);
    assert!(
        report.exact * 100 >= report.steps * 85,
        "{} of {} exact, {} title only",
        report.exact,
        report.steps,
        report.title_only
    );
}

// `routes.md`: the six verified loops close on the new server (each ends in the room it
// started in with every step's title as the route's room id names it), and the two plain
// paths resolve (`rivpgnol` to its end on map 2; `rivpboat`'s step 1 `go passage` is broken
// in the data and is refused, not crashed on): "6 loops verified, 8 routes resolved".
// `routecheck.py` itself reads the world files and talks to no server, so the tests below are
// its telnet counterpart (see the PR description), one walk each on a server of its own so
// they run side by side. The crypt, the labyrinth and the sewers are dark: their rooms are
// checked against the data's exits, not a title. slmsloop, the sixth loop, is
// `walk_slum_loop_titles_and_exits`.

/// A verified loop: nothing broken, at least `min_checked` rooms checked, back where it began.
async fn routecheck_loop(file: &str, map_room: u32, min_checked: usize) {
    let (report, start, end) = walk_file(file, map_room, true).await;
    assert!(report.broken.is_empty(), "{file}: {:?}", report.broken);
    assert!(
        report.steps >= min_checked,
        "{file}: {} rooms checked",
        report.steps
    );
    assert_eq!(start, end, "{file} is a closed loop");
}

#[tokio::test]
async fn routecheck_slm2loop_closes() {
    routecheck_loop("slm2loop.mp", 1076, 70).await;
}

#[tokio::test]
async fn routecheck_cry1loop_closes() {
    routecheck_loop("cry1loop.mp", 1028, 0).await;
}

#[tokio::test]
async fn routecheck_narnloop_closes() {
    routecheck_loop("narnloop.mp", 2150, 0).await;
}

#[tokio::test]
async fn routecheck_madwloop_closes() {
    routecheck_loop("madwloop.mp", 2323, 0).await;
}

#[tokio::test]
async fn routecheck_labeloop_closes() {
    routecheck_loop("labeloop.mp", 1832, 0).await;
}

/// A locked door refuses a keyless walker with its own text (labeloop, step 11).
#[tokio::test]
async fn routecheck_labeloop_locked_door_refuses() {
    let (lab_locked, _, _) = walk_file("labeloop.mp", 1832, false).await;
    assert_eq!(lab_locked.broken.len(), 1, "{:?}", lab_locked.broken);
    assert!(
        lab_locked.broken[0].contains("step 11 (w)"),
        "{:?}",
        lab_locked.broken
    );
    assert!(
        lab_locked.broken[0].contains("The door is locked."),
        "{:?}",
        lab_locked.broken
    );
}

/// The plain path rivpgnol walks to map 2.
#[tokio::test]
async fn routecheck_rivpgnol_reaches_map_2() {
    let (path, start, end) = walk_file("rivpgnol.mp", 2398, true).await;
    assert!(path.broken.is_empty(), "rivpgnol: {:?}", path.broken);
    assert_ne!(start, end);
    assert_eq!(end.map(|e| e.0), Some(2), "ends on map 2");
}

/// rivpboat's step 1 `go passage` is broken in the data (routes.md) and the server refuses it
/// without moving.
#[tokio::test]
async fn routecheck_rivpboat_broken_step_moves_nobody() {
    let (_, start, end) = walk_file("rivpboat.mp", 2398, true).await;
    assert_eq!(start, end, "rivpboat: the broken step moves nobody");
}

/// Character creation and the line-mode editor (`creation-line-mode`, 79x24): race list,
/// class list, lawful prompt, the editor, the validation message, the suicide notice and the
/// first room, byte-equal to the capture; the race list has 12 rows.
#[tokio::test]
async fn creation_line_mode_reaches_realm() {
    let (_dir, url) = fresh_db();
    common::stock::add_account(&url, "Corvin", "corvin1").await;
    let cap = capture("creation-line-mode");
    let r = start(&url).await;
    let mut c = TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    login_to_menu(&mut c, "Corvin", "corvin1").await;
    // The race list, the class list and the lawful question are the original's bytes. The
    // editor itself is not: the original draws MBBSEmu's positioned form even at 79x24, this
    // server prompts field by field (T-022; see the PR description).
    let reports = replay_steps(&mut c, &cap, 4..7, &[], WAIT).await;
    assert_clean("creation-line-mode lists", &reports);
    let text = c.plain();
    let races = text
        .lines()
        .filter(|l| l.starts_with('[') && l.contains("]  ") && !l.starts_with("[HP"))
        .take(12)
        .count();
    assert_eq!(races, 12, "12 race rows");
    let editor_start = c.position();
    for (line, next) in [
        ("N", "Given name"),
        ("", "Family name"),
        ("Ashdown", "Strength"),
        ("", "Intellect"),
        ("", "Willpower"),
        ("", "Agility"),
        ("", "Health"),
        ("", "Charm"),
        ("", "Hair length"),
        ("", "Hair colour"),
        ("", "Eye colour"),
        ("", r"SAVE or QUIT"),
        ("", PROMPT),
    ] {
        c.send_line(line).await.unwrap();
        c.until(next, WAIT).await.unwrap();
    }
    // as in the capture the room shows with the next Enter (statline FULL)
    c.send_line("").await.unwrap();
    c.until("Newhaven, Village Entrance", WAIT).await.unwrap();
    let text = c.plain_from(editor_start);
    assert!(text.contains("You have 100 CP to distribute."));
    assert!(text.contains("Please wait - Validating your name."));
    assert!(text.contains("To prevent accidental suicide or reroll"));
    assert!(text.contains("Newhaven, Village Entrance"));
    r.stop().await.unwrap();
}

/// An empty line shows the room again only under the full status line
/// (`SL_FULL`, or mode 4, `SL_FULLCUSTOM`); the other modes just print the prompt.
#[tokio::test]
async fn enter_redisplays_room_only_in_full() {
    let (_dir, url) = aldric_db();
    let (mut c, r) = in_realm(&url).await;
    for (set, redisplays) in [
        ("statline off", false),
        ("statline on", false),
        ("set statline custom HP %h", false),
        ("statline full", true),
        ("set statline full custom HP %h", true),
    ] {
        c.send_line(set).await.unwrap();
        c.until(r"(?m)(\[HP=|^>|HP \d+)", WAIT).await.unwrap();
        let at = c.position();
        c.send_line("").await.unwrap();
        c.until(r"(?m)(\[HP=|^>|HP \d+)", WAIT).await.unwrap();
        let text = c.plain_from(at);
        assert_eq!(
            text.contains("Newhaven, Village Entrance"),
            redisplays,
            "{set}: {text:?}"
        );
    }
    r.stop().await.unwrap();
}

/// No false `no exit`: for 500 pseudo-randomly chosen Normal exits of the
/// data the server never answers `There is no exit in that direction!`.
#[tokio::test]
async fn no_exit_never_for_existing_exit() {
    let (_dir, url) = aldric_db();
    let (mut c, r) = in_realm_ticked_by_test(&url).await;
    let ticker = walker_ticker(&r);
    let mut exits: Vec<(u32, usize)> = on_world(&r, |w| {
        use silvermere_types::enums::ExitType;
        let mut v = Vec::new();
        for (k, room) in &w.rooms {
            if k.map.0 != 1 {
                continue;
            }
            for (d, e) in room.exits.iter().enumerate() {
                if e.kind == ExitType::Normal
                    && e.target.0 != 0
                    && w.rooms.contains_key(&silvermere_types::ids::RoomKey {
                        map: k.map,
                        room: e.target,
                    })
                {
                    v.push((k.room.0, d));
                }
            }
        }
        v.sort_unstable();
        v
    })
    .await;
    // a fixed pseudo-random pick (xorshift), 500 of them
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut picks = Vec::new();
    for _ in 0..500 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        picks.push(exits.swap_remove((x % exits.len() as u64) as usize));
    }
    let words = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"];
    for (room, d) in picks {
        on_world(&r, move |w| {
            use silvermere_engine::movement::move_user::set_user_location;
            for id in w.session_ids() {
                if w.session(id).is_some_and(|s| s.character.is_some()) {
                    set_user_location(
                        w,
                        id,
                        silvermere_types::ids::RoomKey {
                            map: silvermere_types::ids::MapId(1),
                            room: silvermere_types::ids::RoomId(room),
                        },
                        true,
                    );
                }
            }
        })
        .await;
        let at = c.position();
        c.send_line(words[d]).await.unwrap();
        // Done when this command has answered: a move shows the room (a monster's noise in
        // the next room sends a prompt of its own, so a prompt alone is not the answer, and
        // the echo is off while the move's delay runs).
        c.until(
            r"(Obvious exits:|very dark|[Cc]losed|locked|no exit|can.t|cannot|not allowed|You say)",
            WAIT,
        )
        .await
        .unwrap();
        let text = c.plain_from(at);
        assert!(
            !text.contains("There is no exit in that direction!"),
            "room {room} {}: {text:?}",
            words[d]
        );
    }
    ticker.abort();
    r.stop().await.unwrap();
}

/// `07` layer 3 "Persistence": the room and the settings of a character survive a server
/// restart (clean logoff saves; a hang-up saves on its own path).
#[tokio::test]
async fn persistence_round_trip_after_the_walk() {
    let (_dir, url) = aldric_db();
    let (mut c, r) = in_realm(&url).await;
    for line in ["w", "w", "statline off"] {
        c.send_line(line).await.unwrap();
        c.until(r"(?s)\n.*\[HP=|\n.*\n", Duration::from_secs(5))
            .await
            .ok();
        tokio::time::sleep(Duration::from_millis(1200)).await; // the move delay
    }
    common::stock::quit_to_menu(&mut c, &r.to_engine, "Aldric").await;
    c.disconnect().await;
    r.stop().await.unwrap();

    let r = start(&url).await;
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    c.send_line("E").await.unwrap();
    c.until("Newhaven, Narrow Road", WAIT).await.unwrap();
    let mode = on_world(&r, |w| {
        w.session_ids()
            .into_iter()
            .find_map(|id| w.session(id)?.character.as_ref().map(|c| c.statline))
    })
    .await;
    assert_eq!(mode, Some(silvermere_types::enums::StatlineMode::Off));
    r.stop().await.unwrap();
}

/// A direction typed right after a prompt is always echoed (the host echoes
/// at receive time, with the echo back on by the time the prompt shows). The room-walk
/// differential saw `w` go unechoed about once in three runs.
#[tokio::test]
async fn a_move_typed_after_the_prompt_is_always_echoed() {
    let (_dir, url) = aldric_db();
    // the moves' ticks come from the test: each move is still typed the moment its
    // prompt arrives, without a second's wait for the server's own tick
    let (mut c, r) = in_realm_ticked_by_test(&url).await;
    let ticker = walker_ticker(&r);
    for _ in 0..2 {
        c.send_line("w").await.unwrap();
        c.until(PROMPT, WAIT).await.unwrap();
    }
    assert!(c.plain().contains("Newhaven, Narrow Road"));
    for n in 0..12 {
        let dir = if n % 2 == 0 { "e" } else { "w" };
        let at = c.position();
        c.send_line(dir).await.unwrap();
        c.until(r"\[HP=-?\d+[^\]\n]*\]:", WAIT).await.unwrap();
        let got = c.plain_from(at);
        assert!(got.starts_with(&format!("{dir}\n")), "move {n}: {got:?}");
    }
    ticker.abort();
    r.stop().await.unwrap();
}

/// A line sent the instant the prompt arrives, with a few hundred microseconds of jitter and
/// the CR and LF sometimes in separate writes, is always echoed, answered, and followed by the
/// next prompt; the stuck-session reports of the 3-run gate (input after a prompt neither
/// echoed nor processed, the account never released) would stall this loop.
#[tokio::test]
async fn a_line_sent_the_instant_the_prompt_arrives_is_never_lost() {
    let (_dir, url) = aldric_db();
    let (mut c, r) = in_realm(&url).await;
    let cmds = ["exp", "st", "keys", "health", "wealth", "i"];
    for n in 0..300usize {
        let cmd = cmds[n % cmds.len()];
        let at = c.position();
        tokio::time::sleep(Duration::from_micros(((n * 7919) % 5) as u64 * 300)).await;
        if n % 3 == 0 {
            c.send_text(cmd).await.unwrap();
            c.send_raw(b"\r\n").await.unwrap();
        } else {
            c.send_line(cmd).await.unwrap();
        }
        c.until(r"\[HP=-?\d+[^\]\n]*\]:", WAIT)
            .await
            .unwrap_or_else(|e| panic!("line {n} ({cmd}): {e:?}"));
        let got = c.plain_from(at);
        assert!(got.starts_with(&format!("{cmd}\n")), "line {n}: {got:?}");
    }
    r.stop().await.unwrap();
}

/// A client that drops the connection while its password is being checked must not leave the
/// account claimed: the login task claims it, finds its session gone, and used to return with
/// the claim held, so every later login as that account got "This account is already logged
/// on." (the stuck sessions of the 3-run gate).
#[tokio::test]
async fn dropping_the_connection_mid_login_releases_the_account() {
    let (_dir, url) = fresh_db();
    common::stock::add_account(&url, "Aldric", "aldric1").await;
    let r = start(&url).await;
    for round in 0..8 {
        let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
        c.until("Username:", WAIT).await.unwrap();
        c.send_line("Aldric").await.unwrap();
        c.until("Password:", WAIT).await.unwrap();
        c.send_line("aldric1").await.unwrap();
        c.disconnect().await;
        // give the server time to finish the password check and notice the dead session
        tokio::time::sleep(Duration::from_millis(900)).await;
        let mut again = TelnetClient::connect(r.telnet_addr).await.unwrap();
        again.until("Username:", WAIT).await.unwrap();
        again.send_line("Aldric").await.unwrap();
        again.until("Password:", WAIT).await.unwrap();
        again.send_line("aldric1").await.unwrap();
        let got = again
            .until(r"Make your selection|already logged on", WAIT)
            .await
            .unwrap();
        assert!(
            !got.plain.contains("already logged on"),
            "round {round}: {:?}",
            got.plain
        );
        again.disconnect().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    r.stop().await.unwrap();
}
