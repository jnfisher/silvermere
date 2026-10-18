//! Lockout: ten wrong passwords within 15 minutes lock an account for 15 minutes
//! (silvermere's rule; the BBS had none). Runs on the backend `SILVERMERE_DATABASE_URL`
//! names (a temp SQLite file when unset).

use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use silvermere_auth::login::Refusal;
use silvermere_auth::password::Cost;
use silvermere_auth::{
    AuthMsg, Clock, LockoutConfig, LoginConfig, LoginDialogue, LoginService, Output, Role, State,
    cli,
};
use silvermere_db::{Db, TestDb};

#[derive(Debug)]
struct TestClock(Mutex<Timestamp>);

impl TestClock {
    fn advance(&self, secs: i64) {
        let mut t = self.0.lock().unwrap();
        *t = t.checked_add(SignedDuration::from_secs(secs)).unwrap();
    }
}

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

async fn setup() -> (
    TestDb,
    LoginService<Arc<Db>, Arc<TestClock>>,
    Arc<TestClock>,
) {
    let t = TestDb::from_env().await.expect("test database");
    cli::create_with_cost(
        &**t.db(),
        "Kasimir",
        "kangtest1",
        &[Role::Player],
        Cost::Low,
    )
    .await
    .unwrap();
    let clock = Arc::new(TestClock(Mutex::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    )));
    let svc = LoginService::new(t.db().clone(), clock.clone(), LockoutConfig::default());
    (t, svc, clock)
}

#[tokio::test]
async fn ten_failures_lock_until_the_lock_expires() {
    let (t, svc, clock) = setup().await;
    for i in 0..10 {
        let r = svc.authenticate("kasimir", "wrong").await.unwrap();
        assert_eq!(r.unwrap_err(), Refusal::InvalidCredentials, "failure {i}");
        clock.advance(30);
    }
    let a = svc.account("kasimir").await.unwrap().unwrap();
    assert_eq!(a.failed_logins, 10);
    let until = a.locked_until.expect("locked after ten failures");

    // The right password is refused while locked, with the lockout text.
    let r = svc.authenticate("Kasimir", "kangtest1").await.unwrap();
    assert!(matches!(r, Err(Refusal::Locked { minutes: 15 })), "{r:?}");
    let mut dlg = LoginDialogue::new(LoginConfig::default());
    dlg.start();
    svc.drive(&mut dlg, "kasimir").await.unwrap();
    let out = svc.drive(&mut dlg, "kangtest1").await.unwrap();
    assert!(
        out.contains(&Output::Text(AuthMsg::AccountLocked { minutes: 15 })),
        "{out:?}"
    );
    assert_eq!(dlg.state(), &State::Username);

    // Once `locked_until` has passed, it succeeds and the count resets.
    let now = clock.now();
    clock.advance(until.duration_since(now).as_secs());
    let r = svc.authenticate("kasimir", "kangtest1").await.unwrap();
    let account = r.expect("unlocked");
    assert_eq!(account.last_login_at, Some(clock.now()));
    let a = svc.account("kasimir").await.unwrap().unwrap();
    assert_eq!(a.failed_logins, 0);
    assert_eq!(a.locked_until, None);

    // The audit trail has every step.
    let kinds: Vec<String> = t
        .events()
        .recent(100)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.event.kind)
        .collect();
    for kind in [
        "auth.account_created",
        "auth.login_failed",
        "auth.lockout",
        "auth.login_locked",
        "auth.login",
    ] {
        assert!(kinds.iter().any(|k| k == kind), "{kind} in {kinds:?}");
    }
    drop(svc);
    t.close().await;
}

#[tokio::test]
async fn a_success_resets_the_count_before_the_lock() {
    let (t, svc, _clock) = setup().await;
    for _ in 0..9 {
        assert!(svc.authenticate("kasimir", "wrong").await.unwrap().is_err());
    }
    assert!(
        svc.authenticate("kasimir", "kangtest1")
            .await
            .unwrap()
            .is_ok()
    );
    svc.online()
        .release(svc.account("kasimir").await.unwrap().unwrap().id);
    assert_eq!(
        svc.account("kasimir").await.unwrap().unwrap().failed_logins,
        0
    );
    // Nine more do not lock.
    for _ in 0..9 {
        assert!(svc.authenticate("kasimir", "wrong").await.unwrap().is_err());
    }
    assert!(
        svc.authenticate("kasimir", "kangtest1")
            .await
            .unwrap()
            .is_ok()
    );
    drop(svc);
    t.close().await;
}

#[tokio::test]
async fn the_numbers_are_configurable() {
    let (t, _, clock) = setup().await;
    let svc = LoginService::new(
        t.db().clone(),
        clock.clone(),
        LockoutConfig {
            max_failures: 2,
            window_secs: 60,
            lock_secs: 120,
        },
    );
    svc.authenticate("kasimir", "a").await.unwrap().unwrap_err();
    clock.advance(61);
    // The first failure has left the window.
    svc.authenticate("kasimir", "b").await.unwrap().unwrap_err();
    assert_eq!(
        svc.account("kasimir").await.unwrap().unwrap().locked_until,
        None
    );
    svc.authenticate("kasimir", "c").await.unwrap().unwrap_err();
    let r = svc.authenticate("kasimir", "kangtest1").await.unwrap();
    assert_eq!(r.unwrap_err(), Refusal::Locked { minutes: 2 });
    drop(svc);
    t.close().await;
}
