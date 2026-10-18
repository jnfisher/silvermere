//! `serve_in_process` loads the world, binds the telnet port, runs the engine and stops.

mod common;

use std::time::Duration;

use silvermere::config::Config;
use silvermere_engine::{EngineInput, SeededRng, SystemClock};

#[tokio::test]
async fn starts_serves_and_stops() {
    let (_t, url) = common::seeded(&[]).await;
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7)),
    )
    .await
    .expect("within 5 s")
    .expect("starts");
    let _conn = tokio::net::TcpStream::connect(r.telnet_addr)
        .await
        .expect("connect");
    // The engine opens a session for the connection: ask it through the admin channel
    // until the session shows up.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        // `Who` lists only players in the realm, so count the sessions on the world thread.
        let (tx, rx) = std::sync::mpsc::channel();
        r.to_engine
            .send(EngineInput::Custom {
                session: None,
                run: Box::new(move |w| {
                    let _ = tx.send(w.sessions.len());
                }),
            })
            .unwrap();
        let sessions = tokio::task::spawn_blocking(move || rx.recv().unwrap())
            .await
            .unwrap();
        if sessions == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no session after 5 s: {sessions}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let world = tokio::time::timeout(Duration::from_secs(5), r.stop())
        .await
        .expect("stops within 5 s")
        .expect("engine joins");
    assert_eq!(world.rooms.len(), 4);
    assert_eq!(world.sessions.len(), 1, "the connection became a session");
}

#[tokio::test]
async fn refuses_to_start_without_a_special_room() {
    let (_t, url) = common::seeded(&[376]).await;
    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    let err = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .err()
        .expect("refused");
    assert_eq!(
        err.to_string(),
        "game.jail_room = 376: room does not exist on map 1"
    );
}
