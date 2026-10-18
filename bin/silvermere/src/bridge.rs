//! The adapters between the engine and the crates it never imports: the world snapshot
//! from `silvermere-db`, the session events from `silvermere-net`, the fast tick.
//!
//! The engine runs on its own thread and talks over one `std::sync::mpsc` channel in
//! and one out (`silvermere_engine::event_loop`). This module merges the sources the
//! plan's `select!` names (session tasks, the 1 s tick, admin requests) into that one
//! input channel and fans the outputs back to the session tasks.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use silvermere_db::Db;
use silvermere_engine::{Caps, ConnId, EngineInput, EngineOutput, WorldSnapshot};
use silvermere_net::{SessionCommand, SessionEvent, cp437};
use silvermere_types::sizes::NTOPLIST;
use tokio::sync::mpsc as tmpsc;

use crate::gate::{GateControl, GateEvent};

/// Everything `World::load` needs, read once through the repositories.
pub async fn load_snapshot(db: &Db) -> anyhow::Result<WorldSnapshot> {
    let started = std::time::Instant::now();
    let snapshot = read_snapshot(db).await?;
    metrics::histogram!(silvermere_engine::metrics::DB_LATENCY_SECONDS, "op" => "load")
        .record(started.elapsed());
    Ok(snapshot)
}

async fn read_snapshot(db: &Db) -> anyhow::Result<WorldSnapshot> {
    Ok(WorldSnapshot {
        rooms: db.rooms().all().await?,
        items: db.items().all().await?,
        monsters: db.monsters().all().await?,
        active_monsters: db.active_monsters().all().await?,
        spells: db.spells().all().await?,
        classes: db.classes().all().await?,
        races: db.races().all().await?,
        shops: db.shops().all().await?,
        messages: db.messages().all().await?,
        text_blocks: db.text_blocks().all().await?,
        actions: db.actions().all().await?,
        gangs: db.gangs().all().await?,
        item_ownership: db.item_ownership().all().await?,
        bank_books: db.bank_books().all().await?,
        // Profiles and evil points are per account: loaded at login (T-021+).
        profiles: Vec::new(),
        evil_points: Vec::new(),
        top_characters: db
            .characters()
            .top_by_exp(NTOPLIST)
            .await?
            .into_iter()
            .map(|row| row.character)
            .collect(),
    })
}

/// The net crate's connection number, as the engine's [`ConnId`].
fn conn_id(id: silvermere_net::SessionId) -> ConnId {
    ConnId(u64::from(id.0))
}

/// Where each connection's events go: the engine id once the engine has opened the
/// session, a wait list before that, and a note of the connections the engine closed or
/// refused whose transport `Disconnect` has yet to arrive.
#[derive(Default)]
struct Routes {
    ids: HashMap<ConnId, silvermere_engine::SessionId>,
    /// Events that arrived before the engine answered `Connected`, per connection.
    waiting: HashMap<ConnId, Vec<GateEvent>>,
    /// Closed or refused by the engine; the transport's `Disconnect` is still to come
    /// and is dropped (the id may be reused), which also removes the note.
    forgotten: std::collections::HashSet<ConnId>,
}

fn is_disconnect(ev: &GateEvent) -> bool {
    matches!(ev, GateEvent::Net(SessionEvent::Disconnect(_)))
}

impl Routes {
    /// The engine answered for `conn`: the events that were waiting, translated.
    fn opened(
        &mut self,
        conn: ConnId,
        id: Option<silvermere_engine::SessionId>,
    ) -> Vec<EngineInput> {
        match id {
            Some(id) => {
                self.ids.insert(conn, id);
                self.waiting
                    .remove(&conn)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|ev| translate(id, ev))
                    .collect()
            }
            // Refused, or closed by the engine: the connection's later events (its
            // `Disconnect`) must not reach an id that may be reused.
            None => {
                self.ids.remove(&conn);
                let disconnected = self
                    .waiting
                    .remove(&conn)
                    .is_some_and(|evs| evs.iter().any(is_disconnect));
                // Remember the connection while its `Disconnect` is still to come. (A
                // client that hung up first has no engine close after it, so the one
                // note a hang-up racing the close could leave is the whole cost.)
                if !disconnected {
                    self.forgotten.insert(conn);
                }
                Vec::new()
            }
        }
    }

    /// An event from `conn`'s transport: the engine input to send, if any.
    fn event(&mut self, conn: ConnId, ev: GateEvent) -> Option<EngineInput> {
        if let Some(&id) = self.ids.get(&conn) {
            let closing = is_disconnect(&ev);
            let input = translate(id, ev);
            if closing {
                self.ids.remove(&conn);
            }
            return input;
        }
        if self.forgotten.contains(&conn) {
            if is_disconnect(&ev) {
                self.forgotten.remove(&conn);
            }
            return None;
        }
        self.waiting.entry(conn).or_default().push(ev);
        None
    }
}

