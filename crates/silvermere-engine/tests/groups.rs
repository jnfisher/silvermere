//! Parties (T-043): INVITE and FOLLOW, the march in the leader's fast cycle, the drop at a
//! level exit, the rank rules, the PARTY screen byte for byte, SHARE, AID, the broadcast
//! channels, UNINVITE, DISBAND PARTY and the leader's logoff; and a property: no list
//! holds its owner and no leader has more than `NGROUP` followers.
//!
//! The rooms: a lane of three on map 1 (`WEST_END` - `MIDDLE` - `EAST_END`, east-west),
//! with `NORTH_OF_MIDDLE` north of the middle, and `GATED` east of the east end behind a
//! Level exit for levels 5 and up.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, test_config, tiny_snapshot};
use proptest::prelude::*;
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, SeededRng, SessionId, World, WorldSnapshot,
};
use silvermere_types::enums::{ExitType, Magery, MessageLevel};
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::{ClassNo, RoomId, RoomKey};
use silvermere_types::records::{Character, Exit};
use silvermere_types::sizes::NGROUP;

const WEST_END: u32 = 6001;
const MIDDLE: u32 = 6002;
const EAST_END: u32 = 6003;
const NORTH_OF_MIDDLE: u32 = 6004;
const GATED: u32 = 6005;

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
    east.exits[2] = exit(GATED, ExitType::Level);
    east.exits[2].par1 = 5;
    let mut north = room(NORTH_OF_MIDDLE, "North Side");
    north.exits[1] = exit(MIDDLE, ExitType::Normal);
    let mut gated = room(GATED, "Gated Hall");
    gated.exits[3] = exit(EAST_END, ExitType::Normal);
    s.rooms.extend([west, middle, east, north, gated]);
    s.classes[0].name = "Warrior".into();
    let mut mage = blank::clear_class(ClassNo(2));
    mage.name = "Mage".into();
    mage.magery = Magery::Mage;
    let mut mystic = blank::clear_class(ClassNo(3));
    mystic.name = "Mystic".into();
    mystic.magery = Magery::Kai;
    s.classes.extend([mage, mystic]);
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
        let mut world = World::load(test_config(), 16, snapshot()).expect("world loads");
        world.rng = SeededRng::new(43);
        let engine = Engine::new(world, clock.clone(), SeededRng::new(7), tx);
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
        let id = enter(&mut self.engine.world, name, room, MessageLevel::High);
        edit(self.w(), id, |c| {
            c.movedir = -1;
            c.str = 50;
            c.level = 1;
            c.perception = 115;
            // a finished character: cleanup_when_user_leaves acts only on one
            c.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
            // where entering the realm would have put him (set_user_location)
            c.trailrooms[0] = RoomId(room);
            c.trailmaps[0] = 1.into();
        });
        id
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

    fn bytes_since(&self, from: usize, id: SessionId) -> Vec<u8> {
        self.log[from..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Write { id: i, bytes } if *i == id => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    fn lines_since(&self, from: usize, id: SessionId) -> Vec<String> {
        lines(&self.bytes_since(from, id))
    }

    /// INVITE by the leader, FOLLOW by each of `followers`, in order.
    fn party(&mut self, leader: SessionId, followers: &[SessionId]) {
        let lname = name(self.w(), leader);
        for &f in followers {
            let fname = name(self.w(), f);
            self.line(leader, &format!("invite {fname}"));
            self.line(f, &format!("follow {lname}"));
        }
    }

    fn mark(&self) -> usize {
        self.log.len()
    }
}

fn edit(world: &mut World, id: SessionId, f: impl FnOnce(&mut Character)) {
    f(world.session_mut(id).unwrap().character.as_mut().unwrap());
}

fn plr(world: &World, id: SessionId) -> &Character {
    world.session(id).unwrap().player().unwrap()
}

fn name(world: &World, id: SessionId) -> String {
    plr(world, id).firstname.clone()
}

fn room_of(world: &World, id: SessionId) -> u32 {
    plr(world, id).room.0
}

fn leader(world: &World, id: SessionId) -> Option<SessionId> {
    world.session(id).unwrap().party.leader
}

fn group(world: &World, id: SessionId) -> [Option<SessionId>; NGROUP] {
    world.session(id).unwrap().party.group
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

/// The screen lines: ANSI removed, the prompts taken out, split at `\r\n`, empty lines
/// dropped.
fn lines(bytes: &[u8]) -> Vec<String> {
    let plain = silvermere_text::strip_ansi(&drop_backspace_pairs(bytes));
    let text = String::from_utf8_lossy(&plain).into_owned();
    let prompt = regex::Regex::new(r"\[HP=-?\d+(?:/(?:MA|KAI)=\d+)?\]:").unwrap();
    prompt
        .replace_all(&text, "")
        .split("\r\n")
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
        .collect()
}

fn prompts(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes).matches("[HP=").count()
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

/// Ticks until `id` has left `from` (at most `max` ticks); how many it took.
fn tick_until_moved(h: &mut Harness, id: SessionId, from: u32, max: u32) -> u32 {
    for n in 1..=max {
        h.ticks(1);
        if room_of(h.w(), id) != from {
            return n;
        }
    }
    panic!("{id:?} never left {from}");
}

// ---------------------------------------------------------------------------------------

// cmd_invite, cmd_follow, add_to_group
#[test]
fn invite_then_follow_links_the_party() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", MIDDLE);
    // not invited yet
    let m = h.mark();
    h.line(b, "follow aldric");
    let ls = h.lines_since(m, b);
    assert_eq!(ls, ["You must be invited first!"]);
    assert!(pattern("invite_needed").is_match(&ls[0]));
    // INVITE: the invitee is told with his prompt, the inviter is told, front rank
    let m = h.mark();
    h.line(a, "invite bob");
    let ls = h.lines_since(m, b);
    assert_eq!(ls, ["Aldric has invited you to follow him."]);
    assert!(pattern("invited_follow").is_match(&ls[0]));
    assert_eq!(prompts(&h.bytes_since(m, b)), 1);
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["You have invited Bob to follow you."]);
    assert!(pattern("you_invited").is_match(&ls[0]));
    assert_eq!(plr(h.w(), a).rank, 1);
    assert_eq!(h.w().session(b).unwrap().ust.inviter, "Aldric");
    assert_eq!(group(h.w(), a)[0], Some(b));
    // a second INVITE of the same player: the inviter's line only (sic)
    let m = h.mark();
    h.line(a, "invite bob");
    assert!(h.lines_since(m, b).is_empty());
    assert_eq!(h.lines_since(m, a), ["You have invited Bob to follow you."]);
    // FOLLOW: no period on the follower's line; the leader is told with a prompt
    let m = h.mark();
    h.line(b, "follow aldric");
    let ls = h.lines_since(m, b);
    assert_eq!(ls, ["You are now following Aldric"]);
    assert!(pattern("following").is_match(&ls[0]));
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["Bob started to follow you."]);
    assert!(pattern("follower_joined").is_match(&ls[0]));
    assert_eq!(leader(h.w(), b), Some(a));
    assert_eq!(group(h.w(), b)[0], Some(a));
    assert_eq!(plr(h.w(), b).rank, 0, "a follower joins at midrank");
    assert!(h.w().session(b).unwrap().ust.inviter.is_empty());
    // a second member: every member lists every other
    h.party(a, &[c]);
    assert_eq!(group(h.w(), a)[..2], [Some(b), Some(c)]);
    assert_eq!(group(h.w(), b)[..2], [Some(a), Some(c)]);
    assert_eq!(group(h.w(), c)[..2], [Some(a), Some(b)]);
    // the syntax lines, and yourself
    let m = h.mark();
    h.line(a, "invite");
    h.line(a, "invite zed");
    h.line(c, "follow zed");
    assert_eq!(
        h.lines_since(m, a),
        ["Syntax: INVITE {user name}", "You don't see zed here!"]
    );
    assert_eq!(h.lines_since(m, c), ["You don't see that anywhere!"]);
}

