//! The admin API's connections to the rest of the process: the `events` table behind
//! [`EventLog`], [`HealthProbe`] over the engine's liveness, and the `ApiState` the
//! listener is started with.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use metrics_exporter_prometheus::PrometheusHandle;
use silvermere_api::events::{BoxFuture, EventRecord, LogError, NewEvent, RoomRef};
use silvermere_api::{ApiState, EventLog, HealthProbe, WorldLoader};
use silvermere_db::events::StoredEvent;
use silvermere_db::{Db, Event, PersistStatus};
use silvermere_engine::{EngineInput, GameEvent, WorldSnapshot};
use silvermere_types::ids::{CharacterId, MapId, RoomId, RoomKey, SessionId};
use tokio::sync::{broadcast, mpsc, watch};

use crate::config::Config;

/// [`EventLog`] over `silvermere_db::Db::events()`.
pub struct DbEventLog {
    db: Arc<Db>,
}

impl DbEventLog {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

fn record(stored: StoredEvent) -> EventRecord {
    let StoredEvent { id, event } = stored;
    EventRecord {
        id,
        at: event.at.to_string(),
        kind: event.kind,
        session: event.session.map(|s| s.0),
        character: event.character.map(|c| c.0),
        room: event.room.map(|r| RoomRef {
            map: r.map.0,
            room: r.room.0,
        }),
        payload: event.payload,
    }
}

impl EventLog for DbEventLog {
    fn append(&self, ev: NewEvent) -> BoxFuture<'_, Result<EventRecord, LogError>> {
        Box::pin(async move {
            let event = Event {
                at: jiff::Timestamp::now(),
                kind: ev.kind,
                session: ev.session.map(SessionId),
                character: ev.character.map(CharacterId),
                room: ev.room.map(|r| RoomKey {
                    map: MapId(r.map),
                    room: RoomId(r.room),
                }),
                payload: ev.payload,
            };
            let id = self.db.events().append(&event).await?;
            Ok(record(StoredEvent { id, event }))
        })
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, LogError>> {
        Box::pin(async move {
            let rows = self.db.events().recent(1).await?;
            Ok(rows.first().map_or(0, |r| r.id))
        })
    }

    fn since(
        &self,
        after_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<EventRecord>, LogError>> {
        Box::pin(async move {
            let rows = self.db.events().since(after_id, u64::from(limit)).await?;
            Ok(rows.into_iter().map(record).collect())
        })
    }
}

/// [`WorldLoader`] over the repositories: the same read `World::load` starts from.
pub struct DbWorldLoader {
    db: Arc<Db>,
}

impl DbWorldLoader {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

impl WorldLoader for DbWorldLoader {
    fn load(&self) -> BoxFuture<'_, Result<Box<WorldSnapshot>, LogError>> {
        Box::pin(async move {
            let snapshot = crate::bridge::load_snapshot(&self.db)
                .await
                .map_err(|e| LogError::from(e.to_string()))?;
            Ok(Box::new(snapshot))
        })
    }
}

/// What the process knows about itself, shared with the tasks that update it.
#[derive(Debug)]
pub struct Vitals {
    /// Cleared when the engine's output channel closes (it stopped or died).
    pub engine_alive: Arc<AtomicBool>,
    /// Fast ticks sent to the engine.
    pub ticks: Arc<AtomicU64>,
    /// The persistence worker's status (`store.rs`), when one is running.
    pub persist: Option<watch::Receiver<PersistStatus>>,
}

impl Vitals {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            engine_alive: Arc::new(AtomicBool::new(true)),
            ticks: Arc::new(AtomicU64::new(0)),
            persist: None,
        })
    }

    /// As [`Vitals::new`], watching the persistence worker's status.
    pub fn with_persist(persist: watch::Receiver<PersistStatus>) -> Arc<Self> {
        Arc::new(Self {
            engine_alive: Arc::new(AtomicBool::new(true)),
            ticks: Arc::new(AtomicU64::new(0)),
            persist: Some(persist),
        })
    }
}

impl Vitals {
    pub fn engine_alive_flag(&self) -> Arc<AtomicBool> {
        self.engine_alive.clone()
    }
    pub fn ticks_counter(&self) -> Arc<AtomicU64> {
        self.ticks.clone()
    }
}

impl HealthProbe for Vitals {
    fn engine_alive(&self) -> bool {
        self.engine_alive.load(Ordering::Relaxed)
    }
    fn engine_tick(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
    fn persist_healthy(&self) -> bool {
        self.persist
            .as_ref()
            .is_none_or(|p| *p.borrow() == PersistStatus::Healthy)
    }
}

/// The process-wide metrics recorder, installed once (tests start many servers).
fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| match silvermere_api::metrics::install() {
            Ok(h) => h,
            // Another recorder owns the facade; serve an empty one rather than refuse.
            Err(_) => {
                tracing::warn!("another metrics recorder is installed; /metrics will be empty");
                silvermere_api::metrics::build().1
            }
        })
        .clone()
}

/// Builds the API state: the engine sender, the event bus, the table, the probe, the
/// metrics handle, the configured tokens and the backup schedule's state.
pub fn api_state(
    cfg: &Config,
    db: Arc<Db>,
    engine: std::sync::mpsc::Sender<EngineInput>,
    vitals: Arc<Vitals>,
    backup: Arc<dyn silvermere_api::BackupProbe>,
) -> ApiState {
    if cfg.admin.legacy_env_token
        && matches!(
            silvermere_api::is_loopback_bind(&cfg.server.admin_bind),
            Ok(false)
        )
    {
        tracing::warn!(
            bind = %cfg.server.admin_bind,
            "SILVERMERE_ADMIN_TOKEN gives a token with every scope and the admin API is not on loopback; issue named tokens with `silvermere admin token new`"
        );
    }
    ApiState {
        engine,
        events: ApiState::event_bus(),
        log: Arc::new(DbEventLog::new(db.clone())),
        world: Arc::new(DbWorldLoader::new(db)),
        health: vitals,
        metrics: metrics_handle(),
        tokens: Arc::new(cfg.admin.tokens.iter().map(|t| t.to_token()).collect()),
        backup,
    }
}

/// Appends each game event to the table and puts the stored row on the bus, one path so
/// the table and the stream agree on ids.
pub async fn forward_game_events(
    mut events: mpsc::UnboundedReceiver<GameEvent>,
    log: Arc<dyn EventLog>,
    bus: broadcast::Sender<EventRecord>,
) {
    while let Some(ev) = events.recv().await {
        match log.append(NewEvent::from_game_event(&ev)).await {
            Ok(rec) => {
                // No receiver is fine: nobody is streaming.
                let _ = bus.send(rec);
            }
            Err(e) => tracing::warn!(%e, kind = ev.kind(), "event row not written"),
        }
    }
}
