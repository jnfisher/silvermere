//! The admin API over real HTTP: the router served on a loopback socket and a plain
//! HTTP/1.1 client over TCP. The auth matrix on every route, reload-world, the SSE
//! filters and keep-alive, the log-leak check and the doc-versus-router checks.

use crate::common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{FakeHealth, FakeLog, FakeWorld};
use serde_json::{Value, json};
use silvermere_api::{ApiState, EventKind, NewEvent, Scope, Token, metrics, router, routes};
use silvermere_engine::session::AccountDeletion;
use silvermere_engine::{AdminRequest, EngineInput, WorldError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKENS: [(&str, &str, &[Scope]); 3] = [
    ("reader", "read-secret", &[Scope::Read]),
    ("writer", "write-secret", &[Scope::Write]),
    ("root", "root-secret", &[Scope::Admin]),
];

/// A planned shutdown that only notes what it was asked.
#[derive(Default)]
struct FakeShutdown {
    plans: Mutex<Vec<(u64, String)>>,
}

impl silvermere_api::ShutdownPlan for FakeShutdown {
    fn schedule(
        &self,
        after: Duration,
        message: String,
    ) -> Result<silvermere_api::Planned, String> {
        self.plans.lock().unwrap().push((after.as_secs(), message));
        Ok(silvermere_api::Planned {
            at: "2026-11-18T20:05:00Z".into(),
            in_secs: after.as_secs(),
            restart: false,
        })
    }

    fn cancel(&self) -> bool {
        self.plans.lock().unwrap().pop().is_some()
    }

    fn schedule_restart(
        &self,
        after: Duration,
        message: String,
    ) -> Result<silvermere_api::Planned, String> {
        self.plans.lock().unwrap().push((after.as_secs(), message));
        Ok(silvermere_api::Planned {
            at: "2026-11-18T20:05:00Z".into(),
            in_secs: after.as_secs(),
            restart: true,
        })
    }
}

/// Backups that are made at once, in name only.
struct FakeBackups;

impl silvermere_api::BackupProbe for FakeBackups {
    fn last_ok(&self) -> Option<bool> {
        None
    }

    fn status(&self) -> Value {
        json!({ "enabled": true, "next": null, "last": null, "backups": [] })
    }

    fn run_now(&self) -> Result<Value, String> {
        Ok(json!({ "at": "2026-11-18T20:05:00Z", "ok": true, "file": "b.db", "bytes": 1 }))
    }
}

/// Moderation that only notes what it was asked; `nobody` is no player.
#[derive(Default)]
struct FakeModerator {
    actions: Mutex<Vec<(silvermere_api::Action, String)>>,
}

impl silvermere_api::Moderator for FakeModerator {
    fn act(
        &self,
        action: silvermere_api::Action,
        by: String,
    ) -> silvermere_api::events::BoxFuture<'_, Result<Value, silvermere_api::ModError>> {
        use silvermere_api::Action;
        let refused = matches!(
            &action,
            Action::Kick { player, .. } | Action::Put { player, .. } if player == "nobody"
        );
        self.actions.lock().unwrap().push((action, by));
        Box::pin(async move {
            if refused {
                Err(silvermere_api::ModError::NoSuchPlayer)
            } else {
                Ok(json!({ "ok": true }))
            }
        })
    }

    fn status(
        &self,
    ) -> silvermere_api::events::BoxFuture<'_, Result<Value, silvermere_api::ModError>> {
        Box::pin(async {
            Ok(json!({ "realm": { "closed": false }, "sanctions": [], "ip_bans": [] }))
        })
    }
}

struct Server {
    addr: std::net::SocketAddr,
    state: ApiState,
    shutdown: Arc<FakeShutdown>,
    moderator: Arc<FakeModerator>,
    world: Arc<FakeWorld>,
    /// Snapshots the fake engine received through `ReloadContent`.
    reloads: Arc<Mutex<usize>>,
    engine_fails: Arc<AtomicBool>,
    _engine: Sender<EngineInput>,
}

