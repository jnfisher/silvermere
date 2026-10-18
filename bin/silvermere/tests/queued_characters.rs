//! T-088: the engine's queued character records against the real store, on the backend
//! `SILVERMERE_DATABASE_URL` names (SQLite by default). A gang lookup the store answered
//! before a hang-up, applied after it, must not write the older row back over the
//! hang-up's save; and once the store has committed every queued write, it says so and
//! the engine holds no queued record.

mod common;

use std::sync::mpsc::Receiver;
use std::time::Duration;

use silvermere::bridge::load_snapshot;
use silvermere::store::{self, Job};
use silvermere_db::TestDb;
use silvermere_db::repo::accounts::Account;
use silvermere_engine::gangs::{GangQuery, OfflineOp};
use silvermere_engine::state::login::Login;
use silvermere_engine::{
    AccountRef, Caps, ConnId, Engine, EngineInput, EngineOutput, SeededRng, SessionId, SystemClock,
    World,
};
use silvermere_types::config::{GameConfig, PunHup};
use silvermere_types::enums::SessionState;
use silvermere_types::flags::{GangFlags, PlayerFlags, PlayerStatus};
use silvermere_types::ids::ItemNo;
use silvermere_types::records::Gang;
use tokio::sync::mpsc::UnboundedSender;

/// What `bridge::forward_outputs` hands the store: persistence requests and gang lookups.
fn forward(rx: &Receiver<EngineOutput>, jobs: &UnboundedSender<Job>) {
    for o in rx.try_iter() {
        match o {
            EngineOutput::Persist(r) => jobs.send(Job::Persist(r)).unwrap(),
            EngineOutput::GangQuery { id, query } => {
                jobs.send(Job::GangQuery { id, query }).unwrap();
            }
            _ => {}
        }
    }
}

fn opened(rx: &Receiver<EngineOutput>, jobs: &UnboundedSender<Job>) -> SessionId {
    let mut id = None;
    for o in rx.try_iter() {
        match o {
            EngineOutput::Opened { id: i, .. } => id = Some(i),
            EngineOutput::Persist(r) => jobs.send(Job::Persist(r)).unwrap(),
            _ => {}
        }
    }
    id.expect("opened")
}

// In the original, a look-up of an offline player (a gang command, a name scan) found the
// record the hang-up's save had already written; here the store's read can come before
// the hang-up's save, so the engine's queued record stands for it.
#[tokio::test]
async fn an_offline_edit_racing_a_hang_up_keeps_the_hang_up() {
    let t = TestDb::from_env().await.expect("test database");
    common::seed(t.db(), &[]).await;
    let db = t.db().clone();
    db.gangs()
        .insert(&Gang {
            key: "WOLVES".into(),
            name: "Wolves".into(),
            exp: 0,
            leader: "Wanda".into(),
            created: 0,
            members: 2,
            flags: GangFlags::empty(),
            overexp: 0,
            overflows: 0,
        })
        .await
        .unwrap();
    let account = db
        .accounts()
        .insert(&Account::new("aldric", "", jiff::Timestamp::now()))
        .await
        .unwrap();
    let mut c = common::stored_character("ALDRIC", "Aldric");
    c.gang = "Wolves".into();
    c.items[0] = ItemNo(1);
    c.itemuses[0] = 0;
    let cid = db.characters().insert(account, &c).await.unwrap();

    let config = GameConfig {
        pun_hup: PunHup::High,
        min_hup_hp: 20,
        max_hup_hp: 20,
        max_hup_items: 1,
        ..GameConfig::default()
    };
    let world = World::load(config, 8, load_snapshot(&db).await.unwrap()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut engine = Engine::new(world, SystemClock, SeededRng::new(1), tx);

    let (persist, worker) =
        silvermere_db::Persist::spawn(db.clone(), silvermere_db::PersistConfig::default());
    let (jobs, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (to_engine, from_store) = std::sync::mpsc::channel();
    let (flushed, _barriers) = tokio::sync::watch::channel(0u64);
    let store = tokio::spawn(store::run(
        db.clone(),
        persist,
        worker,
        jobs_rx,
        to_engine,
        flushed,
    ));

    // Wanda's PROMOTE ALDRIC reaches the store while he is off line: it reads his row
    jobs.send(Job::GangQuery {
        id: SessionId(999),
        query: GangQuery::Offline {
            name: "aldric".into(),
            gang: "Wolves".into(),
            op: OfflineOp::Promote,
        },
    })
    .unwrap();
    let (answer, from_store) = tokio::task::spawn_blocking(move || {
        let a = from_store
            .recv_timeout(Duration::from_secs(20))
            .expect("the store answers the lookup");
        (a, from_store)
    })
    .await
    .unwrap();

    // before the answer reaches the engine, Aldric comes in and drops the line
    engine.handle(EngineInput::Connected {
        conn: ConnId(1),
        caps: Caps::default(),
    });
    let id = opened(&rx, &jobs);
    let row = db.characters().by_account(account).await.unwrap().unwrap();
    engine.handle(EngineInput::LoggedIn {
        id,
        login: Box::new(Login {
            account: AccountRef {
                id: account,
                username: "aldric".into(),
                sysop: false,
                gameop: false,
                sex: b'M',
            },
            character: Some(Box::new(row.character)),
            may_gossip: true,
            may_auction: true,
            sysop_keys: silvermere_engine::commands::sysop::Keys::empty(),
        }),
    });
    for line in ["M", "E"] {
        engine.handle(EngineInput::Line {
            id,
            text: line.into(),
        });
    }
    assert_eq!(
        engine.world.session(id).map(|s| s.state),
        Some(SessionState::Playing)
    );
    engine.handle(EngineInput::Disconnect { id });
    forward(&rx, &jobs);

    // the lookup's answer: the promotion is made on the hang-up's record
    engine.handle(answer);
    forward(&rx, &jobs);
    drop(jobs);
    store.await.unwrap();

    let row = db.characters().get(cid).await.unwrap().unwrap().character;
    assert!(row.flags.contains(PlayerFlags::PROMOTED), "the promotion");
    assert!(row.status.contains(PlayerStatus::HUNGUP), "ST_HUNGUP");
    // the hang-up penalty: 20% of 35 is 7, and the one item dropped
    assert_eq!(row.hp, 28);
    assert_eq!(row.items[0], ItemNo(0));

    // the store acknowledged every write: nothing stays queued
    assert!(!engine.world.queued_characters.is_empty());
    for input in from_store.try_iter() {
        engine.handle(input);
    }
    assert!(engine.world.queued_characters.is_empty());
    t.close().await;
}
