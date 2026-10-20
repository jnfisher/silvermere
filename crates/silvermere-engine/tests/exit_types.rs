//! The 25 exit types through `move_user` (T-026): one test per type with the pass case
//! and every refusal text the original prints at an exit, byte-exact; the door, gate and key lock
//! states against OPEN, CLOSE, LOCK, BASH and PICKLOCK; the trap damage range; the toll
//! paid and unaffordable; the Level exit low, high and ok as at 1:33; the alignment
//! exits; the arena's `maxlevel`. Every refusal line is matched against the pattern
//! registry (`tests/fixtures/patterns.json`) and pinned in one `insta` snapshot.
//!
//! The world is a hub room per exit type (`5100 + type`) with the exit under test to
//! the north, leading to `5200 + type`; the walker stands in the hub.

use crate::common;

use std::collections::BTreeMap;

use common::{enter, tiny_snapshot};
use silvermere_engine::commands::{Cmd, CommandLine};
use silvermere_engine::movement::exits::{
    LOCK_CLOSED, LOCK_LOCKED, LOCK_OPEN, exit_is_available, live_exit,
};
use silvermere_engine::movement::{doors, move_user};
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{SeededRng, SessionId, World, WorldSnapshot};
use silvermere_types::config::GameConfig;
use silvermere_types::enums::{Direction, ExitType, MessageLevel, RoomType};
use silvermere_types::flags::{ExitFlags, PlayerStatus2};
use silvermere_types::ids::{ItemNo, MapId, MessageNo, RoomId, RoomKey};
use silvermere_types::records::{Coins, Exit, Message};

// ---------------------------------------------------------------------------------------
// The world.

fn hub(kind: ExitType) -> u32 {
    5100 + u32::from(kind.code())
}

fn far(kind: ExitType) -> u32 {
    5200 + u32::from(kind.code())
}

/// The arena (room type Arena, `maxlevel` 3) behind a Normal exit of its own hub.
const ARENA_HUB: u32 = 5300;
const ARENA: u32 = 5301;
/// A room on map 2 for the Map Change exit.
const OTHER_MAP_ROOM: u32 = 7;

const KEY_ITEM: i32 = 777;

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

fn message(n: i32, texts: [&str; 3]) -> Message {
    Message {
        number: MessageNo(n),
        msgs: texts.map(str::to_owned),
    }
}

fn snapshot() -> WorldSnapshot {
    let mut s = tiny_snapshot();
    for code in 0..25u16 {
        let kind = ExitType::from_code(code);
        let mut h = blank::clear_room(RoomKey::new(1, hub(kind)));
        h.name = format!("Hub {}", kind.code());
        h.exits[0] = exit(far(kind), kind);
        // the way back, so the other-side logic of the door commands has something to do
        let mut f = blank::clear_room(RoomKey::new(1, far(kind)));
        f.name = format!("Far {}", kind.code());
        f.exits[1] = exit(hub(kind), kind);
        s.rooms.push(h);
        s.rooms.push(f);
    }
    let mut ah = blank::clear_room(RoomKey::new(1, ARENA_HUB));
    ah.name = "Arena Gate".into();
    ah.exits[0] = exit(ARENA, ExitType::Normal);
    let mut ar = blank::clear_room(RoomKey::new(1, ARENA));
    ar.name = "Newhaven, Arena".into();
    ar.room_type = RoomType::Arena;
    ar.maxlevel = 3;
    s.rooms.push(ah);
    s.rooms.push(ar);
    let mut other = blank::clear_room(RoomKey::new(2, OTHER_MAP_ROOM));
    other.name = "Dragon's Teeth".into();
    s.rooms.push(other);
    let mut key = blank::clear_item(ItemNo(KEY_ITEM));
    key.name = "brass key".into();
    s.items.push(key);
    s.messages.extend([
        message(
            9001,
            [
                "You squeeze through the crack.",
                "%s squeezes through a crack to the %s.",
                "%s squeezes in from the %s.",
            ],
        ),
        message(
            9002,
            ["The way is barred to you.", "%s tugs at the bars.", ""],
        ),
        message(
            9003,
            [
                "You hop in one of the skiffs%s.",
                "%s hops in one of the skiffs%s.",
                "%s rows in%s.",
            ],
        ),
        message(9004, ["borrow skiff", "go skiff", "row skiff"]),
        message(
            9005,
            [
                "You row to the far shore%s.",
                "%s rows away%s.",
                "%s rows in from the shore%s.",
            ],
        ),
        message(
            9006,
            [
                "You fall through a trap door for %d damage!",
                "%s falls through a trap door!",
                "%s drops in from above!",
            ],
        ),
        message(
            9007,
            [
                "Sparks fly around you.",
                "%s trips a glyph.",
                "%s staggers in.",
            ],
        ),
        message(
            9008,
            [
                "The portcullis to the %s rises.",
                "The portcullis to the %s falls.",
                "The portcullis to the %s is down.",
            ],
        ),
        message(
            9009,
            [
                "A chill passes through you.",
                "%s shivers.",
                "%s shakes off a chill.",
            ],
        ),
        message(
            9010,
            [
                "Your mind is too weak for this.",
                "%s frowns at the arch.",
                "",
            ],
        ),
    ]);
    s
}

fn world() -> World {
    world_with(16)
}

fn world_with(max_sessions: u16) -> World {
    let mut w = World::load(GameConfig::default(), max_sessions, snapshot()).expect("world loads");
    w.rng = SeededRng::new(26);
    w
}

fn player(world: &mut World, name: &str, room: u32) -> SessionId {
    let id = enter(world, name, room, MessageLevel::High);
    edit(world, id, |c| {
        c.movedir = -1;
        c.str = 50;
        c.maxwgt = 2400; // 48 per point
        c.level = 1;
    });
    id
}

fn edit(
    world: &mut World,
    id: SessionId,
    f: impl FnOnce(&mut silvermere_types::records::Character),
) {
    f(world.session_mut(id).unwrap().character.as_mut().unwrap());
}

fn set_exit(world: &mut World, room: u32, dir: usize, f: impl FnOnce(&mut Exit)) {
    let key = RoomKey::new(1, room);
    f(&mut world.rooms.get_mut(&key).unwrap().exits[dir]);
    // the exit as set is its live state: drop the start-up reset laid over it (T-035)
    world.room_state.get_mut(&key).unwrap().exit_overrides[dir] = Default::default();
}

