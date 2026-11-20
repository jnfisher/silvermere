//! The router, the shared state, the bearer-token guard with its audit trail, and `serve`.

use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use silvermere_engine::{AdminRequest, EngineInput};

use crate::auth::Scope;
use crate::auth::{Token, authenticate};
use crate::backup::BackupProbe;
use crate::events::{EVENT_BUFFER, EventLog, EventRecord, NewEvent};
use crate::health::HealthProbe;
use crate::reload::WorldLoader;
use crate::{
    accounts, backup, broadcast as broadcast_route, events, health, metrics, reload, reserved, who,
};

/// How long a request waits for the engine thread to answer.
const ENGINE_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything the handlers share. `Clone` is cheap.
#[derive(Clone)]
pub struct ApiState {
    /// The engine's input channel (`EngineInput::Admin`).
    pub engine: Sender<EngineInput>,
    /// Live events for `/v1/events`; `bin` sends each event after it is in the log.
    pub events: broadcast::Sender<EventRecord>,
    /// The `events` table.
    pub log: Arc<dyn EventLog>,
    /// Engine and persistence health for `/healthz`.
    pub health: Arc<dyn HealthProbe>,
    /// Renders `/metrics`.
    pub metrics: PrometheusHandle,
    /// The configured tokens.
    pub tokens: Arc<Vec<Token>>,
    /// Reads the static world tables for `/v1/reload-world`.
    pub world: Arc<dyn WorldLoader>,
    /// The scheduled backup's state for `/v1/backup` and `/healthz` (T-150).
    pub backup: Arc<dyn BackupProbe>,
    /// The planned shutdown for `/v1/shutdown`.
    pub shutdown: Arc<dyn crate::shutdown::ShutdownPlan>,
    /// Kicks, bans, jail, mutes and the closed realm for the moderation routes.
    pub moderation: Arc<dyn crate::moderation::Moderator>,
    /// The web console's token: made at start, held in memory only, never configured. A
    /// call with it names the signed-in sysop in [`ACTING_FOR`] and is audited as
    /// `web:<name>`; without that header it is refused.
    pub console: Option<Token>,
}

/// The header a web console call names its sysop in.
pub const ACTING_FOR: &str = "x-silvermere-sysop";

/// A sysop's name as the console may give it: 1 to 64 letters, digits, `-`, `_`, `.`.
fn acting_name_ok(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl ApiState {
    /// A fresh event bus of [`EVENT_BUFFER`] events, for `events`.
    pub fn event_bus() -> broadcast::Sender<EventRecord> {
        broadcast::channel(EVENT_BUFFER).0
    }
}

/// An error answered as `{"error": code, "message": text}`.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub &'static str);

/// The human text that goes with an error code.
fn message_of(code: &str) -> &'static str {
    match code {
        "unauthorized" => "a valid bearer token is required",
        "forbidden" => "the token lacks the scope this route needs",
        "bad_text" => {
            "text must have no control characters and be 1 to 1000 characters for a broadcast, \
             at most 200 for a reason or a message"
        }
        "bad_query" => "the query string is not valid for this route",
        "engine_unavailable" => "the game engine is not running",
        "engine_timeout" => "the game engine did not answer in time",
        "reload_failed" => "the world tables were not reloaded; the previous tables are in place",
        "bad_id" => "the account id must be a positive integer",
        "no_such_account" => "no account has that id",
        "delete_failed" => "the account was not deleted",
        "bad_delay" => "in_secs must be 0 to 86400",
        "bad_body" => "the request body is not the JSON this route takes",
        "shutdown_unavailable" => "this server cannot schedule its own shutdown",
        "no_such_player" => "no character or account has that name",
        "not_connected" => "the player is not connected",
        "door_account" => "a BBS door account: the BBS manages its own users",
        "bad_network" => "network must be an IP address or CIDR block",
        "no_such_ban" => "no ban of that address or block",
        "not_sanctioned" => "the player has no sanction of that kind",
        "bad_duration" => "for_secs must be 1 to 315360000, or absent for until lifted",
        "store_failed" => "the database refused the change; the cause is in the server log",
        "moderation_unavailable" => "this server has no moderation",
        "backup_unavailable" => "this server takes no backups",
        "not_found" => "no such route",
        "internal" => "internal error",
        _ => "error",
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({ "error": self.1, "message": message_of(self.1) })),
        )
            .into_response()
    }
}

