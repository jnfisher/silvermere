//! M4 scripts (T-057, from T-054), layer 3 over telnet on the stock world: the Newhaven
//! skiff (a Text exit), GREET and ASK of the boatman, and the level-10 crypt quest
//! walked by a Witchunter made level 10 with
//! `sysop tweak level 10`, with the mummy and the troll fought for real, their keys picked
//! up and used, and the mithril cutlass taken from the Large Tomb.
//!
//! Stock records: rooms 1:2140 Village Entrance, 1:1813 Pier (specific monster 61, the
//! boatman), 1:834 Graveyard, Tomb Entrance, 1:1309 Crypt, Sealed Tomb (specific monster
//! 31 mummy: bone key 177 at 100%), 1:2224 Large Chamber (82 troll: iron key 338 at
//! 100%), 1:2236 Large Tomb (85 witchunter spirit; item 342 mithril cutlass).

mod common;

use std::time::Duration;

use common::*;
use silvermere_auth::Role;

const WITCHUNTER: u16 = 2;
const WARRIOR: u16 = 1;
const TICK: Duration = Duration::from_millis(15);

fn assert_clean(cl: &Classifier, out: &str, sent: &[&str]) {
    let unknown = cl.unknown(out, sent);
    assert!(
        unknown.is_empty(),
        "unclassified lines: {unknown:#?}\n{out}"
    );
}

#[test]
fn an_echo_followed_by_prompts_is_clean() {
    let cl = Classifier::bare();
    assert!(cl.unknown("w[HP=30000]:", &["w"]).is_empty());
    assert!(cl.unknown("w[HP=30000]:[HP=30000]:", &["w"]).is_empty());
}

#[test]
fn an_echo_followed_by_anything_else_stays_unknown() {
    let cl = Classifier::bare();
    assert!(
        !cl.unknown("w[HP=30000]:zork frobnicates", &["w"])
            .is_empty()
    );
    assert!(!cl.unknown("nobody is here.", &["n"]).is_empty());
    assert!(!cl.unknown("wObvious exits: NONE!!!", &["w"]).is_empty());
}

/// The quest monster of `room` (its specific monster `monster`), made fresh for the fight
/// and alone in the room; returns its active number and exact name. The world setup keeps
/// the fight about the kill and the drop:
/// - every other monster in the room and every other copy of `monster` goes, so `attack`
///   cannot pick a wandering mummy (the crypt's wanderers include small and large ones);
/// - `followpct` 100: a monster not fought in a round may wander off with chance
///   `(100 - followpct) / 2` (`wander_chance`), as in the original;
///   at test speed that ended the fight before the kill;
/// - hit points (and maximum) 12, short enough for an unarmed character.
async fn quest_monster(
    r: &silvermere::Running,
    room: u32,
    monster: u16,
) -> (silvermere_types::ids::ActiveMonsterId, String) {
    on_world(r, move |w| {
        let gone: Vec<_> = w
            .active_monsters
            .values()
            .filter(|a| (a.room.0 == room && a.map.0 == 1) || a.knownnum.0 == i32::from(monster))
            .map(|a| a.number)
            .collect();
        for n in gone {
            silvermere_engine::monsters::active::remove_active_monster(w, n, false);
        }
        if let Some(s) = w
            .room_state
            .get_mut(&silvermere_types::ids::RoomKey::new(1, room))
        {
            s.specalive = false;
        }
    })
    .await;
    let made = try_spawn(r, 1, room, monster)
        .await
        .unwrap_or_else(|| panic!("no monster {monster} in {room}"));
    on_world(r, move |w| {
        let a = w
            .active_monsters
            .values_mut()
            .find(|a| a.number == made)
            .expect("the quest monster");
        a.followpct = 100;
        a.hp = 12;
        a.maxhp = 12;
        (made, a.name.clone())
    })
    .await
}

