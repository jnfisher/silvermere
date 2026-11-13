//! The web pages against a running server: the documentation is open, a sysop signs in
//! with his game account, and a save writes the settings file with startup's checks.

use crate::common;

use std::net::SocketAddr;
use std::time::Duration;

use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_engine::{SeededRng, SystemClock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// One request with `Connection: close`: the status, the header block and the body.
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    form: Option<&str>,
) -> (u16, String, String) {
    let cookie = cookie.map_or(String::new(), |c| format!("Cookie: {c}\r\n"));
    let (ctype, body) = match form {
        Some(f) => ("Content-Type: application/x-www-form-urlencoded\r\n", f),
        None => ("", ""),
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{cookie}{ctype}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(addr).await.expect("connect web");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut raw))
        .await
        .expect("an answer within 10 s")
        .unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head.to_ascii_lowercase(), body.to_owned())
}

/// The value of the header `name` (lower case) in a header block.
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}: ")))
}

#[tokio::test]
async fn a_sysop_changes_a_setting_in_the_browser() {
    let (t, url) = common::seeded(&[]).await;
    create_with_cost(
        t.db(),
        "Wendy",
        "wendy-pw1",
        &[Role::Player, Role::Sysop],
        Cost::Low,
    )
    .await
    .unwrap();
    create_with_cost(t.db(), "Pete", "pete-pw12", &[Role::Player], Cost::Low)
        .await
        .unwrap();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("web-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = Config {
        data_dir: dir.clone(),
        ..Config::default()
    };
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.server.web_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    let r = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .expect("starts");
    let web = r.web_addr.expect("the web pages are on");

    // open to anyone: the home page with the logo, a documentation page, the FAQ
    let (status, _, body) = http(web, "GET", "/", None, None).await;
    assert_eq!(status, 200);
    assert!(body.contains("/assets/images/silvermere-logo.png"));
    assert!(body.contains(concat!("Silvermere ", env!("CARGO_PKG_VERSION"))));
    assert_eq!(http(web, "GET", "/docs/install", None, None).await.0, 200);
    assert_eq!(http(web, "GET", "/faq", None, None).await.0, 200);

    // the settings are the sysop's
    let (status, head, _) = http(web, "GET", "/settings", None, None).await;
    assert_eq!((status, header(&head, "location")), (303, Some("/login")));
    let (status, _, body) = http(
        web,
        "POST",
        "/login",
        None,
        Some("username=Wendy&password=wrong"),
    )
    .await;
    assert_eq!(status, 401);
    assert!(body.contains("Unknown account or wrong password."));
    let (status, _, body) = http(
        web,
        "POST",
        "/login",
        None,
        Some("username=Pete&password=pete-pw12"),
    )
    .await;
    assert_eq!(status, 401);
    assert!(body.contains("not a sysop"), "{body}");
    let (status, head, _) = http(
        web,
        "POST",
        "/login",
        None,
        Some("username=wendy&password=wendy-pw1"),
    )
    .await;
    assert_eq!(
        (status, header(&head, "location")),
        (303, Some("/settings"))
    );
    let cookie = header(&head, "set-cookie")
        .and_then(|c| c.split(';').next())
        .expect("a session cookie")
        .to_owned();

    // every setting, with the description from docs/config.md; secrets locked
    let (status, _, body) = http(web, "GET", "/settings", Some(&cookie), None).await;
    assert_eq!(status, 200);
    assert!(
        body.contains("name=\"game.max_top\" value=\"10\""),
        "the field"
    );
    assert!(body.contains("Hit points at which a player dies."));
    assert!(body.contains("id=\"f-database.url\"") && body.contains("Can hold a password"));
    assert!(!body.contains("has changes this server has not loaded"));
    let csrf = body
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a csrf field")
        .to_owned();

    // a value startup would refuse: its message, and no file
    let form = format!("csrf={csrf}&section=game&game.max_top=10&game.inactive_secs=5");
    let (status, _, body) = http(web, "POST", "/settings", Some(&cookie), Some(&form)).await;
    assert_eq!(status, 400);
    assert!(
        body.contains("Not saved:") && body.contains("game.inactive_secs"),
        "{body}"
    );
    let file = dir.join("silvermere.toml");
    assert!(!file.exists());

    // a good one is written to the file `config set` writes
    let form = format!("csrf={csrf}&section=game&game.max_top=20&game.open_signup=false");
    let (status, head, _) = http(web, "POST", "/settings", Some(&cookie), Some(&form)).await;
    assert_eq!(
        (status, header(&head, "location")),
        (303, Some("/settings?saved=2#game"))
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("max_top = 20") && text.contains("open_signup = false"),
        "{text}"
    );
    let (_, _, body) = http(web, "GET", "/settings?saved=2", Some(&cookie), None).await;
    assert!(body.contains("Saved 2 settings. Restart the server to apply them."));
    assert!(body.contains("name=\"game.max_top\" value=\"20\""));
    assert!(body.contains("has changes this server has not loaded"));

    // back to the default: out of the file
    let form = format!("csrf={csrf}&section=game&game.max_top=10&game.open_signup=false");
    let (status, _, _) = http(web, "POST", "/settings", Some(&cookie), Some(&form)).await;
    assert_eq!(status, 303);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        !text.lines().any(|l| l.trim_start().starts_with("max_top")),
        "{text}"
    );

    tokio::time::timeout(Duration::from_secs(10), r.stop())
        .await
        .expect("stops within 10 s")
        .expect("engine joins");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pages are on by default, so a taken address must not stop the game.
#[tokio::test]
async fn a_taken_web_address_does_not_stop_the_server() {
    let (_t, url) = common::seeded(&[]).await;
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.server.web_bind = taken.local_addr().unwrap().to_string();
    cfg.database.url = url;
    let r = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .expect("starts without the web pages");
    assert!(r.web_addr.is_none());
    r.stop().await.expect("engine joins");
}
