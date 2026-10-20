//! The login dialogue: `Username:` / `Password:` / `NEW`, as MBBSEmu's `MenuRoutines`
//! drove it and legacy login scripts expect it.
//!
//! [`LoginDialogue`] is a pure state machine: lines in, [`Output`]s out, no I/O and no
//! async. When it needs the account store it emits an [`Output::Request`] and waits for
//! [`LoginDialogue::on_reply`]; [`LoginService::drive`] answers those requests against an
//! [`AccountStore`], applying lockout, the one-session rule and the audit trail. The
//! session task (T-015/T-021) owns the socket, renders [`AuthMsg`] text, applies the echo
//! mode to its line editor and, once the dialogue is [`State::Done`], hands the loaded
//! account to the BBS menu shim. This module ends at "authenticated, account loaded".
//!
//! The flow, from `MenuRoutines.cs`:
//!
//! 1. `welcome_end`, then `login_intro` + `Username:` (LoginUsernameDisplay).
//! 2. A blank or whitespace-only line re-displays (at `Password:` too, without counting as
//!    a failed attempt); more than 29 characters prints `username_too_long`;
//!    `NEW` in any case starts signup ([`crate::signup`]); anything else asks
//!    `Password:` with secure echo, one `*` per byte as MBBSEmu echoed (`wid = 0xFF`).
//! 3. A wrong password or unknown name prints `Invalid Credentials` and goes back to
//!    step 1's intro and prompt. silvermere closes the connection after the third
//!    failure (MBBSEmu let a client retry forever) and refuses a locked account with
//!    `account_locked`.
//!    An account with no password (one an import created) is refused with
//!    `no_password`, which never counts towards the lockout.
//! 4. The right password for an account already in use prints `duplicate_login` and
//!    closes; otherwise the dialogue is [`State::Done`].

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use jiff::Timestamp;
use silvermere_db::{DbError, Event};
use silvermere_text::catalogue::{self, Catalogue, TemplatePiece};
use silvermere_types::ids::AccountId;

use crate::AuthError;
use crate::account::{Account, AccountStore, NewAccount, USERNAME_MAX, is_new};
use crate::lockout::{Lockout, LockoutConfig};
use crate::password::{self, Secret};
use crate::signup::SignupState;

/// The catalogue of this crate's texts, `messages/auth.toml`.
pub const AUTH_TOML: &str = include_str!("../messages/auth.toml");

fn auth_catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| catalogue::parse(AUTH_TOML).expect("messages/auth.toml is valid"))
}

/// A text of the dialogue: one `messages/auth.toml` entry with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMsg {
    /// `compat.login_banner`.
    Banner {
        /// The configured text.
        banner: String,
    },
    /// The end of MBBSEmu's welcome screen.
    WelcomeEnd,
    /// `Enter Username or enter "NEW" to create a new Account`.
    LoginIntro,
    /// `Username: `.
    UsernamePrompt,
    /// `Password: `.
    PasswordPrompt,
    /// More than 29 characters.
    UsernameTooLong,
    /// `Invalid Credentials`.
    InvalidCredentials,
    /// The account is locked.
    AccountLocked {
        /// Whole minutes left, at least 1.
        minutes: u32,
    },
    /// The account already has a session.
    DuplicateLogin,
    /// The account has no password yet.
    NoPassword,
    /// `NEW` while signup is closed.
    SignupClosed,
    /// Signup: username prompt.
    SignupUsernamePrompt,
    /// Signup: invalid username.
    SignupUsernameInvalid,
    /// Signup: username taken.
    SignupUsernameTaken,
    /// Signup: password prompt.
    SignupPasswordPrompt,
    /// Signup: password length.
    SignupPasswordInvalid {
        /// The longest password accepted.
        max: u32,
    },
    /// Signup: `password`.
    SignupPasswordWeak,
    /// Signup: confirmation prompt.
    SignupConfirmPrompt,
    /// Signup: the two passwords differ.
    SignupPasswordMismatch,
    /// Signup: e-mail prompt.
    SignupEmailPrompt,
    /// Signup: e-mail refused.
    SignupEmailInvalid,
    /// Signup: gender prompt.
    SignupGenderPrompt,
    /// Signup: not `M`/`F`.
    SignupGenderInvalid,
}

