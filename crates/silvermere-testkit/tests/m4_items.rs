//! M4 items (T-057, from T-053), layer 3 over telnet on the stock world: a scroll learned
//! and its spell cast in a fight (the learn-then-cast end-to-end test the T-050 and T-053
//! reviews asked for), a potion drunk, a key used, a torch lit and burnt out, food eaten.
//!
//! Stock records: room 1:2144 Newhaven Spell Shop, 1:2150 Arena, 1:1309 Crypt, Sealed Tomb
//! (north: Key exit for item 177); items 142 scroll of vine strike, 175 torch, 177 bone
//! key, 223 minor healing potion, 282 iron ration; spell 25 vine strike; monster 1 giant
//! rat. Rules: using items, learning from scrolls, light, eat, drink, and the slow cycle
//! (light burns down).

use crate::common;

use std::time::Duration;

use common::*;
use silvermere_types::ids::SpellNo;

const DRUID: u16 = 13;
const SPELL_SHOP: (u32, u32) = (1, 2144);
const ARENA: (u32, u32) = (1, 2150);
const TOMB: (u32, u32) = (1, 1309);
const SCROLL: i32 = 142;
const TORCH: i32 = 175;
const BONE_KEY: i32 = 177;
const POTION: i32 = 223;
const RATION: i32 = 282;
const VINE: SpellNo = SpellNo(25);

async fn rowan(r: &silvermere::Running, url: &str) -> silvermere_testkit::TelnetClient {
    let c = new_character(r, url, "Rowan", DRUID, &[]).await;
    edit_player(r, "Rowan", |p| {
        p.coins.gold = 500;
        p.maxhp = 400;
        p.hp = 400;
    })
    .await;
    c
}

fn assert_clean(cl: &Classifier, out: &str, sent: &[&str]) {
    let unknown = cl.unknown(out, sent);
    assert!(
        unknown.is_empty(),
        "unclassified lines: {unknown:#?}\n{out}"
    );
    assert!(
        regex::Regex::new(&format!("{PROMPT}\\s*$"))
            .unwrap()
            .is_match(out.trim_end()),
        "does not end at a prompt:\n{out}"
    );
}

/// use_scroll, end to end: a level-1 druid with no spells buys the scroll of vine strike
/// in the Spell Shop and reads it (`learn_scroll`, the scroll used up), SPELLS lists vine
/// strike, a second scroll is `already_known` and stays; then in the Arena `vine rat`
/// engages and the learned spell is cast round after round until the rat dies.
#[tokio::test(flavor = "multi_thread")]
async fn scroll_learned_then_cast_in_a_fight() {
    let db = stock_db();
    let r = start(&db.url, 21).await;
    let mut c = rowan(&r, &db.url).await;
    place(&r, "Rowan", SPELL_SHOP.0, SPELL_SHOP.1).await;
    let cl = Classifier::new(&r).await;
    let mut log = String::new();
    let before = cmd(&mut c, "spells").await;
    assert!(before.contains("You have no spells."), "{before}");
    log += &cmd(&mut c, "buy scroll of vine strike").await;
    assert_eq!(carried(&r, "Rowan", SCROLL).await, 1, "{log}");
    let read = cmd(&mut c, "use scroll of vine strike").await;
    assert!(
        read.contains("You read scroll of vine strike and learn the spell vine strike."),
        "{read}"
    );
    assert!(
        cl.names(&read).iter().any(|n| n == "learn_scroll"),
        "{read}"
    );
    assert_eq!(carried(&r, "Rowan", SCROLL).await, 0);
    let spells = cmd(&mut c, "spells").await;
    assert!(spells.contains("vine strike"), "{spells}");
    log += &read;
    log += &spells;
    log += &cmd(&mut c, "buy scroll of vine strike").await;
    let again = cmd(&mut c, "read scroll of vine strike").await;
    assert!(
        again.contains("You realize that you already know this scroll!"),
        "{again}"
    );
    assert!(
        cl.names(&again).iter().any(|n| n == "already_known"),
        "{again}"
    );
    assert_eq!(carried(&r, "Rowan", SCROLL).await, 1);
    log += &again;
    let book = on_world(&r, |w| {
        let id = session_of(w, "Rowan");
        w.session(id)
            .unwrap()
            .character
            .as_ref()
            .unwrap()
            .spellbook
            .clone()
    })
    .await;
    assert!(book.contains(&VINE), "{book:?}");

    // the learned spell in a fight: engine ticks, the server's seeded RNG
    place(&r, "Rowan", ARENA.0, ARENA.1).await;
    let rat = spawn(&r, ARENA.0, ARENA.1, 1).await;
    cmd(&mut c, "").await;
    let t = ticker(&r, Duration::from_millis(15));
    let at = c.position();
    c.send_line("vine rat").await.unwrap();
    c.until(r"You gain \d+ experience\.(?s:.*)\[HP=", WAIT)
        .await
        .unwrap();
    t.abort();
    let fight = c.plain_from(at);
    assert!(fight.contains("*Combat Engaged*"), "{fight}");
    assert!(
        regex::Regex::new(r"You cast vine strike at [a-z ]*giant rat for \d+ damage!")
            .unwrap()
            .is_match(&fight),
        "the learned spell never landed:\n{fight}"
    );
    assert_eq!(monster_hp(&r, rat).await, None, "the rat is dead");
    let tail = cmd(&mut c, "").await;
    assert_clean(
        &cl,
        &format!("{log}{fight}{tail}"),
        &[
            "spells",
            "buy scroll of vine strike",
            "use scroll of vine strike",
            "read scroll of vine strike",
            "vine rat",
        ],
    );
    r.stop().await.unwrap();
}

