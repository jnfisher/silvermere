//! `sysop`: the sysop's commands on a running server, over its admin API (T-018).

use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;

use super::api_client::{call, reach, token_for};
use crate::cli::SysopCmd;
use crate::config::{Config, TokenScope};

/// A reload reads every static table; the rest answer at once.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `action` against the server on `server.admin_bind`: exit 0 with the answer on
/// stdout, else 1 with one line on stderr.
pub fn run(cfg: &Config, action: &SysopCmd, json: bool) -> ExitCode {
    match request(cfg, action) {
        Ok(body) => {
            if json {
                println!("{}", body.trim());
            } else {
                print!("{}", render(action, &body));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The call for `action`; the body of a `200`.
fn request(cfg: &Config, action: &SysopCmd) -> Result<String, String> {
    let bind = &cfg.server.admin_bind;
    let addr = reach(bind).ok_or_else(|| format!("server.admin_bind `{bind}` is not host:port"))?;
    let (method, path, scope, body) = match action {
        SysopCmd::Who => ("GET", "/v1/who", TokenScope::Read, None),
        SysopCmd::Broadcast { message } => (
            "POST",
            "/v1/broadcast",
            TokenScope::Write,
            Some(serde_json::json!({ "text": message }).to_string()),
        ),
        SysopCmd::ReloadWorld => ("POST", "/v1/reload-world", TokenScope::Write, None),
    };
    let token = token_for(cfg, scope).ok_or_else(|| {
        format!(
            "the config has no admin API token with its secret for this ({} scope): set \
             SILVERMERE_ADMIN_TOKEN to the server's token",
            scope_name(scope)
        )
    })?;
    let (status, body) =
        call(addr, method, path, Some(&token), body.as_deref(), TIMEOUT).map_err(|e| {
            match e.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    format!("no server is running on {addr} (server.admin_bind)")
                }
                _ => format!("calling {method} {path} on {addr}: {e}"),
            }
        })?;
    if status == 200 {
        return Ok(body);
    }
    // the API's error body: {"error": code, "message": text}
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned());
    Err(match status {
        401 => format!("the server on {addr} does not know this token: {message}"),
        403 => format!("the token lacks the {} scope: {message}", scope_name(scope)),
        _ => format!("the server on {addr} answered {status}: {message}"),
    })
}

fn scope_name(scope: TokenScope) -> &'static str {
    match scope {
        TokenScope::Read => "read",
        TokenScope::Write => "write",
        TokenScope::Admin => "admin",
    }
}

/// The answer as text. An answer that is not the documented shape is printed as it came.
fn render(action: &SysopCmd, body: &str) -> String {
    let shown = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| match action {
            SysopCmd::Who => who(&v),
            SysopCmd::Broadcast { .. } => {
                let n = v.get("delivered")?.as_u64()?;
                Some(format!("Told {n} player{}.\n", plural(n)))
            }
            SysopCmd::ReloadWorld => reloaded(&v),
        });
    shown.unwrap_or_else(|| format!("{}\n", body.trim()))
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The user list as a table, in the server's order.
fn who(v: &Value) -> Option<String> {
    let players = v.get("players")?.as_array()?;
    if players.is_empty() {
        return Some("Nobody is in the game.\n".to_owned());
    }
    let text = |p: &Value, key: &str| p.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let rows: Vec<[String; 6]> = players
        .iter()
        .map(|p| {
            let mut name = format!("{} {}", text(p, "first_name"), text(p, "last_name"));
            name.truncate(name.trim_end().len());
            if p.get("hidden").and_then(Value::as_bool) == Some(true) {
                name.push_str(" (hidden)");
            }
            let idle = p.get("idle_secs").and_then(Value::as_u64).unwrap_or(0);
            [
                name,
                text(p, "class"),
                text(p, "race"),
                p.get("level")
                    .and_then(Value::as_u64)
                    .map_or(String::new(), |l| l.to_string()),
                text(p, "gang"),
                format!("{}:{:02}", idle / 60, idle % 60),
            ]
        })
        .collect();
    let head = ["Name", "Class", "Race", "Level", "Gang", "Idle"];
    let width: Vec<usize> = (0..head.len())
        .map(|i| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain([head[i].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String]| {
        let mut l = String::new();
        for (i, c) in cells.iter().enumerate() {
            l.push_str(&format!("{c:<w$}  ", w = width[i]));
        }
        format!("{}\n", l.trim_end())
    };
    let mut out = line(&head.map(str::to_owned));
    for r in &rows {
        out.push_str(&line(r));
    }
    let n = rows.len() as u64;
    out.push_str(&format!("{n} player{} in the game.\n", plural(n)));
    Some(out)
}

/// The rows read per table, and the time taken.
fn reloaded(v: &Value) -> Option<String> {
    let tables = v.get("reloaded")?.as_object()?;
    let ms = v.get("ms").and_then(Value::as_u64).unwrap_or(0);
    let counts: Vec<String> = tables
        .iter()
        .filter_map(|(name, n)| Some(format!("{} {}", n.as_u64()?, name.replace('_', " "))))
        .collect();
    Some(format!(
        "World reloaded in {ms} ms: {}.\n",
        counts.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_is_a_table() {
        let body = r#"{"players":[{"session_id":1,"first_name":"Abe","last_name":"Smith",
            "title":"Apprentice","class":"Warrior","race":"Human","level":3,"alignment":-5,
            "gang":"","idle_secs":67,"hidden":false},
            {"session_id":2,"first_name":"Zed","last_name":"","class":"Mage","race":"Elf",
            "level":12,"gang":"Old Guard","idle_secs":5,"hidden":true}]}"#;
        assert_eq!(
            render(&SysopCmd::Who, body),
            "Name          Class    Race   Level  Gang       Idle\n\
             Abe Smith     Warrior  Human  3                 1:07\n\
             Zed (hidden)  Mage     Elf    12     Old Guard  0:05\n\
             2 players in the game.\n"
        );
        assert_eq!(
            render(&SysopCmd::Who, r#"{"players":[]}"#),
            "Nobody is in the game.\n"
        );
    }

    #[test]
    fn broadcast_and_reload_say_what_happened() {
        let b = SysopCmd::Broadcast {
            message: "x".into(),
        };
        assert_eq!(render(&b, r#"{"delivered":1}"#), "Told 1 player.\n");
        assert_eq!(render(&b, r#"{"delivered":4}"#), "Told 4 players.\n");
        assert_eq!(
            render(
                &SysopCmd::ReloadWorld,
                r#"{"reloaded":{"rooms":131,"text_blocks":500},"ms":42}"#
            ),
            "World reloaded in 42 ms: 131 rooms, 500 text blocks.\n"
        );
    }

    #[test]
    fn an_unexpected_answer_is_printed_as_it_came() {
        assert_eq!(render(&SysopCmd::Who, "{\"x\":1}\n"), "{\"x\":1}\n");
    }
}
