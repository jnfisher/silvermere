//! The session task over loopback: ordering, backpressure, disconnects, the accept loop.

use std::net::SocketAddr;
use std::time::Duration;

use silvermere_net::{
    Reason, ServerConfig, SessionCommand, SessionConfig, SessionEvent, SessionId, bind, serve,
    spawn_session, spawn_session_with,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};

const LOOPBACK: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);

async fn pair() -> (
    TcpStream,
    mpsc::Receiver<(SessionId, SessionEvent)>,
    mpsc::Sender<SessionCommand>,
) {
    let listener = bind(LOOPBACK, &ServerConfig::default()).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    // A small channel, so the engine side is the slow one.
    let (tx, mut events) = mpsc::channel(4);
    let commands = spawn_session(SessionId(7), server, tx);
    let (id, first) = events.recv().await.unwrap();
    assert_eq!(id, SessionId(7));
    assert!(matches!(first, SessionEvent::Connected(_)));
    (client, events, commands)
}

async fn next(events: &mut mpsc::Receiver<(SessionId, SessionEvent)>) -> SessionEvent {
    timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("event in time")
        .expect("channel open")
        .1
}

#[tokio::test]
async fn burst_of_1000_lines_arrives_in_order() {
    let (mut client, mut events, commands) = pair().await;
    // Drain echo so the client's own buffers never stall the server.
    let (mut rd, mut wr) = client.split();
    let burst: Vec<u8> = (0..1000)
        .flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect();
    let drain = async {
        let mut sink = vec![0u8; 65536];
        while rd.read(&mut sink).await.unwrap_or(0) > 0 {}
    };
    let check = async {
        wr.write_all(&burst).await.unwrap();
        for i in 0..1000 {
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Line(format!("line {i}").into_bytes())
            );
            // the engine has run the line (it acknowledges, as the engine's output does)
            commands.send(SessionCommand::LineDone).await.unwrap();
            // The engine is slow: the session must hold, not drop.
            if i % 100 == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    };
    tokio::select! {
        () = check => {}
        () = drain => panic!("connection closed early"),
    }
}