/// The bytes the session would receive now, with `<P>` where a prompt goes, and the
/// batch emptied.
fn take(world: &mut World, id: SessionId) -> Vec<u8> {
    let palette = palette_of(world, id);
    let s = world.session_mut(id).unwrap();
    let out = s.output.render(palette, b"<P>");
    s.output.clear();
    out
}

fn room_of(world: &World, id: SessionId) -> (u32, u32) {
    let p = world.session(id).unwrap().player().unwrap();
    (p.map.0, p.room.0)
}

/// `move_user` north with `how` from the hub of `kind`.
fn walk(world: &mut World, id: SessionId, how: i32) -> bool {
    move_user(world, id, 0, how)
}

/// Every `X\b` pair dropped: the anti-script letter of `dirnames[]`.
fn drop_backspace_pairs(bytes: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    for &b in bytes {
        if b == 8 && out.pop().is_some() {
            continue;
        }
        out.push(b);
    }
    out
}

fn esc(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

/// The screen lines: ANSI and `X\b` pairs removed, split at `\r\n`, prompts dropped.
fn lines(bytes: &[u8]) -> Vec<String> {
    let plain = silvermere_text::strip_ansi(&drop_backspace_pairs(bytes));
    String::from_utf8_lossy(&plain)
        .split("\r\n")
        .map(|l| l.replace("<P>", ""))
        .filter(|l| !l.is_empty())
        .collect()
}

/// The registry's regexes by name, anchored, compiled on first use (some rows carry
/// `patterns.py` placeholders the Rust regex crate rejects).
struct Registry {
    raw: BTreeMap<String, String>,
    compiled: BTreeMap<String, regex::Regex>,
}

impl Registry {
    fn load() -> Self {
        let reg: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
        let raw = reg["static"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["name"].as_str().unwrap().to_owned(),
                    p["rx"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        Self {
            raw,
            compiled: BTreeMap::new(),
        }
    }

    fn get(&mut self, name: &str) -> &regex::Regex {
        let raw = self
            .raw
            .get(name)
            .unwrap_or_else(|| panic!("no pattern {name}"));
        self.compiled
            .entry(name.to_owned())
            .or_insert_with(|| regex::Regex::new(&format!("^(?:{raw})$")).unwrap())
    }
}

/// The refusal lines seen so far, by test, for the one snapshot at the end of
/// [`refusals_match_the_registry_and_the_snapshot`].
struct Refusals {
    seen: Vec<(String, String)>,
    registry: Registry,
}

impl Refusals {
    fn new() -> Self {
        Self {
            seen: Vec::new(),
            registry: Registry::load(),
        }
    }

    /// `bytes` is exactly one line (plus no prompt) that matches pattern `name`.
    fn one(&mut self, name: &str, bytes: &[u8]) {
        let ls = lines(bytes);
        assert_eq!(ls.len(), 1, "{name}: {}", esc(bytes));
        let rx = self.registry.get(name);
        assert!(rx.is_match(&ls[0]), "{name}: {:?} !~ {rx}", ls[0]);
        self.seen.push((name.to_owned(), esc(bytes)));
    }

    /// `bytes` is `user_allowed_in_room`'s refusal (pattern `name`) followed by
    /// `move_user`'s `You are not permitted in that room!`.
    fn room_refusal(&mut self, name: &str, bytes: &[u8]) {
        let ls = lines(bytes);
        assert_eq!(ls.len(), 2, "{name}: {}", esc(bytes));
        assert!(
            self.registry.get(name).is_match(&ls[0]),
            "{name}: {:?}",
            ls[0]
        );
        assert!(
            self.registry.get("not_permitted").is_match(&ls[1]),
            "{:?}",
            ls[1]
        );
        self.seen.push((name.to_owned(), esc(bytes)));
    }

    fn snapshot(&self) -> String {
        self.seen
            .iter()
            .map(|(n, b)| format!("{n}: {b}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ---------------------------------------------------------------------------------------
// One test per exit type.

// EXIT_NORMAL and the move
#[test]
fn normal_exit_passes() {
    let mut w = world();
    let id = player(&mut w, "Aldric", hub(ExitType::Normal));
    assert!(walk(&mut w, id, 0));
    assert_eq!(room_of(&w, id), (1, far(ExitType::Normal)));
    let out = take(&mut w, id);
    // ANSI_NORMAL, then the room, no prompt for a plain walk
    assert!(esc(&out).starts_with("\\x1b[0;37;40m\\x1b[79D\\x1b[K\\x1b[1;36mFar 0"));
    assert!(!out.windows(3).any(|w| w == b"<P>"));
}

// No exit in that direction
#[test]
fn no_exit_at_all() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Normal));
    assert!(!move_user(&mut w, id, 1, 0));
    r.one("no_exit", &take(&mut w, id));
    assert_eq!(lines(r.seen[0].1.as_bytes()).len(), 1);
    assert_eq!(
        r.seen[0].1,
        "\\x1b[79D\\x1b[KThere is no exit in that direction!\\r\\n"
    );
}

// EXIT_SPELL
#[test]
fn spell_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Spell));
    assert!(!walk(&mut w, id, 0));
    r.one("need_spell", &take(&mut w, id));
    assert!(!exit_is_available(
        &w,
        RoomKey::new(1, hub(ExitType::Spell)),
        Direction::North
    ));
    // behind a leader (how 3) and by typed words (how 4) the way is open
    assert!(walk(&mut w, id, 4));
    assert_eq!(room_of(&w, id), (1, far(ExitType::Spell)));
    insta::assert_snapshot!("spell_refusal", r.snapshot());
}

// EXIT_KEY: exitpar2s is the lock; a bash (how 5) gets through
#[test]
fn key_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Key));
    set_exit(&mut w, hub(ExitType::Key), 0, |e| {
        e.par2 = LOCK_LOCKED as i16
    });
    assert!(!walk(&mut w, id, 0));
    r.one("door_closed_drag", &take(&mut w, id));
    assert!(walk(&mut w, id, 5));
    assert_eq!(room_of(&w, id), (1, far(ExitType::Key)));
    // open: an ordinary walk
    let id2 = player(&mut w, "Bob", hub(ExitType::Key));
    set_exit(&mut w, hub(ExitType::Key), 0, |e| e.par2 = LOCK_OPEN as i16);
    assert!(walk(&mut w, id2, 0));
    insta::assert_snapshot!("key_refusal", r.snapshot());
}

