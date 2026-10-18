//! `GET /healthz`: 200 while the engine thread runs and the persistence worker keeps up,
//! 503 otherwise. Unauthenticated (loopback by default); the Docker healthcheck hits it.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::router::ApiState;

/// What `/healthz` asks of the rest of the process; `bin` implements it over the engine
/// thread handle, the fast-tick counter and `silvermere-db`'s `PersistStatus`.
pub trait HealthProbe: Send + Sync {
    /// Whether the engine thread is running.
    fn engine_alive(&self) -> bool;
    /// The fast ticks the engine has run.
    fn engine_tick(&self) -> u64;
    /// Whether the persistence worker's last batch committed.
    fn persist_healthy(&self) -> bool;
}

pub async fn healthz(State(state): State<ApiState>) -> (StatusCode, Json<Value>) {
    let alive = state.health.engine_alive();
    let persist = state.health.persist_healthy();
    let ok = alive && persist;
    let body = json!({
        "status": if ok { "ok" } else { "unavailable" },
        "engine_tick": state.health.engine_tick(),
        "persist": if persist { "healthy" } else { "unhealthy" },
        "engine": if alive { "running" } else { "dead" },
    });
    let code = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(body))
}
