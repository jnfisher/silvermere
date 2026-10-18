//! Admin HTTP API (health, metrics, event stream; CRUD reserved).
//!
//! A token-authenticated HTTP and JSON admin API served with axum on the admin port: health,
//! Prometheus metrics, who, broadcast, world reload and a server-sent event stream. The
//! world CRUD routes are reserved for the editor project. `docs/admin-api.md` is the
//! contract.
//!
//! The API task never touches the `World`: it sends [`AdminRequest`]s to the engine thread
//! and waits for the reply. The crate depends on the engine and the
//! types only; what lives in `silvermere-db` (the `events` table, the persistence worker's
//! status) is reached through the [`EventLog`] and [`HealthProbe`] traits that `bin`
//! implements.

#![forbid(unsafe_code)]

pub mod accounts;
pub mod auth;
pub mod broadcast;
pub mod events;
pub mod health;
pub mod metrics;
pub mod reload;
pub mod reserved;
pub mod router;
pub mod who;

pub use auth::{Scope, Token};
pub use events::{EventKind, EventLog, EventRecord, NewEvent};
pub use health::HealthProbe;
pub use reload::WorldLoader;
pub use router::{ApiState, RouteSpec, router, routes, serve};

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

/// Why the admin API refuses to start.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    /// `server.admin_bind` is not `host:port`.
    #[error("server.admin_bind `{0}` is not host:port")]
    Invalid(String),
    /// A non-loopback bind with no token configured would expose an open admin API.
    #[error(
        "server.admin_bind `{0}` is reachable from other hosts but no [[admin.tokens]] is configured"
    )]
    OpenToTheNetwork(String),
}

/// Whether `bind` resolves only to loopback addresses.
pub fn is_loopback_bind(bind: &str) -> Result<bool, BindError> {
    let addrs: Vec<SocketAddr> = bind
        .to_socket_addrs()
        .map_err(|_| BindError::Invalid(bind.to_string()))?
        .collect();
    if addrs.is_empty() {
        return Err(BindError::Invalid(bind.to_string()));
    }
    Ok(addrs.iter().all(|a| match a.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }))
}

/// The startup check: binding anything but a loopback
/// address without at least one token refuses to start.
pub fn check_bind(bind: &str, token_count: usize) -> Result<(), BindError> {
    if is_loopback_bind(bind)? || token_count > 0 {
        Ok(())
    } else {
        Err(BindError::OpenToTheNetwork(bind.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_needs_no_token() {
        assert!(check_bind("127.0.0.1:8080", 0).is_ok());
        assert!(check_bind("[::1]:8080", 0).is_ok());
    }

    #[test]
    fn open_bind_needs_a_token() {
        assert!(matches!(
            check_bind("0.0.0.0:8080", 0),
            Err(BindError::OpenToTheNetwork(_))
        ));
        assert!(check_bind("0.0.0.0:8080", 1).is_ok());
        assert!(matches!(
            check_bind("nonsense", 1),
            Err(BindError::Invalid(_))
        ));
    }
}
