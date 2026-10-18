//! Shared by the Layer-3 tests: a temp SQLite seeded with the tiny fixture world.
//!
//! The world is the four special rooms of the default config on map 1, two items, two
//! known monsters, one class and one race, built from the engine's zeroed records
//! (`silvermere_engine::world::blank`). A follow-up moves this to `silvermere-testkit`
//! once that crate can host it.
#![allow(dead_code)]

use silvermere_db::{Db, TestDb};
use silvermere_engine::world::blank;
use silvermere_types::config::GameConfig;
use silvermere_types::ids::{ClassNo, ItemNo, MonsterNo, RaceNo, RoomKey};
use silvermere_types::records::Room;

/// The four special rooms of the default config.
pub fn special_rooms() -> [u32; 4] {
    let g = GameConfig::default();
    [g.start_room, g.reinc_room, g.evil_reinc_room, g.jail_room]
}

/// The fixture rooms.
pub fn rooms() -> Vec<Room> {
    special_rooms()
        .into_iter()
        .enumerate()
        .map(|(i, n)| {
            let mut r = blank::clear_room(RoomKey::new(1, n));
            r.name = format!("Room {}", i + 1);
            r
        })
        .collect()
}

/// Inserts the fixture world, minus the rooms in `without`.
pub async fn seed(db: &Db, without: &[u32]) {
    for r in rooms() {
        if without.contains(&r.key.room.0) {
            continue;
        }
        db.rooms().insert(&r).await.unwrap();
    }
    for n in [1, 2] {
        db.items()
            .insert(&blank::clear_item(ItemNo(n)))
            .await
            .unwrap();
        db.monsters()
            .insert(&blank::clear_known_monster(MonsterNo(n)))
            .await
            .unwrap();
    }
    db.classes()
        .insert(&blank::clear_class(ClassNo(1)))
        .await
        .unwrap();
    db.races()
        .insert(&blank::clear_race(RaceNo(1)))
        .await
        .unwrap();
}

/// A temp SQLite with the fixture world (minus `without`), and its URL.
pub async fn seeded(without: &[u32]) -> (TestDb, String) {
    let t = TestDb::sqlite_temp().await.expect("temp sqlite");
    seed(t.db(), without).await;
    let url = t.db().url().to_string();
    (t, url)
}
pub mod m3;
pub mod stock;

/// Every metric `docs/metrics.md` lists.
pub const ALL_METRICS: [&str; 12] = [
    "silvermere_sessions",
    "silvermere_commands_total",
    "silvermere_tick_seconds",
    "silvermere_persist_queue_depth",
    "silvermere_db_latency_seconds",
    "silvermere_logins_total",
    "silvermere_deaths_total",
    "silvermere_levelups_total",
    "silvermere_handler_panics_total",
    "silvermere_output_dropped_total",
    "silvermere_persist_errors_total",
    "silvermere_build_info",
];

