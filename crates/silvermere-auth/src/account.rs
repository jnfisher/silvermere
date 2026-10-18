//! Accounts: what Worldgroup's `usracc` record and its key list were to the module.
//!
//! An [`Account`] is loaded once at login and read from then on, as the original decided a
//! player's sysop standing once on entering the game: a role change takes effect at the
//! next login. `username` is immutable: `characters.userid` keeps a copy as the join key
//! the game looks players up by (the BBS's 29-character user-ID).
//!
//! Storage (T-003's `accounts` and `account_roles`, no further migration):
//!
//! - `accounts.flags` holds [`AccountFlags`] plus bit 0 for a female account.
//! - `account_roles.role` holds the role names (`player`, `sysop`, `gameop`) and the
//!   explicitly granted permissions by INI name (`SYS_GOTO_KEY`, ...).
//! - `failed_logins` and `locked_until` are not stored: they live in the server's
//!   [`Lockout`](crate::lockout::Lockout) and are filled in when an account is loaded
//!   through the login service.

use std::future::Future;
use std::sync::Mutex;

use jiff::Timestamp;
use silvermere_db::{Db, DbError, Event, repo::accounts as db_accounts};
use silvermere_types::ids::AccountId;

use crate::roles::{Perm, PermSet, Role, RoleSet};

/// The longest username: a BBS user-ID held 29 usable characters
/// (MBBSEmu's signup limit).
pub const USERNAME_MAX: usize = 29;

/// The longest password accepted (a sanity bound; there is no 5-9 rule).
pub const PASSWORD_MAX: usize = 128;

/// `usracc.sex`: `'M'` or `'F'`. The game copies it into `player.sex` at character
/// creation and picks pronouns from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Sex {
    /// `'M'`.
    #[default]
    Male,
    /// `'F'`.
    Female,
}

impl Sex {
    /// The byte the BBS account record stored.
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Male => b'M',
            Self::Female => b'F',
        }
    }

    /// `M` or `F`, either case.
    pub fn from_answer(answer: &str) -> Option<Self> {
        match answer {
            "M" | "m" => Some(Self::Male),
            "F" | "f" => Some(Self::Female),
            _ => None,
        }
    }
}

/// Account flags. Bit 0 of the stored column is the sex (see the module doc), so these
/// start at bit 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AccountFlags(u32);

impl AccountFlags {
    /// The stored bit for `Sex::Female`; never set in an [`Account`]'s `flags`.
    const FEMALE: u32 = 1;

    /// No flags.
    pub const NONE: Self = Self(0);
    /// Gossip taken away (the original's `GOSSKEY`).
    pub const NO_GOSSIP: Self = Self(1 << 1);
    /// Auctions taken away (the original's `AUCTKEY`).
    pub const NO_AUCTION: Self = Self(1 << 2);
    /// Bank loans withheld (the original required `LOANKEY`).
    pub const NO_LOAN: Self = Self(1 << 3);

    /// The named flags, as the CLI and `docs/auth.md` spell them.
    pub const NAMED: [(&'static str, Self); 3] = [
        ("no_gossip", Self::NO_GOSSIP),
        ("no_auction", Self::NO_AUCTION),
        ("no_loan", Self::NO_LOAN),
    ];

    /// The flag named `name` (`no_gossip`, ...), ignoring case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::NAMED
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, f)| *f)
    }

    /// The raw bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every bit of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Sets `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Clears `other`.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

/// A login account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The primary key.
    pub id: AccountId,
    /// The login name: unique ignoring case, stored with the case given, never renamed.
    pub username: String,
    /// argon2id PHC string.
    pub password_hash: String,
    /// Optional, never validated strictly; the game never reads it.
    pub email: Option<String>,
    /// `usracc.sex`.
    pub sex: Sex,
    /// Roles held.
    pub roles: RoleSet,
    /// Permissions granted explicitly.
    pub perms: PermSet,
    /// Flags.
    pub flags: AccountFlags,
    /// When the account was created.
    pub created_at: Timestamp,
    /// The last successful login.
    pub last_login_at: Option<Timestamp>,
    /// Failed logins inside the lockout window (from the lockout tracker).
    pub failed_logins: u32,
    /// Locked until then, if locked (from the lockout tracker).
    pub locked_until: Option<Timestamp>,
}

