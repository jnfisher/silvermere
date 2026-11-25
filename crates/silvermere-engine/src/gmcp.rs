//! GMCP (Generic MUD Communication Protocol): structured data beside the text, for clients
//! with mappers and gauges (Mudlet, TinTin++, ...). <https://tintin.mudhalla.net/protocols/gmcp/>
//!
//! The transport negotiates telnet option 201 and frames the messages; this module decides
//! what to send. A session that agreed to GMCP gets:
//!
//! | Package | When | Data |
//! |---|---|---|
//! | `Room.Info` | each time the room display shows a room (moving, `LOOK`) | `num`, `name`, `area`, `environment`, `map`, `room`, `exits` |
//! | `Char.Name` | entering the realm | `name`, `fullname` |
//! | `Char.Status` | entering the realm, and when one of its fields changes | `name`, `level`, `class`, `race`, `exp` |
//! | `Char.Vitals` | entering the realm, and when one of its fields changes | `hp`, `maxhp`, `mp`, `maxmp` |
//! | `Core.Ping` | in answer to the client's `Core.Ping` | none |
//!
//! `Room.Info` follows the room display, so a player who cannot see (darkness, blindness)
//! gets no room data either, and the exits are the `Obvious exits:` ones: a mapper learns
//! nothing the text does not show. A room's `num` is `map * 1_000_000 + room`, the same
//! across restarts and imports. Rooms have no area names, so `area` is `Map <n>`.
//!
//! Until the client sends `Core.Supports.Set`, every package is sent; after it only the
//! modules it lists (`Char`, `Room`, `Core` always).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use silvermere_types::enums::RoomType;
use silvermere_types::ids::SessionId;
use silvermere_types::records::{ExitParams, Room};

use crate::display::exits::visible_exit;
use crate::event_loop::EngineOutput;
use crate::world::World;

/// The short direction names, by `DIR_` index (Mudlet's mapper reads these).
const DIRS: [&str; 10] = ["n", "s", "e", "w", "ne", "nw", "se", "sw", "u", "d"];

/// Room numbers per map in [`room_num`].
pub const ROOMS_PER_MAP: u64 = 1_000_000;

/// One session's GMCP state; present once the client agreed to GMCP.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gmcp {
    /// The modules the client asked for (`Core.Supports.Set`), lower case; `None` until it
    /// says, when everything is sent.
    pub supports: Option<BTreeSet<String>>,
    /// The client's `Core.Hello` name and version, for the log.
    pub client: Option<String>,
    /// The last `Char.Name`, `Char.Status` and `Char.Vitals` sent, so only changes go out.
    last_name: Option<String>,
    last_status: Option<String>,
    last_vitals: Option<String>,
}

impl Gmcp {
    /// Whether the client takes `package` (`Room.Info` is in module `room`).
    pub fn wants(&self, package: &str) -> bool {
        let Some(supports) = &self.supports else {
            return true;
        };
        let lower = package.to_ascii_lowercase();
        let module = lower.split('.').next().unwrap_or_default();
        module == "core" || supports.contains(module) || supports.contains(&lower)
    }

    /// Forgets what was sent, so the next [`refresh`] sends it all again.
    fn forget(&mut self) {
        self.last_name = None;
        self.last_status = None;
        self.last_vitals = None;
    }
}

/// The client agreed to GMCP (`on`) or turned it off.
pub fn on_enabled(world: &mut World, id: SessionId, on: bool) {
    if let Some(s) = world.session_mut(id) {
        s.gmcp = on.then(Gmcp::default);
    }
}

