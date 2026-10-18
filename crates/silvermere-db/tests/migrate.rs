//! The migrations apply cleanly (and twice) on the configured backend, and `TestDb`
//! hands out isolated databases.

mod common;

use silvermere_db::{Db, Dialect, TestDb};

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let test = common::db().await;
    // a second run is a no-op
    test.migrate().await.unwrap();
    let dialect = test.dialect();
    assert_eq!(dialect, Dialect::from_url(test.url()).unwrap());
    assert_eq!(test.rooms().all().await.unwrap(), vec![]);
    assert_eq!(test.characters().count().await.unwrap(), 0);
    test.close().await;
}

#[tokio::test]
async fn test_databases_are_isolated() {
    let a = common::db().await;
    let b = common::db().await;
    let mut seq = common::Seq::new(40);
    a.messages()
        .insert(&common::message(&mut seq, 1))
        .await
        .unwrap();
    assert_eq!(a.messages().all().await.unwrap().len(), 1);
    assert_eq!(b.messages().all().await.unwrap().len(), 0);
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn sqlite_temp_is_always_available() {
    let t = TestDb::sqlite_temp().await.unwrap();
    assert_eq!(t.dialect(), Dialect::Sqlite);
    assert!(t.url().starts_with("sqlite://"));
    assert_eq!(t.events().count().await.unwrap(), 0);
    t.close().await;
}

#[tokio::test]
async fn in_memory_sqlite_works_with_one_connection() {
    let db = Db::connect_and_migrate("sqlite://").await.unwrap();
    assert_eq!(db.characters().count().await.unwrap(), 0);
    db.close().await;
}

#[tokio::test]
async fn unsupported_url_is_refused() {
    assert!(matches!(
        Db::connect("mysql://nope").await,
        Err(silvermere_db::DbError::UnsupportedUrl(_))
    ));
}
