//! The built-in web pages: documentation, FAQ and the settings editor.
//!
//! One axum router, served on `server.web_bind`: the repository's `docs/` rendered as
//! pages (embedded at build time, so a binary serves the documentation of its own
//! commit), a FAQ, and an editor for the settings file that a sysop signs in to with his
//! game account. [`export`] writes the open pages as static files for a public copy.
//!
//! The crate knows neither the database nor the settings file: the binary signs sysops in
//! through [`SysopLogin`] and reads and writes settings through [`Settings`].
//!
//! `/play` and `/ws` are kept for playing in the browser, which is not built: this
//! listener is the one players would reach.

#![forbid(unsafe_code)]

pub mod console;
pub mod export;
pub mod page;
pub mod session;
pub mod settings;
pub mod site;

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

pub use console::{Console, LineOutcome};
pub use export::export;
pub use session::{LoginOutcome, Sessions, SysopLogin};
pub use settings::{Change, Kind, Lock, SettingValue, Settings, SettingsView, Source};
pub use site::Site;

use page::Chrome;
use session::Session;

/// What the pages are served from.
pub struct WebState {
    /// The documentation, rendered.
    pub site: Site,
    /// The server's version, shown in the footer.
    pub version: String,
    /// The settings file; `None` serves the reference only.
    pub settings: Option<Arc<dyn Settings>>,
    /// The sign-in check; `None` refuses every sign-in.
    pub login: Option<Arc<dyn SysopLogin>>,
    /// Who is signed in.
    pub sessions: Sessions,
    /// The host names the pages answer to (the `Host` header without its port, lower
    /// case); `None` answers to any (a public copy, the tests). Anything else is refused,
    /// so a web page elsewhere cannot reach these pages through a name it controls (DNS
    /// rebinding).
    pub hosts: Option<Vec<String>>,
    /// The admin API for the console; `None`: the console says it is not available.
    pub console: Option<Arc<dyn Console>>,
    /// Lines the console shows on top for the sysop (the admin API reachable from other
    /// computers, say).
    pub notices: Vec<String>,
}

impl WebState {
    /// The pages with no settings file and no sign-in behind them.
    pub fn open(version: &str) -> Self {
        WebState {
            site: Site::build(""),
            version: version.to_owned(),
            settings: None,
            login: None,
            sessions: Sessions::default(),
            hosts: None,
            console: None,
            notices: Vec::new(),
        }
    }
}

type Shared = Arc<WebState>;

/// The routes.
pub fn router(state: WebState) -> Router {
    let router = Router::new()
        .route("/", get(home))
        .route("/docs", get(docs_index))
        .route("/docs/{*slug}", get(doc))
        .route("/faq", get(faq))
        .route("/settings", get(settings_page).post(settings_save))
        .route("/settings/apply", axum::routing::post(settings_apply))
        .route("/console", get(console_page))
        .route("/console/api/{*path}", axum::routing::any(console_api))
        .route("/console/line", axum::routing::post(console_line))
        .route("/console/password", axum::routing::post(console_password))
        .route("/assets/console.js", get(console_script))
        .route("/login", get(login_page).post(login))
        .route("/logout", axum::routing::post(logout))
        .route("/assets/style.css", get(style))
        .route("/assets/app.js", get(script))
        .route("/assets/{*path}", get(image))
        // kept for playing in the browser
        .route("/play", get(not_built))
        .route("/ws", get(not_built))
        .fallback(not_found);
    let state = Arc::new(state);
    router
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// The host part of a `Host` header, lower case: `[::1]:8088` is `::1`.
fn host_name(host: &str) -> String {
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':')
            .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
            .map_or(host, |(h, _)| h)
    };
    name.to_ascii_lowercase()
}

