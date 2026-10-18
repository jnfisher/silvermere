//! M3 communication and gang tests (T-048): the telepath allowance per medium cycle, the
//! `set maxpages` ceiling, say / say-to / yell / gossip and the gang lifecycle through two
//! live sessions.
//!
//! The medium cycle is driven by hand (`start_manual` + `tick`): the burst of telepaths
//! cannot straddle a real-time cycle boundary.

mod common;

use common::m3::{
    Player, assert_classified, body_lines, classify, edit_char, place, start_manual, stored,
    team_db, tick,
};

fn names(raw: &[u8]) -> Vec<String> {
    classify(raw).into_iter().map(|e| e.name).collect()
}

/// The two players at `room` on a server whose cycles run only by hand.
async fn pair(url: &str, a_room: u32, b_room: u32) -> (silvermere::Running, Player, Player) {
    place(url, "Brynja", a_room).await;
    place(url, "Isolde", b_room).await;
    let r = start_manual(url).await;
    let mut b = Player::enter(&r, "Brynja").await;
    let at = b.c.position();
    let i = Player::enter(&r, "Isolde").await;
    b.wait_for(at, "Isolde just entered the Realm\\.").await;
    (r, b, i)
}

/// Five telepaths in one cycle: four are delivered (the test is `pagecnt <= pagemax`, so
/// one more than the maximum of 3 gets through, sic) and the fifth prints
/// `--- Telepath Not Sent ---`; after a medium cycle the next one is delivered again.
#[tokio::test]
async fn telepath_allowance() {
    let (_dir, url) = team_db();
    let (r, mut b, mut i) = pair(&url, 2140, 2140).await;
    let (ab, ai) = (b.c.position(), i.c.position());
    let mut receipts = Vec::new();
    for n in 0..5 {
        let t = b.cmd(&format!("/isolde m{n}")).await;
        receipts.push(body_lines(&t)[1].clone());
    }
    assert_eq!(
        receipts,
        [
            "--- Telepath Sent to Isolde ---",
            "--- Telepath Sent to Isolde ---",
            "--- Telepath Sent to Isolde ---",
            "--- Telepath Sent to Isolde ---",
            "--- Telepath Not Sent ---"
        ]
    );
    // a command of Isolde's returns after everything queued for her has arrived
    i.cmd("i").await;
    let heard = body_lines(&i.c.plain_from(ai));
    assert_eq!(
        heard[..4],
        [
            "Brynja telepaths: m0",
            "Brynja telepaths: m1",
            "Brynja telepaths: m2",
            "Brynja telepaths: m3"
        ]
    );
    assert!(!heard.iter().any(|l| l.contains("m4")), "{heard:?}");
    let ev = names(&b.c.raw()[ab..]);
    assert_eq!(ev.iter().filter(|n| *n == "telepath_status").count(), 5);
    assert_classified(
        "telepath burst",
        &b.c.raw()[ab..],
        &[
            "/isolde m0",
            "/isolde m1",
            "/isolde m2",
            "/isolde m3",
            "/isolde m4",
        ],
    );
    // three fast ticks make one medium cycle: the counters are zeroed
    tick(&r, 3).await;
    let t = b.cmd("/isolde m5").await;
    assert_eq!(body_lines(&t)[1], "--- Telepath Sent to Isolde ---");
    i.cmd("i").await;
    assert!(i.c.plain().contains("Brynja telepaths: m5"));
    b.leave_ticking(&r).await;
    i.leave_ticking(&r).await;
    r.stop().await.unwrap();
}

/// `set maxpages 30` (the receiver's setting, saved with the character): 32 telepaths in
/// one cycle deliver 31 (`pagecnt <= pagemax`) and the 32nd is `Not Sent`.
#[tokio::test]
async fn telepath_maxpages() {
    let (_dir, url) = team_db();
    let (r, mut b, mut i) = pair(&url, 2140, 2140).await;
    let at = i.c.position();
    let t = i.cmd("set maxpages 30").await;
    assert_eq!(body_lines(&t), ["set maxpages 30", "Maxpages set to 30"]);
    assert!(names(&i.c.raw()[at..]).contains(&"maxpages_set".into()));
    let ai = i.c.position();
    let mut sent = 0;
    let mut not_sent = 0;
    for n in 0..32 {
        let t = b.cmd(&format!("/isolde m{n}")).await;
        match body_lines(&t)[1].as_str() {
            "--- Telepath Sent to Isolde ---" => sent += 1,
            "--- Telepath Not Sent ---" => not_sent += 1,
            other => panic!("{other}"),
        }
    }
    assert_eq!((sent, not_sent), (31, 1));
    i.cmd("i").await;
    let heard = i.c.plain_from(ai);
    assert_eq!(heard.matches("Brynja telepaths: m").count(), 31);
    assert!(heard.contains("Brynja telepaths: m30"));
    assert!(!heard.contains("Brynja telepaths: m31"));
    b.leave_ticking(&r).await;
    i.leave_ticking(&r).await;
    r.stop().await.unwrap();
    assert_eq!(stored(&url, "Isolde").await.pagemax, 30);
}