/// use_potion: the stock minor healing potion drunk while hurt: `You drink the minor
/// healing potion.`, its One Time Cast heals (`receive_hp` or the spell's own line), and
/// when its uses are gone it is no longer carried.
#[tokio::test(flavor = "multi_thread")]
async fn potion_drunk_until_gone() {
    let db = stock_db();
    let r = start(&db.url, 22).await;
    let mut c = rowan(&r, &db.url).await;
    give(&r, "Rowan", POTION, 1).await;
    let charges = on_world(&r, |w| {
        w.items[&silvermere_types::ids::ItemNo(POTION)].charges
    })
    .await
    .max(1);
    let cl = Classifier::new(&r).await;
    let mut log = String::new();
    for i in 0..charges {
        edit_player(&r, "Rowan", |p| p.hp = 10).await;
        let out = cmd(&mut c, "drink minor healing potion").await;
        assert!(out.contains("You drink the minor healing potion."), "{out}");
        let hp = on_world(&r, |w| {
            let id = session_of(w, "Rowan");
            w.session(id).unwrap().character.as_ref().unwrap().hp
        })
        .await;
        assert!(hp > 10, "drink {i}: not healed:\n{out}");
        log += &out;
    }
    assert_eq!(carried(&r, "Rowan", POTION).await, 0, "{log}");
    let none = cmd(&mut c, "drink minor healing potion").await;
    assert!(!none.contains("You drink"), "{none}");
    assert_clean(
        &cl,
        &format!("{log}{none}"),
        &["drink minor healing potion"],
    );
    r.stop().await.unwrap();
}