// EXIT_ITEM
#[test]
fn item_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Item));
    let watcher = player(&mut w, "Bob", hub(ExitType::Item));
    let beyond = player(&mut w, "Cid", far(ExitType::Item));
    set_exit(&mut w, hub(ExitType::Item), 0, |e| {
        e.par1 = KEY_ITEM as u32;
        e.par3 = 9001;
    });
    // without the item and without a refusal message: the standard text
    assert!(!walk(&mut w, id, 0));
    r.one("need_item", &take(&mut w, id));
    // with a refusal message: its texts to the player and the room
    set_exit(&mut w, hub(ExitType::Item), 0, |e| e.par2 = 9002);
    assert!(!walk(&mut w, id, 0));
    assert_eq!(
        esc(&take(&mut w, id)),
        "\\x1b[79D\\x1b[K\\x1b[1;33mThe way is barred to you.\\r\\n"
    );
    assert_eq!(
        esc(&take(&mut w, watcher)),
        "\\x1b[79D\\x1b[K\\x1b[1;33m\\x1b[0;32mAldric tugs at the bars.\\r\\n<P>"
    );
    // carrying the item: through, with the exit's texts, no prompt (goto out)
    edit(&mut w, id, |c| c.items[0] = ItemNo(KEY_ITEM));
    assert!(walk(&mut w, id, 0));
    assert_eq!(room_of(&w, id), (1, far(ExitType::Item)));
    let own = take(&mut w, id);
    assert!(
        esc(&own).starts_with(
            "\\x1b[79D\\x1b[K\\x1b[0;32mYou squeeze through the crack.\\r\\n\\x1b[79D\\x1b[K\\x1b[1;36mFar 3"
        ),
        "{}",
        esc(&own)
    );
    assert!(!own.windows(3).any(|w| w == b"<P>"));
    assert_eq!(
        lines(&take(&mut w, watcher)),
        ["Aldric squeezes through a crack to the north."]
    );
    assert_eq!(
        lines(&take(&mut w, beyond)),
        ["Aldric squeezes in from the south."]
    );
    insta::assert_snapshot!("item_refusal", r.snapshot());
}

// EXIT_TOLL: exitpar1s gold crowns, paid by value
#[test]
fn toll_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Toll));
    set_exit(&mut w, hub(ExitType::Toll), 0, |e| e.par1 = 5);
    edit(&mut w, id, |c| c.coins.gold = 4);
    assert!(!walk(&mut w, id, 0));
    r.one("toll_short", &take(&mut w, id));
    // 1 crown: the singular
    set_exit(&mut w, hub(ExitType::Toll), 0, |e| e.par1 = 1);
    edit(&mut w, id, |c| c.coins = Coins::default());
    assert!(!walk(&mut w, id, 0));
    assert_eq!(
        lines(&take(&mut w, id)),
        ["You do not have enough to cover the toll of 1 gold crown."]
    );
    // paid from a platinum piece: no whole coin fits 5 crowns, so the piece is broken
    // into 100 crowns and 5 of those are handed over
    set_exit(&mut w, hub(ExitType::Toll), 0, |e| e.par1 = 5);
    edit(&mut w, id, |c| {
        c.coins = Coins {
            platinum: 1,
            ..Coins::default()
        }
    });
    assert!(walk(&mut w, id, 0));
    let out = take(&mut w, id);
    assert!(
        esc(&out).starts_with("\\x1b[0;37;40mYou just paid 5 gold crowns in toll charges.\\r\\n"),
        "{}",
        esc(&out)
    );
    r.one(
        "toll_paid",
        b"\x1b[0;37;40mYou just paid 5 gold crowns in toll charges.\r\n",
    );
    let coins = w.session(id).unwrap().player().unwrap().coins;
    assert_eq!(coins.platinum, 0);
    // 1 platinum = 100 gold; 5 paid, 95 gold of change
    assert_eq!(coins.gold, 95);
    // the mixed list: 1 crown and 10 nobles pay 2 crowns from 1 gold, 15 silver
    let id2 = player(&mut w, "Bob", hub(ExitType::Toll));
    set_exit(&mut w, hub(ExitType::Toll), 0, |e| e.par1 = 2);
    edit(&mut w, id2, |c| {
        c.coins = Coins {
            gold: 1,
            silver: 15,
            ..Coins::default()
        }
    });
    assert!(walk(&mut w, id2, 0));
    assert_eq!(
        lines(&take(&mut w, id2))[0],
        "You just paid 1 gold crown, 10 silver nobles in toll charges."
    );
    assert_eq!(w.session(id2).unwrap().player().unwrap().coins.silver, 5);
    insta::assert_snapshot!("toll_refusal", r.snapshot());
}

// EXIT_ACTION
#[test]
fn action_exit() {
    let mut w = world();
    let id = player(&mut w, "Aldric", hub(ExitType::Action));
    let watcher = player(&mut w, "Bob", hub(ExitType::Action));
    let beyond = player(&mut w, "Cid", far(ExitType::Action));
    // exitpar1s 0: a plain exit
    assert!(walk(&mut w, id, 0));
    assert!(lines(&take(&mut w, watcher))[0].starts_with("Aldric just left to the north."));
    take(&mut w, beyond);
    edit(&mut w, id, |c| c.room = RoomId(hub(ExitType::Action)));
    take(&mut w, id);
    // with its message: the three texts, the room shown, prompt-less
    set_exit(&mut w, hub(ExitType::Action), 0, |e| e.par1 = 9003);
    assert!(walk(&mut w, id, 0));
    assert_eq!(room_of(&w, id), (1, far(ExitType::Action)));
    let own = take(&mut w, id);
    assert!(
        esc(&own).starts_with(
            "\\x1b[79D\\x1b[K\\x1b[0;32mYou hop in one of the skiffs.\\r\\n\\x1b[79D\\x1b[K\\x1b[1;36mFar 5"
        ),
        "{}",
        esc(&own)
    );
    assert_eq!(
        esc(&take(&mut w, watcher)),
        "\\x1b[79D\\x1b[K\\x1b[1;33m\\x1b[0;32mAldric hops in one of the skiffs.\\r\\n<P>"
    );
    assert_eq!(lines(&take(&mut w, beyond)), ["Aldric rows in."]);
}

