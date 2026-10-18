//! The admin API against a running server: `serve_in_process` with a token configured, and
//! `serve` refusing an open bind with no token.

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use silvermere::config::{AdminCfg, Config, TokenCfg, TokenScope};
use silvermere_engine::{SeededRng, SystemClock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-secret";

async fn send(addr: std::net::SocketAddr, req: &str) -> TcpStream {
    let mut s = TcpStream::connect(addr).await.expect("connect admin");
    s.write_all(req.as_bytes()).await.unwrap();
    s
}

/// One request with `Connection: close`; returns the status code and the body.
async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let (ctype, len, body) = match body {
        Some(b) => ("Content-Type: application/json\r\n", b.len(), b),
        None => ("", 0, ""),
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}{ctype}Content-Length: {len}\r\n\r\n{body}"
    );
    let mut s = send(addr, &req).await;
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut raw))
        .await
        .expect("response within 5 s")
        .unwrap();
    let code = raw.split(' ').nth(1).unwrap().parse().unwrap();
    let body = raw
        .split_once("\r\n\r\n")
        .map_or("", |(_, b)| b)
        .to_string();
    (code, body)
}

/// Reads from `s` into `buf` until it contains `needle`.
async fn read_until(s: &mut TcpStream, buf: &mut String, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut chunk = [0u8; 4096];
    while !buf.contains(needle) {
        let left = deadline.saturating_duration_since(Instant::now());
        let n = tokio::time::timeout(left, s.read(&mut chunk))
            .await
            .unwrap_or_else(|_| panic!("no {needle:?} in {buf:?}"))
            .unwrap();
        assert!(n > 0, "stream closed before {needle:?}: {buf:?}");
        buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }
}

#[tokio::test]
async fn admin_api_answers_and_streams_events() {
    let (_t, url) = common::seeded(&[]).await;
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url.clone();
    cfg.admin = AdminCfg {
        tokens: vec![TokenCfg {
            name: "ops".into(),
            sha256: None,
            token: Some(TOKEN.into()),
            scopes: vec![TokenScope::Read, TokenScope::Write],
        }],
        ..Default::default()
    };
    let r = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .expect("starts");
    let admin = r.admin_addr.expect("admin listener bound");

    assert_eq!(http(admin, "GET", "/healthz", None, None).await.0, 200);
    let (code, metrics) = http(admin, "GET", "/metrics", None, None).await;
    assert_eq!(code, 200, "{metrics}");
    assert_eq!(http(admin, "GET", "/v1/who", None, None).await.0, 401);
    assert_eq!(
        http(admin, "GET", "/v1/who", Some(TOKEN), None).await.0,
        200
    );
    for text in [
        r#"{"text":"a\u001b[2Jb"}"#,
        r#"{"text":"a\nb"}"#,
        r#"{"text":"a\u007fb"}"#,
        r#"{"text":"a\u009bb"}"#,
    ] {
        let (code, body) = http(admin, "POST", "/v1/broadcast", Some(TOKEN), Some(text)).await;
        assert_eq!((code, body.contains("bad_text")), (400, true), "{text}");
    }
    let ok = http(
        admin,
        "POST",
        "/v1/broadcast",
        Some(TOKEN),
        Some(r#"{"text":"hello"}"#),
    )
    .await;
    assert_eq!(ok.0, 200, "{}", ok.1);

    // An SSE client; the audit row of a /v1/who call proves it is subscribed.
    let mut sse = send(
        admin,
        &format!("GET /v1/events HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\n\r\n"),
    )
    .await;
    let mut seen = String::new();
    read_until(&mut sse, &mut seen, "\r\n\r\n").await;
    for _ in 0..50 {
        http(admin, "GET", "/v1/who", Some(TOKEN), None).await;
        if tokio::time::timeout(
            Duration::from_millis(100),
            read_until(&mut sse, &mut seen, "admin.who"),
        )
        .await
        .is_ok()
        {
            break;
        }
    }
    assert!(seen.contains("admin.who"), "subscribed: {seen}");

    let _telnet = TcpStream::connect(r.telnet_addr).await.unwrap();
    read_until(&mut sse, &mut seen, "event: connected").await;
    read_until(&mut sse, &mut seen, "\n\n").await;
    let frame = seen.rsplit("id: ").next().unwrap();
    let id: i64 = frame.split_whitespace().next().unwrap().parse().unwrap();

    // The table holds the same row.
    let db = silvermere_db::Db::connect(&url).await.unwrap();
    let rows = db.events().since(0, 1000).await.unwrap();
    let row = rows
        .iter()
        .find(|e| e.event.kind == "connected")
        .expect("a connected row");
    assert_eq!(row.id, id, "SSE id and table id agree");
    assert!(
        rows.iter()
            .any(|e| e.event.kind == "admin.broadcast" && e.event.payload["status"] == 200),
        "the broadcast is audited"
    );
    db.close().await;

    // Stop must not hang on the open SSE connection.
    tokio::time::timeout(Duration::from_secs(5), r.stop())
        .await
        .expect("stops within 5 s with an SSE client attached")
        .expect("engine joins");
}

/// `serve` refuses a non-loopback admin bind with no token, with a message that says why.
#[test]
fn serve_refuses_open_admin_bind_without_tokens() {
    let dir = std::env::temp_dir().join(format!("silvermere-admin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(&dir)
        .args(["serve", "--admin-bind", "0.0.0.0:8080"]);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    let o = c.output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(
        err.contains("0.0.0.0:8080") && err.contains("[[admin.tokens]]"),
        "{err}"
    );
}