/// `Err(403)` unless the caller's scopes allow `needed`.
pub fn require(caller: &crate::auth::Caller, needed: Scope) -> Result<(), ApiError> {
    if caller.allows(needed) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::FORBIDDEN, "forbidden"))
    }
}

/// One route of the contract, for the docs and the tests that keep them equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSpec {
    pub method: &'static str,
    pub path: String,
    /// `None` for the open routes.
    pub scope: Option<Scope>,
    /// `false` for the reserved `501` routes.
    pub built: bool,
}

/// Every route the router registers (method and path), in documentation order.
pub fn routes() -> Vec<RouteSpec> {
    let r = |method, path: &str, scope, built| RouteSpec {
        method,
        path: path.to_string(),
        scope,
        built,
    };
    let mut v = vec![
        r("GET", "/healthz", None, true),
        r("GET", "/metrics", None, true),
        r("GET", "/v1/who", Some(Scope::Read), true),
        r("GET", "/v1/events", Some(Scope::Read), true),
        r("GET", "/v1/backup", Some(Scope::Read), true),
        r("POST", "/v1/backup", Some(Scope::Admin), true),
        r("POST", "/v1/broadcast", Some(Scope::Write), true),
        r("POST", "/v1/reload-world", Some(Scope::Write), true),
        r("DELETE", "/v1/accounts/{id}", Some(Scope::Admin), true),
        r("GET", "/v1/shutdown", Some(Scope::Read), true),
        r("POST", "/v1/shutdown", Some(Scope::Admin), true),
        r("DELETE", "/v1/shutdown", Some(Scope::Admin), true),
        r("POST", "/v1/restart", Some(Scope::Admin), true),
        r("DELETE", "/v1/restart", Some(Scope::Admin), true),
        r("GET", "/v1/moderation", Some(Scope::Read), true),
        r("POST", "/v1/players/{name}/kick", Some(Scope::Admin), true),
        r("POST", "/v1/players/{name}/tell", Some(Scope::Write), true),
        r("PUT", "/v1/players/{name}/{kind}", Some(Scope::Admin), true),
        r(
            "DELETE",
            "/v1/players/{name}/{kind}",
            Some(Scope::Admin),
            true,
        ),
        r("POST", "/v1/ip-bans", Some(Scope::Admin), true),
        r("DELETE", "/v1/ip-bans", Some(Scope::Admin), true),
        r("POST", "/v1/realm/close", Some(Scope::Admin), true),
        r("POST", "/v1/realm/open", Some(Scope::Admin), true),
    ];
    for table in reserved::RESOURCES {
        for (method, suffix) in [
            ("GET", ""),
            ("POST", ""),
            ("GET", "/{id}"),
            ("PUT", "/{id}"),
            ("DELETE", "/{id}"),
        ] {
            v.push(r(
                method,
                &format!("/v1/world/{table}{suffix}"),
                Some(Scope::Admin),
                false,
            ));
        }
    }
    v
}

/// Extra fields for the audit row, attached to a response by a handler.
#[derive(Clone)]
pub struct AuditDetail(pub Value);

/// Sends `req` to the engine and waits for the answer on `rx`.
pub async fn ask<T: Send + 'static>(
    state: &ApiState,
    req: AdminRequest,
    rx: Receiver<T>,
) -> Result<T, ApiError> {
    state
        .engine
        .send(EngineInput::Admin(req))
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "engine_unavailable"))?;
    let waited = tokio::task::spawn_blocking(move || rx.recv_timeout(ENGINE_TIMEOUT))
        .await
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal"))?;
    waited.map_err(|e| match e {
        RecvTimeoutError::Timeout => ApiError(StatusCode::GATEWAY_TIMEOUT, "engine_timeout"),
        RecvTimeoutError::Disconnected => {
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "engine_unavailable")
        }
    })
}

