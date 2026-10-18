//! T-113: the worker over a slow network. CI's PostgreSQL is on loopback, so these tests
//! reach it through a proxy that adds 15 ms each way (a 30 ms round trip, a hosted
//! database in another region). On SQLite there is no network and they return early.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use silvermere_db::repo::{MonsterState, RoomState};
use silvermere_db::{Persist, PersistConfig, TestDb, Write};
use silvermere_types::ids::{ItemNo, MonsterNo, RoomKey};
use silvermere_types::records::Coins;

const ONE_WAY: Duration = Duration::from_millis(15);

async fn slow_db() -> Option<(TestDb, silvermere_db::latency::LatencyProxy)> {
    let found = TestDb::postgres_with_latency(ONE_WAY)
        .await
        .expect("test database");
    if found.is_none() {
        eprintln!("SILVERMERE_DATABASE_URL names no PostgreSQL: nothing to measure");
    }
    found
}

/// A burst of 2,000 writes of four kinds over a 30 ms round trip commits in a number of
/// round trips set by the kinds, not the writes: one write per round trip would take
/// 2,000 or more (over a minute).
#[tokio::test]
async fn a_burst_of_writes_costs_round_trips_per_table_not_per_row() {
    let Some((test, proxy)) = slow_db().await else {
        return;
    };
    let mut seq = Seq::new(113);
    const ROOMS: u32 = 500;
    const MONSTERS: i32 = 300;
    const ITEMS: i32 = 200;
    let rooms: Vec<_> = (1..=ROOMS).map(|n| room(&mut seq, 1, n)).collect();
    let monsters: Vec<_> = (1..=MONSTERS).map(|n| monster(&mut seq, n)).collect();
    let items: Vec<_> = (1..=ITEMS).map(|n| item(&mut seq, n)).collect();
    let state = silvermere_db::repo::StateWrite::Reset;
    let started = Instant::now();
    test.rooms().put_many(&rooms, state).await.unwrap();
    test.monsters().put_many(&monsters, state).await.unwrap();
    test.items().put_many(&items, state).await.unwrap();
    let seeded = proxy.round_trips();
    eprintln!(
        "seeded {ROOMS} rooms, {MONSTERS} monsters, {ITEMS} items in {seeded} round trips, {:?}",
        started.elapsed()
    );

    let config = PersistConfig {
        batch_max: 4096,
        batch_window: Duration::from_millis(200),
        ..PersistConfig::default()
    };
    let (persist, worker) = Persist::spawn(Arc::clone(test.db()), config);
    let before = proxy.round_trips();
    let started = Instant::now();
    for n in 1..=1000 {
        persist
            .send(Write::ActiveMonsterUpdate(active_monster(&mut seq, n)))
            .await
            .unwrap();
    }
    for n in 1..=ROOMS {
        let mut s = RoomState::empty(RoomKey::new(1, n));
        s.coins = Coins {
            copper: n,
            ..Coins::default()
        };
        persist.send(Write::RoomState(s)).await.unwrap();
    }
    for n in 1..=MONSTERS {
        persist
            .send(Write::MonsterState(MonsterState {
                number: MonsterNo(n),
                curalive: 1,
                killed_at: 0,
            }))
            .await
            .unwrap();
    }
    for n in 1..=ITEMS {
        persist
            .send(Write::ItemActive {
                item: ItemNo(n),
                active: 3,
            })
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(60), persist.flush())
        .await
        .expect("flush within the timeout")
        .unwrap();
    let took = started.elapsed();
    let trips = proxy.round_trips() - before;
    eprintln!("2,000 writes committed in {trips} round trips, {took:?}");
    assert_eq!(persist.dropped(), 0);
    // per batch: begin, commit, and per kind its statements (active monsters: one
    // upsert, four child deletes, four child inserts; rooms: the id read, an upsert, two
    // deletes, two inserts; one each for the rest), each a prepare and an execute the
    // first time it is seen
    assert!(trips <= 60, "{trips} round trips");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(test.active_monsters().all().await.unwrap().len(), 1000);
    assert_eq!(
        test.room_state()
            .get(RoomKey::new(1, ROOMS))
            .await
            .unwrap()
            .unwrap()
            .coins
            .copper,
        ROOMS
    );
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}