// is_user_forgotten. Resolving the name clears the output buffer (clear_output): FOLLOW, INVITE and
// UNINVITE lose the dispatcher's ANSI_NORMAL in front of their own line. Seen on the
// original by the m3/party differential (FOLLOW and UNINVITE differed in colour only).
#[test]
fn follow_invite_and_uninvite_start_without_the_dispatchers_reset() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    for (who, line, want) in [
        (
            a,
            "invite bob",
            &b"\x1b[79D\x1b[K\x1b[1m\x1b[34mYou have invited Bob to follow you.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:"[..],
        ),
        (
            b,
            "follow aldric",
            &b"\x1b[79D\x1b[K\x1b[0;37mYou are now following Aldric\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:"[..],
        ),
        (
            a,
            "uninvite bob",
            &b"\x1b[79D\x1b[K\x1b[0;31mBob has been removed from your followers.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:"[..],
        ),
    ] {
        let m = h.mark();
        h.line(who, line);
        let bytes = h.bytes_since(m, who);
        assert_eq!(bytes, want, "{line}: {:?}", String::from_utf8_lossy(&bytes));
    }
}

#[test]
fn an_invitation_from_a_forgotten_player_is_refused() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    h.line(b, "forget aldric");
    let m = h.mark();
    h.line(a, "invite bob");
    assert_eq!(h.lines_since(m, a), ["You cannot invite bob."]);
    assert!(h.lines_since(m, b).is_empty());
}

