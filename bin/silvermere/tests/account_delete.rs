//! T-069: deleting an account while the server runs. `DELETE /v1/accounts/{id}` against a
//! live engine, and `silvermere account delete` choosing the route, the direct deletion or
//! a refusal from what answers on `server.admin_bind` and the single-writer guard (T-072). On the backend
//! `SILVERMERE_DATABASE_URL` names (SQLite by default).

use crate::common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Output};
use std::time::Duration;

use silvermere::config::{AdminCfg, Config, TokenCfg, TokenScope};
use silvermere_db::repo::accounts::Account;
use silvermere_db::{Db, TestDb, TryAcquire, WriterGuard};
use silvermere_engine::world::blank;
use silvermere_engine::{SeededRng, SystemClock};
use silvermere_types::enums::OwnerKind;
use silvermere_types::flags::GangFlags;
use silvermere_types::ids::{AccountId, ActiveMonsterId, ItemNo, MapId, RoomId, ShopNo};
use silvermere_types::records::{BankBook, Gang, ItemOwnership};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "root-secret";
/// The fixture's limited item: one copy in play, carried by Lars.
const LIMITED: ItemNo = ItemNo(3);

/// A fresh database with the fixture world, and its URL.
async fn world() -> (TestDb, String) {
    let t = TestDb::from_env().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let mut item = blank::clear_item(LIMITED);
    item.limit = 1;
    item.active = 1;
    t.db().items().insert(&item).await.unwrap();
    let url = t.db().url().to_string();
    (t, url)
}

/// An account whose character, `first`, belongs to Wolves (two members) and carries the
/// limited item when `limited`; it has a bank book.
async fn player(db: &Db, first: &str, limited: bool) -> AccountId {
    let account = db
        .accounts()
        .insert(&Account::new(
            &first.to_ascii_lowercase(),
            "",
            jiff::Timestamp::now(),
        ))
        .await
        .unwrap();
    let userid = first.to_ascii_uppercase();
    let mut c = common::stored_character(&userid, first);
    c.gang = "Wolves".into();
    if limited {
        c.items[0] = LIMITED;
        c.itemuses[0] = -1;
        db.item_ownership()
            .insert(&ItemOwnership {
                id: 1,
                itemnum: LIMITED,
                owntype: OwnerKind::User,
                mapnum: MapId(0),
                roomnum: RoomId(0),
                userid: userid.clone(),
                monnum: ActiveMonsterId(0),
            })
            .await
            .unwrap();
    }
    db.characters().insert(account, &c).await.unwrap();
    db.bank_books()
        .put(
            account,
            &BankBook {
                userid,
                bank: ShopNo(8),
                balance: 500,
                loan: 0,
            },
        )
        .await
        .unwrap();
    account
}

async fn wolves(db: &Db) {
    db.gangs()
        .insert(&Gang {
            key: "WOLVES".into(),
            name: "Wolves".into(),
            exp: 0,
            leader: "Lars".into(),
            created: 0,
            members: 2,
            flags: GangFlags::empty(),
            overexp: 0,
            overflows: 0,
        })
        .await
        .unwrap();
}

fn config(url: &str, scopes: Vec<TokenScope>) -> Config {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url.into();
    cfg.admin = AdminCfg {
        tokens: vec![TokenCfg {
            name: "ops".into(),
            sha256: None,
            token: Some(TOKEN.into()),
            scopes,
        }],
        ..Default::default()
    };
    cfg
}

/// One request with `Connection: close`; the status and the body.
async fn http(addr: SocketAddr, method: &str, path: &str) -> (u16, String) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 0\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_string(&mut raw))
        .await
        .expect("an answer")
        .unwrap();
    let code = raw.split(' ').nth(1).unwrap().parse().unwrap();
    let body = raw
        .split_once("\r\n\r\n")
        .map_or("", |(_, b)| b)
        .to_string();
    (code, body)
}

/// What is left of `account` in the store.
async fn leftovers(db: &Db, account: AccountId) -> Vec<&'static str> {
    let mut left = Vec::new();
    if db.accounts().get(account).await.unwrap().is_some() {
        left.push("account");
    }
    if db.characters().by_account(account).await.unwrap().is_some() {
        left.push("character");
    }
    if !db
        .bank_books()
        .for_account(account)
        .await
        .unwrap()
        .is_empty()
    {
        left.push("bank book");
    }
    left
}

