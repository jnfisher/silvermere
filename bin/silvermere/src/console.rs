//! The web console's way into the server ([`silvermere_web::Console`]): the admin API's
//! router, run in this process for the signed-in sysop.
//!
//! The console holds a token made at start and kept in memory only (`ApiState::console`):
//! each call carries it and the sysop's name, so the API's checks and audit rows apply as
//! they do to `silvermere sysop`, the rows naming `web:<sysop>`. A command line is parsed
//! as `silvermere sysop` parses its arguments, run as the same API call, and answered with
//! the same text ([`crate::commands::sysop`]).

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, header};
use axum::response::{IntoResponse, Response};
use clap::Parser as _;
use silvermere_web::LineOutcome;
use silvermere_web::console::{BoxFuture, needs_password};
use tower::ServiceExt as _;

use crate::cli::ConsoleLine;
use crate::commands::sysop::{error_message, render, route};

/// The admin router and the console's token.
pub struct ApiConsole {
    app: axum::Router,
    secret: String,
}

/// A fresh console token: 32 random bytes as hex. Never written anywhere.
pub fn new_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Splits a command line into words: spaces separate them; `"..."` and `'...'` keep spaces
/// in one, and inside double quotes `\"` and `\\` stand for `"` and `\`.
pub fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        None => return Err("a \" is not closed".into()),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => word.push(e),
                            Some(e) => {
                                word.push('\\');
                                word.push(e);
                            }
                            None => return Err("a \" is not closed".into()),
                        },
                        Some(c) => word.push(c),
                    }
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        None => return Err("a ' is not closed".into()),
                        Some('\'') => break,
                        Some(c) => word.push(c),
                    }
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        out.push(word);
    }
    Ok(out)
}

impl ApiConsole {
    /// The console over `app`, the admin router built with a state whose `console` token
    /// is `secret`.
    pub fn new(app: axum::Router, secret: String) -> Self {
        Self { app, secret }
    }

    async fn request(
        &self,
        sysop: &str,
        method: Method,
        path: &str,
        body: Bytes,
        last_event_id: Option<String>,
    ) -> Response {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.secret))
            .header(silvermere_api::ACTING_FOR, sysop)
            .header(header::HOST, "console");
        if !body.is_empty() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        if let Some(id) = last_event_id {
            req = req.header("last-event-id", id);
        }
        let Ok(req) = req.body(Body::from(body)) else {
            return axum::http::StatusCode::BAD_REQUEST.into_response();
        };
        match self.app.clone().oneshot(req).await {
            Ok(resp) => resp,
            Err(never) => match never {},
        }
    }

    async fn run_line(&self, sysop: &str, line: &str, fresh: bool) -> LineOutcome {
        let words = match words(line) {
            Ok(w) => w,
            Err(e) => return LineOutcome::Error(e),
        };
        let parsed = match ConsoleLine::try_parse_from(&words) {
            Ok(p) => p,
            Err(e) => {
                let text = e.render().to_string();
                return match e.kind() {
                    clap::error::ErrorKind::DisplayHelp
                    | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                        LineOutcome::Text(text)
                    }
                    _ => LineOutcome::Error(text.trim_end().to_owned()),
                };
            }
        };
        let (method, path, _, body) = route(&parsed.action);
        let method: Method = method.parse().unwrap_or(Method::GET);
        if needs_password(&method, &path) && !fresh {
            return LineOutcome::NeedsPassword;
        }
        let resp = self
            .request(
                sysop,
                method,
                &path,
                body.map(Bytes::from).unwrap_or_default(),
                None,
            )
            .await;
        let status = resp.status();
        let text = match axum::body::to_bytes(resp.into_body(), 1 << 20).await {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => return LineOutcome::Error(format!("reading the answer: {e}")),
        };
        if !status.is_success() {
            return LineOutcome::Error(error_message(&text));
        }
        LineOutcome::Text(if parsed.json {
            format!("{}\n", text.trim())
        } else {
            render(&parsed.action, &text)
        })
    }
}

impl silvermere_web::Console for ApiConsole {
    fn call(
        &self,
        sysop: &str,
        method: Method,
        path: &str,
        body: Bytes,
        last_event_id: Option<String>,
    ) -> BoxFuture<'_, Response> {
        let (sysop, path) = (sysop.to_owned(), path.to_owned());
        Box::pin(async move {
            self.request(&sysop, method, &path, body, last_event_id)
                .await
        })
    }

    fn line(&self, sysop: &str, line: &str, fresh: bool) -> BoxFuture<'_, LineOutcome> {
        let (sysop, line) = (sysop.to_owned(), line.to_owned());
        Box::pin(async move { self.run_line(&sysop, &line, fresh).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_like_a_shell() {
        let w = |s: &str| words(s).unwrap();
        assert_eq!(w("kick Lars"), ["kick", "Lars"]);
        assert_eq!(
            w(r#"jail  Lars --for 1h "fighting in town""#),
            ["jail", "Lars", "--for", "1h", "fighting in town"]
        );
        assert_eq!(
            w(r#"tell Lars "say \"hi\" \\ bye""#),
            ["tell", "Lars", r#"say "hi" \ bye"#]
        );
        assert_eq!(w("tell Lars 'it''s'"), ["tell", "Lars", "its"]);
        assert_eq!(w(r#"x a"b c"d"#), ["x", "ab cd"]);
        assert_eq!(w(r#"tell Lars """#), ["tell", "Lars", ""]);
        assert!(w("   ").is_empty());
        assert!(words(r#"tell "Lars"#).is_err());
    }

    #[test]
    fn a_line_parses_as_the_sysop_command_does() {
        let p = ConsoleLine::try_parse_from(words(r#"ban Lars --for 7d "spam" --json"#).unwrap())
            .unwrap();
        assert!(p.json);
        let (method, path, _, body) = route(&p.action);
        assert_eq!((method, path.as_str()), ("PUT", "/v1/players/Lars/ban"));
        assert_eq!(
            body.as_deref(),
            Some(r#"{"for_secs":604800,"reason":"spam"}"#)
        );
        let help = ConsoleLine::try_parse_from(["help"]).unwrap_err();
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        assert!(help.render().to_string().contains("kick"));
    }
}
