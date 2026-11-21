//! `sysop`: the sysop's commands on a running server, over its admin API (T-018).

use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;

use super::api_client::{call, reach, token_for};
use crate::cli::{RealmCmd, SanctionArgs, SysopCmd};
use crate::config::{Config, TokenScope};

/// A reload reads every static table; the rest answer at once.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The in-game line `game` runs: `SYSOP` and the words, unless the first word is `SYSOP`.
pub fn sysop_line(words: &[String]) -> String {
    let line = words.join(" ");
    match words.first() {
        Some(w) if w.eq_ignore_ascii_case("sysop") => line,
        _ => format!("SYSOP {line}"),
    }
}

/// Runs `action` against the server on `server.admin_bind`: exit 0 with the answer on
/// stdout, else 1 with one line on stderr.
pub fn run(cfg: &Config, action: &SysopCmd, json: bool) -> ExitCode {
    if let SysopCmd::Spectate { player, typing } = action {
        return match spectate(cfg, player, *typing) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
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

/// A player's name as one path segment: letters, digits and `-_.~` as they are, the rest
/// percent-encoded.
fn segment(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `PUT /v1/players/{name}/{kind}` for `mute`, `jail` and `ban`.
fn put(kind: &str, a: &SanctionArgs) -> (&'static str, String, TokenScope, Option<String>) {
    (
        "PUT",
        format!("/v1/players/{}/{kind}", segment(&a.player)),
        TokenScope::Admin,
        Some(
            serde_json::json!({
                "for_secs": a.for_secs,
                "reason": a.reason.clone().unwrap_or_default(),
            })
            .to_string(),
        ),
    )
}

fn lift(kind: &str, player: &str) -> (&'static str, String, TokenScope, Option<String>) {
    (
        "DELETE",
        format!("/v1/players/{}/{kind}", segment(player)),
        TokenScope::Admin,
        None,
    )
}

/// The call for `action`; the body of a `200`.
fn request(cfg: &Config, action: &SysopCmd) -> Result<String, String> {
    let bind = &cfg.server.admin_bind;
    let addr = reach(bind).ok_or_else(|| format!("server.admin_bind `{bind}` is not host:port"))?;
    let (method, path, scope, body) = route(action);
    let path = path.as_str();
    let token = token_for(cfg, scope).ok_or_else(|| {
        format!(
            "the config has no admin API token with its secret for this ({} scope): set \
             SILVERMERE_ADMIN_TOKEN to the server's token",
            scope_name(scope)
        )
    })?;
    send(addr, method, path, scope, &token, body)
}

/// The admin API call `action` is: method, path, the scope it needs, the JSON body. The
/// web console runs the same calls in the server ([`crate::console`]).
pub fn route(action: &SysopCmd) -> (&'static str, String, TokenScope, Option<String>) {
    match action {
        SysopCmd::Spectate { player, .. } => (
            "GET",
            format!("/v1/players/{}/screen", segment(player)),
            TokenScope::Admin,
            None,
        ),
        SysopCmd::Type { player, line } => (
            "POST",
            format!("/v1/players/{}/input", segment(player)),
            TokenScope::Admin,
            Some(serde_json::json!({ "line": line.join(" ") }).to_string()),
        ),
        SysopCmd::Game { words } => (
            "POST",
            "/v1/sysop".into(),
            TokenScope::Admin,
            Some(serde_json::json!({ "line": sysop_line(words) }).to_string()),
        ),
        SysopCmd::Restart { cancel: true, .. } => {
            ("DELETE", "/v1/restart".into(), TokenScope::Admin, None)
        }
        SysopCmd::Restart { delay, message, .. } => (
            "POST",
            "/v1/restart".into(),
            TokenScope::Admin,
            Some(
                serde_json::json!({
                    "in_secs": delay,
                    "message": message.clone().unwrap_or_default(),
                })
                .to_string(),
            ),
        ),
        SysopCmd::Kick { player, reason } => (
            "POST",
            format!("/v1/players/{}/kick", segment(player)),
            TokenScope::Admin,
            Some(serde_json::json!({ "reason": reason.clone().unwrap_or_default() }).to_string()),
        ),
        SysopCmd::Tell { player, message } => (
            "POST",
            format!("/v1/players/{}/tell", segment(player)),
            TokenScope::Write,
            Some(serde_json::json!({ "text": message }).to_string()),
        ),
        SysopCmd::Mute(a) => put("mute", a),
        SysopCmd::Jail(a) => put("jail", a),
        SysopCmd::Ban(a) => put("ban", a),
        SysopCmd::Unmute { player } => lift("mute", player),
        SysopCmd::Release { player } => lift("jail", player),
        SysopCmd::Unban { player } => lift("ban", player),
        SysopCmd::BanIp {
            network,
            for_secs,
            reason,
        } => (
            "POST",
            "/v1/ip-bans".into(),
            TokenScope::Admin,
            Some(
                serde_json::json!({
                    "network": network,
                    "for_secs": for_secs,
                    "reason": reason.clone().unwrap_or_default(),
                })
                .to_string(),
            ),
        ),
        SysopCmd::UnbanIp { network } => (
            "DELETE",
            "/v1/ip-bans".into(),
            TokenScope::Admin,
            Some(serde_json::json!({ "network": network }).to_string()),
        ),
        SysopCmd::Moderation => ("GET", "/v1/moderation".into(), TokenScope::Read, None),
        SysopCmd::Backup => ("POST", "/v1/backup".into(), TokenScope::Admin, None),
        SysopCmd::Realm {
            action: RealmCmd::Close { reason },
        } => (
            "POST",
            "/v1/realm/close".into(),
            TokenScope::Admin,
            Some(serde_json::json!({ "reason": reason.clone().unwrap_or_default() }).to_string()),
        ),
        SysopCmd::Realm {
            action: RealmCmd::Open,
        } => ("POST", "/v1/realm/open".into(), TokenScope::Admin, None),
        SysopCmd::Who => ("GET", "/v1/who".into(), TokenScope::Read, None),
        SysopCmd::Broadcast { message } => (
            "POST",
            "/v1/broadcast".into(),
            TokenScope::Write,
            Some(serde_json::json!({ "text": message }).to_string()),
        ),
        SysopCmd::ReloadWorld => ("POST", "/v1/reload-world".into(), TokenScope::Write, None),
        SysopCmd::Shutdown { cancel: true, .. } => {
            ("DELETE", "/v1/shutdown".into(), TokenScope::Admin, None)
        }
        SysopCmd::Shutdown { delay, message, .. } => (
            "POST",
            "/v1/shutdown".into(),
            TokenScope::Admin,
            Some(
                serde_json::json!({
                    "in_secs": delay,
                    "message": message.clone().unwrap_or_default(),
                })
                .to_string(),
            ),
        ),
    }
}

/// `spectate`: the player's screen on stdout until he leaves; with `typing`, each line read
/// from stdin is typed for him.
fn spectate(cfg: &Config, player: &str, typing: bool) -> Result<(), String> {
    use std::io::{BufRead as _, Write as _};
    let bind = &cfg.server.admin_bind;
    let addr = reach(bind).ok_or_else(|| format!("server.admin_bind `{bind}` is not host:port"))?;
    let token = token_for(cfg, TokenScope::Admin).ok_or_else(|| {
        "the config has no admin API token with its secret for this (admin scope): set \
         SILVERMERE_ADMIN_TOKEN to the server's token"
            .to_owned()
    })?;
    let path = format!("/v1/players/{}/screen", segment(player));
    let mut s =
        std::net::TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| match e.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                format!("no server is running on {addr} (server.admin_bind)")
            }
            _ => format!("calling GET {path} on {addr}: {e}"),
        })?;
    // HTTP/1.0: the stream comes as it is, not in chunks, and ends with the connection
    write!(
        s,
        "GET {path} HTTP/1.0\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nAccept: text/event-stream\r\n\r\n"
    )
    .map_err(|e| format!("calling GET {path} on {addr}: {e}"))?;
    let mut r = std::io::BufReader::new(s);
    let mut status_line = String::new();
    r.read_line(&mut status_line)
        .map_err(|e| format!("reading the answer: {e}"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("not an HTTP answer")?;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).map_err(|e| e.to_string())? == 0 || h.trim().is_empty() {
            break;
        }
    }
    if status != 200 {
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut r, &mut body);
        let message = error_message(&body);
        return Err(match status {
            401 => format!("the server on {addr} does not know this token: {message}"),
            403 => format!("the token lacks the admin scope: {message}"),
            _ => format!("the server on {addr} answered {status}: {message}"),
        });
    }
    if typing {
        let (addr, player, token) = (addr, player.to_owned(), token.clone());
        std::thread::spawn(move || {
            for line in std::io::stdin().lines() {
                let Ok(line) = line else { return };
                let body = serde_json::json!({ "line": line }).to_string();
                let path = format!("/v1/players/{}/input", segment(&player));
                match call(addr, "POST", &path, Some(&token), Some(&body), TIMEOUT) {
                    Ok((200, _)) => {}
                    Ok((_, b)) => eprintln!("not typed: {}", error_message(&b)),
                    Err(e) => eprintln!("not typed: {e}"),
                }
            }
        });
    }
    let mut out = std::io::stdout();
    let (mut event, mut data) = (String::new(), String::new());
    let mut name = player.to_owned();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)
            .map_err(|e| format!("reading the screen: {e}"))?
            == 0
        {
            return Ok(());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(v) = line.strip_prefix("event:") {
            event = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push_str(v.strip_prefix(' ').unwrap_or(v));
        } else if line.is_empty() && !event.is_empty() {
            let v: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
            match event.as_str() {
                "screen" => {
                    if let Some(n) = v.get("player").and_then(Value::as_str) {
                        name = n.to_owned();
                    }
                    eprintln!(
                        "Watching {name}{}. Ctrl-C to stop.",
                        if typing {
                            "; each line you type goes to the game as theirs"
                        } else {
                            ""
                        }
                    );
                }
                "end" => {
                    eprintln!("\r\n{name} left the realm.");
                    return Ok(());
                }
                _ => {}
            }
            if let Some(t) = v.get("text").and_then(Value::as_str) {
                let _ = out.write_all(t.as_bytes());
                let _ = out.flush();
            }
            event.clear();
            data.clear();
        }
    }
}

