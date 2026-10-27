//! T-148: the charged wearable items of the stock world over telnet. Each is worn and
//! used by hand (`USE {item}`): the use casts the item's spell, with its messages, and
//! takes one charge; with none left the item says it is used up and stays (all four are
//! kept when empty, so the winged sandals' Shatter does nothing here); below the item's
//! minimum level it can be neither worn nor used; a daily cleanup gives the charges back.
//!
//! - crimson bracers (425): blazing inferno on the room, 3 charges;
//! - ruby-encrusted bracers (1047, Warrior, level 50): bloodstone bracers, which goes on
//!   to ruby-encrusted fire or a dazing blast, 3 charges, +10% encumbrance;
//! - folds of worship (1313, Priest, level 40): +50 mana and +25 hit points while worn,
//!   the spell raises the mana regeneration, 3 charges;
//! - winged sandals (446): "You feel fast!", 10 charges.
//!
//! The fast tick is fed by the test ([`tick`]) and the cleanup is sent to the engine, so
//! nothing waits on a real clock; the server seed fixes the dice.

use crate::common;

use std::time::Duration;

use common::*;
use silvermere_types::ids::ItemNo;

const WARRIOR: u16 = 1;
const PRIEST: u16 = 5;
const ARENA: (u32, u32) = (1, 2150);
const GIANT_RAT: u16 = 1;

const CRIMSON: ItemNo = ItemNo(425);
const RUBY: ItemNo = ItemNo(1047);
const FOLDS: ItemNo = ItemNo(1313);
const SANDALS: ItemNo = ItemNo(446);

/// `EngineInput::DailyCleanup`, waiting until the run has finished.
async fn run_cleanup(r: &silvermere::Running) {
    on_world(r, |w| w.cleanup.last = None).await;
    r.to_engine
        .send(silvermere_engine::EngineInput::DailyCleanup)
        .unwrap();
    for _ in 0..500 {
        if on_world(r, |w| w.cleanup.last.is_some()).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the cleanup never finished");
}

/// The uses left on `name`'s copy of `item`, and whether it is worn (`None` when it is not
/// carried).
async fn state(r: &silvermere::Running, name: &str, item: ItemNo) -> Option<(i16, bool)> {
    let name = name.to_owned();
    on_world(r, move |w| {
        let p = w
            .session(session_of(w, &name))
            .unwrap()
            .character
            .as_ref()
            .unwrap();
        let slot = p.items.iter().position(|&i| i == item)?;
        Some((p.itemuses[slot], p.worn.contains(&item)))
    })
    .await
}

/// A new character `name` of `class` at `level` in the Arena, carrying one `item`; returns
/// the client and the item's name.
async fn holder(
    r: &silvermere::Running,
    url: &str,
    name: &str,
    class: u16,
    level: u16,
    item: ItemNo,
) -> (silvermere_testkit::TelnetClient, String) {
    let c = new_character(r, url, name, class, &[]).await;
    let who = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &who);
        w.session_mut(id).unwrap().character.as_mut().unwrap().level = level;
        silvermere_engine::items::equip::recalc(w, id);
    })
    .await;
    place(r, name, ARENA.0, ARENA.1).await;
    give(r, name, item.0, 1).await;
    let iname = on_world(r, move |w| w.items[&item].name.clone()).await;
    (c, iname)
}

/// The number after `label` in `text` (`stat`'s `Hits:   35/35` gives 35 and 35).
fn pair(text: &str, label: &str) -> (i32, i32) {
    let re = regex::Regex::new(&format!(r"{label}\s*\*?\s*(-?\d+)/\s*(-?\d+)")).unwrap();
    let m = re
        .captures(text)
        .unwrap_or_else(|| panic!("no {label} in:\n{text}"));
    (m[1].parse().unwrap(), m[2].parse().unwrap())
}

/// `USE {iname}`, with a tick for the lines that follow the cast; returns what arrived
/// once `cast` holds, or after a while.
async fn use_once(
    r: &silvermere::Running,
    c: &mut silvermere_testkit::TelnetClient,
    iname: &str,
    cast: impl Fn(&str) -> bool,
) -> String {
    let at = c.position();
    c.send_line(&format!("use {iname}")).await.unwrap();
    for _ in 0..250 {
        tick(r, 1).await;
        pump(c, 20).await;
        if cast(&c.plain_from(at)) {
            break;
        }
    }
    c.plain_from(at)
}