impl AuthMsg {
    /// Every key of `messages/auth.toml`, in the catalogue's (sorted) order.
    pub const KEYS: &'static [&'static str] = &[
        "account_locked",
        "banner",
        "duplicate_login",
        "invalid_credentials",
        "login_intro",
        "no_password",
        "password_prompt",
        "signup_closed",
        "signup_confirm_prompt",
        "signup_email_invalid",
        "signup_email_prompt",
        "signup_gender_invalid",
        "signup_gender_prompt",
        "signup_password_invalid",
        "signup_password_mismatch",
        "signup_password_prompt",
        "signup_password_weak",
        "signup_username_invalid",
        "signup_username_prompt",
        "signup_username_taken",
        "username_prompt",
        "username_too_long",
        "welcome_end",
    ];

    /// The catalogue key.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Banner { .. } => "banner",
            Self::WelcomeEnd => "welcome_end",
            Self::LoginIntro => "login_intro",
            Self::UsernamePrompt => "username_prompt",
            Self::PasswordPrompt => "password_prompt",
            Self::UsernameTooLong => "username_too_long",
            Self::InvalidCredentials => "invalid_credentials",
            Self::AccountLocked { .. } => "account_locked",
            Self::DuplicateLogin => "duplicate_login",
            Self::NoPassword => "no_password",
            Self::SignupClosed => "signup_closed",
            Self::SignupUsernamePrompt => "signup_username_prompt",
            Self::SignupUsernameInvalid => "signup_username_invalid",
            Self::SignupUsernameTaken => "signup_username_taken",
            Self::SignupPasswordPrompt => "signup_password_prompt",
            Self::SignupPasswordInvalid { .. } => "signup_password_invalid",
            Self::SignupPasswordWeak => "signup_password_weak",
            Self::SignupConfirmPrompt => "signup_confirm_prompt",
            Self::SignupPasswordMismatch => "signup_password_mismatch",
            Self::SignupEmailPrompt => "signup_email_prompt",
            Self::SignupEmailInvalid => "signup_email_invalid",
            Self::SignupGenderPrompt => "signup_gender_prompt",
            Self::SignupGenderInvalid => "signup_gender_invalid",
        }
    }

    /// The pattern registry name, if the entry has one.
    pub fn pattern(&self) -> Option<&'static str> {
        auth_catalogue().messages[self.key()].pattern.as_deref()
    }

    fn arg(&self, name: &str) -> String {
        match (self, name) {
            (Self::Banner { banner }, "banner") => banner.clone(),
            (Self::AccountLocked { minutes }, "minutes") => minutes.to_string(),
            (Self::SignupPasswordInvalid { max }, "max") => max.to_string(),
            _ => panic!("{}: no argument {name}", self.key()),
        }
    }

    /// The text as sent: ANSI colour codes included, `\r\n` line ends, UTF-8 (all of it
    /// ASCII apart from a configured banner).
    pub fn text(&self) -> String {
        let key = self.key();
        let entry = &auth_catalogue().messages[key];
        let mut out = String::new();
        for piece in entry.pieces(key).expect("validated at parse") {
            match piece {
                TemplatePiece::Text(t) => out.push_str(&t),
                TemplatePiece::Arg(name) => out.push_str(&self.arg(&name)),
                TemplatePiece::Ansi(name) => out.push_str(
                    std::str::from_utf8(silvermere_text::ansi::by_name(&name).unwrap_or(b""))
                        .unwrap_or(""),
                ),
                TemplatePiece::Colour(_) => {}
            }
        }
        out
    }
}

/// How the line editor echoes the next input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoMode {
    /// Echo as typed.
    On,
    /// Echo `ch` once per byte typed, without MBBSEmu's width limit (it set
    /// `wid = 0xFF`), so a nine-letter password shows `*********`.
    Secure {
        /// The echo character, `*`.
        ch: u8,
    },
}

/// The secure echo of the password prompts.
pub const SECURE_ECHO: EchoMode = EchoMode::Secure { ch: b'*' };

/// Why the dialogue closed the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// Three failed attempts on this connection.
    TooManyAttempts,
    /// The account already has a session.
    DuplicateLogin,
}