/// Sends the call; the body of a `200`, else the error line.
fn send(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    scope: TokenScope,
    token: &str,
    body: Option<String>,
) -> Result<String, String> {
    let (status, body) =
        call(addr, method, path, Some(token), body.as_deref(), TIMEOUT).map_err(|e| {
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
    let message = error_message(&body);
    Err(match status {
        401 => format!("the server on {addr} does not know this token: {message}"),
        403 => format!("the token lacks the {} scope: {message}", scope_name(scope)),
        _ => format!("the server on {addr} answered {status}: {message}"),
    })
}

/// The `message` of the API's error body `{"error": code, "message": text}`, else the body.
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned())
}

fn scope_name(scope: TokenScope) -> &'static str {
    match scope {
        TokenScope::Read => "read",
        TokenScope::Write => "write",
        TokenScope::Admin => "admin",
    }
}

/// The answer as text. An answer that is not the documented shape is printed as it came.
pub fn render(action: &SysopCmd, body: &str) -> String {
    let shown = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| match action {
            SysopCmd::Who => who(&v),
            SysopCmd::Broadcast { .. } => {
                let n = v.get("delivered")?.as_u64()?;
                Some(format!("Told {n} player{}.\n", plural(n)))
            }
            SysopCmd::ReloadWorld => reloaded(&v),
            SysopCmd::Shutdown { cancel: true, .. } => Some(
                if v.get("cancelled")?.as_bool()? {
                    "Cancelled the planned shutdown; the players were told.\n"
                } else {
                    "No shutdown was planned.\n"
                }
                .to_owned(),
            ),
            SysopCmd::Shutdown { .. } => planned(&v),
            SysopCmd::Restart { cancel: true, .. } => Some(
                if v.get("cancelled")?.as_bool()? {
                    "Cancelled the planned restart; the players were told.\n"
                } else {
                    "No restart or shutdown was planned.\n"
                }
                .to_owned(),
            ),
            SysopCmd::Restart { .. } => planned(&v),
            SysopCmd::Kick { .. } => Some(format!(
                "Disconnected {}; the character was saved.\n",
                v.get("kicked")?.as_str()?
            )),
            SysopCmd::Tell { .. } => Some(format!("Told {}.\n", v.get("told")?.as_str()?)),
            SysopCmd::Mute(_) | SysopCmd::Jail(_) | SysopCmd::Ban(_) => put_done(&v),
            SysopCmd::Unmute { .. } | SysopCmd::Release { .. } | SysopCmd::Unban { .. } => {
                lift_done(&v)
            }
            SysopCmd::BanIp { .. } => {
                let kicked: Vec<&str> = v
                    .get("kicked")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                Some(format!(
                    "Banned {}{}. Telnet connections from it are refused; {}.\n",
                    v.get("network")?.as_str()?,
                    until_words(v.get("until")?),
                    if kicked.is_empty() {
                        "nobody in the realm came from it".to_owned()
                    } else {
                        format!("disconnected {} (saved first)", kicked.join(", "))
                    }
                ))
            }
            SysopCmd::UnbanIp { .. } => Some(format!(
                "Lifted the ban of {}.\n",
                v.get("network")?.as_str()?
            )),
            SysopCmd::Moderation => moderation(&v),
            SysopCmd::Spectate { .. } => None,
            SysopCmd::Type { .. } => Some(format!(
                "Typed for {}: {}\n",
                v.get("player")?.as_str()?,
                v.get("line")?.as_str()?
            )),
            SysopCmd::Game { .. } => {
                let out = v.get("output")?.as_str()?;
                Some(if out.ends_with('\n') {
                    out.to_owned()
                } else {
                    format!("{out}\n")
                })
            }
            SysopCmd::Backup => Some(if v.get("ok")?.as_bool()? {
                format!(
                    "Backup made: {} ({} KiB).\n",
                    v.get("file")?.as_str()?,
                    v.get("bytes").and_then(Value::as_u64).unwrap_or(0) / 1024
                )
            } else {
                format!(
                    "The backup failed: {}\n",
                    v.get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("see the server log")
                )
            }),
            SysopCmd::Realm {
                action: RealmCmd::Close { .. },
            } => {
                let n = v.get("disconnected")?.as_u64()?;
                Some(format!(
                    "The realm is closed: only sysops may log in. Disconnected {n} player{}, \
                     saved.\nOpen it with: silvermere sysop realm open\n",
                    plural(n)
                ))
            }
            SysopCmd::Realm {
                action: RealmCmd::Open,
            } => Some(
                if v.get("was_closed")?.as_bool()? {
                    "The realm is open.\n"
                } else {
                    "The realm was already open.\n"
                }
                .to_owned(),
            ),
        });
    shown.unwrap_or_else(|| format!("{}\n", body.trim()))
}

