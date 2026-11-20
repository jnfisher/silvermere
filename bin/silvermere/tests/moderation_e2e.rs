//! Moderation against a running server on the stock world: players on telnet, the sysop's
//! `silvermere sysop` commands over the admin API. Jail moves a player to the jail room and
//! holds him there; release lets him out to town; mute stops gossip; tell reaches him; kick
//! and ban disconnect him, and a ban refuses his login; a closed realm refuses everyone but
//! sysops; an IP ban closes connections before the login screen. The login and logoff events
//! carry the address.

use crate::common;

use std::process::Command;
use std::time::Duration;

use common::m3::{Player, password, place, start_manual, team_db, tick};
use common::stock::{WAIT, on_world, start_with};
use silvermere::config::{AdminCfg, TokenCfg, TokenScope};
use silvermere_db::Db;
use silvermere_testkit::TelnetClient;
use silvermere_types::ids::RoomKey;

const TOKEN: &str = "moderation-secret";

fn admin() -> AdminCfg {
    AdminCfg {
        tokens: vec![TokenCfg {
            name: "ops".into(),
            sha256: None,
            token: Some(TOKEN.into()),
            scopes: vec![TokenScope::Admin],
        }],
        ..Default::default()
    }
}

/// `silvermere sysop <args>` against the server's admin API; exit code, stdout, stderr.
async fn sysop(r: &silvermere::Running, args: &[&str]) -> (i32, String, String) {
    let addr = r.admin_addr.expect("admin listener").to_string();
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_silvermere"));
        c.arg("sysop").args(&args);
        for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("SILVERMERE_")) {
            c.env_remove(k);
        }
        c.env("SILVERMERE_SERVER__ADMIN_BIND", addr)
            .env("SILVERMERE_ADMIN_TOKEN", TOKEN)
            .current_dir(std::env::temp_dir());
        let o = c.output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    })
    .await
    .unwrap()
}

/// `sysop` that must succeed; its stdout.
async fn ok(r: &silvermere::Running, args: &[&str]) -> String {
    let (code, out, err) = sysop(r, args).await;
    assert_eq!(code, 0, "sysop {args:?}: {err}");
    out
}

/// The room the character `name` stands in.
async fn room_of(r: &silvermere::Running, name: &'static str) -> Option<RoomKey> {
    on_world(r, move |w| {
        w.session_ids().into_iter().find_map(|id| {
            let s = w.session(id)?;
            (s.player()?.firstname == name)
                .then(|| s.room_key())
                .flatten()
        })
    })
    .await
}

/// The jail room and the town a released player goes to.
async fn jail_and_town(r: &silvermere::Running) -> (RoomKey, RoomKey) {
    on_world(r, |w| {
        (
            silvermere_engine::World::map1(w.config.jail_room),
            silvermere_engine::World::map1(w.config.reinc_room),
        )
    })
    .await
}

/// A telnet client at the password prompt's answer: what the login said, up to `until`.
async fn try_login(r: &silvermere::Running, name: &str, until: &str) -> String {
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    c.until("Username:", WAIT).await.unwrap();
    c.send_line(name).await.unwrap();
    c.until("Password:", WAIT).await.unwrap();
    let at = c.position();
    c.send_line(&password(name)).await.unwrap();
    c.until(until, WAIT).await.unwrap();
    c.plain_from(at)
}

