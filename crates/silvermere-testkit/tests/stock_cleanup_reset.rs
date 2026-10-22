//! T-141: the daily cleanup ends with the room reset the original's nightly restart gave,
//! on the stock world over telnet. The alder chest (item 974), placed in the Hidden Keep
//! Treasure Room (9/863), comes back after it is taken, and is not doubled when it is
//! still there; a padded vest ("Delete at cleanup") dropped on a floor is gone while an
//! ordinary item there is hidden. The player in the room stays on and is told nothing.
//!
//! T-142: the run also gives the shop slots with no restock interval the roll the restart
//! gave. In the Floating Platform Store (shop 122, 16/2663) slot 0 holds two of item 1294,
//! no interval, +100 at 100%: bought out, it is stocked again after the cleanup.
//! T-143: the run then fills the Shop rooms as the start-up does. The amazon Armour Shop's
//! front room (16/317, no keeper, three of its town's pool at most) is filled when cleared
//! and gets nothing when full; the Curio Shop (1/332, its keeper only) gets its keeper back
//! and nothing more.

use crate::common;

use std::time::Duration;

use common::*;
use silvermere_types::ids::{ItemNo, RoomKey, ShopNo};

const WARRIOR: u16 = 1;
const TREASURE: (u32, u32) = (9, 863);
const CHEST: ItemNo = ItemNo(974);
const PADDED_VEST: ItemNo = ItemNo(332);
/// Grey robes: gettable, not free, no abilities (the stock rations are "Delete at cleanup"
/// too).
const ROBES: ItemNo = ItemNo(9);
/// The Village Entrance.
const VILLAGE: (u32, u32) = (1, 2140);
/// The amazon town's Armour Shop, Front Room: a Shop with no keeper, `maxmonsters` 3.
const ARMOUR_SHOP: (u32, u32) = (16, 317);
/// The Curio Shop: a Shop whose pool gives nothing, only its keeper.
const CURIO_SHOP: (u32, u32) = (1, 332);

/// The active monsters in the room, in its list order.
async fn monsters(r: &silvermere::Running, at: (u32, u32)) -> Vec<i32> {
    on_world(r, move |w| {
        w.room_state[&RoomKey::new(at.0, at.1)]
            .monsters
            .iter()
            .map(|m| m.0)
            .filter(|&n| n != 0)
            .collect()
    })
    .await
}

/// (in view, hidden) copies of `item` in the room.
async fn copies(r: &silvermere::Running, at: (u32, u32), item: ItemNo) -> (usize, usize) {
    on_world(r, move |w| {
        let s = &w.room_state[&RoomKey::new(at.0, at.1)];
        let n = |v: &[silvermere_types::records::RoomItem]| {
            v.iter()
                .filter(|i| i.item == item)
                .map(|i| usize::try_from(i.count.max(0)).unwrap_or(0) + 1)
                .sum()
        };
        (n(&s.items), n(&s.hiditems))
    })
    .await
}

