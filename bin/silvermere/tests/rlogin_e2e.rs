//! The rlogin door login over a socket (`server.rlogin_bind`): a BBS that knows its
//! caller opens the game with the caller's name and the caller is not asked to log in
//! again; a first visit asks only the gender; an account without the `door` flag is
//! refused, and one with it is reached from the door and over telnet; an address that is
//! not trusted is closed; `R` at the BBS menu shim goes back to the BBS.

use crate::common;

use std::time::Duration;

use silvermere::config::Config;
use silvermere_auth::Role;
use silvermere_auth::cli::create_with_cost;
use silvermere_auth::password::Cost;
use silvermere_db::Db;
use silvermere_engine::{SeededRng, SystemClock};
use silvermere_testkit::TelnetClient;

const WAIT: Duration = Duration::from_secs(30);
const MENU: &str = r"Make your selection \(X to exit\):";

async fn server(url: String, trusted: &[&str]) -> silvermere::Running {
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.server.rlogin_bind = "127.0.0.1:0".into();
    cfg.server.rlogin_trusted = trusted.iter().map(|s| s.to_string()).collect();
    cfg.database.url = url;
    silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .expect("starts")
}

/// Connects to the rlogin port and sends the handshake for `user`, as a BBS door does
/// (RedialBBS: the caller as the client user, the server user blank or the same). A
/// server that has already closed the connection (an untrusted address) is no error here.
async fn rlogin(r: &silvermere::Running, client_user: &str, server_user: &str) -> TelnetClient {
    let mut c = TelnetClient::connect(r.rlogin_addr.expect("rlogin is on"))
        .await
        .unwrap();
    let _ = c
        .send_raw(format!("\0{client_user}\0{server_user}\0ansi/38400\0").as_bytes())
        .await;
    c
}

// An existing door account goes straight to the menu: no username, no password, no
// gender question.
#[tokio::test]
async fn an_existing_account_skips_the_login() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url, &[]).await;
    // the first visit makes the door account
    let mut first = rlogin(&r, "M", "Aldric").await;
    first.until(MENU, WAIT).await.unwrap();
    first.send_line("X").await.unwrap();
    first.until("R to re-logon", WAIT).await.unwrap();
    first.send_line("Y").await.unwrap();
    assert!(first.wait_closed(WAIT).await.unwrap());

    let mut c = rlogin(&r, "Aldric", "").await;
    let got = c.until(MENU, WAIT).await.unwrap();
    assert!(!got.plain.contains("Username:"), "{}", got.plain);
    assert!(!got.plain.contains("Password:"), "{}", got.plain);
    assert!(!got.plain.contains("gender"), "{}", got.plain);
    // the one-session rule holds: a second door visit while the first is on is refused
    let mut again = rlogin(&r, "x", "aldric").await;
    again.until("already logged on", WAIT).await.unwrap();
    assert!(again.wait_closed(WAIT).await.unwrap());
    // R at the logoff question goes back to the BBS, not to a password prompt
    c.send_line("X").await.unwrap();
    c.until("R to re-logon", WAIT).await.unwrap();
    c.send_line("R").await.unwrap();
    assert!(c.wait_closed(WAIT).await.unwrap());
    assert!(!c.plain().contains("Username:"));
    r.stop().await.unwrap();
}

/// Opens the door as `server_user` and expects the refusal and the close, never the menu.
async fn refused(r: &silvermere::Running, server_user: &str) {
    let mut c = rlogin(r, "M", server_user).await;
    let got = c.until("used from the BBS door", WAIT).await.unwrap();
    assert!(!got.plain.contains("Make your selection"), "{}", got.plain);
    assert!(c.wait_closed(WAIT).await.unwrap());
}

/// Opens the door as `server_user`, expects the menu and leaves.
async fn let_in(r: &silvermere::Running, server_user: &str) {
    let mut c = rlogin(r, "M", server_user).await;
    let got = c.until(MENU, WAIT).await.unwrap();
    assert!(!got.plain.contains("Password:"), "{}", got.plain);
    c.send_line("X").await.unwrap();
    c.until("R to re-logon", WAIT).await.unwrap();
    c.send_line("Y").await.unwrap();
    assert!(c.wait_closed(WAIT).await.unwrap());
}

/// Logs in over telnet with a password and leaves.
async fn telnet_login(r: &silvermere::Running, name: &str, password: &str) {
    let mut t = TelnetClient::connect(r.telnet_addr).await.unwrap();
    t.until("Username:", WAIT).await.unwrap();
    t.send_line(name).await.unwrap();
    t.until("Password:", WAIT).await.unwrap();
    t.send_line(password).await.unwrap();
    t.until(MENU, WAIT).await.unwrap();
    t.send_line("X").await.unwrap();
    t.until("R to re-logon", WAIT).await.unwrap();
    t.send_line("Y").await.unwrap();
    assert!(t.wait_closed(WAIT).await.unwrap());
}

/// Makes `name` with `password` and `roles`, and with the `door` flag when `door`.
async fn account(url: &str, name: &str, password: &str, roles: &[Role], door: bool) {
    let db = Db::connect_and_migrate(url).await.unwrap();
    create_with_cost(&db, name, password, roles, Cost::Low)
        .await
        .unwrap();
    if door {
        silvermere_auth::cli::grant(&db, name, "door".parse().unwrap())
            .await
            .unwrap();
    }
    db.close().await;
}