/// ` until 2026-12-01 18:00 EST` in this computer's time zone, or ` until lifted`.
fn until_words(until: &Value) -> String {
    match until
        .as_str()
        .and_then(|t| t.parse::<jiff::Timestamp>().ok())
    {
        Some(t) => format!(
            " until {}",
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M %Z")
        ),
        None => " until lifted".to_owned(),
    }
}

fn put_done(v: &Value) -> Option<String> {
    let player = v.get("player")?.as_str()?;
    let online = v.get("online")?.as_bool()?;
    let until = until_words(v.get("until")?);
    Some(match v.get("kind")?.as_str()? {
        "ban" => format!(
            "Banned {player}{until}.{}\n",
            if online {
                " Disconnected from the game."
            } else {
                ""
            }
        ),
        "jail" => format!(
            "Jailed {player}{until}.{}\n",
            if online {
                " Moved to the jail room."
            } else {
                " Held there from the next entry to the realm."
            }
        ),
        _ => format!(
            "Muted {player}{until}: no gossip or auction.{}\n",
            if online { " Told in the game." } else { "" }
        ),
    })
}

fn lift_done(v: &Value) -> Option<String> {
    let player = v.get("player")?.as_str()?;
    Some(match v.get("kind")?.as_str()? {
        "ban" => format!("Lifted {player}'s ban.\n"),
        "jail" => format!("Released {player} from jail.\n"),
        _ => format!("Lifted {player}'s mute.\n"),
    })
}