// Three moves, each followed by the
// follower in the same fast tick, a bystander seeing the leader leave first.
#[test]
fn the_follower_marches_in_the_leaders_fast_tick() {
    for follower_first in [false, true] {
        let mut h = Harness::new();
        // session order decides who moves where in the cycle: a follower on a lower id
        // follows from the leader's queued move, on a higher id in his own update
        let (a, b) = if follower_first {
            let b = h.playing("Bob", WEST_END);
            (h.playing("Aldric", WEST_END), b)
        } else {
            let a = h.playing("Aldric", WEST_END);
            (a, h.playing("Bob", WEST_END))
        };
        let watcher = h.playing("Cid", WEST_END);
        h.party(a, &[b]);
        let mut from = WEST_END;
        for (dir, word, back, to) in [
            ("e", "east", "west", MIDDLE),
            ("n", "north", "south", NORTH_OF_MIDDLE),
            ("s", "south", "north", MIDDLE),
        ] {
            if from == WEST_END || to == NORTH_OF_MIDDLE {
                // the watcher waits where the party leaves from
                edit(h.w(), watcher, |c| c.room = RoomId(from));
            }
            let m = h.mark();
            h.line(a, dir);
            let n = tick_until_moved(&mut h, a, from, 5);
            assert_eq!(room_of(h.w(), b), to, "same tick ({n}) for {word}");
            // he stood in the room his leader left: that line first, then his own move
            let ls = h.lines_since(m, b);
            assert_eq!(ls[0], format!("Aldric just left to the {word}."));
            assert_eq!(ls[1], format!(" -- Following your Party leader {word} --"));
            let ls = h.lines_since(m, watcher);
            if room_of(h.w(), watcher) == from {
                assert_eq!(
                    ls,
                    [
                        format!("Aldric just left to the {word}."),
                        format!("Bob just left to the {word}.")
                    ]
                );
            }
            // the leader's room first, then his follower walks in after him
            let ls = h.lines_since(m, a);
            assert_eq!(
                ls.last().unwrap(),
                &format!("Bob walks into the room from the {back}.")
            );
            assert!(ls.iter().position(|l| l.starts_with("Obvious exits")) < Some(ls.len() - 1));
            from = to;
        }
        assert_eq!(leader(h.w(), b), Some(a));
    }
}

// the Level exit refuses the follower; he leaves the party line with `leader = -1` only. The
// leader is told nothing, and (the original, not the card) neither is the follower beyond the
// exit's own refusal: `stop_following` is not called on a refused exit. He stays in the
// leader's list, so FOLLOW takes him back without a new invitation.
#[test]
fn a_follower_drops_at_a_level_exit() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", EAST_END);
    let b = h.playing("Bob", EAST_END);
    edit(h.w(), a, |c| c.level = 5);
    h.party(a, &[b]);
    let m = h.mark();
    h.line(a, "e");
    tick_until_moved(&mut h, a, EAST_END, 5);
    h.ticks(1);
    assert_eq!(room_of(h.w(), a), GATED);
    assert_eq!(room_of(h.w(), b), EAST_END);
    assert_eq!(leader(h.w(), b), None);
    let ls = h.lines_since(m, b);
    assert!(
        ls.iter().all(|l| !l.contains("no longer following")),
        "{ls:?}"
    );
    assert!(
        ls.contains(&"You have not progressed far enough to go through this exit!".to_owned()),
        "{ls:?}"
    );
    assert!(
        h.lines_since(m, a).iter().all(|l| !l.contains("Bob")),
        "{:?}",
        h.lines_since(m, a)
    );
    // still listed: back together, FOLLOW works without INVITE
    assert_eq!(group(h.w(), a)[0], Some(b));
    h.line(a, "w");
    tick_until_moved(&mut h, a, GATED, 5);
    let m = h.mark();
    h.line(b, "follow aldric");
    assert_eq!(h.lines_since(m, b), ["You are now following Aldric"]);
}

// a held follower is dropped and told; the leader not
#[test]
fn a_held_follower_is_told_he_no_longer_follows() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    h.party(a, &[b]);
    edit(h.w(), b, |c| {
        c.status2 |= silvermere_types::flags::PlayerStatus2::HELD;
    });
    let m = h.mark();
    h.line(a, "e");
    tick_until_moved(&mut h, a, MIDDLE, 5);
    h.ticks(1);
    let ls = h.lines_since(m, b);
    assert!(
        ls.contains(&"You are no longer following Aldric.".to_owned()),
        "{ls:?}"
    );
    assert!(ls.iter().any(|l| pattern("unfollowed").is_match(l)));
    assert!(h.lines_since(m, a).iter().all(|l| !l.contains("Bob")));
    assert_eq!(leader(h.w(), b), None);
    assert_eq!(group(h.w(), a), [None; NGROUP]);
}

/// Logs `id` in the way `enter_realm` does (`load_player`) with the record's
/// `dragging`, keeping the test's own stat edits.
fn login_with_dragging(h: &mut Harness, id: SessionId, dragging: i16) {
    let before = plr(h.w(), id).clone();
    let mut row = before.clone();
    row.dragging = dragging;
    row.version = silvermere_engine::player::PLAYER_VERSION;
    silvermere_engine::player::load_player(h.w(), id, row, 0).expect("loads");
    // the test's own setup stays as it was; what the login did to the session's party stays
    edit(h.w(), id, |c| {
        *c = before;
        c.dragging = dragging;
    });
}

fn first_march(h: &mut Harness, leader: SessionId, follower: SessionId, to: u32) -> Vec<u8> {
    let m = h.mark();
    h.line(leader, "e");
    tick_until_moved(h, leader, MIDDLE, 5);
    h.ticks(1);
    assert_eq!(room_of(h.w(), follower), to);
    h.bytes_since(m, follower)
}

const DRAG_REFUSAL: &str = "You may not drag someone while in a party!";

