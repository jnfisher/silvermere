//! `GET /v1/backup` (T-150): the scheduled backup's settings, its next and last run (with the
//! after-backup command's result) and the backups on disk, as `silvermere backup --list`
//! shows them.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{Value, json};

use crate::auth::{Caller, Scope};
use crate::router::{ApiError, ApiState};

/// What the backup routes ask of the rest of the process; `bin` implements it over the
/// schedule's state and the backups folder.
pub trait BackupProbe: Send + Sync {
    /// Whether the last scheduled backup was made; `None` before the first.
    fn last_ok(&self) -> Option<bool>;
    /// The whole status, the body of `GET /v1/backup`.
    fn status(&self) -> Value;
    /// Makes a backup now, as a scheduled one is made (pruned, the after-backup command
    /// run); the run as `last` shows it, or `Err` when this server has no backups. Blocks.
    fn run_now(&self) -> Result<Value, String> {
        Err("this server takes no backups".into())
    }
}

/// A server with no backup schedule (the tests' in-process servers).
pub struct NoBackups;

impl BackupProbe for NoBackups {
    fn last_ok(&self) -> Option<bool> {
        None
    }

    fn status(&self) -> Value {
        json!({ "enabled": false, "next": null, "last": null, "backups": [] })
    }
}

/// `POST /v1/backup`: a backup now. `{"ok": true, "file": ..., ...}` as `last` in
/// `GET /v1/backup`; a backup that failed is still a `200` with `"ok": false` and `error`.
pub async fn run_now(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(axum::http::StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let probe = state.backup.clone();
    match tokio::task::spawn_blocking(move || probe.run_now()).await {
        Ok(Ok(v)) => {
            let mut resp = Json(v.clone()).into_response();
            resp.extensions_mut().insert(crate::router::AuditDetail(
                json!({ "ok": v["ok"], "file": v["file"] }),
            ));
            resp
        }
        Ok(Err(_)) => ApiError(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "backup_unavailable",
        )
        .into_response(),
        Err(_) => {
            ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
        }
    }
}

pub async fn status(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if !caller.allows(Scope::Read) {
        return ApiError(axum::http::StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let probe = state.backup.clone();
    match tokio::task::spawn_blocking(move || probe.status()).await {
        Ok(v) => Json(v).into_response(),
        Err(_) => {
            ApiError(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
        }
    }
}