/// `sysop moderation`: the realm, then the sanctions and the IP bans in force.
fn moderation(v: &Value) -> Option<String> {
    let realm = v.get("realm")?;
    let mut out = if realm.get("closed")?.as_bool()? {
        let reason = realm.get("reason").and_then(Value::as_str).unwrap_or("");
        format!(
            "The realm is closed{}: only sysops may log in.\n",
            if reason.is_empty() {
                String::new()
            } else {
                format!(" ({reason})")
            }
        )
    } else {
        "The realm is open.\n".to_owned()
    };
    let sanctions = v.get("sanctions")?.as_array()?;
    let ip_bans = v.get("ip_bans")?.as_array()?;
    if sanctions.is_empty() && ip_bans.is_empty() {
        out.push_str("No bans, jails or mutes are in force.\n");
        return Some(out);
    }
    for s in sanctions {
        let who = s
            .get("character")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .or_else(|| s.get("username").and_then(Value::as_str))
            .unwrap_or("?");
        let reason = s.get("reason").and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!(
            "{:<5} {who}{}{} (by {})\n",
            s.get("kind")?.as_str()?,
            until_words(s.get("until")?),
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            },
            s.get("by").and_then(Value::as_str).unwrap_or("?"),
        ));
    }
    for b in ip_bans {
        let reason = b.get("reason").and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!(
            "ip    {}{}{} (by {})\n",
            b.get("network")?.as_str()?,
            until_words(b.get("until")?),
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            },
            b.get("by").and_then(Value::as_str).unwrap_or("?"),
        ));
    }
    Some(out)
}