/// Forwards session events to the engine, translating the net connection number to the
/// engine's dense session id once the engine has opened the session. Runs until the net
/// side closes.
pub async fn forward_events(
    mut events: tmpsc::Receiver<(silvermere_net::SessionId, GateEvent)>,
    mut opened: tmpsc::UnboundedReceiver<(ConnId, Option<silvermere_engine::SessionId>)>,
    to_engine: mpsc::Sender<EngineInput>,
) {
    let mut routes = Routes::default();
    loop {
        tokio::select! {
            Some((conn, id)) = opened.recv() => {
                for input in routes.opened(conn, id) {
                    if to_engine.send(input).is_err() {
                        return;
                    }
                }
            }
            ev = events.recv() => {
                let Some((net_id, ev)) = ev else { return };
                let conn = conn_id(net_id);
                if let GateEvent::Net(SessionEvent::Connected(caps)) = ev {
                    let caps = Caps { columns: caps.columns, rows: caps.rows, ..Caps::default() };
                    if to_engine.send(EngineInput::Connected { conn, caps }).is_err() {
                        return;
                    }
                    continue;
                }
                if let Some(input) = routes.event(conn, ev)
                    && to_engine.send(input).is_err() {
                    return;
                }
            }
        }
    }
}

fn translate(id: silvermere_engine::SessionId, ev: GateEvent) -> Option<EngineInput> {
    let ev = match ev {
        GateEvent::Net(ev) => ev,
        GateEvent::LoggedIn(login) => return Some(EngineInput::LoggedIn { id, login }),
    };
    Some(match ev {
        SessionEvent::Connected(_) => return None,
        SessionEvent::Line(bytes) => EngineInput::Line {
            id,
            text: cp437::decode(&bytes),
        },
        // The overflow message is the line discipline's (T-024 decides the text).
        SessionEvent::LineTooLong => return None,
        SessionEvent::Key(byte) => EngineInput::Key { id, byte },
        SessionEvent::Naws(columns, rows) => EngineInput::Naws { id, columns, rows },
        SessionEvent::Disconnect(_) => EngineInput::Disconnect { id },
    })
}

/// Where the engine's non-session outputs go: game events to the admin event log, saves
/// and name checks to the store task (`store.rs`).
#[derive(Debug, Clone)]
pub struct Sinks {
    pub events: tmpsc::UnboundedSender<silvermere_engine::GameEvent>,
    pub store: tmpsc::UnboundedSender<crate::store::Job>,
    /// Reads the game options again for SYSOP RELOAD (T-055); `None` in tests.
    pub reload: Option<ConfigReloader>,
}

/// Reads the configuration file again and gives its `[game]` options (T-055).
#[derive(Clone)]
pub struct ConfigReloader(
    pub std::sync::Arc<dyn Fn() -> Option<silvermere_types::config::GameConfig> + Send + Sync>,
);

impl std::fmt::Debug for ConfigReloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfigReloader")
    }
}

impl Sinks {
    /// Sinks whose receivers are already gone (tests).
    pub fn closed() -> Self {
        Self {
            events: tmpsc::unbounded_channel().0,
            store: tmpsc::unbounded_channel().0,
            reload: None,
        }
    }
}