/// Fights the quest monster `(number, name)` by its exact name until the kill's
/// experience line, and checks that it is the one that died; returns the text.
async fn fight(
    r: &silvermere::Running,
    c: &mut silvermere_testkit::TelnetClient,
    (number, name): &(silvermere_types::ids::ActiveMonsterId, String),
) -> String {
    let at = c.position();
    c.send_line(&format!("attack {name}")).await.unwrap();
    c.until(
        r"You gain \d+ experience\.(?s:.*)\[HP=",
        Duration::from_secs(90),
    )
    .await
    .unwrap();
    let out = c.plain_from(at);
    assert_eq!(
        monster_hp(r, *number).await,
        None,
        "{name} is not the one that died:\n{out}"
    );
    out + &cmd(c, "").await
}

/// script_skiff and ask_greet: from the Village Entrance `se s`, `borrow skiff` crosses to
/// Silvermere (the Text exit's line, then the far side's room); on the Pier the boatman
/// greets (his talk block 5 links to 6), `ask boatman skiff` answers block 7 and an unknown
/// word gets `has nothing to tell you!` (T-054).
#[tokio::test(flavor = "multi_thread")]
async fn skiff_crossing_and_the_boatman() {
    let db = stock_db();
    let r = start(&db.url, 31).await;
    let mut c = new_character(&r, &db.url, "Aldric", WARRIOR, &[]).await;
    edit_player(&r, "Aldric", |p| {
        p.maxhp = 400;
        p.hp = 400;
    })
    .await;
    place(&r, "Aldric", 1, 2140).await;
    let cl = Classifier::new(&r).await;
    let t = ticker(&r, TICK);
    let (to_dock, dock) = walk(&r, &mut c, "Aldric", "se s").await;
    let before = room_of(&r, "Aldric").await;
    let crossing = cmd_until(&mut c, "borrow skiff", r"Obvious exits:.*\n(?s:.*)\[HP=").await;
    let after = room_of(&r, "Aldric").await;
    assert_ne!(before, after, "the skiff did not move:\n{crossing}");
    assert!(
        crossing.lines().count() > 3,
        "the crossing text and the far side:\n{crossing}"
    );
    let (to_pier, pier) = walk(&r, &mut c, "Aldric", "s s e n").await;
    assert_eq!(pier, 1813, "{to_pier}");
    assert!(to_pier.contains("Pier"), "{to_pier}");
    quest_monster(&r, 1813, 61).await;
    let greet = cmd(&mut c, "greet boatman").await;
    let ask = cmd(&mut c, "ask boatman skiff").await;
    let nothing = cmd(&mut c, "ask boatman weather").await;
    t.abort();
    assert!(greet.lines().count() > 2, "{greet}");
    assert!(ask.lines().count() > 2, "{ask}");
    assert!(nothing.contains("has nothing to tell you!"), "{nothing}");
    let _ = dock;
    assert_clean(
        &cl,
        &format!("{to_dock}{crossing}{to_pier}{greet}{ask}{nothing}"),
        &[
            "se",
            "s",
            "e",
            "n",
            "borrow skiff",
            "greet boatman",
            "ask boatman skiff",
            "ask boatman weather",
        ],
    );
    r.stop().await.unwrap();
}

