//! Walking the world (T-026): the departure, arrival, noise and follower lines seen by
//! three sessions in the right rooms and order; `There is no exit in that direction!` for
//! a missing, a remote and an unfound hidden exit; `search` finding the slum's hidden exit
//! at 1:619 west; a bashed door relocking after `exitpar3 * 300`
//! fast ticks on the `TestClock`; a monster's free attack and a follower on leaving; the
//! queued move of the ten direction commands and of DRAG through the engine.
//!
//! The rooms: a lane of three on map 1 (`WEST_END` - `MIDDLE` - `EAST_END`, east-west),
//! with `NORTH_OF_MIDDLE` north of the middle; the slum street 619 with its hidden exit
//! west to the sewer 620; a door room pair for the bash.

mod common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, test_config, tiny_snapshot};
use silvermere_engine::commands::movement::cmd_move;
use silvermere_engine::commands::{Cmd, CommandLine};
use silvermere_engine::movement::exits::{LOCK_LOCKED, LOCK_OPEN, live_exit};
use silvermere_engine::movement::move_user;
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, SeededRng, SessionId, World, WorldSnapshot,
};
use silvermere_types::enums::{
    Alignment, Direction, ExitType, GroupRole, MessageLevel, MonsterType,
};
use silvermere_types::flags::{ActiveMonsterFlags, ExitFlags, PlayerStatus};
use silvermere_types::ids::{ActiveMonsterId, ItemNo, MapId, MonsterNo, RoomId, RoomKey, SpellNo};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins, Exit};

const WEST_END: u32 = 6001;
const MIDDLE: u32 = 6002;
const EAST_END: u32 = 6003;
const NORTH_OF_MIDDLE: u32 = 6004;
const SLUM: u32 = 619;
const SEWER: u32 = 620;
const DOOR_ROOM: u32 = 6101;
const BEHIND_DOOR: u32 = 6102;

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

fn snapshot() -> WorldSnapshot {
    let mut s = tiny_snapshot();
    let room = |n: u32, name: &str| {
        let mut r = blank::clear_room(RoomKey::new(1, n));
        r.name = name.into();
        r
    };
    let mut west = room(WEST_END, "West End");
    west.exits[2] = exit(MIDDLE, ExitType::Normal);
    let mut middle = room(MIDDLE, "Middle");
    middle.exits[3] = exit(WEST_END, ExitType::Normal);
    middle.exits[2] = exit(EAST_END, ExitType::Normal);
    middle.exits[0] = exit(NORTH_OF_MIDDLE, ExitType::Normal);
    let mut east = room(EAST_END, "East End");
    east.exits[3] = exit(MIDDLE, ExitType::Normal);
    let mut north = room(NORTH_OF_MIDDLE, "North Side");
    north.exits[1] = exit(MIDDLE, ExitType::Normal);
    // the remote-action and hidden exits of the no-exit test
    north.exits[2] = exit(EAST_END, ExitType::RemoteAction);
    north.exits[3] = exit(WEST_END, ExitType::Hidden);
    north.exits[3].par1 = ExitFlags::NEEDSEARCH.bits();
    let mut slum = room(SLUM, "Slum Street");
    slum.exits[3] = exit(SEWER, ExitType::Hidden);
    slum.exits[3].par1 = ExitFlags::NEEDSEARCH.bits();
    slum.exits[2] = exit(MIDDLE, ExitType::Normal);
    let mut sewer = room(SEWER, "Slimy Sewer Tunnels");
    sewer.exits[2] = exit(SLUM, ExitType::Normal);
    let mut door_room = room(DOOR_ROOM, "Before the Door");
    door_room.exits[0] = exit(BEHIND_DOOR, ExitType::Door);
    door_room.exits[0].par1 = LOCK_LOCKED;
    door_room.exits[0].par3 = 2;
    let mut behind = room(BEHIND_DOOR, "Behind the Door");
    behind.exits[1] = exit(DOOR_ROOM, ExitType::Door);
    behind.exits[1].par1 = LOCK_LOCKED;
    behind.exits[1].par3 = 2;
    s.rooms
        .extend([west, middle, east, north, slum, sewer, door_room, behind]);
    s
}

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
}