/// Fans engine outputs out to the session tasks. `registry` carries each new
/// connection's command sender (the net crate sends it before the session's first
/// event). Returns when the engine reports `Stopped` or drops its sender.
pub async fn forward_outputs(
    outputs: mpsc::Receiver<EngineOutput>,
    mut registry: tmpsc::UnboundedReceiver<(
        silvermere_net::SessionId,
        tmpsc::Sender<SessionCommand>,
    )>,
    opened: tmpsc::UnboundedSender<(ConnId, Option<silvermere_engine::SessionId>)>,
    full_message: Vec<u8>,
    gate: tmpsc::UnboundedSender<GateControl>,
    sinks: Sinks,
    engine_alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let Sinks {
        events,
        store,
        reload,
    } = sinks;
    // The engine's std channel is read on a blocking thread and relayed here.
    let (relay_tx, mut relay) = tmpsc::unbounded_channel();
    let reader = tokio::task::spawn_blocking(move || {
        while let Ok(out) = outputs.recv() {
            let stopped = matches!(out, EngineOutput::Stopped);
            if relay_tx.send(out).is_err() || stopped {
                break;
            }
        }
    });
    let mut by_conn: HashMap<ConnId, tmpsc::Sender<SessionCommand>> = HashMap::new();
    let mut by_id: HashMap<silvermere_engine::SessionId, ConnId> = HashMap::new();
    // Refusals the engine answered before the connection's registry entry arrived.
    let mut refused: std::collections::HashSet<ConnId> = std::collections::HashSet::new();
    loop {
        tokio::select! {
            // Registry first: a session's entry is sent before the engine can answer for it,
            // so draining it ahead of the relay means Opened/Write never find the entry
            // missing (which would silently skip the write and its drop accounting).
            biased;
            Some((net_id, tx)) = registry.recv() => {
                let conn = conn_id(net_id);
                if refused.remove(&conn) {
                    let _ = tx.try_send(SessionCommand::Write(full_message.clone()));
                    let _ = tx.try_send(SessionCommand::Close);
                } else {
                    by_conn.insert(conn, tx);
                }
            }
            out = relay.recv() => {
                let Some(out) = out else { break };
                match out {
                    EngineOutput::Opened { conn, id } => {
                        by_id.insert(id, conn);
                        let _ = opened.send((conn, Some(id)));
                    }
                    EngineOutput::Refused { conn } => {
                        let _ = opened.send((conn, None));
                        match by_conn.remove(&conn) {
                            Some(tx) => {
                                let _ = tx.try_send(SessionCommand::Write(full_message.clone()));
                                let _ = tx.try_send(SessionCommand::Close);
                            }
                            None => {
                                refused.insert(conn);
                            }
                        }
                    }
                    EngineOutput::Write { id, bytes } => send(&by_conn, &by_id, id, SessionCommand::Write(bytes)),
                    EngineOutput::SetEcho { id, mode } => {
                        let mode = match mode {
                            silvermere_engine::EchoMode::On => silvermere_net::EchoMode::On,
                            silvermere_engine::EchoMode::Off => silvermere_net::EchoMode::Off,
                            silvermere_engine::EchoMode::Secure { ch, width } => silvermere_net::EchoMode::Secure { ch, width },
                        };
                        send(&by_conn, &by_id, id, SessionCommand::SetEcho(mode));
                    }
                    EngineOutput::SetCharMode { id, on } => send(&by_conn, &by_id, id, SessionCommand::SetCharMode(on)),
                    EngineOutput::LineStarted { id } => send(&by_conn, &by_id, id, SessionCommand::LineStarted),
                    EngineOutput::LineDone { id } => send(&by_conn, &by_id, id, SessionCommand::LineDone),
                    EngineOutput::Close { id } => {
                        send(&by_conn, &by_id, id, SessionCommand::Close);
                        if let Some(conn) = by_id.remove(&id) {
                            by_conn.remove(&conn);
                            let _ = opened.send((conn, None));
                        }
                    }
                    EngineOutput::ReturnToLogin { id } => {
                        if let Some(&conn) = by_id.get(&id) {
                            let _ = gate.send(GateControl::ReturnToLogin(conn));
                        }
                    }
                    // One path to the `events` table and the SSE bus (`admin::forward_game_events`).
                    EngineOutput::Event(ev) => {
                        tracing::debug!(?ev, "game event");
                        let _ = events.send(ev);
                    }
                    // Saves and name checks, in order, to the store task (`store.rs`).
                    EngineOutput::Persist(req) => {
                        let _ = store.send(crate::store::Job::Persist(req));
                    }
                    EngineOutput::NameCheck { id, name } => {
                        let _ = store.send(crate::store::Job::NameCheck { id, name });
                    }
                    EngineOutput::Roster { id } => {
                        let _ = store.send(crate::store::Job::Roster { id });
                    }
                    EngineOutput::ReloadConfig { id } => {
                        let game = reload.as_ref().and_then(|r| (r.0)()).map(Box::new);
                        let _ = store.send(crate::store::Job::Reload { id, game });
                    }
                    EngineOutput::GangQuery { id, query } => {
                        let _ = store.send(crate::store::Job::GangQuery { id, query });
                    }
                    EngineOutput::CleanupScan => {
                        let _ = store.send(crate::store::Job::CleanupScan);
                    }
                    EngineOutput::AccountDeleteScan { account } => {
                        let _ = store.send(crate::store::Job::AccountDeleteScan { account });
                    }
                    EngineOutput::Stopped => break,
                }
            }
        }
    }
    reader.abort();
    // Stopped or dropped: either way the engine is no longer running.
    engine_alive.store(false, std::sync::atomic::Ordering::Relaxed);
}