/// What signup and `silvermere account create` supply for a new account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    /// The login name.
    pub username: String,
    /// argon2id PHC string.
    pub password_hash: String,
    /// Optional address.
    pub email: Option<String>,
    /// `M`/`F`.
    pub sex: Sex,
    /// Roles; `player` is always added.
    pub roles: RoleSet,
}

/// Why a username is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UsernameError {
    /// Blank.
    #[error("the username is empty")]
    Empty,
    /// More than [`USERNAME_MAX`] characters.
    #[error("the username is longer than {USERNAME_MAX} characters")]
    TooLong,
    /// A character outside printable ASCII 32-126 (the BBS's user-ID characters).
    #[error("the username may hold only printable ASCII characters")]
    BadChar,
    /// Starts or ends with a space.
    #[error("the username may not start or end with a space")]
    EdgeSpace,
    /// `NEW`, which starts signup at the username prompt.
    #[error("NEW is reserved")]
    Reserved,
}

/// Checks a new username: 1-29 printable ASCII characters (32-126), no leading or
/// trailing space, not `NEW`.
pub fn validate_username(name: &str) -> Result<(), UsernameError> {
    if name.is_empty() {
        return Err(UsernameError::Empty);
    }
    if !name.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(UsernameError::BadChar);
    }
    if name.len() > USERNAME_MAX {
        return Err(UsernameError::TooLong);
    }
    if name.starts_with(' ') || name.ends_with(' ') {
        return Err(UsernameError::EdgeSpace);
    }
    if is_new(name) {
        return Err(UsernameError::Reserved);
    }
    Ok(())
}

/// Whether `line` is `NEW` in any case, which starts signup.
pub fn is_new(line: &str) -> bool {
    line.eq_ignore_ascii_case("NEW")
}

/// Why a password is refused at signup or by `set_password`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PasswordError {
    /// Empty or longer than [`PASSWORD_MAX`].
    #[error("the password must be 1 to {PASSWORD_MAX} characters long")]
    Length,
    /// `password` in any case (MBBSEmu's one refusal, kept).
    #[error("the password may not be `password`")]
    Weak,
}

/// Checks a new password: 1-128 bytes, not only whitespace (the login dialogue re-asks on
/// such a line, so it could never be typed) and not `password`.
pub fn validate_password(plain: &str) -> Result<(), PasswordError> {
    if plain.trim().is_empty() || plain.len() > PASSWORD_MAX {
        return Err(PasswordError::Length);
    }
    if plain.eq_ignore_ascii_case("password") {
        return Err(PasswordError::Weak);
    }
    Ok(())
}

