//! For each record type in `silvermere-types`: insert a fully populated value (arrays
//! filled to their `N*` sizes), read it back, assert `==`. Plus `Account`, the profile
//! (`PermanentRecord`), evil points (`EvilRecord`) and the three state structs.
//!
//! Runs on the backend `SILVERMERE_DATABASE_URL` names (a temp SQLite file by default).

use crate::common;

use common::*;
use silvermere_db::DbError;
use silvermere_db::repo::{MonsterState, RoomState, ShopStock};
use silvermere_types::ids::*;
use silvermere_types::sizes::*;

#[tokio::test]
async fn rooms_round_trip() {
    let db = db().await;
    let mut seq = Seq::new(1);
    let a = room(&mut seq, 1, 5);
    let b = room(&mut seq, 1, 2);
    let c = room(&mut seq, 2, 1);
    for r in [&a, &b, &c] {
        db.rooms().insert(r).await.unwrap();
    }
    assert_eq!(db.rooms().get(a.key).await.unwrap().as_ref(), Some(&a));
    assert_eq!(db.rooms().get(RoomKey::new(9, 9)).await.unwrap(), None);
    // all() in (map, room) order, all_in_map in room order
    let all = db.rooms().all().await.unwrap();
    assert_eq!(all, vec![b.clone(), a.clone(), c.clone()]);
    assert_eq!(
        db.rooms().all_in_map(MapId(1)).await.unwrap(),
        vec![b.clone(), a.clone()]
    );
    assert_eq!(
        db.rooms().highest_in_map(MapId(1)).await.unwrap(),
        Some(RoomId(5))
    );
    assert_eq!(db.rooms().highest_in_map(MapId(3)).await.unwrap(), None);
    // duplicate key
    assert!(matches!(
        db.rooms().insert(&a).await,
        Err(DbError::Duplicate(_))
    ));
    // a content refresh leaves the live state alone
    let mut a2 = room(&mut seq, 1, 5);
    a2.key = a.key;
    db.rooms().put_content(&a2).await.unwrap();
    let got = db.rooms().get(a.key).await.unwrap().unwrap();
    assert_eq!(got.name, a2.name);
    assert_eq!(got.exits, a2.exits);
    assert_eq!(got.coins, a.coins);
    assert_eq!(got.items, a.items);
    // RF_SPECALIVE is the one flag that is state; the rest are content
    let mut d = room(&mut seq, 3, 1);
    d.flags = silvermere_types::flags::RoomFlags::PROTECTED
        | silvermere_types::flags::RoomFlags::SPECALIVE;
    db.rooms().insert(&d).await.unwrap();
    assert!(db.room_state().get(d.key).await.unwrap().unwrap().specalive);
    assert_eq!(db.rooms().get(d.key).await.unwrap().unwrap().flags, d.flags);
    let mut d2 = d.clone();
    d2.flags = silvermere_types::flags::RoomFlags::PATROL;
    db.rooms().put_content(&d2).await.unwrap();
    assert_eq!(
        db.rooms().get(d.key).await.unwrap().unwrap().flags,
        silvermere_types::flags::RoomFlags::PATROL | silvermere_types::flags::RoomFlags::SPECALIVE
    );
    // room state round trip
    let state = RoomState::of(&c);
    assert_eq!(
        db.room_state().get(c.key).await.unwrap(),
        Some(state.clone())
    );
    let mut changed = RoomState::of(&a);
    changed.key = c.key;
    db.room_state().put(&changed).await.unwrap();
    assert_eq!(db.room_state().get(c.key).await.unwrap(), Some(changed));
    assert!(db.rooms().delete(c.key).await.unwrap());
    assert_eq!(db.room_state().get(c.key).await.unwrap(), None);
    db.close().await;
}

