//! The single-writer guard (T-072) on the backend `SILVERMERE_DATABASE_URL` names (SQLite
//! by default): one holder per store, released on drop, separate stores do not collide.

use std::time::Duration;

use silvermere_db::{GuardLost, TestDb, TryAcquire, Watchdog, WriterGuard};

async fn take(url: &str) -> TryAcquire {
    WriterGuard::try_acquire(url).await.expect("try the guard")
}

#[tokio::test]
async fn one_holder_per_store() {
    let t = TestDb::from_env().await.expect("test database");
    let other = TestDb::from_env().await.expect("second test database");
    let url = t.db().url().to_string();

    let TryAcquire::Taken(first) = take(&url).await else {
        panic!("a fresh store's guard is free");
    };
    match take(&url).await {
        TryAcquire::Held(by) => assert!(by.contains("held") && by.contains("pid"), "{by}"),
        TryAcquire::Taken(_) => panic!("a second holder on one store"),
    }
    // another store (file or schema) has a guard of its own
    let TryAcquire::Taken(elsewhere) = take(other.db().url()).await else {
        panic!("another store's guard is free");
    };
    first.release().await;
    let TryAcquire::Taken(again) = take(&url).await else {
        panic!("released guards can be taken again");
    };
    drop(again);
    assert!(
        matches!(take(&url).await, TryAcquire::Taken(_)),
        "drop releases"
    );
    elsewhere.release().await;
    other.close().await;
    t.close().await;
}

#[tokio::test]
async fn in_memory_needs_no_guard() {
    assert!(matches!(take("sqlite://").await, TryAcquire::Taken(_)));
    assert!(matches!(take("sqlite://").await, TryAcquire::Taken(_)));
}

/// The PostgreSQL URL of the test store, or `None` on SQLite.
fn postgres(t: &TestDb) -> Option<String> {
    (t.db().dialect() == silvermere_db::Dialect::Postgres).then(|| t.db().url().to_string())
}

/// Ends the backend holding the guard of `url`, waiting for it to go.
async fn kill_holder(url: &str) {
    let pid = silvermere_db::guard::postgres_holder(url)
        .await
        .unwrap()
        .expect("someone holds the guard");
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1, 5000)")
        .bind(pid)
        .fetch_one(&t_pool(url).await)
        .await
        .unwrap();
    assert!(killed, "backend {pid} ended");
}

async fn t_pool(url: &str) -> sqlx::AnyPool {
    sqlx::any::install_default_drivers();
    sqlx::any::AnyPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap()
}

fn quick() -> Watchdog {
    Watchdog {
        every: Duration::from_millis(1500),
        give_up: Duration::from_secs(3),
    }
}

/// T-073: when the lock's connection dies, the watchdog takes the lock again on a new one.
#[tokio::test]
async fn a_killed_guard_connection_is_re_taken() {
    let t = TestDb::from_env().await.expect("test database");
    let Some(url) = postgres(&t) else {
        return t.close().await;
    };
    let TryAcquire::Taken(mut guard) = take(&url).await else {
        panic!("a fresh store's guard is free");
    };
    let first = silvermere_db::guard::postgres_holder(&url).await.unwrap();
    kill_holder(&url).await;
    let lost = tokio::time::timeout(Duration::from_secs(4), guard.watch(quick())).await;
    assert!(lost.is_err(), "the watchdog re-took the lock: {lost:?}");
    let now = silvermere_db::guard::postgres_holder(&url).await.unwrap();
    assert!(now.is_some() && now != first, "held again by a new backend");
    assert!(
        matches!(take(&url).await, TryAcquire::Held(_)),
        "still one holder"
    );
    guard.release().await;
    t.close().await;
}

/// T-073: a process that takes the lock while ours is gone keeps it; the watchdog gives up.
#[tokio::test]
async fn a_competing_holder_makes_the_watchdog_give_up() {
    let t = TestDb::from_env().await.expect("test database");
    let Some(url) = postgres(&t) else {
        return t.close().await;
    };
    let TryAcquire::Taken(mut guard) = take(&url).await else {
        panic!("a fresh store's guard is free");
    };
    kill_holder(&url).await;
    let TryAcquire::Taken(other) = take(&url).await else {
        panic!("the killed holder's lock is free");
    };
    let lost = tokio::time::timeout(Duration::from_secs(10), guard.watch(quick()))
        .await
        .expect("the watchdog gives up");
    assert!(matches!(lost, GuardLost::HeldElsewhere(_)), "{lost:?}");
    other.release().await;
    t.close().await;
}

/// T-073: a `search_path` naming no existing schema is named in the error.
#[tokio::test]
async fn a_missing_schema_is_named() {
    let t = TestDb::from_env().await.expect("test database");
    let Some(url) = postgres(&t) else {
        return t.close().await;
    };
    let base = url.split("options=").next().unwrap();
    let bad = format!("{base}options=-c%20search_path%3Dno_such_schema_t073");
    let err = WriterGuard::try_acquire(&bad)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("search_path") && err.contains("current_schema()"),
        "{err}"
    );
    t.close().await;
}
