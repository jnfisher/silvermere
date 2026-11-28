//! `/v1/world-hold`: lets an editor write the world tables of the database while the game
//! runs, then has the game take them, with nobody disconnected.
//!
//! `serve` holds the single-writer guard for as long as it runs, and its persistence worker
//! writes the live parts of the content tables as play changes them (`items.active`, a
//! gang shop's slots, `monster_state`). A hold flushes the worker and stops it: the writes
//! the game makes meanwhile wait in its queue, so an editor can write the world tables
//! with none of the game's in flight. `POST /v1/world-hold/{id}/reload` then reloads the
//! world, as `/v1/reload-world` does, and ends the hold; the waiting writes follow in
//! order. A hold that is not ended in `secs` ends by itself, with no reload.
//!
//! One hold at a time; while one is held another is `409 held`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{Caller, Scope};
use crate::reload;
use crate::router::{ApiError, ApiState, AuditDetail, require};

/// A hold lasts this long when the request names no time.
pub const DEFAULT_SECS: u64 = 60;
/// The longest hold: the game's writes wait this long at most.
pub const MAX_SECS: u64 = 600;
/// How long taking a hold may wait for the writes queued before it.
const HOLD_TIMEOUT: Duration = Duration::from_secs(30);

/// The hold now held, if any.
#[derive(Default)]
pub struct Holds {
    current: Mutex<Option<Hold>>,
    next: AtomicU64,
}

struct Hold {
    id: String,
    until: Instant,
    /// Dropping it lets the game write again.
    _guard: Box<dyn Send + Sync>,
}

impl Holds {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Hold>> {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The hold `id`, taken away (and so ended) when it is the current one.
    fn take(&self, id: &str) -> Option<Hold> {
        let mut cur = self.lock();
        if cur.as_ref().is_some_and(|h| h.id == id) {
            cur.take()
        } else {
            None
        }
    }

    /// Ends the hold held now, if any: a shutdown does, so its last saves are not kept
    /// waiting. Whether there was one.
    pub fn end_all(&self) -> bool {
        self.lock().take().is_some()
    }

    /// Whether a hold is held now: one past its time is ended here.
    fn busy(&self) -> bool {
        let mut cur = self.lock();
        if cur.as_ref().is_some_and(|h| h.until <= Instant::now()) {
            *cur = None;
        }
        cur.is_some()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    secs: Option<u64>,
}

/// `GET /v1/world-hold`: `{"hold": null}` or `{"hold": {"id": ..., "expires_in": s}}`.
pub async fn status(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if let Err(e) = require(&caller, Scope::Read) {
        return e.into_response();
    }
    let busy = state.world_hold.busy();
    let cur = state.world_hold.lock();
    let hold = cur.as_ref().filter(|_| busy).map(|h| {
        json!({
            "id": h.id,
            "expires_in": h.until.saturating_duration_since(Instant::now()).as_secs(),
        })
    });
    Json(json!({ "hold": hold })).into_response()
}

/// `POST /v1/world-hold`, `{"secs": n}` (optional, 1 to [`MAX_SECS`]): holds the game's
/// writes and answers `{"hold": id, "secs": n}` once everything before is written.
pub async fn take(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    if let Err(e) = require(&caller, Scope::Admin) {
        return e.into_response();
    }
    let body: Body = if raw.iter().all(u8::is_ascii_whitespace) {
        Body::default()
    } else {
        match serde_json::from_slice(&raw) {
            Ok(b) => b,
            Err(_) => return ApiError(StatusCode::BAD_REQUEST, "bad_body").into_response(),
        }
    };
    let secs = body.secs.unwrap_or(DEFAULT_SECS);
    if !(1..=MAX_SECS).contains(&secs) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_hold").into_response();
    }
    if state.world_hold.busy() {
        return ApiError(StatusCode::CONFLICT, "held").into_response();
    }
    let guard = match tokio::time::timeout(HOLD_TIMEOUT, state.world.hold()).await {
        Ok(Ok(g)) => g,
        Ok(Err(e)) => {
            tracing::warn!(%e, "world hold refused");
            return ApiError(StatusCode::SERVICE_UNAVAILABLE, "hold_unavailable").into_response();
        }
        Err(_) => return ApiError(StatusCode::GATEWAY_TIMEOUT, "hold_timeout").into_response(),
    };
    let n = state.world_hold.next.fetch_add(1, Ordering::Relaxed) + 1;
    let id = format!("h{n}-{:08x}", rand_word());
    {
        let mut cur = state.world_hold.lock();
        if cur.is_some() {
            // another request took one while this one waited: this guard drops here
            return ApiError(StatusCode::CONFLICT, "held").into_response();
        }
        *cur = Some(Hold {
            id: id.clone(),
            until: Instant::now() + Duration::from_secs(secs),
            _guard: guard,
        });
    }
    tracing::info!(hold = %id, secs, by = %caller.name, "world hold taken");
    let holds = state.world_hold.clone();
    let expiring = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        if holds.take(&expiring).is_some() {
            tracing::warn!(hold = %expiring, secs, "world hold ran out: the game writes again, the world was not reloaded");
        }
    });
    let mut resp = Json(json!({ "hold": id, "secs": secs })).into_response();
    resp.extensions_mut()
        .insert(AuditDetail(json!({ "hold": id, "secs": secs })));
    resp
}

/// `POST /v1/world-hold/{id}/reload`: reloads the world tables (as `/v1/reload-world`),
/// then ends the hold. The hold ends even when the reload fails.
pub async fn reload_and_end(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    if let Err(e) = require(&caller, Scope::Admin) {
        return e.into_response();
    }
    // still held while the world is read, so the tables are the editor's alone
    if !state.world_hold.lock().as_ref().is_some_and(|h| h.id == id) {
        return ApiError(StatusCode::NOT_FOUND, "no_such_hold").into_response();
    }
    let done = reload::reload(&state).await;
    let ended = state.world_hold.take(&id).is_some();
    tracing::info!(hold = %id, reloaded = done.is_ok(), ended, "world hold ended");
    reload::answer(done)
}

/// `DELETE /v1/world-hold/{id}`: ends the hold with no reload: `{"ended": true}`.
pub async fn end(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    if let Err(e) = require(&caller, Scope::Admin) {
        return e.into_response();
    }
    match state.world_hold.take(&id) {
        Some(_) => {
            tracing::info!(hold = %id, "world hold ended, no reload");
            Json(json!({ "ended": true })).into_response()
        }
        None => ApiError(StatusCode::NOT_FOUND, "no_such_hold").into_response(),
    }
}

/// A word that keeps one hold's id from being guessed from the last.
fn rand_word() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.finish() as u32
}