/// Every request: the `Host` must be one the pages answer to, a request that changes
/// something must come from these pages (`Origin`, `Sec-Fetch-Site`), and a signed-in
/// session is checked against the account (still a sysop, same password) and its cookie
/// renewed, which keeps it from going idle.
async fn guard(
    State(state): State<Shared>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let headers = req.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(allowed) = &state.hosts
        && !allowed.contains(&host_name(host))
    {
        tracing::warn!(
            host,
            "web request for a host name these pages do not answer to; refused (server.web_hosts)"
        );
        return (
            StatusCode::MISDIRECTED_REQUEST,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!(
                "This server does not answer to the name {:?}. Add it to server.web_hosts.\n",
                host_name(host)
            ),
        )
            .into_response();
    }
    let unsafe_method = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    if unsafe_method {
        let origin_ok = match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            None => true,
            Some(origin) => origin
                .split_once("://")
                .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(host)),
        };
        let site_ok = headers
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
            .is_none_or(|s| s == "same-origin" || s == "none");
        if !origin_ok || !site_ok {
            tracing::warn!(host, "web request from another site refused");
            return (
                StatusCode::FORBIDDEN,
                "Refused: this request came from another site.\n",
            )
                .into_response();
        }
    }
    let assets = req
        .uri()
        .path()
        .starts_with(&format!("{}/assets/", state.site.base));
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    let live = if assets {
        None
    } else {
        state.sessions.find(cookie)
    };
    // the console's own refreshes do not count as use: an open page still goes idle
    let polling = headers.contains_key("x-console-poll");
    let mut renew = None;
    let mut ended = false;
    if let Some(s) = live {
        let current = match &state.login {
            Some(check) => check.fingerprint(s.user.clone()).await,
            None => None,
        };
        if current.as_deref() == Some(s.fingerprint.as_str()) {
            if !polling {
                renew = Some(s.used_at(session::now()));
            }
        } else {
            tracing::info!(target: "audit", user = %s.user, "web session ended: the account changed");
            req.headers_mut().remove(header::COOKIE);
            ended = true;
        }
    }
    let mut resp = next.run(req).await;
    if !resp.headers().contains_key(header::SET_COOKIE) {
        let value = match (&renew, ended) {
            (Some(s), _) => Some(state.sessions.set_cookie(s)),
            (None, true) => Some(session::clear_cookie()),
            (None, false) => None,
        };
        if let Some(v) = value.and_then(|v| HeaderValue::from_str(&v).ok()) {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
    }
    resp
}

/// Serves the pages on `listener` until `shutdown` completes.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: WebState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

/// Nothing loads from elsewhere, no script but this site's files (none inline), and no
/// other site may frame the pages.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
    img-src 'self' data:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; \
    base-uri 'none'; form-action 'self'";