// EXIT_HIDDEN
#[test]
fn hidden_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Hidden));
    set_exit(&mut w, hub(ExitType::Hidden), 0, |e| {
        e.par1 = ExitFlags::NEEDSEARCH.bits()
    });
    assert!(!walk(&mut w, id, 0));
    r.one("no_exit", &take(&mut w, id));
    for flag in [
        ExitFlags::PASSABLE,
        ExitFlags::VISSEARCHED,
        ExitFlags::VISBLOCKED,
    ] {
        set_exit(&mut w, hub(ExitType::Hidden), 0, |e| e.par1 = flag.bits());
        assert!(walk(&mut w, id, 0), "{flag:?}");
        edit(&mut w, id, |c| c.room = RoomId(hub(ExitType::Hidden)));
        take(&mut w, id);
    }
    insta::assert_snapshot!("hidden_refusal", r.snapshot());
}

// EXIT_DOOR / EXIT_GATE: exitpar1s 1 "Closed!", 2 "closed!"
#[test]
fn door_and_gate_exits() {
    let mut w = world();
    let mut r = Refusals::new();
    for (kind, one, two) in [
        (ExitType::Door, "The door is Closed!", "The door is closed!"),
        (ExitType::Gate, "The gate is Closed!", "The gate is closed!"),
    ] {
        let id = player(&mut w, "Aldric", hub(kind));
        set_exit(&mut w, hub(kind), 0, |e| e.par1 = LOCK_CLOSED);
        assert!(!walk(&mut w, id, 0));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [one]);
        r.one("door_closed", &out);
        set_exit(&mut w, hub(kind), 0, |e| e.par1 = LOCK_LOCKED);
        assert!(!walk(&mut w, id, 0));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [two]);
        r.one("door_closed", &out);
        set_exit(&mut w, hub(kind), 0, |e| e.par1 = LOCK_OPEN);
        assert!(walk(&mut w, id, 0));
        w.sessions.remove(&id);
    }
    insta::assert_snapshot!("door_gate_refusals", r.snapshot());
}

/// The lock state of the hub's north exit as it stands now.
fn lock(world: &World, kind: ExitType) -> u32 {
    let e = live_exit(world, RoomKey::new(1, hub(kind)), Direction::North).unwrap();
    if kind == ExitType::Key {
        e.par2 as u32
    } else {
        e.par1
    }
}

/// The lock state of the far room's south exit (the same door from the other side).
fn far_lock(world: &World, kind: ExitType) -> u32 {
    let e = live_exit(world, RoomKey::new(1, far(kind)), Direction::South).unwrap();
    if kind == ExitType::Key {
        e.par2 as u32
    } else {
        e.par1
    }
}

fn cmd(line: &str) -> CommandLine {
    CommandLine::parse(line)
}

