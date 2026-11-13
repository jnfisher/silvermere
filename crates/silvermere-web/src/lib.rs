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

pub use export::export;
pub use session::{LoginOutcome, Sessions, SysopLogin};
pub use settings::{Change, Kind, SettingValue, Settings, SettingsView, Source};
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
        }
    }
}

type Shared = Arc<WebState>;

/// The routes.
pub fn router(state: WebState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/docs", get(docs_index))
        .route("/docs/{*slug}", get(doc))
        .route("/faq", get(faq))
        .route("/settings", get(settings_page).post(settings_save))
        .route("/login", get(login_page).post(login))
        .route("/logout", axum::routing::post(logout))
        .route("/assets/style.css", get(style))
        .route("/assets/app.js", get(script))
        .route("/assets/{*path}", get(image))
        // kept for playing in the browser
        .route("/play", get(not_built))
        .route("/ws", get(not_built))
        .fallback(not_found)
        .with_state(Arc::new(state))
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

/// A page with the headers every page carries: nothing loads from elsewhere, and no other
/// site may frame it.
fn html(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
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
    let banner = q.saved.map(|n| match n {
        0 => "Nothing changed.".to_owned(),
        1 => "Saved 1 setting. Restart the server to apply it.".to_owned(),
        n => format!("Saved {n} settings. Restart the server to apply them."),
    });
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

async fn login_page(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if session(&state, &headers).is_some() {
        return see_other(&format!("{}/settings", state.site.base), None);
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
            let cookie = state.sessions.open(&name);
            see_other(
                &format!("{}/settings", state.site.base),
                Some(session::set_cookie(&cookie)),
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
        state.sessions.close(cookie);
    }
    see_other(
        &format!("{}/", state.site.base),
        Some(session::clear_cookie()),
    )
}
