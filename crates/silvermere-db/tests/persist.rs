//! The write-behind worker: throughput, last-write-wins, outage recovery, status.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use silvermere_db::repo::RoomState;
use silvermere_db::{Persist, PersistConfig, PersistStatus, Write};
use silvermere_types::ids::{ItemNo, RoomKey};
use silvermere_types::records::Coins;

const ROOMS: u32 = 50;

async fn seed_rooms(db: &silvermere_db::Db, seq: &mut Seq) {
    for n in 1..=ROOMS {
        let mut r = room(seq, 1, n);
        // no floor items: the state write is the upsert alone
        r.items = [silvermere_types::records::RoomItem {
            item: ItemNo(0),
            uses: 0,
            count: 0,
        }; silvermere_types::sizes::NROOMITEMS];
        r.hiditems = [silvermere_types::records::RoomItem {
            item: ItemNo(0),
            uses: 0,
            count: 0,
        }; silvermere_types::sizes::NHIDITEMS];
        db.rooms().insert(&r).await.unwrap();
    }
}

fn state(n: u32, copper: u32) -> RoomState {
    let mut s = RoomState::empty(RoomKey::new(1, n));
    s.coins = Coins {
        copper,
        ..Coins::default()
    };
    s.lastkill = (copper % 60000) as u16;
    s
}

/// 10,000 queued room-state writes flush within the test timeout and the last write to
/// each room wins.
#[tokio::test]
async fn ten_thousand_room_state_writes_flush_and_the_last_wins() {
    let test = db().await;
    let mut seq = Seq::new(30);
    seed_rooms(&test, &mut seq).await;
    let db: Arc<silvermere_db::Db> = Arc::clone(test.db());
    let (persist, worker) = Persist::spawn(db, PersistConfig::default());
    for i in 0..10_000u32 {
        let n = i % ROOMS + 1;
        persist
            .send(Write::RoomState(state(n, i + 1)))
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(120), persist.flush())
        .await
        .expect("flush within the timeout")
        .unwrap();
    assert_eq!(persist.status(), PersistStatus::Healthy);
    assert_eq!(persist.dropped(), 0);
    for n in 1..=ROOMS {
        let got = test
            .room_state()
            .get(RoomKey::new(1, n))
            .await
            .unwrap()
            .unwrap();
        // the last i with i % ROOMS + 1 == n is 9_999 - ((9_999 - (n - 1)) % ROOMS)
        let last_i = 9_999 - ((9_999 + 1 - n) % ROOMS);
        assert_eq!(got.coins.copper, last_i + 1, "room {n}");
        assert_eq!(got.lastkill, ((last_i + 1) % 60000) as u16);
    }
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}

/// `flush()` after a simulated outage (the pool closed, then reopened) completes with no
/// write lost, and `PersistStatus` flips to `Unhealthy` and back.
#[tokio::test]
async fn outage_is_retried_and_status_flips() {
    let test = db().await;
    let mut seq = Seq::new(31);
    seed_rooms(&test, &mut seq).await;
    let db: Arc<silvermere_db::Db> = Arc::clone(test.db());
    let config = PersistConfig {
        batch_window: Duration::from_millis(20),
        retry_min: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        ..PersistConfig::default()
    };
    let (persist, worker) = Persist::spawn(Arc::clone(&db), config);
    let mut status = persist.watch();

    // a write that commits while healthy
    persist.send(Write::RoomState(state(1, 11))).await.unwrap();
    persist.flush().await.unwrap();
    assert_eq!(persist.status(), PersistStatus::Healthy);

    // the outage: every later batch fails until the pool is reopened
    db.close().await;
    for i in 0..200u32 {
        persist
            .send(Write::RoomState(state(i % ROOMS + 1, 1000 + i)))
            .await
            .unwrap();
    }
    let flush = tokio::spawn({
        let persist = persist.clone();
        async move { persist.flush().await }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            status.changed().await.unwrap();
            if matches!(*status.borrow(), PersistStatus::Unhealthy { .. }) {
                break;
            }
        }
    })
    .await
    .expect("status flips to Unhealthy");
    let PersistStatus::Unhealthy { failures, error } = persist.status() else {
        panic!("expected Unhealthy");
    };
    assert!(failures >= 1);
    assert!(!error.is_empty());
    assert!(
        !flush.is_finished(),
        "flush must wait for the outage to end"
    );

    // recovery
    tokio::time::sleep(Duration::from_millis(150)).await;
    db.reconnect().await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), flush)
        .await
        .expect("flush completes after the outage")
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !matches!(*status.borrow(), PersistStatus::Healthy) {
            status.changed().await.unwrap();
        }
    })
    .await
    .expect("status flips back to Healthy");
    assert_eq!(persist.status(), PersistStatus::Healthy);
    assert_eq!(persist.dropped(), 0);
    for n in 1..=ROOMS {
        let got = test
            .room_state()
            .get(RoomKey::new(1, n))
            .await
            .unwrap()
            .unwrap();
        let last_i = 199 - ((199 + 1 - n) % ROOMS);
        assert_eq!(got.coins.copper, 1000 + last_i, "room {n}");
    }
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}