/// `EngineInput::DailyCleanup`, waiting until the run has finished.
async fn run_cleanup(r: &silvermere::Running) {
    let before = on_world(r, |w| w.cleanup.last.clone()).await;
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
    panic!("the cleanup never finished (last run {before:?})");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_alder_chest_is_back_after_cleanup() {
    let db = stock_db();
    let r = start(&db.url, 141).await;
    // the stock chest lies in the treasure room after the start-up
    let at_start = copies(&r, TREASURE, CHEST).await;
    assert_eq!(at_start.0 + at_start.1, 1, "{at_start:?}");
    let mut ann = new_character(&r, &db.url, "Ann", WARRIOR, &[]).await;
    place(&r, "Ann", TREASURE.0, TREASURE.1).await;
    // a cleanup with the chest still there puts no second one down
    run_cleanup(&r).await;
    let kept = copies(&r, TREASURE, CHEST).await;
    assert_eq!(kept.0 + kept.1, 1, "not doubled: {kept:?}");
    // the chest is taken
    on_world(&r, |w| {
        let s = w
            .room_state
            .get_mut(&RoomKey::new(TREASURE.0, TREASURE.1))
            .unwrap();
        for i in s.items.iter_mut().chain(s.hiditems.iter_mut()) {
            if i.item == CHEST {
                i.item = ItemNo(0);
                i.count = 0;
            }
        }
    })
    .await;
    assert_eq!(copies(&r, TREASURE, CHEST).await, (0, 0));
    let at = ann.position();
    run_cleanup(&r).await;
    let back = copies(&r, TREASURE, CHEST).await;
    assert_eq!(back.0 + back.1, 1, "the chest is back: {back:?}");
    let report = on_world(&r, |w| w.cleanup.last.clone().unwrap()).await;
    assert!(report.placed_items >= 1, "{report:?}");
    // Ann was told nothing and plays on
    assert_eq!(ann.plain_from(at).trim(), "");
    let out = cmd(&mut ann, "look").await;
    assert!(out.contains("[HP="), "{out}");
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_padded_vest_is_gone_after_cleanup() {
    let db = stock_db();
    let r = start(&db.url, 142).await;
    on_world(&r, |w| {
        let key = RoomKey::new(VILLAGE.0, VILLAGE.1);
        for item in [PADDED_VEST, ROBES] {
            silvermere_engine::items::room_items::add_item_to_room(w, key, item, false, -1);
        }
    })
    .await;
    assert_eq!(copies(&r, VILLAGE, PADDED_VEST).await, (1, 0));
    run_cleanup(&r).await;
    assert_eq!(copies(&r, VILLAGE, PADDED_VEST).await, (0, 0), "deleted");
    assert_eq!(copies(&r, VILLAGE, ROBES).await, (0, 1), "hidden");
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cleared_shop_room_is_filled_again() {
    let db = stock_db();
    // the start-up's monsters are taken out: every room is cleared
    let r = start(&db.url, 143).await;
    // the pools back, as at start-up (the harness empties them so nothing wanders in)
    on_world(&r, silvermere_engine::monsters::quickref::rebuild).await;
    assert!(monsters(&r, ARMOUR_SHOP).await.is_empty());
    let mut ann = new_character(&r, &db.url, "Ann", WARRIOR, &[]).await;
    place(&r, "Ann", ARMOUR_SHOP.0, ARMOUR_SHOP.1).await;
    let at = ann.position();
    run_cleanup(&r).await;
    let filled = monsters(&r, ARMOUR_SHOP).await;
    assert_eq!(filled.len(), 3, "up to the room's cap: {filled:?}");
    let report = on_world(&r, |w| w.cleanup.last.clone().unwrap()).await;
    assert!(report.shop_monsters >= 3, "{report:?}");
    // Ann sees them arrive as at start-up, with the start-up's arrival lines, and plays on
    let out = cmd(&mut ann, "look").await;
    assert!(out.contains("Also here:"), "{out}");
    let told = ann.plain_from(at);
    assert!(told.contains("into the room!"), "{told}");
    // full: the next run gives it nothing
    run_cleanup(&r).await;
    assert_eq!(monsters(&r, ARMOUR_SHOP).await, filled, "not doubled");
    r.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_keeper_only_shop_room_keeps_its_keeper() {
    let db = stock_db();
    let r = start(&db.url, 144).await;
    run_cleanup(&r).await;
    let keeper = monsters(&r, CURIO_SHOP).await;
    assert_eq!(keeper.len(), 1, "the keeper is back: {keeper:?}");
    run_cleanup(&r).await;
    assert_eq!(monsters(&r, CURIO_SHOP).await, keeper, "unchanged");
    r.stop().await.unwrap();
}

const PLATFORM_STORE: ShopNo = ShopNo(122);
const PLATFORM_ROOM: (u32, u32) = (16, 2663);
/// Slot 0 of the Floating Platform Store: item 1294, ceiling 2, no interval.
const UNTIMED_SLOT: usize = 0;

async fn shop_qty(r: &silvermere::Running, shop: ShopNo, slot: usize) -> u16 {
    on_world(r, move |w| w.shops[&shop].stock[slot].qty).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bought_out_untimed_shop_slot_is_back_after_cleanup() {
    let db = stock_db();
    let r = start(&db.url, 143).await;
    let (item, maxqty, regentime) = on_world(&r, |w| {
        let s = w.shops[&PLATFORM_STORE].stock[UNTIMED_SLOT];
        (s.item, s.maxqty, s.regentime)
    })
    .await;
    assert_eq!((item, maxqty, regentime), (ItemNo(1294), 2, 0));
    let name = on_world(&r, move |w| w.items[&item].name.clone()).await;
    assert_eq!(shop_qty(&r, PLATFORM_STORE, UNTIMED_SLOT).await, 2);
    let mut ann = new_character(&r, &db.url, "Ann", WARRIOR, &[]).await;
    edit_player(&r, "Ann", |p| {
        p.coins.runic = 1000;
        p.maxhp = 400;
        p.hp = 400;
    })
    .await;
    place(&r, "Ann", PLATFORM_ROOM.0, PLATFORM_ROOM.1).await;
    let mut log = String::new();
    for _ in 0..maxqty {
        log += &cmd(&mut ann, &format!("buy {name}")).await;
    }
    assert_eq!(carried(&r, "Ann", item.0).await, 2, "{log}");
    assert_eq!(shop_qty(&r, PLATFORM_STORE, UNTIMED_SLOT).await, 0, "{log}");
    run_cleanup(&r).await;
    assert_eq!(shop_qty(&r, PLATFORM_STORE, UNTIMED_SLOT).await, 2);
    let report = on_world(&r, |w| w.cleanup.last.clone().unwrap()).await;
    assert!(report.shop_slots_restocked >= 1, "{report:?}");
    let list = cmd(&mut ann, "list").await;
    assert!(list.contains(&name), "{list}");
    r.stop().await.unwrap();
}

// the Lairs are left to the running server's refill: a cleared one stays empty
#[tokio::test(flavor = "multi_thread")]
async fn the_lairs_are_left_alone() {
    let db = stock_db();
    let r = start(&db.url, 145).await;
    on_world(&r, silvermere_engine::monsters::quickref::rebuild).await;
    run_cleanup(&r).await;
    let filled = on_world(&r, |w| {
        w.rooms
            .iter()
            .filter(|(_, rm)| rm.room_type == silvermere_types::enums::RoomType::Lair)
            .filter(|(k, _)| w.room_state[*k].monsters.iter().any(|m| m.0 != 0))
            .count()
    })
    .await;
    assert_eq!(filled, 0);
    r.stop().await.unwrap();
}

// a kill just before the run does not hold the fill back: after the original's restart
// the kill's minute stamp lay ahead of the new start-up's minute, so it held nothing
#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_kill_does_not_hold_the_shop_fill_back() {
    let db = stock_db();
    let r = start(&db.url, 146).await;
    on_world(&r, silvermere_engine::monsters::quickref::rebuild).await;
    on_world(&r, |w| {
        let now = w.minute_stamp().max(1);
        w.room_state
            .get_mut(&RoomKey::new(ARMOUR_SHOP.0, ARMOUR_SHOP.1))
            .unwrap()
            .lastkill = now;
    })
    .await;
    run_cleanup(&r).await;
    assert_eq!(monsters(&r, ARMOUR_SHOP).await.len(), 3);
    r.stop().await.unwrap();
}
