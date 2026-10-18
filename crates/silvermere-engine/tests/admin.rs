//! The admin requests the engine answers: `Who` (the data `display_scan_of_users` lists) and
//! `Broadcast` (the sysop `SPEAK`).

mod common;

use std::sync::mpsc::channel;

use silvermere_engine::{AdminRequest, Clock, EngineInput, EngineOutput};
use silvermere_types::enums::MessageLevel;

// Players in the realm only, in `listslot` order (first names upper-cased, sorted).
#[test]
fn who_lists_players_in_the_realm_in_listslot_order() {
    let (mut engine, clock, _rx) = common::engine(8);
    let zed = common::enter(&mut engine.world, "zed", 2140, MessageLevel::High);
    let _bob = common::enter(&mut engine.world, "Bob", 2140, MessageLevel::High);
    let _amy = common::enter(&mut engine.world, "amy", 2189, MessageLevel::High);
    // A connected session that is not in the realm (no character) is not listed.
    let _ = engine.world.ids.alloc();
    engine.world.sessions.get_mut(&zed).unwrap().idle_since = clock.now();
    clock.advance(std::time::Duration::from_secs(42));
    let (reply, rx) = channel();
    engine.handle(EngineInput::Admin(AdminRequest::Who { reply }));
    let rows = rx.recv().unwrap();
    let names: Vec<_> = rows.iter().map(|r| r.first_name.as_str()).collect();
    assert_eq!(names, ["amy", "Bob", "zed"]);
    assert_eq!(
        rows.iter().map(|r| r.listslot).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(rows[2].id, zed);
    assert_eq!(rows[2].idle_secs, 42);
}

// Sysop SPEAK: clear-line, the text and `\r`, then every player in the game gets his prompt;
// no colour prefix.
#[test]
fn broadcast_reaches_every_player_with_a_prompt() {
    let (mut engine, _clock, out) = common::engine(8);
    let a = common::enter(&mut engine.world, "Amy", 2140, MessageLevel::High);
    let b = common::enter(&mut engine.world, "Bob", 2189, MessageLevel::Low);
    let _ = engine.world.ids.alloc(); // connected, not in the realm
    let (reply, rx) = channel();
    engine.handle(EngineInput::Admin(AdminRequest::Broadcast {
        text: "Reboot in 5".into(),
        reply,
    }));
    assert_eq!(rx.recv().unwrap(), 2);
    let writes: Vec<_> = common::drain(&out)
        .into_iter()
        .filter_map(|o| match o {
            EngineOutput::Write { id, bytes } => Some((id, bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 2);
    assert_eq!([writes[0].0, writes[1].0], [a, b]);
    let text = String::from_utf8_lossy(&writes[0].1).into_owned();
    assert!(text.starts_with("\x1b[79D\x1b[K"), "{text:?}");
    assert!(text.contains("Reboot in 5\r\n"), "{text:?}");
    assert!(!text.contains("\x1b[1;"), "no colour prefix: {text:?}");
}
