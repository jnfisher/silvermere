//! M4 sysop commands (T-057, from T-055), layer 3 over telnet on the stock world: a BBS
//! sysop (Sysadm), a game operator (Gil) and a player (Bob) on one server.
//!
//! Rules pinned: SYSOP GOTO's towns, GOD, LIGHTNING, LIST, ARENA and the role gates
//! (`has_permission`).

mod common;

use std::time::Duration;

use common::*;
use silvermere_auth::Role;
use silvermere_engine::commands::sysop::Key;

const WARRIOR: u16 = 1;

/// Sysop screen lines the screen-pattern registry has no row for yet: the bare `Done` of
/// the SYSOP sub-commands and the wrapped tail of the GOTO location list. Listed as
/// registry candidates in the PR; the
/// original prints the same lines, so the differential does not count them against the
/// new server.
const REGISTRY_GAPS: [&str; 2] = ["Done", "KHAZARAD, LOSTCITY"];

fn assert_clean(cl: &Classifier, out: &str, sent: &[&str]) {
    let unknown: Vec<String> = cl
        .unknown(out, sent)
        .into_iter()
        .filter(|l| !REGISTRY_GAPS.contains(&l.trim()))
        .collect();
    assert!(
        unknown.is_empty(),
        "unclassified lines: {unknown:#?}\n{out}"
    );
}

async fn where_is(r: &silvermere::Running, name: &str) -> (u32, u32) {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        let p = w.session(id).unwrap().character.as_ref().unwrap();
        (p.map.0, p.room.0)
    })
    .await
}

struct Cast {
    r: silvermere::Running,
    _db: StockDb,
    sysop: silvermere_testkit::TelnetClient,
    gameop: silvermere_testkit::TelnetClient,
    player: silvermere_testkit::TelnetClient,
    cl: Classifier,
}

async fn cast(seed: u64) -> Cast {
    let db = stock_db();
    let r = start(&db.url, seed).await;
    let sysop = new_character(&r, &db.url, "Sysadm", WARRIOR, &[Role::Sysop]).await;
    let gameop = new_character(&r, &db.url, "Gil", WARRIOR, &[Role::GameOp]).await;
    let player = new_character(&r, &db.url, "Bob", WARRIOR, &[]).await;
    let cl = Classifier::new(&r).await;
    Cast {
        r,
        _db: db,
        sysop,
        gameop,
        player,
        cl,
    }
}

/// sysop goto x6: the six towns of GOTO_PLACES, each arrived at silently; an unknown
/// place lists them (`Current GOTO locations`).
#[tokio::test(flavor = "multi_thread")]
async fn goto_the_six_towns() {
    let mut k = cast(41).await;
    let reinc = on_world(&k.r, |w| w.config.reinc_room).await;
    let mut log = String::new();
    for (town, map, room) in [
        ("newhaven", 1, 2150),
        ("silvermere", 1, reinc),
        ("support", 1, 164),
        ("rhudaur", 2, 2519),
        ("khazarad", 6, 1249),
        ("lostcity", 16, 426),
    ] {
        let out = cmd(&mut k.sysop, &format!("sysop goto {town}")).await;
        // the room screen follows on the next prompt
        let screen = cmd(&mut k.sysop, "").await;
        assert_eq!(
            where_is(&k.r, "Sysadm").await,
            (map, room),
            "{town}:\n{out}{screen}"
        );
        log += &out;
        log += &screen;
    }
    let none = cmd(&mut k.sysop, "sysop goto nowhere").await;
    assert!(none.contains("Current GOTO locations"), "{none}");
    log += &none;
    let sent = [
        "sysop goto newhaven",
        "sysop goto silvermere",
        "sysop goto support",
        "sysop goto rhudaur",
        "sysop goto khazarad",
        "sysop goto lostcity",
        "sysop goto nowhere",
    ];
    assert_clean(&k.cl, &log, &sent);
    k.r.stop().await.unwrap();
}

