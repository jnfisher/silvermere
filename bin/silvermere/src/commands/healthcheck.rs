//! `silvermere healthcheck`: GET `/healthz` on the admin port, for the image's HEALTHCHECK
//! (distroless has no curl). Std-only: a hand-written HTTP/1.0 request over `TcpStream`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;

const TIMEOUT: Duration = Duration::from_secs(2);

/// `healthcheck` flags.
#[derive(Debug, Args)]
pub struct HealthcheckArgs {
    /// Health endpoint to probe (plain http only).
    #[arg(long, default_value = "http://127.0.0.1:8080/healthz")]
    pub url: String,
}

/// Exit 0 on an HTTP 200, else 1 with one line on stderr.
pub fn run(args: &HealthcheckArgs) -> ExitCode {
    match probe(&args.url) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("healthcheck failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn probe(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("only http:// URLs are supported")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let addr = host
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host}: no address"))?;
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(|e| format!("{host}: {e}"))?;
    s.set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    write!(
        s,
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| format!("send: {e}"))?;
    let mut head = [0u8; 32];
    let mut n = 0;
    while n < head.len() {
        match s.read(&mut head[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    let line = String::from_utf8_lossy(&head[..n]);
    let status = line.split_whitespace().nth(1).unwrap_or("");
    if status == "200" {
        Ok(())
    } else {
        Err(format!(
            "unexpected response {:?}",
            line.lines().next().unwrap_or("")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Serves one connection with `reply` (or never replies when `None`) and returns the URL.
    fn fake(reply: Option<&'static str>) -> (String, std::thread::JoinHandle<()>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/healthz", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            // Read the whole request so closing never resets the connection under the client.
            let mut req = Vec::new();
            let mut buf = [0u8; 256];
            while !req.ends_with(b"\r\n\r\n") {
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            match reply {
                Some(r) => c.write_all(r.as_bytes()).unwrap(),
                None => std::thread::sleep(TIMEOUT + Duration::from_millis(500)),
            }
        });
        (url, h)
    }

    #[test]
    fn ok_on_200() {
        let (url, h) = fake(Some("HTTP/1.1 200 OK\r\n\r\n{}"));
        assert!(probe(&url).is_ok());
        h.join().unwrap();
    }

    #[test]
    fn err_on_503() {
        let (url, h) = fake(Some("HTTP/1.1 503 Service Unavailable\r\n\r\n"));
        assert!(probe(&url).unwrap_err().contains("503"));
        h.join().unwrap();
    }

    #[test]
    fn err_on_refused() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/healthz", l.local_addr().unwrap());
        drop(l);
        assert!(probe(&url).is_err());
    }

    #[test]
    fn err_on_timeout() {
        let (url, h) = fake(None);
        assert!(probe(&url).unwrap_err().contains("read"));
        h.join().unwrap();
    }

    #[test]
    fn rejects_https() {
        assert!(probe("https://127.0.0.1:8080/healthz").is_err());
    }
}