// The route against a live engine: Lars, in Wolves with the limited item, is deleted; the
// gang counts one member fewer, the copy is released, the rows go, and the shutdown's
// flush writes none of it back.
#[tokio::test]
async fn delete_route_runs_dlarou_in_the_engine() {
    let (t, url) = world().await;
    let db = t.db().clone();
    wolves(&db).await;
    let lars = player(&db, "Lars", true).await;
    let kai = player(&db, "Kai", false).await;

    let r = silvermere::serve_in_process(
        config(&url, vec![TokenScope::Admin]),
        SystemClock,
        SeededRng::new(7),
    )
    .await
    .expect("starts");
    let admin = r.admin_addr.expect("admin listener");
    let (code, body) = http(admin, "DELETE", &format!("/v1/accounts/{}", lars.0)).await;
    assert_eq!(code, 200, "{body}");
    let j: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["deleted"]["account"], lars.0);
    assert_eq!(j["deleted"]["character"], "Lars");
    let (code, body) = http(admin, "DELETE", &format!("/v1/accounts/{}", lars.0)).await;
    assert_eq!(code, 404, "deleted already: {body}");
    // the engine's later writes: a reload and the shutdown's saves and flush
    assert_eq!(http(admin, "POST", "/v1/reload-world").await.0, 200);
    tokio::time::timeout(Duration::from_secs(10), r.stop())
        .await
        .expect("stops")
        .expect("engine joins");

    assert_eq!(leftovers(&db, lars).await, Vec::<&str>::new());
    assert_eq!(
        leftovers(&db, kai).await,
        ["account", "character", "bank book"]
    );
    let g = db.gangs().by_name("WOLVES").await.unwrap().expect("gang");
    assert_eq!(g.members, 1);
    assert!(g.flags.contains(GangFlags::DISBANDED), "Lars led it");
    assert!(db.item_ownership().all().await.unwrap().is_empty());
    let item = db.items().get(LIMITED).await.unwrap().expect("item");
    assert_eq!(item.active, 0);
    let kinds: Vec<String> = db
        .events()
        .recent(100)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.event.kind)
        .collect();
    assert!(
        kinds.contains(&"auth.account_deleted".to_string()),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&format!("admin.accounts.{}", lars.0)),
        "{kinds:?}"
    );
    t.close().await;
}

/// Runs the binary with a clean `SILVERMERE_*` environment plus `env`.
fn silvermere(env: &[(&str, String)], args: &[&str]) -> Output {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

async fn delete_cli(url: &str, admin: String, token: Option<&str>, name: &str) -> Output {
    let mut env = vec![
        ("SILVERMERE_DATABASE_URL", url.to_string()),
        ("SILVERMERE_SERVER__ADMIN_BIND", admin),
    ];
    if let Some(t) = token {
        env.push(("SILVERMERE_ADMIN_TOKEN", t.to_string()));
    }
    let name = name.to_string();
    tokio::task::spawn_blocking(move || silvermere(&env, &["account", "delete", &name, "--yes"]))
        .await
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A port nothing listens on.
fn closed_port() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    addr.to_string()
}

/// Something on the admin port that is not a working API: every connection is answered
/// with `reply`.
fn broken_api(reply: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { break };
            let mut buf = [0u8; 1024];
            let _ = c.read(&mut buf);
            let _ = c.write_all(reply.as_bytes());
        }
    });
    addr
}