/// A write the database refuses (a room that does not exist) is skipped and counted; the
/// writes around it still commit.
#[tokio::test]
async fn refused_write_is_skipped_not_retried_forever() {
    let test = db().await;
    let mut seq = Seq::new(32);
    seed_rooms(&test, &mut seq).await;
    let db: Arc<silvermere_db::Db> = Arc::clone(test.db());
    let (persist, worker) = Persist::spawn(db, PersistConfig::default());
    persist.send(Write::RoomState(state(1, 5))).await.unwrap();
    persist.send(Write::RoomState(state(999, 5))).await.unwrap();
    persist.send(Write::RoomState(state(2, 6))).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), persist.flush())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persist.dropped(), 1);
    assert_eq!(persist.status(), PersistStatus::Healthy);
    assert_eq!(
        test.room_state()
            .get(RoomKey::new(1, 1))
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        5
    );
    assert_eq!(
        test.room_state()
            .get(RoomKey::new(1, 2))
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        6
    );
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}

/// Every kind of write goes through the worker.
#[tokio::test]
async fn every_write_kind_applies() {
    let test = db().await;
    let mut seq = Seq::new(33);
    let acct = new_account(&test, &mut seq, "wendy").await;
    let c = character(&mut seq, "wendy", "Wendy");
    let cid = test.characters().insert(acct, &c).await.unwrap();
    let it = item(&mut seq, 1);
    test.items().insert(&it).await.unwrap();
    let km = monster(&mut seq, 1);
    test.monsters().insert(&km).await.unwrap();
    let sh = shop(&mut seq, 1);
    test.shops().insert(&sh).await.unwrap();

    let db: Arc<silvermere_db::Db> = Arc::clone(test.db());
    let (persist, worker) = Persist::spawn(db, PersistConfig::default());
    let mut c2 = c.clone();
    c2.level = 42;
    let am = active_monster(&mut seq, 7);
    let mut am2 = am.clone();
    am2.hp = 1;
    let am3 = active_monster(&mut seq, 8);
    let g = gang(&mut seq, "Hawks");
    let own = item_ownership(&mut seq, 5);
    let mut stock = silvermere_db::repo::ShopStock::of(&sh);
    stock.qty[0] = 77;
    let writes = vec![
        Write::Character {
            id: cid,
            character: Box::new(c2.clone()),
        },
        Write::ItemActive {
            item: it.number,
            active: 9,
        },
        Write::MonsterState(silvermere_db::repo::MonsterState {
            number: km.number,
            curalive: 3,
            killed_at: 1_760_400_000,
        }),
        Write::ShopStock(stock),
        Write::ActiveMonsterInsert(am.clone()),
        Write::ActiveMonsterUpdate(am2.clone()),
        Write::ActiveMonsterInsert(am3.clone()),
        Write::ActiveMonsterDelete(am3.number),
        Write::Profile {
            account: acct,
            record: permanent_record(&mut seq, "wendy"),
        },
        Write::EvilPoints {
            account: acct,
            record: evil_record(&mut seq, "wendy"),
        },
        Write::BankBook {
            account: acct,
            book: bank_book(&mut seq, "wendy", 8),
        },
        Write::BankBook {
            account: acct,
            book: bank_book(&mut seq, "wendy", 9),
        },
        Write::BankBookDelete {
            account: acct,
            bank: silvermere_types::ids::ShopNo(9),
        },
        Write::Gang(g.clone()),
        Write::Gang(gang(&mut seq, "Doves")),
        Write::GangDelete("doves".to_string()),
        Write::ItemOwnershipInsert(own.clone()),
        Write::ItemOwnershipInsert(item_ownership(&mut seq, 6)),
        Write::ItemOwnershipDelete(6),
        Write::Event(silvermere_db::Event::new(
            at(1),
            "login",
            serde_json::json!({"who": "Wendy"}),
        )),
    ];
    for w in writes {
        persist.send(w).await.unwrap();
    }
    persist.flush().await.unwrap();
    assert_eq!(persist.dropped(), 0);
    assert_eq!(
        test.characters().get(cid).await.unwrap().unwrap().character,
        c2
    );
    assert_eq!(
        test.items().get(it.number).await.unwrap().unwrap().active,
        9
    );
    assert_eq!(
        test.monster_state()
            .get(km.number)
            .await
            .unwrap()
            .unwrap()
            .curalive,
        3
    );
    assert_eq!(
        test.shop_stock().get(sh.number).await.unwrap().unwrap().qty[0],
        77
    );
    assert_eq!(test.active_monsters().all().await.unwrap(), vec![am2]);
    assert!(test.profiles().get(acct).await.unwrap().is_some());
    assert!(test.evil_points().get(acct).await.unwrap().is_some());
    assert_eq!(test.bank_books().for_account(acct).await.unwrap().len(), 1);
    assert_eq!(test.gangs().all().await.unwrap(), vec![g]);
    assert_eq!(test.item_ownership().all().await.unwrap(), vec![own]);
    assert_eq!(test.events().count().await.unwrap(), 1);
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}