/// A page with the headers every page carries: nothing loads from elsewhere, and no other
/// site may frame it.
fn html(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "same-origin"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

fn asset(mime: &'static str, body: impl IntoResponse) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// A redirect after a form, with a cookie to set if any.
fn see_other(to: &str, cookie: Option<String>) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(to) {
        headers.insert(header::LOCATION, v);
    }
    if let Some(v) = cookie.and_then(|c| HeaderValue::from_str(&c).ok()) {
        headers.insert(header::SET_COOKIE, v);
    }
    (StatusCode::SEE_OTHER, headers).into_response()
}

/// The settings as they stand, in the order the documentation lists them (a key it does
/// not list comes last).
fn view_of(state: &WebState, settings: &dyn Settings) -> Result<SettingsView, String> {
    let mut view = settings.view()?;
    let place = |key: &str| {
        state
            .site
            .keys
            .iter()
            .position(|(k, _)| k == key)
            .unwrap_or(usize::MAX)
    };
    view.values.sort_by_key(|v| place(&v.key));
    Ok(view)
}

fn session(state: &WebState, headers: &HeaderMap) -> Option<Session> {
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    state.sessions.find(cookie)
}

fn chrome<'a>(state: &'a WebState, session: Option<&'a Session>) -> Chrome<'a> {
    Chrome {
        site: &state.site,
        version: &state.version,
        user: session.map(|s| (s.user.as_str(), s.csrf.as_str())),
        exported: false,
    }
}

async fn home(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let s = session(&state, &headers);
    html(StatusCode::OK, page::home(&chrome(&state, s.as_ref())))
}

async fn docs_index(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let s = session(&state, &headers);
    html(
        StatusCode::OK,
        page::docs_index(&chrome(&state, s.as_ref())),
    )
}

async fn doc(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let s = session(&state, &headers);
    let c = chrome(&state, s.as_ref());
    let slug = slug.trim_end_matches('/');
    match page::doc(&c, slug) {
        Some(body) => html(StatusCode::OK, body),
        None => html(
            StatusCode::NOT_FOUND,
            page::notice(&c, "Not found", "There is no such page."),
        ),
    }
}

async fn faq(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let s = session(&state, &headers);
    let c = chrome(&state, s.as_ref());
    match page::doc(&c, site::FAQ) {
        Some(body) => html(StatusCode::OK, body),
        None => html(
            StatusCode::NOT_FOUND,
            page::notice(&c, "Not found", "There is no FAQ in this build."),
        ),
    }
}

async fn not_found(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let s = session(&state, &headers);
    html(
        StatusCode::NOT_FOUND,
        page::notice(
            &chrome(&state, s.as_ref()),
            "Not found",
            "There is no such page.",
        ),
    )
}

async fn not_built(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let s = session(&state, &headers);
    html(
        StatusCode::NOT_IMPLEMENTED,
        page::notice(
            &chrome(&state, s.as_ref()),
            "Not built yet",
            "Playing in the browser is not part of this version. Connect with a telnet client.",
        ),
    )
}

async fn console_script() -> Response {
    asset("text/javascript; charset=utf-8", page::CONSOLE_SCRIPT)
}

/// A JSON error for the console's calls.
fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = format!(
        "{{\"error\":{},\"message\":{}}}",
        json_string(code),
        json_string(message)
    );
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// `s` as a JSON string.
fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

async fn console_page(State(state): State<Shared>, headers: HeaderMap) -> Response {
    let Some(s) = session(&state, &headers) else {
        return see_other(&format!("{}/login", state.site.base), None);
    };
    let c = chrome(&state, Some(&s));
    if state.console.is_none() {
        return html(
            StatusCode::SERVICE_UNAVAILABLE,
            page::notice(&c, "Console", "This server has no console."),
        );
    }
    html(StatusCode::OK, page::console(&c, &s.csrf, &state.notices))
}

/// The session of a console call, and for one that changes something its CSRF token.
fn console_session(
    state: &WebState,
    headers: &HeaderMap,
    changes: bool,
) -> Result<Session, Box<Response>> {
    let Some(s) = session(state, headers) else {
        return Err(Box::new(json_error(
            StatusCode::UNAUTHORIZED,
            "signed_out",
            "Your session has ended: sign in again.",
        )));
    };
    let token = headers.get("x-csrf-token").and_then(|v| v.to_str().ok());
    if changes && token != Some(s.csrf.as_str()) {
        return Err(Box::new(json_error(
            StatusCode::FORBIDDEN,
            "csrf",
            "This page is out of date: reload it.",
        )));
    }
    Ok(s)
}

/// `/console/api/v1/...` and `/console/api/healthz`: the admin API as the signed-in sysop.
async fn console_api(
    State(state): State<Shared>,
    method: axum::http::Method,
    headers: HeaderMap,
    Path(path): Path<String>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    body: axum::body::Bytes,
) -> Response {
    let changes = method != axum::http::Method::GET;
    let s = match console_session(&state, &headers, changes) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(console) = &state.console else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_console",
            "This server has no console.",
        );
    };
    if !(path.starts_with("v1/") || path == "healthz") || path.contains("..") {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such route");
    }
    let mut full = format!("/{path}");
    if let Some(q) = query {
        full.push('?');
        full.push_str(&q);
    }
    if console::needs_password(&method, &full) && !s.fresh(session::now()) {
        return console::password_wanted();
    }
    let last = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    console.call(&s.user, method, &full, body, last).await
}

#[derive(serde::Deserialize)]
struct LineBody {
    line: String,
}

/// `POST /console/line`: one command line, as `silvermere sysop` takes it.
async fn console_line(
    State(state): State<Shared>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<LineBody>,
) -> Response {
    let s = match console_session(&state, &headers, true) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(console) = &state.console else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_console",
            "This server has no console.",
        );
    };
    match console
        .line(&s.user, &body.line, s.fresh(session::now()))
        .await
    {
        LineOutcome::Text(text) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            format!("{{\"text\":{}}}", json_string(&text)),
        )
            .into_response(),
        LineOutcome::Error(e) => json_error(StatusCode::BAD_REQUEST, "failed", &e),
        LineOutcome::NeedsPassword => console::password_wanted(),
    }
}

#[derive(serde::Deserialize)]
struct PasswordBody {
    password: String,
}

