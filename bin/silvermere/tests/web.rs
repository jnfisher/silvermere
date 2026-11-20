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
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n{cookie}{ctype}Content-Length: {}\r\n\r\n{body}",
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
    assert_eq!((status, header(&head, "location")), (303, Some("/console")));
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
    assert!(!body.contains("Saved but"));
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
    assert!(body.contains("Saved 2 settings."));
    assert!(body.contains("name=\"game.max_top\" value=\"20\""));
    // game.max_top can be loaded now; game.open_signup only at a restart
    let live = body
        .split("Saved but not loaded yet:")
        .nth(1)
        .and_then(|rest| rest.split("</p>").next())
        .expect("the Apply now banner");
    assert!(
        live.contains("game.max_top") && !live.contains("open_signup"),
        "{live}"
    );
    let restart = body
        .split("Saved but read only when the server starts:")
        .nth(1)
        .and_then(|rest| rest.split("</p>").next())
        .expect("the restart banner");
    assert!(restart.contains("game.open_signup"), "{restart}");

    // Apply now: the running game takes game.max_top, as SYSOP RELOAD would
    let form = format!("csrf={csrf}");
    let (status, head, _) = http(web, "POST", "/settings/apply", Some(&cookie), Some(&form)).await;
    assert_eq!(
        (status, header(&head, "location")),
        (303, Some("/settings?applied=1"))
    );
    let (_, _, body) = http(web, "GET", "/settings?applied=1", Some(&cookie), None).await;
    assert!(body.contains("Loaded 1 setting into the running game."));
    assert!(!body.contains("Saved but not loaded yet"), "{body}");
    assert!(body.contains("Saved but read only when the server starts"));

    // back to the default: out of the file
    let form = format!("csrf={csrf}&section=game&game.max_top=10&game.open_signup=false");
    let (status, _, _) = http(web, "POST", "/settings", Some(&cookie), Some(&form)).await;
    assert_eq!(status, 303);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        !text.lines().any(|l| l.trim_start().starts_with("max_top")),
        "{text}"
    );

    let world = tokio::time::timeout(Duration::from_secs(10), r.stop())
        .await
        .expect("stops within 10 s")
        .expect("engine joins");
    // the applied value is in force; the startup-only one is not, nor the change saved
    // after the apply
    assert_eq!(world.config.max_top, 20);
    assert!(world.config.open_signup);
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