impl Harness {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let world = World::load(test_config(), 16, snapshot()).expect("world loads");
        let engine = Engine::new(world, clock.clone(), SeededRng::new(26), tx);
        Self {
            engine,
            clock,
            rx,
            log: Vec::new(),
        }
    }

    fn w(&mut self) -> &mut World {
        &mut self.engine.world
    }

    fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            self.log.push(o);
        }
    }

    fn playing(&mut self, name: &str, room: u32) -> SessionId {
        player(&mut self.engine.world, name, room)
    }

    fn line(&mut self, id: SessionId, text: &str) {
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.pump();
    }

    fn ticks(&mut self, n: u32) {
        self.clock.advance_ticks(n, &mut self.engine);
        self.pump();
    }

    fn text_since(&self, from: usize, id: SessionId) -> String {
        let bytes: Vec<u8> = self.log[from..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Write { id: i, bytes } if *i == id => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn lines_since(&self, from: usize, id: SessionId) -> Vec<String> {
        lines(self.text_since(from, id).as_bytes())
    }
}

fn player(world: &mut World, name: &str, room: u32) -> SessionId {
    let id = enter(world, name, room, MessageLevel::High);
    edit(world, id, |c| {
        c.movedir = -1;
        c.str = 50;
        c.level = 1;
        c.perception = 115;
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

fn monster(world: &mut World, n: i32, room: u32, name: &str, following: &str, followpct: i16) {
    let m = ActiveMonster {
        number: ActiveMonsterId(n),
        knownnum: MonsterNo(1),
        level: 1,
        map: MapId(1),
        room: RoomId(room),
        poison: 0,
        energy: 0,
        hp: 5,
        following: following.into(),
        trail: std::array::from_fn(|_| RoomId::default()),
        hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
        hunting: ActiveMonsterId::default(),
        name: name.into(),
        wornitem: ItemNo::default(),
        weapon: ItemNo::default(),
        items: std::array::from_fn(|_| (ItemNo::default(), 0)),
        cash: Coins::default(),
        maxhp: 5,
        alignment: Alignment::Evil,
        followpct,
        dr: 0,
        ac: 0,
        exp: 0,
        maxenergy: 0,
        friendly: 0,
        genroom: RoomId(room),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: MonsterType::Lair,
        target: 0,
        hpregen: 0,
        lastdir: 0,
        lastupd: 0,
        grouprole: GroupRole::Solo,
        spells: std::array::from_fn(|_| ActiveSpell {
            spell: SpellNo::default(),
            value: 0,
            rounds: 0,
        }),
        downrounds: 0,
        created: 0,
    };
    world.active_monsters.insert(m);
    let state = world
        .room_state
        .get_mut(&RoomKey::new(1, room))
        .expect("room state");
    let slot = state.monsters.iter().position(|m| m.0 == 0).unwrap();
    state.monsters[slot] = ActiveMonsterId(n);
}

/// The bytes the session would receive now, with `<P>` where a prompt goes.
fn take(world: &mut World, id: SessionId) -> Vec<u8> {
    let palette = palette_of(world, id);
    let s = world.session_mut(id).unwrap();
    let out = s.output.render(palette, b"<P>");
    s.output.clear();
    out
}

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

/// The screen lines: ANSI and `X\b` pairs removed, the prompts (`<P>` from `take`, the
/// rendered `[HP=n]:` from the engine) taken out, split at `\r\n`, empty lines dropped.
fn lines(bytes: &[u8]) -> Vec<String> {
    let plain = silvermere_text::strip_ansi(&drop_backspace_pairs(bytes));
    let text = String::from_utf8_lossy(&plain).replace("<P>", "");
    let prompt = regex::Regex::new(r"\[HP=-?\d+\]:").unwrap();
    prompt
        .replace_all(&text, "")
        .split("\r\n")
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
        .collect()
}

/// How many prompts `bytes` carries.
fn prompts(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    text.matches("<P>").count() + text.matches("[HP=").count()
}

fn pattern(name: &str) -> regex::Regex {
    let reg: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
    let rx = reg["static"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no pattern {name}"))["rx"]
        .as_str()
        .unwrap()
        .to_owned();
    regex::Regex::new(&format!("^(?:{rx})$")).unwrap()
}

fn room_of(world: &World, id: SessionId) -> u32 {
    world.session(id).unwrap().player().unwrap().room.0
}

// ---------------------------------------------------------------------------------------

// The departure and arrival lines; the noise next door
#[test]
fn three_sessions_see_a_walk_in_order() {
    let mut w = World::load(test_config(), 16, snapshot()).unwrap();
    w.rng = SeededRng::new(3);
    let walker = player(&mut w, "Aldric", MIDDLE);
    let stays = player(&mut w, "Bob", MIDDLE);
    let ahead = player(&mut w, "Cid", EAST_END);
    let aside = player(&mut w, "Dora", NORTH_OF_MIDDLE);
    let behind = player(&mut w, "Eve", WEST_END);
    assert!(move_user(&mut w, walker, 2, 0));
    assert_eq!(room_of(&w, walker), EAST_END);
    // the room left: the departure, with the listener's prompt
    let out = take(&mut w, stays);
    assert_eq!(lines(&out), ["Aldric just left to the east."]);
    assert_eq!(prompts(&out), 1);
    assert!(pattern("player_leaves").is_match("Aldric just left to the east."));
    // the room entered: the noise next door went to the rooms off the new room before
    // the arrival line; Cid is in the new room itself
    let out = take(&mut w, ahead);
    assert_eq!(lines(&out), ["Aldric walks into the room from the west."]);
    assert!(pattern("player_arrives").is_match("Aldric walks into the room from the west."));
    // the rooms next to the new one: the new room's only other exit is back west to
    // the middle, where Bob hears movement from the east
    let out = take(&mut w, stays);
    assert_eq!(lines(&out), [] as [&str; 0]);
    // (the noise went out in the same batch as the departure: see below)
    let _ = aside;
    let out = take(&mut w, behind);
    assert_eq!(lines(&out), [] as [&str; 0]);
    // the walker: ANSI_NORMAL then the room, no prompt
    let own = take(&mut w, walker);
    assert!(esc(&own).starts_with("\\x1b[0;37;40m\\x1b[79D\\x1b[K\\x1b[1;36mEast End"));
    assert!(!own.windows(3).any(|w| w == b"<P>"));
}

// display_entry_movement before the lines; the order in one batch
#[test]
fn noise_reaches_the_rooms_beside_the_new_room_first() {
    let mut w = World::load(test_config(), 16, snapshot()).unwrap();
    w.rng = SeededRng::new(3);
    let walker = player(&mut w, "Aldric", WEST_END);
    let middle = player(&mut w, "Bob", MIDDLE);
    let east = player(&mut w, "Cid", EAST_END);
    let north = player(&mut w, "Dora", NORTH_OF_MIDDLE);
    assert!(move_user(&mut w, walker, 2, 0));
    // Bob, in the room entered, gets the arrival only (the noise goes to the other rooms)
    assert_eq!(
        lines(&take(&mut w, middle)),
        ["Aldric walks into the room from the west."]
    );
    // Cid east of the middle hears movement to the west; Dora north of it, to the south
    let out = take(&mut w, east);
    assert_eq!(lines(&out), ["You hear movement to the west."]);
    assert!(pattern("noise").is_match("You hear movement to the west."));
    assert_eq!(
        lines(&take(&mut w, north)),
        ["You hear movement to the south."]
    );
}

// The three wordings of "just left to the", with the anti-script
// bytes; up and down have their own
#[test]
fn departure_wordings() {
    let mut w = World::load(test_config(), 16, snapshot()).unwrap();
    let watcher = player(&mut w, "Bob", MIDDLE);
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..12u64 {
        w.rng = SeededRng::new(seed);
        let walker = player(&mut w, "Aldric", MIDDLE);
        assert!(move_user(&mut w, walker, 2, 0));
        let raw = take(&mut w, watcher);
        let text = String::from_utf8_lossy(&silvermere_text::strip_ansi(&raw)).into_owned();
        let text = text
            .trim_end_matches("<P>")
            .trim_end_matches("\r\n")
            .to_owned();
        assert_eq!(
            String::from_utf8_lossy(&drop_backspace_pairs(text.as_bytes())),
            "Aldric just left to the east."
        );
        seen.insert(text);
        take(&mut w, walker);
        w.sessions.remove(&walker);
        w.ids.free(walker);
    }
    let mut seen: Vec<String> = seen.into_iter().collect();
    seen.sort();
    assert_eq!(
        seen,
        [
            "Aldric just left to  \u{8}the ea \u{8}st.",
            "Aldric just left to t \u{8}he e \u{8}ast.",
            "Aldric just left to th \u{8}e eas \u{8}t.",
        ]
    );
}

// The party line, no room when following blind
#[test]
fn a_follower_is_told_where_the_march_went() {
    let mut w = World::load(test_config(), 16, snapshot()).unwrap();
    w.rng = SeededRng::new(3);
    let leader = player(&mut w, "Aldric", MIDDLE);
    let follower = player(&mut w, "Bob", MIDDLE);
    w.session_mut(follower).unwrap().party.leader = Some(leader);
    w.session_mut(leader).unwrap().party.group[0] = Some(follower);
    assert!(move_user(&mut w, leader, 0, 0));
    take(&mut w, follower);
    assert!(move_user(&mut w, follower, 0, 3));
    let out = take(&mut w, follower);
    let ls = lines(&out);
    assert_eq!(ls[0], " -- Following your Party leader north --");
    // the registry's `follow_leader` regex, unanchored: the line starts with a space
    assert!(
        regex::Regex::new(r"-- Following your Party leader (?P<dir>.+) --")
            .unwrap()
            .is_match(&ls[0])
    );
    assert_eq!(ls[1], "North Side");
    // a typed exit or a march ends with the prompt
    assert!(esc(&out).ends_with("<P>"));
    // held: the follower drops out of the march and is told
    edit(&mut w, follower, |c| {
        c.room = RoomId(MIDDLE);
        c.status2 |= silvermere_types::flags::PlayerStatus2::HELD;
    });
    assert!(!move_user(&mut w, follower, 0, 3));
    let out = take(&mut w, follower);
    assert_eq!(
        lines(&out),
        [
            "You can't seem to move anywhere!",
            "You are no longer following Aldric.",
        ]
    );
    // stop_following's prompt alone: a held follower returns before the march's own
    // prompt
    assert_eq!(prompts(&out), 1);
    assert_eq!(w.session(follower).unwrap().party.leader, None);
    assert!(w.session(leader).unwrap().party.members().next().is_none());
}

// Missing, remote, unfound hidden
#[test]
fn there_is_no_exit_for_a_missing_remote_and_unfound_hidden_exit() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", NORTH_OF_MIDDLE);
    let other = h.playing("Bob", NORTH_OF_MIDDLE);
    let no_exit = pattern("no_exit");
    // missing (up): cmd_move walks into the wall, the room sees it
    let mark = h.log.len();
    h.line(id, "u");
    let ls = h.lines_since(mark, id);
    assert!(no_exit.is_match(&ls[0]), "{ls:?}");
    let ls = h.lines_since(mark, other);
    assert_eq!(ls, ["Aldric ran into the wall to the up."]);
    assert!(pattern("ran_into_wall").is_match(&ls[0]));
    // remote (east) and unfound hidden (west): no exit either, and the player stays
    for word in ["e", "w"] {
        let mark = h.log.len();
        h.line(id, word);
        let ls = h.lines_since(mark, id);
        assert!(no_exit.is_match(&ls[0]), "{word}: {ls:?}");
        assert_eq!(room_of(h.w(), id), NORTH_OF_MIDDLE);
    }
    // the real exit south: queued with a one-tick delay, then walked on the tick
    let mark = h.log.len();
    h.line(id, "s");
    // nothing yet: the dispatcher's ANSI_NORMAL is dropped with the output buffer (the room-walk
    // capture shows a single one, the arrival's)
    assert_eq!(h.text_since(mark, id), "");
    assert_eq!(room_of(h.w(), id), NORTH_OF_MIDDLE);
    h.ticks(1);
    assert_eq!(room_of(h.w(), id), MIDDLE);
    let ls = h.lines_since(mark, id);
    assert_eq!(ls[0], "Middle");
    assert!(
        h.text_since(mark, id).ends_with("]:"),
        "the prompt after the room"
    );
    assert!(h.lines_since(mark, other)[0].starts_with("Aldric just left to the south."));
}

// The delay of a move
#[test]
fn the_move_delay() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", WEST_END);
    let delay = |h: &mut Harness| h.w().session(id).unwrap().player().unwrap().delay;
    assert_eq!(
        cmd_move(h.w(), id, Cmd::East, &CommandLine::parse("e")),
        silvermere_engine::commands::Handled::Wait
    );
    assert_eq!(delay(&mut h), 1);
    edit(h.w(), id, |c| {
        c.delay = 0;
        c.wgt = 400; // str 50 carries 2400; 400 is 16%
        c.status2 |= silvermere_types::flags::PlayerStatus2::SLOWED;
    });
    cmd_move(h.w(), id, Cmd::East, &CommandLine::parse("e"));
    assert_eq!(delay(&mut h), 2, "slowed doubles");
    edit(h.w(), id, |c| {
        c.delay = 0;
        c.wgt = 1700; // 70%: one more
    });
    cmd_move(h.w(), id, Cmd::East, &CommandLine::parse("e"));
    // (1 + 1) * 2, and moves is now 3: one more
    assert_eq!(delay(&mut h), 5);
}

// cmd_search: the slum's hidden exit at 1:619 west
#[test]
fn search_finds_the_slum_hidden_exit() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", SLUM);
    let other = h.playing("Bob", SLUM);
    let key = RoomKey::new(1, SLUM);
    // not found yet: no exit west, and the exits line hides it
    let mark = h.log.len();
    h.line(id, "w");
    assert!(pattern("no_exit").is_match(&h.lines_since(mark, id)[0]));
    // search north: nothing there
    let mark = h.log.len();
    h.line(id, "search n");
    let ls = h.lines_since(mark, id);
    assert_eq!(ls, ["You notice nothing different to the north."]);
    assert!(pattern("search_nothing_dir").is_match(&ls[0]));
    assert_eq!(
        h.lines_since(mark, other),
        ["Aldric is searching for exits."]
    );
    h.ticks(1);
    // search west: perception 115 makes the roll certain (max(3, 115-15) = 100)
    let mark = h.log.len();
    h.line(id, "search w");
    let ls = h.lines_since(mark, id);
    assert_eq!(ls, ["You found an exit to the west!"]);
    assert!(pattern("search_found_exit").is_match(&ls[0]));
    let exit = live_exit(h.w(), key, Direction::West).unwrap();
    assert_eq!(exit.par1, ExitFlags::VISSEARCHED.bits());
    // the content is untouched
    assert_eq!(
        h.w().room(key).unwrap().exits[3].par1,
        ExitFlags::NEEDSEARCH.bits()
    );
    h.ticks(1);
    // it shows in the exits line and can be walked
    let mark = h.log.len();
    h.line(id, "exits");
    assert_eq!(
        h.lines_since(mark, id),
        ["Obvious exits: east, secret passage west"]
    );
    let mark = h.log.len();
    h.line(id, "w");
    h.ticks(1);
    assert_eq!(room_of(h.w(), id), SEWER);
    assert_eq!(h.lines_since(mark, id)[0], "Slimy Sewer Tunnels");
    // one step (300 ticks) after the search it hides again
    h.ticks(300);
    let exit = live_exit(h.w(), key, Direction::West).unwrap();
    assert_eq!(exit.par1, ExitFlags::NEEDSEARCH.bits());
    // a general search: the room is told, and nothing is here
    let mark = h.log.len();
    h.line(other, "search");
    assert_eq!(
        h.lines_since(mark, other),
        ["Your search revealed nothing."]
    );
    h.ticks(1);
    let mark = h.log.len();
    h.line(other, "search here now");
    assert!(
        h.text_since(mark, other)
            .contains("Your command had no effect.")
    );
}

// cmd_search and the dispatcher's ANSI_NORMAL: the reset goes out with the first text sent.
// SEARCH alone shows the player his finds first, so he gets it, unless nothing turned up:
// the output buffer is cleared then, which drops it. Blind (can_see() tells him) and a
// word that is no direction keep it; SEARCH {dir} tells the room first, so it is the
// room's.
#[test]
fn search_keeps_the_pending_reset_on_its_own_paths() {
    const RESET: &str = "\x1b[0;37;40m";
    const CLRLINE: &str = "\x1b[79D\x1b[K";
    let mut h = Harness::new();
    let id = h.playing("Aldric", SLUM);
    let other = h.playing("Bob", SLUM);
    let key = RoomKey::new(1, SLUM);
    let prompt = |h: &mut Harness, who: SessionId| {
        String::from_utf8(silvermere_engine::tell::prompt_bytes(h.w(), who)).unwrap()
    };
    // hidden coins are always found
    h.w().room_state.get_mut(&key).unwrap().hidcoins.gold = 5;
    let mark = h.log.len();
    h.line(id, "search");
    assert_eq!(
        h.text_since(mark, id),
        format!("{RESET}\x1b[0;36mYou notice 5 gold crowns here.\r\n")
    );
    let bob = prompt(&mut h, other);
    assert_eq!(
        h.text_since(mark, other),
        format!("{CLRLINE}\x1b[0;36mAldric is searching the area.\r\n{bob}")
    );
    // nothing here now
    h.w().room_state.get_mut(&key).unwrap().hidcoins.gold = 0;
    h.ticks(1);
    let mark = h.log.len();
    h.line(id, "search");
    assert_eq!(
        h.text_since(mark, id),
        "\x1b[0;36mYour search revealed nothing.\r\n"
    );
    assert_eq!(
        h.text_since(mark, other),
        format!("{CLRLINE}\x1b[0;36mAldric is searching the area.\r\n{bob}")
    );
    // nothing on the floor, but Bob is hiding: the buffer is cleared before
    // the hidden players are spotted, so the spotting line comes without the reset
    edit(h.w(), other, |c| c.hidden = 1);
    h.ticks(1);
    let mark = h.log.len();
    h.line(id, "search");
    assert_eq!(
        h.text_since(mark, id),
        "\x1b[1;31mYou see Bob hiding in the shadows.\r\n"
    );
    // not a direction
    h.ticks(1);
    let mark = h.log.len();
    h.line(id, "search shadows");
    assert_eq!(
        h.text_since(mark, id),
        format!("{RESET}Why would you want to search that?\r\n")
    );
    // a direction: Bob is told first and takes the reset
    h.ticks(1);
    let mark = h.log.len();
    h.line(id, "search u");
    assert_eq!(
        h.text_since(mark, id),
        format!("{CLRLINE}You notice nothing different above you.\r\n")
    );
    assert_eq!(
        h.text_since(mark, other),
        format!("{RESET}{CLRLINE}\x1b[0;36mAldric is searching for exits.\r\n{bob}")
    );
    // blind: can_see() says so, then the prompt (`Handled::Done`)
    h.ticks(1);
    edit(h.w(), id, |c| c.status |= PlayerStatus::BLIND);
    let mark = h.log.len();
    h.line(id, "search");
    let me = prompt(&mut h, id);
    assert_eq!(
        h.text_since(mark, id),
        format!("{RESET}You are blind.\r\n{me}")
    );
    assert!(h.text_since(mark, other).is_empty());
}

// cmd_bash; open_door_by_force; the relock after exitpar3 steps
#[test]
fn bash_relocks_after_exitpar3_steps() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", DOOR_ROOM);
    let behind = h.playing("Bob", BEHIND_DOOR);
    edit(h.w(), id, |c| c.str = 200);
    let key = RoomKey::new(1, DOOR_ROOM);
    let back = RoomKey::new(1, BEHIND_DOOR);
    let mark = h.log.len();
    h.line(id, "bash n");
    let ls = h.lines_since(mark, id);
    assert_eq!(ls[0], "You bashed the door open.");
    assert!(pattern("bash_ok").is_match(&ls[0]));
    let ls = h.lines_since(mark, behind);
    assert_eq!(ls, ["The door to the south just flew open."]);
    assert!(pattern("door_moved_remote").is_match(&ls[0]));
    assert_eq!(
        live_exit(h.w(), key, Direction::North).unwrap().par1,
        LOCK_OPEN
    );
    assert_eq!(
        live_exit(h.w(), back, Direction::South).unwrap().par1,
        LOCK_OPEN
    );
    // 2 steps of 300 ticks: still open one tick before
    h.ticks(599);
    assert_eq!(
        live_exit(h.w(), key, Direction::North).unwrap().par1,
        LOCK_OPEN
    );
    let mark = h.log.len();
    h.ticks(1);
    assert_eq!(
        live_exit(h.w(), key, Direction::North).unwrap().par1,
        LOCK_LOCKED
    );
    assert_eq!(
        live_exit(h.w(), back, Direction::South).unwrap().par1,
        LOCK_LOCKED
    );
    let ls = h.lines_since(mark, id);
    assert_eq!(ls, ["The door to the north just locked!"]);
    assert!(pattern("door_moved_remote").is_match(&ls[0]));
    assert_eq!(
        h.lines_since(mark, behind),
        ["The door to the south just locked!"]
    );
    // a locked door refuses with the lower-case wording (exitpar1 2)
    let mark = h.log.len();
    h.line(id, "n");
    h.ticks(1);
    assert_eq!(h.lines_since(mark, id), ["The door is closed!"]);
}

// give_monsters_a_free_attack; handle_following_monsters
#[test]
fn a_monster_strikes_and_a_follower_comes_along() {
    let mut w = World::load(test_config(), 16, snapshot()).unwrap();
    w.rng = SeededRng::new(3);
    let id = player(&mut w, "Aldric", MIDDLE);
    let watcher = player(&mut w, "Bob", MIDDLE);
    let ahead = player(&mut w, "Cid", EAST_END);
    // an evil monster with followpct 100 always takes the roll; T-033's attack is a
    // hook, so the strike shows as the attackers count
    monster(&mut w, 1, MIDDLE, "giant rat", "", 100);
    // a monster following Aldric with followpct 100 always comes along
    monster(&mut w, 2, MIDDLE, "loyal hound", "Aldric", 100);
    assert!(move_user(&mut w, id, 2, 0));
    assert_eq!(w.session(id).unwrap().player().unwrap().attackers, 1);
    let ls = lines(&take(&mut w, watcher));
    assert_eq!(
        ls,
        [
            "Aldric just left to the east.",
            "loyal hound just left to the east."
        ]
    );
    assert!(pattern("monster_leaves_generic").is_match("loyal hound just left to the east."));
    let ls = lines(&take(&mut w, ahead));
    assert_eq!(
        ls,
        [
            "Aldric walks into the room from the west.",
            "loyal hound moves into the room from the west."
        ]
    );
    assert!(
        pattern("monster_arrives_generic")
            .is_match("loyal hound moves into the room from the west.")
    );
    let middle = &w.room_state[&RoomKey::new(1, MIDDLE)];
    assert!(middle.monsters.contains(&ActiveMonsterId(1)));
    assert!(!middle.monsters.contains(&ActiveMonsterId(2)));
    assert!(
        w.room_state[&RoomKey::new(1, EAST_END)]
            .monsters
            .contains(&ActiveMonsterId(2))
    );
    let hound = w
        .active_monsters
        .values()
        .find(|m| m.number.0 == 2)
        .unwrap();
    assert_eq!(hound.room, RoomId(EAST_END));
    assert_eq!(hound.trail[0], RoomId(EAST_END));
    // ST_JUSTARRIVED is set for the round
    assert!(
        w.session(id)
            .unwrap()
            .player()
            .unwrap()
            .status
            .contains(PlayerStatus::JUSTARRIVED)
    );
}

// cmd_drag; the queued drag; the drag's texts
#[test]
fn dragging_a_downed_player() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", MIDDLE);
    let victim = h.playing("Bob", MIDDLE);
    let watcher = h.playing("Cid", MIDDLE);
    let ahead = h.playing("Dora", EAST_END);
    // on his feet: refused, and everyone hears of it
    let mark = h.log.len();
    h.line(id, "drag bob");
    assert_eq!(
        h.lines_since(mark, id),
        ["Bob is too healthy to be dragged."]
    );
    assert_eq!(
        h.lines_since(mark, victim),
        ["Aldric tried to drag you from the room."]
    );
    assert_eq!(
        h.lines_since(mark, watcher),
        ["Aldric attempted to drag Bob out of the room."]
    );
    // down: taken hold of
    edit(h.w(), victim, |c| c.hp = 0);
    let mark = h.log.len();
    h.line(id, "drag bob");
    let ls = h.lines_since(mark, id);
    assert_eq!(ls, ["You are now dragging Bob."]);
    assert!(pattern("dragging").is_match(&ls[0]));
    assert_eq!(h.w().session(id).unwrap().party.dragging, Some(victim));
    // the walk east drags him along, with the suffix on both lines
    let mark = h.log.len();
    h.line(id, "e");
    h.ticks(2);
    assert_eq!(room_of(h.w(), id), EAST_END);
    assert_eq!(room_of(h.w(), victim), EAST_END);
    let ls = h.lines_since(mark, watcher);
    assert_eq!(ls, ["Aldric just left to the east (Dragging Bob)."]);
    assert!(pattern("player_leaves").is_match(&ls[0]));
    assert_eq!(
        h.lines_since(mark, ahead),
        ["Aldric walks into the room from the west (Dragging Bob)."]
    );
    let ls = h.lines_since(mark, id);
    assert_eq!(ls[0], "You are dragging Bob.");
    let ls = h.lines_since(mark, victim);
    assert_eq!(ls[0], "Aldric is dragging you around.");
    assert!(pattern("dragging").is_match(&ls[0]));
    assert_eq!(ls[1], "East End");
    // DRAG alone lets go
    let mark = h.log.len();
    h.line(id, "drag");
    assert_eq!(h.lines_since(mark, id), ["You are no longer dragging Bob."]);
    // DRAG {name} {direction} queues the move at once
    let mark = h.log.len();
    h.line(id, "drag bob w");
    // the ambiguous-match handling drops the dispatcher's ANSI_NORMAL:
    // nothing is sent until the move
    assert_eq!(h.text_since(mark, id), "");
    h.ticks(2);
    assert_eq!(room_of(h.w(), victim), MIDDLE);
    assert_eq!(room_of(h.w(), id), MIDDLE);
    // through an exit that bars dragging: the Toll refusal, and the drag holds
    h.w().rooms.get_mut(&RoomKey::new(1, MIDDLE)).unwrap().exits[3].kind = ExitType::Toll;
    let mark = h.log.len();
    h.line(id, "w");
    h.ticks(2);
    assert_eq!(
        h.lines_since(mark, id)[0],
        "You may not drag anyone through this exit."
    );
    assert_eq!(room_of(h.w(), id), MIDDLE);
}

