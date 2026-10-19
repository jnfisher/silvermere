//! M4 cleanup and hang-up (T-057, from T-056), layer 3 over telnet on the stock world with
//! the real store behind the in-process server.
//!
//! Rules pinned: the hang-up penalty and its notice on the next entry; the daily cleanup
//! with players online (the host's `mcurou` call, T-056; John's decision: no downtime,
//! nobody disconnected) and its forgiveness and minute folding.

use crate::common;

use std::time::Duration;

use common::*;
use silvermere_types::config::PunHup;
use silvermere_types::ids::{ItemNo, RoomKey};

const WARRIOR: u16 = 1;
/// Darkwood Forest: neither protected nor an arena (a hang-up in a safe room only moves
/// the player to its hang-up room).
const FOREST: (u32, u32) = (1, 424);
const RATION: i32 = 282;

fn assert_clean(cl: &Classifier, out: &str, sent: &[&str]) {
    let unknown = cl.unknown(out, sent);
    assert!(
        unknown.is_empty(),
        "unclassified lines: {unknown:#?}\n{out}"
    );
}

async fn floor_count(r: &silvermere::Running, map: u32, room: u32, item: i32) -> i32 {
    on_world(r, move |w| {
        let s = &w.room_state[&RoomKey::new(map, room)];
        s.items
            .iter()
            .chain(s.hiditems.iter())
            .filter(|i| i.item == ItemNo(item))
            .map(|i| i32::from(i.count.max(1)))
            .sum()
    })
    .await
}

/// hangup: with `PUNHUP=HIGH`, Bob's socket drops in the middle of a fight. Ann, in the
/// same room, sees him go; his next entry shows the two notice lines
/// (`enter_hangup_note`, `enter_hangup_punished`) before the room, he has lost hit
/// points, and some of what he carried lies on the floor of the old room.
#[tokio::test(flavor = "multi_thread")]
async fn hangup_mid_fight_is_punished() {
    hangup_mid_fight(51, PunHup::High, false).await;
}

/// The same fight under `PUNHUP=MEDIUM`, with the room protected after the fight began
/// (in-game a fight starts in a protected room only under tournament play): auto-combat
/// makes a protected room unsafe and MEDIUM punishes it.
#[tokio::test(flavor = "multi_thread")]
async fn hangup_mid_fight_in_a_protected_room_is_punished_at_medium() {
    hangup_mid_fight(54, PunHup::Medium, true).await;
}

async fn hangup_mid_fight(seed: u64, pun_hup: PunHup, protected: bool) {
    let db = stock_db();
    let r = start_with(&db.url, seed, move |c| {
        c.game.pun_hup = pun_hup;
        // the stock WCCMMUD.MSG values are 0; a range and an item count to see them work
        c.game.min_hup_hp = 10;
        c.game.max_hup_hp = 30;
        c.game.max_hup_items = 3;
    })
    .await;
    let mut ann = new_character(&r, &db.url, "Ann", WARRIOR, &[]).await;
    let mut bob = new_character(&r, &db.url, "Bob", WARRIOR, &[]).await;
    for who in ["Ann", "Bob"] {
        place(&r, who, FOREST.0, FOREST.1).await;
        edit_player(&r, who, |p| {
            p.maxhp = 300;
            p.hp = 300;
        })
        .await;
    }
    give(&r, "Bob", RATION, 6).await;
    let rat = spawn(&r, FOREST.0, FOREST.1, 1).await;
    on_world(&r, move |w| {
        if let Some(a) = w.active_monsters.values_mut().find(|a| a.number == rat) {
            a.hp = 3000;
            a.maxhp = 3000;
        }
    })
    .await;
    let cl = Classifier::new(&r).await;
    cmd(&mut ann, "").await;
    cmd(&mut bob, "").await;
    let engaged = cmd_until(&mut bob, "attack rat", r"\*Combat Engaged\*").await;
    assert!(engaged.contains("*Combat Engaged*"), "{engaged}");
    if protected {
        on_world(&r, |w| {
            w.rooms
                .get_mut(&RoomKey::new(FOREST.0, FOREST.1))
                .unwrap()
                .flags
                .insert(silvermere_types::flags::RoomFlags::PROTECTED);
        })
        .await;
    }
    let hp_before = 300;
    let at = ann.position();
    bob.disconnect().await;
    ann.until("Bob just disconnected!!!", WAIT).await.unwrap();
    let seen = ann.plain_from(at);
    // Ann's line comes out of the same engine event as the hang-up's punishment and save
    let dropped = floor_count(&r, FOREST.0, FOREST.1, RATION).await;
    // T-075: straight back in, without waiting for the save to reach the store: the
    // login starts from the record the hang-up queued.
    let mut again = silvermere_testkit::TelnetClient::connect(r.telnet_addr)
        .await
        .unwrap()
        .with_naws(79, 24);
    login_to_menu(&mut again, "Bob", "bob1").await;
    let entry = cmd_until(&mut again, "E", &format!("Obvious exits(?s:.*){PROMPT}")).await;
    let note = entry
        .find("Last time you were on, you disconnected while playing.")
        .unwrap_or_else(|| panic!("no hang-up note (dropped {dropped}):\n{seen}\n---\n{entry}"));
    let punished = entry
        .find("The gods have punished you appropriately.")
        .unwrap_or_else(|| panic!("no punishment line:\n{entry}"));
    let room = entry.find("Obvious exits").unwrap();
    assert!(note < punished && punished < room, "{entry}");
    let names = cl.names(&entry);
    for want in ["enter_hangup_note", "enter_hangup_punished"] {
        assert!(names.iter().any(|n| n == want), "{want}:\n{entry}");
    }
    let (hp, carried_now) = on_world(&r, |w| {
        let id = session_of(w, "Bob");
        let p = w.session(id).unwrap().character.as_ref().unwrap();
        (
            p.hp,
            p.items.iter().filter(|i| i.0 == RATION).count() as i32,
        )
    })
    .await;
    assert!(hp < hp_before, "no hit points lost: {hp}");
    assert_eq!(
        dropped + carried_now,
        6,
        "rations: {dropped} on the floor, {carried_now} kept"
    );
    assert!(dropped > 0, "nothing left on the old room's floor");
    assert_clean(&cl, &format!("{seen}{entry}"), &["E"]);
    r.stop().await.unwrap();
}

