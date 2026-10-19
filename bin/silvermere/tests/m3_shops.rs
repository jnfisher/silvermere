//! M3 shop tests (T-048): the Newhaven shops of the T-005 `shop` capture replayed against
//! the in-process server and compared byte for byte (ANSI stripped, then with colour kept).
//! Pins: the buy price, the sell price, and that a Hospital sells `healing` and `curing`
//! only.

mod common;

use common::m3::{
    PROMPT, Player, assert_classified, body_lines, classify, copies, edit_char, place, stored,
    stored_stock_total, team_db,
};
use common::stock::{WAIT, aldric_db, login_to_menu, start};
use silvermere_testkit::capture_compare::{assert_clean, replay_steps};
use silvermere_testkit::{Capture, TelnetClient, captures};
use silvermere_types::ids::ClassNo;

fn capture(name: &str) -> Capture {
    Capture::load(captures::captures_dir().join(name)).expect("capture")
}

/// The whole `shop` capture (steps 5-40): weapon shop list, two buys, sell, appraise,
/// wield, the armour shop list and padded set, the general store (too poor, not sold),
/// the healer's empty list.
#[tokio::test]
async fn shop_capture_replays_byte_equal() {
    let (_dir, url) = aldric_db();
    let cap = capture("shop");
    let r = start(&url).await;
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    login_to_menu(&mut c, "Aldric", "aldric1").await;
    c.send_line("E").await.unwrap();
    c.until(PROMPT, WAIT).await.unwrap();
    // Shop quantities depend on how often the original's start-up fill ran (the capture's
    // own mask list masks them in the gang scenario too); the item names and prices stay.
    let reports = replay_steps(&mut c, &cap, 5..41, &[QTY], WAIT).await;
    assert_clean("shop", &reports);
    r.stop().await.unwrap();
}

/// The quantity column of a shop row.
const QTY: &str = r"(?m)\d+(?= +(?:Free|\d+ \w+)\r?$)";

/// The Newhaven Spell Shop (1:2144) `list`, as a Druid of the first level saw it in the
/// original under MBBSEmu (ANSI stripped): the stock, the quantities and the per-class suffixes.
const SPELL_SHOP_LIST: &str = "The following items are for sale here:

