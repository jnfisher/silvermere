//! Layer 3: `World::load` over the fixture world read through the `silvermere-db`
//! repositories on a temp SQLite.

mod common;

use silvermere::bridge::load_snapshot;
use silvermere_engine::World;
use silvermere_types::config::GameConfig;
use silvermere_types::ids::RoomKey;

#[tokio::test]
async fn load_through_the_repositories_populates_the_maps() {
    let (t, _url) = common::seeded(&[]).await;
    let snapshot = load_snapshot(t.db()).await.unwrap();
    assert_eq!(snapshot.rooms.len(), 4);
    let world = World::load(GameConfig::default(), 8, snapshot).unwrap();
    assert_eq!(world.rooms.len(), 4);
    assert_eq!(world.room_state.len(), 4);
    assert_eq!(world.items.len(), 2);
    assert_eq!(world.monsters.len(), 2);
    assert_eq!(world.classes.len(), 1);
    assert_eq!(world.races.len(), 1);
    assert!(world.topten.is_empty());
    assert_eq!(world.room(RoomKey::new(1, 2140)).unwrap().name, "Room 1");
}

#[tokio::test]
async fn reload_content_through_the_repositories_keeps_room_state() {
    let (t, _url) = common::seeded(&[]).await;
    let snapshot = load_snapshot(t.db()).await.unwrap();
    let mut world = World::load(GameConfig::default(), 8, snapshot).unwrap();
    let key = RoomKey::new(1, 2140);
    world.room_state.get_mut(&key).unwrap().searchers = 3;

    let mut renamed = t.db().rooms().get(key).await.unwrap().unwrap();
    renamed.name = "Town Square".into();
    t.db().rooms().put_content(&renamed).await.unwrap();
    let fresh = load_snapshot(t.db()).await.unwrap();
    world.reload_content(&fresh).unwrap();
    assert_eq!(world.room(key).unwrap().name, "Town Square");
    assert_eq!(world.room_state[&key].searchers, 3);
}

/// Bank books are read into the snapshot at start-up: after a restart the balance is
/// still there and the next deposit adds to it instead of upserting over the row.
#[tokio::test]
async fn bank_books_survive_a_restart() {
    use silvermere_db::repo::accounts::Account;
    use silvermere_engine::commands::bank::BankBooks;
    use silvermere_types::ids::ShopNo;
    use silvermere_types::records::BankBook;

    let (t, _url) = common::seeded(&[]).await;
    let acct = t
        .db()
        .accounts()
        .insert(&Account::new("ann", "x", jiff::Timestamp::UNIX_EPOCH))
        .await
        .unwrap();
    let book = |bank, balance| BankBook {
        userid: "ANN".into(),
        bank: ShopNo(bank),
        balance,
        loan: 0,
    };
    t.db().bank_books().put(acct, &book(7, 523)).await.unwrap();
    t.db().bank_books().put(acct, &book(9, 40)).await.unwrap();

    let snapshot = load_snapshot(t.db()).await.unwrap();
    assert_eq!(snapshot.bank_books.len(), 2);
    let mut world = World::load(GameConfig::default(), 8, snapshot).unwrap();
    assert_eq!(world.bank_books.balance_at(acct, ShopNo(7), 0), 523);
    assert_eq!(world.bank_books.balance_at(acct, ShopNo(9), 0), 40);
    // a deposit after the restart adds to the stored balance
    assert_eq!(
        world.bank_books.deposit(acct, "ANN", ShopNo(7), 77),
        Ok(600)
    );
}