/// A write that meets a lock held past `busy_timeout` (SQLite BUSY) is not a refusal: it
/// is retried until the lock goes away, and then commits. Nothing is dropped.
#[tokio::test]
async fn a_write_that_meets_a_held_lock_is_retried_not_dropped() {
    let test = db().await;
    if test.db().dialect() != silvermere_db::Dialect::Sqlite {
        eprintln!("skipped: needs SQLite's single-writer lock");
        test.close().await;
        return;
    }
    let mut seq = Seq::new(33);
    seed_rooms(&test, &mut seq).await;
    let db: Arc<silvermere_db::Db> = Arc::clone(test.db());
    // Shorten busy_timeout on the other pooled connections: a file database's pool holds
    // four (`max` in `open_pool`, pool.rs), one of which is the locker below, so three
    // others. Then keep the locker inside a write transaction.
    let mut locker = db.pool().acquire().await.unwrap();
    {
        let mut others = Vec::new();
        for _ in 0..3 {
            let mut c = db.pool().acquire().await.unwrap();
            sqlx::query("PRAGMA busy_timeout = 100")
                .execute(&mut *c)
                .await
                .unwrap();
            others.push(c);
        }
    }
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *locker)
        .await
        .unwrap();
    let (persist, worker) = Persist::spawn(db, PersistConfig::default());
    persist.send(Write::RoomState(state(1, 5))).await.unwrap();
    // The worker has met the lock: it either dropped the write or is retrying it.
    tokio::time::timeout(Duration::from_secs(30), async {
        while persist.dropped() == 0 && persist.status() == PersistStatus::Healthy {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the worker meets the lock");
    sqlx::query("ROLLBACK").execute(&mut *locker).await.unwrap();
    drop(locker);
    tokio::time::timeout(Duration::from_secs(30), persist.flush())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persist.dropped(), 0, "a BUSY write must not be dropped");
    assert_eq!(persist.status(), PersistStatus::Healthy);
    assert_eq!(
        test.room_state()
            .get(RoomKey::new(1, 1))
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        5
    );
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}

/// A SQLite file on a full filesystem (`SQLITE_FULL`) is an outage: the write is retried,
/// not dropped, and once there is room it commits and reads back. SQLite rolls the
/// transaction back by itself when the disk fills; the connection it ran on must not go
/// back to the pool, or the retry reports a commit that never happened and leaves a
/// transaction open that holds the write lock.
///
/// Mounts a small tmpfs, so it needs root and Linux: `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "mounts a tmpfs: needs root"]
async fn a_full_disk_is_retried_and_leaves_no_open_transaction() {
    use std::io::Write as _;
    use std::process::Command;

    struct Mount(std::path::PathBuf);
    impl Drop for Mount {
        fn drop(&mut self) {
            let _ = Command::new("umount").arg("-l").arg(&self.0).status();
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    let dir = std::env::temp_dir().join(format!("silvermere-fulldisk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let status = Command::new("mount")
        .args(["-t", "tmpfs", "-o", "size=16m", "tmpfs"])
        .arg(&dir)
        .status()
        .expect("mount");
    assert!(status.success(), "mounting a tmpfs needs root");
    let _mount = Mount(dir.clone());

    let url = format!("sqlite://{}", dir.join("test.db").display());
    let db = Arc::new(silvermere_db::Db::connect_and_migrate(&url).await.unwrap());
    let mut seq = Seq::new(34);
    seed_rooms(&db, &mut seq).await;
    // Warm every pooled connection, then take all the space left.
    let warm: Vec<_> = {
        let mut v = Vec::new();
        for _ in 0..4 {
            v.push(db.pool().acquire().await.unwrap());
        }
        v
    };
    drop(warm);
    let filler_path = dir.join("filler");
    let mut filler = std::fs::File::create(&filler_path).unwrap();
    let chunk = [0u8; 512];
    while filler.write_all(&chunk).is_ok() {}
    drop(filler);

    let (persist, worker) = Persist::spawn(Arc::clone(&db), PersistConfig::default());
    for n in 1..=ROOMS {
        persist.send(Write::RoomState(state(n, 5))).await.unwrap();
    }
    // Let the retries meet the full disk several times before there is room.
    tokio::time::timeout(Duration::from_secs(30), async {
        while !matches!(persist.status(), PersistStatus::Unhealthy { failures, .. } if failures >= 6)
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a full disk keeps the status Unhealthy");
    assert_eq!(
        persist.dropped(),
        0,
        "a write that met a full disk is not a refusal"
    );
    // Make room; the retry commits.
    std::fs::remove_file(&filler_path).unwrap();
    tokio::time::timeout(Duration::from_secs(30), persist.flush())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persist.dropped(), 0);
    assert_eq!(persist.status(), PersistStatus::Healthy);
    let got = db
        .room_state()
        .get(RoomKey::new(1, 1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.coins.copper, 5, "the retried write is in the database");
    // No pooled connection is left inside a transaction.
    let mut conns = Vec::new();
    for _ in 0..4 {
        conns.push(db.pool().acquire().await.unwrap());
    }
    for c in &mut conns {
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut **c)
            .await
            .expect("no connection is left in an open transaction");
        sqlx::query("ROLLBACK").execute(&mut **c).await.unwrap();
    }
    drop(conns);
    drop(persist);
    worker.await.unwrap();
    db.close().await;
}

/// T-113: on PostgreSQL a character deleted directly (account delete, cleanup) while a
/// save of it is being written stays deleted. The save's existence read locks the row,
/// so it waits for the delete and finds no row (refused, skipped) rather than upserting
/// the character back.
#[tokio::test]
async fn a_character_deleted_while_its_save_is_written_stays_deleted() {
    let test = db().await;
    if test.dialect() != silvermere_db::Dialect::Postgres {
        return;
    }
    let mut seq = Seq::new(116);
    let acct = new_account(&test, &mut seq, "dora").await;
    let c = character(&mut seq, "dora", "Dora");
    let id = test.characters().insert(acct, &c).await.unwrap();

    // the delete, not yet committed
    let mut deleting = test.pool().begin().await.unwrap();
    sqlx::query("DELETE FROM characters WHERE id = $1")
        .bind(id.0)
        .execute(&mut *deleting)
        .await
        .unwrap();

    let (persist, worker) = Persist::spawn(Arc::clone(test.db()), PersistConfig::default());
    let mut saved = c.clone();
    saved.level = 7;
    persist
        .send(Write::Character {
            id,
            character: Box::new(saved),
        })
        .await
        .unwrap();
    let flush = tokio::spawn({
        let persist = persist.clone();
        async move { persist.flush().await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    deleting.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), flush)
        .await
        .expect("flush")
        .unwrap()
        .unwrap();
    assert!(test.characters().get(id).await.unwrap().is_none());
    assert_eq!(
        persist.dropped(),
        1,
        "the save of a deleted character is refused"
    );
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}