Item                          Quantity    Price
------------------------------------------------------
scroll of magic missile       120          Free (You can't use)
scroll of illuminate          100          4 gold crowns (You can't use)
scroll of blur                105          Free (You can't use)
scroll of smite               60           8 gold crowns (You can't use)
scroll of cause harm          125          Free (You can't use)
scroll of minor healing       140          Free (You can't use)
scroll of bless               110          4 gold crowns (You can't use)
scroll of vine strike         118          Free
scroll of starlight           95           Free
scroll of mend                80           8 gold crowns (Too powerful)
songsheet of valour           100          4 gold crowns (You can't use)
songsheet of misfortune       82           8 gold crowns (You can't use)
songsheet of discord          65           8 gold crowns (You can't use)";

/// Shop LIST in the Spell Shop equals the original's lines for a Druid, and every line of
/// the reply is one the registry classifies (`shop_list_intro`, `shop_list_header`,
/// `shop_list_rule`, then `shop_row`).
#[tokio::test]
async fn spell_shop_list_matches_the_original() {
    let (_dir, url) = team_db();
    place(&url, "Aldric", 2144).await;
    edit_char(&url, "Aldric", |c| c.class = ClassNo(13)).await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let at = p.c.position();
    let text = p.cmd("list").await;
    let want: Vec<&str> = SPELL_SHOP_LIST.lines().collect();
    let got = body_lines(&text);
    // the echoed command comes first; the blank line between intro and header is dropped
    let want: Vec<&str> = want.into_iter().filter(|l| !l.is_empty()).collect();
    assert_eq!(&got[1..], &want[..], "{text}");
    let events = classify(&p.c.raw()[at..]);
    let names: Vec<&str> = events.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"shop_list_intro"), "{names:?}");
    assert!(names.contains(&"shop_list_header"), "{names:?}");
    assert!(names.contains(&"shop_list_rule"), "{names:?}");
    assert_eq!(names.iter().filter(|n| **n == "shop_row").count(), 13);
    assert_classified("spell shop list", &p.c.raw()[at..], &["list"]);
    r.stop().await.unwrap();
}

/// `buy quarterstaff` twice, `i`, then `buy club`, `appraise club` and `sell club`: the
/// appraisal and the sale name the same price, a second `sell club` finds
/// none; the saved record holds the two quarterstaffs and no club, and after a restart the
/// login shows the same `i`.
#[tokio::test]
async fn buy_twice_sell_once_appraise_once_and_persist() {
    let (_dir, url) = team_db();
    place(&url, "Aldric", 2141).await;
    let before = stored_stock_total(&url, 45).await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let mut seen = Vec::new();
    for _ in 0..2 {
        let at = p.c.position();
        let t = p.cmd("buy quarterstaff").await;
        assert_eq!(
            body_lines(&t),
            [
                "buy quarterstaff",
                "You just bought quarterstaff for nothing."
            ]
        );
        seen.extend(classify(&p.c.raw()[at..]).into_iter().map(|e| e.name));
    }
    assert_eq!(
        seen.iter().filter(|n| *n == "bought").count(),
        2,
        "{seen:?}"
    );
    let inv = p.cmd("i").await;
    assert!(inv.contains("You are carrying 2 quarterstaff"), "{inv}");
    let t = p.cmd("buy club").await;
    assert!(t.contains("You just bought club for nothing."), "{t}");
    let appraised = p.cmd("appraise club").await;
    let sold = p.cmd("sell club").await;
    let would = body_lines(&appraised);
    let did = body_lines(&sold);
    assert!(would[1].starts_with("You would get "), "{would:?}");
    let price = would[1]
        .trim_start_matches("You would get ")
        .trim_end_matches(" for your club.");
    assert_eq!(did[1], format!("You sold club for {price}."));
    let missing = p.cmd("sell club").await;
    assert!(
        missing.contains("You don't have club to sell!"),
        "{missing}"
    );
    p.leave().await;
    r.stop().await.unwrap();
    let rec = stored(&url, "Aldric").await;
    assert_eq!(copies(&rec, 100), 2, "two quarterstaffs saved");
    assert_eq!(copies(&rec, 90), 0, "the club is sold");
    // the shelves: two quarterstaffs gone, the club bought and put back
    assert_eq!(stored_stock_total(&url, 45).await, before - 2);
    // restart: the same inventory screen
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    let inv = p.cmd("i").await;
    assert!(inv.contains("You are carrying 2 quarterstaff"), "{inv}");
    p.leave().await;
    r.stop().await.unwrap();
}

/// `buy sowrd`, `list` outside a shop, `buy lantern` without the coins and `buy quarterstaff`
/// in the General Store (a shop refuses what it does not stock with the unknown-item
/// line): `buy_unknown` twice, `not_in_shop`, `buy_poor`, every line
/// classified.
#[tokio::test]
async fn shop_refusals() {
    let (_dir, url) = team_db();
    place(&url, "Aldric", 2141).await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    // the Weapons Shop, the Village Entrance, the Narrow Path, the General Store
    let mut route: Vec<(&[&str], &str, &str, &str)> = vec![
        (
            &[],
            "buy sowrd",
            "buy_unknown",
            "sowrd is not a known item.",
        ),
        (
            &["s"],
            "list",
            "not_in_shop",
            "You cannot LIST if you are not in a shop!",
        ),
        (
            &["w", "s"],
            "buy lantern",
            "buy_poor",
            "You cannot afford lantern.",
        ),
        (
            &[],
            "buy quarterstaff",
            "buy_unknown",
            "quarterstaff is not a known item.",
        ),
    ];
    for (moves, line, name, text) in route.drain(..) {
        for m in moves {
            p.cmd(m).await;
        }
        let at = p.c.position();
        let t = p.cmd(line).await;
        assert!(body_lines(&t).contains(&text.to_owned()), "{line}: {t}");
        let names: Vec<_> = classify(&p.c.raw()[at..])
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(names.iter().any(|n| n == name), "{line}: {names:?}");
        assert_classified(line, &p.c.raw()[at..], &[line]);
    }
    r.stop().await.unwrap();
}

/// The Healer (1:2190): `list` prints nothing (a Hospital sells `healing` and `curing`
/// only), `buy healing` heals a wounded character for the original's price,
/// `buy curing` cures the poison, and a second `buy curing` still takes the coins and says
/// he was not poisoned (sic). The purse is saved.
#[tokio::test]
async fn hospital_heals_and_cures() {
    let (_dir, url) = team_db();
    place(&url, "Aldric", 2190).await;
    edit_char(&url, "Aldric", |c| {
        c.coins.gold = 10;
        c.hp = 10;
        c.poison = 3;
    })
    .await;
    let r = start(&url).await;
    let mut p = Player::enter(&r, "Aldric").await;
    assert_eq!(body_lines(&p.cmd("list").await), ["list"]);
    for (line, want, name) in [
        (
            "buy healing",
            "You hand over 5 silver nobles and all your wounds are healed.",
            "healed_paid",
        ),
        (
            "buy curing",
            "You hand over 2 gold crowns, 5 silver nobles and your poisoning is cured.",
            "cured_paid",
        ),
        (
            "buy curing",
            "You hand over 1 gold crown, 5 silver nobles and find that you were not poisoned!",
            "cured_paid",
        ),
    ] {
        let at = p.c.position();
        let t = p.cmd(line).await;
        assert_eq!(body_lines(&t), [line, want]);
        let names: Vec<_> = classify(&p.c.raw()[at..])
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(names.iter().any(|n| n == name), "{names:?}");
    }
    assert!(
        p.cmd("wealth")
            .await
            .contains("Wealth: 550 copper farthings")
    );
    p.leave().await;
    r.stop().await.unwrap();
    let rec = stored(&url, "Aldric").await;
    assert_eq!(rec.hp, rec.maxhp);
    assert_eq!(rec.poison, 0);
    assert_eq!(rec.coins.gold, 5);
    assert_eq!(rec.coins.silver, 5);
}