// clear_player zeroes the record, and neither load_player nor create_player sets `dragging`
// back to -1: a new character drags "channel 0", so his first marched step is refused (seen
// on the original in m3/party, step 16). `dragging` is saved with the record, so the -1 that
// refusal leaves is what the next session starts with.
#[test]
fn the_drag_refusal_comes_once_in_a_characters_life() {
    use silvermere_engine::party::Party;
    assert_eq!(Party::from_record(0).dragging, Some(SessionId(0)));
    assert_eq!(Party::from_record(-1).dragging, None);
    assert_eq!(Party::from_record(0).record_dragging(), 0);
    assert_eq!(Party::default().record_dragging(), -1);

    // first session of a new character (record dragging 0)
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    login_with_dragging(&mut h, b, 0);
    h.party(a, &[b]);
    assert_eq!(h.w().session(b).unwrap().party.dragging, Some(SessionId(0)));
    let bytes = first_march(&mut h, a, b, EAST_END);
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        "\x1b[79D\x1b[K\x1b[1;31mAldric\x1b[0;32m just left to  \x08the ea \x08st.\r\n\
         \x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:\
         \x1b[79D\x1b[K\x1b[1;37mYou may not drag someone while in a party!\r\n\
         \x1b[79D\x1b[K\x1b[0;37m -- Following your Party leader e \x08ast --\r\n\
         \x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;36mEast End\r\n\
         \x1b[0;35mAlso here: \x1b[1;35mAldric\x1b[0;35m.\r\n\
         \x1b[0m\x1b[0;32mObvious exits: e \x08ast, w \x08est\r\n\
         \x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:"
    );
    assert_eq!(h.w().session(b).unwrap().party.dragging, None);

    // he leaves the realm with X: the record saved on the way out says -1 (the record
    // is saved before the state changes), and so does the character the session keeps
    {
        let s = h.w().session_mut(b).unwrap();
        s.account = Some(silvermere_engine::AccountRef {
            id: silvermere_types::ids::AccountId(7),
            username: "BOB".into(),
            sysop: false,
            gameop: false,
            sex: b'M',
        });
        s.created = true;
        s.character.as_mut().unwrap().settings |=
            silvermere_types::flags::PlayerSettings::VALIDATED;
    }
    let m = h.mark();
    h.line(b, "x");
    for _ in 0..60 {
        if h.w().session(b).unwrap().state == silvermere_types::enums::SessionState::MainMenu {
            break;
        }
        h.ticks(1);
    }
    assert_eq!(
        h.w().session(b).unwrap().state,
        silvermere_types::enums::SessionState::MainMenu
    );
    let saved = h.log[m..]
        .iter()
        .filter_map(|o| match o {
            EngineOutput::Persist(silvermere_engine::PersistRequest::Character {
                character,
                ..
            }) => Some(character.dragging),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        !saved.is_empty() && saved.iter().all(|&d| d == -1),
        "{saved:?}"
    );
    let saved = saved[0];
    assert_eq!(h.w().session(b).unwrap().player().unwrap().dragging, -1);
    // E in the same connection: the session starts from that record, nobody dragged
    h.line(b, "E");
    assert_eq!(h.w().session(b).unwrap().party.dragging, None);

    // his second session starts from that record: no refusal on the first march
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    login_with_dragging(&mut h, b, saved);
    h.party(a, &[b]);
    let bytes = first_march(&mut h, a, b, EAST_END);
    assert!(!String::from_utf8_lossy(&bytes).contains(DRAG_REFUSAL));
    assert!(String::from_utf8_lossy(&bytes).contains("Following your Party leader"));
}

// a deleted character is `clear_player`ed, so the
// character made next on the same channel starts at 0 again and gets its own first refusal
#[test]
fn a_new_character_after_a_deletion_gets_the_refusal_again() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    login_with_dragging(&mut h, b, -1); // an old hand: nothing to refuse
    {
        let s = h.w().session_mut(b).unwrap();
        s.account = Some(silvermere_engine::AccountRef {
            id: silvermere_types::ids::AccountId(7),
            username: "BOB".into(),
            sysop: false,
            gameop: false,
            sex: b'M',
        });
    }
    let mut fresh = plr(h.w(), b).clone();
    fresh.dragging = 0; // what create_player leaves (clear_player)
    fresh.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
    silvermere_engine::player::delete_player(h.w(), b);
    {
        let s = h.w().session_mut(b).unwrap();
        s.character = Some(fresh);
        s.created = true;
    }
    let m = h.mark();
    assert!(silvermere_engine::player::save_player(h.w(), b));
    h.ticks(1);
    let saved: Vec<i16> = h.log[m..]
        .iter()
        .filter_map(|o| match o {
            EngineOutput::Persist(silvermere_engine::PersistRequest::Character {
                character,
                ..
            }) => Some(character.dragging),
            _ => None,
        })
        .collect();
    assert_eq!(saved, [0]);
    login_with_dragging(&mut h, b, saved[0]);
    h.party(a, &[b]);
    let bytes = first_march(&mut h, a, b, EAST_END);
    assert!(String::from_utf8_lossy(&bytes).contains(DRAG_REFUSAL));
}

// a following drag is let go
#[test]
fn a_follower_may_not_drag() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", MIDDLE);
    h.party(a, &[b]);
    edit(h.w(), c, |p| p.hp = 0);
    h.line(b, "drag cid");
    assert_eq!(h.w().session(b).unwrap().party.dragging, Some(c));
    let m = h.mark();
    h.line(a, "e");
    tick_until_moved(&mut h, a, MIDDLE, 5);
    h.ticks(1);
    let ls = h.lines_since(m, b);
    assert!(
        ls.contains(&"You may not drag someone while in a party!".to_owned()),
        "{ls:?}"
    );
    assert_eq!(room_of(h.w(), b), EAST_END);
    assert_eq!(room_of(h.w(), c), MIDDLE);
    assert_eq!(h.w().session(b).unwrap().party.dragging, None);
}

