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

/// The takeover tests' dialogue settings: `net.login_takeover` on.
fn takeover_on() -> LoginConfig {
    LoginConfig {
        takeover: true,
        ..LoginConfig::default()
    }
}

/// An [`Evict`] that notes whom it was asked to disconnect.
#[derive(Default)]
struct Evicted(Mutex<Vec<silvermere_types::ids::AccountId>>);

impl silvermere_auth::login::Evict for Evicted {
    fn evict(&self, account: silvermere_types::ids::AccountId) {
        self.0.lock().unwrap().push(account);
    }
}

async fn service_with_evict() -> (Service, Arc<Evicted>) {
    let evicted = Arc::new(Evicted::default());
    (service().await.with_evict(evicted.clone()), evicted)
}

#[tokio::test]
async fn with_takeover_on_a_second_login_is_asked_and_n_keeps_the_refusal() {
    let (svc, evicted) = service_with_evict().await;
    let (_, first) = run(&svc, takeover_on(), &["kasimir", "kangtest1"]).await;
    assert!(matches!(first.state(), State::Done(_)));
    let (out, dlg) = run(&svc, takeover_on(), &["Kasimir", "kangtest1"]).await;
    assert!(matches!(dlg.state(), State::Takeover { .. }));
    assert!(out.contains(&Output::Text(AuthMsg::TakeoverPrompt)));
    assert!(!out.contains(&Output::Close));
    let mut dlg = dlg;
    let out = svc.drive(&mut dlg, "n").await.unwrap();
    assert_eq!(dlg.state(), &State::Closed(CloseReason::DuplicateLogin));
    assert!(texts(&out).contains(&"This account is already logged on.".to_string()));
    assert_eq!(out.last(), Some(&Output::Close));
    assert!(evicted.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn y_disconnects_the_other_session_and_logs_in_here() {
    let (svc, evicted) = service_with_evict().await;
    let (_, first) = run(&svc, takeover_on(), &["kasimir", "kangtest1"]).await;
    let id = first.account().unwrap().id;
    let (_, mut dlg) = run(&svc, takeover_on(), &["Kasimir", "kangtest1", "maybe"]).await;
    // an answer that is neither Y nor N asks again
    assert!(matches!(dlg.state(), State::Takeover { .. }));
    // the other session goes as the server would close it: once asked to
    let release = async {
        loop {
            if evicted.0.lock().unwrap().contains(&id) {
                svc.online().release(id);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    let (out, ()) = tokio::join!(svc.drive(&mut dlg, " y"), release);
    let out = out.unwrap();
    assert!(!out.contains(&Output::Close));
    assert_eq!(dlg.state(), &State::Done(id));
    assert!(
        svc.online().contains(id),
        "the new session holds the account"
    );
    assert_eq!(evicted.0.lock().unwrap().as_slice(), &[id]);
}

#[tokio::test]
async fn the_question_again_after_a_bad_answer() {
    let (svc, _) = service_with_evict().await;
    run(&svc, takeover_on(), &["kasimir", "kangtest1"]).await;
    let (out, dlg) = run(&svc, takeover_on(), &["Kasimir", "kangtest1", "what"]).await;
    assert!(matches!(dlg.state(), State::Takeover { .. }));
    assert_eq!(out.last(), Some(&Output::Text(AuthMsg::TakeoverQuestion)));
}

#[tokio::test(start_paused = true)]
async fn a_session_that_never_lets_go_fails_the_takeover_after_the_wait() {
    let (svc, evicted) = service_with_evict().await;
    let (_, first) = run(&svc, takeover_on(), &["kasimir", "kangtest1"]).await;
    let id = first.account().unwrap().id;
    let started = tokio::time::Instant::now();
    let (out, dlg) = run(&svc, takeover_on(), &["Kasimir", "kangtest1", "Y"]).await;
    assert!(started.elapsed() >= silvermere_auth::login::TAKEOVER_WAIT);
    assert_eq!(dlg.state(), &State::Closed(CloseReason::DuplicateLogin));
    assert!(out.contains(&Output::Text(AuthMsg::TakeoverFailed)));
    assert_eq!(out.last(), Some(&Output::Close));
    assert_eq!(evicted.0.lock().unwrap().as_slice(), &[id]);
    // the first session still holds the account
    assert!(svc.online().contains(id));
}

#[tokio::test]
async fn a_wrong_password_is_never_asked_about_a_takeover() {
    let (svc, evicted) = service_with_evict().await;
    run(&svc, takeover_on(), &["kasimir", "kangtest1"]).await;
    let (out, dlg) = run(&svc, takeover_on(), &["Kasimir", "wrong"]).await;
    assert!(out.contains(&Output::Text(AuthMsg::InvalidCredentials)));
    assert!(!out.contains(&Output::Text(AuthMsg::TakeoverPrompt)));
    assert_eq!(dlg.state(), &State::Username);
    assert!(evicted.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn takeover_off_refuses_as_before() {
    let (svc, evicted) = service_with_evict().await;
    run(&svc, LoginConfig::default(), &["kasimir", "kangtest1"]).await;
    let (out, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert_eq!(dlg.state(), &State::Closed(CloseReason::DuplicateLogin));
    assert!(!out.contains(&Output::Text(AuthMsg::TakeoverPrompt)));
    assert!(evicted.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_banned_account_is_told_why_and_closed_after_the_right_password() {
    let svc = service().await;
    let id = svc.account("Kasimir").await.unwrap().unwrap().id;
    let until = Timestamp::from_second(1_800_003_600).unwrap();
    svc.store().set_ban(
        id,
        Some(silvermere_auth::account::Ban {
            until: Some(until),
            reason: "spamming gossip".into(),
        }),
    );
    // a wrong password says nothing of the ban
    let (out, _) = run(&svc, LoginConfig::default(), &["Kasimir", "wrong"]).await;
    assert!(texts(&out).contains(&"Invalid Credentials".to_string()));
    let (out, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert_eq!(dlg.state(), &State::Closed(CloseReason::Banned));
    assert!(
        texts(&out).contains(
            &"This account is banned until 2027-01-15 09:00 UTC. Reason: spamming gossip"
                .to_string()
        ),
        "{:?}",
        texts(&out)
    );
    assert_eq!(out.last(), Some(&Output::Close));
    assert!(!svc.online().contains(id), "a refused login claims nothing");
    // a ban with no end and no reason
    svc.store().set_ban(
        id,
        Some(silvermere_auth::account::Ban {
            until: None,
            reason: String::new(),
        }),
    );
    let (out, _) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert!(texts(&out).contains(&"This account is banned.".to_string()));
    // lifted: in again
    svc.store().set_ban(id, None);
    let (_, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert!(matches!(dlg.state(), State::Done(_)));
}

#[tokio::test]
async fn a_closed_realm_refuses_players_and_admits_sysops() {
    let svc = service().await;
    svc.store()
        .insert(
            &NewAccount {
                username: "Boss".into(),
                password_hash: password::hash_with("bosstest1", Cost::Low),
                email: None,
                sex: Sex::Male,
                roles: [silvermere_auth::Role::Player, silvermere_auth::Role::Sysop]
                    .into_iter()
                    .collect(),
            },
            Timestamp::UNIX_EPOCH,
        )
        .await
        .unwrap();
    svc.realm().close("upgrading to 1.0");
    let (out, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert_eq!(dlg.state(), &State::Closed(CloseReason::RealmClosed));
    assert!(
        texts(&out).contains(
            &"The realm is closed: upgrading to 1.0. Please try again later.".to_string()
        ),
        "{:?}",
        texts(&out)
    );
    let (_, dlg) = run(&svc, LoginConfig::default(), &["Boss", "bosstest1"]).await;
    assert!(matches!(dlg.state(), State::Done(_)), "a sysop logs in");
    svc.realm().open();
    let (_, dlg) = run(&svc, LoginConfig::default(), &["Kasimir", "kangtest1"]).await;
    assert!(matches!(dlg.state(), State::Done(_)));
}
