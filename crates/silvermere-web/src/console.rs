//! The sysop's console (`/console`): the players, the moderation, the server's state and
//! a command line, live in the browser.
//!
//! The page itself is static ([`crate::page::console`] and `console.js`); everything it
//! shows or does goes through the admin API, which the binary runs in the same process
//! for the signed-in sysop ([`Console`]): the same routes, checks and audit rows as
//! `silvermere sysop`, the rows naming `web:<sysop>`. The browser never holds an API token.
//!
//! Every call needs the session; one that changes something carries the session's CSRF
//! token in `X-CSRF-Token`; one that cannot be undone ([`needs_password`]) wants the
//! password given in the last ten minutes, else it answers `401 {"error":"password"}` and
//! the page asks for it (`POST /console/password`).

use std::future::Future;
use std::pin::Pin;

use axum::body::Bytes;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};

/// A boxed future, for the trait objects.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a command line came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineOutcome {
    /// The answer, as `silvermere sysop` prints it.
    Text(String),
    /// Why it was refused or failed.
    Error(String),
    /// It cannot be undone and the password was not given lately.
    NeedsPassword,
}

/// The admin API on behalf of the signed-in sysop; `bin` runs its router in-process.
pub trait Console: Send + Sync + 'static {
    /// One API request (`path` from `/v1/`, with any query) as `sysop`: its response as is,
    /// JSON or the event stream.
    fn call(
        &self,
        sysop: &str,
        method: Method,
        path: &str,
        body: Bytes,
        last_event_id: Option<String>,
    ) -> BoxFuture<'_, Response>;

    /// One command line, as `silvermere sysop` takes it (`kick Lars spamming`). `fresh`:
    /// the password was given lately, so actions that cannot be undone may run.
    fn line(&self, sysop: &str, line: &str, fresh: bool) -> BoxFuture<'_, LineOutcome>;
}

/// Whether an API call cannot be undone and so wants the password given lately: a ban,
/// an IP ban, deleting an account, closing the realm, a planned shutdown or restart, a line
/// typed for a player and the game's own SYSOP commands.
pub fn needs_password(method: &Method, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    match *method {
        Method::POST => {
            matches!(
                path,
                "/v1/shutdown" | "/v1/restart" | "/v1/realm/close" | "/v1/ip-bans" | "/v1/sysop"
            ) || (path.starts_with("/v1/players/") && path.ends_with("/input"))
        }
        Method::PUT => path.starts_with("/v1/players/") && path.ends_with("/ban"),
        Method::DELETE => path.starts_with("/v1/accounts/"),
        _ => false,
    }
}

/// `401 {"error":"password", ...}`: give the password again.
pub fn password_wanted() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        r#"{"error":"password","message":"This cannot be undone: give your password again."}"#,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_cannot_be_undone_wants_the_password() {
        for (m, p) in [
            (Method::POST, "/v1/shutdown"),
            (Method::POST, "/v1/restart"),
            (Method::POST, "/v1/realm/close"),
            (Method::POST, "/v1/ip-bans"),
            (Method::PUT, "/v1/players/Lars/ban"),
            (Method::DELETE, "/v1/accounts/7"),
            (Method::POST, "/v1/sysop"),
            (Method::POST, "/v1/players/Lars/input"),
        ] {
            assert!(needs_password(&m, p), "{m} {p}");
        }
        for (m, p) in [
            (Method::GET, "/v1/who"),
            (Method::PUT, "/v1/players/Lars/jail"),
            (Method::DELETE, "/v1/players/Lars/ban"),
            (Method::POST, "/v1/players/Lars/kick"),
            (Method::DELETE, "/v1/shutdown"),
            (Method::POST, "/v1/realm/open"),
            (Method::DELETE, "/v1/ip-bans"),
            (Method::GET, "/v1/players/Lars/screen"),
        ] {
            assert!(!needs_password(&m, p), "{m} {p}");
        }
    }
}
