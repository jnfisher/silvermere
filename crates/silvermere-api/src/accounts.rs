//! `DELETE /v1/accounts/{id}` (T-069): deletes an account through the running engine, so
//! that nothing it holds for the account (gang member count, bank books, limited-item
//! ownership) is written back after the rows are gone. The engine logs a character in the
//! game off, runs `session::on_account_deleted` (what the host's account-deleted hook
//! `dlarou` did) and queues the rows' deletion behind its own writes. Answers
//! `200 {"deleted": {"account", "character"}}` once that is queued, `404` for an unknown
//! account.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;
use silvermere_engine::AdminRequest;
use silvermere_engine::session::AccountDeletion;
use silvermere_types::ids::AccountId;

use crate::auth::{Caller, Scope};
use crate::router::{ApiError, ApiState, AuditDetail, ask, require};

pub async fn delete(
    State(state): State<ApiState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    if let Err(e) = require(&caller, Scope::Admin) {
        return e.into_response();
    }
    let Some(account) = id.parse::<i64>().ok().filter(|n| *n > 0).map(AccountId) else {
        return ApiError(StatusCode::BAD_REQUEST, "bad_id").into_response();
    };
    let (reply, rx) = std::sync::mpsc::channel();
    let req = AdminRequest::DeleteAccount { account, reply };
    let (mut resp, detail) = match ask(&state, req, rx).await {
        Ok(AccountDeletion::Queued { character }) => {
            let deleted = json!({ "account": account.0, "character": character });
            (Json(json!({ "deleted": deleted })).into_response(), deleted)
        }
        Ok(AccountDeletion::NotFound) => (
            ApiError(StatusCode::NOT_FOUND, "no_such_account").into_response(),
            json!({ "account": account.0 }),
        ),
        Ok(AccountDeletion::Failed(e)) => {
            tracing::error!(account = account.0, %e, "account deletion failed");
            (
                ApiError(StatusCode::INTERNAL_SERVER_ERROR, "delete_failed").into_response(),
                json!({ "account": account.0, "failure": e }),
            )
        }
        Err(e) => return e.into_response(),
    };
    resp.extensions_mut().insert(AuditDetail(detail));
    resp
}