/// A GMCP message from the client: `Package.Name`, then optional JSON after white space.
pub fn on_message(world: &mut World, id: SessionId, text: &str) {
    let text = text.trim();
    let (package, data) = match text.split_once(char::is_whitespace) {
        Some((p, d)) => (p, d.trim()),
        None => (text, ""),
    };
    let Some(state) = world.session_mut(id).and_then(|s| s.gmcp.as_mut()) else {
        return;
    };
    match package.to_ascii_lowercase().as_str() {
        "core.hello" => {
            let v: Value = serde_json::from_str(data).unwrap_or(Value::Null);
            let name = v.get("client").and_then(Value::as_str).unwrap_or("?");
            let version = v.get("version").and_then(Value::as_str).unwrap_or("");
            let client = format!("{name} {version}").trim().to_owned();
            tracing::debug!(session = id.0, %client, "GMCP client");
            state.client = Some(client);
        }
        "core.supports.set" => {
            state.supports = Some(modules(data).collect());
            state.forget();
        }
        "core.supports.add" => {
            state
                .supports
                .get_or_insert_with(BTreeSet::new)
                .extend(modules(data));
            state.forget();
        }
        "core.supports.remove" => {
            if let Some(supports) = &mut state.supports {
                for m in modules(data) {
                    supports.remove(&m);
                }
            }
        }
        "core.ping" => send(world, id, "Core.Ping", None),
        _ => {}
    }
}

/// The module names of a `Core.Supports` list (`["Char 1", "Room 1"]`), lower case.
fn modules(data: &str) -> impl Iterator<Item = String> {
    let list: Vec<String> = serde_json::from_str(data).unwrap_or_default();
    list.into_iter().filter_map(|entry| {
        let name = entry.split_whitespace().next()?;
        Some(name.to_ascii_lowercase())
    })
}

/// Queues `package` with `data` for the session, if it agreed to GMCP and takes the package.
pub fn send(world: &mut World, id: SessionId, package: &str, data: Option<&Value>) {
    let wants = world
        .session(id)
        .and_then(|s| s.gmcp.as_ref())
        .is_some_and(|g| g.wants(package));
    if !wants {
        return;
    }
    world.outbox.push(EngineOutput::Gmcp {
        id,
        message: message(package, data),
    });
}

/// `Package.Name json`, or the bare name.
fn message(package: &str, data: Option<&Value>) -> String {
    match data {
        Some(v) => format!("{package} {v}"),
        None => package.to_owned(),
    }
}

/// The room's GMCP number: `map * 1_000_000 + room`.
pub fn room_num(map: u32, room: u32) -> u64 {
    u64::from(map) * ROOMS_PER_MAP + u64::from(room)
}

/// `Room.Info` for the room the display just showed (`room` with its live exits).
pub fn room_info(world: &mut World, id: SessionId, room: &Room) {
    let enabled = world.session(id).is_some_and(|s| s.gmcp.is_some());
    if !enabled {
        return;
    }
    let data = room_json(room);
    send(world, id, "Room.Info", Some(&data));
}

/// The `Room.Info` data of `room`.
pub fn room_json(room: &Room) -> Value {
    let map = room.key.map.0;
    let mut exits = serde_json::Map::new();
    for (dir, exit) in room.exits.iter().enumerate() {
        if exit.target.0 == 0 || !visible_exit(exit) {
            continue;
        }
        let to_map = match exit.params() {
            ExitParams::MapChange { map } => map.0,
            _ => map,
        };
        if let Some(name) = DIRS.get(dir) {
            exits.insert((*name).to_owned(), json!(room_num(to_map, exit.target.0)));
        }
    }
    json!({
        "num": room_num(map, room.key.room.0),
        "name": room.name.trim(),
        "area": format!("Map {map}"),
        "environment": environment(room.room_type),
        "map": map,
        "room": room.key.room.0,
        "exits": exits,
    })
}

/// The room type as a word, for the mapper's room colours.
fn environment(kind: RoomType) -> &'static str {
    match kind {
        RoomType::Shop => "shop",
        RoomType::Arena => "arena",
        RoomType::Lair => "lair",
        RoomType::Hotel => "hotel",
        RoomType::Colliseum => "colliseum",
        RoomType::Jail => "jail",
        RoomType::Library => "library",
        _ => "normal",
    }
}