/// A clean `x` logoff punishes nothing: the next entry has no notice.
#[tokio::test(flavor = "multi_thread")]
async fn clean_logoff_is_not_punished() {
    let db = stock_db();
    let r = start_with(&db.url, 52, |c| c.game.pun_hup = PunHup::High).await;
    let mut bob = new_character(&r, &db.url, "Bob", WARRIOR, &[]).await;
    cmd_until(&mut bob, "x", r"\[MAJORMUD\]:").await;
    let entry = cmd_until(&mut bob, "E", &format!("Obvious exits(?s:.*){PROMPT}")).await;
    assert!(!entry.contains("disconnected while playing"), "{entry}");
    r.stop().await.unwrap();
}

/// cleanup: the daily job forced while Ann plays in the Village Entrance. She stays
/// connected and keeps playing; her session minutes are folded into `lifemins` (half of
/// them) and `sesstime` is 0; the coins on the floor are hidden
/// and the gettable items on the floor go out of view (T-056 §5).
#[tokio::test(flavor = "multi_thread")]
async fn cleanup_with_a_player_online() {
    let db = stock_db();
    let r = start(&db.url, 53).await;
    let mut ann = new_character(&r, &db.url, "Ann", WARRIOR, &[]).await;
    place(&r, "Ann", 1, 2140).await;
    edit_player(&r, "Ann", |p| p.sesstime = 40).await;
    let lifemins = on_world(&r, |w| {
        let id = session_of(w, "Ann");
        w.session(id).unwrap().character.as_ref().unwrap().lifemins
    })
    .await;
    // the floor: rations and coins
    on_world(&r, |w| {
        let key = RoomKey::new(1, 2140);
        for _ in 0..4 {
            silvermere_engine::items::room_items::add_item_to_room(
                w,
                key,
                ItemNo(RATION),
                false,
                -2,
            );
        }
        w.room_state.get_mut(&key).unwrap().coins.copper = 25;
    })
    .await;
    let cl = Classifier::new(&r).await;
    let before = cmd(&mut ann, "look").await;
    assert!(before.contains("iron ration"), "{before}");
    let at = ann.position();
    r.to_engine
        .send(silvermere_engine::EngineInput::DailyCleanup)
        .unwrap();
    // the job asks the store for the stored characters and finishes on its answer
    let mut done = false;
    for _ in 0..500 {
        let s = on_world(&r, |w| {
            let id = session_of(w, "Ann");
            w.session(id).unwrap().character.as_ref().unwrap().sesstime
        })
        .await;
        if s == 0 {
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(done, "the cleanup never folded Ann's minutes");
    let (life, sess) = on_world(&r, |w| {
        let id = session_of(w, "Ann");
        let p = w.session(id).unwrap().character.as_ref().unwrap();
        (p.lifemins, p.sesstime)
    })
    .await;
    assert_eq!((life, sess), (lifemins + 20, 0));
    let (coins, hidcoins, visible) = on_world(&r, |w| {
        let s = &w.room_state[&RoomKey::new(1, 2140)];
        (
            s.coins.copper,
            s.hidcoins.copper,
            s.items.iter().filter(|i| i.item == ItemNo(RATION)).count(),
        )
    })
    .await;
    assert_eq!((coins, hidcoins), (0, 25), "the coins are hidden");
    assert_eq!(visible, 0, "the rations are out of view");
    // nothing was said to Ann, and she plays on
    let quiet = ann.plain_from(at);
    let after = cmd(&mut ann, "look").await;
    assert!(after.contains("Village Entrance"), "{after}");
    assert!(!after.contains("iron ration"), "{after}");
    assert_clean(&cl, &format!("{before}{quiet}{after}"), &["look"]);
    r.stop().await.unwrap();
}