#[tokio::test]
async fn client_that_never_reads_is_dropped_for_backpressure() {
    let (_client, mut events, commands) = pair().await;
    let start = Instant::now();
    let chunk = vec![b'x'; 16 * 1024];
    for _ in 0..64 {
        // 1 MB in total; the session may already be gone part way through.
        if commands
            .send(SessionCommand::Write(chunk.clone()))
            .await
            .is_err()
        {
            break;
        }
    }
    loop {
        if let SessionEvent::Disconnect(reason) = next(&mut events).await {
            assert_eq!(reason, Reason::Backpressure);
            break;
        }
    }
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn socket_drop_is_eof() {
    let (client, mut events, _commands) = pair().await;
    drop(client);
    assert_eq!(
        next(&mut events).await,
        SessionEvent::Disconnect(Reason::Eof)
    );
}

#[tokio::test]
async fn lines_typed_before_a_drop_are_delivered_first() {
    let (mut client, mut events, _commands) = pair().await;
    client.write_all(b"quit\r\n").await.unwrap();
    client.shutdown().await.unwrap();
    assert_eq!(
        next(&mut events).await,
        SessionEvent::Line(b"quit".to_vec())
    );
    assert_eq!(
        next(&mut events).await,
        SessionEvent::Disconnect(Reason::Eof)
    );
}

#[tokio::test]
async fn close_flushes_output_then_disconnects() {
    let (mut client, mut events, commands) = pair().await;
    commands
        .send(SessionCommand::Write(b"Bye!\r\n".to_vec()))
        .await
        .unwrap();
    commands.send(SessionCommand::Close).await.unwrap();
    let mut got = Vec::new();
    timeout(Duration::from_secs(2), client.read_to_end(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert!(got.ends_with(b"Bye!\r\n"));
    assert_eq!(
        next(&mut events).await,
        SessionEvent::Disconnect(Reason::Closed)
    );
}

#[tokio::test]
async fn char_mode_and_secure_echo_follow_commands() {
    let (mut client, mut events, commands) = pair().await;
    let mut offers = [0u8; 12];
    client.read_exact(&mut offers).await.unwrap();

    commands
        .send(SessionCommand::SetEcho(silvermere_net::EchoMode::Secure {
            ch: b'*',
            width: 8,
        }))
        .await
        .unwrap();
    commands
        .send(SessionCommand::Write(b"Password: ".to_vec()))
        .await
        .unwrap();
    let mut prompt = [0u8; 10];
    client.read_exact(&mut prompt).await.unwrap();
    client.write_all(b"pw\r\n").await.unwrap();
    assert_eq!(next(&mut events).await, SessionEvent::Line(b"pw".to_vec()));
    let mut echo = [0u8; 4];
    client.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"**\r\n");

    commands
        .send(SessionCommand::SetCharMode(true))
        .await
        .unwrap();
    commands
        .send(SessionCommand::Write(b">".to_vec()))
        .await
        .unwrap();
    let mut b = [0u8; 1];
    client.read_exact(&mut b).await.unwrap();
    client.write_all(b"q").await.unwrap();
    assert_eq!(next(&mut events).await, SessionEvent::Key(b'q'));
}

/// T-094: with `hold_line_echo`, what the engine writes before it starts a line (a tick's
/// prompt redraw) goes out ahead of the line's CR echo, and the line's own output follows the
/// echo directly; typed characters still echo at once, with a tick's output between them.
#[tokio::test]
async fn a_lines_cr_echo_waits_for_the_engine_to_start_the_line() {
    let listener = bind(LOOPBACK, &ServerConfig::default()).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    let (tx, mut events) = mpsc::channel(4);
    let config = SessionConfig {
        hold_line_echo: true,
        ..SessionConfig::default()
    };
    let commands = spawn_session_with(SessionId(7), server, tx, config);
    assert!(matches!(
        next(&mut events).await,
        SessionEvent::Connected(_)
    ));
    let mut offers = [0u8; 12];
    client.read_exact(&mut offers).await.unwrap();

    async fn read_n(client: &mut TcpStream, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        timeout(Duration::from_secs(5), client.read_exact(&mut buf))
            .await
            .expect("bytes in time")
            .unwrap();
        buf
    }

    client.write_all(b"lo").await.unwrap();
    assert_eq!(read_n(&mut client, 2).await, b"lo");
    // a tick between typed characters lands there, as on the original
    commands
        .send(SessionCommand::Write(b"[T1]".to_vec()))
        .await
        .unwrap();
    assert_eq!(read_n(&mut client, 4).await, b"[T1]");
    client.write_all(b"ok\r\n").await.unwrap();
    assert_eq!(
        next(&mut events).await,
        SessionEvent::Line(b"look".to_vec())
    );
    assert_eq!(read_n(&mut client, 2).await, b"ok");
    // a tick queued before the engine took the line, then the line itself
    for command in [
        SessionCommand::Write(b"[T2]".to_vec()),
        SessionCommand::LineStarted,
        SessionCommand::Write(b"output".to_vec()),
        SessionCommand::LineDone,
    ] {
        commands.send(command).await.unwrap();
    }
    assert_eq!(read_n(&mut client, 12).await, b"[T2]\r\noutput");
}

#[tokio::test]
async fn serve_refuses_connections_over_the_cap() {
    let config = ServerConfig {
        max_sessions: 1,
        ..ServerConfig::default()
    };
    let listener = bind(LOOPBACK, &config).unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut events) = mpsc::channel(16);
    let (reg_tx, mut registry) = mpsc::unbounded_channel();
    tokio::spawn(serve(listener, config, tx, reg_tx));

    let _first = TcpStream::connect(addr).await.unwrap();
    let (id, _commands) = registry.recv().await.unwrap();
    assert_eq!(
        next(&mut events).await,
        SessionEvent::Connected(Default::default())
    );
    assert_eq!(id, SessionId(1));

    let mut second = TcpStream::connect(addr).await.unwrap();
    let mut got = Vec::new();
    timeout(Duration::from_secs(2), second.read_to_end(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"The server is full. Please try again later.\r\n");
}

#[tokio::test]
async fn heartbeat_sends_nop_when_idle() {
    use silvermere_net::{SessionConfig, spawn_session_with};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    let (tx, _events) = mpsc::channel(4);
    let config = SessionConfig {
        heartbeat: Some(Duration::from_millis(50)),
        ..SessionConfig::default()
    };
    let _commands = spawn_session_with(SessionId(1), server, tx, config);
    let mut got = [0u8; 14];
    timeout(Duration::from_secs(2), client.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got[12..], &[0xFF, 241]);
}

/// The engine's `LineDone` for the line before can already be waiting in the command queue when
/// the next line arrives. The task used to apply it only after it had skipped feeding the held
/// input, leaving the line in the inbox with nothing armed to wake the task (reading is off while
/// the inbox is non-empty, the hold timer is off once the hold is cleared): the session never
/// echoed or passed on the line, and never saw the client close.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_line_behind_a_held_one_is_taken_when_line_done_is_already_queued() {
    let (mut client, mut events, commands) = pair().await;
    for round in 0..400 {
        let a = format!("a{round}");
        let b = format!("b{round}");
        client
            .write_all(format!("{a}\r\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Line(a.clone().into_bytes())
        );
        // the next line and the previous line's acknowledgement reach the task together
        client
            .write_all(format!("{b}\r\n").as_bytes())
            .await
            .unwrap();
        commands.send(SessionCommand::LineDone).await.unwrap();
        let got = timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap_or_else(|_| panic!("round {round}: the line was never taken"))
            .expect("channel open")
            .1;
        assert_eq!(
            got,
            SessionEvent::Line(b.clone().into_bytes()),
            "round {round}"
        );
        commands.send(SessionCommand::LineDone).await.unwrap();
        // drain the echo so the client's buffers never matter
        let mut sink = [0u8; 256];
        while let Ok(Ok(n)) = timeout(Duration::from_millis(5), client.read(&mut sink)).await {
            if n == 0 {
                break;
            }
        }
    }
}
