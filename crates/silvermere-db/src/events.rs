//! The append-only `events` table.
//!
//! Every game event the engine emits is a row here as well as a `tracing` line; everything the
//! original wrote to the BBS audit trail (`shocst`) becomes an [`Event`]
//! Rows are pruned to a retention count.

use jiff::Timestamp;
use silvermere_types::ids::{CharacterId, MapId, RoomId, RoomKey, SessionId};

use crate::pool::Db;
use crate::repo::{Args, Conn, Get, insert_sql};
use crate::{DbError, Result};

/// One game event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// When it happened.
    pub at: Timestamp,
    /// The kind, e.g. `login`, `death`, `level`, `sysop`.
    pub kind: String,
    /// The session concerned, if any.
    pub session: Option<SessionId>,
    /// The character concerned, if any.
    pub character: Option<CharacterId>,
    /// The room concerned, if any.
    pub room: Option<RoomKey>,
    /// The details, a JSON object.
    pub payload: serde_json::Value,
}

impl Event {
    /// An event of `kind` at `at` with no session, character or room.
    pub fn new(at: Timestamp, kind: &str, payload: serde_json::Value) -> Self {
        Self {
            at,
            kind: kind.to_string(),
            session: None,
            character: None,
            room: None,
            payload,
        }
    }
}

/// A stored event with its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// The id, ascending in time.
    pub id: i64,
    /// The event.
    pub event: Event,
}

const COLS: &[&str] = &[
    "at",
    "kind",
    "session",
    "character",
    "map",
    "room",
    "payload",
];

fn args(event: &Event) -> Args {
    let mut a = Args::new();
    a.add(event.at.to_string())
        .add(&event.kind)
        .add(event.session.map(|s| s.0))
        .add(event.character.map(|c| c.0))
        .add(event.room.map(|r| r.map.0))
        .add(event.room.map(|r| r.room.0))
        .add(event.payload.to_string());
    a
}

/// Appends `events` in order, in multi-row statements (T-113).
pub(crate) async fn append_many(conn: &mut Conn, events: &[&Event]) -> Result<()> {
    let rows = events.iter().map(|e| args(e)).collect();
    crate::repo::insert_many_bulk(conn, "events", COLS, rows).await
}

/// Appends `event`.
pub(crate) async fn append(conn: &mut Conn, event: &Event) -> Result<i64> {
    let row = args(event)
        .query(format!("{} RETURNING id", insert_sql("events", COLS)))
        .fetch_one(&mut *conn)
        .await?;
    Get::new(&row, "events").i64("id")
}

fn read(row: &sqlx::any::AnyRow) -> Result<StoredEvent> {
    let g = Get::new(row, "events");
    let at_text = g.text("at")?;
    let at = at_text.parse().map_err(|e: jiff::Error| DbError::Decode {
        table: "events",
        column: "at".to_string(),
        reason: e.to_string(),
    })?;
    let payload_text = g.text("payload")?;
    let payload = serde_json::from_str(&payload_text).map_err(|e| DbError::Decode {
        table: "events",
        column: "payload".to_string(),
        reason: e.to_string(),
    })?;
    let room = match (g.opt_i64("map")?, g.opt_i64("room")?) {
        (Some(map), Some(room)) => Some(RoomKey {
            map: MapId(u32::try_from(map).map_err(|_| decode_err("map"))?),
            room: RoomId(u32::try_from(room).map_err(|_| decode_err("room"))?),
        }),
        _ => None,
    };
    Ok(StoredEvent {
        id: g.i64("id")?,
        event: Event {
            at,
            kind: g.text("kind")?,
            session: g
                .opt_i64("session")?
                .map(|s| {
                    u16::try_from(s)
                        .map(SessionId)
                        .map_err(|_| decode_err("session"))
                })
                .transpose()?,
            character: g.opt_i64("character")?.map(CharacterId),
            room,
            payload,
        },
    })
}

fn decode_err(column: &str) -> DbError {
    DbError::Decode {
        table: "events",
        column: column.to_string(),
        reason: "out of range".to_string(),
    }
}

/// The events table.
#[derive(Debug, Clone, Copy)]
pub struct Events<'a> {
    db: &'a Db,
}

impl<'a> Events<'a> {
    pub(crate) fn new(db: &'a Db) -> Self {
        Self { db }
    }

    /// Appends `event` and returns its id.
    pub async fn append(&self, event: &Event) -> Result<i64> {
        let mut conn = self.db.pool().acquire().await?;
        append(&mut conn, event).await
    }

    /// Deletes every event but the newest `retention`; returns how many went.
    pub async fn prune(&self, retention: u64) -> Result<u64> {
        let mut a = Args::new();
        a.add(i64::try_from(retention).unwrap_or(i64::MAX));
        let done = a
            .query(
                "DELETE FROM events WHERE id NOT IN (SELECT id FROM events ORDER BY id DESC LIMIT $1)"
                    .to_string(),
            )
            .execute(&self.db.pool())
            .await?;
        Ok(done.rows_affected())
    }

    /// How many events are stored.
    pub async fn count(&self) -> Result<u64> {
        let row = Args::new()
            .query("SELECT COUNT(*) AS n FROM events".to_string())
            .fetch_one(&self.db.pool())
            .await?;
        Get::new(&row, "events").int("n")
    }

    /// The newest `n` events, newest first.
    pub async fn recent(&self, n: u64) -> Result<Vec<StoredEvent>> {
        let mut a = Args::new();
        a.add(i64::try_from(n).unwrap_or(i64::MAX));
        let rows = a
            .query(format!(
                "SELECT id, {} FROM events ORDER BY id DESC LIMIT $1",
                COLS.join(", ")
            ))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }

    /// The events with an id greater than `after`, oldest first, at most `limit` (the
    /// admin API's event stream catch-up).
    pub async fn since(&self, after: i64, limit: u64) -> Result<Vec<StoredEvent>> {
        let mut a = Args::new();
        a.add(after).add(i64::try_from(limit).unwrap_or(i64::MAX));
        let rows = a
            .query(format!(
                "SELECT id, {} FROM events WHERE id > $1 ORDER BY id LIMIT $2",
                COLS.join(", ")
            ))
            .fetch_all(&self.db.pool())
            .await?;
        rows.iter().map(read).collect()
    }
}
