//! Backup and restore on SQLite: the round trip and every refusal of `restore`.

use std::fs;
use std::path::{Path, PathBuf};

use silvermere_db::Db;
use silvermere_db::backup::{self, BackupError};
use sqlx::Row;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sm-backup-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

async fn open(path: &Path) -> Db {
    Db::connect_and_migrate(&format!("sqlite://{}", path.display()))
        .await
        .unwrap()
}

async fn add_events(db: &Db, n: usize) {
    for i in 0..n {
        sqlx::query(
            "INSERT INTO events (at, kind, payload) VALUES ('2026-10-15T00:00:00Z', ?, '{}')",
        )
        .bind(format!("k{i}"))
        .execute(&db.pool())
        .await
        .unwrap();
    }
}

async fn counts(db: &Db) -> Vec<(String, i64)> {
    let pool = db.pool();
    let tables: Vec<String> = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<String, _>(0))
    .collect();
    let mut out = Vec::new();
    for t in tables {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM \"{t}\"")))
                .fetch_one(&pool)
                .await
                .unwrap();
        out.push((t, n));
    }
    out
}

fn now() -> jiff::Timestamp {
    "2026-10-15T12:00:00Z".parse().unwrap()
}

#[tokio::test]
async fn backup_copies_every_table_while_the_pool_is_open() {
    let dir = scratch("copy");
    let live = open(&dir.join("silvermere.db")).await;
    add_events(&live, 5).await;
    let dest = dir
        .join("backups")
        .join(backup::backup_name(now(), silvermere_db::Dialect::Sqlite));
    backup::backup_sqlite(&live.pool(), &dest).await.unwrap();
    assert!(dest.exists());
    assert!(
        !dir.join("backups")
            .join(format!(
                "{}.partial",
                backup::backup_name(now(), silvermere_db::Dialect::Sqlite)
            ))
            .exists()
    );
    let copy = open(&dest).await;
    assert_eq!(counts(&live).await, counts(&copy).await);
    // The destination exists: refused, nothing overwritten.
    let again = backup::backup_sqlite(&live.pool(), &dest).await;
    assert!(matches!(again, Err(BackupError::Refused(_))));
    live.close().await;
    copy.close().await;
}

#[tokio::test]
async fn restore_brings_back_the_backed_up_state() {
    let dir = scratch("roundtrip");
    let path = dir.join("silvermere.db");
    let live = open(&path).await;
    add_events(&live, 3).await;
    let file = dir.join("b.db");
    backup::backup_sqlite(&live.pool(), &file).await.unwrap();
    let before = counts(&live).await;
    add_events(&live, 4).await;
    live.close().await;

    backup::check_restorable(&file, &path).await.unwrap();
    let moved = backup::restore_sqlite(&file, &path, now())
        .unwrap()
        .unwrap();
    assert!(moved.ends_with("silvermere.db.pre-restore-20261015-120000"));
    assert!(moved.exists());
    for ext in ["-wal", "-shm"] {
        let mut w = path.as_os_str().to_owned();
        w.push(ext);
        assert!(!PathBuf::from(w).exists(), "stale {ext} removed");
    }
    let restored = open(&path).await;
    assert_eq!(counts(&restored).await, before);
    restored.close().await;
}

#[tokio::test]
async fn restore_refuses_in_order() {
    let dir = scratch("refuse");
    let path = dir.join("silvermere.db");
    let live = open(&path).await;
    add_events(&live, 1).await;
    let good = dir.join("good.db");
    backup::backup_sqlite(&live.pool(), &good).await.unwrap();
    let msg = |r: Result<(), BackupError>| match r {
        Err(BackupError::Refused(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    };

    // 1. not a SQLite database
    let junk = dir.join("junk.db");
    fs::write(
        &junk,
        b"this is not a database at all, but longer than sixteen bytes",
    )
    .unwrap();
    assert!(msg(backup::check_restorable(&junk, &path).await).contains("not a SQLite database"));
    let missing = dir.join("missing.db");
    assert!(msg(backup::check_restorable(&missing, &path).await).contains("not a SQLite database"));

    // 2. integrity_check fails: damage a page of a good copy
    let bad = dir.join("bad.db");
    let mut bytes = fs::read(&good).unwrap();
    let at = bytes.len() / 2;
    for b in &mut bytes[at..at + 4096] {
        *b = 0xA5;
    }
    fs::write(&bad, bytes).unwrap();
    let r = backup::check_restorable(&bad, &path).await;
    assert!(r.is_err(), "damaged file refused: {r:?}");

    // 3. made by a newer binary
    let newer = dir.join("newer.db");
    fs::copy(&good, &newer).unwrap();
    let n = open_raw(&newer).await;
    sqlx::query(
        "UPDATE _sqlx_migrations SET version = 99999999999999 \
         WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)",
    )
    .execute(&n)
    .await
    .unwrap();
    n.close().await;
    assert!(msg(backup::check_restorable(&newer, &path).await).contains("newer silvermere"));

    // 4. a server holds the live database (the pool is still open)
    assert!(
        msg(backup::check_restorable(&good, &path).await).contains("server appears to be running")
    );

    // After the server stops, the same file is accepted.
    live.close().await;
    backup::check_restorable(&good, &path).await.unwrap();
}

async fn open_raw(path: &Path) -> sqlx::AnyPool {
    sqlx::any::install_default_drivers();
    sqlx::AnyPool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap()
}

#[test]
fn pg_dump_missing_is_reported_with_the_card_wording() {
    // Only meaningful where pg_dump is absent; elsewhere the call would try the network.
    if std::process::Command::new("pg_dump")
        .arg("--version")
        .output()
        .is_ok()
    {
        return;
    }
    let e = backup::backup_postgres("postgres://x", Path::new("/tmp/never.dump")).unwrap_err();
    assert_eq!(
        e.to_string(),
        "pg_dump not found: install postgresql-client or back up with your own tooling"
    );
}

#[tokio::test]
#[ignore = "needs pg_dump on PATH and SILVERMERE_TEST_PG_URL"]
async fn postgres_backup_writes_a_dump() {
    let (Ok(url), Ok(_)) = (
        std::env::var("SILVERMERE_TEST_PG_URL"),
        std::process::Command::new("pg_dump")
            .arg("--version")
            .output(),
    ) else {
        eprintln!("skipped: pg_dump or SILVERMERE_TEST_PG_URL missing");
        return;
    };
    let dir = scratch("pg");
    let dest = dir.join("pg.dump");
    backup::backup_postgres(&url, &dest).unwrap();
    assert!(fs::metadata(&dest).unwrap().len() > 0);
}
