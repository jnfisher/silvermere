//! T-113: over a slow database a name check does not wait behind the write-behind queue.
//! CI's PostgreSQL is reached through a proxy adding 15 ms each way (a 30 ms round trip);
//! on SQLite there is no network and the test returns early.
//!
//! The store's character reads lay the queued character writes over what they read
//! instead of flushing the worker, so a name a queued write holds reads as taken, a name
//! a queued write gives up reads as free, and the answer comes in a few round trips
//! while 20,000 monster saves are still queued.

mod common;

use std::time::{Duration, Instant};

use silvermere::store::{self, Job};
use silvermere_db::TestDb;
use silvermere_db::repo::accounts::Account;
use silvermere_engine::world::blank;
use silvermere_engine::{EngineInput, PersistRequest, SessionId};
use silvermere_types::ids::ActiveMonsterId;

const ONE_WAY: Duration = Duration::from_millis(15);
const SAVES: i32 = 20_000;

#[tokio::test]
async fn a_name_check_answers_while_the_queue_is_deep() {
    let Some((t, _proxy)) = TestDb::postgres_with_latency(ONE_WAY)
        .await
        .expect("test database")
    else {
        eprintln!("SILVERMERE_DATABASE_URL names no PostgreSQL: nothing to measure");
        return;
    };
    let db = t.db().clone();
    let account = db
        .accounts()
        .insert(&Account::new("aldric", "", jiff::Timestamp::now()))
        .await
        .unwrap();
    let stored = common::stored_character("aldric", "Aldo");
    db.characters().insert(account, &stored).await.unwrap();

    let (persist, worker) =
        silvermere_db::Persist::spawn(db.clone(), silvermere_db::PersistConfig::default());
    let watcher = persist.clone();
    let (jobs, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (to_engine, from_store) = std::sync::mpsc::channel();
    let (flushed, _) = tokio::sync::watch::channel(0u64);
    let store = tokio::spawn(store::run(
        db.clone(),
        persist,
        worker,
        jobs_rx,
        to_engine,
        flushed,
    ));

    // the start-up flood: every generated monster saved once
    for n in 1..=SAVES {
        let m = blank::clear_active_monster(ActiveMonsterId(n));
        jobs.send(Job::Persist(PersistRequest::ActiveMonster(Box::new(m))))
            .unwrap();
    }
    // a character save behind it gives up "Aldo" and takes "Zed"
    let mut renamed = stored.clone();
    renamed.firstname = "Zed".into();
    jobs.send(Job::Persist(PersistRequest::Character {
        account,
        character: Box::new(renamed),
    }))
    .unwrap();
    let asked = Instant::now();
    for (n, name) in [(1, "Zed"), (2, "Aldo"), (3, "Nobody")] {
        jobs.send(Job::NameCheck {
            id: SessionId(n),
            name: name.into(),
        })
        .unwrap();
    }

    let mut answered = 0;
    while answered < 3 {
        assert!(
            asked.elapsed() < Duration::from_secs(5),
            "{answered} of 3 name checks answered in 5 s"
        );
        match from_store.try_recv() {
            Ok(EngineInput::Custom {
                session: Some(_), ..
            }) => answered += 1,
            Ok(_) => {}
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let took = asked.elapsed();
    let queued = watcher.queued();
    eprintln!("3 name checks answered in {took:?} with {queued} writes still queued");
    assert!(queued > 1_000, "the queue was still deep: {queued}");

    // what the checks read: the queued save, not the stored row
    assert!(
        store::by_first_name(&db, &watcher, "zed")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store::by_first_name(&db, &watcher, "Aldo")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store::by_first_name(&db, &watcher, "Nobody")
            .await
            .unwrap()
            .is_none()
    );

    // once flushed, the store reads the same from the rows alone
    watcher.flush().await.unwrap();
    assert!(watcher.pending_characters().is_empty());
    assert!(
        db.characters()
            .by_first_name("Zed")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.characters()
            .by_first_name("Aldo")
            .await
            .unwrap()
            .is_none()
    );
    drop(watcher);
    drop(jobs);
    store.await.unwrap();
    t.close().await;
}
