//! `POST /v1/shutdown` and `DELETE /v1/shutdown`: a planned shutdown. The players in the
//! game are warned as the time comes closer, then the server stops as it does on SIGTERM
//! (every character saved). Scheduling again replaces the plan; `DELETE` cancels it.
//! `POST /v1/restart` plans a restart the same way: the server stops, then starts again in
//! the same process. `GET /v1/shutdown` says what is planned. One plan at a time: a restart
//! replaces a planned shutdown and the other way round.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{Caller, Scope};
use crate::router::{ApiError, ApiState, AuditDetail};

/// The longest wait accepted: a day.
pub const MAX_DELAY_SECS: u64 = 86_400;
/// The wait when the request names none: five minutes.
pub const DEFAULT_DELAY_SECS: u64 = 300;
/// The longest reason accepted, in characters.
pub const MAX_MESSAGE: usize = 200;

/// A scheduled shutdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// When the server stops, RFC 3339 in UTC.
    pub at: String,
    /// Seconds from now.
    pub in_secs: u64,
    /// A restart rather than a stop.
    pub restart: bool,
}

/// What the shutdown routes ask of the rest of the process; `bin` runs the countdown and
/// stops the server.
pub trait ShutdownPlan: Send + Sync {
    /// Stops the server `after` from now, warning the players with `message` (empty for
    /// none), in place of any earlier plan. `Err` when this server cannot.
    fn schedule(&self, after: Duration, message: String) -> Result<Planned, String>;
    /// Cancels the plan; whether there was one.
    fn cancel(&self) -> bool;
    /// Restarts the server `after` from now, as [`ShutdownPlan::schedule`] stops it. `Err`
    /// when this server cannot, or a restart would not come back (the reason).
    fn schedule_restart(&self, after: Duration, message: String) -> Result<Planned, String> {
        let _ = (after, message);
        Err("this server cannot restart itself".into())
    }
    /// The plan in force, `in_secs` counted from now.
    fn current(&self) -> Option<Planned> {
        None
    }
}

/// A server that cannot stop itself (the tests' in-process servers).
pub struct NoShutdown;

impl ShutdownPlan for NoShutdown {
    fn schedule(&self, _: Duration, _: String) -> Result<Planned, String> {
        Err("this server does not run a planned shutdown".into())
    }

    fn cancel(&self) -> bool {
        false
    }
}

#[derive(Deserialize, Default)]
pub struct Body {
    /// Seconds from now, 0 to [`MAX_DELAY_SECS`]; [`DEFAULT_DELAY_SECS`] when absent.
    pub in_secs: Option<u64>,
    /// Said to the players with every warning.
    #[serde(default)]
    pub message: String,
}

/// `GET /v1/shutdown`: `{"planned": null}` or `{"planned": {"at", "in_secs", "restart"}}`,
/// `in_secs` counted from now.
pub async fn status(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if !caller.allows(Scope::Read) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let planned = state
        .shutdown
        .current()
        .map(|p| json!({ "at": p.at, "in_secs": p.in_secs, "restart": p.restart }));
    Json(json!({ "planned": planned })).into_response()
}

pub async fn schedule(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    plan(state, caller, raw, false).await
}

/// `POST /v1/restart`.
pub async fn schedule_restart(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    plan(state, caller, raw, true).await
}

async fn plan(state: ApiState, caller: Caller, raw: axum::body::Bytes, restart: bool) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    // an empty body takes every default
    let body: Body = if raw.iter().all(u8::is_ascii_whitespace) {
        Body::default()
    } else {
        match serde_json::from_slice(&raw) {
            Ok(b) => b,
            Err(_) => return ApiError(StatusCode::BAD_REQUEST, "bad_body").into_response(),
        }
    };
    let in_secs = body.in_secs.unwrap_or(DEFAULT_DELAY_SECS);
    if in_secs > MAX_DELAY_SECS {
        return ApiError(StatusCode::BAD_REQUEST, "bad_delay").into_response();
    }
    // the warnings go out as broadcasts: the same rule as `POST /v1/broadcast`
    let message = body.message.trim().to_owned();
    if message.chars().count() > MAX_MESSAGE || message.chars().any(char::is_control) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    let after = Duration::from_secs(in_secs);
    let planned = if restart {
        state.shutdown.schedule_restart(after, message.clone())
    } else {
        state.shutdown.schedule(after, message.clone())
    };
    match planned {
        Ok(planned) => {
            let mut resp = Json(json!({
                "at": planned.at,
                "in_secs": planned.in_secs,
                "restart": planned.restart,
            }))
            .into_response();
            resp.extensions_mut().insert(AuditDetail(json!({
                "in_secs": planned.in_secs,
                "at": planned.at,
                "text": message,
                "restart": restart,
            })));
            resp
        }
        Err(e) if restart => {
            tracing::warn!(%e, "planned restart refused");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "restart_refused", "message": e })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(%e, "planned shutdown refused");
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "shutdown_unavailable").into_response()
        }
    }
}

pub async fn cancel(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let cancelled = state.shutdown.cancel();
    let mut resp = Json(json!({ "cancelled": cancelled })).into_response();
    resp.extensions_mut()
        .insert(AuditDetail(json!({ "cancelled": cancelled })));
    resp
}
