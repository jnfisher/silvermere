//! Doubles shared by the API test files.
#![allow(dead_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use silvermere_api::events::{BoxFuture, LogError};
use silvermere_api::{EventLog, EventRecord, HealthProbe, NewEvent, WorldLoader};
use silvermere_engine::WorldSnapshot;

/// A [`WorldLoader`] that serves an empty snapshot, or fails when told to.
#[derive(Default)]
pub struct FakeWorld {
    pub fail: Mutex<Option<String>>,
    pub loads: AtomicUsize,
}

impl WorldLoader for FakeWorld {
    fn load(&self) -> BoxFuture<'_, Result<Box<WorldSnapshot>, LogError>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let failure = self.fail.lock().unwrap().clone();
        Box::pin(async move {
            match failure {
                Some(e) => Err(LogError::from(e)),
                None => Ok(Box::new(WorldSnapshot::default())),
            }
        })
    }
}

#[derive(Default)]
pub struct FakeLog(pub Mutex<Vec<EventRecord>>);

impl EventLog for FakeLog {
    fn append(&self, e: NewEvent) -> BoxFuture<'_, Result<EventRecord, LogError>> {
        let mut rows = self.0.lock().unwrap();
        let rec = EventRecord {
            id: rows.len() as i64 + 1,
            at: "2026-10-14T00:00:00Z".into(),
            kind: e.kind,
            session: e.session,
            character: e.character,
            room: e.room,
            payload: e.payload,
        };
        rows.push(rec.clone());
        Box::pin(async move { Ok(rec) })
    }

    fn latest_id(&self) -> BoxFuture<'_, Result<i64, LogError>> {
        let id = self.0.lock().unwrap().last().map_or(0, |r| r.id);
        Box::pin(async move { Ok(id) })
    }

    fn since(&self, after: i64, limit: u32) -> BoxFuture<'_, Result<Vec<EventRecord>, LogError>> {
        let rows: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.id > after)
            .take(limit as usize)
            .cloned()
            .collect();
        Box::pin(async move { Ok(rows) })
    }
}

pub struct FakeHealth {
    pub alive: AtomicBool,
    pub persist: AtomicBool,
    pub tick: AtomicU64,
}

impl HealthProbe for FakeHealth {
    fn engine_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    fn engine_tick(&self) -> u64 {
        self.tick.load(Ordering::SeqCst)
    }
    fn persist_healthy(&self) -> bool {
        self.persist.load(Ordering::SeqCst)
    }
}