// FRONTRANK and BACKRANK, in their refusal order, with the lines of the
// allowed moves. The back-outnumbers-front refusal needs four: the others are counted,
// not the mover (front 1 < back 2).
#[test]
fn rank_rules() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", MIDDLE);
    let d = h.playing("Dora", MIDDLE);
    // alone
    let m = h.mark();
    h.line(a, "backrank");
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["You are not in a party at the present time."]);
    assert!(pattern("party_none").is_match(&ls[0]));
    h.party(a, &[b, c, d]);
    let refused = |h: &mut Harness, who: SessionId, cmd: &str, want: &str| {
        let m = h.mark();
        h.line(who, cmd);
        let ls = h.lines_since(m, who);
        assert_eq!(ls, [want], "{cmd}");
        assert!(pattern("rank_refused").is_match(&ls[0]));
        for o in [a, b, c, d] {
            if o != who {
                assert!(h.lines_since(m, o).is_empty(), "{cmd}");
            }
        }
    };
    // the leader to the back
    refused(
        &mut h,
        a,
        "backrank",
        "You may not enter the backrank of your own party.",
    );
    // the last of the front to the middle
    refused(
        &mut h,
        a,
        "midrank",
        "You are the only one in the front rank.  You may not leave.",
    );
    // allowed: Bob to the back; every member of the same party is told
    let m = h.mark();
    h.line(b, "backrank");
    // a send to one player only rewinds the write position, so the output buffer still holds
    // the line, the
    // dispatcher's ANSI_NORMAL first, when the next member is told: each gets the same
    // bytes, and Bob's own line is written over them
    assert_eq!(
        h.bytes_since(m, b),
        b"You have moved to the back ranks of your group.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:"
    );
    for o in [a, c, d] {
        assert_eq!(
            h.bytes_since(m, o),
            b"\x1b[0;37;40m\x1b[79D\x1b[KBob just moved to the back rank in your group.\r\n",
            "{}",
            name(h.w(), o)
        );
    }
    let ls = h.lines_since(m, b);
    assert_eq!(ls, ["You have moved to the back ranks of your group."]);
    assert!(pattern("rank_moved").is_match(&ls[0]));
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["Bob just moved to the back rank in your group."]);
    assert!(pattern("rank_moved").is_match(&ls[0]));
    h.line(c, "backrank");
    assert_eq!(plr(h.w(), c).rank, 2);
    // Dora to the back: front 1 (Aldric) < back 2
    refused(
        &mut h,
        d,
        "backrank",
        "There would not be enough people left in the frontrank if you did that.",
    );
    // the leader keeps his place in the middle too: Dora to the front first
    let m = h.mark();
    h.line(d, "frontrank");
    assert_eq!(
        h.lines_since(m, d),
        ["You have moved to the front ranks of your group."]
    );
    assert_eq!(
        h.lines_since(m, a),
        ["Dora just moved to the front rank in your group."]
    );
    refused(
        &mut h,
        a,
        "midrank",
        "You may not leave your current rank in your own party.",
    );
    // Bob back to the middle, Cid the last of the back
    h.line(b, "midrank");
    assert_eq!(plr(h.w(), b).rank, 0);
    refused(
        &mut h,
        c,
        "midrank",
        "You are the only one in the back rank.  You may not leave.",
    );
    // a rank change ends resting
    edit(h.w(), b, |p| p.status |= PlayerStatus::RESTING);
    let m = h.mark();
    h.line(b, "frontrank");
    assert!(!plr(h.w(), b).status.contains(PlayerStatus::RESTING));
    assert_eq!(
        h.lines_since(m, b),
        ["You have moved to the front ranks of your group."]
    );
}

// a leader with only an invitation tells nobody, so the line meant
// for the members comes to him before his own (nothing was sent to a member)
#[test]
fn a_rank_change_with_only_invitations_shows_both_lines() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    h.line(a, "invite bob");
    let m = h.mark();
    h.line(a, "frontrank");
    assert_eq!(
        h.lines_since(m, a),
        [
            "Aldric just moved to the front rank in your group.",
            "You have moved to the front ranks of your group."
        ]
    );
    assert!(h.lines_since(m, b).is_empty());
}

