//! Graceful shutdown: tell the players, save every character, flush the persistence queue, close.
//!
//! `bin` stops the acceptor, sends [`EngineControl::Shutdown`] and waits for the engine's
//! reply and for the barrier ([`PersistRequest::Flush`]) to come back from the persistence
//! worker. Players are logged off the way a normal logoff does it (`World::logoff`: the
//! last write, then `Close`), never through the hang-up path, so nobody meets the
//! `PUNHUP` penalty on the next entry.

use std::sync::mpsc::Sender;
use std::time::Duration;

use silvermere_text::ansi::ANSI_CLRLINE;
use silvermere_text::{cp437, finish};

use crate::world::World;

/// What every connected player is told. The original has no such text (its host went down
/// with the BBS), so the wording is ours.
pub const NOTICE: &str = "The server is shutting down. Your character has been saved.";

/// Longest the net layer may take to drain a session's queue before the socket closes.
pub const DRAIN_BOUND: Duration = Duration::from_secs(2);

/// Orders for the engine that are not admin API requests.
#[derive(Debug)]
pub enum EngineControl {
    /// Stop cleanly: see [`begin`]. `grace` is how long `bin` lets the sessions drain.
    Shutdown {
        grace: Duration,
        reply: Sender<ShutdownReport>,
    },
}

/// What [`begin`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShutdownReport {
    /// Sessions that were told.
    pub notified: usize,
    /// Characters a save was emitted for, filled in when the event's flush has run.
    pub saved: usize,
    /// Sessions queued to close.
    pub closed: usize,
}

/// The shutdown handler: (a) the notice to every session, (b) every session logs off
/// the way a normal logoff does (`World::logoff`): its last write, `Close`, and at the end
/// of the event's flush the `lofrou` save (`save_player` at the logoff save point, with its
/// guards; no second save path). The barrier ([`PersistRequest::Flush`]) and the reply
/// follow the last of those saves, in `Engine::flush`. Nothing here can fail; a session
/// with nothing to save is still told and closed.
pub fn begin(world: &mut World) -> ShutdownReport {
    let mut prf = ANSI_CLRLINE.to_vec();
    prf.extend_from_slice(&cp437::encode(NOTICE));
    prf.push(b'\r');
    let notice = finish(&prf, None);
    let mut report = ShutdownReport::default();
    for id in world.session_ids() {
        if let Some(s) = world.session_mut(id) {
            s.output.push_raw(notice.clone());
            report.notified += 1;
        }
        world.touch(id);
        world.logoff(id);
        report.closed += 1;
    }
    report
}
