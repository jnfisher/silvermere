//! The completed admin API against a running server (T-060): reload-world with a player
//! logged in, every emitted event kind on the SSE stream and in the `events` table, and the
//! `admin token` CLI against the bearer check.

mod common;

use std::collections::BTreeMap;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use silvermere::config::{AdminCfg, Config, TokenCfg, TokenScope};
use silvermere_api::{EventKind, Scope, Token, auth::authenticate};
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_db::Db;
use silvermere_engine::{SeededRng, SystemClock, clear_player};
use silvermere_testkit::TelnetClient;
use silvermere_types::enums::StatlineMode;
use silvermere_types::ids::{ClassNo, ItemNo, MapId, MonsterNo, RaceNo, RoomId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const WAIT: Duration = Duration::from_secs(30);
const TOKEN: &str = "sm_test-token";

fn config(url: &str, scopes: Vec<TokenScope>) -> Config {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url.to_string();
    cfg.admin = AdminCfg {
        tokens: vec![TokenCfg {
            name: "ops".into(),
            sha256: Some(silvermere_api::auth::sha256_hex(TOKEN)),
            token: None,
            scopes,
        }],
        ..Default::default()
    };
    cfg
}

async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
) -> (u16, Value) {
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}Content-Length: 0\r\n\r\n"
    );
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_string(&mut raw))
        .await
        .expect("response within 5 s")
        .unwrap();
    let code = raw.split(' ').nth(1).unwrap().parse().unwrap();
    let body = raw.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (code, serde_json::from_str(body).unwrap_or(Value::Null))
}

async fn read_until(s: &mut TcpStream, buf: &mut String, needle: &str) {
    let mut chunk = [0u8; 4096];
    while !buf.contains(needle) {
        let n = tokio::time::timeout(WAIT, s.read(&mut chunk))
            .await
            .unwrap_or_else(|_| panic!("no {needle:?} in {buf:?}"))
            .unwrap();
        assert!(n > 0, "closed before {needle:?}: {buf:?}");
        buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }
}