fn esc(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

/// The bytes of one engine reply without the trailing prompt.
fn reply_body(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let cut = text
        .rfind("\x1b[79D\x1b[K\x1b[0;37m[HP=")
        .unwrap_or(text.len());
    bytes[..cut].to_vec()
}

// the leader's PARTY with one follower, byte for byte, then the
// follower's
#[test]
fn party_screen_bytes() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", MIDDLE);
    edit(h.w(), a, |p| {
        p.lastname = "Stonebrook".into();
        p.hp = 8;
    });
    edit(h.w(), b, |p| {
        p.lastname = "Smith".into();
        p.class = ClassNo(2);
        p.mana = 5;
        p.maxmana = 10;
        p.poison = 3;
    });
    edit(h.w(), c, |p| {
        p.lastname = "Of The Very Long Family Name Indeed".into();
        p.class = ClassNo(3);
        p.mana = 10;
        p.maxmana = 10;
    });
    h.party(a, &[b]);
    h.line(a, "invite cid");
    // FOLLOW ended any rest: Bob rests now
    edit(h.w(), b, |p| p.status |= PlayerStatus::RESTING);
    let m = h.mark();
    h.line(a, "par");
    let got = reply_body(&h.bytes_since(m, a));
    let want = concat!(
        "\x1b[0;37;40m",
        "\x1b[79D\x1b[K",
        "\x1b[0;37mThe following people are in your travel party:\r\n",
        "  Bob Smith                      (Mage)       [M: 50%] [H:100%]PR - Midrank\r\n",
        "  Cid Of The Very Long Family Na (Mystic)     [Invited]\r\n",
        "  Aldric Stonebrook              (Warrior)             [H: 80%]   - Frontrank\r\n",
    );
    assert_eq!(esc(&got), esc(want.as_bytes()));
    for l in lines(&got) {
        let header = pattern("party_header").is_match(&l);
        assert!(
            header || l.starts_with("  "),
            "{l:?} is the header or a row"
        );
    }
    // the follower: his leader, then the members of his list (Aldric, and Cid invited)
    let m = h.mark();
    h.line(b, "par");
    let got = reply_body(&h.bytes_since(m, b));
    let want = concat!(
        "\x1b[0;37;40m",
        "\x1b[79D\x1b[K",
        "\x1b[1m\x1b[36mYou are following Aldric.\r\n",
        "\x1b[0;37mThe following people are in your travel party:\r\n",
        "  Aldric Stonebrook              (Warrior)             [H: 80%]   - Frontrank\r\n",
        "  Bob Smith                      (Mage)       [M: 50%] [H:100%]PR - Midrank\r\n",
    );
    assert_eq!(esc(&got), esc(want.as_bytes()));
    // the kai letter and a short hit-point figure
    h.line(c, "follow aldric");
    edit(h.w(), c, |p| {
        p.mana = 5;
        p.hp = -1;
    });
    let m = h.mark();
    h.line(a, "par");
    let ls = h.lines_since(m, a);
    assert_eq!(ls.len(), 4, "{ls:?}");
    let ls = &ls[..3];
    assert_eq!(
        ls.last().unwrap(),
        "  Cid Of The Very Long Family Na (Mystic)     [K: 50%] [H:-10%]   - Midrank"
    );
}

// nobody listed; the capture `status-screens` (raw.bin, `par`)
#[test]
fn party_screen_alone_matches_the_capture() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    edit(h.w(), a, |p| {
        p.lastname = "Stonebrook".into();
        p.hp = 35;
        p.maxhp = 35;
        p.rank = 1;
    });
    let m = h.mark();
    h.line(a, "par");
    let got = reply_body(&h.bytes_since(m, a));
    let want = concat!(
        "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;31mYou are not in a party at the present time.\r\n",
        "  Aldric Stonebrook              (Warrior)             [H:100%]   - Midrank\r\n",
    );
    assert_eq!(esc(&got), esc(want.as_bytes()));
    assert_eq!(plr(h.w(), a).rank, 0);
}

// cmd_share, share_currency
#[test]
fn share_splits_and_keeps_the_remainder() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", EAST_END);
    let watcher = h.playing("Dora", MIDDLE);
    edit(h.w(), a, |p| p.coins.gold = 10);
    // alone: everything comes back
    h.line(a, "share 10 gold");
    assert_eq!(plr(h.w(), a).coins.gold, 10);
    h.party(a, &[b]);
    let m = h.mark();
    h.line(a, "share 10 gold");
    assert_eq!(plr(h.w(), a).coins.gold, 5);
    assert_eq!(plr(h.w(), b).coins.gold, 5);
    assert_eq!(h.lines_since(m, a), ["You gave Bob 5 gold crowns"]);
    assert_eq!(h.lines_since(m, b), ["Aldric gave you 5 gold crowns"]);
    assert_eq!(
        h.lines_since(m, watcher),
        ["Aldric just gave Bob some coins."]
    );
    // three ways: 10 / 3 = 3 each, the sharer keeps 3 + 1; party members anywhere count
    edit(h.w(), c, |p| p.room = RoomId(MIDDLE));
    h.party(a, &[c]);
    edit(h.w(), c, |p| p.room = RoomId(EAST_END));
    edit(h.w(), a, |p| p.coins.gold = 10);
    edit(h.w(), b, |p| p.coins.gold = 0);
    h.line(a, "share 10 gold with party");
    assert_eq!(plr(h.w(), a).coins.gold, 4);
    assert_eq!(plr(h.w(), b).coins.gold, 3);
    assert_eq!(plr(h.w(), c).coins.gold, 3);
    // the room instead: Bob and Dora are here, Cid is not
    edit(h.w(), a, |p| p.coins.gold = 7);
    h.line(a, "share 9 gold with room");
    assert_eq!(plr(h.w(), a).coins.gold, 3);
    assert_eq!(plr(h.w(), b).coins.gold, 5);
    assert_eq!(plr(h.w(), watcher).coins.gold, 2);
    assert_eq!(plr(h.w(), c).coins.gold, 3);
    // no-receive players are left out
    edit(h.w(), b, |p| {
        p.settings |= silvermere_types::flags::PlayerSettings::NORECEIVE;
    });
    edit(h.w(), a, |p| p.coins.gold = 10);
    h.line(a, "share 10 gold");
    assert_eq!(plr(h.w(), a).coins.gold, 5);
    assert_eq!(plr(h.w(), c).coins.gold, 8);
    // the refusals
    edit(h.w(), a, |p| p.coins.copper = 0);
    let m = h.mark();
    h.line(a, "share 5 copper");
    h.line(a, "share 5");
    h.line(a, "share 0 gold");
    h.line(a, "share 5 wood");
    assert_eq!(
        h.lines_since(m, a),
        [
            "You do not have enough to share!",
            "Syntax: SHARE {amount} {currency} [WITH ROOM/PARTY] - default: PARTY",
            "Syntax: SHARE {amount} {currency} [WITH ROOM/PARTY]",
            "Syntax: SHARE {amount} {currency} [WITH ROOM/PARTY]",
        ]
    );
}

