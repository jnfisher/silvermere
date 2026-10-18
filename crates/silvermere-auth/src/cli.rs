//! The `silvermere account` subcommands as library functions (`silvermere account create
//! NAME --sysop`). The binary maps its arguments onto these;
//! T-055's in-game sysop commands call the same functions.
//!
//! Each is a plain `async fn` over an [`AccountStore`] (on a server, the [`Db`] pool)
//! and writes an audit row. Role changes take effect at the account's next login, as
//! the original decided a player's sysop standing once on entering the game.
//!
//! [`Db`]: silvermere_db::Db

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use silvermere_db::{DbError, Event};
use silvermere_types::ids::AccountId;

use crate::AuthError;
use crate::account::{
    Account, AccountFlags, AccountStore, NewAccount, Sex, validate_password, validate_username,
};
use crate::password::{self, Cost};
use crate::roles::{Perm, Role, RoleSet, UnknownName};

/// What `grant`/`revoke` change: a role, a permission, or a "take away" flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// `player`, `sysop`, `gameop`.
    Role(Role),
    /// A `SYS_*` INI key, `GOSSIP`, `AUCTION` or `LOAN` held explicitly.
    Perm(Perm),
    /// `no_gossip`, `no_auction`, `no_loan`.
    Flag(AccountFlags),
}

impl FromStr for Grant {
    type Err = UnknownName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(role) = s.parse::<Role>() {
            Ok(Self::Role(role))
        } else if let Some(perm) = Perm::from_ini_name(s) {
            Ok(Self::Perm(perm))
        } else if let Some(flag) = AccountFlags::from_name(s) {
            Ok(Self::Flag(flag))
        } else {
            Err(UnknownName {
                what: "role, permission or flag",
                name: s.to_string(),
            })
        }
    }
}

impl fmt::Display for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Role(r) => write!(f, "{r}"),
            Self::Perm(p) => write!(f, "{p}"),
            Self::Flag(flag) => {
                let name = AccountFlags::NAMED
                    .iter()
                    .find(|(_, v)| v == flag)
                    .map_or("flag", |(n, _)| n);
                f.write_str(name)
            }
        }
    }
}

async fn audit<S: AccountStore>(
    store: &S,
    kind: &str,
    payload: serde_json::Value,
) -> Result<(), AuthError> {
    tracing::info!(target: "audit", kind, %payload, "account change");
    Ok(store
        .audit(Event::new(Timestamp::now(), kind, payload))
        .await?)
}

async fn find<S: AccountStore>(store: &S, username: &str) -> Result<Account, AuthError> {
    store
        .by_username(username)
        .await?
        .ok_or_else(|| AuthError::NoSuchAccount(username.to_string()))
}

/// Creates an account with `roles` (plus `player`) and sex `M`; the hash has the default
/// cost.
pub async fn create<S: AccountStore>(
    store: &S,
    username: &str,
    password: &str,
    roles: &[Role],
) -> Result<Account, AuthError> {
    create_with_cost(store, username, password, roles, Cost::Default).await
}

/// [`create`] with an explicit hash cost (tests).
pub async fn create_with_cost<S: AccountStore>(
    store: &S,
    username: &str,
    password: &str,
    roles: &[Role],
    cost: Cost,
) -> Result<Account, AuthError> {
    validate_username(username)?;
    validate_password(password)?;
    let new = NewAccount {
        username: username.to_string(),
        password_hash: password::hash_with(password, cost),
        email: None,
        sex: Sex::Male,
        roles: roles.iter().copied().collect::<RoleSet>(),
    };
    let account = match store.insert(&new, Timestamp::now()).await {
        Ok(a) => a,
        Err(DbError::Duplicate(_)) => return Err(AuthError::UsernameTaken(username.to_string())),
        Err(e) => return Err(e.into()),
    };
    let roles: Vec<&str> = account.roles.iter().map(Role::name).collect();
    audit(
        store,
        "auth.account_created",
        serde_json::json!({ "account": account.id.0, "username": account.username, "roles": roles }),
    )
    .await?;
    Ok(account)
}

/// Sets a new password (default cost).
pub async fn set_password<S: AccountStore>(
    store: &S,
    username: &str,
    password: &str,
) -> Result<(), AuthError> {
    set_password_with_cost(store, username, password, Cost::Default).await
}

