//! What each connection's player sees, for a sysop watching over his shoulder.
//!
//! Every byte a session task puts on the wire for the player (the game's output and the echo
//! of what he types, before telnet escaping; not the telnet negotiation) is recorded here:
//! the last [`RECENT`] bytes, so a watcher starts with the screen as it stands, and a copy to
//! each watcher. A watcher that falls [`WATCHER_QUEUE`] writes behind is dropped (its stream
//! ends; it can watch again). The record ends with the connection.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::events::SessionId;

/// How many of the latest bytes are kept per connection.
pub const RECENT: usize = 8 * 1024;
/// How many writes a watcher may have waiting.
pub const WATCHER_QUEUE: usize = 256;

#[derive(Default)]
struct Screen {
    recent: VecDeque<u8>,
    watchers: Vec<mpsc::Sender<Vec<u8>>>,
}

/// The screens of the connections, shared by the session tasks and the watchers.
#[derive(Default)]
pub struct Screens {
    inner: Mutex<HashMap<SessionId, Screen>>,
}

impl std::fmt::Debug for Screens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Screens").finish_non_exhaustive()
    }
}

impl Screens {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Screen>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `bytes` went to connection `id`'s player.
    pub fn record(&self, id: SessionId, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut screens = self.lock();
        let screen = screens.entry(id).or_default();
        screen.recent.extend(bytes);
        let over = screen.recent.len().saturating_sub(RECENT);
        screen.recent.drain(..over);
        screen
            .watchers
            .retain(|w| w.try_send(bytes.to_vec()).is_ok());
    }

    /// The latest bytes of connection `id`'s screen and a stream of what follows; `None` when
    /// the connection is not known (gone, or nothing shown yet).
    pub fn watch(&self, id: SessionId) -> Option<(Vec<u8>, mpsc::Receiver<Vec<u8>>)> {
        let mut screens = self.lock();
        let screen = screens.get_mut(&id)?;
        let (tx, rx) = mpsc::channel(WATCHER_QUEUE);
        screen.watchers.push(tx);
        Some((screen.recent.iter().copied().collect(), rx))
    }

    /// How many watch connection `id` now.
    pub fn watchers(&self, id: SessionId) -> usize {
        self.lock().get_mut(&id).map_or(0, |s| {
            s.watchers.retain(|w| !w.is_closed());
            s.watchers.len()
        })
    }

    /// Connection `id` is gone: its record is dropped and its watchers' streams end.
    pub fn end(&self, id: SessionId) {
        self.lock().remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_watcher_gets_the_recent_screen_then_what_follows() {
        let s = Screens::default();
        let id = SessionId(3);
        assert!(s.watch(id).is_none());
        s.record(id, b"Hello ");
        let (recent, mut rx) = s.watch(id).unwrap();
        assert_eq!(recent, b"Hello ");
        s.record(id, b"world");
        assert_eq!(rx.recv().await.unwrap(), b"world");
        assert_eq!(s.watchers(id), 1);
        s.end(id);
        assert!(
            rx.recv().await.is_none(),
            "the stream ends with the connection"
        );
    }

    #[test]
    fn only_the_latest_bytes_are_kept_and_a_slow_watcher_is_dropped() {
        let s = Screens::default();
        let id = SessionId(1);
        s.record(id, &vec![b'a'; RECENT]);
        s.record(id, b"xyz");
        let (recent, _rx) = s.watch(id).unwrap();
        assert_eq!(recent.len(), RECENT);
        assert!(recent.ends_with(b"aaxyz"));
        for _ in 0..=WATCHER_QUEUE {
            s.record(id, b".");
        }
        assert_eq!(
            s.watchers(id),
            0,
            "a watcher that stopped reading is dropped"
        );
    }
}