/// The loose e-mail check: blank, or no whitespace and one `@` with text either side.
pub fn email_acceptable(email: &str) -> bool {
    if email.is_empty() {
        return true;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !email.chars().any(char::is_whitespace)
}

/// Errors of the account store.
pub type StoreResult<T> = Result<T, DbError>;

/// Where accounts live. [`Db`] is the server's store; [`MemoryStore`] serves tests and
/// tools that need no database. Inserting a username that exists in any case is
/// `DbError::Duplicate`.
pub trait AccountStore: Send + Sync {
    /// The account named `username`, ignoring case.
    fn by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = StoreResult<Option<Account>>> + Send;
    /// The account `id`.
    fn get(&self, id: AccountId) -> impl Future<Output = StoreResult<Option<Account>>> + Send;
    /// Every account, by id.
    fn all(&self) -> impl Future<Output = StoreResult<Vec<Account>>> + Send;
    /// Stores a new account created at `now`.
    fn insert(
        &self,
        new: &NewAccount,
        now: Timestamp,
    ) -> impl Future<Output = StoreResult<Account>> + Send;
    /// Rewrites an account (everything but `id`, `failed_logins`, `locked_until`).
    fn update(&self, account: &Account) -> impl Future<Output = StoreResult<bool>> + Send;
    /// Deletes an account and its role rows; `false` if there was none.
    fn delete(&self, id: AccountId) -> impl Future<Output = StoreResult<bool>> + Send;
    /// Removes what the game keeps for the account (its character): the hook `cli::delete`
    /// calls first. T-023 owns the character side.
    fn delete_characters(&self, id: AccountId) -> impl Future<Output = StoreResult<()>> + Send;
    /// Appends an audit row to `events`.
    fn audit(&self, event: Event) -> impl Future<Output = StoreResult<()>> + Send;
}

fn to_row(account: &Account) -> db_accounts::Account {
    let mut roles: Vec<String> = account.roles.iter().map(|r| r.name().to_string()).collect();
    roles.extend(account.perms.iter().map(|p| p.ini_name().to_string()));
    let sex_bit = match account.sex {
        Sex::Male => 0,
        Sex::Female => AccountFlags::FEMALE,
    };
    db_accounts::Account {
        username: account.username.clone(),
        password_hash: account.password_hash.clone(),
        email: account.email.clone(),
        created: account.created_at,
        last_login: account.last_login_at,
        flags: (account.flags.bits() & !AccountFlags::FEMALE) | sex_bit,
        roles,
    }
}

fn from_row(row: db_accounts::AccountRow) -> Account {
    let a = row.account;
    let mut roles = RoleSet::empty();
    let mut perms = PermSet::empty();
    for name in &a.roles {
        if let Ok(role) = name.parse::<Role>() {
            roles.insert(role);
        } else if let Some(perm) = Perm::from_ini_name(name) {
            perms.insert(perm);
        } else {
            tracing::warn!(account = row.id.0, role = %name, "unknown account_roles row ignored");
        }
    }
    Account {
        id: row.id,
        username: a.username,
        password_hash: a.password_hash,
        email: a.email,
        sex: if a.flags & AccountFlags::FEMALE != 0 {
            Sex::Female
        } else {
            Sex::Male
        },
        roles,
        perms,
        flags: AccountFlags(a.flags & !AccountFlags::FEMALE),
        created_at: a.created,
        last_login_at: a.last_login,
        failed_logins: 0,
        locked_until: None,
    }
}

fn new_account(id: AccountId, new: &NewAccount, now: Timestamp) -> Account {
    let mut roles = new.roles;
    roles.insert(Role::Player);
    Account {
        id,
        username: new.username.clone(),
        password_hash: new.password_hash.clone(),
        email: new.email.clone().filter(|e| !e.is_empty()),
        sex: new.sex,
        roles,
        perms: PermSet::empty(),
        flags: AccountFlags::NONE,
        created_at: now,
        last_login_at: None,
        failed_logins: 0,
        locked_until: None,
    }
}

impl AccountStore for Db {
    async fn by_username(&self, username: &str) -> StoreResult<Option<Account>> {
        Ok(self.accounts().by_username(username).await?.map(from_row))
    }

    async fn get(&self, id: AccountId) -> StoreResult<Option<Account>> {
        Ok(self.accounts().get(id).await?.map(from_row))
    }

    async fn all(&self) -> StoreResult<Vec<Account>> {
        Ok(self
            .accounts()
            .all()
            .await?
            .into_iter()
            .map(from_row)
            .collect())
    }

    async fn insert(&self, new: &NewAccount, now: Timestamp) -> StoreResult<Account> {
        let mut account = new_account(AccountId(0), new, now);
        account.id = self.accounts().insert(&to_row(&account)).await?;
        Ok(account)
    }

    async fn update(&self, account: &Account) -> StoreResult<bool> {
        self.accounts().update(account.id, &to_row(account)).await
    }

    async fn delete(&self, id: AccountId) -> StoreResult<bool> {
        self.accounts().delete(id).await
    }

    async fn delete_characters(&self, id: AccountId) -> StoreResult<()> {
        if let Some(row) = self.characters().by_account(id).await? {
            self.characters().delete(row.id).await?;
        }
        Ok(())
    }

    async fn audit(&self, event: Event) -> StoreResult<()> {
        self.events().append(&event).await.map(|_| ())
    }
}

impl<S: AccountStore> AccountStore for std::sync::Arc<S> {
    fn by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = StoreResult<Option<Account>>> + Send {
        (**self).by_username(username)
    }

    fn get(&self, id: AccountId) -> impl Future<Output = StoreResult<Option<Account>>> + Send {
        (**self).get(id)
    }

    fn all(&self) -> impl Future<Output = StoreResult<Vec<Account>>> + Send {
        (**self).all()
    }

    fn insert(
        &self,
        new: &NewAccount,
        now: Timestamp,
    ) -> impl Future<Output = StoreResult<Account>> + Send {
        (**self).insert(new, now)
    }

    fn update(&self, account: &Account) -> impl Future<Output = StoreResult<bool>> + Send {
        (**self).update(account)
    }

    fn delete(&self, id: AccountId) -> impl Future<Output = StoreResult<bool>> + Send {
        (**self).delete(id)
    }

    fn delete_characters(&self, id: AccountId) -> impl Future<Output = StoreResult<()>> + Send {
        (**self).delete_characters(id)
    }

    fn audit(&self, event: Event) -> impl Future<Output = StoreResult<()>> + Send {
        (**self).audit(event)
    }
}

/// An in-memory [`AccountStore`] for tests and database-free tools.
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: Mutex<MemoryInner>,
}