/// Say, say-to, yell (one room away) and gossip: what the speaker, the room and the
/// neighbour see, each line classified (`you_say`, `says`, `says_to_you`, `yell`, `gossip`).
#[tokio::test]
async fn comms_misc() {
    let (_dir, url) = team_db();
    let (r, mut b, mut i) = pair(&url, 2140, 2140).await;
    // same room: SAY and SAY-TO
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = b.cmd(".hello").await;
    assert_eq!(body_lines(&t), [".hello", "You say \"hello\""]);
    let t = b.cmd(">isolde psst").await;
    assert_eq!(body_lines(&t)[0], ">isolde psst");
    i.cmd("i").await;
    let heard = body_lines(&i.c.plain_from(ai));
    assert!(
        heard.contains(&"Brynja says \"hello\"".to_owned()),
        "{heard:?}"
    );
    assert!(
        heard
            .iter()
            .any(|l| l.starts_with("Brynja says (to you) \"psst\"")),
        "{heard:?}"
    );
    let ev = names(&i.c.raw()[ai..]);
    assert!(
        ev.contains(&"says".into()) && ev.contains(&"says_to_you".into()),
        "{ev:?}"
    );
    assert!(names(&b.c.raw()[ab..]).contains(&"you_say".into()));

    // Isolde walks north to the Weapons Shop; Brynja yells from the Village Entrance
    let t = i.walk_ticking(&r, "n", "Newhaven, Weapons Shop").await;
    assert!(t.contains("Newhaven, Weapons Shop"), "{t}");
    let ai = i.c.position();
    let t = b.cmd("\"help").await;
    assert!(
        body_lines(&t).contains(&"You yell \"help\"".to_owned()),
        "{t}"
    );
    i.cmd("i").await;
    let heard = body_lines(&i.c.plain_from(ai));
    assert!(
        heard.contains(&"Someone yells from the south \"help\"".to_owned()),
        "{heard:?}"
    );
    assert!(names(&i.c.raw()[ai..]).contains(&"yell".into()));

    // GOSSIP reaches the speaker and everyone listening
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = b.cmd("gossip anyone?").await;
    assert_eq!(
        body_lines(&t),
        ["gossip anyone?", "Brynja gossips: anyone?"]
    );
    i.cmd("i").await;
    assert!(body_lines(&i.c.plain_from(ai)).contains(&"Brynja gossips: anyone?".to_owned()));
    assert!(names(&b.c.raw()[ab..]).contains(&"gossip".into()));
    assert_classified("comms", &i.c.raw()[ai..], &["i"]);
    b.leave_ticking(&r).await;
    i.leave_ticking(&r).await;
    r.stop().await.unwrap();
}