/// One charged item held by one character.
struct Held {
    name: &'static str,
    item: ItemNo,
    iname: String,
    charges: i16,
    /// an attack spell: a rat is put in the room for each use
    rat: bool,
    /// the lines of a use
    cast: fn(&str) -> bool,
}

/// Uses the item `charges` times, checking each use's lines and that it takes one charge;
/// then the empty use.
async fn use_up(r: &silvermere::Running, c: &mut silvermere_testkit::TelnetClient, h: &Held) {
    let Held {
        name,
        item,
        ref iname,
        charges,
        rat,
        cast,
    } = *h;
    for left in (0..charges).rev() {
        if rat {
            spawn(r, ARENA.0, ARENA.1, GIANT_RAT).await;
        }
        let out = use_once(r, c, iname, cast).await;
        assert!(cast(&out), "use with {} left:\n{out}", left + 1);
        assert_eq!(state(r, name, item).await, Some((left, true)), "{out}");
        tick(r, 10).await;
    }
    let out = cmd(c, &format!("use {iname}")).await;
    assert!(
        out.contains(&format!("There are no more uses in {iname}.")),
        "{out}"
    );
    assert_eq!(state(r, name, item).await, Some((0, true)), "kept: {out}");
}

/// After the cleanup the charges are back and the item works again.
async fn recharged(r: &silvermere::Running, c: &mut silvermere_testkit::TelnetClient, h: &Held) {
    let Held {
        name,
        item,
        ref iname,
        charges,
        rat,
        cast,
    } = *h;
    // played since the last cleanup: the run looks at this character
    edit_player(r, name, |p| p.sesstime = p.sesstime.max(2)).await;
    run_cleanup(r).await;
    assert_eq!(state(r, name, item).await, Some((charges, true)));
    if rat {
        spawn(r, ARENA.0, ARENA.1, GIANT_RAT).await;
    }
    let out = use_once(r, c, iname, cast).await;
    assert!(cast(&out), "{out}");
    assert_eq!(state(r, name, item).await, Some((charges - 1, true)));
}

/// Below the minimum level the item can be neither worn nor, when worn, used.
async fn too_low(
    r: &silvermere::Running,
    url: &str,
    class: u16,
    level: u16,
    item: ItemNo,
    charges: i16,
) {
    let (mut c, iname) = holder(r, url, "Lowe", class, level, item).await;
    let out = cmd(&mut c, &format!("wear {iname}")).await;
    assert!(out.contains("You may not wear that item!"), "{out}");
    assert_eq!(state(r, "Lowe", item).await, Some((charges, false)));
    // worn before the level was lost
    edit_player(r, "Lowe", move |p| {
        let free = p.worn.iter().position(|&i| i == ItemNo(0)).unwrap();
        p.worn[free] = item;
    })
    .await;
    let out = cmd(&mut c, &format!("use {iname}")).await;
    assert!(out.contains("You may not use that item!"), "{out}");
    assert_eq!(state(r, "Lowe", item).await, Some((charges, true)));
}

fn inferno(out: &str) -> bool {
    out.contains("You cross your bracers!")
        && regex::Regex::new(r"A blazing inferno sears the room for \d+ damage!")
            .unwrap()
            .is_match(out)
}

fn ruby(out: &str) -> bool {
    out.contains("You cross your bracers!")
        && (regex::Regex::new(r"A blazing inferno sears the room for \d+ damage!")
            .unwrap()
            .is_match(out)
            || out.contains("A magical blast erupts, dazing your foes!"))
}

fn worship(out: &str) -> bool {
    out.contains("You pull your cloak tight about you!")
        && out.contains("You feel at one with the gods!")
}

fn fast(out: &str) -> bool {
    out.contains("The wings on your sandals glow slightly!") && out.contains("You feel fast!")
}