/// `POST /console/password`: the password given again, for the actions that want it.
async fn console_password(
    State(state): State<Shared>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<PasswordBody>,
) -> Response {
    let s = match console_session(&state, &headers, true) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(check) = &state.login else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_login",
            "Signing in is not available.",
        );
    };
    match check.login(s.user.clone(), body.password).await {
        LoginOutcome::Sysop(name) if name == s.user => {
            tracing::info!(target: "audit", user = %s.user, "web console: password given again");
            let renewed = s.reauthenticated(session::now());
            let mut resp = (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"ok\":true}",
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&state.sessions.set_cookie(&renewed)) {
                resp.headers_mut().insert(header::SET_COOKIE, v);
            }
            resp
        }
        LoginOutcome::Sysop(_) => json_error(
            StatusCode::UNAUTHORIZED,
            "wrong_password",
            "That is not your password.",
        ),
        LoginOutcome::Refused(why) => json_error(StatusCode::UNAUTHORIZED, "wrong_password", &why),
    }
}

async fn style() -> Response {
    asset("text/css; charset=utf-8", page::STYLE)
}

async fn script() -> Response {
    asset("text/javascript; charset=utf-8", page::SCRIPT)
}

async fn image(Path(path): Path<String>) -> Response {
    match site::image(&path) {
        Some((bytes, mime)) => asset(mime, bytes),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize)]
struct Saved {
    saved: Option<usize>,
    applied: Option<usize>,
}

async fn settings_page(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<Saved>,
) -> Response {
    let s = session(&state, &headers);
    let c = chrome(&state, s.as_ref());
    let Some(settings) = &state.settings else {
        return html(StatusCode::OK, page::settings_reference(&c));
    };
    if s.is_none() {
        return see_other(&format!("{}/login", state.site.base), None);
    }
    let banner = match (q.saved, q.applied) {
        (_, Some(0)) => {
            Some("Nothing new to load: the running game has every saved game option.".to_owned())
        }
        (_, Some(1)) => Some("Loaded 1 setting into the running game.".to_owned()),
        (_, Some(n)) => Some(format!("Loaded {n} settings into the running game.")),
        (Some(0), None) => Some("Nothing changed.".to_owned()),
        (Some(1), None) => Some("Saved 1 setting.".to_owned()),
        (Some(n), None) => Some(format!("Saved {n} settings.")),
        (None, None) => None,
    };
    match view_of(&state, settings.as_ref()) {
        Ok(view) => html(
            StatusCode::OK,
            page::settings(&c, &view, banner.as_deref().map(|b| (b, false))),
        ),
        Err(e) => html(
            StatusCode::INTERNAL_SERVER_ERROR,
            page::notice(&c, "The settings cannot be read", &e),
        ),
    }
}

async fn settings_save(
    State(state): State<Shared>,
    headers: HeaderMap,
    Form(posted): Form<Vec<(String, String)>>,
) -> Response {
    let Some(s) = session(&state, &headers) else {
        return see_other(&format!("{}/login", state.site.base), None);
    };
    let c = chrome(&state, Some(&s));
    let field = |name: &str| {
        posted
            .iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v.as_str())
    };
    if field("csrf") != s.csrf {
        return html(
            StatusCode::FORBIDDEN,
            page::notice(
                &c,
                "Not saved",
                "This form is out of date. Open the settings again.",
            ),
        );
    }
    let Some(settings) = &state.settings else {
        return html(
            StatusCode::NOT_FOUND,
            page::notice(
                &c,
                "Not saved",
                "This server has no settings file to write.",
            ),
        );
    };
    let section = field("section").to_owned();
    let view = match view_of(&state, settings.as_ref()) {
        Ok(v) => v,
        Err(e) => {
            return html(
                StatusCode::INTERNAL_SERVER_ERROR,
                page::notice(&c, "The settings cannot be read", &e),
            );
        }
    };
    let changes = settings::changes(&view, &section, &posted);
    if changes.is_empty() {
        return see_other(
            &format!("{}/settings?saved=0#{section}", state.site.base),
            None,
        );
    }
    let keys: Vec<&str> = changes.iter().map(|c| c.key.as_str()).collect();
    match settings.apply(&changes) {
        Ok(()) => {
            tracing::info!(target: "audit", user = %s.user, ?keys, "settings changed on the web");
            see_other(
                &format!(
                    "{}/settings?saved={}#{section}",
                    state.site.base,
                    changes.len()
                ),
                None,
            )
        }
        Err(e) => {
            tracing::info!(target: "audit", user = %s.user, ?keys, error = %e, "settings change refused");
            html(
                StatusCode::BAD_REQUEST,
                page::settings(&c, &view, Some((&format!("Not saved: {e}"), true))),
            )
        }
    }
}

