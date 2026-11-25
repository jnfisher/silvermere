//! GMCP (`silvermere_engine::gmcp`): `Room.Info` with each room display, `Char.*` when it
//! changes, `Core.Supports.Set` filtering, `Core.Ping`, and nothing for a session that did
//! not agree to GMCP.
//!
//! The rooms: `WEST` - `MIDDLE` east-west on map 1, `NORTH` north of the middle with an
//! unfound hidden exit west, and a map-change exit east from the middle to room 7 of map 2.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, test_config, tiny_snapshot};
use serde_json::Value;
use silvermere_engine::world::blank;
use silvermere_engine::{Engine, EngineInput, EngineOutput, SeededRng, SessionId, World};
use silvermere_types::enums::{ExitType, MessageLevel};
use silvermere_types::flags::ExitFlags;
use silvermere_types::ids::{RoomId, RoomKey};
use silvermere_types::records::Exit;

const WEST: u32 = 7001;
const MIDDLE: u32 = 7002;
const NORTH: u32 = 7003;

fn exit(target: u32, kind: ExitType) -> Exit {
    Exit {
        target: RoomId(target),
        kind,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    }
}

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
}

impl Harness {
    fn new() -> Self {
        let mut s = tiny_snapshot();
        let room = |map: u32, n: u32, name: &str| {
            let mut r = blank::clear_room(RoomKey::new(map, n));
            r.name = name.into();
            r
        };
        let mut west = room(1, WEST, "West Gate");
        west.exits[2] = exit(MIDDLE, ExitType::Normal);
        let mut middle = room(1, MIDDLE, "Market Square");
        middle.exits[3] = exit(WEST, ExitType::Normal);
        middle.exits[0] = exit(NORTH, ExitType::Normal);
        middle.exits[2] = exit(7, ExitType::MapChange);
        middle.exits[2].par1 = 2;
        let mut north = room(1, NORTH, "Alley");
        north.exits[1] = exit(MIDDLE, ExitType::Normal);
        north.exits[3] = exit(WEST, ExitType::Hidden);
        north.exits[3].par1 = ExitFlags::NEEDSEARCH.bits();
        let mut other = room(2, 7, "Far Road");
        other.exits[3] = exit(MIDDLE, ExitType::MapChange);
        other.exits[3].par1 = 1;
        s.rooms.extend([west, middle, north, other]);
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let world = World::load(test_config(), 16, s).expect("world loads");
        let engine = Engine::new(world, clock.clone(), SeededRng::new(201), tx);
        Self { engine, clock, rx }
    }

    fn player(&mut self, name: &str, room: u32) -> SessionId {
        let id = enter(&mut self.engine.world, name, room, MessageLevel::High);
        if let Some(c) = self
            .engine
            .world
            .session_mut(id)
            .and_then(|s| s.character.as_mut())
        {
            c.movedir = -1;
            c.str = 50;
            c.maxwgt = 2400;
            c.level = 1;
        }
        id
    }

    fn input(&mut self, input: EngineInput) -> Vec<(SessionId, String)> {
        self.engine.handle(input);
        self.drain()
    }

