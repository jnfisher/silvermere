//! Accounts, passwords, roles, login state machine.
//!
//! Replaces the Worldgroup login: accounts with argon2id password hashes, roles and
//! permission sets, lockout after repeated failures, and the login and signup dialogue whose
//! prompt texts legacy clients script against.
//!
//! - [`account`]: [`Account`], [`Sex`], the username rules and the [`AccountStore`]
//!   trait, implemented for `silvermere_db::Db` and by [`MemoryStore`].
//! - [`password`]: argon2id [`hash`](password::hash) and [`verify`](password::verify).
//! - [`roles`]: [`Role`], [`Perm`] and [`has_permission`], where the module called
//!   `haskey(lock)`.
//! - [`login`] and [`signup`]: the pure [`LoginDialogue`] and the [`LoginService`] that
//!   answers its requests.
//! - [`art`]: the login art sent before the first prompt, and its MegaMud-safe rules.
//! - [`lockout`]: failures per account and the lock they lead to.
//! - [`cli`]: what `silvermere account ...` calls.
//!
//! The dialogue, its texts and the tables are described in `docs/auth.md`.

#![forbid(unsafe_code)]

pub mod account;
pub mod art;
pub mod cli;
pub mod lockout;
pub mod login;
pub mod password;
pub mod roles;
pub mod signup;

pub use account::{
    Account, AccountFlags, AccountStore, MemoryStore, NewAccount, PasswordError, Sex, UsernameError,
};
pub use lockout::{Lockout, LockoutConfig};
pub use login::{
    AuthMsg, Clock, CloseReason, EchoMode, LoginConfig, LoginDialogue, LoginService, Output, State,
    SystemClock,
};
pub use password::Secret;
pub use roles::{Perm, PermSet, Role, RoleSet, has_permission, has_role};

/// The errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The username is refused.
    #[error(transparent)]
    Username(#[from] UsernameError),
    /// The password is refused.
    #[error(transparent)]
    Password(#[from] PasswordError),
    /// The username exists already (in any case).
    #[error("an account named {0} exists already")]
    UsernameTaken(String),
    /// No account has this name.
    #[error("no account named {0}")]
    NoSuchAccount(String),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] silvermere_db::DbError),
}