/// use_key: `use bone key n` in the Sealed Tomb unlocks the Key exit north; `open n` and
/// `n` reach the Shadowed Hall.
#[tokio::test(flavor = "multi_thread")]
async fn bone_key_unlocks_the_sealed_tomb() {
    let db = stock_db();
    let r = start(&db.url, 23).await;
    let mut c = rowan(&r, &db.url).await;
    edit_player(&r, "Rowan", |p| p.level = 10).await;
    place(&r, "Rowan", TOMB.0, TOMB.1).await;
    give(&r, "Rowan", BONE_KEY, 1).await;
    give(&r, "Rowan", TORCH, 1).await;
    // the test is about the key: the tomb's mummy, made on entry, goes (still counted as
    // alive, so none is made again), so no round of its interleaves with the answers
    on_world(&r, |w| {
        let here: Vec<_> = w
            .active_monsters
            .values()
            .filter(|a| a.map.0 == TOMB.0 && a.room.0 == TOMB.1)
            .map(|a| a.number)
            .collect();
        for n in here {
            silvermere_engine::monsters::active::remove_active_monster(w, n, false);
        }
        if let Some(s) = w
            .room_state
            .get_mut(&silvermere_types::ids::RoomKey::new(TOMB.0, TOMB.1))
        {
            s.specalive = true;
        }
    })
    .await;
    let cl = Classifier::new(&r).await;
    // the crypt is dark
    let lit = cmd(&mut c, "light torch").await;
    let locked = cmd(&mut c, "open n").await;
    assert!(locked.contains("locked"), "{locked}");
    let used = cmd(&mut c, "use bone key n").await;
    assert!(
        used.contains("You successfully unlocked the door."),
        "{used}"
    );
    let open = cmd(&mut c, "open n").await;
    // a move lands on the fast tick after its first prompt: wait for the room to change
    let (moved, at) = walk(&r, &mut c, "Rowan", "n").await;
    assert_eq!(at, 1377, "{open}{moved}");
    assert!(moved.contains("Crypt, Shadowed Hall"), "{open}{moved}");
    assert_clean(
        &cl,
        &format!("{lit}{locked}{used}{open}{moved}"),
        &["light torch", "open n", "use bone key n", "n"],
    );
    r.stop().await.unwrap();
}

/// use_light: `light torch` (`light_ok`), `light` tells the level (`light_level`), a
/// second light source is refused while one burns, and the torch burns out on the slow
/// cycle.
#[tokio::test(flavor = "multi_thread")]
async fn torch_lit_then_burnt_out() {
    let db = stock_db();
    let r = start(&db.url, 24).await;
    let mut c = rowan(&r, &db.url).await;
    give(&r, "Rowan", TORCH, 2).await;
    let cl = Classifier::new(&r).await;
    let lit = cmd(&mut c, "light torch").await;
    assert!(lit.contains("You lit the torch."), "{lit}");
    let level = cmd(&mut c, "light").await;
    assert!(level.contains("The current light level is"), "{level}");
    for (text, want) in [(&lit, "light_ok"), (&level, "light_level")] {
        assert!(cl.names(text).iter().any(|n| n == want), "{want}: {text}");
    }
    let second = cmd(&mut c, "use torch").await;
    assert!(
        second.contains("You already have something lit!"),
        "{second}"
    );
    // burn it down: the slow cycle runs every 30 fast ticks
    let at = c.position();
    let mut burnt = false;
    for _ in 0..200 {
        tick(&r, 30).await;
        if carried(&r, "Rowan", TORCH).await == 1 {
            burnt = true;
            break;
        }
    }
    let tail = cmd(&mut c, "").await;
    let burn = c.plain_from(at);
    assert!(burnt, "the torch never burnt out:\n{burn}");
    assert_clean(
        &cl,
        &format!("{lit}{level}{second}{burn}{tail}"),
        &["light torch", "light", "use torch", ""],
    );
    r.stop().await.unwrap();
}

/// eat: the stock iron ration: `You eat the iron ration.` and it is gone.
#[tokio::test(flavor = "multi_thread")]
async fn iron_ration_eaten() {
    let db = stock_db();
    let r = start(&db.url, 25).await;
    let mut c = rowan(&r, &db.url).await;
    give(&r, "Rowan", RATION, 1).await;
    let cl = Classifier::new(&r).await;
    let out = cmd(&mut c, "eat iron ration").await;
    assert!(out.contains("You eat the iron ration."), "{out}");
    assert_eq!(carried(&r, "Rowan", RATION).await, 0);
    assert_clean(&cl, &out, &["eat iron ration"]);
    r.stop().await.unwrap();
}