/// "Apply now": the saved game options into the running game.
async fn settings_apply(
    State(state): State<Shared>,
    headers: HeaderMap,
    Form(posted): Form<Vec<(String, String)>>,
) -> Response {
    let Some(s) = session(&state, &headers) else {
        return see_other(&format!("{}/login", state.site.base), None);
    };
    let c = chrome(&state, Some(&s));
    if !posted.iter().any(|(k, v)| k == "csrf" && *v == s.csrf) {
        return html(
            StatusCode::FORBIDDEN,
            page::notice(
                &c,
                "Not applied",
                "This form is out of date. Open the settings again.",
            ),
        );
    }
    let Some(settings) = &state.settings else {
        return html(
            StatusCode::NOT_FOUND,
            page::notice(&c, "Not applied", "This server has no settings file."),
        );
    };
    match settings.load_now() {
        Ok(keys) => {
            tracing::info!(target: "audit", user = %s.user, ?keys, "settings loaded into the running game from the web");
            see_other(
                &format!("{}/settings?applied={}", state.site.base, keys.len()),
                None,
            )
        }
        Err(e) => {
            tracing::info!(target: "audit", user = %s.user, error = %e, "settings load refused");
            let view = view_of(&state, settings.as_ref());
            match view {
                Ok(view) => html(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    page::settings(&c, &view, Some((&format!("Not applied: {e}"), true))),
                ),
                Err(e2) => html(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    page::notice(&c, "Not applied", &format!("{e} ({e2})")),
                ),
            }
        }
    }
}

async fn login_page(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if session(&state, &headers).is_some() {
        return see_other(&format!("{}/console", state.site.base), None);
    }
    html(StatusCode::OK, page::login(&chrome(&state, None), None, ""))
}

#[derive(serde::Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

async fn login(State(state): State<Shared>, Form(form): Form<Credentials>) -> Response {
    let c = chrome(&state, None);
    let Some(check) = &state.login else {
        return html(
            StatusCode::SERVICE_UNAVAILABLE,
            page::login(&c, Some("Signing in is not available on this server."), ""),
        );
    };
    let username = form.username.trim().to_owned();
    match check.login(username.clone(), form.password).await {
        LoginOutcome::Sysop(name) => {
            let Some(fp) = check.fingerprint(name.clone()).await else {
                return html(
                    StatusCode::UNAUTHORIZED,
                    page::login(&c, Some("That account cannot sign in here."), &username),
                );
            };
            let s = state.sessions.open(&name, &fp, session::now());
            see_other(
                &format!("{}/console", state.site.base),
                Some(state.sessions.set_cookie(&s)),
            )
        }
        LoginOutcome::Refused(why) => html(
            StatusCode::UNAUTHORIZED,
            page::login(&c, Some(&why), &username),
        ),
    }
}

async fn logout(
    State(state): State<Shared>,
    headers: HeaderMap,
    Form(posted): Form<Vec<(String, String)>>,
) -> Response {
    let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
    if let Some(s) = state.sessions.find(cookie)
        && posted.iter().any(|(k, v)| k == "csrf" && *v == s.csrf)
    {
        tracing::info!(target: "audit", user = %s.user, "web sign-out");
        state.sessions.close(&s);
    }
    see_other(
        &format!("{}/", state.site.base),
        Some(session::clear_cookie()),
    )
}

#[cfg(test)]
mod tests {
    use super::host_name;

    #[test]
    fn the_host_name_drops_the_port_and_brackets() {
        assert_eq!(host_name("127.0.0.1:8088"), "127.0.0.1");
        assert_eq!(host_name("LocalHost"), "localhost");
        assert_eq!(host_name("[::1]:8088"), "::1");
        assert_eq!(
            host_name("silvermere.example.org:443"),
            "silvermere.example.org"
        );
        assert_eq!(host_name(""), "");
    }
}