    /// A line, then one tick (a move is queued with a one-tick delay and walked on it).
    fn line(&mut self, id: SessionId, text: &str) -> Vec<(SessionId, String)> {
        let mut out = self.input(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.clock.advance_ticks(1, &mut self.engine);
        out.extend(self.drain());
        out
    }

    fn drain(&mut self) -> Vec<(SessionId, String)> {
        let mut out = Vec::new();
        while let Ok(o) = self.rx.try_recv() {
            if let EngineOutput::Gmcp { id, message } = o {
                out.push((id, message));
            }
        }
        out
    }

    fn agree(&mut self, id: SessionId) -> Vec<(SessionId, String)> {
        self.input(EngineInput::Gmcp { id, on: true })
    }
}

/// The data of the one `package` message in `msgs`.
fn data(msgs: &[(SessionId, String)], package: &str) -> Value {
    let found: Vec<&str> = msgs
        .iter()
        .filter_map(|(_, m)| m.strip_prefix(package)?.strip_prefix(' '))
        .collect();
    assert_eq!(found.len(), 1, "one {package} in {msgs:?}");
    serde_json::from_str(found[0]).unwrap()
}

fn packages(msgs: &[(SessionId, String)]) -> Vec<&str> {
    msgs.iter()
        .map(|(_, m)| m.split(' ').next().unwrap_or_default())
        .collect()
}

#[test]
fn agreeing_sends_the_character_once() {
    let mut h = Harness::new();
    let id = h.player("Kore", WEST);
    let msgs = h.agree(id);
    assert_eq!(
        packages(&msgs),
        vec!["Char.Name", "Char.Status", "Char.Vitals"]
    );
    assert_eq!(data(&msgs, "Char.Name")["name"], "Kore");
    let v = data(&msgs, "Char.Vitals");
    assert_eq!(
        (v["hp"].as_i64(), v["maxhp"].as_i64()),
        (Some(10), Some(10))
    );
    assert_eq!(data(&msgs, "Char.Status")["level"], 1);
    // nothing changed: nothing more
    h.clock.advance_ticks(1, &mut h.engine);
    assert!(h.drain().is_empty());
    // a change goes out once
    h.engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .hp = 4;
    h.clock.advance_ticks(1, &mut h.engine);
    let msgs = h.drain();
    assert_eq!(packages(&msgs), vec!["Char.Vitals"]);
    assert_eq!(data(&msgs, "Char.Vitals")["hp"], 4);
}

#[test]
fn moving_sends_room_info_with_the_visible_exits() {
    let mut h = Harness::new();
    let id = h.player("Kore", WEST);
    h.agree(id);
    let msgs = h.line(id, "e");
    let room = data(&msgs, "Room.Info");
    assert_eq!(room["num"], 1_007_002);
    assert_eq!(room["name"], "Market Square");
    assert_eq!(room["area"], "Map 1");
    assert_eq!(
        (room["map"].as_u64(), room["room"].as_u64()),
        (Some(1), Some(7002))
    );
    let exits = room["exits"].as_object().unwrap();
    assert_eq!(exits["w"], 1_007_001);
    assert_eq!(exits["n"], 1_007_003);
    // the map-change exit names the room on the other map
    assert_eq!(exits["e"], 2_000_007);
    assert_eq!(exits.len(), 3);

    // the unfound hidden exit stays off the map
    let msgs = h.line(id, "n");
    let room = data(&msgs, "Room.Info");
    let exits = room["exits"].as_object().unwrap();
    assert_eq!(exits.keys().collect::<Vec<_>>(), vec!["s"]);

    // LOOK shows the room again, and so does Room.Info
    let msgs = h.line(id, "look");
    assert_eq!(data(&msgs, "Room.Info")["num"], 1_007_003);
}

#[test]
fn supports_set_limits_the_modules() {
    let mut h = Harness::new();
    let id = h.player("Kore", WEST);
    h.agree(id);
    h.input(EngineInput::GmcpMessage {
        id,
        text: r#"Core.Supports.Set ["Char 1"]"#.into(),
    });
    let msgs = h.line(id, "e");
    assert!(!packages(&msgs).contains(&"Room.Info"), "{msgs:?}");
}

#[test]
fn ping_is_answered() {
    let mut h = Harness::new();
    let id = h.player("Kore", WEST);
    h.agree(id);
    let msgs = h.input(EngineInput::GmcpMessage {
        id,
        text: "Core.Ping".into(),
    });
    assert_eq!(msgs, vec![(id, "Core.Ping".to_owned())]);
}

#[test]
fn no_gmcp_without_agreement() {
    let mut h = Harness::new();
    let plain = h.player("Plain", WEST);
    let mapper = h.player("Mapper", WEST);
    h.agree(mapper);
    let msgs = h.line(plain, "e");
    assert!(msgs.iter().all(|(i, _)| *i != plain), "{msgs:?}");
    // turned off again: nothing more
    h.input(EngineInput::Gmcp {
        id: mapper,
        on: false,
    });
    let msgs = h.line(mapper, "e");
    assert!(msgs.is_empty(), "{msgs:?}");
}

#[test]
fn gmcp_clients_get_plain_direction_names() {
    let mut h = Harness::new();
    let plain = h.player("Plain", MIDDLE);
    let mapper = h.player("Mapper", MIDDLE);
    h.agree(mapper);
    h.engine.world.display.dirnames.set_letter(3, 'Q');
    let mut texts = std::collections::BTreeMap::new();
    h.engine.handle(EngineInput::Line {
        id: plain,
        text: "look".into(),
    });
    h.engine.handle(EngineInput::Line {
        id: mapper,
        text: "look".into(),
    });
    while let Ok(o) = h.rx.try_recv() {
        if let EngineOutput::Write { id, bytes } = o {
            texts
                .entry(id)
                .or_insert_with(Vec::new)
                .extend_from_slice(&bytes);
        }
    }
    let text = |id| String::from_utf8_lossy(&texts[&id]).into_owned();
    assert!(text(plain).contains("wQ\u{8}est"), "{}", text(plain));
    assert!(text(mapper).contains("west"), "{}", text(mapper));
    assert!(!text(mapper).contains('\u{8}'), "{}", text(mapper));
}

/// Telnet CHARSET: the engine's half ([`silvermere_engine::charset`]).
mod charset {
    use super::*;

    fn requests(h: &mut Harness, input: EngineInput) -> Vec<(SessionId, String)> {
        h.engine.handle(input);
        let mut out = Vec::new();
        while let Ok(o) = h.rx.try_recv() {
            if let EngineOutput::Charset { id, name } = o {
                out.push((id, name));
            }
        }
        out
    }

    #[test]
    fn set_utf8_asks_a_client_that_agreed_and_no_other() {
        let mut h = Harness::new();
        let mudlet = h.player("Mudlet", WEST);
        let telnet = h.player("Telnet", WEST);
        // Mudlet accepted IBM437 at connect; plain telnet refused CHARSET
        h.engine.handle(EngineInput::Charset {
            id: mudlet,
            accepted: Some("IBM437".into()),
        });
        let line = |id| EngineInput::Line {
            id,
            text: "set utf8 on".into(),
        };
        assert_eq!(
            requests(&mut h, line(mudlet)),
            vec![(mudlet, "UTF-8".to_owned())]
        );
        assert!(requests(&mut h, line(telnet)).is_empty());
        // already UTF-8: nothing more; back to CP437: asked again
        assert!(requests(&mut h, line(mudlet)).is_empty());
        assert_eq!(
            requests(
                &mut h,
                EngineInput::Line {
                    id: mudlet,
                    text: "set utf8 off".into(),
                }
            ),
            vec![(mudlet, "IBM437".to_owned())]
        );
    }
}