// cmd_open, cmd_close, cmd_lock, cmd_picklock, cmd_bash: the lock states against the five
// commands, for a door, a gate and a key exit
#[test]
fn lock_states_against_open_close_lock_bash_picklock() {
    let mut w = world();
    let mut r = Refusals::new();
    for kind in [ExitType::Door, ExitType::Gate, ExitType::Key] {
        let what = if kind == ExitType::Gate {
            "gate"
        } else {
            "door"
        };
        let id = player(&mut w, "Aldric", hub(kind));
        let watcher = player(&mut w, "Bob", hub(kind));
        let beyond = player(&mut w, "Cid", far(kind));
        let set = |w: &mut World, state: u32| {
            set_exit(w, hub(kind), 0, |e| {
                if kind == ExitType::Key {
                    e.par2 = state as i16;
                } else {
                    e.par1 = state;
                }
            });
            set_exit(w, far(kind), 1, |e| {
                if kind == ExitType::Key {
                    e.par2 = state as i16;
                } else {
                    e.par1 = state;
                }
            });
            if let Some(s) = w.room_state.get_mut(&RoomKey::new(1, hub(kind))) {
                s.exit_overrides = Default::default();
            }
            if let Some(s) = w.room_state.get_mut(&RoomKey::new(1, far(kind))) {
                s.exit_overrides = Default::default();
            }
        };
        // --- OPEN
        set(&mut w, LOCK_OPEN);
        doors::cmd_open(&mut w, id, &cmd("open n"));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [format!("The {what} was already open.")]);
        r.one("door_already_open", &out);
        set(&mut w, LOCK_LOCKED);
        doors::cmd_open(&mut w, id, &cmd("open n"));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [format!("The {what} is locked.")]);
        r.one("door_locked", &out);
        set(&mut w, LOCK_CLOSED);
        doors::cmd_open(&mut w, id, &cmd("open n"));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [format!("The {what} is now open.")]);
        r.one("door_opened", &out);
        assert_eq!(lock(&w, kind), LOCK_OPEN);
        assert_eq!(far_lock(&w, kind), LOCK_OPEN, "the other side opens too");
        let out = take(&mut w, watcher);
        assert_eq!(
            lines(&out),
            [format!("You see Aldric open the {what} to the north.")]
        );
        r.one("door_others", &out);
        let out = take(&mut w, beyond);
        assert_eq!(
            lines(&out),
            [format!("The {what} to the south just opened.")]
        );
        r.one("door_moved_remote", &out);
        assert_eq!(w.session(id).unwrap().player().unwrap().delay, 1);
        edit(&mut w, id, |c| c.delay = 0);
        // --- CLOSE
        doors::cmd_close(&mut w, id, &cmd("close n"));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [format!("The {what} is now closed.")]);
        r.one("door_closed_now", &out);
        assert_eq!(lock(&w, kind), LOCK_CLOSED);
        assert_eq!(far_lock(&w, kind), LOCK_CLOSED);
        let out = take(&mut w, watcher);
        assert_eq!(
            lines(&out),
            [format!("You see Aldric close the {what} to the north.")]
        );
        r.one("door_others", &out);
        let out = take(&mut w, beyond);
        assert_eq!(
            lines(&out),
            [format!("The {what} to the south just closed.")]
        );
        r.one("door_moved_remote", &out);
        doors::cmd_close(&mut w, id, &cmd("close n"));
        let out = take(&mut w, id);
        // (sic) a Key exit's "not open" line says gate
        let sic = if kind == ExitType::Key { "gate" } else { what };
        assert_eq!(
            lines(&out),
            [format!(
                "That {sic} is not open. Closing it will do nothing!"
            )]
        );
        r.one("door_not_open", &out);
        edit(&mut w, id, |c| c.delay = 0);
        // --- LOCK
        doors::cmd_lock(&mut w, id, &cmd("lock n"));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), [format!("The {what} is now locked.")]);
        r.one("door_locked_now", &out);
        assert_eq!(lock(&w, kind), LOCK_LOCKED);
        assert_eq!(far_lock(&w, kind), LOCK_LOCKED);
        assert_eq!(w.session(id).unwrap().player().unwrap().delay, 2);
        edit(&mut w, id, |c| c.delay = 0);
        set(&mut w, LOCK_OPEN);
        doors::cmd_lock(&mut w, id, &cmd("lock n"));
        let out = take(&mut w, id);
        assert_eq!(
            lines(&out),
            [format!("You must close the {what} before you may lock it.")]
        );
        r.one("lock_need_close", &out);
        edit(&mut w, id, |c| c.delay = 0);
        // --- PICKLOCK: skill 100 plus the exit's bonus always opens the lock
        set(&mut w, LOCK_LOCKED);
        edit(&mut w, id, |c| c.picklocks = 100);
        doors::cmd_picklock(&mut w, id, &cmd("picklock n"));
        let out = take(&mut w, id);
        assert_eq!(
            lines(&out),
            [format!("You successfully unlocked the {what}.")]
        );
        r.one("unlocked", &out);
        assert_eq!(lock(&w, kind), LOCK_CLOSED, "unlocked is the closed state");
        assert_eq!(far_lock(&w, kind), LOCK_CLOSED);
        let out = take(&mut w, watcher);
        assert_eq!(
            lines(&out),
            [format!(
                "You see Aldric pick the lock on the {what} to the north."
            )]
        );
        r.one("door_others", &out);
        assert_eq!(w.session(id).unwrap().player().unwrap().delay, 2);
        edit(&mut w, id, |c| {
            c.delay = 0;
            c.picklocks = 0;
        });
        set(&mut w, LOCK_LOCKED);
        doors::cmd_picklock(&mut w, id, &cmd("picklock n"));
        let out = take(&mut w, id);
        assert_eq!(lines(&out), ["Your skill fails you this time."]);
        r.one("picklock_fail", &out);
        assert_eq!(w.session(id).unwrap().player().unwrap().delay, 4);
        edit(&mut w, id, |c| c.delay = 0);
        // --- BASH (a door or gate only; a key exit is the attack, T-033)
        if kind != ExitType::Key {
            set(&mut w, LOCK_LOCKED);
            // str 200: a roll of 0..100 is always below 200
            edit(&mut w, id, |c| c.str = 200);
            assert_eq!(
                doors::cmd_bash_door(&mut w, id, &cmd("bash n")),
                Some(silvermere_engine::commands::Handled::Done)
            );
            let out = take(&mut w, id);
            let ls = lines(&out);
            assert_eq!(ls[0], format!("You bashed the {what} open."));
            r.one(
                "bash_ok",
                format!("You bashed the {what} open.\r\n").as_bytes(),
            );
            if ls.len() > 1 {
                assert!(ls[1].starts_with("You take "), "{ls:?}");
            }
            assert_eq!(lock(&w, kind), LOCK_OPEN);
            assert_eq!(far_lock(&w, kind), LOCK_OPEN);
            let out = take(&mut w, watcher);
            assert_eq!(
                lines(&out),
                [format!("You see Aldric bash the {what} to the north.")]
            );
            r.one("door_others", &out);
            let out = take(&mut w, beyond);
            assert_eq!(
                lines(&out),
                [format!("The {what} to the south just flew open.")]
            );
            r.one("door_moved_remote", &out);
            edit(&mut w, id, |c| {
                c.delay = 0;
                c.str = 0;
            });
            // str 0 and no bonus: never
            set(&mut w, LOCK_LOCKED);
            doors::cmd_bash_door(&mut w, id, &cmd("bash n"));
            let out = take(&mut w, id);
            assert_eq!(lines(&out)[0], "Your attempts to bash through fail!");
            r.one("bash_fail", b"Your attempts to bash through fail!\r\n");
            let out = take(&mut w, watcher);
            assert_eq!(
                lines(&out),
                [format!(
                    "You see Aldric attempt to bash the {what} to the north."
                )]
            );
            r.one("door_others", &out);
            set(&mut w, LOCK_OPEN);
            doors::cmd_bash_door(&mut w, id, &cmd("bash n"));
            assert_eq!(
                lines(&take(&mut w, id))[0],
                format!("The {what} is already open.")
            );
        } else {
            assert_eq!(doors::cmd_bash_door(&mut w, id, &cmd("bash n")), None);
            take(&mut w, id);
        }
        edit(&mut w, id, |c| c.delay = 0);
        for s in [id, watcher, beyond] {
            w.sessions.remove(&s);
        }
    }
    // the odd words
    let id = player(&mut w, "Dora", hub(ExitType::Normal));
    doors::cmd_open(&mut w, id, &cmd("open n"));
    r.one("not_a_door", &take(&mut w, id));
    doors::cmd_open(&mut w, id, &cmd("open"));
    r.one("syntax", &take(&mut w, id));
    doors::cmd_close(&mut w, id, &cmd("close"));
    r.one("syntax", &take(&mut w, id));
    doors::cmd_lock(&mut w, id, &cmd("lock"));
    r.one("syntax", &take(&mut w, id));
    doors::cmd_lock(&mut w, id, &cmd("lock n"));
    r.one("lock_no_benefit", &take(&mut w, id));
    doors::cmd_lock(&mut w, id, &cmd("lock s"));
    r.one("look_no_exit", &take(&mut w, id));
    doors::cmd_picklock(&mut w, id, &cmd("picklock sideways"));
    r.one("syntax", &take(&mut w, id));
    insta::assert_snapshot!("lock_state_lines", r.snapshot());
}

// EXIT_MAPCHANGE: into map exitpar1s (add-on maps are always on)
#[test]
fn map_change_exit() {
    let mut w = world();
    let id = player(&mut w, "Aldric", hub(ExitType::MapChange));
    set_exit(&mut w, hub(ExitType::MapChange), 0, |e| {
        e.target = RoomId(OTHER_MAP_ROOM);
        e.par1 = 2;
    });
    assert!(walk(&mut w, id, 0));
    assert_eq!(room_of(&w, id), (2, OTHER_MAP_ROOM));
    let p = w.session(id).unwrap().player().unwrap();
    assert_eq!(
        (p.trailmaps[0], p.trailrooms[0]),
        (MapId(2), RoomId(OTHER_MAP_ROOM))
    );
}

