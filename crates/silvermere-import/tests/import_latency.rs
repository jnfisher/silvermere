//! T-113: the stock import against a PostgreSQL 30 ms away (CI's, through a proxy that
//! adds 15 ms each way) finishes well inside a minute and writes the rows a local import
//! writes. On SQLite there is no network and the test returns early.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use silvermere_db::{Db, TestDb};
use silvermere_import::importer::{ImportOptions, Importer};

#[path = "../../silvermere-testkit/src/stock.rs"]
#[allow(dead_code)]
mod stock_data;

const ONE_WAY: Duration = Duration::from_millis(15);

/// Every table the import writes.
const TABLES: &[&str] = &[
    "rooms",
    "room_exits",
    "room_placed_items",
    "room_state",
    "room_floor_items",
    "room_monsters",
    "items",
    "item_abilities",
    "item_classes",
    "item_races",
    "item_negates",
    "monsters",
    "monster_abilities",
    "monster_attacks",
    "monster_items",
    "monster_spells",
    "monster_state",
    "spells",
    "spell_abilities",
    "classes",
    "class_abilities",
    "races",
    "race_abilities",
    "shops",
    "shop_items",
    "shop_stock",
    "messages",
    "text_blocks",
    "actions",
    "action_texts",
];

fn stock() -> PathBuf {
    stock_data::vir_dir()
}

async fn counts(db: &Db) -> Vec<(&'static str, i64)> {
    let mut out = Vec::new();
    for t in TABLES {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {t}")))
            .fetch_one(&db.pool())
            .await
            .unwrap();
        out.push((*t, n));
    }
    out
}

#[tokio::test]
async fn the_stock_import_over_a_slow_link_is_quick_and_writes_the_same_rows() {
    let Some((slow, proxy)) = TestDb::postgres_with_latency(ONE_WAY)
        .await
        .expect("test database")
    else {
        eprintln!("SILVERMERE_DATABASE_URL names no PostgreSQL: nothing to measure");
        return;
    };
    let started = Instant::now();
    Importer::new(ImportOptions::new(stock()), Some(&slow))
        .run()
        .await
        .expect("import");
    let took = started.elapsed();
    let trips = proxy.round_trips();
    eprintln!("stock import over a 30 ms round trip: {took:?}, {trips} round trips");
    // the target is a minute at 25-35 ms; one statement per row took 7m38s. The round
    // trips are what this asserts (about 470 measured); the time only past a slow runner
    assert!(trips <= 600, "{trips} round trips");
    assert!(took < Duration::from_secs(90), "{took:?}");

    let local = TestDb::sqlite_temp().await.expect("sqlite");
    Importer::new(ImportOptions::new(stock()), Some(&local))
        .run()
        .await
        .expect("import");
    assert_eq!(counts(&slow).await, counts(&local).await);
    slow.close().await;
    local.close().await;
}