/// A question for the account store; answered with [`LoginDialogue::on_reply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Check `username` and `password`.
    Authenticate {
        /// As typed.
        username: String,
        /// As typed.
        password: Secret,
    },
    /// Is `username` free for signup?
    UsernameFree {
        /// As typed.
        username: String,
    },
    /// Create the account (the password is still plain; the service hashes it).
    Create {
        /// The login name.
        username: String,
        /// The password as typed.
        password: Secret,
        /// Blank or an address.
        email: Option<String>,
        /// `M`/`F`.
        sex: crate::account::Sex,
    },
}

/// Why an authentication was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Unknown name or wrong password.
    InvalidCredentials,
    /// Locked until then.
    Locked {
        /// Whole minutes left.
        minutes: u32,
    },
    /// The account already has a session.
    AlreadyOnline,
    /// The account has no password yet (an imported player's, until the sysop sets one).
    NoPassword,
}

/// The store's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// For `Authenticate`.
    Authenticated(Result<Box<Account>, Refusal>),
    /// For `UsernameFree`.
    UsernameFree(bool),
    /// For `Create`: the new account, or `None` when the name was taken meanwhile.
    Created(Option<Box<Account>>),
}

/// What the dialogue asks of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Send these bytes as they are (the login art, CP437).
    Raw(Vec<u8>),
    /// Send this text.
    Text(AuthMsg),
    /// Echo the next line this way.
    SetEcho(EchoMode),
    /// Answer this with [`LoginDialogue::on_reply`] before the next line.
    Request(Request),
    /// Close the connection (after sending what came before).
    Close,
}

/// The dialogue's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginConfig {
    /// The login art (CP437 bytes, [`crate::art`]), sent before everything else; empty
    /// sends nothing. The server loads it with [`crate::art::load`].
    pub login_art: Vec<u8>,
    /// `compat.login_banner`: one line before the first prompt; empty prints nothing.
    pub login_banner: String,
    /// `game.open_signup`: whether `NEW` creates accounts (John, 2026-10-14: true).
    pub open_signup: bool,
    /// Failed attempts before the connection is closed (3).
    pub max_attempts: u32,
}

impl Default for LoginConfig {
    fn default() -> Self {
        Self {
            login_art: Vec::new(),
            login_banner: String::new(),
            open_signup: true,
            max_attempts: 3,
        }
    }
}

/// Where the dialogue is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing sent yet; [`LoginDialogue::start`] prints the banner and first prompt.
    Banner,
    /// At `Username:`.
    Username,
    /// At `Password:` for `username`.
    Password {
        /// The name typed.
        username: String,
    },
    /// Waiting for the reply to an `Authenticate` request.
    Authenticating,
    /// In the `NEW` dialogue.
    Signup(SignupState),
    /// Logged in.
    Done(AccountId),
    /// The connection is to be closed.
    Closed(CloseReason),
}

/// The login and signup dialogue of one connection.
#[derive(Debug, Clone)]
pub struct LoginDialogue {
    config: LoginConfig,
    state: State,
    attempts: u32,
    account: Option<Account>,
}

impl LoginDialogue {
    /// A dialogue that has sent nothing yet.
    pub fn new(config: LoginConfig) -> Self {
        Self {
            config,
            state: State::Banner,
            attempts: 0,
            account: None,
        }
    }

    /// The current state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The settings.
    pub fn config(&self) -> &LoginConfig {
        &self.config
    }

    /// The account once [`State::Done`].
    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// Takes the account once [`State::Done`].
    pub fn into_account(self) -> Option<Account> {
        self.account
    }