/// script_crypt_quest: the level-10 crypt quest end to end on the new server.
#[tokio::test(flavor = "multi_thread")]
async fn crypt_quest_by_a_level_10_witchunter() {
    let db = stock_db();
    let r = start(&db.url, 32).await;
    let mut c = new_character(&r, &db.url, "Kang", WITCHUNTER, &[Role::Sysop]).await;
    // §4: the level comes from `sysop tweak level 10` (T-055), never from grinding; the
    // DEBUG grant stands in for the original's CODEC_OF_TWINKS key
    on_world(&r, |w| {
        let id = session_of(w, "Kang");
        w.session_mut(id)
            .unwrap()
            .sysop_keys
            .insert(silvermere_engine::commands::sysop::Key::Debug);
    })
    .await;
    let tweak = cmd(&mut c, "sysop tweak level 10").await;
    edit_player(&r, "Kang", |p| {
        assert_eq!(p.level, 10, "sysop tweak level 10");
        // enough to outlast the mummy and the troll at test speed
        p.maxhp = 30000;
        p.hp = 30000;
        p.str = 60;
    })
    .await;
    give(&r, "Kang", 175, 3).await; // torches: the crypt is dark
    let cl = Classifier::new(&r).await;
    let mut log = tweak;
    let mut sent: Vec<String> = vec!["sysop tweak level 10".into()];
    let t = ticker(&r, TICK);
    // the character is kept at full health: at test speed the crypt's monsters get many
    // rounds in while the walk waits for each move
    let heal = {
        let tx = r.to_engine.clone();
        tokio::spawn(async move {
            loop {
                let run = Box::new(|w: &mut silvermere_engine::World| {
                    for id in w.session_ids() {
                        if let Some(p) = w.session_mut(id).and_then(|s| s.character.as_mut()) {
                            // the level tweak recomputes the maximum: pin it again
                            p.maxhp = 30000;
                            p.hp = 30000;
                        }
                    }
                });
                if tx
                    .send(silvermere_engine::EngineInput::Custom { session: None, run })
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };

    // §1 step 2: Graveyard, Tomb Entrance; the door is locked at start-up and bashed open
    place(&r, "Kang", 1, 834).await;
    log += &cmd(&mut c, "light torch").await;
    let open = cmd(&mut c, "open n").await;
    assert!(open.contains("The door is locked."), "{open}");
    log += &open;
    for _ in 0..200 {
        if room_of(&r, "Kang").await != 834 {
            break;
        }
        let bash = cmd(&mut c, "bash n").await;
        let opened = bash.contains("You bashed the door open.");
        log += &bash;
        if opened && room_of(&r, "Kang").await == 834 {
            // the door gave and stayed open: walk through
            log += &cmd(&mut c, "n").await;
        }
    }
    assert_eq!(
        room_of(&r, "Kang").await,
        835,
        "the tomb door never gave:\n{log}"
    );

    // steps 3-8: first level, the two levers, the passage
    let (out, at) = walk(&r, &mut c, "Kang", "d").await;
    assert!(out.contains("Crypt, Lower Level"), "{out}");
    log += &out;
    let (out, at2) = walk(&r, &mut c, "Kang", "n n n n e e s s s s").await;
    assert_eq!((at, at2), (1028, 1038), "{out}");
    log += &out;
    let pull = cmd(&mut c, "pull lever").await;
    assert!(
        pull.contains("You pull the lever. Off in the distance you hear a small click."),
        "{pull}"
    );
    log += &pull;
    let (out, at) = walk(&r, &mut c, "Kang", "n n n n w w w w s s s s").await;
    assert_eq!(at, 1044, "{out}");
    log += &out;
    log += &cmd(&mut c, "pull lever").await;
    let (out, at) = walk(&r, &mut c, "Kang", "n n n n e e n n n n n n n n n d").await;
    assert_eq!(at, 1070, "{out}");
    assert!(out.contains("Crypt, Small Chamber"), "{out}");
    log += &out;

    // steps 9-12: second level, the secret passage south, the statue's button
    for _ in 0..60 {
        log += &cmd(&mut c, "search s").await;
        log += &cmd(&mut c, "s").await;
        if room_of(&r, "Kang").await != 1070 {
            break;
        }
    }
    assert_eq!(room_of(&r, "Kang").await, 1237, "the passage south:\n{log}");
    let (out, at) = walk(&r, &mut c, "Kang", "se sw se se se sw sw sw").await;
    assert_eq!(at, 1250, "{out}");
    log += &out;
    let push = cmd(&mut c, "push button").await;
    assert!(push.contains("You push the button."), "{push}");
    log += &push;

    // step 13: third level up to the Sealed Tomb (a Level exit for exactly level 10)
    let (out, at) = walk(&r, &mut c, "Kang", "s s s d n n n n e n n w n n n n n").await;
    assert_eq!(at, 1309, "{out}");
    assert!(out.contains("Crypt, Sealed Tomb"), "{out}");
    log += &out;

    // steps 14-17: the mummy, its bone key, the Key exit north
    let mummy = quest_monster(&r, 1309, 31).await;
    let kill = fight(&r, &mut c, &mummy).await;
    log += &kill;
    let get = cmd(&mut c, "get key").await;
    assert_eq!(
        carried(&r, "Kang", 177).await,
        1,
        "no bone key:\n{kill}{get}"
    );
    log += &get;
    let used = cmd(&mut c, "use bone key n").await;
    assert!(
        used.contains("You successfully unlocked the door."),
        "{used}"
    );
    log += &used;
    log += &cmd(&mut c, "open n").await;
    let (out, at) = walk(&r, &mut c, "Kang", "n").await;
    assert_eq!(at, 1377, "{out}");
    assert!(
        out.contains("Crypt, Shadowed Hall") || out.contains("You are blind"),
        "{out}"
    );
    log += &out;

    // steps 18-19: the Warrior's west is refused; north, then the Witchunter's west
    let refused = cmd(&mut c, "w").await;
    assert_eq!(room_of(&r, "Kang").await, 1377, "{refused}");
    log += &refused;
    let (out, at) = walk(&r, &mut c, "Kang", "n w w w w w w w w").await;
    assert_eq!(at, 2219, "{out}");
    assert!(
        out.contains("Marble Room") || out.contains("You are blind"),
        "{out}"
    );
    log += &out;

    // steps 20-23: the clawed hand, the troll, its iron key
    let (out, at) = walk(&r, &mut c, "Kang", "s sw").await;
    assert_eq!(at, 2223, "{out}");
    log += &out;
    let swivel = cmd_until(
        &mut c,
        "push button",
        r"(?:Large Chamber|You are blind)(?s:.*)\[HP=",
    )
    .await;
    assert!(
        swivel.contains("The wall swivels rapidly around, moving you into another room!"),
        "{swivel}"
    );
    log += &swivel;
    let troll = quest_monster(&r, 2224, 82).await;
    let kill = fight(&r, &mut c, &troll).await;
    log += &kill;
    log += &cmd(&mut c, "get key").await;
    assert_eq!(carried(&r, "Kang", 338).await, 1, "no iron key:\n{kill}");
    let used = cmd(&mut c, "use iron key n").await;
    assert!(
        used.contains("You successfully unlocked the door."),
        "{used}"
    );
    log += &used;
    log += &cmd(&mut c, "open n").await;

    // steps 24-27: Marble Chamber, the Large Tomb, the spirit, the cutlass
    let (out, at) = walk(&r, &mut c, "Kang", "n n w w").await;
    assert_eq!(at, 2236, "{out}");
    assert!(
        out.contains("Large Tomb") || out.contains("You are blind"),
        "{out}"
    );
    log += &out;
    quest_monster(&r, 2236, 85).await;
    let greet = cmd(&mut c, "greet witch").await;
    assert!(
        greet.contains("The witchunter spirit greets you with a bow"),
        "{greet}"
    );
    log += &greet;
    let get = cmd(&mut c, "get mith").await;
    assert_eq!(carried(&r, "Kang", 342).await, 1, "{get}");
    log += &get;
    // step 29: the portal back to the Tomb Entrance
    let portal = cmd_until(
        &mut c,
        "go portal",
        r"(?:Tomb Entrance|You are blind)(?s:.*)\[HP=",
    )
    .await;
    assert!(
        portal.contains("You step through the shimmering portal."),
        "{portal}"
    );
    assert_eq!(room_of(&r, "Kang").await, 834);
    log += &portal;
    t.abort();
    heal.abort();

    for s in [
        "light torch",
        "open n",
        "bash n",
        "d",
        "n",
        "s",
        "e",
        "w",
        "se",
        "sw",
        "pull lever",
        "search s",
        "push button",
        "get key",
        "use bone key n",
        "use iron key n",
        "greet witch",
        "get mith",
        "go portal",
    ] {
        sent.push(s.into());
    }
    sent.push(format!("attack {}", mummy.1));
    sent.push(format!("attack {}", troll.1));
    let sent: Vec<&str> = sent.iter().map(String::as_str).collect();
    assert_clean(&cl, &log, &sent);
    r.stop().await.unwrap();
}

#[test]
fn has_reply_skips_tick_prompts_before_the_reply() {
    let echo = "[HP=400]:greet boatman";
    assert!(!has_reply(echo, "greet boatman"));
    assert!(!has_reply(&format!("{echo}\n[HP=400]:"), "greet boatman"));
    assert!(!has_reply(
        &format!("{echo}\n[HP=400]:[HP=400]:"),
        "greet boatman"
    ));
    assert!(has_reply(
        &format!("{echo}\n[HP=400]:\nThe boatman nods.\n[HP=400]:"),
        "greet boatman"
    ));
    // a resting prompt carries a state tag
    assert!(has_reply(
        "rest\nYou are now resting.\n[HP=100/MA=12]: (Resting) ",
        "rest"
    ));
    assert!(!has_reply(
        "rest\n[HP=100/MA=12]: (Resting) [HP=103/MA=12]: (Resting) ",
        "rest"
    ));
    // the echo is a whole line: `w` inside other output is not it
    assert!(!has_reply(
        "The wolf arrives from the west.\n[HP=400]:w\n[HP=400]:",
        "w"
    ));
    assert!(has_reply(
        "The wolf arrives from the west.\n[HP=400]:w\nYou walk west.\n[HP=400]:",
        "w"
    ));
    assert!(!has_reply(
        "The wolf arrives from the west.\n[HP=400]:",
        "w"
    ));
    // the reply with no prompt behind it yet is not complete
    assert!(!has_reply(
        &format!("{echo}\nThe boatman nods."),
        "greet boatman"
    ));
}

/// A fake server: tick-redrawn prompts first, the reply later, then the closing prompt.
#[tokio::test]
async fn cmd_returns_the_reply_not_the_first_tick_prompt() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf).await.unwrap();
        s.write_all(b"[HP=400]:greet boatman").await.unwrap();
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            s.write_all(b"\r\n[HP=400]:").await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
        s.write_all(b"\r\nThe boatman nods.\r\n[HP=400]:")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let mut c = silvermere_testkit::TelnetClient::connect(addr)
        .await
        .unwrap();
    let out = cmd(&mut c, "greet boatman").await;
    assert!(out.contains("The boatman nods."), "{out}");
}

/// A command that prints nothing still returns, once only prompts have come and gone quiet.
#[tokio::test]
async fn cmd_with_no_reply_returns_after_the_quiet() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf).await.unwrap();
        s.write_all(b"[HP=400]:hide\r\n[HP=400]:").await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
    });
    let mut c = silvermere_testkit::TelnetClient::connect(addr)
        .await
        .unwrap();
    let out = cmd(&mut c, "hide").await;
    assert!(out.contains("[HP=400]:"), "{out}");
}

/// Prompts redrawn without pause (resting, the ticker) must not keep a silent command waiting.
#[tokio::test]
async fn cmd_with_no_reply_returns_through_continuous_prompt_redraws() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf).await.unwrap();
        s.write_all(b"[HP=400]:hide").await.unwrap();
        for _ in 0..70 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            if s.write_all(b"\r\n[HP=400]:").await.is_err() {
                break;
            }
        }
    });
    let mut c = silvermere_testkit::TelnetClient::connect(addr)
        .await
        .unwrap();
    let t0 = std::time::Instant::now();
    let out = cmd(&mut c, "hide").await;
    assert!(out.contains("[HP=400]:"), "{out}");
    assert!(
        t0.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t0.elapsed()
    );
}