#[derive(Debug, Default)]
struct MemoryInner {
    accounts: Vec<Account>,
    next_id: i64,
    events: Vec<Event>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The audit events appended so far.
    pub fn events(&self) -> Vec<Event> {
        self.lock().events.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl AccountStore for MemoryStore {
    async fn by_username(&self, username: &str) -> StoreResult<Option<Account>> {
        Ok(self
            .lock()
            .accounts
            .iter()
            .find(|a| a.username.eq_ignore_ascii_case(username))
            .cloned())
    }

    async fn get(&self, id: AccountId) -> StoreResult<Option<Account>> {
        Ok(self.lock().accounts.iter().find(|a| a.id == id).cloned())
    }

    async fn all(&self) -> StoreResult<Vec<Account>> {
        Ok(self.lock().accounts.clone())
    }

    async fn insert(&self, new: &NewAccount, now: Timestamp) -> StoreResult<Account> {
        let mut inner = self.lock();
        if inner
            .accounts
            .iter()
            .any(|a| a.username.eq_ignore_ascii_case(&new.username))
        {
            return Err(DbError::Duplicate(format!(
                "accounts.username {}",
                new.username
            )));
        }
        inner.next_id += 1;
        let account = new_account(AccountId(inner.next_id), new, now);
        inner.accounts.push(account.clone());
        Ok(account)
    }

    async fn update(&self, account: &Account) -> StoreResult<bool> {
        let mut inner = self.lock();
        match inner.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(slot) => {
                *slot = Account {
                    failed_logins: 0,
                    locked_until: None,
                    ..account.clone()
                };
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete(&self, id: AccountId) -> StoreResult<bool> {
        let mut inner = self.lock();
        let before = inner.accounts.len();
        inner.accounts.retain(|a| a.id != id);
        Ok(inner.accounts.len() < before)
    }

    async fn delete_characters(&self, _id: AccountId) -> StoreResult<()> {
        Ok(())
    }

    async fn audit(&self, event: Event) -> StoreResult<()> {
        self.lock().events.push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_only_passwords_are_refused() {
        for p in ["", " ", "   ", "\t ", " \t \t"] {
            assert_eq!(validate_password(p), Err(PasswordError::Length), "{p:?}");
        }
        assert_eq!(validate_password(" a "), Ok(()));
    }

    #[test]
    fn usernames() {
        assert_eq!(validate_username("Kasimir"), Ok(()));
        assert_eq!(validate_username("a b"), Ok(()));
        assert_eq!(validate_username(&"x".repeat(29)), Ok(()));
        assert_eq!(
            validate_username(&"x".repeat(30)),
            Err(UsernameError::TooLong)
        );
        assert_eq!(validate_username(""), Err(UsernameError::Empty));
        assert_eq!(validate_username(" bob"), Err(UsernameError::EdgeSpace));
        assert_eq!(validate_username("b\u{e9}b"), Err(UsernameError::BadChar));
        assert_eq!(validate_username("nEw"), Err(UsernameError::Reserved));
    }

    #[test]
    fn emails_are_checked_loosely() {
        assert!(email_acceptable(""));
        assert!(email_acceptable("aldric@example.com"));
        assert!(email_acceptable("a@b"));
        assert!(!email_acceptable("nope"));
        assert!(!email_acceptable("a @b"));
        assert!(!email_acceptable("@b"));
    }

    #[test]
    fn sex_survives_the_flags_column() {
        let mut a = new_account(
            AccountId(1),
            &NewAccount {
                username: "x".into(),
                password_hash: String::new(),
                email: None,
                sex: Sex::Female,
                roles: RoleSet::empty(),
            },
            Timestamp::UNIX_EPOCH,
        );
        a.flags.insert(AccountFlags::NO_GOSSIP);
        a.perms.insert(Perm::SysGoto);
        let row = to_row(&a);
        assert_eq!(row.flags, 0b11);
        assert_eq!(row.roles, vec!["player", "SYS_GOTO_KEY"]);
        let back = from_row(db_accounts::AccountRow {
            id: AccountId(1),
            account: row,
        });
        assert_eq!(back, a);
    }
}
