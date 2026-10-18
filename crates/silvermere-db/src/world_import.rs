//! `world_import` (T-109): whether the last import finished writing. `silvermere import`
//! calls [`begin`] before its first table and [`finish`] after its last, so a start can tell
//! a world an interrupted import left half written (rooms but no items, say) from a whole one.

use crate::Result;
use crate::pool::Db;

/// The last import's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldImport {
    /// No import by this version wrote here (a test's seeded world, or none at all).
    Never,
    /// An import began writing and did not finish: the world is partial.
    Unfinished,
    /// The last import wrote every table.
    Finished,
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

/// Records that an import is about to write.
pub async fn begin(db: &Db) -> Result<()> {
    sqlx::query(
        "INSERT INTO world_import (id, started_at, finished_at) VALUES (1, $1, NULL) \
         ON CONFLICT (id) DO UPDATE SET started_at = excluded.started_at, finished_at = NULL",
    )
    .bind(now())
    .execute(&db.pool())
    .await?;
    Ok(())
}

/// Records that the import begun by [`begin`] wrote every table.
pub async fn finish(db: &Db) -> Result<()> {
    sqlx::query("UPDATE world_import SET finished_at = $1 WHERE id = 1")
        .bind(now())
        .execute(&db.pool())
        .await?;
    Ok(())
}

/// The last import's progress.
pub async fn state(db: &Db) -> Result<WorldImport> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT finished_at FROM world_import WHERE id = 1")
            .fetch_optional(&db.pool())
            .await?;
    Ok(match row {
        None => WorldImport::Never,
        Some((None,)) => WorldImport::Unfinished,
        Some((Some(_),)) => WorldImport::Finished,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn begin_then_finish() {
        let t = crate::TestDb::from_env().await.expect("test database");
        let db = t.db();
        assert_eq!(state(db).await.unwrap(), WorldImport::Never);
        begin(db).await.unwrap();
        assert_eq!(state(db).await.unwrap(), WorldImport::Unfinished);
        finish(db).await.unwrap();
        assert_eq!(state(db).await.unwrap(), WorldImport::Finished);
        // a second import that is cut off leaves the world unfinished again
        begin(db).await.unwrap();
        assert_eq!(state(db).await.unwrap(), WorldImport::Unfinished);
        t.close().await;
    }
}