async fn server() -> Server {
    log_sink();
    let (tx, rx) = channel::<EngineInput>();
    let reloads = Arc::new(Mutex::new(0));
    let engine_fails = Arc::new(AtomicBool::new(false));
    {
        let (reloads, engine_fails) = (reloads.clone(), engine_fails.clone());
        std::thread::spawn(move || {
            while let Ok(input) = rx.recv() {
                match input {
                    EngineInput::Admin(AdminRequest::Who { reply }) => {
                        let _ = reply.send(Vec::new());
                    }
                    EngineInput::Admin(AdminRequest::Broadcast { reply, .. }) => {
                        let _ = reply.send(0);
                    }
                    EngineInput::Admin(AdminRequest::ReloadContent { reply, .. }) => {
                        *reloads.lock().unwrap() += 1;
                        let _ = reply.send(if engine_fails.load(Ordering::SeqCst) {
                            Err(WorldError::NoRooms)
                        } else {
                            Ok(())
                        });
                    }
                    // account 7 has a character, 8 none; any other id is unknown
                    EngineInput::Admin(AdminRequest::DeleteAccount { account, reply }) => {
                        let _ = reply.send(match account.0 {
                            7 => AccountDeletion::Queued {
                                character: Some("Lars".into()),
                            },
                            8 => AccountDeletion::Queued { character: None },
                            _ => AccountDeletion::NotFound,
                        });
                    }
                    _ => {}
                }
            }
        });
    }
    let world = Arc::new(FakeWorld::default());
    let shutdown = Arc::new(FakeShutdown::default());
    let moderator = Arc::new(FakeModerator::default());
    let (_recorder, handle) = metrics::build();
    let state = ApiState {
        engine: tx.clone(),
        events: ApiState::event_bus(),
        log: Arc::new(FakeLog::default()),
        health: Arc::new(FakeHealth {
            alive: AtomicBool::new(true),
            persist: AtomicBool::new(true),
            tick: AtomicU64::new(1),
        }),
        metrics: handle,
        world: world.clone(),
        backup: Arc::new(FakeBackups),
        shutdown: shutdown.clone(),
        moderation: moderator.clone(),
        console: Some(Token::new("web-console", "console-secret", &[Scope::Admin])),
        tokens: Arc::new(
            TOKENS
                .iter()
                .map(|(n, s, sc)| Token::new(n, s, sc))
                .collect(),
        ),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Server {
        addr,
        state,
        shutdown,
        moderator,
        world,
        reloads,
        engine_fails,
        _engine: tx,
    }
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

async fn request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> Reply {
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut raw))
        .await
        .expect("response within 5 s")
        .unwrap();
    let status = raw.split(' ').nth(1).unwrap().parse().unwrap();
    let (headers, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    Reply {
        status,
        headers: headers.to_string(),
        body: body.to_string(),
    }
}

/// A concrete request path for a route template.
fn concrete(path: &str) -> String {
    path.replace("{id}", "7")
        .replace("{name}", "Lars")
        .replace("{kind}", "jail")
}

fn body_for(method: &str, path: &str) -> Option<&'static str> {
    match (method, path) {
        ("POST", "/v1/broadcast" | "/v1/players/{name}/tell") => Some(r#"{"text":"hello"}"#),
        (_, "/v1/ip-bans") => Some(r#"{"network":"203.0.113.7"}"#),
        _ => None,
    }
}

#[tokio::test]
async fn auth_matrix_on_every_route() {
    let srv = server().await;
    for spec in routes() {
        let path = concrete(&spec.path);
        let body = body_for(spec.method, &spec.path);
        // SSE: do not hold the connection open; the matrix needs only the refusals and,
        // for the allowed tokens, the status line.
        if spec.path == "/v1/events" {
            continue;
        }
        let call = |token: Option<&'static str>| {
            let (m, p) = (spec.method, path.clone());
            let addr = srv.addr;
            async move { request(addr, m, &p, token, body).await }
        };
        let who = format!("{} {}", spec.method, spec.path);
        match spec.scope {
            None => {
                // Open routes ignore the token.
                assert_eq!(call(None).await.status, 200, "{who}");
                assert_eq!(call(Some("wrong")).await.status, 200, "{who}");
            }
            Some(needed) => {
                let none = call(None).await;
                assert_eq!(none.status, 401, "{who}: no token");
                assert_eq!(none.json()["error"], "unauthorized", "{who}");
                assert!(none.json()["message"].is_string(), "{who}");
                let wrong = call(Some("not-a-token")).await;
                assert_eq!(wrong.status, 401, "{who}: wrong token");
                for (name, secret, scopes) in TOKENS {
                    let allowed = scopes.contains(&Scope::Admin) || scopes.contains(&needed);
                    let r = call(Some(secret)).await;
                    if allowed {
                        let ok = if spec.built { 200 } else { 501 };
                        assert_eq!(r.status, ok, "{who} with {name}: {}", r.body);
                    } else {
                        assert_eq!(r.status, 403, "{who} with {name}");
                        assert_eq!(r.json()["error"], "forbidden", "{who} with {name}");
                        assert!(r.json()["message"].is_string(), "{who} with {name}");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn events_route_checks_the_token_over_http() {
    let srv = server().await;
    assert_eq!(
        request(srv.addr, "GET", "/v1/events", None, None)
            .await
            .status,
        401
    );
    assert_eq!(
        request(srv.addr, "GET", "/v1/events", Some("write-secret"), None)
            .await
            .status,
        403
    );
}

#[tokio::test]
async fn reload_world_reports_counts_and_asks_the_engine_once() {
    let srv = server().await;
    let r = request(
        srv.addr,
        "POST",
        "/v1/reload-world",
        Some("write-secret"),
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let j = r.json();
    for table in [
        "rooms",
        "items",
        "monsters",
        "spells",
        "classes",
        "races",
        "shops",
        "messages",
        "text_blocks",
        "actions",
    ] {
        assert_eq!(j["reloaded"][table], 0, "{table}");
    }
    assert!(j["ms"].is_u64());
    assert_eq!(*srv.reloads.lock().unwrap(), 1);
    // GET is not the verb.
    assert_eq!(
        request(
            srv.addr,
            "GET",
            "/v1/reload-world",
            Some("write-secret"),
            None
        )
        .await
        .status,
        405
    );
}

#[tokio::test]
async fn delete_account_answers_from_the_engine_and_is_audited() {
    let srv = server().await;
    let del = |path: &'static str| request(srv.addr, "DELETE", path, Some("root-secret"), None);
    let r = del("/v1/accounts/7").await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["deleted"]["account"], 7);
    assert_eq!(r.json()["deleted"]["character"], "Lars");
    let r = del("/v1/accounts/8").await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.json()["deleted"]["character"].is_null());
    let r = del("/v1/accounts/9").await;
    assert_eq!(
        (r.status, r.json()["error"].as_str()),
        (404, Some("no_such_account"))
    );
    for bad in ["/v1/accounts/x", "/v1/accounts/0", "/v1/accounts/-3"] {
        let r = del(bad).await;
        assert_eq!(
            (r.status, r.json()["error"].as_str()),
            (400, Some("bad_id")),
            "{bad}"
        );
    }
    // write scope is not enough
    let r = request(
        srv.addr,
        "DELETE",
        "/v1/accounts/7",
        Some("write-secret"),
        None,
    )
    .await;
    assert_eq!(r.status, 403);

    let rows = srv.state.log.since(0, 100).await.unwrap();
    let audited: Vec<_> = rows
        .iter()
        .filter(|e| e.kind == "admin.accounts.7")
        .map(|e| (e.payload["status"].clone(), e.payload["token"].clone()))
        .collect();
    assert_eq!(
        audited,
        vec![(json!(200), json!("root")), (json!(403), json!("writer"))]
    );
    let ok = rows.iter().find(|e| e.kind == "admin.accounts.7").unwrap();
    assert_eq!(ok.payload["method"], "DELETE");
    assert_eq!(ok.payload["character"], "Lars");
    let missing = rows.iter().find(|e| e.kind == "admin.accounts.9").unwrap();
    assert_eq!(missing.payload["status"], 404);
}

#[tokio::test]
async fn reload_failure_is_500_reload_failed_from_the_loader_or_the_engine() {
    let srv = server().await;
    *srv.world.fail.lock().unwrap() = Some("database is down".into());
    let r = request(
        srv.addr,
        "POST",
        "/v1/reload-world",
        Some("root-secret"),
        None,
    )
    .await;
    assert_eq!(
        (r.status, r.json()["error"].clone()),
        (500, json!("reload_failed"))
    );
    assert_eq!(*srv.reloads.lock().unwrap(), 0, "engine never asked");

    *srv.world.fail.lock().unwrap() = None;
    srv.engine_fails.store(true, Ordering::SeqCst);
    let r = request(
        srv.addr,
        "POST",
        "/v1/reload-world",
        Some("root-secret"),
        None,
    )
    .await;
    assert_eq!(
        (r.status, r.json()["error"].clone()),
        (500, json!("reload_failed"))
    );
    assert!(r.json()["message"].is_string());
    assert_eq!(*srv.reloads.lock().unwrap(), 1);
}

// The moderation routes: each body becomes its action, named by the token; the scopes,
// the limits and the errors.
#[tokio::test]
async fn moderation_routes_check_their_bodies_and_pass_the_action_on() {
    use silvermere_api::Action;
    use silvermere_api::moderation::Kind;
    let srv = server().await;
    let call = |method: &'static str, path: &'static str, token, body: Option<&'static str>| {
        request(srv.addr, method, path, Some(token), body)
    };
    let r = call(
        "PUT",
        "/v1/players/Lars/jail",
        "root-secret",
        Some(r#"{"for_secs":3600,"reason":" pk in town "}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call("PUT", "/v1/players/Lars/mute", "root-secret", None).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call("DELETE", "/v1/players/Lars/ban", "root-secret", None).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(
        "POST",
        "/v1/players/Lars/kick",
        "root-secret",
        Some(r#"{"reason":"break"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(
        "POST",
        "/v1/players/Lars/tell",
        "write-secret",
        Some(r#"{"text":"hello"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(
        "POST",
        "/v1/ip-bans",
        "root-secret",
        Some(r#"{"network":"203.0.113.0/24"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(
        "DELETE",
        "/v1/ip-bans",
        "root-secret",
        Some(r#"{"network":"203.0.113.0/24"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(
        "POST",
        "/v1/realm/close",
        "root-secret",
        Some(r#"{"reason":"upgrading"}"#),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call("POST", "/v1/realm/open", "root-secret", None).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call("GET", "/v1/moderation", "read-secret", None).await;
    assert_eq!(
        (r.status, r.json()["realm"]["closed"].clone()),
        (200, json!(false))
    );
    assert_eq!(
        srv.moderator.actions.lock().unwrap().clone(),
        vec![
            (
                Action::Put {
                    player: "Lars".into(),
                    kind: Kind::Jail,
                    for_secs: Some(3600),
                    reason: "pk in town".into()
                },
                "root".into()
            ),
            (
                Action::Put {
                    player: "Lars".into(),
                    kind: Kind::Mute,
                    for_secs: None,
                    reason: String::new()
                },
                "root".into()
            ),
            (
                Action::Lift {
                    player: "Lars".into(),
                    kind: Kind::Ban
                },
                "root".into()
            ),
            (
                Action::Kick {
                    player: "Lars".into(),
                    reason: "break".into()
                },
                "root".into()
            ),
            (
                Action::Tell {
                    player: "Lars".into(),
                    text: "hello".into()
                },
                "writer".into()
            ),
            (
                Action::BanIp {
                    network: "203.0.113.0/24".into(),
                    for_secs: None,
                    reason: String::new()
                },
                "root".into()
            ),
            (
                Action::UnbanIp {
                    network: "203.0.113.0/24".into()
                },
                "root".into()
            ),
            (
                Action::CloseRealm {
                    reason: "upgrading".into()
                },
                "root".into()
            ),
            (Action::OpenRealm, "root".into()),
        ]
    );
    // refusals reach no moderator
    for (method, path, token, body, status, code) in [
        (
            "PUT",
            "/v1/players/Lars/jail",
            "root-secret",
            Some(r#"{"for_secs":0}"#),
            400,
            "bad_duration",
        ),
        (
            "PUT",
            "/v1/players/Lars/jail",
            "root-secret",
            Some(r#"{"for_secs":315360001}"#),
            400,
            "bad_duration",
        ),
        (
            "PUT",
            "/v1/players/Lars/ban",
            "root-secret",
            Some(r#"{"reason":"a\u001bb"}"#),
            400,
            "bad_text",
        ),
        (
            "PUT",
            "/v1/players/Lars/ban",
            "root-secret",
            Some(r#"{"until":"tomorrow"}"#),
            400,
            "bad_body",
        ),
        (
            "PUT",
            "/v1/players/Lars/frog",
            "root-secret",
            None,
            404,
            "not_found",
        ),
        (
            "POST",
            "/v1/players/Lars/tell",
            "write-secret",
            Some(r#"{"text":"  "}"#),
            400,
            "bad_text",
        ),
        (
            "PUT",
            "/v1/players/Lars/ban",
            "write-secret",
            None,
            403,
            "forbidden",
        ),
        (
            "POST",
            "/v1/players/Lars/kick",
            "write-secret",
            None,
            403,
            "forbidden",
        ),
        (
            "POST",
            "/v1/realm/close",
            "write-secret",
            None,
            403,
            "forbidden",
        ),
        (
            "POST",
            "/v1/players/Lars/tell",
            "read-secret",
            Some(r#"{"text":"x"}"#),
            403,
            "forbidden",
        ),
    ] {
        let r = request(srv.addr, method, path, Some(token), body).await;
        assert_eq!(
            (r.status, r.json()["error"].clone()),
            (status, json!(code)),
            "{method} {path} {body:?}"
        );
    }
    assert_eq!(srv.moderator.actions.lock().unwrap().len(), 9);
    // the moderator's own refusal
    let r = call("PUT", "/v1/players/nobody/ban", "root-secret", None).await;
    assert_eq!(
        (r.status, r.json()["error"].clone()),
        (404, json!("no_such_player"))
    );
}

// The web console's token acts for the sysop it names, audited as web:<name>; without a
// name, or with one that is not a name, it is refused, and the header means nothing with
// any other token.
#[tokio::test]
async fn the_console_token_acts_for_the_named_sysop() {
    let srv = server().await;
    let call = |token: &'static str, acting: Option<&'static str>| {
        let addr = srv.addr;
        async move {
            let extra = acting.map_or(String::new(), |n| format!("X-Silvermere-Sysop: {n}\r\n"));
            let req = format!(
                "POST /v1/players/Lars/tell HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\
                 Authorization: Bearer {token}\r\n{extra}Content-Type: application/json\r\n\
                 Content-Length: 13\r\n\r\n{{\"text\":\"hi\"}}"
            );
            let mut s = TcpStream::connect(addr).await.unwrap();
            s.write_all(req.as_bytes()).await.unwrap();
            let mut raw = String::new();
            s.read_to_string(&mut raw).await.unwrap();
            raw.split(' ').nth(1).unwrap().parse::<u16>().unwrap()
        }
    };
    assert_eq!(call("console-secret", Some("Boss")).await, 200);
    assert_eq!(call("console-secret", None).await, 401);
    assert_eq!(call("console-secret", Some("a b")).await, 401);
    assert_eq!(call("write-secret", Some("Boss")).await, 200);
    let by: Vec<String> = srv
        .moderator
        .actions
        .lock()
        .unwrap()
        .iter()
        .map(|(_, by)| by.clone())
        .collect();
    assert_eq!(by, ["web:Boss", "writer"]);
}

// POST /v1/shutdown: the defaults, the limits, and the audit row; DELETE cancels.
#[tokio::test]
async fn shutdown_schedules_checks_its_body_and_cancels() {
    let srv = server().await;
    let post = |body: &'static str| {
        request(
            srv.addr,
            "POST",
            "/v1/shutdown",
            Some("root-secret"),
            Some(body),
        )
    };
    let r = post("").await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["in_secs"], 300, "five minutes when none is named");
    assert_eq!(r.json()["at"], "2026-11-18T20:05:00Z");
    let r = post(r#"{"in_secs":30,"message":"  upgrading  "}"#).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(
        srv.shutdown.plans.lock().unwrap().last().cloned(),
        Some((30, "upgrading".to_string()))
    );
    for (body, code) in [
        (r#"{"in_secs":86401}"#, "bad_delay"),
        (r#"{"message":"a\u001b[2Jb"}"#, "bad_text"),
        ("not json", "bad_body"),
    ] {
        let r = post(body).await;
        assert_eq!(
            (r.status, r.json()["error"].clone()),
            (400, json!(code)),
            "{body}"
        );
    }
    assert_eq!(
        srv.shutdown.plans.lock().unwrap().len(),
        2,
        "refusals plan nothing"
    );
    let r = request(
        srv.addr,
        "DELETE",
        "/v1/shutdown",
        Some("root-secret"),
        None,
    )
    .await;
    assert_eq!(r.json()["cancelled"], true);
    // a write token cannot stop the server
    let r = request(
        srv.addr,
        "POST",
        "/v1/shutdown",
        Some("write-secret"),
        Some(""),
    )
    .await;
    assert_eq!(r.status, 403);
    let audit = srv.state.log.since(0, 100).await.unwrap();
    let rows: Vec<_> = audit
        .iter()
        .filter(|e| e.kind == "admin.shutdown")
        .collect();
    assert!(
        rows.iter()
            .any(|e| e.payload["in_secs"] == 30 && e.payload["text"] == "upgrading"),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|e| e.payload["cancelled"] == true),
        "{rows:?}"
    );
}

#[tokio::test]
async fn unauthorized_carries_the_challenge_header() {
    let srv = server().await;
    let r = request(srv.addr, "GET", "/v1/who", None, None).await;
    assert!(
        r.headers
            .to_lowercase()
            .contains("www-authenticate: bearer")
    );
    assert_eq!(r.json()["error"], "unauthorized");
}

// ---- SSE over a socket ---------------------------------------------------------------

async fn open_events(srv: &Server, query: &str, last_id: Option<i64>) -> TcpStream {
    let last = last_id.map_or(String::new(), |i| format!("Last-Event-ID: {i}\r\n"));
    let req = format!(
        "GET /v1/events{query} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer read-secret\r\n{last}\r\n"
    );
    let mut s = TcpStream::connect(srv.addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    s
}

async fn read_until(s: &mut TcpStream, buf: &mut String, needle: &str) {
    let mut chunk = [0u8; 4096];
    while !buf.contains(needle) {
        let n = tokio::time::timeout(Duration::from_secs(30), s.read(&mut chunk))
            .await
            .unwrap_or_else(|_| panic!("no {needle:?} in {buf:?}"))
            .unwrap();
        assert!(n > 0, "closed before {needle:?}: {buf:?}");
        buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }
}

async fn publish(srv: &Server, kind: &str) {
    let rec = srv
        .state
        .log
        .append(NewEvent::new(kind, json!({ "k": kind })))
        .await
        .unwrap();
    let _ = srv.state.events.send(rec);
}

#[tokio::test]
async fn kinds_filter_applies_live_and_on_replay() {
    let srv = server().await;
    publish(&srv, "closed").await; // id 1
    publish(&srv, "death").await; // id 2
    publish(&srv, "auth.lockout").await; // id 3
    let mut s = open_events(&srv, "?kinds=death,auth", Some(0)).await;
    let mut buf = String::new();
    read_until(&mut s, &mut buf, "event: auth.lockout").await;
    assert!(buf.contains("event: death"), "{buf}");
    assert!(!buf.contains("event: closed"), "{buf}");
    publish(&srv, "connected").await; // filtered out live
    publish(&srv, "level").await; // not in the filter either
    publish(&srv, "death").await; // id 6 passes
    // The audit row of the stream request itself takes an id too, so count deaths.
    while buf.matches("event: death").count() < 2 {
        let mut chunk = [0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(30), s.read(&mut chunk))
            .await
            .expect("second death frame")
            .unwrap();
        assert!(n > 0);
        buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }
    assert!(
        !buf.contains("event: connected") && !buf.contains("event: level"),
        "{buf}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_keepalive_comment_arrives_every_15_seconds() {
    let srv = server().await;
    let mut s = open_events(&srv, "", None).await;
    let mut buf = String::new();
    let t0 = tokio::time::Instant::now();
    read_until(&mut s, &mut buf, ": keepalive\n").await;
    let waited = t0.elapsed();
    assert!(
        (Duration::from_secs(15)..Duration::from_secs(16)).contains(&waited),
        "{waited:?}"
    );
}

// ---- the token never reaches a log ---------------------------------------------------

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}

/// One subscriber for the whole test binary, installed before any server starts, so every
/// callsite registers against it and every test's log lines land in the sink.
fn log_sink() -> Sink {
    static SINK: std::sync::OnceLock<Sink> = std::sync::OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Sink::default();
        let sub = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(sink.clone())
            .finish();
        let _ = tracing::subscriber::set_global_default(sub);
        sink
    })
    .clone()
}

#[tokio::test]
async fn no_token_or_authorization_header_reaches_the_log() {
    let sink = log_sink();
    let srv = server().await;
    for token in [
        "read-secret",
        "write-secret",
        "root-secret",
        "sm_wrongwrongwrong",
    ] {
        request(srv.addr, "GET", "/v1/who", Some(token), None).await;
        request(srv.addr, "POST", "/v1/reload-world", Some(token), None).await;
        request(
            srv.addr,
            "POST",
            "/v1/broadcast",
            Some(token),
            Some(r#"{"text":"x"}"#),
        )
        .await;
        request(srv.addr, "GET", "/v1/world/rooms", Some(token), None).await;
    }
    *srv.world.fail.lock().unwrap() = Some("boom".into());
    request(
        srv.addr,
        "POST",
        "/v1/reload-world",
        Some("root-secret"),
        None,
    )
    .await;
    let log = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("audit"),
        "the audit lines were captured: {log:?}"
    );
    for needle in [
        "read-secret",
        "write-secret",
        "root-secret",
        "sm_wrongwrongwrong",
        "uthorization",
        "Bearer",
    ] {
        assert!(
            !log.contains(needle),
            "{needle:?} leaked into the log:\n{log}"
        );
    }
    // The token's name is what the audit trail carries.
    assert!(log.contains("reader"));
}

// ---- docs/admin-api.md against the code -------------------------------------------------

fn doc() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/admin-api.md");
    std::fs::read_to_string(path).expect("docs/admin-api.md")
}

/// The text between `<!-- name:start -->` and `<!-- name:end -->`.
fn region(doc: &str, name: &str) -> String {
    let start = format!("<!-- {name}:start -->");
    let end = format!("<!-- {name}:end -->");
    let a = doc
        .find(&start)
        .unwrap_or_else(|| panic!("{start} missing"))
        + start.len();
    let b = doc.find(&end).unwrap_or_else(|| panic!("{end} missing"));
    doc[a..b].trim().to_string()
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().trim_matches('`').to_string())
        .collect()
}

#[test]
fn the_built_routes_table_lists_exactly_the_built_routes() {
    let table = region(&doc(), "built-routes");
    let documented: BTreeSet<(String, String, String)> = table
        .lines()
        .skip(2)
        .map(|l| {
            let c = cells(l);
            (c[0].clone(), c[1].clone(), c[2].clone())
        })
        .collect();
    let expected: BTreeSet<_> = routes()
        .into_iter()
        .filter(|r| r.built)
        .map(|r| {
            (
                r.method.to_string(),
                r.path,
                r.scope.map_or("none", Scope::name).to_string(),
            )
        })
        .collect();
    assert_eq!(documented, expected);
}

#[test]
fn the_reserved_routes_table_lists_every_reserved_route() {
    let table = region(&doc(), "reserved-routes");
    let mut documented = BTreeSet::new();
    for l in table.lines().skip(2) {
        let c = cells(l);
        // | table | collection verbs | item verbs | schema table |
        let base = format!("/v1/world/{}", c[0]);
        for v in c[1].split(' ') {
            documented.insert((v.to_string(), base.clone()));
        }
        for v in c[2].split(' ') {
            documented.insert((v.to_string(), format!("{base}/{{id}}")));
        }
        assert_eq!(
            Some(c[3].as_str()),
            silvermere_api::reserved::schema_table(&c[0]),
            "schema table of {}",
            c[0]
        );
    }
    let expected: BTreeSet<_> = routes()
        .into_iter()
        .filter(|r| !r.built)
        .map(|r| (r.method.to_string(), r.path))
        .collect();
    assert_eq!(documented, expected);
}

#[test]
fn the_event_kinds_table_is_generated_from_event_kind_all() {
    let table = region(&doc(), "event-kinds");
    let generated = silvermere_api::events::kinds_table();
    assert_eq!(
        table, generated,
        "docs/admin-api.md event-kinds table is stale; replace it with:\n{generated}"
    );
    assert_eq!(EventKind::ALL.len(), generated.lines().count() - 2);
}
