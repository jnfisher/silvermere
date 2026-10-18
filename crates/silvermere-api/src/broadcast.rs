//! `POST /v1/broadcast {"text": "..."}`: the sysop `SPEAK` to everyone in the game. Answers
//! `{"delivered": n}`, `n` the players told (`tell_game`'s count).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;
use silvermere_engine::AdminRequest;

use crate::auth::{Caller, Scope};
use crate::router::{ApiError, ApiState, AuditDetail, ask};

/// The longest text accepted, in characters (the API's own limit).
pub const MAX_TEXT: usize = 1000;

#[derive(Deserialize)]
pub struct Body {
    pub text: String,
}

pub async fn broadcast(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Json(body): Json<Body>,
) -> Response {
    if !caller.allows(Scope::Write) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    // No control characters (C0, DEL, C1: some terminals act on U+009B CSI): the API sends only what the in-game
    // `SYSOP SPEAK` line editor could type.
    if body.text.trim().is_empty()
        || body.text.chars().count() > MAX_TEXT
        || body.text.chars().any(char::is_control)
    {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    let (reply, rx) = std::sync::mpsc::channel();
    let req = AdminRequest::Broadcast {
        text: body.text.clone(),
        reply,
    };
    let delivered = match ask(&state, req, rx).await {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };
    let mut resp = Json(json!({ "delivered": delivered })).into_response();
    resp.extensions_mut().insert(AuditDetail(json!({
        "text": body.text,
        "delivered": delivered,
    })));
    resp
}