// cmd_open through the engine, and a closed door's wording after it
#[test]
fn open_then_walk_through_the_door() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", DOOR_ROOM);
    // locked: OPEN refuses
    let mark = h.log.len();
    h.line(id, "open n");
    assert_eq!(h.lines_since(mark, id), ["The door is locked."]);
    // unlock it from the content side and open
    doors_set(h.w(), DOOR_ROOM, 0, 1);
    doors_set(h.w(), BEHIND_DOOR, 1, 1);
    let mark = h.log.len();
    h.line(id, "n");
    h.ticks(1);
    assert_eq!(h.lines_since(mark, id), ["The door is Closed!"]);
    let mark = h.log.len();
    h.line(id, "open n");
    assert_eq!(h.lines_since(mark, id), ["The door is now open."]);
    h.ticks(1);
    let mark = h.log.len();
    h.line(id, "exits");
    assert_eq!(h.lines_since(mark, id), ["Obvious exits: open door north"]);
    let mark = h.log.len();
    h.line(id, "n");
    h.ticks(1);
    assert_eq!(room_of(h.w(), id), BEHIND_DOOR);
    assert_eq!(h.lines_since(mark, id)[0], "Behind the Door");
}

fn doors_set(world: &mut World, room: u32, dir: usize, state: u32) {
    world.rooms.get_mut(&RoomKey::new(1, room)).unwrap().exits[dir].par1 = state;
    if let Some(s) = world.room_state.get_mut(&RoomKey::new(1, room)) {
        s.exit_overrides[dir].par1 = None;
    }
}