async fn character(db: &Db, name: &str, pass: &str) {
    let account = create_with_cost(db, name, pass, &[Role::Player], Cost::Low)
        .await
        .unwrap();
    let mut c = clear_player();
    c.userid = name.to_uppercase();
    c.firstname = name.into();
    c.map = MapId(1);
    c.room = RoomId(common::special_rooms()[0]);
    c.hp = 35;
    c.maxhp = 35;
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.statline = StatlineMode::Full;
    // A stored row as `load_player` (T-023) accepts it: record version 22, level 1, stats
    // inside the zeroed test race's range, the name validated, and `hpbonus` making the
    // recomputed maximum the 35 hit points the prompt shows.
    c.version = 22;
    c.level = 1;
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (50, 50, 50, 50, 50, 50);
    (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (50, 50, 50, 50, 50, 50);
    c.hpbonus = 10;
    c.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
    c.items = vec![ItemNo(1), ItemNo(2)];
    c.itemuses = vec![0, 0];
    db.characters().insert(account.id, &c).await.unwrap();
}

async fn enter_realm(client: &mut TelnetClient, name: &str, pass: &str) {
    client.until("Username:", WAIT).await.unwrap();
    client.send_line(name).await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    client.send_line(pass).await.unwrap();
    client
        .until(r"Make your selection \(X to exit\):", WAIT)
        .await
        .unwrap();
    client.send_line("M").await.unwrap();
    client.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    client.send_line("E").await.unwrap();
    client.until(r"\[HP=35\]:", WAIT).await.unwrap();
}

// docs/admin-api.md "POST /v1/reload-world": a changed monster row reaches the engine, the
// player who is logged in stays connected and keeps his inventory.
#[tokio::test]
async fn reload_world_applies_a_changed_row_and_keeps_the_player() {
    let (_t, url) = common::seeded(&[]).await;
    let db = Db::connect_and_migrate(&url).await.unwrap();
    character(&db, "Aldric", "aldric1").await;
    let r = silvermere::serve_in_process(
        config(&url, vec![TokenScope::Read, TokenScope::Write]),
        SystemClock,
        SeededRng::new(7),
    )
    .await
    .expect("starts");
    let admin = r.admin_addr.unwrap();
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    enter_realm(&mut client, "Aldric", "aldric1").await;

    let mut m = db.monsters().get(MonsterNo(1)).await.unwrap().unwrap();
    m.name = "Reloaded Rat".into();
    db.monsters().put(&m).await.unwrap();

    // Read-only is refused; the write token reloads.
    let (code, _) = http(admin, "POST", "/v1/reload-world", None).await;
    assert_eq!(code, 401);
    let (code, body) = http(admin, "POST", "/v1/reload-world", Some(TOKEN)).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(body["reloaded"]["monsters"], 2);
    assert_eq!(body["reloaded"]["items"], 2);
    assert!(body["ms"].is_u64());

    // Still connected: the realm answers a command and who lists him.
    client.send_line("").await.unwrap();
    client.until(r"\[HP=35\]:", WAIT).await.unwrap();
    let (code, who) = http(admin, "GET", "/v1/who", Some(TOKEN)).await;
    assert_eq!(code, 200);
    assert_eq!(who["players"][0]["first_name"], "Aldric");

    let world = r.stop().await.unwrap();
    assert_eq!(world.monsters[&MonsterNo(1)].name, "Reloaded Rat");
    let kept = world
        .sessions
        .values()
        .filter_map(|s| s.character.as_ref())
        .find(|c| c.firstname == "Aldric")
        .expect("the player was still in the world");
    assert_eq!(kept.items[..2], [ItemNo(1), ItemNo(2)]);
    db.close().await;
}

// A reload that fails leaves the world as it was: here the database lost a special room.
#[tokio::test]
async fn reload_world_failure_leaves_the_old_tables() {
    let (_t, url) = common::seeded(&[]).await;
    let db = Db::connect_and_migrate(&url).await.unwrap();
    let r = silvermere::serve_in_process(
        config(&url, vec![TokenScope::Write]),
        SystemClock,
        SeededRng::new(7),
    )
    .await
    .expect("starts");
    let admin = r.admin_addr.unwrap();
    let mut m = db.monsters().get(MonsterNo(1)).await.unwrap().unwrap();
    m.name = "Never Loaded".into();
    db.monsters().put(&m).await.unwrap();
    sqlx::query("DELETE FROM rooms WHERE room = ?")
        .bind(i64::from(common::special_rooms()[0]))
        .execute(&db.pool())
        .await
        .unwrap();
    let (code, body) = http(admin, "POST", "/v1/reload-world", Some(TOKEN)).await;
    assert_eq!(
        (code, &body["error"]),
        (500, &Value::from("reload_failed")),
        "{body}"
    );
    let world = r.stop().await.unwrap();
    assert_ne!(world.monsters[&MonsterNo(1)].name, "Never Loaded");
    db.close().await;
}

// docs/admin-api.md "SSE and event kinds": each kind that something emits arrives on the
// stream and is a row in `events` with the same id, kind and payload.
#[tokio::test]
async fn every_emitted_event_kind_is_on_the_stream_and_in_the_table() {
    let (_t, url) = common::seeded(&[]).await;
    let db = Db::connect_and_migrate(&url).await.unwrap();
    character(&db, "Aldric", "aldric1").await;
    let r = silvermere::serve_in_process(
        config(&url, vec![TokenScope::Read, TokenScope::Write]),
        SystemClock,
        SeededRng::new(7),
    )
    .await
    .expect("starts");
    let admin = r.admin_addr.unwrap();
    let mut sse = TcpStream::connect(admin).await.unwrap();
    sse.write_all(
        format!("GET /v1/events HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let mut seen = String::new();
    read_until(&mut sse, &mut seen, "\r\n\r\n").await;
    // The audit row of a /v1/who call proves the subscription is live.
    for _ in 0..50 {
        http(admin, "GET", "/v1/who", Some(TOKEN)).await;
        if tokio::time::timeout(
            Duration::from_millis(100),
            read_until(&mut sse, &mut seen, "event: admin.who"),
        )
        .await
        .is_ok()
        {
            break;
        }
    }
    // connected, auth.login_failed, closed, level
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    client.send_line("Aldric").await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    client.send_line("wrong-password").await.unwrap();
    read_until(&mut sse, &mut seen, "event: auth.login_failed").await;
    drop(client);
    read_until(&mut sse, &mut seen, "event: closed").await;
    // level: what `train_level` emits on a level gain (T-039; the engine tests drive TRAIN
    // itself), raised here from inside the engine as the trained character would.
    r.to_engine
        .send(silvermere_engine::EngineInput::Custom {
            session: None,
            run: Box::new(|w| w.level_gained(silvermere_engine::SessionId(0), 2)),
        })
        .unwrap();
    read_until(&mut sse, &mut seen, "event: level").await;

    let mut frames: BTreeMap<i64, (String, Value)> = BTreeMap::new();
    for frame in seen.split("\n\n").filter(|f| f.contains("event: ")) {
        let field = |name: &str| {
            frame
                .lines()
                .find_map(|l| l.strip_prefix(name).map(str::trim))
                .map(str::to_string)
        };
        let (Some(id), Some(kind), Some(data)) = (field("id:"), field("event:"), field("data:"))
        else {
            continue;
        };
        frames.insert(
            id.parse().unwrap(),
            (kind, serde_json::from_str(&data).unwrap()),
        );
    }
    let rows = db.events().since(0, 10_000).await.unwrap();
    let mut families = std::collections::HashSet::new();
    for (id, (kind, data)) in &frames {
        let row = rows
            .iter()
            .find(|e| e.id == *id)
            .unwrap_or_else(|| panic!("event {id} ({kind}) is not in the events table"));
        assert_eq!(&row.event.kind, kind, "id {id}");
        assert_eq!(&row.event.payload, &data["payload"], "payload of id {id}");
        assert_eq!(data["kind"], kind.as_str());
        families.insert(EventKind::of(kind).unwrap_or_else(|| panic!("unknown kind {kind}")));
    }
    for k in EventKind::ALL {
        assert_eq!(
            families.contains(k),
            k.emitted(),
            "{k:?}: seen on the stream = emitted() ({families:?})"
        );
    }
    db.close().await;
    r.stop().await.unwrap();
}

// ---- admin token CLI ---------------------------------------------------------------------

fn silvermere(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(dir).args(args);
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
        c.env_remove(k);
    }
    c.output().unwrap()
}

#[test]
fn token_new_prints_a_token_whose_hash_the_bearer_check_accepts() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("admin-token");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let o = silvermere(
        &dir,
        &["admin", "token", "new", "editor", "--scopes", "read,write"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    let token = out
        .lines()
        .find_map(|l| l.strip_prefix("Token (shown once, store it now): "))
        .expect("the token line")
        .to_string();
    assert!(token.starts_with("sm_") && token.len() == 46, "{token}");
    let block = &out[out.find("[[admin.tokens]]").expect("a toml block")..];
    assert!(!block.contains(&token), "the block holds only the hash");

    // The block loads as config and its hash accepts exactly this token.
    std::fs::write(dir.join("silvermere.toml"), block).unwrap();
    let cfg = Config::load(Some(&dir), &Default::default()).unwrap();
    assert_eq!(cfg.admin.tokens.len(), 1);
    assert_eq!(cfg.admin.tokens[0].name, "editor");
    let tokens: Vec<Token> = cfg.admin.tokens.iter().map(|t| t.to_token()).collect();
    let caller = authenticate(&tokens, Some(&format!("Bearer {token}"))).expect("accepted");
    assert_eq!(caller.name, "editor");
    assert!(caller.allows(Scope::Read) && caller.allows(Scope::Write));
    assert!(!caller.allows(Scope::Admin));
    assert!(authenticate(&tokens, Some(&format!("Bearer {token}x"))).is_none());

    // list shows the name, scopes and a hash prefix, never a token.
    let o = silvermere(&dir, &["admin", "token", "list"]);
    let list = String::from_utf8_lossy(&o.stdout).into_owned();
    let hex = silvermere_api::auth::sha256_hex(&token);
    assert!(list.contains("editor\tread,write\tsha256:"), "{list}");
    assert!(
        list.contains(&hex[..8]) && !list.contains(&hex[..16]),
        "{list}"
    );
    assert!(!list.contains(&token));

    // Two tokens differ.
    let again = silvermere(
        &dir,
        &["admin", "token", "new", "editor", "--scopes", "read"],
    );
    assert!(!String::from_utf8_lossy(&again.stdout).contains(&token));
    let bad = silvermere(&dir, &["admin", "token", "new", "x", "--scopes", "root"]);
    assert!(!bad.status.success());
}

#[test]
fn the_legacy_env_token_is_a_default_token_with_every_scope() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("admin-legacy");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
    c.current_dir(&dir)
        .args(["admin", "token", "list"])
        .env("SILVERMERE_ADMIN_TOKEN", "legacy-secret");
    let o = c.output().unwrap();
    let list = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        list.contains("default\tread,write,admin\tsha256:"),
        "{list}"
    );
    assert!(!list.contains("legacy-secret"));
}
