//! The `silvermere account` library functions through the database pool.

use std::sync::Mutex;

use jiff::Timestamp;
use silvermere_auth::cli::{self, Grant};
use silvermere_auth::password::{self, Cost};
use silvermere_auth::{
    Account, AccountFlags, AccountStore, AuthError, MemoryStore, NewAccount, Perm, Role,
};
use silvermere_db::{DbError, Event, TestDb};
use silvermere_types::ids::AccountId;

async fn role_rows(t: &TestDb, id: AccountId) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM account_roles WHERE account_id = $1")
        .bind(id.0)
        .fetch_one(&t.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn create_list_grant_set_password_delete() {
    let t = TestDb::from_env().await.expect("test database");
    let db = &**t.db();

    let a = cli::create_with_cost(db, "Kasimir", "kangtest1", &[], Cost::Low)
        .await
        .unwrap();
    let b = cli::create_with_cost(db, "Aldric", "aldric1", &[Role::GameOp], Cost::Low)
        .await
        .unwrap();
    assert!(a.roles.contains(Role::Player) && !a.roles.contains(Role::Sysop));
    assert!(b.roles.contains(Role::Player) && b.roles.contains(Role::GameOp));

    // Names are unique ignoring case.
    let dup = cli::create_with_cost(db, "KASIMIR", "x1", &[], Cost::Low).await;
    assert!(matches!(dup, Err(AuthError::UsernameTaken(_))), "{dup:?}");
    let bad = cli::create_with_cost(db, &"z".repeat(30), "x1", &[], Cost::Low).await;
    assert!(matches!(bad, Err(AuthError::Username(_))), "{bad:?}");

    let names: Vec<String> = cli::list(db)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.username)
        .collect();
    assert_eq!(names, ["Kasimir", "Aldric"]);

    let k = cli::grant_role(db, "kasimir", Role::Sysop).await.unwrap();
    assert!(k.roles.contains(Role::Sysop));
    let stored = db.by_username("Kasimir").await.unwrap().unwrap();
    assert!(stored.roles.contains(Role::Sysop));
    assert_eq!(role_rows(&t, a.id).await, 2);

    // Permissions and flags go through the same functions.
    cli::grant(db, "aldric", "SYS_GOTO_KEY".parse::<Grant>().unwrap())
        .await
        .unwrap();
    cli::grant(db, "aldric", "no_gossip".parse::<Grant>().unwrap())
        .await
        .unwrap();
    let stored = db.by_username("aldric").await.unwrap().unwrap();
    assert!(stored.perms.contains(Perm::SysGoto));
    assert!(stored.flags.contains(AccountFlags::NO_GOSSIP));
    cli::revoke(db, "aldric", Grant::Perm(Perm::SysGoto))
        .await
        .unwrap();
    cli::revoke_role(db, "aldric", Role::GameOp).await.unwrap();
    let stored = db.by_username("aldric").await.unwrap().unwrap();
    assert!(!stored.perms.contains(Perm::SysGoto));
    assert!(!stored.roles.contains(Role::GameOp));
    assert!(cli::describe(&stored).contains("no_gossip"));

    cli::set_password_with_cost(db, "Kasimir", "newpass", Cost::Low)
        .await
        .unwrap();
    let stored = db.by_username("kasimir").await.unwrap().unwrap();
    assert!(password::verify("newpass", &stored.password_hash));
    assert!(!password::verify("kangtest1", &stored.password_hash));
    assert!(matches!(
        cli::set_password(db, "nobody", "x1").await,
        Err(AuthError::NoSuchAccount(_))
    ));

    assert!(cli::delete(db, "KASIMIR").await.unwrap());
    assert!(!cli::delete(db, "kasimir").await.unwrap());
    assert_eq!(db.by_username("kasimir").await.unwrap(), None);
    assert_eq!(role_rows(&t, a.id).await, 0, "account_roles rows go too");
    assert_eq!(cli::list(db).await.unwrap().len(), 1);

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
        "auth.role",
        "auth.password",
        "auth.account_deleted",
    ] {
        assert!(kinds.iter().any(|k| k == kind), "{kind} in {kinds:?}");
    }
    t.close().await;
}

/// A store that records the character hook.
#[derive(Default)]
struct Recording {
    inner: MemoryStore,
    hook: Mutex<Vec<AccountId>>,
}

impl AccountStore for Recording {
    async fn by_username(&self, u: &str) -> Result<Option<Account>, DbError> {
        self.inner.by_username(u).await
    }
    async fn get(&self, id: AccountId) -> Result<Option<Account>, DbError> {
        self.inner.get(id).await
    }
    async fn all(&self) -> Result<Vec<Account>, DbError> {
        self.inner.all().await
    }
    async fn insert(&self, new: &NewAccount, now: Timestamp) -> Result<Account, DbError> {
        self.inner.insert(new, now).await
    }
    async fn update(&self, a: &Account) -> Result<bool, DbError> {
        self.inner.update(a).await
    }
    async fn delete(&self, id: AccountId) -> Result<bool, DbError> {
        // The character goes first.
        assert_eq!(self.hook.lock().unwrap().last(), Some(&id));
        self.inner.delete(id).await
    }
    async fn delete_characters(&self, id: AccountId) -> Result<(), DbError> {
        self.hook.lock().unwrap().push(id);
        Ok(())
    }
    async fn audit(&self, e: Event) -> Result<(), DbError> {
        self.inner.audit(e).await
    }
}

#[tokio::test]
async fn delete_calls_the_character_hook_first() {
    let store = Recording::default();
    let a = cli::create_with_cost(&store, "bob", "pw", &[], Cost::Low)
        .await
        .unwrap();
    assert!(cli::delete(&store, "Bob").await.unwrap());
    assert_eq!(*store.hook.lock().unwrap(), [a.id]);
}
