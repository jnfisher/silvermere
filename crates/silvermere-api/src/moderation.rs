//! Moderation routes: disconnect a player, a line to one player, bans, jail and mutes on an
//! account, IP bans, and closing the realm to all but sysops. `bin` does the work
//! ([`Moderator`]): it stores the sanction, then tells the running game.
//!
//! A player is named by his character's first name or his account's username. Durations
//! are `for_secs` from now; absent (or `null`) means until lifted.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Caller, Scope};
use crate::events::BoxFuture;
use crate::router::{ApiError, ApiState, AuditDetail};

/// The longest sanction with an end: ten years.
pub const MAX_FOR_SECS: u64 = 10 * 365 * 86_400;
/// The longest reason or line accepted, in characters.
pub const MAX_TEXT: usize = 200;

/// What an account sanction stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No login.
    Ban,
    /// Held in the jail room.
    Jail,
    /// No gossip or auction.
    Mute,
}

impl Kind {
    /// The route word and the stored kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Ban => "ban",
            Kind::Jail => "jail",
            Kind::Mute => "mute",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Kind::Ban, Kind::Jail, Kind::Mute]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// One moderation action, as the routes and the web console ask for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Disconnect the player, saved, telling him `reason` (may be empty).
    Kick { player: String, reason: String },
    /// A line to the player.
    Tell { player: String, text: String },
    /// Put a ban, jail or mute on the player's account, replacing one of the same kind.
    Put {
        player: String,
        kind: Kind,
        for_secs: Option<u64>,
        reason: String,
    },
    /// Lift the player's ban, jail or mute.
    Lift { player: String, kind: Kind },
    /// Refuse telnet connections from an address or CIDR block.
    BanIp {
        network: String,
        for_secs: Option<u64>,
        reason: String,
    },
    /// Lift the ban of an address or block.
    UnbanIp { network: String },
    /// Close the realm to all but sysops; the players in it are saved and disconnected.
    CloseRealm { reason: String },
    /// Open the realm again.
    OpenRealm,
    /// A line typed for the player as if he had typed it, by a sysop watching his screen;
    /// nothing on his screen says so.
    Input { player: String, line: String },
    /// One in-game `SYSOP ...` line, run for the caller outside the game.
    SysopLine { line: String },
}

/// Why an action was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModError {
    /// No character or account has that name.
    NoSuchPlayer,
    /// The player is not connected (kick, tell).
    NotConnected,
    /// A door account: the BBS manages its own users.
    DoorAccount,
    /// Not an address or CIDR block.
    BadNetwork,
    /// No ban of that address or block.
    NoSuchBan,
    /// The player had no sanction of that kind.
    NotSanctioned,
    /// The engine did not answer: `engine_unavailable` or `engine_timeout`.
    Engine(&'static str),
    /// The store failed; the cause is in the server log.
    Store(String),
    /// This server has no moderation (the tests' in-process servers).
    Unavailable,
    /// Not a `SYSOP ...` line.
    NotSysopLine,
    /// The `SYSOP` sub-command needs the sysop's character in the realm.
    NeedsCharacter,
    /// Every session slot is taken.
    ServerFull,
}

impl ModError {
    pub(crate) fn api(&self) -> ApiError {
        match self {
            ModError::NoSuchPlayer => ApiError(StatusCode::NOT_FOUND, "no_such_player"),
            ModError::NotConnected => ApiError(StatusCode::CONFLICT, "not_connected"),
            ModError::DoorAccount => ApiError(StatusCode::CONFLICT, "door_account"),
            ModError::BadNetwork => ApiError(StatusCode::BAD_REQUEST, "bad_network"),
            ModError::NoSuchBan => ApiError(StatusCode::NOT_FOUND, "no_such_ban"),
            ModError::NotSanctioned => ApiError(StatusCode::NOT_FOUND, "not_sanctioned"),
            ModError::Engine("engine_timeout") => {
                ApiError(StatusCode::GATEWAY_TIMEOUT, "engine_timeout")
            }
            ModError::Engine(_) => ApiError(StatusCode::SERVICE_UNAVAILABLE, "engine_unavailable"),
            ModError::Store(_) => ApiError(StatusCode::INTERNAL_SERVER_ERROR, "store_failed"),
            ModError::Unavailable => {
                ApiError(StatusCode::SERVICE_UNAVAILABLE, "moderation_unavailable")
            }
            ModError::NotSysopLine => ApiError(StatusCode::BAD_REQUEST, "not_sysop_line"),
            ModError::NeedsCharacter => ApiError(StatusCode::CONFLICT, "needs_character"),
            ModError::ServerFull => ApiError(StatusCode::SERVICE_UNAVAILABLE, "server_full"),
        }
    }
}

/// What the routes ask of the rest of the process. `by` names who acts (the token's name
/// here; the sysop's account in the web console), for the stored row and the audit.
pub trait Moderator: Send + Sync {
    /// Does `action`; the JSON answer.
    fn act(&self, action: Action, by: String) -> BoxFuture<'_, Result<Value, ModError>>;
    /// The realm's state, the sanctions in force and the IP bans.
    fn status(&self) -> BoxFuture<'_, Result<Value, ModError>>;
    /// The player's screen: what it shows now, then what follows, until he leaves.
    fn watch(&self, player: String) -> BoxFuture<'_, Result<crate::spectate::Watch, ModError>> {
        let _ = player;
        Box::pin(async { Err(ModError::Unavailable) })
    }
}

