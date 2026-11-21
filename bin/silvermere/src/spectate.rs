//! Which connection each engine session is on, for the sysop who watches a player's screen
//! ([`silvermere_net::Screens`]) or types for him ([`SessionCommand::Inject`]). The output
//! bridge keeps it: a session is linked when the engine opens it and unlinked when it closes.

use std::collections::HashMap;
use std::sync::Mutex;

use silvermere_engine::SessionId;
use silvermere_net::{SessionCommand, SessionId as NetId};
use tokio::sync::mpsc;

/// The engine sessions' connections.
#[derive(Debug, Default)]
pub struct Links(Mutex<HashMap<SessionId, (NetId, mpsc::Sender<SessionCommand>)>>);

impl Links {
    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<SessionId, (NetId, mpsc::Sender<SessionCommand>)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The engine opened `id` on connection `net`.
    pub fn link(&self, id: SessionId, net: NetId, tx: mpsc::Sender<SessionCommand>) {
        self.lock().insert(id, (net, tx));
    }

    /// The engine closed `id`.
    pub fn unlink(&self, id: SessionId) {
        self.lock().remove(&id);
    }

    /// The connection `id` is on.
    pub fn of(&self, id: SessionId) -> Option<(NetId, mpsc::Sender<SessionCommand>)> {
        self.lock().get(&id).cloned()
    }
}

/// A write to a player's screen as text: UTF-8 when it is (`SET UTF8 ON`), else CP437.
pub fn screen_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => silvermere_net::cp437::decode(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp437_and_utf8_screens_read_as_text() {
        assert_eq!(screen_text(b"\x1b[1;32mHi\r\n"), "\u{1b}[1;32mHi\r\n");
        assert_eq!(screen_text(&[0xb0, 0xb1]), "░▒");
        assert_eq!(screen_text("░▒".as_bytes()), "░▒");
    }
}
