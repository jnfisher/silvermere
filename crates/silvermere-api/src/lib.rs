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
pub mod backup;
pub mod broadcast;
pub mod events;
pub mod health;
pub mod metrics;
pub mod moderation;
pub mod reload;
pub mod reserved;
pub mod router;
pub mod shutdown;
pub mod who;

pub use auth::{Scope, Token};
pub use backup::{BackupProbe, NoBackups};
pub use events::{EventKind, EventLog, EventRecord, NewEvent};
pub use health::HealthProbe;
pub use moderation::{Action, ModError, Moderator, NoModeration};
pub use reload::WorldLoader;
pub use router::{ACTING_FOR, ApiState, RouteSpec, router, routes, serve};
pub use shutdown::{NoShutdown, Planned, ShutdownPlan};

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

/// Why the admin API refuses to start.
#[derive(Debug, thiserror::Error)]
pub enum BindError {
    /// `server.admin_bind` is not `host:port`.
    #[error("server.admin_bind `{0}` is not host:port")]
    Invalid(String),
    /// A non-loopback bind without `admin.allow_remote = true`.
    #[error(
        "server.admin_bind is {0}, which other computers can reach. The admin API is meant for \
         this computer: anyone who can reach it can try tokens, and an admin token can delete \
         accounts and shut the server down. To allow it anyway, set admin.allow_remote = true. \
         See docs/admin-api.md#remote"
    )]
    NotAllowedRemote(String),
    /// A non-loopback bind with no token configured would expose an open admin API.
    #[error(
        "server.admin_bind `{0}` is reachable from other hosts but no [[admin.tokens]] is configured"
    )]
    OpenToTheNetwork(String),
}

/// The warning logged at every start, and shown by the web console, while the admin API
/// listens on an address other computers reach (`admin.allow_remote = true`).
pub fn remote_warning(bind: &str) -> String {
    format!(
        "The admin API is reachable from other computers ({bind}, admin.allow_remote = true). \
         It is plain HTTP: tokens and everything sent to it can be read on the network. Keep it \
         behind an SSH tunnel, a private network such as Tailscale, or a reverse proxy with TLS. \
         In Docker this is expected: it is safe while the port is published on 127.0.0.1 only."
    )
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

/// The startup check: binding anything but a loopback address refuses to start unless
/// `allow_remote` is set and at least one token is configured.
pub fn check_bind(bind: &str, token_count: usize, allow_remote: bool) -> Result<(), BindError> {
    if is_loopback_bind(bind)? {
        Ok(())
    } else if !allow_remote {
        Err(BindError::NotAllowedRemote(bind.to_string()))
    } else if token_count == 0 {
        Err(BindError::OpenToTheNetwork(bind.to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_needs_no_token() {
        assert!(check_bind("127.0.0.1:8080", 0, false).is_ok());
        assert!(check_bind("[::1]:8080", 0, false).is_ok());
    }

    #[test]
    fn open_bind_needs_allow_remote_and_a_token() {
        assert!(matches!(
            check_bind("0.0.0.0:8080", 1, false),
            Err(BindError::NotAllowedRemote(_))
        ));
        assert!(matches!(
            check_bind("0.0.0.0:8080", 0, true),
            Err(BindError::OpenToTheNetwork(_))
        ));
        assert!(check_bind("0.0.0.0:8080", 1, true).is_ok());
        assert!(matches!(
            check_bind("nonsense", 1, true),
            Err(BindError::Invalid(_))
        ));
    }

    #[test]
    fn the_refusal_names_the_setting() {
        let e = check_bind("0.0.0.0:8080", 1, false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("admin.allow_remote = true"), "{e}");
        assert!(e.contains("0.0.0.0:8080"), "{e}");
    }
}
