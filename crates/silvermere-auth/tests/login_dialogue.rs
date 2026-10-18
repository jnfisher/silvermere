//! The login and signup dialogue against an in-memory store: no sockets, no database.
//! Texts are MBBSEmu's (`MenuRoutines.cs`), as the captures `entry-existing` and
//! `entry-new-account` show them.

use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use silvermere_auth::login::{Request, SECURE_ECHO};
use silvermere_auth::password::{self, Cost};
use silvermere_auth::{
    AccountStore, AuthMsg, Clock, CloseReason, EchoMode, LockoutConfig, LoginConfig, LoginDialogue,
    LoginService, MemoryStore, NewAccount, Output, RoleSet, Sex, State,
};
use silvermere_text::strip_ansi_str;

#[derive(Debug)]
struct TestClock(Mutex<Timestamp>);

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

type Service = LoginService<Arc<MemoryStore>, Arc<TestClock>>;

async fn service() -> Service {
    let store = Arc::new(MemoryStore::new());
    store
        .insert(
            &NewAccount {
                username: "Kasimir".into(),
                password_hash: password::hash_with("kangtest1", Cost::Low),
                email: None,
                sex: Sex::Male,
                roles: RoleSet::player(),
            },
            Timestamp::UNIX_EPOCH,
        )
        .await
        .unwrap();
    let clock = Arc::new(TestClock(Mutex::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    )));
    LoginService::new(store, clock, LockoutConfig::default()).with_cost(Cost::Low)
}

/// Runs `lines` through a fresh dialogue; returns every output and the dialogue.
async fn run(svc: &Service, config: LoginConfig, lines: &[&str]) -> (Vec<Output>, LoginDialogue) {
    let mut dlg = LoginDialogue::new(config);
    let mut out = svc.start(&mut dlg);
    for line in lines {
        out.extend(svc.drive(&mut dlg, line).await.unwrap());
    }
    (out, dlg)
}

fn texts(out: &[Output]) -> Vec<String> {
    out.iter()
        .filter_map(|o| match o {
            Output::Text(m) => Some(strip_ansi_str(&m.text()).trim().to_string()),
            _ => None,
        })
        .filter(|t| !t.is_empty())
        .collect()
}