// A sneaker who is noticed
#[test]
fn a_failed_sneak_is_noticed_in_both_rooms() {
    let mut w = World::load(test_config(), 16, snapshot()).unwrap();
    w.rng = SeededRng::new(5);
    let id = player(&mut w, "Aldric", MIDDLE);
    let watcher = player(&mut w, "Bob", MIDDLE);
    let ahead = player(&mut w, "Cid", EAST_END);
    edit(&mut w, id, |c| {
        c.status |= PlayerStatus::SNEAKING;
        c.stealth = 0;
        c.perception = 0;
    });
    assert!(silvermere_engine::movement::move_user_with(
        &mut w, id, 2, 1, true, None
    ));
    let ls = lines(&take(&mut w, watcher));
    assert_eq!(ls, ["You notice Aldric sneaking out to the east."]);
    assert!(pattern("sneak_seen_out").is_match("You notice Aldric sneaking out to the east."));
    let ls = lines(&take(&mut w, ahead));
    assert_eq!(ls, ["You notice Aldric sneak in from the west."]);
    assert!(pattern("sneak_seen_in").is_match("You notice Aldric sneak in from the west."));
    assert!(
        !w.session(id)
            .unwrap()
            .player()
            .unwrap()
            .status
            .contains(PlayerStatus::SNEAKING)
    );
    // Perfect Stealth: unseen, nobody told, the room shown
    edit(&mut w, id, |c| {
        c.room = RoomId(MIDDLE);
        c.status |= PlayerStatus::SNEAKING;
        c.abils[0] = (silvermere_types::enums::Ability::PerfectStealth, 1);
    });
    take(&mut w, id);
    assert!(silvermere_engine::movement::move_user_with(
        &mut w, id, 2, 1, true, None
    ));
    assert_eq!(take(&mut w, watcher), b"");
    assert_eq!(take(&mut w, ahead), b"");
    assert_eq!(lines(&take(&mut w, id))[0], "East End");
}

// The transport echoes at receive time. The tick that runs a queued move turns the echo back
// on (`echonu`) and shows the room and prompt; the switch must be on the wire before that
// prompt, or a client that answers at once is fed its first byte with echo still off.
#[test]
fn echo_is_back_on_before_the_prompt_of_a_queued_move() {
    let mut h = Harness::new();
    let id = h.playing("Aldric", NORTH_OF_MIDDLE);
    h.line(id, "s");
    let mark = h.log.len();
    h.ticks(1);
    let on = h.log[mark..].iter().position(|o| {
        matches!(
            o,
            EngineOutput::SetEcho { id: i, mode: silvermere_engine::EchoMode::On } if *i == id
        )
    });
    let prompt = h.log[mark..].iter().position(|o| {
        matches!(o, EngineOutput::Write { id: i, bytes }
            if *i == id && String::from_utf8_lossy(bytes).contains("[HP="))
    });
    let (on, prompt) = (on.expect("echo back on"), prompt.expect("prompt written"));
    assert!(on < prompt, "SetEcho(On) at {on}, prompt at {prompt}");
}
