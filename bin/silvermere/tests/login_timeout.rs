//! `net.login_timeout_secs`: a connection that never types in the login dialogue is
//! closed (T-021 review, silvermere#26).

mod common;

use std::time::Duration;

use silvermere::config::Config;
use silvermere_engine::{SeededRng, SystemClock};
use silvermere_testkit::TelnetClient;

#[tokio::test]
async fn an_idle_login_dialogue_is_closed() {
    let (_t, url) = common::seeded(&[]).await;
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    cfg.net.login_timeout_secs = 1;
    let r = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(3))
        .await
        .unwrap();
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client
        .until("Username:", Duration::from_secs(30))
        .await
        .unwrap();
    // Say nothing: the server closes the socket, so waiting for more output ends in an
    // error rather than a timeout of ours.
    let started = std::time::Instant::now();
    let res = client.until("NEVER-APPEARS", Duration::from_secs(20)).await;
    assert!(res.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "closed by the server, not by our wait: {:?}",
        started.elapsed()
    );
    r.stop().await.unwrap();
}
