//! A TCP proxy that delays every byte by a fixed one-way time, for tests of how the
//! store behaves over a real network (T-113): a hosted PostgreSQL in another region costs
//! 25-35 ms per round trip, which loopback hides.
//!
//! Each direction of each connection is relayed with the same delay, so a round trip
//! costs twice it while bytes in flight still pipeline (bandwidth is not limited). The
//! proxy also counts round trips: the times a client sends after the server last spoke.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// A running proxy; stops, with every connection it relays, when dropped.
#[derive(Debug)]
pub struct LatencyProxy {
    port: u16,
    round_trips: Arc<AtomicU64>,
    accept: JoinHandle<()>,
    blocked: Arc<AtomicBool>,
    relays: Relays,
}

impl LatencyProxy {
    /// Listens on a free loopback port and relays each connection to `upstream`
    /// (`host:port`), delaying each direction by `one_way`.
    pub async fn start(upstream: &str, one_way: Duration) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let round_trips = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&round_trips);
        let blocked = Arc::new(AtomicBool::new(false));
        let relays: Relays = Arc::default();
        let (is_blocked, live) = (Arc::clone(&blocked), Arc::clone(&relays));
        let upstream = upstream.to_string();
        let accept = tokio::spawn(async move {
            loop {
                let client = match listener.accept().await {
                    Ok((client, _)) => client,
                    Err(e) => {
                        // a full file table and the like: say so, then keep accepting
                        eprintln!("latency proxy: accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                if is_blocked.load(Ordering::Relaxed) {
                    drop(client);
                    continue;
                }
                // the upstream connect off the accept loop, so connections open in parallel
                let (upstream, counter, conns) =
                    (upstream.clone(), Arc::clone(&counter), Arc::clone(&live));
                let connect = tokio::spawn(async move {
                    let Ok(server) = TcpStream::connect(&upstream).await else {
                        return;
                    };
                    let _ = client.set_nodelay(true);
                    let _ = server.set_nodelay(true);
                    // whether the server spoke last on this connection
                    let server_spoke = Arc::new(AtomicBool::new(true));
                    let (cr, cw) = client.into_split();
                    let (sr, sw) = server.into_split();
                    let up = Turns {
                        counter: Some(counter),
                        server_spoke: Arc::clone(&server_spoke),
                    };
                    let down = Turns {
                        counter: None,
                        server_spoke,
                    };
                    let mut conns = lock(&conns);
                    conns.retain(|h| !h.is_finished());
                    conns.push(tokio::spawn(relay(cr, sw, one_way, up)).abort_handle());
                    conns.push(tokio::spawn(relay(sr, cw, one_way, down)).abort_handle());
                });
                lock(&live).push(connect.abort_handle());
            }
        });
        Ok(Self {
            port,
            round_trips,
            accept,
            blocked,
            relays,
        })
    }

    /// Blocks the link, as a firewall rule dropping the database port would: every open
    /// connection is cut and new ones are refused until it is unblocked.
    pub fn set_blocked(&self, blocked: bool) {
        self.blocked.store(blocked, Ordering::Relaxed);
        if blocked {
            for h in lock(&self.relays).drain(..) {
                h.abort();
            }
        }
    }

    /// The port to connect to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Round trips seen so far, over every connection.
    pub fn round_trips(&self) -> u64 {
        self.round_trips.load(Ordering::Relaxed)
    }
}

impl Drop for LatencyProxy {
    fn drop(&mut self) {
        self.accept.abort();
        for h in lock(&self.relays).drain(..) {
            h.abort();
        }
    }
}

type Relays = Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>;

fn lock(relays: &Relays) -> std::sync::MutexGuard<'_, Vec<tokio::task::AbortHandle>> {
    relays
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Which way a relay goes, for counting round trips: the client-to-server relay counts a
/// trip whenever it sends after the server spoke; the other one notes that it spoke.
struct Turns {
    counter: Option<Arc<AtomicU64>>,
    server_spoke: Arc<AtomicBool>,
}

async fn relay(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
    turns: Turns,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let writer = tokio::spawn(async move {
        while let Some((due, bytes)) = rx.recv().await {
            tokio::time::sleep_until(due).await;
            if to.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = to.shutdown().await;
    });
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        match &turns.counter {
            Some(c) => {
                if turns.server_spoke.swap(false, Ordering::Relaxed) {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            }
            None => turns.server_spoke.store(true, Ordering::Relaxed),
        }
        if tx
            .send((Instant::now() + delay, buf[..n].to_vec()))
            .is_err()
        {
            break;
        }
    }
    drop(tx);
    let _ = writer.await;
}

/// `url` (a `postgres://` URL) with its host and port replaced by `127.0.0.1:port`, and
/// the `host:port` it named (port 5432 when it named none).
pub fn redirect_url(url: &str, port: u16) -> Option<(String, String)> {
    let scheme_end = url.find("://")? + 3;
    let rest = &url[scheme_end..];
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(at) => (&authority[..=at], &authority[at + 1..]),
        None => ("", authority),
    };
    // a port follows the last `:`, outside an IPv6 literal's brackets
    let has_port = match hostport.rfind(']') {
        Some(end) => hostport[end..].contains(':'),
        None => hostport.contains(':'),
    };
    let upstream = if has_port {
        hostport.to_string()
    } else {
        format!("{hostport}:5432")
    };
    let redirected = format!(
        "{}{userinfo}127.0.0.1:{port}{}",
        &url[..scheme_end],
        &rest[authority_end..]
    );
    Some((redirected, upstream))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_keeps_everything_but_the_host() {
        assert_eq!(
            redirect_url("postgres://u:p@db.example:6543/d?sslmode=disable", 9),
            Some((
                "postgres://u:p@127.0.0.1:9/d?sslmode=disable".to_string(),
                "db.example:6543".to_string()
            ))
        );
        assert_eq!(
            redirect_url("postgres://u@[::1]/d", 9).map(|(_, up)| up),
            Some("[::1]:5432".to_string())
        );
        assert_eq!(
            redirect_url("postgres://u@[::1]:6543/d", 9).map(|(_, up)| up),
            Some("[::1]:6543".to_string())
        );
        assert_eq!(
            redirect_url("postgresql://h/d", 9),
            Some((
                "postgresql://127.0.0.1:9/d".to_string(),
                "h:5432".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn a_round_trip_costs_twice_the_delay() {
        let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = echo.accept().await.unwrap();
            let mut b = [0u8; 4];
            while s.read_exact(&mut b).await.is_ok() {
                s.write_all(&b).await.unwrap();
            }
        });
        let proxy = LatencyProxy::start(&addr.to_string(), Duration::from_millis(20))
            .await
            .unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        let started = std::time::Instant::now();
        for _ in 0..3 {
            c.write_all(b"ping").await.unwrap();
            let mut b = [0u8; 4];
            c.read_exact(&mut b).await.unwrap();
            assert_eq!(&b, b"ping");
        }
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(120), "{took:?}");
        assert!(took < Duration::from_millis(600), "{took:?}");
        assert_eq!(proxy.round_trips(), 3);
    }
}