// EXIT_TRAP: exitpar1s/2 to exitpar1s+1 damage
#[test]
fn trap_exit() {
    let mut w = world_with(256);
    let mut r = Refusals::new();
    let watcher = player(&mut w, "Bob", hub(ExitType::Trap));
    let beyond = player(&mut w, "Cid", far(ExitType::Trap));
    set_exit(&mut w, hub(ExitType::Trap), 0, |e| {
        e.par1 = 10;
        e.par3 = 9006;
    });
    let mut seen = [0u32; 11];
    for i in 0..200 {
        let id = player(&mut w, &format!("W{i}"), hub(ExitType::Trap));
        edit(&mut w, id, |c| {
            c.hp = 100;
            c.maxhp = 100;
        });
        assert!(walk(&mut w, id, 0));
        assert_eq!(room_of(&w, id), (1, far(ExitType::Trap)));
        let hp = w.session(id).unwrap().player().unwrap().hp;
        let dmg = 100 - hp;
        assert!((5..=10).contains(&dmg), "damage {dmg}");
        seen[dmg as usize] += 1;
        let own = lines(&take(&mut w, id));
        assert_eq!(
            own[0],
            format!("You fall through a trap door for {dmg} damage!")
        );
        assert_eq!(
            lines(&take(&mut w, watcher)),
            [format!("W{i} falls through a trap door!")]
        );
        assert_eq!(
            lines(&take(&mut w, beyond)),
            [format!("W{i} drops in from above!")]
        );
        w.sessions.remove(&id);
    }
    assert!(seen[5..=10].iter().all(|&n| n > 0), "{seen:?}");
    // kind 2 springs only on purpose (how 7); kinds 1 and 4 are ordinary exits
    let id = player(&mut w, "Aldric", hub(ExitType::Trap));
    set_exit(&mut w, hub(ExitType::Trap), 0, |e| e.par2 = 2);
    assert!(!walk(&mut w, id, 0));
    r.one("no_exit", &take(&mut w, id));
    assert!(!exit_is_available(
        &w,
        RoomKey::new(1, hub(ExitType::Trap)),
        Direction::North
    ));
    assert!(walk(&mut w, id, 7));
    edit(&mut w, id, |c| {
        c.room = RoomId(hub(ExitType::Trap));
        c.hp = 100;
    });
    take(&mut w, id);
    set_exit(&mut w, hub(ExitType::Trap), 0, |e| e.par2 = 1);
    assert!(walk(&mut w, id, 0));
    assert_eq!(w.session(id).unwrap().player().unwrap().hp, 100);
    insta::assert_snapshot!("trap_refusal", r.snapshot());
}

// EXIT_TEXT
#[test]
fn text_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Text));
    let watcher = player(&mut w, "Bob", hub(ExitType::Text));
    let beyond = player(&mut w, "Cid", far(ExitType::Text));
    set_exit(&mut w, hub(ExitType::Text), 0, |e| {
        e.par1 = 9004;
        e.par3 = 9005;
    });
    // a plain walk: there is no exit
    assert!(!walk(&mut w, id, 0));
    r.one("no_exit", &take(&mut w, id));
    assert!(!exit_is_available(
        &w,
        RoomKey::new(1, hub(ExitType::Text)),
        Direction::North
    ));
    // the words (how 4): the texts, the room, the prompt
    assert!(walk(&mut w, id, 4));
    assert_eq!(room_of(&w, id), (1, far(ExitType::Text)));
    let own = take(&mut w, id);
    assert!(
        esc(&own).starts_with(
            "\\x1b[79D\\x1b[K\\x1b[1;33mYou row to the far shore.\\r\\n\\x1b[79D\\x1b[K\\x1b[1;36mFar 10"
        ),
        "{}",
        esc(&own)
    );
    assert!(esc(&own).ends_with("<P>"));
    assert_eq!(
        esc(&take(&mut w, watcher)),
        "\\x1b[79D\\x1b[K\\x1b[1;33mAldric\\x1b[0;32m rows away.\\r\\n<P>"
    );
    assert_eq!(
        esc(&take(&mut w, beyond)),
        "\\x1b[79D\\x1b[K\\x1b[0;32m\\x1b[1;33mAldric\\x1b[0;32m rows in from the shore.\\r\\n<P>"
    );
    insta::assert_snapshot!("text_refusal", r.snapshot());
}

// EXIT_REMOTE: never walked
#[test]
fn remote_action_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::RemoteAction));
    assert!(!walk(&mut w, id, 0));
    r.one("no_exit", &take(&mut w, id));
    assert!(!walk(&mut w, id, 4));
    take(&mut w, id);
    assert!(!exit_is_available(
        &w,
        RoomKey::new(1, hub(ExitType::RemoteAction)),
        Direction::North
    ));
    insta::assert_snapshot!("remote_refusal", r.snapshot());
}

// EXIT_CLASS / EXIT_RACE
#[test]
fn class_and_race_exits() {
    let mut w = world();
    let mut r = Refusals::new();
    for kind in [ExitType::Class, ExitType::Race] {
        let id = player(&mut w, "Aldric", hub(kind));
        edit(&mut w, id, |c| {
            c.class = silvermere_types::ids::ClassNo(3);
            c.race = silvermere_types::ids::RaceNo(3);
        });
        // only number 7 may pass
        set_exit(&mut w, hub(kind), 0, |e| e.par1 = 7);
        assert!(!walk(&mut w, id, 0));
        r.one("exit_forbidden", &take(&mut w, id));
        set_exit(&mut w, hub(kind), 0, |e| e.par1 = 3);
        assert!(walk(&mut w, id, 0));
        edit(&mut w, id, |c| c.room = RoomId(hub(kind)));
        take(&mut w, id);
        // everyone but number 3 may pass
        set_exit(&mut w, hub(kind), 0, |e| {
            e.par1 = 0;
            e.par2 = 3;
        });
        assert!(!walk(&mut w, id, 0));
        r.one("exit_forbidden", &take(&mut w, id));
        set_exit(&mut w, hub(kind), 0, |e| e.par2 = 4);
        assert!(walk(&mut w, id, 0));
        take(&mut w, id);
        w.sessions.remove(&id);
    }
    // the tournament variant
    w.config.tournament_play = true;
    let id = player(&mut w, "Bob", hub(ExitType::Class));
    assert!(!walk(&mut w, id, 0));
    r.one("exit_forbidden", &take(&mut w, id));
    insta::assert_snapshot!("class_race_refusals", r.snapshot());
}