#[tokio::test]
async fn items_round_trip() {
    let db = db().await;
    let mut seq = Seq::new(2);
    let a = item(&mut seq, 300);
    let b = item(&mut seq, 7);
    db.items().insert(&a).await.unwrap();
    db.items().insert(&b).await.unwrap();
    assert_eq!(db.items().get(a.number).await.unwrap(), Some(a.clone()));
    assert_eq!(db.items().all().await.unwrap(), vec![b.clone(), a.clone()]);
    assert!(matches!(
        db.items().insert(&a).await,
        Err(DbError::Duplicate(_))
    ));
    let mut a2 = item(&mut seq, 300);
    a2.number = a.number;
    db.items().put(&a2).await.unwrap();
    assert_eq!(db.items().get(a.number).await.unwrap(), Some(a2));
    db.items().set_active(a.number, 3).await.unwrap();
    assert_eq!(db.items().get(a.number).await.unwrap().unwrap().active, 3);
    db.close().await;
}

// A monster's casting counts the chance of every slot, not only slots with a spell.
#[tokio::test]
async fn monster_spell_slot_without_a_spell_keeps_its_chance_and_level() {
    let db = db().await;
    let mut seq = Seq::new(31);
    let mut m = monster(&mut seq, 7);
    m.spells[3].spell = SpellNo(0);
    m.spells[3].pct = 40;
    m.spells[3].level = 12;
    m.spells[4].spell = SpellNo(0);
    m.spells[4].pct = 0;
    m.spells[4].level = 0;
    db.monsters().insert(&m).await.unwrap();
    assert_eq!(db.monsters().get(m.number).await.unwrap(), Some(m));
    db.close().await;
}

#[tokio::test]
async fn known_monster_and_state() {
    let db = db().await;
    let mut seq = Seq::new(3);
    let a = monster(&mut seq, 44);
    let b = monster(&mut seq, 3);
    db.monsters().insert(&a).await.unwrap();
    db.monsters().insert(&b).await.unwrap();
    assert_eq!(db.monsters().get(a.number).await.unwrap(), Some(a.clone()));
    assert_eq!(
        db.monsters().all().await.unwrap(),
        vec![b.clone(), a.clone()]
    );
    let state = MonsterState {
        number: a.number,
        curalive: 9,
        killed_at: 4_102_444_800,
    };
    db.monster_state().put(&state).await.unwrap();
    assert_eq!(db.monster_state().get(a.number).await.unwrap(), Some(state));
    let got = db.monsters().get(a.number).await.unwrap().unwrap();
    assert_eq!((got.curalive, got.killed_at), (9, 4_102_444_800));
    // a content refresh keeps the live state
    let mut a2 = monster(&mut seq, 44);
    a2.number = a.number;
    db.monsters().put(&a2).await.unwrap();
    let got = db.monsters().get(a.number).await.unwrap().unwrap();
    assert_eq!(got.attacks, a2.attacks);
    assert_eq!(got.curalive, 9);
    db.close().await;
}

#[tokio::test]
async fn active_monsters_round_trip() {
    let db = db().await;
    let mut seq = Seq::new(4);
    let a = active_monster(&mut seq, 12);
    let b = active_monster(&mut seq, 4);
    db.active_monsters().insert(&a).await.unwrap();
    db.active_monsters().insert(&b).await.unwrap();
    assert_eq!(
        db.active_monsters().get(a.number).await.unwrap(),
        Some(a.clone())
    );
    assert_eq!(
        db.active_monsters().all().await.unwrap(),
        vec![b.clone(), a.clone()]
    );
    assert_eq!(
        db.active_monsters().max_id().await.unwrap(),
        ActiveMonsterId(12)
    );
    let mut a2 = active_monster(&mut seq, 12);
    a2.number = a.number;
    db.active_monsters().update(&a2).await.unwrap();
    assert_eq!(db.active_monsters().get(a.number).await.unwrap(), Some(a2));
    assert!(db.active_monsters().delete(a.number).await.unwrap());
    assert!(!db.active_monsters().delete(a.number).await.unwrap());
    assert_eq!(
        db.active_monsters().max_id().await.unwrap(),
        ActiveMonsterId(4)
    );
    db.close().await;
}

