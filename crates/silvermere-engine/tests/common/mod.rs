//! Shared by the integration tests: a hand-driven clock, a tiny world and sessions in it.
//!
//! The world is four rooms on map 1 (the four special rooms of the default config), two
//! items, two known monsters, one class and one race, built from the engine's zeroed
//! records. T-014's testkit (once it can depend on this crate's fixtures) or a follow-up
//! replaces this with the shared fixture world.
#![allow(dead_code)]

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use silvermere_engine::player_session::Caps;
use silvermere_engine::scheduler::Clock;
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, PlayerSession, SeededRng, SessionId, World, WorldSnapshot,
    clear_player,
};
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{InGame, MessageLevel, SessionState, StatlineMode};
use silvermere_types::ids::{ClassNo, ItemNo, MapId, MonsterNo, RaceNo, RoomId, RoomKey};

/// A clock that only moves when told to; clones share one timeline.
#[derive(Debug, Clone)]
pub struct TestClock {
    base: Instant,
    offset: Arc<Mutex<Duration>>,
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    pub fn advance(&self, by: Duration) {
        *self.offset.lock().unwrap() += by;
    }

    /// Advances `n` fast ticks and injects one `Tick` input per tick.
    pub fn advance_ticks(&self, n: u32, engine: &mut Engine<TestClock, SeededRng>) {
        for _ in 0..n {
            self.advance(Duration::from_secs(1));
            engine.handle(EngineInput::Tick);
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }

    fn today(&self) -> jiff::civil::Date {
        self.wall().to_zoned(jiff::tz::TimeZone::UTC).date()
    }

    fn wall(&self) -> jiff::Timestamp {
        jiff::Timestamp::UNIX_EPOCH
            .checked_add(*self.offset.lock().unwrap())
            .expect("in range")
    }
}

/// The four special rooms of the default config, in order.
pub fn special_rooms() -> [u32; 4] {
    let g = GameConfig::default();
    [g.start_room, g.reinc_room, g.evil_reinc_room, g.jail_room]
}

/// The tiny world as a snapshot.
pub fn tiny_snapshot() -> WorldSnapshot {
    let mut s = WorldSnapshot::default();
    for (i, n) in special_rooms().into_iter().enumerate() {
        let mut r = blank::clear_room(RoomKey::new(1, n));
        r.name = format!("Room {}", i + 1);
        s.rooms.push(r);
    }
    s.items.push(blank::clear_item(ItemNo(1)));
    s.items.push(blank::clear_item(ItemNo(2)));
    s.monsters.push(blank::clear_known_monster(MonsterNo(1)));
    s.monsters.push(blank::clear_known_monster(MonsterNo(2)));
    s.classes.push(blank::clear_class(ClassNo(1)));
    s.races.push(blank::clear_race(RaceNo(1)));
    s
}

/// Fills map 1's missing room numbers up to `upto` with blank rooms, so the start-up room
/// pass reaches rooms numbered that high: it ends a map's scan at its 50th missing room
/// number (`MAXMISSROOMS`), and the stock maps have no gaps.
pub fn dense_map1(s: &mut WorldSnapshot, upto: u32) {
    let have: std::collections::HashSet<RoomKey> = s.rooms.iter().map(|r| r.key).collect();
    for n in 1..=upto {
        let k = RoomKey::new(1, n);
        if !have.contains(&k) {
            s.rooms.push(blank::clear_room(k));
        }
    }
}

/// The module defaults for the periods: the stock `WCCMMUD.MSG` that
/// `GameConfig::default()` follows sets `SECOTH` to 160 (so saves every 120 s after
/// rounding); the tests pin the module's own 60.
pub fn test_config() -> GameConfig {
    GameConfig {
        sec_oth: 60,
        ..GameConfig::default()
    }
}

/// The tiny world, loaded, with room for `max_sessions` sessions.
pub fn tiny_world(max_sessions: u16) -> World {
    World::load(test_config(), max_sessions, tiny_snapshot()).expect("tiny world loads")
}

/// An engine over the tiny world, with its clock and the output receiver.
pub fn engine(
    max_sessions: u16,
) -> (
    Engine<TestClock, SeededRng>,
    TestClock,
    Receiver<EngineOutput>,
) {
    let (tx, rx) = channel();
    let clock = TestClock::new();
    let engine = Engine::new(
        tiny_world(max_sessions),
        clock.clone(),
        SeededRng::new(1),
        tx,
    );
    (engine, clock, rx)
}

/// Everything the engine emitted so far.
pub fn drain(rx: &Receiver<EngineOutput>) -> Vec<EngineOutput> {
    let mut out = Vec::new();
    while let Ok(o) = rx.try_recv() {
        out.push(o);
    }
    out
}

/// Puts a playing character named `name` into `room` (map 1) on a fresh session and
/// returns its id. The character is `clear_player()` plus the fields given.
pub fn enter(world: &mut World, name: &str, room: u32, msglevel: MessageLevel) -> SessionId {
    let id = world.ids.alloc().expect("a free session id");
    let mut s = PlayerSession::new(id, Caps::default(), Instant::now());
    let mut c = clear_player();
    c.firstname = name.to_string();
    c.userid = name.to_ascii_uppercase();
    c.map = MapId(1);
    c.room = RoomId(room);
    c.msglevel = msglevel;
    c.hp = 10;
    c.maxhp = 10;
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.sex = b'M';
    c.statline = StatlineMode::On;
    s.character = Some(c);
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    world.sessions.insert(id, s);
    id
}

/// A sender/receiver pair for a spawned engine.
pub fn channels() -> (Sender<EngineInput>, Receiver<EngineInput>) {
    channel()
}
