//! T-113 b: the cold start keeps only the last save of each row (`coalesce_saves`). The
//! database after the start-up's writes must be what writing every save leaves.

mod common;

use std::sync::Arc;

use silvermere::bridge::load_snapshot;
use silvermere::store::{self, Job};
use silvermere_db::{Db, Persist, PersistConfig};
use silvermere_engine::{Engine, EngineInput, EngineOutput, SeededRng, SystemClock, World};
use silvermere_types::config::GameConfig;
use sqlx::{Column, Row};

/// Every table the start-up pass writes.
const TABLES: &[&str] = &[
    "room_state",
    "room_floor_items",
    "room_monsters",
    "active_monsters",
    "active_monster_trail",
    "active_monster_hunters",
    "active_monster_items",
    "active_monster_spells",
    "monster_state",
    "shop_stock",
    "items",
];

/// The rows of `table`, each as text, sorted.
async fn rows(db: &Db, table: &str) -> Vec<String> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}")))
        .fetch_all(&db.pool())
        .await
        .unwrap();
    let mut out: Vec<String> = rows
        .iter()
        .map(|row| {
            row.columns()
                .iter()
                .map(|c| {
                    let i = c.ordinal();
                    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
                        format!("{v:?}")
                    } else if let Ok(v) = row.try_get::<Option<String>, _>(i) {
                        format!("{v:?}")
                    } else {
                        format!("{:?}", row.try_get::<Option<bool>, _>(i).ok().flatten())
                    }
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    out.sort();
    out
}

/// Starts the stock world (seeded) on a fresh copy, writes the start-up's saves through
/// the store, and returns the tables and how many saves there were.
async fn start(coalesce: bool) -> (Vec<Vec<String>>, usize) {
    let (_dir, url) = common::stock::fresh_db();
    let db = Arc::new(Db::connect(&url).await.unwrap());
    let snapshot = load_snapshot(&db).await.unwrap();
    let world = World::load_seeded(GameConfig::default(), 8, snapshot, 113, coalesce).unwrap();
    let (outputs, from_engine) = std::sync::mpsc::channel();
    let mut engine = Engine::new(world, SystemClock, SeededRng::new(1), outputs);
    engine.handle(EngineInput::Custom {
        session: None,
        run: Box::new(|_| {}),
    });

    let (persist, worker) = Persist::spawn(Arc::clone(&db), PersistConfig::default());
    let (jobs, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (to_engine, _answers) = std::sync::mpsc::channel();
    let (flushed, _) = tokio::sync::watch::channel(0u64);
    let store = tokio::spawn(store::run(
        Arc::clone(&db),
        persist,
        worker,
        jobs_rx,
        to_engine,
        flushed,
    ));
    let mut saves = 0;
    for o in from_engine.try_iter() {
        if let EngineOutput::Persist(r) = o {
            saves += 1;
            jobs.send(Job::Persist(r)).unwrap();
        }
    }
    drop(jobs);
    store.await.unwrap();
    let mut tables = Vec::new();
    for t in TABLES {
        tables.push(rows(&db, t).await);
    }
    db.close().await;
    (tables, saves)
}

#[tokio::test]
async fn coalescing_the_cold_start_saves_changes_nothing_stored() {
    let (every, all_saves) = start(false).await;
    let (coalesced, kept) = start(true).await;
    eprintln!("cold start of the stock world: {all_saves} saves, {kept} once coalesced");
    // about 22,000 saves of about 11,000 rows
    assert!(kept * 3 < all_saves * 2, "{kept} of {all_saves} kept");
    for (t, (a, b)) in TABLES.iter().zip(every.iter().zip(&coalesced)) {
        // new monsters have no trail or hunters yet
        let written = [
            "room_state",
            "room_monsters",
            "active_monsters",
            "monster_state",
        ];
        assert!(!a.is_empty() || !written.contains(t), "{t} written");
        assert!(a == b, "{t} differs: {} rows against {}", a.len(), b.len());
    }
}