#[tokio::test]
async fn jail_release_mute_and_tell_reach_a_player_in_the_game() {
    let (_dir, url) = team_db();
    place(&url, "Brynja", 2140).await;
    let r = start_with(&url, |cfg| {
        cfg.game.sec_fast = 86_400;
        cfg.admin = admin();
    })
    .await;
    let (jail, town) = jail_and_town(&r).await;
    let mut b = Player::enter(&r, "Brynja").await;

    // who shows the address
    let who = ok(&r, &["--json", "who"]).await;
    let v: serde_json::Value = serde_json::from_str(&who).unwrap();
    assert_eq!(v["players"][0]["first_name"], "Brynja");
    assert_eq!(v["players"][0]["ip"], "127.0.0.1");
    assert_eq!(v["players"][0]["jailed"], false);

    // jail: told, moved, held
    let at = b.c.position();
    let out = ok(&r, &["jail", "brynja", "--for", "1h", "fighting in town"]).await;
    assert!(out.starts_with("Jailed Brynja until "), "{out}");
    assert!(out.contains("Moved to the jail room."), "{out}");
    let got = b
        .wait_for(
            at,
            r"The sysop has sent you to jail until .*Reason: fighting in town",
        )
        .await;
    assert!(got.contains("UTC."), "{got}");
    assert_eq!(room_of(&r, "Brynja").await, Some(jail));
    // every exit of the jail room is refused (an exit that is not there says so as ever)
    let exits: Vec<&str> = on_world(&r, move |w| {
        let words = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"];
        w.room(jail).map_or(Vec::new(), |rm| {
            rm.exits
                .iter()
                .zip(words)
                .filter(|(e, _)| e.target.0 != 0)
                .map(|(_, d)| d)
                .collect()
        })
    })
    .await;
    assert!(!exits.is_empty(), "the stock jail room has a way out");
    for dir in exits {
        // a move waits for the fast cycle, which this server runs by hand
        let at = b.c.position();
        b.say(dir).await;
        let mut held = false;
        for _ in 0..50 {
            tick(&r, 1).await;
            let got =
                b.c.wait_from(
                    at,
                    &silvermere_testkit::client::Wait {
                        until: Some("You are held in jail until".into()),
                        timeout: Duration::from_millis(100),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            if got.matched {
                held = true;
                break;
            }
        }
        assert!(held, "{dir}: {}", b.c.plain_from(at));
    }
    assert_eq!(room_of(&r, "Brynja").await, Some(jail), "nobody walks out");
    let v: serde_json::Value = serde_json::from_str(&ok(&r, &["--json", "who"]).await).unwrap();
    assert_eq!(v["players"][0]["jailed"], true);
    let listed = ok(&r, &["moderation"]).await;
    assert!(
        listed.contains("jail  Brynja until ") && listed.contains("fighting in town (by ops)"),
        "{listed}"
    );

    // release: told and moved to town
    let at = b.c.position();
    assert_eq!(
        ok(&r, &["release", "Brynja"]).await,
        "Released Brynja from jail.\n"
    );
    b.wait_for(at, "Your time in jail is over").await;
    assert_eq!(room_of(&r, "Brynja").await, Some(town));
    let (code, _, err) = sysop(&r, &["release", "Brynja"]).await;
    assert_eq!(code, 1);
    assert!(err.contains("no sanction of that kind"), "{err}");

    // a jail that runs out lets the player out at the next tick after its end
    ok(&r, &["jail", "Brynja", "--for", "1"]).await;
    assert_eq!(room_of(&r, "Brynja").await, Some(jail));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let at = b.c.position();
    tick(&r, 1).await;
    b.wait_for(at, "Your time in jail is over").await;
    assert_eq!(room_of(&r, "Brynja").await, Some(town));

    // mute: gossip refused until lifted
    ok(&r, &["mute", "Brynja", "spamming"]).await;
    let t = b.cmd("gossip hello all").await;
    assert!(t.contains("You are muted. Reason: spamming"), "{t}");
    ok(&r, &["unmute", "Brynja"]).await;
    let t = b.cmd("gossip hello all").await;
    assert!(!t.contains("muted"), "{t}");

    // tell
    let at = b.c.position();
    assert_eq!(
        ok(&r, &["tell", "Brynja", "please keep it civil"]).await,
        "Told Brynja.\n"
    );
    b.wait_for(at, "The sysop tells you: please keep it civil")
        .await;

    // nobody of that name, and a player not connected
    let (code, _, err) = sysop(&r, &["kick", "Nobodyhere"]).await;
    assert_eq!(code, 1);
    assert!(
        err.contains("no character or account has that name"),
        "{err}"
    );
    let (code, _, err) = sysop(&r, &["tell", "Isolde", "hi"]).await;
    assert_eq!(code, 1);
    assert!(err.contains("not connected"), "{err}");

    b.leave_ticking(&r).await;
    r.stop().await.unwrap();
}

#[tokio::test]
async fn kick_ban_ip_ban_and_a_closed_realm_keep_players_out() {
    let (_dir, url) = team_db();
    place(&url, "Brynja", 2140).await;
    place(&url, "Isolde", 2140).await;
    let r = start_with(&url, |cfg| {
        cfg.game.sec_fast = 86_400;
        cfg.admin = admin();
    })
    .await;

    // kick: told, saved, disconnected; the logoff is `closed`
    let mut b = Player::enter(&r, "Brynja").await;
    let at = b.c.position();
    assert_eq!(
        ok(&r, &["kick", "Brynja", "take a break"]).await,
        "Disconnected Brynja; the character was saved.\n"
    );
    b.wait_text(
        at,
        r"You have been disconnected by the sysop\. Reason: take a break\s+Your character has been saved",
    )
    .await;
    drop(b);

    // ban: a player in the game is disconnected, his login refused until it is lifted
    let mut b = Player::enter(&r, "Brynja").await;
    let at = b.c.position();
    let out = ok(&r, &["ban", "Brynja", "--for", "7d", "spamming gossip"]).await;
    assert!(out.contains("Disconnected from the game."), "{out}");
    b.wait_text(at, r"Reason: banned until \S+ \S+ UTC: spamming gossip")
        .await;
    drop(b);
    let said = try_login(&r, "Brynja", "Reason: spamming gossip").await;
    assert!(said.contains("This account is banned until "), "{said}");
    ok(&r, &["unban", "Brynja"]).await;
    let b = Player::enter(&r, "Brynja").await;

    // a closed realm: the players are disconnected and refused; the rlogin door and sysops
    // are covered by the auth tests
    let mut i = Player::enter(&r, "Isolde").await;
    let at = i.c.position();
    let out = ok(&r, &["realm", "close", "upgrading"]).await;
    assert!(out.contains("Disconnected 2 players"), "{out}");
    i.wait_text(
        at,
        "The realm is closed: upgrading. Your character has been saved.",
    )
    .await;
    drop((b, i));
    let said = try_login(&r, "Isolde", "try again later").await;
    assert!(said.contains("The realm is closed: upgrading."), "{said}");
    assert!(
        ok(&r, &["moderation"])
            .await
            .contains("The realm is closed (upgrading)")
    );
    ok(&r, &["realm", "open"]).await;
    let i = Player::enter(&r, "Isolde").await;

    // an IP ban closes a telnet connection before anything is sent
    let out = ok(&r, &["ban-ip", "127.0.0.0/8", "--for", "1h", "bots"]).await;
    assert!(out.starts_with("Banned 127.0.0.0/8 until "), "{out}");
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    let got = c.read(Duration::from_secs(2)).await;
    assert!(
        c.plain_from(0).is_empty(),
        "nothing is sent to a banned address: {got:?}"
    );
    let (code, _, err) = sysop(&r, &["ban-ip", "not-an-address"]).await;
    assert_eq!(code, 1);
    assert!(err.contains("network must be an IP address"), "{err}");
    ok(&r, &["unban-ip", "127.0.0.0/8"]).await;
    let mut c = TelnetClient::connect(r.telnet_addr).await.unwrap();
    c.until("Username:", WAIT).await.unwrap();

    i.leave_ticking(&r).await;
    r.stop().await.unwrap();

    // the events carry the address and how each logoff happened
    let db = Db::connect_and_migrate(&url).await.unwrap();
    let events = db.events().recent(500).await.unwrap();
    let mut seen: Vec<(String, String)> = events
        .iter()
        .rev()
        .filter(|e| e.event.kind == "login" || e.event.kind == "logoff")
        .map(|e| {
            let p = &e.event.payload;
            (
                e.event.kind.clone(),
                p.get("how")
                    .or_else(|| p.get("ip"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect();
    seen.dedup();
    assert!(
        seen.contains(&("login".into(), "127.0.0.1".into())),
        "{seen:?}"
    );
    assert!(
        seen.contains(&("logoff".into(), "closed".into())),
        "{seen:?}"
    );
    assert!(seen.contains(&("logoff".into(), "exit".into())), "{seen:?}");
    db.close().await;
}

/// A sanction stored while the server was down holds at the next start: a jailed player
/// enters the realm in the jail room, and a closed realm stays closed.
#[tokio::test]
async fn sanctions_hold_across_a_restart() {
    let (_dir, url) = team_db();
    place(&url, "Brynja", 2140).await;
    {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        let account = db
            .accounts()
            .by_username("Brynja")
            .await
            .unwrap()
            .unwrap()
            .id;
        let now = jiff::Timestamp::now();
        db.moderation()
            .set(&silvermere_db::repo::Sanction {
                account,
                kind: silvermere_db::repo::SanctionKind::Jail,
                until: None,
                reason: "wait for the sysop".into(),
                by_whom: "ops".into(),
                created: now,
            })
            .await
            .unwrap();
        db.close().await;
    }
    let r = start_manual(&url).await;
    let (jail, _) = jail_and_town(&r).await;
    let mut b = Player::enter(&r, "Brynja").await;
    // a command's reply comes after everything queued for her
    b.cmd("i").await;
    let seen = b.c.plain_from(0);
    assert!(
        seen.contains("You are held in jail. Reason: wait for the sysop"),
        "{seen}"
    );
    assert_eq!(room_of(&r, "Brynja").await, Some(jail));
    b.leave_ticking(&r).await;
    r.stop().await.unwrap();
}