// The BBS's names are not this server's: a BBS caller who shares a telnet player's name
// does not get that player's account, which has no `door` flag.
#[tokio::test]
async fn an_unmarked_account_is_refused() {
    let (_t, url) = common::seeded(&[]).await;
    account(&url, "Dorin", "dorin123", &[Role::Player], false).await;
    let r = server(url, &[]).await;
    refused(&r, "dorin").await;
    // the account is untouched: its owner still logs in over telnet
    telnet_login(&r, "Dorin", "dorin123").await;
    r.stop().await.unwrap();
}

// With the `door` flag an account with a password is reached both ways: from the BBS
// door without the password, over telnet with it.
#[tokio::test]
async fn a_marked_account_works_from_the_door_and_telnet() {
    let (_t, url) = common::seeded(&[]).await;
    account(&url, "Dorin", "dorin123", &[Role::Player], true).await;
    let r = server(url, &[]).await;
    let_in(&r, "dorin").await;
    telnet_login(&r, "Dorin", "dorin123").await;
    r.stop().await.unwrap();
}

// Whoever registers `Warden` on the BBS does not become this server's sysop of that name;
// once the sysop marks the account for the door, a door sysop is fine.
#[tokio::test]
async fn a_sysop_account_needs_the_door_flag() {
    let (_t, url) = common::seeded(&[]).await;
    account(
        &url,
        "Warden",
        "warden123",
        &[Role::Player, Role::Sysop],
        false,
    )
    .await;
    let r = server(url.clone(), &[]).await;
    refused(&r, "warden").await;
    {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        silvermere_auth::cli::grant(&db, "Warden", "door".parse().unwrap())
            .await
            .unwrap();
        db.close().await;
    }
    let_in(&r, "warden").await;
    r.stop().await.unwrap();
}

// A first visit makes the account, asking only the gender; it has no password, so the
// telnet login cannot be used to reach it.
#[tokio::test]
async fn a_first_visit_asks_only_the_gender() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url.clone(), &[]).await;
    let mut c = rlogin(&r, "Brenna Vale", "").await;
    c.until("gender", WAIT).await.unwrap();
    c.send_line("x").await.unwrap();
    c.until("valid gender", WAIT).await.unwrap();
    c.send_line("F").await.unwrap();
    c.until(MENU, WAIT).await.unwrap();
    c.send_line("X").await.unwrap();
    c.until("R to re-logon", WAIT).await.unwrap();
    c.send_line("Y").await.unwrap();
    assert!(c.wait_closed(WAIT).await.unwrap());

    let mut t = TelnetClient::connect(r.telnet_addr).await.unwrap();
    t.until("Username:", WAIT).await.unwrap();
    t.send_line("Brenna Vale").await.unwrap();
    t.until("Password:", WAIT).await.unwrap();
    t.send_line("anything").await.unwrap();
    t.until("no password yet", WAIT).await.unwrap();
    r.stop().await.unwrap();

    let db = Db::connect_and_migrate(&url).await.unwrap();
    let account = silvermere_auth::AccountStore::by_username(&db, "brenna vale")
        .await
        .unwrap()
        .expect("the visit made the account");
    assert_eq!(account.username, "Brenna Vale");
    assert_eq!(account.sex, silvermere_auth::Sex::Female);
    assert!(account.password_hash.is_empty());
    assert!(account.flags.contains(silvermere_auth::AccountFlags::DOOR));
    db.close().await;
}

// A BBS that knows the caller's gender sends it as the client user (RedialBBS:
// `--remote-client "{gender}"`), and a first visit asks nothing at all.
#[tokio::test]
async fn a_first_visit_with_the_gender_asks_nothing() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url.clone(), &[]).await;
    let mut c = rlogin(&r, "F", "Cara Wynn").await;
    let got = c.until(MENU, WAIT).await.unwrap();
    assert!(!got.plain.contains("gender"), "{}", got.plain);
    c.send_line("X").await.unwrap();
    c.until("R to re-logon", WAIT).await.unwrap();
    c.send_line("Y").await.unwrap();
    assert!(c.wait_closed(WAIT).await.unwrap());
    r.stop().await.unwrap();

    let db = Db::connect_and_migrate(&url).await.unwrap();
    let account = silvermere_auth::AccountStore::by_username(&db, "cara wynn")
        .await
        .unwrap()
        .expect("the visit made the account");
    assert_eq!(account.sex, silvermere_auth::Sex::Female);
    db.close().await;
}

// Only the trusted addresses may say who is calling; anyone else is closed unanswered.
#[tokio::test]
async fn an_untrusted_address_is_closed() {
    let (_t, url) = common::seeded(&[]).await;
    let r = server(url, &["192.0.2.7"]).await;
    let mut c = rlogin(&r, "Aldric", "").await;
    assert!(c.wait_closed(WAIT).await.unwrap());
    assert!(c.raw().is_empty(), "{:?}", c.raw());
    r.stop().await.unwrap();
}