fn pattern_example(name: &str) -> String {
    let registry: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
    registry["static"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("pattern {name}"))["example"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn login_prints_the_mbbsemu_prompts_and_succeeds() {
    let svc = service().await;
    let (out, dlg) = run(&svc, LoginConfig::default(), &["kasimir", "kangtest1"]).await;
    assert_eq!(
        texts(&out),
        vec![
            pattern_example("bbs_banner"),
            pattern_example("bbs_username"),
            pattern_example("bbs_password"),
        ]
    );
    let State::Done(id) = dlg.state() else {
        panic!("{:?}", dlg.state())
    };
    let account = dlg.account().unwrap();
    assert_eq!(account.id, *id);
    assert_eq!(account.username, "Kasimir", "stored with the case given");
    assert!(account.last_login_at.is_some());
    assert!(svc.online().contains(*id));
}

#[tokio::test]
async fn bytes_before_the_password_match_the_capture() {
    // tests/captures/entry-existing/raw.bin after login.ans's `\e[255D`;
    // the user's echo ("Aldric\r\n") is the line editor's and not part of the texts.
    let mut dlg = LoginDialogue::new(LoginConfig::default());
    let wire: String = dlg
        .start()
        .iter()
        .filter_map(|o| match o {
            Output::Text(m) => Some(m.text()),
            _ => None,
        })
        .collect();
    assert_eq!(
        wire,
        "\r\n \r\n\x1b[33mEnter Username or enter \"\x1b[1mNEW\x1b[0m\x1b[33m\" to create a new Account\r\n\x1b[1m\x1b[37mUsername:\x1b[0m "
    );
    assert_eq!(
        AuthMsg::PasswordPrompt.text(),
        "\x1b[1m\x1b[37mPassword:\x1b[0m "
    );
}

#[tokio::test]
async fn the_banner_comes_first_when_configured() {
    let svc = service().await;
    let config = LoginConfig {
        login_banner: "Welcome to silvermere.".into(),
        ..LoginConfig::default()
    };
    let (out, _) = run(&svc, config, &[]).await;
    assert_eq!(texts(&out)[0], "Welcome to silvermere.");
}

#[tokio::test]
async fn username_is_case_insensitive() {
    let svc = service().await;
    let (_, dlg) = run(&svc, LoginConfig::default(), &["KASIMIR", "kangtest1"]).await;
    assert!(matches!(dlg.state(), State::Done(_)), "{:?}", dlg.state());
}

#[tokio::test]
async fn three_wrong_passwords_close_the_connection() {
    let svc = service().await;
    let lines = ["kasimir", "nope", "kasimir", "nope", "kasimir", "nope"];
    let (out, dlg) = run(&svc, LoginConfig::default(), &lines).await;
    assert_eq!(dlg.state(), &State::Closed(CloseReason::TooManyAttempts));
    assert_eq!(out.last(), Some(&Output::Close));
    let invalid = out
        .iter()
        .filter(|o| **o == Output::Text(AuthMsg::InvalidCredentials))
        .count();
    assert_eq!(invalid, 3);
    // After the first two the username prompt comes back, as MenuRoutines did.
    let prompts = out
        .iter()
        .filter(|o| **o == Output::Text(AuthMsg::UsernamePrompt))
        .count();
    assert_eq!(prompts, 3);
}

#[tokio::test]
async fn an_unknown_name_is_invalid_credentials() {
    let svc = service().await;
    let (out, dlg) = run(&svc, LoginConfig::default(), &["nobody", "x"]).await;
    assert!(out.contains(&Output::Text(AuthMsg::InvalidCredentials)));
    assert_eq!(dlg.state(), &State::Username);
}

#[tokio::test]
async fn a_second_session_is_refused_and_closed() {
    let svc = service().await;
    let (_, first) = run(&svc, LoginConfig::default(), &["kasimir", "kangtest1"]).await;
    assert!(matches!(first.state(), State::Done(_)));
    let (out, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert_eq!(dlg.state(), &State::Closed(CloseReason::DuplicateLogin));
    assert!(texts(&out).contains(&"This account is already logged on.".to_string()));
    assert_eq!(out.last(), Some(&Output::Close));
    // Once the first session ends, logging in works again.
    svc.online().release(first.account().unwrap().id);
    let (_, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert!(matches!(dlg.state(), State::Done(_)));
}

#[tokio::test]
async fn signup_creates_an_account() {
    let svc = service().await;
    let (out, dlg) = run(
        &svc,
        LoginConfig::default(),
        &["new", "bob", "pw", "pw", "", "M"],
    )
    .await;
    let State::Done(id) = dlg.state() else {
        panic!("{:?}", dlg.state())
    };
    let stored = svc.store().get(*id).await.unwrap().unwrap();
    assert_eq!(stored.username, "bob");
    assert_eq!(stored.sex, Sex::Male);
    assert_eq!(stored.email, None);
    assert!(password::verify("pw", &stored.password_hash));
    assert!(svc.online().contains(*id));
    assert_eq!(
        texts(&out)[2..],
        [
            "Please enter a unique Username (Max. 29 Characters):",
            "Please enter a strong Password:",
            "Please re-enter your password to confirm:",
            "Please enter a valid e-Mail Address:",
            "Please enter your gender 'M' or 'F' (can be changed later):",
        ]
    );
    let kinds: Vec<String> = svc
        .store()
        .events()
        .iter()
        .map(|e| e.kind.clone())
        .collect();
    assert_eq!(kinds, ["auth.signup"]);
}

#[tokio::test]
async fn signup_keeps_email_and_female() {
    let svc = service().await;
    let lines = ["NEW", "Aldra", "secret", "secret", "aldra@example.com", "f"];
    let (_, dlg) = run(&svc, LoginConfig::default(), &lines).await;
    let account = dlg.account().unwrap();
    assert_eq!(account.sex, Sex::Female);
    assert_eq!(account.email.as_deref(), Some("aldra@example.com"));
}

#[tokio::test]
async fn signup_reasks_a_taken_username() {
    let svc = service().await;
    let (out, dlg) = run(&svc, LoginConfig::default(), &["new", "kasimir"]).await;
    let t = texts(&out);
    assert_eq!(
        t[t.len() - 2..],
        [
            "That Username is unavailable, please choose another.",
            "Please enter a unique Username (Max. 29 Characters):",
        ]
    );
    assert!(matches!(dlg.state(), State::Signup(_)));
}

#[tokio::test]
async fn a_30_character_username_is_refused() {
    let svc = service().await;
    let long = "x".repeat(30);
    let (out, dlg) = run(&svc, LoginConfig::default(), &[&long]).await;
    assert!(out.contains(&Output::Text(AuthMsg::UsernameTooLong)));
    assert_eq!(dlg.state(), &State::Username);
    let (out, dlg) = run(&svc, LoginConfig::default(), &["new", &long]).await;
    assert!(out.contains(&Output::Text(AuthMsg::UsernameTooLong)));
    assert!(matches!(dlg.state(), State::Signup(_)));
    // 29 is fine.
    let ok = "y".repeat(29);
    let (_, dlg) = run(
        &svc,
        LoginConfig::default(),
        &["new", &ok, "pw", "pw", "", "F"],
    )
    .await;
    assert!(matches!(dlg.state(), State::Done(_)));
}

#[tokio::test]
async fn signup_refusals_reask_the_same_question() {
    let svc = service().await;
    let lines = [
        "new",
        "",    // blank name
        "NEW", // reserved
        "carl",
        "password", // MBBSEmu's one weak password
        "abc",
        "abd", // mismatch: back to the password
        "abc",
        "abc",
        "no address", // refused, re-asked
        "",
        "x", // not M/F
        "m",
    ];
    let (out, dlg) = run(&svc, LoginConfig::default(), &lines).await;
    for msg in [
        AuthMsg::SignupUsernameInvalid,
        AuthMsg::SignupPasswordWeak,
        AuthMsg::SignupPasswordMismatch,
        AuthMsg::SignupEmailInvalid,
        AuthMsg::SignupGenderInvalid,
    ] {
        assert!(out.contains(&Output::Text(msg.clone())), "{msg:?}");
    }
    assert!(matches!(dlg.state(), State::Done(_)), "{:?}", dlg.state());
}

#[tokio::test]
async fn signup_can_be_closed() {
    let svc = service().await;
    let config = LoginConfig {
        open_signup: false,
        ..LoginConfig::default()
    };
    let (out, dlg) = run(&svc, config, &["new"]).await;
    assert!(out.contains(&Output::Text(AuthMsg::SignupClosed)));
    assert_eq!(dlg.state(), &State::Username);
}

#[test]
fn echo_is_secure_for_the_password_only() {
    // Pure: no store needed until the password line.
    let mut dlg = LoginDialogue::new(LoginConfig::default());
    dlg.start();
    let out = dlg.on_line("kasimir");
    assert_eq!(
        out,
        vec![
            Output::SetEcho(SECURE_ECHO),
            Output::Text(AuthMsg::PasswordPrompt)
        ]
    );
    assert_eq!(SECURE_ECHO, EchoMode::Secure { ch: b'*' });
    let out = dlg.on_line("kangtest1");
    assert_eq!(out[0], Output::SetEcho(EchoMode::On));
    assert_eq!(
        out[1],
        Output::Request(Request::Authenticate {
            username: "kasimir".into(),
            password: "kangtest1".into(),
        })
    );
}

#[test]
fn signup_echo_is_secure_for_both_passwords() {
    let mut dlg = LoginDialogue::new(LoginConfig::default());
    dlg.start();
    dlg.on_line("new");
    dlg.on_line("bob");
    let out = dlg.on_reply(silvermere_auth::login::Reply::UsernameFree(true));
    assert_eq!(out[0], Output::SetEcho(SECURE_ECHO));
    let out = dlg.on_line("pw");
    assert_eq!(out[0], Output::SetEcho(EchoMode::On));
    assert_eq!(out[1], Output::SetEcho(SECURE_ECHO));
    let out = dlg.on_line("pw");
    assert_eq!(out[0], Output::SetEcho(EchoMode::On));
    assert_eq!(out[1], Output::Text(AuthMsg::SignupEmailPrompt));
}

#[test]
fn every_text_with_a_pattern_matches_its_example() {
    let registry: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
    for msg in [
        AuthMsg::LoginIntro,
        AuthMsg::UsernamePrompt,
        AuthMsg::PasswordPrompt,
    ] {
        let name = msg.pattern().unwrap();
        let entry = registry["static"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap();
        assert_eq!(
            strip_ansi_str(&msg.text()).trim(),
            entry["example"].as_str().unwrap()
        );
    }
}

/// What a [`FailingStore`] refuses.
#[derive(Clone, Copy)]
enum Fail {
    /// every `update`
    Update,
    /// the audit rows of a successful login or signup
    Audit,
}

/// A store that fails on purpose, to show a failed login leaves no claim.
struct FailingStore(Arc<MemoryStore>, Fail);

impl AccountStore for FailingStore {
    async fn by_username(
        &self,
        username: &str,
    ) -> silvermere_auth::account::StoreResult<Option<silvermere_auth::Account>> {
        self.0.by_username(username).await
    }
    async fn get(
        &self,
        id: silvermere_types::ids::AccountId,
    ) -> silvermere_auth::account::StoreResult<Option<silvermere_auth::Account>> {
        self.0.get(id).await
    }
    async fn all(&self) -> silvermere_auth::account::StoreResult<Vec<silvermere_auth::Account>> {
        self.0.all().await
    }
    async fn insert(
        &self,
        new: &NewAccount,
        now: Timestamp,
    ) -> silvermere_auth::account::StoreResult<silvermere_auth::Account> {
        self.0.insert(new, now).await
    }
    async fn update(
        &self,
        account: &silvermere_auth::Account,
    ) -> silvermere_auth::account::StoreResult<bool> {
        match self.1 {
            Fail::Update => Err(silvermere_db::DbError::Duplicate("forced".into())),
            Fail::Audit => self.0.update(account).await,
        }
    }
    async fn delete(
        &self,
        id: silvermere_types::ids::AccountId,
    ) -> silvermere_auth::account::StoreResult<bool> {
        self.0.delete(id).await
    }
    async fn delete_characters(
        &self,
        id: silvermere_types::ids::AccountId,
    ) -> silvermere_auth::account::StoreResult<()> {
        self.0.delete_characters(id).await
    }
    async fn audit(
        &self,
        event: silvermere_db::Event,
    ) -> silvermere_auth::account::StoreResult<()> {
        if matches!(self.1, Fail::Audit)
            && matches!(event.kind.as_str(), "auth.login" | "auth.signup")
        {
            return Err(silvermere_db::DbError::Duplicate("forced".into()));
        }
        self.0.audit(event).await
    }
}

// T-013 review (silvermere#17): the session is claimed only after the account update is
// stored, so an update error leaves the account free to log in again.
#[tokio::test]
async fn a_failed_update_leaves_no_claim() {
    let inner = Arc::new(MemoryStore::new());
    let account = inner
        .insert(
            &NewAccount {
                username: "Kasimir".into(),
                password_hash: password::hash_with("kangtest1", Cost::Low),
                email: None,
                sex: Sex::Male,
                roles: RoleSet::player(),
            },
            Timestamp::UNIX_EPOCH,
        )
        .await
        .unwrap();
    let clock = Arc::new(TestClock(Mutex::new(Timestamp::UNIX_EPOCH)));
    let svc = LoginService::new(
        FailingStore(inner, Fail::Update),
        clock,
        LockoutConfig::default(),
    );
    assert!(svc.authenticate("Kasimir", "kangtest1").await.is_err());
    assert!(!svc.online().contains(account.id));
    let new = NewAccount {
        username: "Bob".into(),
        password_hash: password::hash_with("pw", Cost::Low),
        email: None,
        sex: Sex::Male,
        roles: RoleSet::player(),
    };
    let created = svc
        .answer(Request::Create {
            username: new.username,
            password: "pw".into(),
            email: None,
            sex: Sex::Male,
        })
        .await;
    assert!(created.is_err());
    let bob = svc.store().0.by_username("Bob").await.unwrap().unwrap();
    assert!(!svc.online().contains(bob.id));
}

// T-029 (gate review, finding 7): the claim is taken before the audit row of the login or the
// signup is written, so an audit error has to give it back, or the account stays "already
// logged on" with nobody holding it.
#[tokio::test]
async fn a_failed_audit_write_releases_the_claim() {
    let inner = Arc::new(MemoryStore::new());
    let account = inner
        .insert(
            &NewAccount {
                username: "Kasimir".into(),
                password_hash: password::hash_with("kangtest1", Cost::Low),
                email: None,
                sex: Sex::Male,
                roles: RoleSet::player(),
            },
            Timestamp::UNIX_EPOCH,
        )
        .await
        .unwrap();
    let clock = Arc::new(TestClock(Mutex::new(Timestamp::UNIX_EPOCH)));
    let svc = LoginService::new(
        FailingStore(inner, Fail::Audit),
        clock,
        LockoutConfig::default(),
    );
    assert!(svc.authenticate("Kasimir", "kangtest1").await.is_err());
    assert!(
        !svc.online().contains(account.id),
        "login: the claim was left behind"
    );
    let created = svc
        .answer(Request::Create {
            username: "Bob".into(),
            password: "pw".into(),
            email: None,
            sex: Sex::Male,
        })
        .await;
    assert!(created.is_err());
    let bob = svc.store().0.by_username("Bob").await.unwrap().unwrap();
    assert!(
        !svc.online().contains(bob.id),
        "signup: the claim was left behind"
    );
}

// T-115: an empty or whitespace-only line re-asks the prompt it answers and is not a
// failed attempt.
#[tokio::test]
async fn empty_lines_reprompt_and_do_not_count_as_failures() {
    let svc = service().await;
    let mut lines = vec![""; 2];
    lines.extend(["  ", "Kasimir", "", " \t", "  "]);
    lines.extend(["nope", "Kasimir", "", "kangtest1"]);
    let (out, dlg) = run(&svc, LoginConfig::default(), &lines).await;
    assert!(matches!(dlg.state(), State::Done(_)), "{:?}", dlg.state());
    let count = |m: AuthMsg| {
        out.iter()
            .filter(|o| **o == Output::Text(m.clone()))
            .count()
    };
    // Start, three stray lines, one after the wrong password: 1 + 3 + 1 prompts, one
    // failure only.
    assert_eq!(count(AuthMsg::UsernamePrompt), 5);
    assert_eq!(count(AuthMsg::PasswordPrompt), 1 + 3 + 1 + 1);
    assert_eq!(count(AuthMsg::InvalidCredentials), 1);
}