/// A server without moderation.
pub struct NoModeration;

impl Moderator for NoModeration {
    fn act(&self, _: Action, _: String) -> BoxFuture<'_, Result<Value, ModError>> {
        Box::pin(async { Err(ModError::Unavailable) })
    }

    fn status(&self) -> BoxFuture<'_, Result<Value, ModError>> {
        Box::pin(async { Err(ModError::Unavailable) })
    }
}

/// A reason or line: at most [`MAX_TEXT`] characters, no control characters.
fn text_ok(t: &str) -> bool {
    t.chars().count() <= MAX_TEXT && !t.chars().any(char::is_control)
}

fn duration_ok(for_secs: Option<u64>) -> bool {
    for_secs.is_none_or(|s| (1..=MAX_FOR_SECS).contains(&s))
}

/// The body, or `{}` when it is empty.
fn body<T: for<'de> Deserialize<'de> + Default>(raw: &[u8]) -> Result<T, ApiError> {
    if raw.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(raw).map_err(|_| ApiError(StatusCode::BAD_REQUEST, "bad_body"))
}

pub(crate) async fn run(
    state: &ApiState,
    caller: &Caller,
    action: Action,
    audit: Value,
) -> Response {
    match state.moderation.act(action, caller.name.clone()).await {
        Ok(answer) => {
            let mut resp = Json(answer).into_response();
            resp.extensions_mut().insert(AuditDetail(audit));
            resp
        }
        Err(e) => {
            if let ModError::Store(cause) = &e {
                tracing::error!(%cause, "moderation: the store failed");
            }
            e.api().into_response()
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ReasonBody {
    #[serde(default)]
    pub reason: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct TextBody {
    #[serde(default)]
    pub text: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PutBody {
    pub for_secs: Option<u64>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct IpBody {
    #[serde(default)]
    pub network: String,
    pub for_secs: Option<u64>,
    #[serde(default)]
    pub reason: String,
}

/// `GET /v1/moderation`.
pub async fn status(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if !caller.allows(Scope::Read) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    match state.moderation.status().await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.api().into_response(),
    }
}

/// `POST /v1/players/{name}/kick`.
pub async fn kick(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(player): Path<String>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b: ReasonBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let reason = b.reason.trim().to_string();
    if !text_ok(&reason) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    let audit = json!({ "player": player, "reason": reason });
    run(&state, &caller, Action::Kick { player, reason }, audit).await
}

/// `POST /v1/players/{name}/tell`.
pub async fn tell(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(player): Path<String>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Write) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b: TextBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let text = b.text.trim().to_string();
    if text.is_empty() || !text_ok(&text) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    let audit = json!({ "player": player, "text": text });
    run(&state, &caller, Action::Tell { player, text }, audit).await
}

/// `PUT /v1/players/{name}/{ban|jail|mute}`.
pub async fn put(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path((player, kind)): Path<(String, String)>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let Some(kind) = Kind::parse(&kind) else {
        return ApiError(StatusCode::NOT_FOUND, "not_found").into_response();
    };
    let b: PutBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let reason = b.reason.trim().to_string();
    if !text_ok(&reason) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    if !duration_ok(b.for_secs) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_duration").into_response();
    }
    let audit = json!({ "player": player, "kind": kind.as_str(), "for_secs": b.for_secs, "reason": reason });
    let action = Action::Put {
        player,
        kind,
        for_secs: b.for_secs,
        reason,
    };
    run(&state, &caller, action, audit).await
}

/// `DELETE /v1/players/{name}/{ban|jail|mute}`.
pub async fn lift(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path((player, kind)): Path<(String, String)>,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let Some(kind) = Kind::parse(&kind) else {
        return ApiError(StatusCode::NOT_FOUND, "not_found").into_response();
    };
    let audit = json!({ "player": player, "kind": kind.as_str() });
    run(&state, &caller, Action::Lift { player, kind }, audit).await
}

/// `POST /v1/ip-bans`.
pub async fn ban_ip(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b: IpBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let reason = b.reason.trim().to_string();
    if !text_ok(&reason) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    if !duration_ok(b.for_secs) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_duration").into_response();
    }
    let audit = json!({ "network": b.network, "for_secs": b.for_secs, "reason": reason });
    let action = Action::BanIp {
        network: b.network,
        for_secs: b.for_secs,
        reason,
    };
    run(&state, &caller, action, audit).await
}

/// `DELETE /v1/ip-bans`.
pub async fn unban_ip(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b: IpBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let audit = json!({ "network": b.network });
    run(
        &state,
        &caller,
        Action::UnbanIp { network: b.network },
        audit,
    )
    .await
}

/// `POST /v1/realm/close`.
pub async fn close_realm(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    raw: axum::body::Bytes,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let b: ReasonBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let reason = b.reason.trim().to_string();
    if !text_ok(&reason) {
        return ApiError(StatusCode::BAD_REQUEST, "bad_text").into_response();
    }
    let audit = json!({ "reason": reason });
    run(&state, &caller, Action::CloseRealm { reason }, audit).await
}

/// `POST /v1/realm/open`.
pub async fn open_realm(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
) -> Response {
    if !caller.allows(Scope::Admin) {
        return ApiError(StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    run(&state, &caller, Action::OpenRealm, json!({})).await
}
