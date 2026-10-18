//! Proves CI reaches both backends: runs `SELECT 1` against the database named by
//! `SILVERMERE_DATABASE_URL` (CI sets `sqlite://` and a PostgreSQL service container).
//! Skips when the variable is unset so a plain local `cargo test` needs no database.

use sqlx::{PgPool, Row, SqlitePool};

#[tokio::test]
async fn select_one() {
    let Ok(url) = std::env::var("SILVERMERE_DATABASE_URL") else {
        eprintln!("SILVERMERE_DATABASE_URL unset; skipping");
        return;
    };
    let one: i32 = if url.starts_with("postgres") {
        let pool = PgPool::connect(&url).await.expect("connect to PostgreSQL");
        sqlx::query("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("SELECT 1")
            .get(0)
    } else {
        let pool = SqlitePool::connect(&url).await.expect("connect to SQLite");
        sqlx::query("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("SELECT 1")
            .get(0)
    };
    assert_eq!(one, 1);
}
