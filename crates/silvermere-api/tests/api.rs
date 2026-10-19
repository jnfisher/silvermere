//! The admin API over a scripted engine double: a thread that answers `AdminRequest`s the
//! way the engine does, a fake `events` table and fake health.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use silvermere_api::{ApiState, EventLog, EventRecord, NewEvent, Scope, Token, metrics, router};
use silvermere_engine::{AdminRequest, EngineInput, SessionId, WhoEntry};
use tokio_stream::StreamExt;
use tower::ServiceExt;

use crate::common;
use common::{FakeHealth, FakeLog};

// ---- doubles ------------------------------------------------------------------------

fn row(id: u16, slot: usize, first: &str, hidden: bool) -> WhoEntry {
    WhoEntry {
        id: SessionId(id),
        listslot: slot,
        first_name: first.into(),
        last_name: "Smith".into(),
        title: "Apprentice".into(),
        class: "Warrior".into(),
        race: "Human".into(),
        level: 3,
        alignment: -5,
        gang: String::new(),
        idle_secs: 7,
        hidden,
    }
}

struct Harness {
    app: Router,
    state: ApiState,
    log: Arc<FakeLog>,
    health: Arc<FakeHealth>,
    /// Texts the fake engine was asked to broadcast.
    broadcasts: Arc<Mutex<Vec<String>>>,
    recorder: metrics_exporter_prometheus::PrometheusRecorder,
    // Keeps the engine channel open for tests that do not use it.
    _engine_tx: Sender<EngineInput>,
}

fn harness() -> Harness {
    let (tx, rx) = channel::<EngineInput>();
    let broadcasts = Arc::new(Mutex::new(Vec::new()));
    let seen = broadcasts.clone();
    std::thread::spawn(move || {
        while let Ok(input) = rx.recv() {
            match input {
                // Out of listslot order, and a hidden sysop, on purpose.
                EngineInput::Admin(AdminRequest::Who { reply }) => {
                    let _ = reply.send(vec![
                        row(4, 2, "Cara", false),
                        row(1, 0, "Abe", false),
                        row(9, 3, "Root", true),
                        row(2, 1, "Bea", false),
                    ]);
                }
                EngineInput::Admin(AdminRequest::Broadcast { text, reply }) => {
                    seen.lock().unwrap().push(text);
                    let _ = reply.send(3);
                }
                _ => {}
            }
        }
    });
    let log = Arc::new(FakeLog::default());
    let health = Arc::new(FakeHealth {
        alive: AtomicBool::new(true),
        persist: AtomicBool::new(true),
        tick: AtomicU64::new(42),
    });
    let (recorder, handle) = metrics::build();
    let state = ApiState {
        engine: tx.clone(),
        events: ApiState::event_bus(),
        log: log.clone(),
        health: health.clone(),
        metrics: handle,
        world: Arc::new(common::FakeWorld::default()),
        tokens: Arc::new(vec![
            Token::new("reader", "read-secret", &[Scope::Read]),
            Token::new("caster", "cast-secret", &[Scope::Write]),
            Token::new("root", "root-secret", &[Scope::Admin]),
        ]),
    };
    Harness {
        app: router(state.clone()),
        state,
        log,
        health,
        broadcasts,
        recorder,
        _engine_tx: tx,
    }
}

fn get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::get(path);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn audit_rows(h: &Harness) -> Vec<EventRecord> {
    let rows = h.log.0.lock().unwrap();
    rows.iter()
        .filter(|r| r.kind.starts_with("admin."))
        .cloned()
        .collect()
}

// ---- /healthz and /metrics --------------------------------------------------------------

