//! `GET /v1/who`: the players `display_scan_of_users` would list, in `listslot` order, as
//! JSON.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;
use silvermere_engine::AdminRequest;

use crate::auth::{Caller, Scope};
use crate::router::{ApiError, ApiState, ask};

pub async fn who(State(state): State<ApiState>, Extension(caller): Extension<Caller>) -> Response {
    if !caller.allows(Scope::Read) {
        return ApiError(axum::http::StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let (reply, rx) = std::sync::mpsc::channel();
    let mut rows = match ask(&state, AdminRequest::Who { reply }, rx).await {
        Ok(rows) => rows,
        Err(e) => return e.into_response(),
    };
    // The in-game list hides invisible sysops from players; the API shows them to the
    // `admin` scope only.
    if !caller.scopes.contains(&Scope::Admin) {
        rows.retain(|r| !r.hidden);
    }
    rows.sort_by_key(|r| r.listslot);
    let players: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "session_id": r.id.0,
                "first_name": r.first_name,
                "last_name": r.last_name,
                "title": r.title,
                "class": r.class,
                "race": r.race,
                "level": r.level,
                "alignment": r.alignment,
                "gang": r.gang,
                "idle_secs": r.idle_secs,
                "hidden": r.hidden,
            })
        })
        .collect();
    Json(json!({ "players": players })).into_response()
}