// cmd_aid
#[test]
fn aid_a_downed_player() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let m = h.mark();
    h.line(a, "aid bob");
    assert_eq!(h.lines_since(m, a), ["Bob is in no need of assistance."]);
    edit(h.w(), b, |p| p.hp = -3);
    let m = h.mark();
    h.line(a, "aid bob");
    assert_eq!(
        h.lines_since(m, a),
        ["You have aided Bob, his wounds are now healing."]
    );
    assert_eq!(h.lines_since(m, b), ["Aldric has aided you."]);
    assert_eq!(prompts(&h.bytes_since(m, b)), 1);
    assert_eq!(plr(h.w(), b).aided, 1);
    let m = h.mark();
    h.line(a, "aid");
    h.line(a, "aid zed");
    assert_eq!(
        h.lines_since(m, a),
        ["Syntax: AID {user name}", "Syntax: AID {user name}"]
    );
}

// cmd_join, cmd_leave, tell_group
#[test]
fn broadcast_channels() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", EAST_END);
    h.line(b, "join 3");
    let m = h.mark();
    h.line(a, "join 3");
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["You just joined channel 3."]);
    assert!(pattern("channel").is_match(&ls[0]));
    let ls = h.lines_since(m, b);
    assert_eq!(ls, ["Aldric just joined your channel (3)"]);
    assert!(pattern("channel").is_match(&ls[0]));
    assert_eq!(prompts(&h.bytes_since(m, b)), 1);
    assert_eq!(plr(h.w(), a).bchannel, 3);
    // to another channel: the old one is told
    let m = h.mark();
    h.line(a, "join 4");
    assert_eq!(h.lines_since(m, b), ["Aldric just left your channel (3)"]);
    h.line(a, "join 3");
    // LEAVE without a leader leaves the channel
    let m = h.mark();
    h.line(a, "leave");
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["You just left group 3."]);
    assert!(pattern("group_left").is_match(&ls[0]));
    let ls = h.lines_since(m, b);
    assert_eq!(ls, ["Aldric just left your group (3)"]);
    assert!(pattern("group_left").is_match(&ls[0]));
    assert_eq!(plr(h.w(), a).bchannel, 0);
    // nothing to leave: not a command (the word is said)
    let m = h.mark();
    h.line(a, "leave");
    assert!(!h.lines_since(m, a).iter().any(|l| l.contains("left group")),);
    // JOIN alone
    let m = h.mark();
    h.line(a, "join");
    assert_eq!(
        h.lines_since(m, a),
        ["Syntax: JOIN {group number} or {user name} or GANG {gangname}"]
    );
}

// JOIN {name} is FOLLOW; LEAVE and FOLLOW alone stop following
#[test]
fn join_by_name_and_leave() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    h.line(a, "invite bob");
    let m = h.mark();
    h.line(b, "join aldric");
    assert_eq!(h.lines_since(m, b), ["You are now following Aldric"]);
    let m = h.mark();
    h.line(b, "leave");
    assert_eq!(h.lines_since(m, b), ["You are no longer following Aldric."]);
    assert!(h.lines_since(m, a).is_empty());
    assert_eq!(leader(h.w(), b), None);
    assert_eq!(group(h.w(), a), [None; NGROUP]);
    assert_eq!(plr(h.w(), a).rank, 0, "alone again: the middle");
    // FOLLOW alone without a leader
    let m = h.mark();
    h.line(b, "follow");
    assert_eq!(h.lines_since(m, b), ["Syntax: FOLLOW {user/monster}"]);
}

// cmd_uninvite
#[test]
fn uninvite_removes_a_follower() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    h.party(a, &[b]);
    let m = h.mark();
    h.line(a, "uninvite bob");
    let ls = h.lines_since(m, a);
    assert_eq!(ls, ["Bob has been removed from your followers."]);
    assert!(pattern("follower_removed").is_match(&ls[0]));
    assert_eq!(h.lines_since(m, b), ["You are no longer following Aldric."]);
    assert_eq!(leader(h.w(), b), None);
    assert_eq!(group(h.w(), a), [None; NGROUP]);
    assert_eq!(plr(h.w(), a).rank, 0);
    let m = h.mark();
    h.line(a, "uninvite");
    h.line(a, "uninvite aldric");
    assert_eq!(
        h.lines_since(m, a),
        [
            "Syntax: UNINVITE {user name}",
            "Why would you uninvite yourself?"
        ]
    );
}

