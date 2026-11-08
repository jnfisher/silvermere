//! The rlogin door login over a socket (`server.rlogin_bind`): a BBS that knows its
//! caller opens the game with the caller's name and the caller is not asked to log in
//! again; a first visit asks only the gender; an address that is not trusted is closed;
//! `R` at the BBS menu shim goes back to the BBS.

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

// An existing account goes straight to the menu: no username, no password.
#[tokio::test]
async fn an_existing_account_skips_the_login() {
    let (_t, url) = common::seeded(&[]).await;
    {
        let db = Db::connect_and_migrate(&url).await.unwrap();
        create_with_cost(&db, "Aldric", "aldric1", &[Role::Player], Cost::Low)
            .await
            .unwrap();
        db.close().await;
    }
    let r = server(url, &[]).await;
    let mut c = rlogin(&r, "Aldric", "").await;
    let got = c.until(MENU, WAIT).await.unwrap();
    assert!(!got.plain.contains("Username:"), "{}", got.plain);
    assert!(!got.plain.contains("Password:"), "{}", got.plain);
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