// EXIT_LEVEL: 1:33 -> 1:296 is 0-3
#[test]
fn level_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Level));
    set_exit(&mut w, hub(ExitType::Level), 0, |e| {
        e.par1 = 0;
        e.par2 = 3;
    });
    edit(&mut w, id, |c| c.level = 4);
    assert!(!walk(&mut w, id, 0));
    r.one("exit_level_high", &take(&mut w, id));
    edit(&mut w, id, |c| c.level = 3);
    assert!(walk(&mut w, id, 0));
    edit(&mut w, id, |c| c.room = RoomId(hub(ExitType::Level)));
    take(&mut w, id);
    set_exit(&mut w, hub(ExitType::Level), 0, |e| e.par1 = 5);
    edit(&mut w, id, |c| c.level = 1);
    assert!(!walk(&mut w, id, 0));
    r.one("exit_level_low", &take(&mut w, id));
    edit(&mut w, id, |c| c.level = 5);
    assert!(!walk(&mut w, id, 0));
    take(&mut w, id);
    set_exit(&mut w, hub(ExitType::Level), 0, |e| e.par2 = 0);
    assert!(walk(&mut w, id, 0));
    insta::assert_snapshot!("level_refusals", r.snapshot());
}

// EXIT_TIMED: closed while exitpar3s != 0
#[test]
fn timed_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Timed));
    set_exit(&mut w, hub(ExitType::Timed), 0, |e| e.par3 = 2);
    assert!(!walk(&mut w, id, 0));
    r.one("exit_timed", &take(&mut w, id));
    set_exit(&mut w, hub(ExitType::Timed), 0, |e| e.par1 = 9008);
    assert!(!walk(&mut w, id, 0));
    assert_eq!(
        lines(&take(&mut w, id)),
        ["The portcullis to the north is down."]
    );
    set_exit(&mut w, hub(ExitType::Timed), 0, |e| e.par3 = 0);
    assert!(walk(&mut w, id, 0));
    insta::assert_snapshot!("timed_refusal", r.snapshot());
}

// Ticket, User Count, Block Guard and Delay pass like ordinary exits
#[test]
fn ticket_usercount_blockguard_delay_exits_pass() {
    let mut w = world();
    for kind in [
        ExitType::Ticket,
        ExitType::UserCount,
        ExitType::BlockGuard,
        ExitType::Delay,
    ] {
        let id = player(&mut w, "Aldric", hub(kind));
        set_exit(&mut w, hub(kind), 0, |e| {
            e.par1 = 5;
            e.par2 = 5;
        });
        assert!(walk(&mut w, id, 0), "{kind:?}");
        assert_eq!(room_of(&w, id), (1, far(kind)));
        assert!(exit_is_available(
            &w,
            RoomKey::new(1, hub(kind)),
            Direction::North
        ));
        w.sessions.remove(&id);
    }
}

// EXIT_ALIGNMENT: evil between exitpar1s and exitpar2s
#[test]
fn alignment_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Alignment));
    set_exit(&mut w, hub(ExitType::Alignment), 0, |e| {
        e.par1 = 10;
        e.par2 = 50;
    });
    edit(&mut w, id, |c| c.evil = 0);
    assert!(!walk(&mut w, id, 0));
    r.one("exit_alignment", &take(&mut w, id));
    edit(&mut w, id, |c| c.evil = 80);
    assert!(!walk(&mut w, id, 0));
    r.one("exit_alignment", &take(&mut w, id));
    edit(&mut w, id, |c| c.evil = 30);
    assert!(walk(&mut w, id, 0));
    // a dragged player is refused before the alignment is looked at
    let victim = player(&mut w, "Bob", hub(ExitType::Alignment));
    edit(&mut w, victim, |c| c.hp = 0);
    edit(&mut w, id, |c| c.room = RoomId(hub(ExitType::Alignment)));
    take(&mut w, id);
    assert!(!silvermere_engine::movement::move_user_with(
        &mut w,
        id,
        0,
        2,
        false,
        Some(victim)
    ));
    assert_eq!(
        esc(&take(&mut w, id)),
        "\\x1b[79D\\x1b[KYou may not drag anyone through this exit.\\r\\n"
    );
    insta::assert_snapshot!("alignment_refusals", r.snapshot());
}

// EXIT_CAST and its epilogue
#[test]
fn cast_exit() {
    let mut w = world();
    let id = player(&mut w, "Aldric", hub(ExitType::Cast));
    let watcher = player(&mut w, "Bob", hub(ExitType::Cast));
    // exitpar1s 0: an ordinary exit
    assert!(walk(&mut w, id, 0));
    edit(&mut w, id, |c| c.room = RoomId(hub(ExitType::Cast)));
    take(&mut w, id);
    take(&mut w, watcher);
    // with a spell (T-051's hook does nothing yet) and message exitpar3s
    set_exit(&mut w, hub(ExitType::Cast), 0, |e| {
        e.par1 = 12;
        e.par3 = 9009;
    });
    assert!(walk(&mut w, id, 0));
    let own = lines(&take(&mut w, id));
    assert_eq!(own[0], "A chill passes through you.");
    // msgs[1] to the room left before the move, msgs[2] to the room the player is in
    // after the attempt (he moved, so the watcher does not see it)
    assert_eq!(
        lines(&take(&mut w, watcher)),
        ["Aldric shivers.", "Aldric just left to the north."]
    );
    // held after the spell: he stays, with the epilogue only
    edit(&mut w, id, |c| {
        c.room = RoomId(hub(ExitType::Cast));
        c.status2 |= PlayerStatus2::HELD;
    });
    assert!(!walk(&mut w, id, 0));
    assert_eq!(
        lines(&take(&mut w, id)),
        ["You can't seem to move anywhere!"]
    );
}

// EXIT_ABILITY: ability exitpar1s between exitpar2s and exitpar3s
#[test]
fn ability_exit() {
    let mut w = world();
    let id = player(&mut w, "Aldric", hub(ExitType::Ability));
    let watcher = player(&mut w, "Bob", hub(ExitType::Ability));
    // ability 31 (Bash) between 5 and 20; nobody has it: refused silently without a
    // message record
    set_exit(&mut w, hub(ExitType::Ability), 0, |e| {
        e.par1 = 31;
        e.par2 = 5;
        e.par3 = 20;
    });
    assert!(!walk(&mut w, id, 0));
    assert_eq!(take(&mut w, id), b"");
    // with message exitpar4s
    set_exit(&mut w, hub(ExitType::Ability), 0, |e| e.par4 = 9010);
    assert!(!walk(&mut w, id, 0));
    assert_eq!(
        lines(&take(&mut w, id)),
        ["Your mind is too weak for this."]
    );
    assert_eq!(
        lines(&take(&mut w, watcher)),
        ["Aldric frowns at the arch."]
    );
    // the record's own slot gives it
    edit(&mut w, id, |c| {
        c.abils[0] = (silvermere_types::enums::Ability::Bash, 10)
    });
    assert!(walk(&mut w, id, 0));
    edit(&mut w, id, |c| {
        c.room = RoomId(hub(ExitType::Ability));
        c.abils[0] = (silvermere_types::enums::Ability::Bash, 25);
    });
    take(&mut w, id);
    assert!(!walk(&mut w, id, 0), "over the top");
}