/// When a planned shutdown happens, in this computer's time zone.
fn planned(v: &Value) -> Option<String> {
    let at = v.get("at")?.as_str()?;
    let secs = v.get("in_secs")?.as_u64()?;
    let local = at
        .parse::<jiff::Timestamp>()
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M:%S %Z")
                .to_string()
        })
        .unwrap_or_else(|_| at.to_owned());
    let restart = v.get("restart").and_then(Value::as_bool) == Some(true);
    Some(match (secs, restart) {
        (0, false) => "The server is stopping now.\n".to_owned(),
        (0, true) => "The server is restarting now.\n".to_owned(),
        (_, false) => format!(
            "The server will stop at {local}, in {}. The players in the game are warned until \
             then.\nCall it off with: shutdown --cancel\n",
            crate::planned::how_long(secs)
        ),
        (_, true) => format!(
            "The server will restart at {local}, in {}. The players in the game are warned \
             until then; a backup is taken before it starts again.\nCall it off with: restart --cancel\n",
            crate::planned::how_long(secs)
        ),
    })
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
    fn moderation_answers_say_what_happened() {
        let mute = SysopCmd::Mute(SanctionArgs {
            player: "lars".into(),
            for_secs: None,
            reason: None,
        });
        assert_eq!(
            render(
                &mute,
                r#"{"player":"Lars","account":3,"kind":"mute","until":null,"online":true}"#
            ),
            "Muted Lars until lifted: no gossip or auction. Told in the game.\n"
        );
        let release = SysopCmd::Release {
            player: "lars".into(),
        };
        assert_eq!(
            render(
                &release,
                r#"{"player":"Lars","account":3,"kind":"jail","lifted":true,"online":false}"#
            ),
            "Released Lars from jail.\n"
        );
        assert_eq!(
            render(
                &SysopCmd::Moderation,
                r#"{"realm":{"closed":false},"sanctions":[],"ip_bans":[]}"#
            ),
            "The realm is open.\nNo bans, jails or mutes are in force.\n"
        );
        assert_eq!(
            render(
                &SysopCmd::Moderation,
                r#"{"realm":{"closed":true,"reason":"upgrading"},
                    "sanctions":[{"account":3,"username":"lars","character":"Lars","kind":"ban",
                      "until":null,"reason":"spam","by":"ops"}],
                    "ip_bans":[{"id":1,"network":"203.0.113.0/24","until":null,"reason":"",
                      "by":"ops"}]}"#
            ),
            "The realm is closed (upgrading): only sysops may log in.\n\
             ban   Lars until lifted: spam (by ops)\n\
             ip    203.0.113.0/24 until lifted (by ops)\n"
        );
    }

    #[test]
    fn names_are_one_path_segment() {
        assert_eq!(segment("Lars"), "Lars");
        assert_eq!(segment("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn an_unexpected_answer_is_printed_as_it_came() {
        assert_eq!(render(&SysopCmd::Who, "{\"x\":1}\n"), "{\"x\":1}\n");
    }
}