/// T-113 e: the link to the database drops (the guard is lost) with the start-up flood
/// and the shutdown's character saves queued, then comes back. Everything queued commits
/// in a bounded number of round trips (a handful of batches, a few statements per table
/// each), well inside the shutdown barrier's 30 s, and nothing is dropped.
#[tokio::test]
async fn after_an_outage_the_queue_flushes_in_a_few_batches() {
    let Some((test, proxy)) = slow_db().await else {
        return;
    };
    let mut seq = Seq::new(115);
    const ROOMS: u32 = 3_300;
    const KNOWN: i32 = 400;
    const ACTIVE: i32 = 7_400;
    const PLAYERS: usize = 5;
    let state = silvermere_db::repo::StateWrite::Reset;
    let rooms: Vec<_> = (1..=ROOMS).map(|n| room(&mut seq, 1, n)).collect();
    test.rooms().put_many(&rooms, state).await.unwrap();
    let monsters: Vec<_> = (1..=KNOWN).map(|n| monster(&mut seq, n)).collect();
    test.monsters().put_many(&monsters, state).await.unwrap();
    let mut players = Vec::new();
    for i in 0..PLAYERS {
        let name = format!("p{i}");
        let acct = new_account(&test, &mut seq, &name).await;
        let c = character(&mut seq, &name, &format!("P{i}"));
        players.push((test.characters().insert(acct, &c).await.unwrap(), c));
    }

    let (persist, worker) = Persist::spawn(Arc::clone(test.db()), PersistConfig::default());
    let progress = persist.progress();
    // the link goes; the flood and the shutdown's character saves queue behind it
    proxy.set_blocked(true);
    for n in 1..=ACTIVE {
        persist
            .send(Write::ActiveMonsterUpdate(active_monster(&mut seq, n)))
            .await
            .unwrap();
    }
    for n in 1..=ROOMS {
        persist
            .send(Write::RoomState(RoomState::empty(RoomKey::new(1, n))))
            .await
            .unwrap();
    }
    for n in 1..=KNOWN {
        persist
            .send(Write::MonsterState(MonsterState {
                number: MonsterNo(n),
                curalive: 2,
                killed_at: 0,
            }))
            .await
            .unwrap();
    }
    for (id, c) in &players {
        let mut c = c.clone();
        c.level = 9;
        persist
            .send(Write::Character {
                id: *id,
                character: Box::new(c),
            })
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        persist.status() != silvermere_db::PersistStatus::Healthy,
        "the outage was seen"
    );
    let unsaved_at_outage = progress.unsaved();
    eprintln!("queued during the outage: {unsaved_at_outage:?}");
    proxy.set_blocked(false);
    let back = Instant::now();
    let before = proxy.round_trips();
    tokio::time::timeout(Duration::from_secs(120), persist.flush())
        .await
        .expect("flush within the timeout")
        .unwrap();
    let took = back.elapsed();
    let trips = proxy.round_trips() - before;
    eprintln!("after the link came back: everything saved in {took:?}, {trips} round trips");
    // about 11,100 writes in batches of 2,048: six batches of about 25 statements, each a
    // prepare and an execute the first time a statement is seen, plus reconnecting
    assert!(trips <= 400, "{trips} round trips");
    // far from the bound on CI's loopback plus 30 ms; the round trips are the assertion
    assert!(took < Duration::from_secs(60), "{took:?}");
    assert!(progress.unsaved().is_empty());
    assert_eq!(persist.dropped(), 0);
    for (id, _) in &players {
        let row = test.characters().get(*id).await.unwrap().unwrap();
        assert_eq!(row.character.level, 9);
    }
    drop(persist);
    worker.await.unwrap();
    test.close().await;
}
