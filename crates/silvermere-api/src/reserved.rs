//! The world-editing routes reserved for the editor project: every one answers `501` with a
//! pointer to `docs/admin-api.md#reserved-routes` (after the auth and scope checks, so an
//! editor can probe its token against them).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::json;

use crate::auth::{Caller, Scope};
use crate::router::{ApiState, require};

/// The `/v1/world/<table>` names the contract reserves, with the `docs/schema.md` table
/// each maps to.
pub const RESOURCES: &[&str] = &[
    "rooms", "monsters", "items", "shops", "spells", "classes", "races", "texts",
];

/// The `docs/schema.md` table behind a `/v1/world/<resource>` name.
pub fn schema_table(resource: &str) -> Option<&'static str> {
    Some(match resource {
        "rooms" => "rooms",
        "monsters" => "monsters",
        "items" => "items",
        "shops" => "shops",
        "spells" => "spells",
        "classes" => "classes",
        "races" => "races",
        "texts" => "text_blocks",
        _ => return None,
    })
}

/// The `501` body.
pub fn reserved_response() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "reserved",
            "message": "world editing is not built yet",
            "docs": "docs/admin-api.md#reserved-routes",
        })),
    )
        .into_response()
}

async fn reserved(Extension(caller): Extension<Caller>) -> Response {
    match require(&caller, Scope::Admin) {
        Ok(()) => reserved_response(),
        Err(e) => e.into_response(),
    }
}

/// Registers the collection and item routes of every reserved table.
pub fn register(mut router: Router<ApiState>) -> Router<ApiState> {
    for table in RESOURCES {
        router = router
            .route(&format!("/v1/world/{table}"), get(reserved).post(reserved))
            .route(
                &format!("/v1/world/{table}/{{id}}"),
                get(reserved).put(reserved).delete(reserved),
            );
    }
    router
}