    /// Whether the dialogue is over (logged in or closing).
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done(_) | State::Closed(_))
    }

    /// The first output of a connection: the login art, the banner, MBBSEmu's welcome
    /// tail and the username prompt.
    pub fn start(&mut self) -> Vec<Output> {
        let mut out = Vec::new();
        if self.state != State::Banner {
            return out;
        }
        if !self.config.login_art.is_empty() {
            out.push(Output::Raw(self.config.login_art.clone()));
        }
        if !self.config.login_banner.is_empty() {
            out.push(Output::Text(AuthMsg::Banner {
                banner: self.config.login_banner.clone(),
            }));
        }
        out.push(Output::Text(AuthMsg::WelcomeEnd));
        self.ask_username(&mut out);
        out
    }

    pub(crate) fn ask_username(&mut self, out: &mut Vec<Output>) {
        out.push(Output::Text(AuthMsg::LoginIntro));
        out.push(Output::Text(AuthMsg::UsernamePrompt));
        self.state = State::Username;
    }

    pub(crate) fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub(crate) fn finish(&mut self, account: Account) {
        self.state = State::Done(account.id);
        self.account = Some(account);
    }

    /// One line typed by the user (without its terminator).
    pub fn on_line(&mut self, line: &str) -> Vec<Output> {
        let mut out = Vec::new();
        match std::mem::replace(&mut self.state, State::Banner) {
            State::Banner => {
                out = self.start();
                return out;
            }
            State::Username => self.username_line(line, &mut out),
            State::Password { username } if line.trim().is_empty() => {
                // A stray Enter (MegaMud's script sends one): ask again, no failed attempt.
                out.push(Output::SetEcho(SECURE_ECHO));
                out.push(Output::Text(AuthMsg::PasswordPrompt));
                self.state = State::Password { username };
            }
            State::Password { username } => {
                out.push(Output::SetEcho(EchoMode::On));
                out.push(Output::Request(Request::Authenticate {
                    username,
                    password: Secret::new(line),
                }));
                self.state = State::Authenticating;
            }
            State::Signup(signup) => signup.on_line(self, line, &mut out),
            // Waiting for a reply, or finished: lines are ignored.
            other => self.state = other,
        }
        out
    }

    fn username_line(&mut self, line: &str, out: &mut Vec<Output>) {
        if line.trim().is_empty() {
            self.ask_username(out);
        } else if line.len() > USERNAME_MAX {
            out.push(Output::Text(AuthMsg::UsernameTooLong));
            self.ask_username(out);
        } else if is_new(line) {
            if self.config.open_signup {
                SignupState::start(self, out);
            } else {
                out.push(Output::Text(AuthMsg::SignupClosed));
                self.ask_username(out);
            }
        } else {
            out.push(Output::SetEcho(SECURE_ECHO));
            out.push(Output::Text(AuthMsg::PasswordPrompt));
            self.state = State::Password {
                username: line.to_string(),
            };
        }
    }

    /// The answer to the last [`Output::Request`].
    pub fn on_reply(&mut self, reply: Reply) -> Vec<Output> {
        let mut out = Vec::new();
        match (std::mem::replace(&mut self.state, State::Banner), reply) {
            (State::Authenticating, Reply::Authenticated(result)) => match result {
                Ok(account) => self.finish(*account),
                Err(Refusal::AlreadyOnline) => {
                    out.push(Output::Text(AuthMsg::DuplicateLogin));
                    self.close(CloseReason::DuplicateLogin, &mut out);
                }
                Err(refusal) => {
                    out.push(Output::Text(match refusal {
                        Refusal::Locked { minutes } => AuthMsg::AccountLocked { minutes },
                        Refusal::NoPassword => AuthMsg::NoPassword,
                        _ => AuthMsg::InvalidCredentials,
                    }));
                    self.attempts += 1;
                    if self.attempts >= self.config.max_attempts {
                        self.close(CloseReason::TooManyAttempts, &mut out);
                    } else {
                        self.ask_username(&mut out);
                    }
                }
            },
            (State::Signup(signup), reply) => signup.on_reply(self, reply, &mut out),
            (state, reply) => {
                self.state = state;
                debug_assert!(false, "unexpected reply {reply:?} in {:?}", self.state);
            }
        }
        out
    }

    fn close(&mut self, reason: CloseReason, out: &mut Vec<Output>) {
        out.push(Output::Close);
        self.state = State::Closed(reason);
    }
}

/// Where "now" comes from: the system clock on a server, a test clock in tests.
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

impl<C: Clock + ?Sized> Clock for std::sync::Arc<C> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

/// The accounts with a session: the one-session-per-account rule.
#[derive(Debug, Default)]
pub struct Online(Mutex<HashSet<AccountId>>);

impl Online {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<AccountId>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claims `id` for a session; `false` if it already has one.
    pub fn claim(&self, id: AccountId) -> bool {
        self.lock().insert(id)
    }

