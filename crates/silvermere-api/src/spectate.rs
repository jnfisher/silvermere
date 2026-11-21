//! Watching a player and acting for him, and the original's `SYSOP` commands from outside
//! the game.
//!
//! `GET /v1/players/{name}/screen` streams what the player sees, as server-sent events: one
//! `screen` event with the screen as it stands (the last few KB), an `out` event for each
//! write after it (the game's output and the echo of what he types), and `end` when he
//! leaves. `POST /v1/players/{name}/input` types a line for him: the game takes it as his
//! own, and nothing on his screen shows it. The player is not told he is watched. Both need
//! the `admin` scope and are audited, the watch included (from the web console too).
//!
//! `POST /v1/sysop` runs one in-game `SYSOP ...` line for the caller, outside the game, and
//! answers what it would have shown him.

use std::convert::Infallible;

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::auth::{Caller, Scope};
use crate::moderation::{Action, run};
use crate::router::{ApiError, ApiState, AuditDetail};

/// The longest line a player can type (the line editor's limit).
pub const MAX_LINE: usize = 254;

/// A player's screen, as [`crate::Moderator::watch`] gives it: text, ANSI codes kept.
pub struct Watch {
    /// The character's name.
    pub name: String,
    /// The screen as it stands: the latest output.
    pub recent: String,
    /// What follows; ends when the player leaves.
    pub stream: mpsc::Receiver<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LineBody {
    #[serde(default)]
    pub line: String,
}

/// A typed line: at most [`MAX_LINE`] characters, no control characters. Empty is a line
/// too (Enter at a prompt).
fn line_ok(t: &str) -> bool {
    t.chars().count() <= MAX_LINE && !t.chars().any(char::is_control)
}

fn line_body(raw: &[u8]) -> Result<LineBody, ApiError> {
    serde_json::from_slice(raw).map_err(|_| ApiError(StatusCode::BAD_REQUEST, "bad_body"))
}

/// `GET /v1/players/{name}/screen`.
pub async fn screen(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(player): Path<String>,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let watch = match state.moderation.watch(player.clone()).await {
        Ok(w) => w,
        Err(e) => return e.api().into_response(),
    };
    let Watch {
        name,
        recent,
        mut stream,
    } = watch;
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    let first = Event::default()
        .event("screen")
        .json_data(json!({ "player": name, "text": recent }))
        .unwrap_or_else(|_| Event::default().event("error"));
    tokio::spawn(async move {
        if tx.send(Ok(first)).await.is_err() {
            return;
        }
        while let Some(text) = stream.recv().await {
            let e = Event::default()
                .event("out")
                .json_data(json!({ "text": text }))
                .unwrap_or_else(|_| Event::default().event("error"));
            if tx.send(Ok(e)).await.is_err() {
                return;
            }
        }
        let _ = tx.send(Ok(Event::default().event("end").data("{}"))).await;
    });
    let mut resp = Sse::new(ReceiverStream::new(rx))
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    resp.extensions_mut()
        .insert(AuditDetail(json!({ "player": player })));
    resp
}

/// `POST /v1/players/{name}/input`.
pub async fn input(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(player): Path<String>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b = match line_body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    if !line_ok(&b.line) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_line").into_response();
    }
    let audit = json!({ "player": player, "line": b.line });
    let action = Action::Input {
        player,
        line: b.line,
    };
    run(&state, &caller, action, audit).await
}

/// `POST /v1/sysop`.
pub async fn sysop_line(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b = match line_body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let line = b.line.trim().to_string();
    if line.is_empty() || !line_ok(&line) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_line").into_response();
    }
    let audit = json!({ "line": line });
    run(&state, &caller, Action::SysopLine { line }, audit).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_what_a_player_could_type() {
        assert!(line_ok(""));
        assert!(line_ok("look"));
        assert!(line_ok(&"x".repeat(MAX_LINE)));
        assert!(!line_ok(&"x".repeat(MAX_LINE + 1)));
        assert!(!line_ok("look\r\nquit"));
    }
}