#[tokio::test(flavor = "multi_thread")]
async fn crimson_bracers_cast_blazing_inferno() {
    let db = stock_db();
    let r = start(&db.url, 1481).await;
    let (mut c, iname) = holder(&r, &db.url, "Ann", WARRIOR, 5, CRIMSON).await;
    let out = cmd(&mut c, &format!("wear {iname}")).await;
    assert!(
        out.contains("You are now wearing crimson bracers."),
        "{out}"
    );
    let held = Held {
        name: "Ann",
        item: CRIMSON,
        iname: iname.clone(),
        charges: 3,
        rat: true,
        cast: inferno,
    };
    use_up(&r, &mut c, &held).await;
    recharged(&r, &mut c, &held).await;
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn ruby_encrusted_bracers_need_level_50() {
    let db = stock_db();
    let r = start(&db.url, 1482).await;
    let (mut c, iname) = holder(&r, &db.url, "Bob", WARRIOR, 50, RUBY).await;
    let before = cmd(&mut c, "i").await;
    let out = cmd(&mut c, &format!("wear {iname}")).await;
    assert!(
        out.contains("You are now wearing ruby-encrusted bracers."),
        "{out}"
    );
    // Alter Encumbrance 10: a tenth more can be carried
    let after = cmd(&mut c, "i").await;
    let cap = |t: &str| pair(t, "Encumbrance:").1;
    assert_eq!(cap(&after), cap(&before) * 110 / 100, "{before}\n{after}");
    let held = Held {
        name: "Bob",
        item: RUBY,
        iname: iname.clone(),
        charges: 3,
        rat: true,
        cast: ruby,
    };
    use_up(&r, &mut c, &held).await;
    recharged(&r, &mut c, &held).await;
    r.stop().await.unwrap();
}

// a limited item (one copy in the realm): a server of its own
#[tokio::test(flavor = "multi_thread")]
async fn ruby_encrusted_bracers_refuse_level_49() {
    let db = stock_db();
    let r = start(&db.url, 1485).await;
    too_low(&r, &db.url, WARRIOR, 49, RUBY, 3).await;
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn folds_of_worship_need_level_40() {
    let db = stock_db();
    let r = start(&db.url, 1483).await;
    let (mut c, iname) = holder(&r, &db.url, "Cat", PRIEST, 40, FOLDS).await;
    let before = cmd(&mut c, "stat").await;
    let out = cmd(&mut c, &format!("wear {iname}")).await;
    assert!(
        out.contains("You are now wearing folds of worship."),
        "{out}"
    );
    // Mana 50 and Alter HP 25, on the maximums
    let after = cmd(&mut c, "stat").await;
    assert_eq!(
        pair(&after, "Mana:").1,
        pair(&before, "Mana:").1 + 50,
        "{before}\n{after}"
    );
    assert_eq!(
        pair(&after, "Hits:").1,
        pair(&before, "Hits:").1 + 25,
        "{before}\n{after}"
    );
    // the spell's mana regeneration is on the character until it wears off
    fn rate(w: &mut silvermere_engine::world::World) -> bool {
        let p = w.session(session_of(w, "Cat")).unwrap();
        p.character
            .as_ref()
            .unwrap()
            .spells
            .iter()
            .any(|s| s.0 == 862)
    }
    assert!(!on_world(&r, rate).await);
    let held = Held {
        name: "Cat",
        item: FOLDS,
        iname: iname.clone(),
        charges: 3,
        rat: false,
        cast: worship,
    };
    use_up(&r, &mut c, &held).await;
    assert!(on_world(&r, rate).await, "folds of worship is on");
    recharged(&r, &mut c, &held).await;
    too_low(&r, &db.url, PRIEST, 39, FOLDS, 3).await;
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn winged_sandals_are_kept_when_used_up() {
    let db = stock_db();
    let r = start(&db.url, 1484).await;
    let (mut c, iname) = holder(&r, &db.url, "Dan", WARRIOR, 5, SANDALS).await;
    let out = cmd(&mut c, &format!("wear {iname}")).await;
    assert!(out.contains("You are now wearing winged sandals."), "{out}");
    let held = Held {
        name: "Dan",
        item: SANDALS,
        iname: iname.clone(),
        charges: 10,
        rat: false,
        cast: fast,
    };
    use_up(&r, &mut c, &held).await;
    // Shatter is not read for an item used by hand: the sandals stay, worn
    let out = cmd(&mut c, "i").await;
    assert!(out.contains("winged sandals (Feet)"), "{out}");
    recharged(&r, &mut c, &held).await;
    r.stop().await.unwrap();
}
