//! `GET /v1/events`: the engine's events as server-sent events, with the `events` table
//! behind it. `Last-Event-ID: n` replays every stored event after `n` first, so a client
//! that reconnected loses nothing the table still holds.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

use axum::Extension;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use silvermere_engine::{AuditValue, GameEvent};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::auth::{Caller, Scope};
use crate::router::{ApiError, ApiState};

/// Capacity of the wake-up bus (a receiver that falls behind just reads the table).
pub const EVENT_BUFFER: usize = 1024;
/// How often the stream reads the table when nothing woke it.
const POLL: std::time::Duration = std::time::Duration::from_secs(1);
/// Rows fetched per page when reading from the table.
const REPLAY_PAGE: u32 = 500;

/// Every kind of event the stream carries, as the docs name them. `auth` and `admin` stand
/// for the families `auth.*` and `admin.<route>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// A session was opened (engine).
    Connected,
    /// A session was closed (engine).
    Closed,
    /// A character entered the realm.
    Login,
    /// A character left the realm.
    Logoff,
    /// A character died.
    Death,
    /// A character gained a level.
    Level,
    /// `auth.*`: login failures, lockouts, account changes (`silvermere-auth`).
    Auth,
    /// `admin.<route>`: the audit row of every authenticated `/v1` call (this crate).
    Admin,
}

impl EventKind {
    /// Every kind. Adding a variant fails to compile in [`EventKind::ordinal`] until it has
    /// a name and a position, and a test checks this slice against the ordinals.
    pub const ALL: &'static [EventKind] = &[
        EventKind::Connected,
        EventKind::Closed,
        EventKind::Login,
        EventKind::Logoff,
        EventKind::Death,
        EventKind::Level,
        EventKind::Auth,
        EventKind::Admin,
    ];

    /// The position in [`EventKind::ALL`]; the exhaustive `match` is the compile-time guard.
    pub const fn ordinal(self) -> usize {
        match self {
            EventKind::Connected => 0,
            EventKind::Closed => 1,
            EventKind::Login => 2,
            EventKind::Logoff => 3,
            EventKind::Death => 4,
            EventKind::Level => 5,
            EventKind::Auth => 6,
            EventKind::Admin => 7,
        }
    }

    /// The name in the `events.kind` column and the SSE `event:` field; for the families it
    /// is the prefix before the dot.
    pub const fn name(self) -> &'static str {
        match self {
            EventKind::Connected => "connected",
            EventKind::Closed => "closed",
            EventKind::Login => "login",
            EventKind::Logoff => "logoff",
            EventKind::Death => "death",
            EventKind::Level => "level",
            EventKind::Auth => "auth",
            EventKind::Admin => "admin",
        }
    }

    /// Whether the kind is a family (`auth.*`, `admin.*`) rather than one fixed name.
    pub const fn is_family(self) -> bool {
        matches!(self, EventKind::Auth | EventKind::Admin)
    }

    /// Where the kind comes from, for the docs.
    pub const fn source(self) -> &'static str {
        match self {
            EventKind::Connected | EventKind::Closed => "engine: a session opened or closed",
            EventKind::Login | EventKind::Logoff | EventKind::Death => {
                "engine: reserved, not emitted yet"
            }
            EventKind::Level => "engine: a character trained a level (train_level)",
            EventKind::Auth => "silvermere-auth: login failures, lockouts, account changes",
            EventKind::Admin => "this API: the audit row of each authenticated call",
        }
    }

    /// Whether anything emits the kind today (the rest are reserved names).
    pub const fn emitted(self) -> bool {
        matches!(
            self,
            EventKind::Connected
                | EventKind::Closed
                | EventKind::Level
                | EventKind::Auth
                | EventKind::Admin
        )
    }

    /// The kind a stored `events.kind` string belongs to (`auth.lockout` is `Auth`).
    pub fn of(kind: &str) -> Option<EventKind> {
        let head = kind.split_once('.').map_or(kind, |(h, _)| h);
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.name() == head && (k.is_family() || head == kind))
    }
}

