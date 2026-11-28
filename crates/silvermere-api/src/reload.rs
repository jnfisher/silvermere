//! `POST /v1/reload-world`: re-read the static world tables from the database into the
//! running engine. The engine swaps them between two events (`World::reload_content`) and
//! leaves players, room state, active monsters, gangs and books alone.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{Value, json};
use silvermere_engine::{AdminRequest, WorldSnapshot};

use crate::auth::{Caller, Scope};
use crate::events::{BoxFuture, LogError};
use crate::router::{ApiError, ApiState, AuditDetail, ask, require};

/// Reads the static world tables; `bin` implements it over `silvermere-db`.
pub trait WorldLoader: Send + Sync {
    fn load(&self) -> BoxFuture<'_, Result<Box<WorldSnapshot>, LogError>>;

    /// Holds the game's own writes to the database for `POST /v1/world-hold`: the future
    /// ends once everything the game queued before it is written, and the game writes
    /// nothing more until the value it gives is dropped. A server with no persistence to
    /// hold answers an error (the default).
    fn hold(&self) -> BoxFuture<'_, Result<Box<dyn Send + Sync>, LogError>> {
        Box::pin(async { Err(LogError::from("this server has no writes to hold")) })
    }
}

/// How many rows of each reloaded table the snapshot holds.
pub fn counts(s: &WorldSnapshot) -> Value {
    json!({
        "rooms": s.rooms.len(),
        "items": s.items.len(),
        "monsters": s.monsters.len(),
        "spells": s.spells.len(),
        "classes": s.classes.len(),
        "races": s.races.len(),
        "shops": s.shops.len(),
        "messages": s.messages.len(),
        "text_blocks": s.text_blocks.len(),
        "actions": s.actions.len(),
    })
}

fn failed(e: impl std::fmt::Display) -> Response {
    tracing::error!(%e, "reload-world failed; the previous tables stay in place");
    let mut resp = ApiError(StatusCode::INTERNAL_SERVER_ERROR, "reload_failed").into_response();
    resp.extensions_mut()
        .insert(AuditDetail(json!({ "failure": e.to_string() })));
    resp
}

pub async fn reload_world(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if let Err(e) = require(&caller, Scope::Write) {
        return e.into_response();
    }
    answer(reload(&state).await)
}

/// Reads the world tables and has the engine take them: the counts read and the time it
/// took, or the error response.
pub(crate) async fn reload(state: &ApiState) -> Result<(Value, u64), Box<Response>> {
    let started = Instant::now();
    let snapshot = state.world.load().await.map_err(|e| Box::new(failed(e)))?;
    let counts = counts(&snapshot);
    let (reply, rx) = std::sync::mpsc::channel();
    let req = AdminRequest::ReloadContent { snapshot, reply };
    match ask(state, req, rx).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(Box::new(failed(e))),
        Err(e) => return Err(Box::new(e.into_response())),
    }
    Ok((counts, started.elapsed().as_millis() as u64))
}

/// The answer to a reload: `{"reloaded": counts, "ms": t}`, audited with both.
pub(crate) fn answer(done: Result<(Value, u64), Box<Response>>) -> Response {
    match done {
        Ok((counts, ms)) => {
            let mut resp = Json(json!({ "reloaded": counts, "ms": ms })).into_response();
            resp.extensions_mut()
                .insert(AuditDetail(json!({ "reloaded": counts, "ms": ms })));
            resp
        }
        Err(resp) => *resp,
    }
}