fn send(
    by_conn: &HashMap<ConnId, tmpsc::Sender<SessionCommand>>,
    by_id: &HashMap<silvermere_engine::SessionId, ConnId>,
    id: silvermere_engine::SessionId,
    cmd: SessionCommand,
) {
    if let Some(tx) = by_id.get(&id).and_then(|c| by_conn.get(c)) {
        // `try_send`: a stuck session must never block the fan-out.
        let is_write = matches!(cmd, SessionCommand::Write(_));
        if tx.try_send(cmd).is_err() && is_write {
            metrics::counter!(silvermere_engine::metrics::OUTPUT_DROPPED_TOTAL).increment(1);
        }
    }
}

/// Sends `EngineInput::Tick` every `period` until the engine drops its receiver or
/// `stop` flips.
pub async fn ticker(
    period: Duration,
    to_engine: mpsc::Sender<EngineInput>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    ticks: std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if to_engine.send(EngineInput::Tick).is_err() {
                    return;
                }
                ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            _ = stop.changed() => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disconnect() -> GateEvent {
        GateEvent::Net(SessionEvent::Disconnect(silvermere_net::Reason::Eof))
    }

    /// After the engine closes a session, the transport's later `Disconnect` is dropped
    /// and leaves nothing behind (T-021 review, silvermere#26).
    #[test]
    fn a_disconnect_after_an_engine_close_is_dropped_not_queued() {
        let mut r = Routes::default();
        let (conn, id) = (ConnId(7), silvermere_engine::SessionId(2));
        assert!(r.opened(conn, Some(id)).is_empty());
        assert!(r.opened(conn, None).is_empty(), "engine closed the session");
        assert!(r.event(conn, disconnect()).is_none());
        assert!(r.waiting.is_empty(), "nothing queued under the closed conn");
        assert!(r.forgotten.is_empty(), "the note goes with the Disconnect");
        assert!(r.ids.is_empty());
    }

    /// A refused connection's `Disconnect` is dropped the same way, and one that beat
    /// the refusal leaves no note.
    #[test]
    fn refused_connections_leave_nothing_behind() {
        let mut r = Routes::default();
        let conn = ConnId(8);
        assert!(r.event(conn, disconnect()).is_none());
        assert!(r.opened(conn, None).is_empty());
        assert!(r.waiting.is_empty() && r.forgotten.is_empty());
        let conn = ConnId(9);
        assert!(r.opened(conn, None).is_empty());
        assert!(r.event(conn, disconnect()).is_none());
        assert!(r.waiting.is_empty() && r.forgotten.is_empty());
    }

    /// Events that beat the engine's answer are delivered, in order, once it comes.
    #[test]
    fn early_events_are_delivered_when_the_session_opens() {
        let mut r = Routes::default();
        let conn = ConnId(1);
        let line = GateEvent::Net(SessionEvent::Line(b"hi".to_vec()));
        assert!(r.event(conn, line).is_none());
        let sent = r.opened(conn, Some(silvermere_engine::SessionId(0)));
        assert_eq!(sent.len(), 1);
        assert!(matches!(sent[0], EngineInput::Line { .. }));
    }

    /// A `Refused` that beats the registry entry still sends the full message and closes.
    #[tokio::test]
    async fn refusal_before_registry_entry_is_buffered() {
        let (out_tx, out_rx) = mpsc::channel();
        let (reg_tx, reg_rx) = tmpsc::unbounded_channel();
        let (opened_tx, mut opened_rx) = tmpsc::unbounded_channel();
        let task = tokio::spawn(forward_outputs(
            out_rx,
            reg_rx,
            opened_tx,
            b"full\r\n".to_vec(),
            tmpsc::unbounded_channel().0,
            Sinks::closed(),
            Default::default(),
        ));
        out_tx
            .send(EngineOutput::Refused { conn: ConnId(3) })
            .unwrap();
        assert_eq!(opened_rx.recv().await, Some((ConnId(3), None)));
        let (cmd_tx, mut cmd_rx) = tmpsc::channel(8);
        reg_tx.send((silvermere_net::SessionId(3), cmd_tx)).unwrap();
        assert_eq!(
            cmd_rx.recv().await,
            Some(SessionCommand::Write(b"full\r\n".to_vec()))
        );
        assert_eq!(cmd_rx.recv().await, Some(SessionCommand::Close));
        out_tx.send(EngineOutput::Stopped).unwrap();
        task.await.unwrap();
    }
}