/// The markdown table of [`EventKind::ALL`] that `docs/admin-api.md` embeds; a test keeps
/// the document equal to it.
pub fn kinds_table() -> String {
    let mut out =
        String::from("| Kind | Name in `events.kind` | Emitted | From |\n|---|---|---|---|");
    for k in EventKind::ALL {
        let name = if k.is_family() {
            format!("{}.*", k.name())
        } else {
            k.name().to_string()
        };
        out.push_str(&format!(
            "\n| `{}` | `{}` | {} | {} |",
            k.name(),
            name,
            if k.emitted() { "yes" } else { "not yet" },
            k.source()
        ));
    }
    out
}

/// Whether a stored kind passes a `?kinds=` filter: each filter word is a full kind
/// (`auth.lockout`) or a name that covers its family (`auth`, `admin`).
pub fn kind_passes(filter: &[String], kind: &str) -> bool {
    filter.is_empty()
        || filter.iter().any(|f| {
            kind == f
                || kind
                    .strip_prefix(f.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        })
}

/// The `Last-Event-ID` header as an event id, if it holds one.
pub fn last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
}

/// A boxed future, so the traits stay object-safe without another dependency.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// An error from the event log.
pub type LogError = Box<dyn std::error::Error + Send + Sync>;

/// A room as `{"map", "room"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RoomRef {
    pub map: u32,
    pub room: u32,
}

/// An event to append to the log.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub kind: String,
    pub session: Option<u16>,
    pub character: Option<i64>,
    pub room: Option<RoomRef>,
    pub payload: Value,
}

impl NewEvent {
    /// An event of `kind` with no session, character or room.
    pub fn new(kind: &str, payload: Value) -> Self {
        Self {
            kind: kind.to_string(),
            session: None,
            character: None,
            room: None,
            payload,
        }
    }

    /// The row an engine event becomes.
    pub fn from_game_event(ev: &GameEvent) -> Self {
        Self {
            kind: ev.kind().to_string(),
            session: ev.session().map(|s| s.0),
            character: None,
            room: None,
            payload: match ev {
                GameEvent::Level { level, .. } => json!({ "level": level }),
                GameEvent::Audit { fields, .. } => {
                    let mut m = serde_json::Map::new();
                    for (k, v) in fields {
                        let v = match v {
                            AuditValue::Int(n) => json!(n),
                            AuditValue::Text(t) => json!(t),
                        };
                        m.insert((*k).to_string(), v);
                    }
                    Value::Object(m)
                }
                // T-055: the sysop's notable actions (`shocst`)
                GameEvent::Sysop { user, detail, .. } => {
                    json!({ "user": user, "detail": detail })
                }
                _ => json!({}),
            },
        }
    }
}

/// A stored event: what `/v1/events` sends as JSON.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventRecord {
    /// The `events` row id, ascending in time.
    pub id: i64,
    /// RFC 3339 timestamp.
    pub at: String,
    pub kind: String,
    pub session: Option<u16>,
    pub character: Option<i64>,
    pub room: Option<RoomRef>,
    pub payload: Value,
}

/// The `events` table, as the API needs it. `bin` implements it over `silvermere-db`
/// (`Events::append`), stamping the time and id.
pub trait EventLog: Send + Sync {
    /// Appends `event` and returns it as stored.
    fn append(&self, event: NewEvent) -> BoxFuture<'_, Result<EventRecord, LogError>>;
    /// The id of the newest stored event, 0 when there is none.
    fn latest_id(&self) -> BoxFuture<'_, Result<i64, LogError>>;
    /// Up to `limit` events with `id > after_id`, oldest first.
    fn since(&self, after_id: i64, limit: u32)
    -> BoxFuture<'_, Result<Vec<EventRecord>, LogError>>;
}

fn frame(rec: &EventRecord) -> Event {
    Event::default()
        .id(rec.id.to_string())
        .event(rec.kind.clone())
        .json_data(rec)
        .unwrap_or_else(|_| Event::default().event("error"))
}

