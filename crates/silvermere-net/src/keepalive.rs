//! TCP keepalive on accepted connections (`net.keepalive_secs`).
//!
//! A player whose Wi-Fi drops or whose laptop sleeps leaves a connection that is open on
//! the server's side and gone on theirs. Nothing arrives on it and, while the player is
//! idle, nothing is sent, so the session would stay in the game, and the account
//! "already logged on", until the inactivity kick. Keepalive probes find such a
//! connection within a couple of minutes; a live client's system answers them without
//! the player doing anything, so an idle player is never disconnected by them.

use std::io;
use std::time::Duration;

use tokio::net::TcpStream;

/// Time between probes once the connection has been quiet for the idle time.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(15);
/// Unanswered probes before the connection is dropped.
pub const PROBES: u32 = 4;

/// How long after its last sign of life a dead connection is dropped: the idle time and
/// then every probe unanswered.
pub fn dead_after(idle: Duration) -> Duration {
    idle + PROBE_INTERVAL * PROBES
}

/// Turns keepalive on for `stream`: the first probe after `idle` without traffic, then
/// one every [`PROBE_INTERVAL`], dropped after [`PROBES`] unanswered.
///
/// On Linux the same bound also covers output the peer never acknowledges
/// (`TCP_USER_TIMEOUT`): keepalive probes are not sent while unacknowledged data is
/// outstanding, and the kernel's own retransmissions would take about fifteen minutes to
/// give up.
pub fn enable(stream: &TcpStream, idle: Duration) -> io::Result<()> {
    let sock = socket2::SockRef::from(stream);
    let params = socket2::TcpKeepalive::new().with_time(idle);
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    let params = params.with_interval(PROBE_INTERVAL).with_retries(PROBES);
    sock.set_tcp_keepalive(&params)?;
    #[cfg(target_os = "linux")]
    sock.set_tcp_user_timeout(Some(dead_after(idle)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_connection_goes_two_minutes_after_the_default_idle_time() {
        assert_eq!(
            dead_after(Duration::from_secs(60)),
            Duration::from_secs(120)
        );
    }

    #[tokio::test]
    async fn an_accepted_socket_takes_the_settings() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(TcpStream::connect(addr));
        let (stream, _) = listener.accept().await.unwrap();
        enable(&stream, Duration::from_secs(60)).unwrap();
        let sock = socket2::SockRef::from(&stream);
        assert!(sock.keepalive().unwrap());
        #[cfg(target_os = "linux")]
        {
            assert_eq!(sock.tcp_keepalive_time().unwrap(), Duration::from_secs(60));
            assert_eq!(sock.tcp_keepalive_interval().unwrap(), PROBE_INTERVAL);
            assert_eq!(sock.tcp_keepalive_retries().unwrap(), PROBES);
            assert_eq!(
                sock.tcp_user_timeout().unwrap(),
                Some(Duration::from_secs(120))
            );
        }
        drop(client.await);
    }
}