/// Runs a scripted session that emits every metric once, against a server on a fresh
/// seeded database, and returns the server still running. `persist_errors_total` comes
/// from a persistence batch that the database refuses at first: the scripted character is
/// saved while its table is renamed away, then renamed back.
pub async fn run_metrics_script() -> silvermere::Running {
    use std::time::Duration;

    use silvermere::config::Config;
    use silvermere_auth::Role;
    use silvermere_auth::cli::create_with_cost;
    use silvermere_auth::password::Cost;
    use silvermere_engine::{EngineInput, SeededRng, SystemClock, clear_player};
    use silvermere_testkit::TelnetClient;

    const WAIT: Duration = Duration::from_secs(30);
    let (_t, url) = seeded(&[]).await;
    let db = Db::connect_and_migrate(&url).await.unwrap();
    let account = create_with_cost(&db, "Aldric", "aldric1", &[Role::Player], Cost::Low)
        .await
        .unwrap();
    let c = stored_character("ALDRIC", "Aldric");
    db.characters().insert(account.id, &c).await.unwrap();
    db.close().await;

    let mut cfg = Config::default();
    cfg.server.telnet_bind = "127.0.0.1:0".into();
    cfg.server.admin_bind = "127.0.0.1:0".into();
    cfg.database.url = url;
    let r = silvermere::serve_in_process(cfg, SystemClock, SeededRng::new(7))
        .await
        .unwrap();
    let mut client = TelnetClient::connect(r.telnet_addr).await.unwrap();
    client.until("Username:", WAIT).await.unwrap();
    client.send_line("Aldric").await.unwrap();
    client.until("Password:", WAIT).await.unwrap();
    client.send_line("aldric1").await.unwrap();
    client
        .until(r"Make your selection \(X to exit\):", WAIT)
        .await
        .unwrap();
    client.send_line("M").await.unwrap();
    client.until(r"\[MAJORMUD\]:", WAIT).await.unwrap();
    client.send_line("E").await.unwrap(); // logins_total
    client.until(r"\[HP=35\]:", WAIT).await.unwrap();
    client.send_line("").await.unwrap();
    client.until(r"\[HP=", WAIT).await.unwrap();
    client.send_line("l").await.unwrap(); // commands_total{family="info"}
    // Wait for the prompt that follows the look: the dispatcher has counted it by then.
    client.until(r"\[HP=", WAIT).await.unwrap();
    // A tick, a handler panic, a character save, and the events nothing emits yet.
    r.to_engine.send(EngineInput::Tick).unwrap();
    r.to_engine
        .send(EngineInput::Custom {
            session: None,
            run: Box::new(|_| panic!("scripted panic for silvermere_handler_panics_total")),
        })
        .unwrap();
    r.to_engine
        .send(EngineInput::Custom {
            session: Some(silvermere_engine::SessionId(0)),
            run: Box::new(|w| {
                silvermere_engine::state::login::leave::save_player(
                    w,
                    silvermere_engine::SessionId(0),
                );
                silvermere_engine::metrics::death();
                silvermere_engine::metrics::levelup();
            }),
        })
        .unwrap();
    // A session whose command queue is full sheds writes.
    {
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        let (reg_tx, reg_rx) = tokio::sync::mpsc::unbounded_channel();
        let (opened_tx, _opened_rx) = tokio::sync::mpsc::unbounded_channel();
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn(silvermere::bridge::forward_outputs(
            out_rx,
            reg_rx,
            opened_tx,
            Vec::new(),
            tokio::sync::mpsc::unbounded_channel().0,
            silvermere::bridge::Sinks::closed(),
            Default::default(),
        ));
        reg_tx.send((silvermere_net::SessionId(1), cmd_tx)).unwrap();
        out_tx
            .send(silvermere_engine::EngineOutput::Opened {
                conn: silvermere_engine::ConnId(1),
                id: silvermere_engine::SessionId(1),
            })
            .unwrap();
        for _ in 0..4 {
            out_tx
                .send(silvermere_engine::EngineOutput::Write {
                    id: silvermere_engine::SessionId(1),
                    bytes: b"x".to_vec(),
                })
                .unwrap();
        }
        out_tx
            .send(silvermere_engine::EngineOutput::Stopped)
            .unwrap();
        task.await.unwrap();
    }
    // A persistence batch that fails on a closed pool is retried: the error counter.
    {
        let (_t, url) = seeded(&[]).await;
        let db = std::sync::Arc::new(Db::connect_and_migrate(&url).await.unwrap());
        let account = create_with_cost(&db, "Brin", "brin1", &[Role::Player], Cost::Low)
            .await
            .unwrap();
        let mut c = clear_player();
        c.userid = "BRIN".into();
        c.firstname = "Brin".into();
        let id = db.characters().insert(account.id, &c).await.unwrap();
        let (persist, _worker) =
            silvermere_db::Persist::spawn(db.clone(), silvermere_db::PersistConfig::default());
        db.close().await;
        persist
            .send(silvermere_db::Write::Character {
                id,
                character: Box::new(c),
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        db.reconnect().await.unwrap();
        persist.flush().await.unwrap();
        db.close().await;
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    drop(client);
    r
}

/// A stored character `load_player` (T-023) accepts: record version 22, level 1, stats
/// inside the zeroed test race's range, validated name, `hpbonus` making the recomputed
/// maximum the 35 hit points the prompt shows.
pub fn stored_character(userid: &str, first: &str) -> silvermere_types::records::Character {
    use silvermere_engine::clear_player;
    use silvermere_types::enums::StatlineMode;
    use silvermere_types::ids::{ClassNo, MapId, RaceNo, RoomId};
    let mut c = clear_player();
    c.userid = userid.into();
    c.firstname = first.into();
    c.map = MapId(1);
    c.room = RoomId(special_rooms()[0]);
    c.hp = 35;
    c.maxhp = 35;
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.statline = StatlineMode::Full;
    c.version = 22;
    c.level = 1;
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (50, 50, 50, 50, 50, 50);
    (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (50, 50, 50, 50, 50, 50);
    c.hpbonus = 10;
    c.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
    c
}