#[tokio::test]
async fn spell_class_race() {
    let db = db().await;
    let mut seq = Seq::new(5);
    let sp = spell(&mut seq, 21);
    let sp2 = spell(&mut seq, 2);
    db.spells().insert(&sp).await.unwrap();
    db.spells().insert(&sp2).await.unwrap();
    assert_eq!(db.spells().get(sp.number).await.unwrap(), Some(sp.clone()));
    assert_eq!(
        db.spells().all().await.unwrap(),
        vec![sp2.clone(), sp.clone()]
    );
    assert_eq!(db.spells().max_number().await.unwrap(), SpellNo(21));

    let cl = class(&mut seq, 3);
    db.classes().insert(&cl).await.unwrap();
    assert_eq!(db.classes().get(cl.number).await.unwrap(), Some(cl.clone()));
    assert_eq!(db.classes().all().await.unwrap(), vec![cl.clone()]);
    let mut cl2 = class(&mut seq, 3);
    cl2.number = cl.number;
    db.classes().put(&cl2).await.unwrap();
    assert_eq!(db.classes().get(cl.number).await.unwrap(), Some(cl2));

    let ra = race(&mut seq, 6);
    db.races().insert(&ra).await.unwrap();
    assert_eq!(db.races().get(ra.number).await.unwrap(), Some(ra.clone()));
    assert_eq!(db.races().all().await.unwrap(), vec![ra]);
    db.close().await;
}

#[tokio::test]
async fn shop_and_stock() {
    let db = db().await;
    let mut seq = Seq::new(6);
    let a = shop(&mut seq, 8);
    let b = shop(&mut seq, 1);
    db.shops().insert(&a).await.unwrap();
    db.shops().insert(&b).await.unwrap();
    assert_eq!(db.shops().get(a.number).await.unwrap(), Some(a.clone()));
    assert_eq!(db.shops().all().await.unwrap(), vec![b.clone(), a.clone()]);
    assert_eq!(
        db.shop_stock().get(a.number).await.unwrap(),
        Some(ShopStock::of(&a))
    );
    let mut stock = ShopStock::of(&a);
    stock.qty = std::array::from_fn(|i| if i % 3 == 0 { 0 } else { i as u16 * 7 });
    db.shop_stock().put(&stock).await.unwrap();
    assert_eq!(db.shop_stock().get(a.number).await.unwrap(), Some(stock));
    assert_eq!(db.shop_stock().get(ShopNo(99)).await.unwrap(), None);
    // a content refresh keeps the live stock
    let mut a2 = shop(&mut seq, 8);
    a2.number = a.number;
    db.shops().put(&a2).await.unwrap();
    let got = db.shops().get(a.number).await.unwrap().unwrap();
    assert_eq!(got.name, a2.name);
    assert_eq!(got.stock[1].item, a2.stock[1].item);
    assert_eq!(got.stock[1].qty, stock.qty[1]);
    db.close().await;
}

#[tokio::test]
async fn message_text_block_action() {
    let db = db().await;
    let mut seq = Seq::new(7);
    let m = message(&mut seq, 500);
    let m2 = message(&mut seq, 20);
    db.messages().insert(&m).await.unwrap();
    db.messages().insert(&m2).await.unwrap();
    assert_eq!(db.messages().get(m.number).await.unwrap(), Some(m.clone()));
    assert_eq!(db.messages().all().await.unwrap(), vec![m2, m]);

    let t = text_block(&mut seq, 1, 77);
    assert!(t.text.len() >= TEXTSIZE - 100);
    db.text_blocks().insert(&t).await.unwrap();
    assert_eq!(
        db.text_blocks().part(1, t.number).await.unwrap(),
        Some(t.clone())
    );
    assert_eq!(db.text_blocks().all().await.unwrap(), vec![t.clone()]);
    assert!(matches!(
        db.text_blocks().insert(&t).await,
        Err(DbError::Duplicate(_))
    ));

    let act = action(&mut seq, "Wave");
    let act2 = action(&mut seq, "bow");
    db.actions().insert(&act).await.unwrap();
    db.actions().insert(&act2).await.unwrap();
    assert_eq!(
        db.actions().all().await.unwrap(),
        vec![act.clone(), act2.clone()]
    );
    assert_eq!(
        db.actions().by_name("WAVE").await.unwrap(),
        Some(act.clone())
    );
    assert!(matches!(
        db.actions().insert(&action(&mut seq, "wave")).await,
        Err(DbError::Duplicate(_))
    ));
    let mut act3 = action(&mut seq, "WAVE");
    act3.texts[4] = String::new();
    db.actions().put(&act3).await.unwrap();
    assert_eq!(db.actions().by_name("wave").await.unwrap(), Some(act3));
    db.close().await;
}