/// GOD info, add life and jail on another player; `Done` closes each change.
#[tokio::test(flavor = "multi_thread")]
async fn god_info_add_life_and_jail() {
    let mut k = cast(42).await;
    edit_player(&k.r, "Bob", |p| p.lives = 3).await;
    let at = k.sysop.position();
    cmd(&mut k.sysop, "sysop god bob info").await;
    // The reply ends at the status page's prompt; the spells, the profile, the abilities and
    // the closing prompt follow one part a fast tick. Left to the
    // wall clock, a part landed in a later command's reply, between the typed `jail` and
    // its CR echo, and the echo was never found (T-119).
    // From `at`: a wall-clock tick may already have brought the end while `cmd` read.
    tick(&k.r, 4).await;
    let end = k
        .sysop
        .wait_from(
            at,
            &silvermere_testkit::client::Wait {
                until: Some(format!(r"User abilities:[^\n]*\n{PROMPT}")),
                timeout: WAIT,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(end.matched, "GOD INFO did not end: {}", end.plain);
    let info = k.sysop.plain_from(at);
    assert!(info.contains("Bob"), "{info}");
    assert!(info.contains("Colour Palette:"), "{info}");
    let life = cmd(&mut k.sysop, "sysop god bob add life").await;
    assert!(life.contains("Done"), "{life}");
    let lives = on_world(&k.r, |w| {
        let id = session_of(w, "Bob");
        w.session(id).unwrap().character.as_ref().unwrap().lives
    })
    .await;
    assert_eq!(lives, 4);
    let jail = cmd(&mut k.sysop, "sysop god bob jail").await;
    assert!(jail.contains("Done"), "{jail}");
    assert!(
        jail.contains("Bob is knocked unconscious and dragged off to jail!"),
        "{jail}"
    );
    // the jail runs the stock text block 1510, `cast 935:cast 936`: spell 936 is a
    // Teleport Room with value 0, so the room is its rolled amount (minval..maxval)
    let (lo, hi) = on_world(&k.r, |w| {
        let spl = &w.spells[&silvermere_types::ids::SpellNo(936)];
        (spl.minval as u32, spl.maxval as u32)
    })
    .await;
    let mut at = (0, 0);
    for _ in 0..100 {
        at = where_is(&k.r, "Bob").await;
        if (lo..=hi).contains(&at.1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        (lo..=hi).contains(&at.1),
        "Bob at {at:?}, the jail is {lo}..={hi}:\n{jail}"
    );
    let bob = cmd(&mut k.player, "").await;
    assert_clean(
        &k.cl,
        &format!("{info}{life}{jail}{bob}"),
        &[
            "sysop god bob info",
            "sysop god bob add life",
            "sysop god bob jail",
        ],
    );
    k.r.stop().await.unwrap();
}

/// LIGHTNING at an evil player: his line, the room's line, `Done` for the sysop.
#[tokio::test(flavor = "multi_thread")]
async fn lightning_strikes_an_evil_player() {
    let mut k = cast(43).await;
    edit_player(&k.r, "Bob", |p| {
        p.evil = 250;
        p.maxhp = 400;
        p.hp = 400;
    })
    .await;
    let done = cmd(&mut k.sysop, "sysop lightning bob").await;
    assert!(done.contains("Done"), "{done}");
    let hit = k
        .player
        .until("A bolt of lightning from the heavens strikes you for", WAIT)
        .await
        .unwrap();
    let _ = hit;
    let bob = cmd(&mut k.player, "").await;
    assert_clean(&k.cl, &format!("{done}{bob}"), &["sysop lightning bob"]);
    k.r.stop().await.unwrap();
}

/// LIST SPELLS (developer gate: the DEBUG grant) and ARENA STATUS.
#[tokio::test(flavor = "multi_thread")]
async fn list_spells_and_arena_status() {
    let mut k = cast(44).await;
    on_world(&k.r, |w| {
        let id = session_of(w, "Sysadm");
        w.session_mut(id).unwrap().sysop_keys.insert(Key::Debug);
    })
    .await;
    let spells = cmd(&mut k.sysop, "sysop list spells").await;
    for name in ["vine strike", "fireball", "mend"] {
        assert!(spells.contains(name), "{name}:\n{spells}");
    }
    let arena = cmd(&mut k.sysop, "sysop arena status").await;
    // the stock arena mode, not the echo of the command
    assert!(
        arena.contains("Arena rooms are in normal mode (death does not count)"),
        "{arena}"
    );
    // the spell list is a table the registry has no rows for (one line per spell); the
    // arena line is checked
    assert_clean(&k.cl, &arena, &["sysop arena status"]);
    k.r.stop().await.unwrap();
}

/// Role refusals: a player's SYSOP is an unknown command; a game operator may GOTO but
/// not ARENA (sysop only) nor LIST SPELLS (developer); GOTO in a fight answers
/// `no_autocombat`.
#[tokio::test(flavor = "multi_thread")]
async fn role_refusals_and_goto_in_a_fight() {
    let mut k = cast(45).await;
    let player = cmd(&mut k.player, "sysop goto newhaven").await;
    assert_ne!(where_is(&k.r, "Bob").await, (1, 2150), "{player}");
    let goto = cmd(&mut k.gameop, "sysop goto newhaven").await;
    let _ = cmd(&mut k.gameop, "").await;
    assert_eq!(where_is(&k.r, "Gil").await, (1, 2150), "{goto}");
    let arena = cmd(&mut k.gameop, "sysop arena status").await;
    assert!(arena.contains("Command not recognized."), "{arena}");
    let list = cmd(&mut k.gameop, "sysop list spells").await;
    assert!(!list.contains("vine strike"), "{list}");

    // a fight: GOTO is refused while in auto-combat
    place(&k.r, "Sysadm", 1, 2150).await;
    let rat = spawn(&k.r, 1, 2150, 1).await;
    on_world(&k.r, move |w| {
        if let Some(a) = w.active_monsters.values_mut().find(|a| a.number == rat) {
            a.hp = 3000;
            a.maxhp = 3000;
        }
    })
    .await;
    edit_player(&k.r, "Sysadm", |p| {
        p.maxhp = 3000;
        p.hp = 3000;
    })
    .await;
    cmd(&mut k.sysop, "").await;
    let engaged = cmd_until(
        &mut k.sysop,
        "attack rat",
        r"\*Combat Engaged\*(?s:.*)\[HP=",
    )
    .await;
    assert!(engaged.contains("*Combat Engaged*"), "{engaged}");
    let refused = cmd(&mut k.sysop, "sysop goto silvermere").await;
    assert!(
        k.cl.names(&refused).iter().any(|n| n == "no_autocombat"),
        "{refused}"
    );
    assert_eq!(where_is(&k.r, "Sysadm").await, (1, 2150));
    assert_clean(
        &k.cl,
        &format!("{player}{goto}{arena}{list}{engaged}{refused}"),
        &[
            "sysop goto newhaven",
            "sysop arena status",
            "sysop list spells",
            "attack rat",
            "sysop goto silvermere",
        ],
    );
    k.r.stop().await.unwrap();
}
