//! T-030: floor items, limited-item ownership and the copies-in-play count survive a
//! restart: the engine's write-behind requests go through `store` to the persistence
//! worker, and a fresh `load_snapshot` brings them back.

use crate::common;

use std::time::Instant;

use silvermere::bridge::load_snapshot;
use silvermere::store::{self, Job};
use silvermere_engine::items::inventory::add_item_to_inventory;
use silvermere_engine::player_session::Caps;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, PlayerSession, SeededRng, SystemClock, World, clear_player,
};
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{InGame, OwnerKind, SessionState};
use silvermere_types::ids::{ClassNo, ItemNo, MapId, RaceNo, RoomId, RoomKey};

#[tokio::test]
async fn floor_items_and_ownership_survive_a_restart() {
    let (t, _url) = common::seeded(&[]).await;
    let key = RoomKey::new(1, 2140);
    let mut world = World::load(
        GameConfig::default(),
        8,
        load_snapshot(t.db()).await.unwrap(),
    )
    .unwrap();
    // item 1 is limited to one copy for this run
    {
        let itm = world.items.get_mut(&ItemNo(1)).unwrap();
        itm.limit = 1;
        itm.name = "relic".into();
        itm.gettable = 1;
    }
    let id = world.ids.alloc().unwrap();
    let mut s = PlayerSession::new(id, Caps::default(), Instant::now());
    let mut c = clear_player();
    c.firstname = "Aldric".into();
    c.userid = "ALDRIC".into();
    c.map = MapId(1);
    c.room = RoomId(2140);
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.str = 50;
    c.hp = 10;
    c.maxhp = 10;
    c.maxwgt = 2400;
    c.readied = -1;
    c.nkeyslots = 50;
    s.character = Some(c);
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    world.sessions.insert(id, s);
    assert!(add_item_to_inventory(&mut world, id, ItemNo(1), -2));
    assert!(add_item_to_inventory(&mut world, id, ItemNo(2), -2));
    world.room_state.get_mut(&key).unwrap().coins.gold = 7;
    silvermere_engine::commands::execute_input(&mut world, id, "drop relic", false);

    // the engine flushes its queued requests at the end of the next event
    let (tx, rx) = std::sync::mpsc::channel();
    let mut engine = Engine::new(world, SystemClock, SeededRng::new(1), tx);
    engine.handle(EngineInput::Tick);
    let (persist, worker) =
        silvermere_db::Persist::spawn(t.db().clone(), silvermere_db::PersistConfig::default());
    let (jobs_tx, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (to_engine, _from_store) = std::sync::mpsc::channel();
    let mut n = 0;
    for o in rx.try_iter() {
        if let EngineOutput::Persist(r) = o {
            jobs_tx.send(Job::Persist(r)).unwrap();
            n += 1;
        }
    }
    assert!(
        n >= 4,
        "room state, ownership insert/delete, active count: {n}"
    );
    drop(jobs_tx);
    let (flushed, _barriers) = tokio::sync::watch::channel(0u64);
    store::run(t.db().clone(), persist, worker, jobs_rx, to_engine, flushed).await;

    // restart
    let world = World::load(
        GameConfig::default(),
        8,
        load_snapshot(t.db()).await.unwrap(),
    )
    .unwrap();
    let rs = &world.room_state[&key];
    assert_eq!(rs.items[0].item, ItemNo(1));
    assert_eq!(rs.coins.gold, 7);
    assert_eq!(world.items[&ItemNo(1)].active, 1);
    let owners: Vec<_> = world.item_ownership.values().collect();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].itemnum, ItemNo(1));
    assert_eq!(owners[0].owntype, OwnerKind::Room);
    assert_eq!((owners[0].mapnum, owners[0].roomnum), (key.map, key.room));
}
