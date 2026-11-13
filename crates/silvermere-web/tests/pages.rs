//! The pages over HTTP: what is open, what needs a sysop, and what a save writes.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use silvermere_web::{
    Change, Kind, LoginOutcome, SettingValue, Settings, SettingsView, Source, SysopLogin, WebState,
    router,
};
use tower::ServiceExt as _;

/// A settings file in memory: one editable number, one secret.
#[derive(Default)]
struct Fake {
    applied: Mutex<Vec<Change>>,
}

impl Settings for Fake {
    fn view(&self) -> Result<SettingsView, String> {
        let value = |key: &str, value: &str, kind, locked: Option<&str>| SettingValue {
            key: key.into(),
            value: value.into(),
            default: value.into(),
            kind,
            source: Source::Default,
            locked: locked.map(str::to_owned),
        };
        Ok(SettingsView {
            file: "/data/silvermere.toml".into(),
            pending: false,
            values: vec![
                value("game.death_hp", "-200", Kind::Integer, None),
                value("database.url", "<redacted>", Kind::Text, Some("a secret")),
                value("server.max_sessions", "256", Kind::Integer, None),
            ],
        })
    }

    fn apply(&self, changes: &[Change]) -> Result<(), String> {
        if changes.iter().any(|c| c.value.as_deref() == Some("7")) {
            return Err("game.death_hp = 7 is out of range".into());
        }
        self.applied.lock().unwrap().extend_from_slice(changes);
        Ok(())
    }
}

struct OnlyBob;

impl SysopLogin for OnlyBob {
    fn login(
        &self,
        username: String,
        password: String,
    ) -> Pin<Box<dyn Future<Output = LoginOutcome> + Send + '_>> {
        Box::pin(async move {
            if username == "bob" && password == "right" {
                LoginOutcome::Sysop("Bob".into())
            } else {
                LoginOutcome::Refused("Unknown account or wrong password.".into())
            }
        })
    }
}

fn state(fake: Arc<Fake>) -> WebState {
    WebState {
        settings: Some(fake),
        login: Some(Arc::new(OnlyBob)),
        ..WebState::open("1.2.3")
    }
}

/// One request; the status, the headers and the body.
async fn call(app: &axum::Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, 8 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut r = Request::get(path);
    if let Some(c) = cookie {
        r = r.header(header::COOKIE, c);
    }
    r.body(Body::empty()).unwrap()
}

fn post(path: &str, cookie: Option<&str>, form: &str) -> Request<Body> {
    let mut r =
        Request::post(path).header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        r = r.header(header::COOKIE, c);
    }
    r.body(Body::from(form.to_owned())).unwrap()
}