// The exit's ability counts every source: a worn item's and the race's add to the total
#[test]
fn ability_exit_counts_items_and_race() {
    let mut w = world();
    let room = hub(ExitType::Ability);
    let id = player(&mut w, "Aldric", room);
    set_exit(&mut w, room, 0, |e| {
        e.par1 = 31;
        e.par2 = 5;
        e.par3 = 20;
    });
    w.items.get_mut(&ItemNo(1)).unwrap().abilities[0] =
        (silvermere_types::enums::Ability::Bash, 10);
    edit(&mut w, id, |c| c.worn[0] = ItemNo(1));
    assert!(walk(&mut w, id, 0), "the worn item gives 10");
    // the race adds 15 more: 25, over the top
    w.races
        .get_mut(&silvermere_types::ids::RaceNo(1))
        .unwrap()
        .abilities[0] = (silvermere_types::enums::Ability::Bash, 15);
    edit(&mut w, id, |c| c.room = RoomId(room));
    take(&mut w, id);
    assert!(!walk(&mut w, id, 0), "10 + 15 is over the top");
    // the race alone: 15
    edit(&mut w, id, |c| c.worn[0] = ItemNo(0));
    assert!(walk(&mut w, id, 0), "the race gives 15");
}

// EXIT_SPELLTRAP: the room texts end with a blank line
#[test]
fn spell_trap_exit() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::SpellTrap));
    let watcher = player(&mut w, "Bob", hub(ExitType::SpellTrap));
    let beyond = player(&mut w, "Cid", far(ExitType::SpellTrap));
    set_exit(&mut w, hub(ExitType::SpellTrap), 0, |e| {
        e.par1 = 12;
        e.par4 = 9007;
    });
    assert!(walk(&mut w, id, 0));
    assert_eq!(room_of(&w, id), (1, far(ExitType::SpellTrap)));
    assert_eq!(lines(&take(&mut w, id))[0], "Sparks fly around you.");
    assert_eq!(
        esc(&take(&mut w, watcher)),
        "\\x1b[0;32mAldric trips a glyph.\\r\\n\\r\\n<P>"
    );
    assert_eq!(
        esc(&take(&mut w, beyond)),
        "\\x1b[0;32mAldric staggers in.\\r\\n\\r\\n<P>"
    );
    edit(&mut w, id, |c| c.room = RoomId(hub(ExitType::SpellTrap)));
    set_exit(&mut w, hub(ExitType::SpellTrap), 0, |e| e.par2 = 2);
    assert!(!walk(&mut w, id, 0));
    r.one("no_exit", &take(&mut w, id));
    insta::assert_snapshot!("spell_trap_refusal", r.snapshot());
}

// user_allowed_in_room: an arena above its maxlevel
#[test]
fn arena_maxlevel() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", ARENA_HUB);
    edit(&mut w, id, |c| c.level = 4);
    assert!(!walk(&mut w, id, 0));
    r.room_refusal("exit_level_high", &take(&mut w, id));
    edit(&mut w, id, |c| c.level = 3);
    assert!(walk(&mut w, id, 0));
    assert_eq!(room_of(&w, id), (1, ARENA));
    insta::assert_snapshot!("arena_refusal", r.snapshot());
}

// The status refusals before any exit is looked at
#[test]
fn status_refusals() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Normal));
    edit(&mut w, id, |c| c.status2 |= PlayerStatus2::HELD);
    assert!(!walk(&mut w, id, 0));
    r.one("cant_move", &take(&mut w, id));
    edit(&mut w, id, |c| {
        c.status2.remove(PlayerStatus2::HELD);
        c.status2 |= PlayerStatus2::KNOCKEDDOWN;
    });
    assert!(!walk(&mut w, id, 0));
    r.one("stunned", &take(&mut w, id));
    // over 100% encumbrance: str 10 carries 480; 600 is too much
    edit(&mut w, id, |c| {
        c.status2.remove(PlayerStatus2::KNOCKEDDOWN);
        c.str = 10;
        c.maxwgt = 480;
        c.wgt = 600;
    });
    assert!(!walk(&mut w, id, 0));
    r.one("too_heavy", &take(&mut w, id));
    assert_eq!(w.session(id).unwrap().player().unwrap().encumpct, 125);
    // the dispatcher's cmd_move has its own wording
    edit(&mut w, id, |c| c.movedir = -1);
    silvermere_engine::commands::movement::cmd_move(&mut w, id, Cmd::North, &cmd("n"));
    r.one("too_heavy", &take(&mut w, id));
    insta::assert_snapshot!("status_refusals", r.snapshot());
}

// Every refusal in one place: the lines above, byte-exact, against the registry.
#[test]
fn refusals_match_the_registry_and_the_snapshot() {
    let mut w = world();
    let mut r = Refusals::new();
    let id = player(&mut w, "Aldric", hub(ExitType::Normal));
    let watcher = player(&mut w, "Bob", hub(ExitType::Normal));
    // user_allowed_in_room's other refusals need T-034's combat
    // state; the unhealthy one is reachable: a colliseum from outside at under half hp
    w.rooms
        .get_mut(&RoomKey::new(1, far(ExitType::Normal)))
        .unwrap()
        .room_type = RoomType::Colliseum;
    edit(&mut w, id, |c| {
        c.hp = 4;
        c.maxhp = 10;
    });
    assert!(!walk(&mut w, id, 0));
    r.room_refusal("exit_unhealthy", &take(&mut w, id));
    edit(&mut w, id, |c| c.hp = 5);
    assert!(walk(&mut w, id, 0));
    // the arena-safe entry keeps his hit points for later
    assert_eq!(w.session(id).unwrap().player().unwrap().arenahp, 5);
    let ls = lines(&take(&mut w, watcher));
    assert_eq!(ls, ["Aldric just left to the north."]);
    assert!(r.registry.get("player_leaves").is_match(&ls[0]));
    insta::assert_snapshot!("unhealthy_refusal", r.snapshot());
}
