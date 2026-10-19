//! The append-only events table: append, read back, prune to a retention count.

use crate::common;

use common::*;
use silvermere_db::Event;
use silvermere_types::ids::{CharacterId, RoomKey, SessionId};

#[tokio::test]
async fn append_100_prune_to_10() {
    let db = db().await;
    let events = db.events();
    for i in 0..100i64 {
        let mut e = Event::new(at(i), "tick", serde_json::json!({ "n": i }));
        if i % 2 == 0 {
            e.session = Some(SessionId(i as u16));
            e.character = Some(CharacterId(i * 10));
            e.room = Some(RoomKey::new(1, i as u32));
        }
        let id = events.append(&e).await.unwrap();
        assert!(id > 0);
    }
    assert_eq!(events.count().await.unwrap(), 100);
    let removed = events.prune(10).await.unwrap();
    assert_eq!(removed, 90);
    assert_eq!(events.count().await.unwrap(), 10);
    // the ten newest survive, newest first from recent()
    let recent = events.recent(100).await.unwrap();
    assert_eq!(recent.len(), 10);
    let ns: Vec<i64> = recent
        .iter()
        .map(|e| e.event.payload["n"].as_i64().unwrap())
        .collect();
    assert_eq!(ns, (90..100).rev().collect::<Vec<_>>());
    let e98 = &recent[1].event;
    assert_eq!(e98.at, at(98));
    assert_eq!(e98.kind, "tick");
    assert_eq!(e98.session, Some(SessionId(98)));
    assert_eq!(e98.character, Some(CharacterId(980)));
    assert_eq!(e98.room, Some(RoomKey::new(1, 98)));
    let e99 = &recent[0].event;
    assert_eq!((e99.session, e99.character, e99.room), (None, None, None));
    // since() walks forward from an id
    let since = events.since(recent[5].id, 3).await.unwrap();
    assert_eq!(since.len(), 3);
    assert_eq!(since[0].id, recent[4].id);
    // pruning with a retention past the count removes nothing
    assert_eq!(events.prune(1000).await.unwrap(), 0);
    db.close().await;
}