/// One console request: method, path, cookie, extra header lines, JSON body; the status,
/// the header block (lower case) and the body.
async fn console(
    addr: SocketAddr,
    method: &str,
    path: &str,
    cookie: &str,
    extra: &str,
    json: Option<&str>,
) -> (u16, String, String) {
    let body = json.unwrap_or("");
    let ctype = if json.is_some() {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nCookie: {cookie}\r\n{extra}{ctype}Content-Length: {}\r\n\r\n{body}",
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

/// The console: a command line runs as the signed-in sysop (audited as `web:<name>`), wants
/// the session's CSRF token, and wants the password given lately for what cannot be undone;
/// the pages answer only to their own host names and refuse requests from other sites; a
/// session ends when its account stops being a sysop.
#[tokio::test]
async fn the_console_acts_for_the_signed_in_sysop() {
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
        .join(format!("web-console-{}", std::process::id()));
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

    // another site's name, and another site's form, are refused
    let (status, _, body) = console(web, "GET", "/", "", "", None).await;
    assert_eq!(status, 200, "{body}");
    let raw = format!(
        "GET / HTTP/1.1\r\nHost: evil.example:{}\r\nConnection: close\r\n\r\n",
        web.port()
    );
    let mut s = TcpStream::connect(web).await.unwrap();
    s.write_all(raw.as_bytes()).await.unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).await.unwrap();
    assert!(answer.starts_with("HTTP/1.1 421"), "{answer}");
    assert!(answer.contains("server.web_hosts"), "{answer}");

    let (status, head, _) = http(
        web,
        "POST",
        "/login",
        None,
        Some("username=Wendy&password=wendy-pw1"),
    )
    .await;
    assert_eq!(status, 303);
    let cookie = header(&head, "set-cookie")
        .and_then(|c| c.split(';').next())
        .unwrap()
        .to_owned();
    let (status, _, page) = console(web, "GET", "/console", &cookie, "", None).await;
    assert_eq!(status, 200);
    let csrf = page
        .split("data-csrf=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_owned();
    let with_csrf = format!("X-CSRF-Token: {csrf}\r\n");

    // a command line, as the sysop
    let line = |l: &str| format!("{{\"line\":{}}}", serde_json::Value::from(l));
    let (status, _, body) = console(
        web,
        "POST",
        "/console/line",
        &cookie,
        "",
        Some(&line("mute Pete spam")),
    )
    .await;
    assert_eq!(
        (status, body.contains("csrf")),
        (403, true),
        "no token, no command"
    );
    let (status, _, body) = console(
        web,
        "POST",
        "/console/line",
        &cookie,
        &format!("{with_csrf}Origin: http://evil.example\r\n"),
        Some(&line("mute Pete spam")),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let (status, _, body) = console(
        web,
        "POST",
        "/console/line",
        &cookie,
        &with_csrf,
        Some(&line(r#"mute Pete "spamming gossip""#)),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("Muted Pete until lifted"), "{body}");
    let (status, _, body) =
        console(web, "GET", "/console/api/v1/moderation", &cookie, "", None).await;
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["sanctions"][0]["by"], "web:Wendy");
    assert_eq!(v["sanctions"][0]["reason"], "spamming gossip");
    let (status, _, body) = console(
        web,
        "POST",
        "/console/line",
        &cookie,
        &with_csrf,
        Some(&line("frobnicate")),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body.contains("unrecognized subcommand"), "{body}");
    let (_, _, body) = console(
        web,
        "POST",
        "/console/line",
        &cookie,
        &with_csrf,
        Some(&line("help")),
    )
    .await;
    assert!(body.contains("ban-ip"), "{body}");

    // a session whose password was given 15 minutes ago: a ban wants it again
    let db = t.db();
    let hash = db
        .accounts()
        .by_username("Wendy")
        .await
        .unwrap()
        .unwrap()
        .account
        .password_hash;
    let sessions = silvermere_web::Sessions::with_key(silvermere::web::session_key(&dir));
    let old = sessions.open(
        "Wendy",
        &silvermere::web::password_fingerprint(&hash),
        silvermere_web::session::now() - 900,
    );
    let stale = format!(
        "{}={}",
        silvermere_web::session::COOKIE,
        sessions.cookie_value(&old)
    );
    let stale_csrf = format!("X-CSRF-Token: {}\r\n", old.csrf);
    let (status, _, body) = console(
        web,
        "POST",
        "/console/line",
        &stale,
        &stale_csrf,
        Some(&line("ban Pete")),
    )
    .await;
    assert_eq!(status, 401);
    assert!(body.contains("\"password\""), "{body}");
    let (status, _, body) = console(
        web,
        "PUT",
        "/console/api/v1/players/Pete/ban",
        &stale,
        &stale_csrf,
        Some("{}"),
    )
    .await;
    assert_eq!(status, 401, "the API path wants it too: {body}");
    let (status, _, _) = console(
        web,
        "POST",
        "/console/password",
        &stale,
        &stale_csrf,
        Some(r#"{"password":"wrong"}"#),
    )
    .await;
    assert_eq!(status, 401);
    let (status, head, _) = console(
        web,
        "POST",
        "/console/password",
        &stale,
        &stale_csrf,
        Some(r#"{"password":"wendy-pw1"}"#),
    )
    .await;
    assert_eq!(status, 200);
    let fresh = header(&head, "set-cookie")
        .and_then(|c| c.split(';').next())
        .unwrap()
        .to_owned();
    let (status, _, body) = console(
        web,
        "POST",
        "/console/line",
        &fresh,
        &stale_csrf,
        Some(&line("ban Pete --for 1d")),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("Banned Pete until"), "{body}");

    // no longer a sysop: the session has ended
    silvermere_auth::cli::revoke_role(db, "Wendy", Role::Sysop)
        .await
        .unwrap();
    let (status, head, body) = console(web, "GET", "/console/api/v1/who", &cookie, "", None).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        header(&head, "set-cookie").is_some_and(|c| c.contains("max-age=0")),
        "{head}"
    );
    r.stop().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