// DISBAND PARTY; the leader leaving
#[test]
fn the_party_ends_with_disband_and_with_the_leaders_logoff() {
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", MIDDLE);
    h.party(a, &[b, c]);
    let m = h.mark();
    h.line(a, "disband party");
    for f in [b, c] {
        assert_eq!(h.lines_since(m, f), ["You are no longer following Aldric."]);
        assert_eq!(leader(h.w(), f), None);
        assert_eq!(group(h.w(), f), [None; NGROUP]);
    }
    assert!(h.lines_since(m, a).is_empty());
    assert_eq!(group(h.w(), a), [None; NGROUP]);
    // the leader logs off
    h.party(a, &[b, c]);
    let m = h.mark();
    silvermere_engine::session::on_logoff(h.w(), a);
    h.engine.flush();
    h.pump();
    for f in [b, c] {
        assert_eq!(h.lines_since(m, f), ["You are no longer following Aldric."]);
        assert_eq!(leader(h.w(), f), None);
        assert_eq!(group(h.w(), f), [None; NGROUP]);
    }
}

// DISBAND PARTY, stop_following for every follower: the first follower told takes the
// dispatcher's ANSI_NORMAL and clear_output drops it for the rest; with no followers nothing is
// told, so it stays in the output buffer and goes out with the leader's prompt.
#[test]
fn disband_party_leaves_the_reset_for_the_next_send() {
    const RESET: &[u8] = b"\x1b[0;37;40m";
    let prompt = |h: &mut Harness, id| silvermere_engine::tell::prompt_bytes(h.w(), id);
    let line = |h: &mut Harness, id| {
        let mut b = b"\x1b[79D\x1b[K\x1b[1;34mYou are no longer following Aldric.\r\n".to_vec();
        b.extend(prompt(h, id));
        b
    };
    let mut h = Harness::new();
    let a = h.playing("Aldric", MIDDLE);
    let b = h.playing("Bob", MIDDLE);
    let c = h.playing("Cid", MIDDLE);
    // nobody follows
    let m = h.mark();
    h.line(a, "disband party");
    assert_eq!(h.bytes_since(m, a), [RESET, &prompt(&mut h, a)].concat());
    // two follow: Bob, the first told, gets it
    h.party(a, &[b, c]);
    let m = h.mark();
    h.line(a, "disband party");
    assert_eq!(h.bytes_since(m, a), prompt(&mut h, a));
    assert_eq!(h.bytes_since(m, b), [RESET, &line(&mut h, b)].concat());
    assert_eq!(h.bytes_since(m, c), line(&mut h, c));
}

// The persistence check: the character row carries no party column (groups and following
// are session-only). `dragging` is not: the record
// saves it.
#[test]
fn the_character_row_has_no_party_columns() {
    for schema in [
        include_str!("../../../migrations/postgres/0001_schema.sql"),
        include_str!("../../../migrations/sqlite/0001_schema.sql"),
    ] {
        let start = schema.find("CREATE TABLE characters (").unwrap();
        let body = &schema[start..start + schema[start..].find(");").unwrap()];
        for line in body.lines().skip(1) {
            let column = line.split_whitespace().next().unwrap_or("");
            assert!(!matches!(column, "leader" | "group" | "grp"), "{line}");
        }
    }
}

#[derive(Debug, Clone)]
enum Act {
    Invite(usize, usize),
    Follow(usize, usize),
    Uninvite(usize, usize),
    Leave(usize),
    Disband(usize),
}

fn act() -> impl Strategy<Value = Act> {
    let p = 0..8usize;
    prop_oneof![
        (p.clone(), p.clone()).prop_map(|(a, b)| Act::Invite(a, b)),
        (p.clone(), p.clone()).prop_map(|(a, b)| Act::Follow(a, b)),
        (p.clone(), p.clone()).prop_map(|(a, b)| Act::Uninvite(a, b)),
        p.clone().prop_map(Act::Leave),
        p.prop_map(Act::Disband),
    ]
}

const NAMES: [&str; 8] = ["Ann", "Ben", "Cat", "Dan", "Eve", "Fay", "Gus", "Hal"];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    // card §6: the follower set never exceeds NGROUP and never holds the leader
    #[test]
    fn lists_stay_within_bounds(acts in proptest::collection::vec(act(), 1..40)) {
        let mut h = Harness::new();
        let ids: Vec<SessionId> = NAMES.iter().map(|n| h.playing(n, MIDDLE)).collect();
        for a in acts {
            match a {
                Act::Invite(x, y) => h.line(ids[x], &format!("invite {}", NAMES[y])),
                Act::Follow(x, y) => h.line(ids[x], &format!("follow {}", NAMES[y])),
                Act::Uninvite(x, y) => h.line(ids[x], &format!("uninvite {}", NAMES[y])),
                Act::Leave(x) => h.line(ids[x], "leave"),
                Act::Disband(x) => h.line(ids[x], "disband party"),
            }
            let w = &h.engine.world;
            for &id in &ids {
                let s = w.session(id).unwrap();
                prop_assert!(!s.party.group.contains(&Some(id)));
                prop_assert_ne!(s.party.leader, Some(id));
                let followers = ids
                    .iter()
                    .filter(|&&f| w.session(f).unwrap().party.leader == Some(id))
                    .count();
                prop_assert!(followers <= NGROUP);
                // (a follower need not be in his leader's list: DISBAND PARTY by a
                // follower, or FOLLOW by a leader, takes him off the lists and leaves
                // `leader` as it was)
            }
        }
    }
}