#[tokio::test]
async fn character_profile_evil() {
    let db = db().await;
    let mut seq = Seq::new(8);
    let acct = new_account(&db, &mut seq, "alice").await;
    let c = character(&mut seq, "alice", "Alice");
    let id = db.characters().insert(acct, &c).await.unwrap();
    let got = db.characters().get(id).await.unwrap().unwrap();
    assert_eq!(got.account, acct);
    assert_eq!(got.character, c);
    assert_eq!(
        db.characters()
            .by_account(acct)
            .await
            .unwrap()
            .map(|r| r.id),
        Some(id)
    );
    assert_eq!(db.characters().count().await.unwrap(), 1);
    // save_player rewrites every column and child table
    let mut c2 = character(&mut seq, "alice", "Alice");
    c2.items[3] = ItemNo(0);
    c2.itemuses[3] = 0;
    c2.spellbook[10] = SpellNo(0);
    assert!(db.characters().update(id, &c2).await.unwrap());
    assert_eq!(
        db.characters().get(id).await.unwrap().unwrap().character,
        c2
    );
    assert!(
        !db.characters()
            .update(CharacterId(9999), &c2)
            .await
            .unwrap()
    );
    // one character per account
    let other = character(&mut seq, "bob", "Bob");
    assert!(matches!(
        db.characters().insert(acct, &other).await,
        Err(DbError::Duplicate(_))
    ));

    let p = permanent_record(&mut seq, "alice");
    db.profiles().put(acct, &p).await.unwrap();
    assert_eq!(db.profiles().get(acct).await.unwrap(), Some(p.clone()));
    let mut p2 = permanent_record(&mut seq, "alice");
    p2.statfmt = "[%h/%H] ".to_string();
    db.profiles().put(acct, &p2).await.unwrap();
    assert_eq!(db.profiles().get(acct).await.unwrap(), Some(p2));

    let e = evil_record(&mut seq, "alice");
    db.evil_points().put(acct, &e).await.unwrap();
    assert_eq!(db.evil_points().get(acct).await.unwrap(), Some(e));
    assert!(db.evil_points().delete(acct).await.unwrap());
    assert_eq!(db.evil_points().get(acct).await.unwrap(), None);

    // deleting the account takes the character and profile with it
    assert!(db.accounts().delete(acct).await.unwrap());
    assert_eq!(db.characters().get(id).await.unwrap(), None);
    assert_eq!(db.profiles().get(acct).await.unwrap(), None);
    db.close().await;
}

#[tokio::test]
async fn gang_bank_book_item_ownership() {
    let db = db().await;
    let mut seq = Seq::new(9);
    let g = gang(&mut seq, "Shadows");
    db.gangs().insert(&g).await.unwrap();
    assert_eq!(
        db.gangs().by_name("shadows").await.unwrap(),
        Some(g.clone())
    );
    let mut g2 = gang(&mut seq, "Shadows");
    g2.exp = 5;
    db.gangs().update(&g2).await.unwrap();
    assert_eq!(db.gangs().all().await.unwrap(), vec![g2.clone()]);
    assert!(db.gangs().delete("SHADOWS").await.unwrap());
    assert_eq!(db.gangs().all().await.unwrap(), vec![]);

    let acct = new_account(&db, &mut seq, "carol").await;
    let b1 = bank_book(&mut seq, "carol", 8);
    let b2 = bank_book(&mut seq, "carol", 3);
    db.bank_books().put(acct, &b1).await.unwrap();
    db.bank_books().put(acct, &b2).await.unwrap();
    assert_eq!(
        db.bank_books().get(acct, ShopNo(8)).await.unwrap(),
        Some(b1.clone())
    );
    assert_eq!(
        db.bank_books().for_account(acct).await.unwrap(),
        vec![b2.clone(), b1.clone()]
    );
    let mut b1b = b1.clone();
    b1b.balance = 1;
    db.bank_books().put(acct, &b1b).await.unwrap();
    assert_eq!(
        db.bank_books().get(acct, ShopNo(8)).await.unwrap(),
        Some(b1b)
    );
    assert!(db.bank_books().delete(acct, ShopNo(8)).await.unwrap());
    assert_eq!(db.bank_books().for_account(acct).await.unwrap(), vec![b2]);

    let o1 = item_ownership(&mut seq, 10);
    let mut o2 = item_ownership(&mut seq, 11);
    o2.itemnum = o1.itemnum;
    let o3 = item_ownership(&mut seq, 12);
    for o in [&o1, &o2, &o3] {
        db.item_ownership().insert(o).await.unwrap();
    }
    assert_eq!(db.item_ownership().get(10).await.unwrap(), Some(o1.clone()));
    let mut holders = db.item_ownership().holders_of(o1.itemnum).await.unwrap();
    holders.sort_by_key(|o| o.id);
    assert_eq!(holders, vec![o1.clone(), o2.clone()]);
    assert_eq!(db.item_ownership().max_id().await.unwrap(), 12);
    assert!(matches!(
        db.item_ownership().insert(&o1).await,
        Err(DbError::Duplicate(_))
    ));
    assert!(db.item_ownership().delete(11).await.unwrap());
    assert_eq!(db.item_ownership().all().await.unwrap(), vec![o1, o3]);
    db.close().await;
}