/// At the end of each event: `Char.Name`, `Char.Status` and `Char.Vitals` for each GMCP
/// session in the realm whose values changed since they were last sent. A session out of
/// the realm forgets them, so entering again sends them all.
pub fn refresh(world: &mut World) -> Vec<EngineOutput> {
    let mut out = Vec::new();
    for id in world.session_ids() {
        let Some(s) = world.session(id) else {
            continue;
        };
        if s.gmcp.is_none() {
            continue;
        }
        if !s.is_in_game() {
            if let Some(g) = world.session_mut(id).and_then(|s| s.gmcp.as_mut()) {
                g.forget();
            }
            continue;
        }
        let Some(c) = world.session(id).and_then(|s| s.player()) else {
            continue;
        };
        let first = c.firstname.trim().to_owned();
        let fullname = format!("{} {}", first, c.lastname.trim()).trim().to_owned();
        let name = json!({ "name": first, "fullname": fullname });
        let status = json!({
            "name": first,
            "fullname": fullname,
            "level": c.level,
            "class": world.classes.get(&c.class).map_or_else(|| c.class.0.to_string(), |k| k.name.clone()),
            "race": world.races.get(&c.race).map_or_else(|| c.race.0.to_string(), |k| k.name.clone()),
            "exp": c.exp,
        });
        let vitals = json!({
            "hp": c.hp,
            "maxhp": c.maxhp,
            "mp": c.mana,
            "maxmp": c.maxmana,
        });
        let Some(g) = world.session_mut(id).and_then(|s| s.gmcp.as_mut()) else {
            continue;
        };
        let wants = [
            g.wants("Char.Name"),
            g.wants("Char.Status"),
            g.wants("Char.Vitals"),
        ];
        for ((package, data, last), wanted) in [
            ("Char.Name", name, &mut g.last_name),
            ("Char.Status", status, &mut g.last_status),
            ("Char.Vitals", vitals, &mut g.last_vitals),
        ]
        .into_iter()
        .zip(wants)
        {
            let text = data.to_string();
            if last.as_deref() == Some(text.as_str()) {
                continue;
            }
            *last = Some(text);
            if wanted {
                out.push(EngineOutput::Gmcp {
                    id,
                    message: message(package, Some(&data)),
                });
            }
        }
    }
    out
}

/// Drops each printable ASCII byte that a backspace takes back before another printable
/// byte overwrites it (`n` `X` BS `orth` becomes `north`), as a terminal would show it. The
/// direction names' anti-script letters ([`crate::display::exits::DirNames`]) are what this is
/// for: a client that applies no backspaces shows the letter and a stray character.
pub fn apply_backspaces(bytes: &mut Vec<u8>) {
    const BS: u8 = 0x08;
    if !bytes.contains(&BS) {
        return;
    }
    let printable = |b: Option<&u8>| b.is_some_and(|b| (0x20..0x7F).contains(b));
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if printable(bytes.get(i)) && bytes.get(i + 1) == Some(&BS) && printable(bytes.get(i + 2)) {
            i += 2;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    *bytes = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backspaced_letters_are_dropped() {
        let mut b = b"Obvious exits: nA\x08orth, sU\x08outh, wW\x08est\x08".to_vec();
        apply_backspaces(&mut b);
        // a backspace with nothing printable after it stays
        assert_eq!(b, b"Obvious exits: north, south, west\x08".to_vec());
    }

    #[test]
    fn supports_filter_by_module() {
        let mut g = Gmcp::default();
        assert!(g.wants("Room.Info"));
        g.supports = Some(modules(r#"["Char 1", "Char.Skills 1"]"#).collect());
        assert!(g.wants("Char.Vitals"));
        assert!(!g.wants("Room.Info"));
        assert!(g.wants("Core.Ping"));
    }

    #[test]
    fn room_numbers_keep_map_and_room_apart() {
        assert_eq!(room_num(1, 1), 1_000_001);
        assert_eq!(room_num(12, 345), 12_000_345);
    }

    #[test]
    fn message_without_data_is_the_name() {
        assert_eq!(message("Core.Ping", None), "Core.Ping");
        assert_eq!(
            message("Char.Vitals", Some(&json!({"hp": 1}))),
            r#"Char.Vitals {"hp":1}"#
        );
    }
}
