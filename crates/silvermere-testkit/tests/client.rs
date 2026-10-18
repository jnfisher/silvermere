//! The telnet client against an in-process listener that behaves like MBBSEmu's login:
//! four option offers, then text and a prompt (`bridge.py` `strip_telnet`, `plain`).

use std::time::Duration;

use silvermere_testkit::{TelnetClient, TestServer, plain};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const IAC: u8 = 255;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const WELCOME: &[u8] = b"Welcome to the BBS\r\n";
const PROMPT: &[u8] = b"\r\n\x1b[79D\x1b[K\x1b[1;32m[HP=30]:\x1b[0m ";

#[tokio::test]
async fn refuses_every_do_records_raw_without_iac_and_waits_for_the_prompt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (replies_tx, replies_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        // four offers (DO ECHO, DO SGA, DO TTYPE, DO NAWS), a WILL that gets no answer,
        // and a subnegotiation to skip
        s.write_all(&[IAC, DO, 1, IAC, DO, 3, IAC, DO, 24, IAC, DO, 31])
            .await
            .unwrap();
        s.write_all(&[IAC, WILL, 5, IAC, SB, 24, 1, IAC, SE])
            .await
            .unwrap();
        let mut got = [0u8; 12];
        s.read_exact(&mut got).await.unwrap();
        replies_tx.send(got).unwrap();
        s.write_all(WELCOME).await.unwrap();
        s.write_all(PROMPT).await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let mut client = TelnetClient::connect(addr).await.unwrap();
    let got = client
        .until(r"\[HP=", Duration::from_secs(5))
        .await
        .unwrap();

    let replies = replies_rx.await.unwrap();
    assert_eq!(
        replies,
        [IAC, WONT, 1, IAC, WONT, 3, IAC, WONT, 24, IAC, WONT, 31],
        "one WONT per DO and nothing for the WILL"
    );
    let expected_raw = [WELCOME, PROMPT].concat();
    assert_eq!(
        client.raw(),
        expected_raw,
        "raw is the stream minus telnet commands"
    );
    assert_eq!(got.raw, expected_raw);
    assert_eq!(got.plain, "Welcome to the BBS\n\n[HP=30]: ");
    assert_eq!(client.plain(), got.plain);
    assert!(got.matched);
}

#[tokio::test]
async fn iac_iac_is_one_0xff_even_across_reads() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        s.write_all(&[b'a', IAC, IAC, b'b', IAC]).await.unwrap();
        s.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        s.write_all(&[IAC, b'c', IAC, DO]).await.unwrap();
        s.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        s.write_all(&[7, b'd']).await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let mut client = TelnetClient::connect(addr).await.unwrap();
    client.until("d", Duration::from_secs(5)).await.unwrap();
    assert_eq!(
        client.raw(),
        [b'a', 0xFF, b'b', 0xFF, b'c', b'd'],
        "the DO 7 is consumed, not data"
    );
}

#[tokio::test]
async fn naws_is_answered_when_a_size_is_given() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        s.write_all(&[IAC, DO, 31]).await.unwrap();
        let mut got = [0u8; 12];
        s.read_exact(&mut got).await.unwrap();
        tx.send(got).unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    let mut client = TelnetClient::connect(addr).await.unwrap().with_naws(79, 24);
    client.read(Duration::from_millis(100)).await.unwrap();
    assert_eq!(
        rx.await.unwrap(),
        [IAC, WILL, 31, IAC, SB, 31, 0, 79, 0, 24, IAC, SE]
    );
}

#[tokio::test]
async fn until_times_out_with_what_it_saw() {
    let server = TestServer::echo().await.unwrap();
    let mut client = server.client().await.unwrap();
    let err = client
        .until("never appears", Duration::from_millis(300))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("never appears") && msg.contains("Welcome"),
        "{msg}"
    );
}

#[tokio::test]
async fn echo_server_round_trip_reports_skipped_text() {
    let server = TestServer::echo().await.unwrap();
    let mut client = server.client().await.unwrap();
    // let the greeting arrive but do not read it: it comes back as `skipped` on the send
    tokio::time::sleep(Duration::from_millis(200)).await;
    let skipped = client.send_line("hello").await.unwrap();
    assert_eq!(skipped, plain("Welcome to the test server.\r\nName: "));
    let got = client
        .until("Name: $", Duration::from_secs(5))
        .await
        .unwrap();
    assert!(got.plain.contains("You typed: hello"), "{:?}", got.plain);
    assert_eq!(got.skipped, skipped);
    client.send_line("quit").await.unwrap();
    assert!(client.wait_closed(Duration::from_secs(5)).await.unwrap());
}