/// [`set_password`] with an explicit hash cost (tests).
pub async fn set_password_with_cost<S: AccountStore>(
    store: &S,
    username: &str,
    password: &str,
    cost: Cost,
) -> Result<(), AuthError> {
    validate_password(password)?;
    let mut account = find(store, username).await?;
    account.password_hash = password::hash_with(password, cost);
    store.update(&account).await?;
    audit(
        store,
        "auth.password",
        serde_json::json!({ "account": account.id.0, "username": account.username }),
    )
    .await
}

/// Grants a role, permission or flag; returns the account as stored.
pub async fn grant<S: AccountStore>(
    store: &S,
    username: &str,
    what: Grant,
) -> Result<Account, AuthError> {
    change(store, username, what, true).await
}

/// Revokes a role, permission or flag; returns the account as stored.
pub async fn revoke<S: AccountStore>(
    store: &S,
    username: &str,
    what: Grant,
) -> Result<Account, AuthError> {
    change(store, username, what, false).await
}

/// Grants `role`.
pub async fn grant_role<S: AccountStore>(
    store: &S,
    username: &str,
    role: Role,
) -> Result<Account, AuthError> {
    grant(store, username, Grant::Role(role)).await
}

/// Revokes `role`.
pub async fn revoke_role<S: AccountStore>(
    store: &S,
    username: &str,
    role: Role,
) -> Result<Account, AuthError> {
    revoke(store, username, Grant::Role(role)).await
}

async fn change<S: AccountStore>(
    store: &S,
    username: &str,
    what: Grant,
    on: bool,
) -> Result<Account, AuthError> {
    let mut account = find(store, username).await?;
    match (what, on) {
        (Grant::Role(r), true) => account.roles.insert(r),
        (Grant::Role(r), false) => account.roles.remove(r),
        (Grant::Perm(p), true) => account.perms.insert(p),
        (Grant::Perm(p), false) => account.perms.remove(p),
        (Grant::Flag(f), true) => account.flags.insert(f),
        (Grant::Flag(f), false) => account.flags.remove(f),
    }
    store.update(&account).await?;
    audit(
        store,
        "auth.role",
        serde_json::json!({
            "account": account.id.0,
            "username": account.username,
            "change": if on { "grant" } else { "revoke" },
            "what": what.to_string(),
        }),
    )
    .await?;
    Ok(account)
}

/// Every account, by id.
pub async fn list<S: AccountStore>(store: &S) -> Result<Vec<Account>, AuthError> {
    Ok(store.all().await?)
}

/// Deletes an account: first its character (the hook T-023 owns; the game deleted
/// the character too when a BBS account went), then the account and its `account_roles`
/// rows. `false` when there was no such account.
pub async fn delete<S: AccountStore>(store: &S, username: &str) -> Result<bool, AuthError> {
    let Some(account) = store.by_username(username).await? else {
        return Ok(false);
    };
    delete_rows(store, &account).await
}

/// [`delete`] by id: what the running server's store does once the engine has let go of
/// the account (`DELETE /v1/accounts/{id}`, T-069).
pub async fn delete_account<S: AccountStore>(store: &S, id: AccountId) -> Result<bool, AuthError> {
    let Some(account) = store.get(id).await? else {
        return Ok(false);
    };
    delete_rows(store, &account).await
}

async fn delete_rows<S: AccountStore>(store: &S, account: &Account) -> Result<bool, AuthError> {
    store.delete_characters(account.id).await?;
    let deleted = store.delete(account.id).await?;
    if deleted {
        audit(
            store,
            "auth.account_deleted",
            serde_json::json!({ "account": account.id.0, "username": account.username }),
        )
        .await?;
    }
    Ok(deleted)
}

/// One line per account for `silvermere account list`: id, name, roles, permissions
/// and flags.
pub fn describe(account: &Account) -> String {
    let mut parts: Vec<String> = account.roles.iter().map(|r| r.name().to_string()).collect();
    parts.extend(account.perms.iter().map(|p| p.ini_name().to_string()));
    parts.extend(
        AccountFlags::NAMED
            .iter()
            .filter(|(_, f)| account.flags.contains(*f))
            .map(|(n, _)| n.to_string()),
    );
    format!(
        "{:>6}  {:<29}  {}  {}",
        account.id.0,
        account.username,
        char::from(account.sex.as_byte()),
        parts.join(",")
    )
}