/// The admin router. `/healthz` and `/metrics` are open; everything under `/v1` needs a
/// bearer token and is audited.
pub fn router(state: ApiState) -> Router {
    let v1 = Router::new()
        .route("/v1/who", get(who::who))
        .route("/v1/broadcast", post(broadcast_route::broadcast))
        .route("/v1/events", get(events::stream))
        .route("/v1/backup", get(backup::status).post(backup::run_now))
        .route("/v1/reload-world", post(reload::reload_world))
        .route("/v1/accounts/{id}", delete(accounts::delete))
        .route(
            "/v1/shutdown",
            get(crate::shutdown::status)
                .post(crate::shutdown::schedule)
                .delete(crate::shutdown::cancel),
        )
        .route(
            "/v1/restart",
            post(crate::shutdown::schedule_restart).delete(crate::shutdown::cancel),
        )
        .route("/v1/moderation", get(crate::moderation::status))
        .route("/v1/players/{name}/kick", post(crate::moderation::kick))
        .route("/v1/players/{name}/tell", post(crate::moderation::tell))
        .route(
            "/v1/players/{name}/{kind}",
            axum::routing::put(crate::moderation::put).delete(crate::moderation::lift),
        )
        .route(
            "/v1/ip-bans",
            post(crate::moderation::ban_ip).delete(crate::moderation::unban_ip),
        )
        .route("/v1/realm/close", post(crate::moderation::close_realm))
        .route("/v1/realm/open", post(crate::moderation::open_realm));
    let v1 = reserved::register(v1).layer(middleware::from_fn_with_state(state.clone(), guard));
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/metrics", get(metrics::render))
        .merge(v1)
        .with_state(state)
}

/// Serves the admin API on `listener` until `shutdown` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: ApiState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

/// Token check (401), then the handler, then the audit row for the call.
async fn guard(State(state): State<ApiState>, mut req: Request, next: Next) -> Response {
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    // the web console's own token first: it acts for the sysop it names
    let console = state
        .console
        .as_ref()
        .and_then(|t| authenticate(std::slice::from_ref(t), header));
    let caller = match console {
        Some(c) => {
            let acting = req
                .headers()
                .get(ACTING_FOR)
                .and_then(|v| v.to_str().ok())
                .filter(|n| acting_name_ok(n));
            match acting {
                Some(name) => Some(crate::auth::Caller {
                    name: format!("web:{name}"),
                    scopes: c.scopes,
                }),
                None => None,
            }
        }
        None => authenticate(&state.tokens, header),
    };
    let Some(caller) = caller else {
        let mut resp = ApiError(StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        resp.headers_mut()
            .insert(header::WWW_AUTHENTICATE, "Bearer".parse().expect("static"));
        return resp;
    };
    let route = req
        .uri()
        .path()
        .trim_start_matches("/v1/")
        .trim_end_matches('/')
        .replace('/', ".");
    let method = req.method().to_string();
    // the web console reads every few seconds: those reads are not audited
    let console_read = caller.name.starts_with("web:") && req.method() == axum::http::Method::GET;
    req.extensions_mut().insert(caller.clone());
    let mut resp = next.run(req).await;
    if console_read {
        tracing::debug!(target: "audit", route = %route, token_name = %caller.name, "console read");
        return resp;
    }
    let status = resp.status().as_u16();
    let detail = resp.extensions_mut().remove::<AuditDetail>();

    tracing::info!(target: "audit", route = %route, token_name = %caller.name, status);
    let mut payload = json!({ "token": caller.name, "method": method, "status": status });
    if let (Some(AuditDetail(Value::Object(extra))), Some(obj)) = (detail, payload.as_object_mut())
    {
        obj.extend(extra);
    }
    match state
        .log
        .append(NewEvent::new(&format!("admin.{route}"), payload))
        .await
    {
        // No receiver is fine: nobody is streaming.
        Ok(record) => {
            let _ = state.events.send(record);
        }
        Err(e) => tracing::warn!(target: "audit", %e, route = %route, "audit row not written"),
    }
    resp
}