// `account delete`: through the route with a server up (and refusing without an admin
// token), directly with none, and refusing when the admin port answers but its API is not
// usable.
#[tokio::test(flavor = "multi_thread")]
async fn account_delete_cli_picks_the_route_the_store_or_a_refusal() {
    let (t, url) = world().await;
    let db = t.db().clone();
    wolves(&db).await;
    let lars = player(&db, "Lars", true).await;
    let kai = player(&db, "Kai", false).await;

    // a server holds the store's guard (T-072) but its API is unreachable, or down: no
    // deletion
    let TryAcquire::Taken(guard) = WriterGuard::try_acquire(&url).await.unwrap() else {
        panic!("the guard is free");
    };
    for reply in [
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
        "garbage",
    ] {
        let o = delete_cli(&url, broken_api(reply), Some(TOKEN), "kai").await;
        assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
        assert!(stderr(&o).contains("cannot be used"), "{}", stderr(&o));
    }
    let o = delete_cli(&url, closed_port(), Some(TOKEN), "kai").await;
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("a server is running on this store"),
        "{}",
        stderr(&o)
    );
    assert!(
        stderr(&o).contains("nothing answers there"),
        "{}",
        stderr(&o)
    );
    assert_eq!(leftovers(&db, kai).await.len(), 3);
    guard.release().await;

    // a server up: the route, which needs an admin token with its secret
    let r = silvermere::serve_in_process(
        config(&url, vec![TokenScope::Read]),
        SystemClock,
        SeededRng::new(7),
    )
    .await
    .expect("starts");
    let admin = r.admin_addr.expect("admin listener").to_string();
    let o = delete_cli(&url, admin.clone(), None, "lars").await;
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("SILVERMERE_ADMIN_TOKEN"),
        "{}",
        stderr(&o)
    );
    assert_eq!(leftovers(&db, lars).await.len(), 3, "refused");
    let o = delete_cli(&url, admin.clone(), Some("not-the-token"), "lars").await;
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("401"), "{}", stderr(&o));
    let o = delete_cli(&url, admin.clone(), Some(TOKEN), "nobody").await;
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("no account named nobody"),
        "{}",
        stderr(&o)
    );
    // SILVERMERE_ADMIN_TOKEN is the server's own `default` token too
    r.stop().await.expect("engine joins");
    let r = silvermere::serve_in_process(
        Config {
            admin: AdminCfg {
                tokens: vec![TokenCfg {
                    name: "default".into(),
                    sha256: None,
                    token: Some(TOKEN.into()),
                    scopes: vec![TokenScope::Read, TokenScope::Write, TokenScope::Admin],
                }],
                ..Default::default()
            },
            ..config(&url, vec![])
        },
        SystemClock,
        SeededRng::new(7),
    )
    .await
    .expect("starts");
    let admin = r.admin_addr.expect("admin listener").to_string();
    let o = delete_cli(&url, admin, Some(TOKEN), "lars").await;
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("through the running server"),
        "{}",
        stderr(&o)
    );
    r.stop().await.expect("engine joins");
    assert_eq!(leftovers(&db, lars).await, Vec::<&str>::new());
    let g = db.gangs().by_name("WOLVES").await.unwrap().expect("gang");
    assert_eq!(g.members, 1, "the engine's dlarou ran");

    // no server: the rows directly, whatever answers on the admin port, since nothing
    // holds the guard
    let o = delete_cli(&url, broken_api("garbage"), None, "kai").await;
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!stderr(&o).contains("through"), "{}", stderr(&o));
    assert_eq!(leftovers(&db, kai).await, Vec::<&str>::new());
    t.close().await;
}

/// An admin API that answers `/healthz` and times out every deletion (504
/// `engine_timeout`).
fn timing_out_api() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { break };
            // the whole request first: unread bytes at close reset the connection
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && c.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let reply = if head.starts_with(b"GET /healthz") {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
            } else {
                "HTTP/1.1 504 Gateway Timeout\r\nContent-Length: 27\r\n\r\n{\"error\":\"engine_timeout\"}"
            };
            let _ = c.write_all(reply.as_bytes());
        }
    });
    addr
}

// T-072: a 504 from the route says the deletion was queued and may still complete.
#[tokio::test(flavor = "multi_thread")]
async fn account_delete_cli_reports_a_504_as_queued() {
    let (t, url) = world().await;
    let db = t.db().clone();
    wolves(&db).await;
    let lars = player(&db, "Lars", true).await;
    let o = delete_cli(&url, timing_out_api(), Some(TOKEN), "lars").await;
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let err = stderr(&o);
    assert!(err.contains("queued the deletion of lars"), "{err}");
    assert!(err.contains("may still complete"), "{err}");
    assert!(!err.contains("refused"), "{err}");
    assert_eq!(leftovers(&db, lars).await.len(), 3);
    t.close().await;
}
