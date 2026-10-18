//! Option negotiation over loopback (in-process listener on port 0).

use std::time::Duration;

use silvermere_net::telnet::options::{BINARY, ECHO, LINEMODE, NAWS, SGA, TTYPE};
use silvermere_net::telnet::parser::{DO, DONT, IAC, SB, SE, WILL, WONT};
use silvermere_net::{
    Capabilities, SessionCommand, SessionEvent, SessionId, TelnetTransport, Transport,
    spawn_session,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(2);

struct Harness {
    client: TcpStream,
    events: mpsc::Receiver<(SessionId, SessionEvent)>,
    commands: mpsc::Sender<SessionCommand>,
}

async fn start() -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    let (tx, events) = mpsc::channel(16);
    let commands = spawn_session(SessionId(1), server, tx);
    let mut h = Harness {
        client,
        events,
        commands,
    };
    assert_eq!(
        h.event().await,
        SessionEvent::Connected(Capabilities::default())
    );
    h.expect(&[
        IAC, WILL, ECHO, IAC, WILL, SGA, IAC, WILL, BINARY, IAC, DO, NAWS,
    ])
    .await;
    h
}

impl Harness {
    async fn event(&mut self) -> SessionEvent {
        timeout(WAIT, self.events.recv()).await.unwrap().unwrap().1
    }

    async fn expect(&mut self, bytes: &[u8]) {
        let mut got = vec![0; bytes.len()];
        timeout(WAIT, self.client.read_exact(&mut got))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, bytes);
    }

    async fn expect_silence(&mut self) {
        let mut b = [0u8; 1];
        let r = timeout(Duration::from_millis(150), self.client.read(&mut b)).await;
        assert!(r.is_err(), "unexpected byte {:?}", b);
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.client.write_all(bytes).await.unwrap();
    }
}

#[tokio::test]
async fn accept_sends_exactly_the_four_offers_in_order() {
    let mut h = start().await;
    h.expect_silence().await;
}

#[tokio::test]
async fn do_echo_is_accepted() {
    let mut h = start().await;
    h.send(&[IAC, DO, ECHO]).await;
    h.expect(&[IAC, WILL, ECHO]).await;
}

#[tokio::test]
async fn do_ttype_is_refused() {
    let mut h = start().await;
    h.send(&[IAC, DO, TTYPE]).await;
    h.expect(&[IAC, WONT, TTYPE]).await;
}

#[tokio::test]
async fn naws_reports_capabilities() {
    let mut h = start().await;
    h.send(&[IAC, WILL, NAWS, IAC, SB, NAWS, 0, 100, 0, 40, IAC, SE])
        .await;
    assert_eq!(h.event().await, SessionEvent::Naws(100, 40));
}

#[tokio::test]
async fn will_linemode_is_refused() {
    let mut h = start().await;
    h.send(&[IAC, WILL, LINEMODE]).await;
    h.expect(&[IAC, DONT, LINEMODE]).await;
}

/// The harness bridge's `strip_telnet` (`tools/differential/vendor/play/bridge.py`): answers every DO with WONT, ignores WILL.
#[tokio::test]
async fn client_refusing_everything_still_gets_lines_through() {
    let mut h = start().await;
    h.send(&[IAC, WONT, NAWS]).await;
    h.send(b"hello\r").await;
    assert_eq!(h.event().await, SessionEvent::Line(b"hello".to_vec()));
    h.expect(b"hello\r\n").await;
    // Refusals are never retried.
    h.expect_silence().await;
}

#[tokio::test]
async fn ff_in_output_arrives_doubled() {
    let mut h = start().await;
    h.commands
        .send(SessionCommand::Write(vec![b'a', 0xFF, b'b']))
        .await
        .unwrap();
    h.expect(&[b'a', 0xFF, 0xFF, b'b']).await;
}

#[tokio::test]
async fn typing_is_echoed_at_receive_time() {
    let mut h = start().await;
    h.send(b"Ka").await;
    h.expect(b"Ka").await;
}

#[tokio::test]
async fn telnet_transport_negotiates_and_strips() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    let mut t = TelnetTransport::accept(server).await.unwrap();

    let mut offers = [0u8; 12];
    client.read_exact(&mut offers).await.unwrap();
    client
        .write_all(&[
            IAC, SB, NAWS, 0, 100, 0, 40, IAC, SE, b'h', IAC, IAC, IAC, DO, TTYPE,
        ])
        .await
        .unwrap();
    let mut buf = [0u8; 16];
    let n = timeout(WAIT, t.read(&mut buf)).await.unwrap().unwrap();
    assert_eq!(&buf[..n], &[b'h', 0xFF]);
    assert_eq!(
        t.capabilities(),
        Capabilities {
            columns: 100,
            rows: 40
        }
    );
    let mut reply = [0u8; 3];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [IAC, WONT, TTYPE]);

    t.write_all(&[0xFF]).await.unwrap();
    let mut out = [0u8; 2];
    client.read_exact(&mut out).await.unwrap();
    assert_eq!(out, [0xFF, 0xFF]);
    t.close().await;
}
