//! T-056: the daily cleanup through the store. `EngineOutput::CleanupScan` goes to
//! `store`, which reads the stored characters and answers with `cleanup::on_scan`; the
//! updated records go back through the persistence worker and the lines the original
//! wrote to the BBS audit trail land in the `events` table.

mod common;

use std::time::Duration;

use silvermere::admin::DbEventLog;
use silvermere::bridge::load_snapshot;
use silvermere::store::{self, Job};
use silvermere_api::{EventLog, NewEvent};
use silvermere_db::repo::accounts::Account;
use silvermere_engine::{Engine, EngineInput, EngineOutput, SeededRng, SystemClock, World};
use silvermere_types::config::GameConfig;

#[tokio::test]
async fn cleanup_reads_the_store_and_writes_audit_events() {
    let (t, _url) = common::seeded(&[]).await;
    let db = t.db().clone();
    let acct = db
        .accounts()
        .insert(&Account::new("evan", "", jiff::Timestamp::now()))
        .await
        .unwrap();
    let mut c = common::stored_character("EVAN", "Evan");
    c.level = 5;
    c.evil = 50;
    c.sesstime = 240;
    c.lifemins = 10;
    let cid = db.characters().insert(acct, &c).await.unwrap();

    let world = World::load(GameConfig::default(), 8, load_snapshot(&db).await.unwrap()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut engine = Engine::new(world, SystemClock, SeededRng::new(1), tx);
    engine.handle(EngineInput::DailyCleanup);
    assert!(rx.try_iter().any(|o| o == EngineOutput::CleanupScan));

    let (persist, worker) =
        silvermere_db::Persist::spawn(db.clone(), silvermere_db::PersistConfig::default());
    let (jobs_tx, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (to_engine, from_store) = std::sync::mpsc::channel();
    let (flushed, _barriers) = tokio::sync::watch::channel(0u64);
    let store = tokio::spawn(store::run(
        db.clone(),
        persist,
        worker,
        jobs_rx,
        to_engine,
        flushed,
    ));
    jobs_tx.send(Job::CleanupScan).unwrap();
    let answer = tokio::task::spawn_blocking(move || {
        from_store
            .recv_timeout(Duration::from_secs(20))
            .expect("the store answers the scan")
    })
    .await
    .unwrap();
    engine.handle(answer);

    let log = DbEventLog::new(db.clone());
    for o in rx.try_iter() {
        match o {
            EngineOutput::Persist(r) => jobs_tx.send(Job::Persist(r)).unwrap(),
            EngineOutput::Event(ev) => {
                log.append(NewEvent::from_game_event(&ev)).await.unwrap();
            }
            _ => {}
        }
    }
    drop(jobs_tx);
    store.await.unwrap();

    // 120 minutes folded in, 5 evil points forgiven
    let row = db.characters().get(cid).await.unwrap().unwrap().character;
    assert_eq!((row.sesstime, row.lifemins, row.evil), (0, 130, 45));

    let events = db.events().recent(50).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.event.kind.as_str()).collect();
    for k in [
        "cleanup.start",
        "cleanup.items",
        "cleanup.users",
        "cleanup.complete",
    ] {
        assert!(kinds.contains(&k), "{k} in {kinds:?}");
    }
    let users = events
        .iter()
        .find(|e| e.event.kind == "cleanup.users")
        .unwrap();
    assert_eq!(users.event.payload["minutes"], 120);
    assert_eq!(users.event.payload["evil_forgiven"], 5);
    assert_eq!(users.event.session, None);
}
