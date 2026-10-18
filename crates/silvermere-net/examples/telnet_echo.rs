//! Manual check for the telnet transport (docs/net.md): serves on 127.0.0.1:2323 (or the
//! address given as the first argument), prints a prompt, and repeats each line back.

use silvermere_net::{EchoMode, ServerConfig, SessionCommand, SessionEvent, bind, cp437, serve};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:2323".to_owned())
        .parse()
        .expect("socket address");
    let config = ServerConfig::default();
    let listener = bind(addr, &config)?;
    let (tx, mut events) = mpsc::channel(64);
    let (reg_tx, mut registry) = mpsc::unbounded_channel();
    tokio::spawn(serve(listener, config, tx, reg_tx));

    let mut sessions = std::collections::HashMap::new();
    while let Some((id, event)) = events.recv().await {
        while let Ok((new_id, commands)) = registry.try_recv() {
            sessions.insert(new_id, commands);
        }
        let Some(out) = sessions.get(&id) else {
            continue;
        };
        let reply = match event {
            SessionEvent::Connected(_) => b"Name: ".to_vec(),
            SessionEvent::Line(line) if line == b"pw" => {
                let _ = out.try_send(SessionCommand::SetEcho(EchoMode::Secure {
                    ch: b'*',
                    width: 8,
                }));
                b"Password: ".to_vec()
            }
            SessionEvent::Line(line) => {
                format!("You typed: {}\r\nName: ", cp437::decode(&line)).into_bytes()
            }
            SessionEvent::LineTooLong => b"Line too long.\r\nName: ".to_vec(),
            SessionEvent::Disconnect(_) => {
                sessions.remove(&id);
                continue;
            }
            _ => continue,
        };
        let _ = out.try_send(SessionCommand::Write(reply));
    }
    Ok(())
}