    /// Releases `id` when its session ends.
    pub fn release(&self, id: AccountId) {
        self.lock().remove(&id);
    }

    /// Whether `id` has a session.
    pub fn contains(&self, id: AccountId) -> bool {
        self.lock().contains(&id)
    }
}

/// Answers the dialogue's requests against a store; one per server, shared by the
/// sessions.
#[derive(Debug)]
pub struct LoginService<S, C> {
    store: S,
    clock: C,
    lockout: Lockout,
    online: Online,
    cost: password::Cost,
}

impl<S: AccountStore, C: Clock> LoginService<S, C> {
    /// A service over `store` with the lockout settings `lockout`.
    pub fn new(store: S, clock: C, lockout: LockoutConfig) -> Self {
        Self {
            store,
            clock,
            lockout: Lockout::new(lockout),
            online: Online::default(),
            cost: password::Cost::Default,
        }
    }

    /// Uses `cost` for the hashes of accounts created by signup (tests).
    pub fn with_cost(mut self, cost: password::Cost) -> Self {
        self.cost = cost;
        self
    }

    /// The store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// The clock.
    pub fn clock(&self) -> &C {
        &self.clock
    }

    /// The lockout tracker.
    pub fn lockout(&self) -> &Lockout {
        &self.lockout
    }

    /// The online set; the session releases its account here when it ends.
    pub fn online(&self) -> &Online {
        &self.online
    }

    /// The account named `username` with its lockout fields filled in.
    pub async fn account(&self, username: &str) -> Result<Option<Account>, AuthError> {
        let now = self.clock.now();
        Ok(self.store.by_username(username).await?.map(|mut a| {
            a.failed_logins = self.lockout.failed_logins(a.id, now);
            a.locked_until = self.lockout.locked_until(a.id, now);
            a
        }))
    }

    /// Feeds `line` to `dialogue` and answers its requests; returns the outputs for the
    /// session (never a `Request`).
    pub async fn drive(
        &self,
        dialogue: &mut LoginDialogue,
        line: &str,
    ) -> Result<Vec<Output>, AuthError> {
        let first = dialogue.on_line(line);
        self.resolve(dialogue, first).await
    }

    /// [`LoginDialogue::start`] for symmetry with [`drive`](Self::drive).
    pub fn start(&self, dialogue: &mut LoginDialogue) -> Vec<Output> {
        dialogue.start()
    }

    async fn resolve(
        &self,
        dialogue: &mut LoginDialogue,
        mut pending: Vec<Output>,
    ) -> Result<Vec<Output>, AuthError> {
        let mut out = Vec::new();
        while !pending.is_empty() {
            let mut next = Vec::new();
            for o in pending {
                match o {
                    Output::Request(req) => {
                        let reply = self.answer(req).await?;
                        next.extend(dialogue.on_reply(reply));
                    }
                    other => out.push(other),
                }
            }
            pending = next;
        }
        Ok(out)
    }

    /// Answers one request.
    pub async fn answer(&self, request: Request) -> Result<Reply, AuthError> {
        Ok(match request {
            Request::Authenticate { username, password } => {
                Reply::Authenticated(self.authenticate(&username, password.expose()).await?)
            }
            Request::UsernameFree { username } => {
                Reply::UsernameFree(self.store.by_username(&username).await?.is_none())
            }
            Request::Create {
                username,
                password,
                email,
                sex,
            } => {
                let new = NewAccount {
                    username,
                    password_hash: password::hash_with(password.expose(), self.cost),
                    email,
                    sex,
                    roles: crate::roles::RoleSet::player(),
                };
                Reply::Created(self.signup(new).await?.map(Box::new))
            }
        })
    }

