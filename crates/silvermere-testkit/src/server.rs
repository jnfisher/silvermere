//! An in-process telnet server for exercising the client and scripts.
//!
//! [`TestServer::echo`] runs the real `silvermere-net` acceptor with a tiny stand-in for
//! the engine (a greeting, an echo of each line, `quit` to hang up). The full server
//! (`serve_in_process` with the engine, a `TestClock` and a seeded RNG, SQLite in a temp
//! dir, the tiny fixture world) needs T-015 and the T-008 fixture import; it joins here
//! behind a `server` feature when they merge, and the `silvermere` binary crate cannot be
//! a dependency of this one meanwhile because it dev-depends on this crate.

use std::collections::HashMap;
use std::net::SocketAddr;

use silvermere_net::{ServerConfig, SessionCommand, SessionEvent, bind, cp437, serve};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::client::TelnetClient;

/// A running echo server on a random local port.
pub struct TestServer {
    addr: SocketAddr,
    tasks: Vec<JoinHandle<()>>,
}

impl TestServer {
    /// Greeting sent to each new connection.
    pub const GREETING: &'static str = "Welcome to the test server.\r\nName: ";

    /// Starts the echo server on `127.0.0.1:0`.
    pub async fn echo() -> std::io::Result<Self> {
        let config = ServerConfig::default();
        let listener = bind("127.0.0.1:0".parse().expect("addr"), &config)?;
        let addr = listener.local_addr()?;
        let (tx, mut events) = mpsc::channel(64);
        let (reg_tx, mut registry) = mpsc::unbounded_channel();
        let acceptor = tokio::spawn(async move {
            let _ = serve(listener, config, tx, reg_tx).await;
        });
        let engine = tokio::spawn(async move {
            let mut sessions = HashMap::new();
            while let Some((id, event)) = events.recv().await {
                while let Ok((new_id, commands)) = registry.try_recv() {
                    sessions.insert(new_id, commands);
                }
                let Some(out) = sessions.get(&id) else {
                    continue;
                };
                let reply = match event {
                    SessionEvent::Connected(_) => Self::GREETING.as_bytes().to_vec(),
                    SessionEvent::Line(line) if line == b"quit" => {
                        let _ = out.try_send(SessionCommand::Write(b"Bye.\r\n".to_vec()));
                        let _ = out.try_send(SessionCommand::Close);
                        continue;
                    }
                    SessionEvent::Line(line) => {
                        format!("You typed: {}\r\nName: ", cp437::decode(&line)).into_bytes()
                    }
                    SessionEvent::Disconnect(_) => {
                        sessions.remove(&id);
                        continue;
                    }
                    _ => continue,
                };
                let _ = out.try_send(SessionCommand::Write(reply));
                let _ = out.try_send(SessionCommand::LineDone);
            }
        });
        Ok(Self {
            addr,
            tasks: vec![acceptor, engine],
        })
    }

    /// The bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// A new client connected to the server.
    pub async fn client(&self) -> std::io::Result<TelnetClient> {
        TelnetClient::connect(self.addr).await
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}