#[tokio::test]
async fn accounts_round_trip() {
    let db = db().await;
    let mut seq = Seq::new(10);
    let a = account(&mut seq, "Dave");
    let id = db.accounts().insert(&a).await.unwrap();
    let got = db.accounts().by_username("dave").await.unwrap().unwrap();
    assert_eq!(got.id, id);
    assert_eq!(got.account, a);
    assert!(db.accounts().exists("DAVE").await.unwrap());
    assert!(!db.accounts().exists("erin").await.unwrap());
    assert!(matches!(
        db.accounts().insert(&account(&mut seq, "dAVE")).await,
        Err(DbError::Duplicate(_))
    ));
    let mut a2 = account(&mut seq, "Dave");
    a2.email = None;
    a2.last_login = None;
    a2.roles = vec!["player".to_string()];
    assert!(db.accounts().update(id, &a2).await.unwrap());
    assert_eq!(db.accounts().get(id).await.unwrap().unwrap().account, a2);
    db.close().await;
}

/// T-108: a key repeated in one bulk write keeps the last row and its children, as the
/// per-row writes did, on both backends (PostgreSQL refuses one statement updating a row
/// twice; the children would collide on their slots).
#[tokio::test]
async fn a_key_repeated_in_put_many_keeps_the_last_row() {
    let db = db().await;
    let mut seq = Seq::new(9);
    let first = item(&mut seq, 41);
    let mut last = item(&mut seq, 99);
    last.number = first.number;
    let other = item(&mut seq, 42);
    db.items()
        .put_many(
            &[first.clone(), other.clone(), last.clone()],
            silvermere_db::repo::StateWrite::CreateMissing,
        )
        .await
        .unwrap();
    assert_eq!(db.items().get(first.number).await.unwrap(), Some(last));
    assert_eq!(db.items().get(other.number).await.unwrap(), Some(other));

    let m1 = monster(&mut seq, 5);
    let mut m2 = monster(&mut seq, 6);
    m2.number = m1.number;
    db.monsters()
        .put_many(
            &[m1, m2.clone()],
            silvermere_db::repo::StateWrite::CreateMissing,
        )
        .await
        .unwrap();
    assert_eq!(db.monsters().get(m2.number).await.unwrap(), Some(m2));

    let s1 = spell(&mut seq, 3);
    let mut s2 = spell(&mut seq, 4);
    s2.number = s1.number;
    db.spells().put_many(&[s1, s2.clone()]).await.unwrap();
    assert_eq!(db.spells().get(s2.number).await.unwrap(), Some(s2));

    let a = message(&mut seq, 7);
    let mut b = message(&mut seq, 8);
    b.number = a.number;
    db.messages().put_many(&[a, b.clone()]).await.unwrap();
    assert_eq!(db.messages().all().await.unwrap(), vec![b]);

    let t1 = text_block(&mut seq, 1, 70);
    let mut t2 = text_block(&mut seq, 1, 71);
    t2.number = t1.number;
    db.text_blocks().put_many(&[t1, t2.clone()]).await.unwrap();
    assert_eq!(db.text_blocks().all().await.unwrap(), vec![t2]);
    db.close().await;
}