    async fn signup(&self, new: NewAccount) -> Result<Option<Account>, AuthError> {
        let now = self.clock.now();
        let mut account = match self.store.insert(&new, now).await {
            Ok(a) => a,
            Err(DbError::Duplicate(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        account.last_login_at = Some(now);
        self.store.update(&account).await?;
        // Claimed only once the update is stored, so a failed update leaves no claim
        // behind (T-013 review). A new account cannot have a session yet.
        self.online.claim(account.id);
        tracing::info!(target: "audit", account = account.id.0, username = %account.username, "signup");
        let audited = self
            .audit(
                now,
                "auth.signup",
                serde_json::json!({ "account": account.id.0, "username": account.username }),
            )
            .await;
        if let Err(e) = audited {
            // nobody holds the claim any more: the caller gets an error, not a session
            self.online.release(account.id);
            return Err(e);
        }
        Ok(Some(account))
    }

    /// Checks a username and password: lockout first, then the hash, then the session
    /// rule. Records the outcome in the tracker and the audit trail.
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Result<Box<Account>, Refusal>, AuthError> {
        let now = self.clock.now();
        let Some(mut account) = self.store.by_username(username).await? else {
            // Spend the same time as a wrong password.
            let _ = password::verify(password, password::dummy_hash());
            tracing::info!(target: "audit", username, "login failed: unknown user");
            self.audit(
                now,
                "auth.login_failed",
                serde_json::json!({ "username": username, "reason": "unknown" }),
            )
            .await?;
            return Ok(Err(Refusal::InvalidCredentials));
        };
        let id = account.id;
        if let Some(until) = self.lockout.locked_until(id, now) {
            tracing::info!(target: "audit", account = id.0, username = %account.username, "login refused: locked");
            self.audit(
                now,
                "auth.login_locked",
                serde_json::json!({ "account": id.0, "username": account.username }),
            )
            .await?;
            return Ok(Err(Refusal::Locked {
                minutes: Lockout::minutes_left(until, now),
            }));
        }
        if account.password_hash.is_empty() {
            // Nobody can log in to it, so a try is not a guess: no lockout count.
            tracing::info!(target: "audit", account = id.0, username = %account.username, "login refused: no password");
            self.audit(
                now,
                "auth.login_failed",
                serde_json::json!({ "account": id.0, "username": account.username, "reason": "no_password" }),
            )
            .await?;
            return Ok(Err(Refusal::NoPassword));
        }
        if !password::verify(password, &account.password_hash) {
            let locked = self.lockout.record_failure(id, now);
            tracing::info!(target: "audit", account = id.0, username = %account.username, "login failed: wrong password");
            self.audit(
                now,
                "auth.login_failed",
                serde_json::json!({ "account": id.0, "username": account.username, "reason": "password" }),
            )
            .await?;
            if let Some(until) = locked {
                tracing::info!(target: "audit", account = id.0, username = %account.username, until = %until, "account locked");
                self.audit(
                    now,
                    "auth.lockout",
                    serde_json::json!({ "account": id.0, "username": account.username, "until": until.to_string() }),
                )
                .await?;
            }
            return Ok(Err(Refusal::InvalidCredentials));
        }
        if self.online.contains(id) {
            tracing::info!(target: "audit", account = id.0, username = %account.username, "login refused: already online");
            self.audit(
                now,
                "auth.login_duplicate",
                serde_json::json!({ "account": id.0, "username": account.username }),
            )
            .await?;
            return Ok(Err(Refusal::AlreadyOnline));
        }
        self.lockout.record_success(id);
        account.last_login_at = Some(now);
        self.store.update(&account).await?;
        // Claimed after the update is stored, so a failed update leaves no claim
        // behind (T-013 review). Another session may have claimed the account while the
        // update ran: refuse as a duplicate then.
        if !self.online.claim(id) {
            return Ok(Err(Refusal::AlreadyOnline));
        }
        tracing::info!(target: "audit", account = id.0, username = %account.username, "login");
        let audited = self
            .audit(
                now,
                "auth.login",
                serde_json::json!({ "account": id.0, "username": account.username }),
            )
            .await;
        if let Err(e) = audited {
            // the caller gets an error, not a session: nobody would ever release the claim
            self.online.release(id);
            return Err(e);
        }
        Ok(Ok(Box::new(account)))
    }

    async fn audit(
        &self,
        now: Timestamp,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<(), AuthError> {
        Ok(self.store.audit(Event::new(now, kind, payload)).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_the_catalogue() {
        let keys: Vec<&str> = auth_catalogue()
            .messages
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(AuthMsg::KEYS, keys.as_slice());
    }

    #[test]
    fn locked_text_substitutes_minutes() {
        let t = AuthMsg::AccountLocked { minutes: 7 }.text();
        assert!(t.contains("locked for 7 more minute(s)"), "{t:?}");
    }
}