/// Signs Bob in; the cookie and the token his forms carry.
async fn sign_in(app: &axum::Router) -> (String, String) {
    let (status, headers, _) = call(app, post("/login", None, "username=bob&password=right")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/settings");
    let set = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Strict"),
        "{set}"
    );
    let cookie = set.split(';').next().unwrap().to_owned();
    let (status, _, body) = call(app, get("/settings", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK);
    let csrf = body
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a csrf field")
        .to_owned();
    (cookie, csrf)
}

#[tokio::test]
async fn the_documentation_is_open_and_carries_the_logo() {
    let app = router(state(Arc::default()));
    let (status, headers, body) = call(&app, get("/", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("/assets/images/silvermere-logo.png"));
    assert!(body.contains("Silvermere 1.2.3"));
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("default-src 'self'")
    );
    let (status, _, body) = call(&app, get("/docs/install", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<h1 id="), "{}", &body[..200]);
    for path in ["/docs", "/faq", "/assets/style.css", "/assets/app.js"] {
        assert_eq!(
            call(&app, get(path, None)).await.0,
            StatusCode::OK,
            "{path}"
        );
    }
    let (status, headers, _) = call(&app, get("/assets/images/silvermere-logo.png", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        call(&app, get("/docs/nope", None)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, get("/nope", None)).await.0,
        StatusCode::NOT_FOUND
    );
    // kept for playing in the browser
    for path in ["/play", "/ws"] {
        assert_eq!(
            call(&app, get(path, None)).await.0,
            StatusCode::NOT_IMPLEMENTED
        );
    }
}

#[tokio::test]
async fn the_settings_need_a_sysop() {
    let app = router(state(Arc::default()));
    let (status, headers, _) = call(&app, get("/settings", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/login");
    let (status, _, _) = call(
        &app,
        post("/settings", None, "section=game&game.death_hp=1"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, headers, body) =
        call(&app, post("/login", None, "username=bob&password=wrong")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get(header::SET_COOKIE).is_none());
    assert!(body.contains("Unknown account or wrong password."));
    // a made-up cookie is nobody
    let (status, _, _) = call(&app, get("/settings", Some("silvermere_session=abc"))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn a_sysop_sees_and_saves_settings() {
    let fake = Arc::new(Fake::default());
    let app = router(state(fake.clone()));
    let (cookie, csrf) = sign_in(&app).await;
    let (_, _, body) = call(&app, get("/settings", Some(&cookie))).await;
    assert!(
        body.contains("name=\"game.death_hp\" value=\"-200\""),
        "the field"
    );
    assert!(body.contains("/data/silvermere.toml"));
    // the meaning comes from docs/config.md
    assert!(body.contains("Hit points at which a player dies."));
    assert!(body.contains("original option <code>DEATHHP</code>"));
    // a secret is shown locked
    assert!(body.contains(
        "name=\"database.url\" value=\"&lt;redacted&gt;\" spellcheck=\"false\" disabled"
    ));

    // without the form's token nothing is saved
    let form = "section=game&game.death_hp=-50";
    let (status, _, _) = call(&app, post("/settings", Some(&cookie), form)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(fake.applied.lock().unwrap().is_empty());

    // a change in the posted section is written; another section's field is not
    let form = format!("csrf={csrf}&section=game&game.death_hp=-50&server.max_sessions=9");
    let (status, headers, _) = call(&app, post("/settings", Some(&cookie), &form)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/settings?saved=1#game");
    assert_eq!(
        *fake.applied.lock().unwrap(),
        [Change {
            key: "game.death_hp".into(),
            value: Some("-50".into())
        }]
    );
    let (_, _, body) = call(&app, get("/settings?saved=1", Some(&cookie))).await;
    assert!(body.contains("Saved 1 setting. Restart the server to apply it."));

    // what the server would refuse at startup is shown, and nothing more is written
    let form = format!("csrf={csrf}&section=game&game.death_hp=7");
    let (status, _, body) = call(&app, post("/settings", Some(&cookie), &form)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Not saved: game.death_hp = 7 is out of range"));
    assert_eq!(fake.applied.lock().unwrap().len(), 1);

    // signed out, the cookie is nobody
    let (status, _, _) = call(
        &app,
        post("/logout", Some(&cookie), &format!("csrf={csrf}")),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, _, _) = call(&app, get("/settings", Some(&cookie))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn the_export_is_the_open_pages_as_files() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let n = silvermere_web::export(&dir, "/silvermere", "1.2.3").unwrap();
    assert!(n > 10, "{n} files");
    let read =
        |p: &str| std::fs::read_to_string(dir.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let home = read("index.html");
    assert!(home.contains("href=\"/silvermere/docs/install\""));
    assert!(!home.contains("Sign in"), "nobody signs in to a copy");
    assert!(read("docs/install.html").contains("/silvermere/assets/style.css"));
    assert!(read("settings.html").contains("server.telnet_bind"));
    assert!(read("faq.html").contains("<h1"));
    assert!(dir.join("assets/images/silvermere-logo.png").exists());
    assert!(dir.join("404.html").exists() && dir.join("docs/index.html").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
