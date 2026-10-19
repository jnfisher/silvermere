//! T-113 e: the shutdown's persistence barrier gives up only when the database stops
//! taking writes, and then names what was not saved, per kind, players first.

use crate::common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use silvermere_db::repo::RoomState;
use silvermere_db::{Persist, PersistConfig, TestDb, Write};
use silvermere_types::ids::RoomKey;

fn config() -> PersistConfig {
    PersistConfig {
        batch_window: Duration::from_millis(10),
        retry_min: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        ..PersistConfig::default()
    }
}

#[tokio::test]
async fn a_barrier_the_database_never_reaches_names_the_writes_not_saved() {
    let t = TestDb::sqlite_temp().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let db = Arc::clone(t.db());
    let (persist, worker) = Persist::spawn(Arc::clone(&db), config());
    // the outage: every batch fails from now on
    db.close().await;
    let room = common::rooms()[0].key;
    for _ in 0..3 {
        persist
            .send(Write::RoomState(RoomState::empty(room)))
            .await
            .unwrap();
    }
    persist
        .send(Write::EvilPointsDelete(silvermere_types::ids::AccountId(1)))
        .await
        .unwrap();
    let (_flushed_tx, mut flushed) = tokio::sync::watch::channel(0u64);
    let started = Instant::now();
    let err = silvermere::barrier(
        &mut flushed,
        &persist.progress(),
        Duration::from_millis(300),
        Duration::from_secs(10),
    )
    .await
    .expect_err("the barrier is not reached");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    let msg = err.to_string();
    // players' records first; the saves of one room the batch in hand coalesced count
    // once (how many it had taken depends on timing)
    assert!(msg.contains("(evil_points 1, room_state "), "{msg}");
    let rooms: u64 = msg
        .split("room_state ")
        .nth(1)
        .and_then(|r| r.trim_end_matches(')').parse().ok())
        .unwrap_or_else(|| panic!("{msg}"));
    assert!((1..=3).contains(&rooms), "{msg}");
    assert!(
        msg.contains(&format!("{} writes not saved", rooms + 1)),
        "{msg}"
    );
    drop(persist);
    worker.abort();
}

#[tokio::test]
async fn a_barrier_waits_past_the_quiet_time_while_writes_are_being_saved() {
    let t = TestDb::sqlite_temp().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let (persist, worker) = Persist::spawn(Arc::clone(t.db()), config());
    let (flushed_tx, mut flushed) = tokio::sync::watch::channel(0u64);
    let room = common::rooms()[0].key;
    // a slow drain: a write saved every 100 ms for 1.5 s, then the barrier
    let drain = tokio::spawn({
        let persist = persist.clone();
        async move {
            for i in 0..15u32 {
                let mut s = RoomState::empty(RoomKey::new(room.map.0, room.room.0));
                s.lastkill = i as u16;
                persist.send(Write::RoomState(s)).await.unwrap();
                persist.flush().await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            flushed_tx.send_modify(|n| *n += 1);
        }
    });
    let started = Instant::now();
    silvermere::barrier(
        &mut flushed,
        &persist.progress(),
        Duration::from_millis(400),
        Duration::from_secs(10),
    )
    .await
    .expect("the barrier is reached");
    assert!(started.elapsed() > Duration::from_millis(1_000));
    drain.await.unwrap();
    drop(persist);
    worker.await.unwrap();
    t.close().await;
}

/// The database goes away for less than the barrier's quiet time and comes back: the
/// barrier waits it out and is reached once the queue is saved. Nothing counts as
/// progress while the database is away (the worker retrying one batch saves nothing).
#[tokio::test]
async fn a_barrier_waits_out_an_outage_shorter_than_its_quiet_time() {
    let t = TestDb::sqlite_temp().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let db = Arc::clone(t.db());
    let (persist, worker) = Persist::spawn(Arc::clone(&db), config());
    let progress = persist.progress();
    let room = common::rooms()[0].key;
    db.close().await;
    for i in 0..20u16 {
        let mut s = RoomState::empty(room);
        s.lastkill = i;
        persist.send(Write::RoomState(s)).await.unwrap();
    }
    let (flushed_tx, mut flushed) = tokio::sync::watch::channel(0u64);
    let barrier = {
        let persist = persist.clone();
        tokio::spawn(async move {
            persist.flush().await.unwrap();
            flushed_tx.send_modify(|n| *n += 1);
        })
    };
    let outage = tokio::spawn({
        let db = Arc::clone(&db);
        let progress = progress.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            assert_eq!(
                progress.saved(),
                0,
                "no progress while the database is away"
            );
            db.reconnect().await.unwrap();
        }
    });
    let started = Instant::now();
    silvermere::barrier(
        &mut flushed,
        &progress,
        Duration::from_millis(1_500),
        Duration::from_secs(10),
    )
    .await
    .expect("the barrier is reached after the outage");
    assert!(started.elapsed() >= Duration::from_millis(600));
    outage.await.unwrap();
    barrier.await.unwrap();
    assert!(progress.unsaved().is_empty());
    assert!(progress.saved() >= 1);
    drop(persist);
    worker.await.unwrap();
    t.close().await;
}