#[tokio::test]
async fn healthz_reports_ok_with_the_documented_shape() {
    let h = harness();
    let (status, body) = call(&h.app, get("/healthz", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["engine_tick"], 42);
    assert_eq!(body["persist"], "healthy");
}

#[tokio::test]
async fn healthz_is_503_when_the_engine_is_dead_or_persistence_is_unhealthy() {
    let h = harness();
    h.health.alive.store(false, Ordering::SeqCst);
    let (status, body) = call(&h.app, get("/healthz", None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "unavailable");
    h.health.alive.store(true, Ordering::SeqCst);
    h.health.persist.store(false, Ordering::SeqCst);
    let (status, body) = call(&h.app, get("/healthz", None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["persist"], "unhealthy");
}

#[tokio::test]
async fn metrics_is_text_plain_and_names_the_sessions_gauge() {
    let h = harness();
    ::metrics::with_local_recorder(&h.recorder, || {
        ::metrics::gauge!("silvermere_sessions").set(3.0);
        ::metrics::histogram!("silvermere_tick_seconds").record(0.002);
    });
    let resp = h.app.clone().oneshot(get("/metrics", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers()[header::CONTENT_TYPE].to_str().unwrap();
    assert!(ct.starts_with("text/plain"), "{ct}");
    let text = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("silvermere_sessions 3"), "{text}");
    assert!(text.contains("silvermere_tick_seconds_bucket"), "{text}");
}

// ---- /v1/who ------------------------------------------------------------------------------

#[tokio::test]
async fn who_needs_a_token() {
    let h = harness();
    let (status, _) = call(&h.app, get("/v1/who", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&h.app, get("/v1/who", Some("wrong"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        audit_rows(&h).is_empty(),
        "unauthenticated calls are not audited"
    );
}

#[tokio::test]
async fn who_lists_in_listslot_order_and_hides_the_sysop_from_read() {
    let h = harness();
    let (status, body) = call(&h.app, get("/v1/who", Some("read-secret"))).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<_> = body["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["first_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Abe", "Bea", "Cara"]);
    let first = &body["players"][0];
    for key in [
        "first_name",
        "last_name",
        "title",
        "class",
        "race",
        "level",
        "alignment",
        "gang",
        "session_id",
        "idle_secs",
    ] {
        assert!(first.get(key).is_some(), "missing {key}");
    }
    assert_eq!(first["session_id"], 1);
    assert_eq!(first["idle_secs"], 7);

    let (status, body) = call(&h.app, get("/v1/who", Some("root-secret"))).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<_> = body["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["first_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Abe", "Bea", "Cara", "Root"]);
}

#[tokio::test]
async fn who_is_503_when_the_engine_is_gone() {
    let h = harness();
    // A router whose engine channel has no receiver.
    let (tx, rx) = channel::<EngineInput>();
    drop(rx);
    let mut state = h.state.clone();
    state.engine = tx;
    let (status, body) = call(&router(state), get("/v1/who", Some("read-secret"))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "engine_unavailable");
}

// ---- /v1/broadcast ------------------------------------------------------------------------

fn post_broadcast(token: &str, text: &str) -> Request<Body> {
    Request::post("/v1/broadcast")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "text": text }).to_string()))
        .unwrap()
}

#[tokio::test]
async fn broadcast_needs_the_broadcast_scope() {
    let h = harness();
    let (status, _) = call(&h.app, post_broadcast("read-secret", "hi")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(h.broadcasts.lock().unwrap().is_empty());

    let (status, body) = call(&h.app, post_broadcast("cast-secret", "Reboot in 5")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "delivered": 3 }));
    assert_eq!(*h.broadcasts.lock().unwrap(), ["Reboot in 5"]);
}

#[tokio::test]
async fn broadcast_rejects_empty_text() {
    let h = harness();
    let (status, _) = call(&h.app, post_broadcast("cast-secret", "  ")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(h.broadcasts.lock().unwrap().is_empty());
}

// ---- reserved routes ----------------------------------------------------------------------

#[tokio::test]
async fn reserved_routes_answer_the_documented_501_to_an_admin_token() {
    let h = harness();
    for (method, path) in [
        ("GET", "/v1/world/rooms"),
        ("PUT", "/v1/world/rooms/12"),
        ("POST", "/v1/world/monsters"),
        ("DELETE", "/v1/world/texts/3"),
    ] {
        let req = |token: &str| {
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap()
        };
        let (status, body) = call(&h.app, req("root-secret")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {path}");
        assert_eq!(
            body,
            json!({
                "error": "reserved",
                "message": "world editing is not built yet",
                "docs": "docs/admin-api.md#reserved-routes"
            })
        );
        let (status, body) = call(&h.app, req("cast-secret")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(body["error"], "forbidden");
        assert!(body["message"].is_string());
    }
    let (status, body) = call(&h.app, get("/v1/world/rooms", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthorized");
    let (status, _) = call(&h.app, get("/v1/world/nonsense", Some("root-secret"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- audit --------------------------------------------------------------------------------

#[tokio::test]
async fn every_authenticated_call_writes_an_audit_row() {
    let h = harness();
    call(&h.app, get("/v1/who", Some("read-secret"))).await; // 200
    call(&h.app, get("/v1/who", None)).await; // 401: not audited
    call(&h.app, post_broadcast("read-secret", "no")).await; // 403
    call(&h.app, post_broadcast("cast-secret", "yes")).await; // 200
    call(&h.app, get("/v1/world/rooms", Some("root-secret"))).await; // 501
    let rows = audit_rows(&h);
    let kinds: Vec<_> = rows.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "admin.who",
            "admin.broadcast",
            "admin.broadcast",
            "admin.world.rooms"
        ]
    );
    assert_eq!(rows[0].payload["token"], "reader");
    assert_eq!(rows[1].payload["status"], 403);
    assert_eq!(rows[2].payload["text"], "yes");
    assert_eq!(rows[2].payload["delivered"], 3);
}

// ---- /v1/events ---------------------------------------------------------------------------

async fn publish(h: &Harness, kind: &str) {
    let rec = h.log.append(NewEvent::new(kind, json!({}))).await.unwrap();
    let _ = h.state.events.send(rec);
}

/// Reads SSE frames from `body` until `n` have arrived; each is `(id, event, data)`.
async fn frames(body: Body, n: usize) -> Vec<(String, String, Value)> {
    let mut stream = body.into_data_stream();
    let mut text = String::new();
    let mut out = Vec::new();
    while out.len() < n {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("frame within 5 s")
            .expect("stream open")
            .unwrap();
        text.push_str(std::str::from_utf8(&chunk).unwrap());
        while let Some(end) = text.find("\n\n") {
            let block: String = text.drain(..end + 2).collect();
            let (mut id, mut ev, mut data) = (String::new(), String::new(), String::new());
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("id:") {
                    id = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("event:") {
                    ev = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data = v.trim().to_string();
                }
            }
            if !id.is_empty() {
                out.push((id, ev, serde_json::from_str(&data).unwrap()));
            }
        }
    }
    out
}

#[tokio::test]
async fn events_streams_live_events_as_sse_frames() {
    let h = harness();
    let resp = h
        .app
        .clone()
        .oneshot(get("/v1/events", Some("read-secret")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/event-stream");
    publish(&h, "login").await;
    publish(&h, "death").await;
    // The first frame is the audit row of this very call.
    let got = frames(resp.into_body(), 3).await;
    let kinds: Vec<_> = got.iter().map(|f| f.1.as_str()).collect();
    assert_eq!(kinds, ["admin.events", "login", "death"]);
    assert_eq!(got[1].2["kind"], "login");
    for key in [
        "id",
        "at",
        "kind",
        "session",
        "character",
        "room",
        "payload",
    ] {
        assert!(got[1].2.get(key).is_some(), "missing {key}");
    }
}

#[tokio::test]
async fn last_event_id_replays_from_the_events_table() {
    let h = harness();
    for kind in ["login", "death", "level"] {
        h.log.append(NewEvent::new(kind, json!({}))).await.unwrap();
    }
    let req = Request::get("/v1/events")
        .header(header::AUTHORIZATION, "Bearer read-secret")
        .header("Last-Event-ID", "1")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let got = frames(resp.into_body(), 2).await;
    assert_eq!(
        got.iter()
            .map(|f| (f.0.as_str(), f.1.as_str()))
            .collect::<Vec<_>>(),
        [("2", "death"), ("3", "level")]
    );
}

#[tokio::test]
async fn events_needs_the_read_scope() {
    let h = harness();
    let (status, _) = call(&h.app, get("/v1/events", Some("cast-secret"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