/// `create gang Wolves`, `invite member isolde`, `join gang wolves`, `gb hi`, `leave gang`,
/// `disband gang` and `y`: the lines of `crates/silvermere-engine/tests/gangs.rs` over two
/// live sessions, and the `gangs` row and the characters' `gang` after each step.
#[tokio::test]
async fn gang_lifecycle() {
    let (_dir, url) = team_db();
    edit_char(&url, "Brynja", |c| c.exprem = 150_000).await;
    let (r, mut b, mut i) = pair(&url, 2140, 2140).await;
    let t = b.cmd("create gang Wolves").await;
    assert_eq!(body_lines(&t), ["create gang Wolves", "Gang created."]);

    let ai = i.c.position();
    let t = b.cmd("invite member isolde").await;
    assert_eq!(
        body_lines(&t),
        [
            "invite member isolde",
            "You have invited Isolde to join your gang."
        ]
    );
    let told = i
        .wait_for(ai, r"Gang leader Brynja has invited you to join Wolves\.")
        .await;
    assert!(
        body_lines(&told)
            .contains(&"Gang leader Brynja has invited you to join Wolves.".to_owned())
    );

    let ab = b.c.position();
    let t = i.cmd("join gang wolves").await;
    assert_eq!(
        body_lines(&t),
        [
            "join gang wolves",
            "You have joined the gang Wolves.",
            "Isolde just joined your gang."
        ]
    );
    b.wait_for(ab, r"Isolde just joined your gang\.").await;
    assert!(names(&b.c.raw()[ab..]).contains(&"gang_msg".into()));

    // the gang channel reaches the speaker and the member
    let (ab, ai) = (b.c.position(), i.c.position());
    let t = b.cmd("gb hi").await;
    assert_eq!(body_lines(&t), ["gb hi", "Brynja gangpaths: hi"]);
    i.wait_for(ai, "Brynja gangpaths: hi").await;
    assert!(names(&i.c.raw()[ai..]).contains(&"gossip".into()));
    let _ = ab;

    let ab = b.c.position();
    let t = i.cmd("leave gang").await;
    assert_eq!(body_lines(&t), ["leave gang", "You have left Wolves."]);
    let told = b.wait_for(ab, r"Isolde has left Wolves\.").await;
    assert!(told.contains("Isolde has left Wolves."));

    // the leader asks before disbanding; Y disbands, the name stays taken
    b.say("disband gang").await;
    b.wait_text(0, "Are you sure you want to disband Wolves\\? ")
        .await;
    let at = b.c.position();
    b.say("y").await;
    let told = b.wait_for(at, "now been disbanded").await;
    assert!(
        told.contains("The gang Wolves has now been disbanded.")
            && told.contains(
                "The name may not be used again until all members have entered the game!"
            ),
        "{told}"
    );
    b.leave_ticking(&r).await;
    i.leave_ticking(&r).await;
    r.stop().await.unwrap();

    let db = silvermere_db::Db::connect_and_migrate(&url).await.unwrap();
    let g = db
        .gangs()
        .by_name("Wolves")
        .await
        .unwrap()
        .expect("gang row");
    db.close().await;
    assert_eq!((g.members, g.leader.as_str()), (0, "Brynja"));
    assert!(
        g.flags
            .contains(silvermere_types::flags::GangFlags::DISBANDED)
    );
    assert_eq!(stored(&url, "Brynja").await.gang, "");
    assert_eq!(stored(&url, "Isolde").await.gang, "");
}

/// `partycheck.py` over the in-process server: two characters send each other 50 numbered
/// telepaths at the bots' pace (three each per medium cycle, `Remotes` rate limit 3 per 3 s),
/// and every one of the 100 arrives, none `Not Sent` (grouping stage G0).
#[tokio::test]
async fn partycheck_hundred_telepaths() {
    let (_dir, url) = team_db();
    let (r, mut a, mut b) = pair(&url, 2140, 2140).await;
    let (ia, ib) = (a.c.position(), b.c.position());
    let mut not_sent = 0;
    for burst in 0..17 {
        for n in (burst * 3)..((burst + 1) * 3).min(50) {
            for (from, to) in [(&mut a, "isolde"), (&mut b, "brynja")] {
                let t = from.cmd(&format!("/{to} ping {n}")).await;
                not_sent += t.matches("Telepath Not Sent").count();
            }
        }
        tick(&r, 3).await;
    }
    // a command of each returns after everything queued for it has arrived
    a.cmd("i").await;
    b.cmd("i").await;
    assert_eq!(not_sent, 0);
    let at_b = b.c.plain_from(ib);
    let at_a = a.c.plain_from(ia);
    for n in 0..50 {
        assert!(
            at_b.contains(&format!("Brynja telepaths: ping {n}\n")),
            "Isolde missed {n}"
        );
        assert!(
            at_a.contains(&format!("Isolde telepaths: ping {n}\n")),
            "Brynja missed {n}"
        );
    }
    assert_eq!(at_a.matches(" telepaths: ping ").count(), 50);
    assert_eq!(at_b.matches(" telepaths: ping ").count(), 50);
    a.leave_ticking(&r).await;
    b.leave_ticking(&r).await;
    r.stop().await.unwrap();
}