/// The `GET /v1/events` query string.
#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    /// Comma-separated kinds to send, e.g. `login,death`; absent means all.
    pub kinds: Option<String>,
}

pub async fn stream(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    if !caller.allows(Scope::Read) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let after = last_event_id(&headers);
    let filter: Vec<String> = query
        .kinds
        .iter()
        .flat_map(|k| k.split(','))
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    // The table is the stream's source, so rows written by other writers (the login
    // dialogue's `auth.*` rows go straight to the table) are sent too, in id order. The
    // bus only wakes the reader early; a 1 s tick covers the rows that never touch it.
    // Subscribe before reading the starting point, so nothing falls in between.
    let mut wake = state.events.subscribe();
    let log = state.log.clone();
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    // Decided before the response starts, so the audit row of this very call (written
    // after the handler returns) is the first thing a new stream sends.
    let mut cursor = match after {
        Some(id) => id,
        None => match log.latest_id().await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(%e, "event stream cannot find the newest event");
                return ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response();
            }
        },
    };
    tokio::spawn(async move {
        loop {
            // Drain everything after the cursor.
            loop {
                let page = match log.since(cursor, REPLAY_PAGE).await {
                    Ok(page) => page,
                    Err(e) => {
                        tracing::warn!(%e, "event read failed");
                        break;
                    }
                };
                let full = page.len() == REPLAY_PAGE as usize;
                for rec in page {
                    cursor = rec.id;
                    if !kind_passes(&filter, &rec.kind) {
                        continue;
                    }
                    if tx.send(Ok(frame(&rec))).await.is_err() {
                        return;
                    }
                }
                if !full {
                    break;
                }
            }
            tokio::select! {
                woke = wake.recv() => {
                    if matches!(woke, Err(broadcast::error::RecvError::Closed)) {
                        return;
                    }
                }
                () = tokio::time::sleep(POLL) => {}
            }
        }
    });
    Sse::new(ReceiverStream::new(rx))
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_matches_the_ordinals_and_names_are_unique() {
        for (i, k) in EventKind::ALL.iter().enumerate() {
            assert_eq!(k.ordinal(), i, "{k:?}");
        }
        let mut names: Vec<_> = EventKind::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), EventKind::ALL.len());
    }

    #[test]
    fn kinds_round_trip_through_json_and_stored_names() {
        for k in EventKind::ALL {
            let json = serde_json::to_string(k).unwrap();
            assert_eq!(json, format!("\"{}\"", k.name()));
            assert_eq!(serde_json::from_str::<EventKind>(&json).unwrap(), *k);
            assert_eq!(EventKind::of(k.name()), Some(*k));
        }
        assert_eq!(EventKind::of("auth.lockout"), Some(EventKind::Auth));
        assert_eq!(
            EventKind::of("admin.world.rooms.12"),
            Some(EventKind::Admin)
        );
        assert_eq!(EventKind::of("login.x"), None);
        assert_eq!(EventKind::of("nope"), None);
    }

    #[test]
    fn filter_matches_names_and_families() {
        let f = |s: &str| -> Vec<String> { s.split(',').map(str::to_string).collect() };
        assert!(kind_passes(&[], "anything"));
        assert!(kind_passes(&f("login,death"), "death"));
        assert!(!kind_passes(&f("login,death"), "closed"));
        assert!(kind_passes(&f("auth"), "auth.lockout"));
        assert!(!kind_passes(&f("auth"), "authx"));
        assert!(kind_passes(&f("auth.lockout"), "auth.lockout"));
        assert!(!kind_passes(&f("auth.lockout"), "auth.signup"));
    }

    #[test]
    fn last_event_id_parses_a_number_and_nothing_else() {
        let mut h = HeaderMap::new();
        assert_eq!(last_event_id(&h), None);
        h.insert("last-event-id", " 42 ".parse().unwrap());
        assert_eq!(last_event_id(&h), Some(42));
        h.insert("last-event-id", "abc".parse().unwrap());
        assert_eq!(last_event_id(&h), None);
    }
}
