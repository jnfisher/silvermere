//! The login flow end to end on the engine (T-021): the account is bound
//! (`EngineInput::LoggedIn`, what `bin` sends once `silvermere-auth`'s dialogue is done),
//! the BBS menu shim, the `[MAJORMUD]:` menu, entering and leaving the realm, the
//! hang-up and the inactivity hang-up. The bytes are compared with the vendored
//! `tests/captures/entry-existing` capture of the original DLL under MBBSEmu.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, special_rooms, test_config, tiny_snapshot};
use silvermere_engine::combat::AutoTarget;
use silvermere_engine::state::login::{Login, leave};
use silvermere_engine::{
    AccountRef, Caps, ConnId, Engine, EngineInput, EngineOutput, PersistRequest, SeededRng,
    SessionId, World, clear_player,
};
use silvermere_rules::combat::Move;
use silvermere_types::config::{GameConfig, PunHup};
use silvermere_types::enums::{Ability, MessageLevel, SessionState, StatlineMode};
use silvermere_types::flags::{PlayerFlags, PlayerStatus};
use silvermere_types::ids::ActiveMonsterId;
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, MapId, RaceNo, RoomId, RoomKey};
use silvermere_types::records::Character;

const CAPTURE: &[u8] = include_bytes!("../../../tests/captures/entry-existing/raw.bin");

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    /// Every output so far, in order.
    log: Vec<EngineOutput>,
    next_conn: u64,
}

impl Harness {
    fn new(config: GameConfig, max_sessions: u16, edit: impl FnOnce(&mut World)) -> Self {
        crate::common::install_text();
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let mut world = World::load(config, max_sessions, snapshot()).expect("world loads");
        edit(&mut world);
        let engine = Engine::new(world, clock.clone(), SeededRng::new(7), tx);
        Self {
            engine,
            clock,
            rx,
            log: Vec::new(),
            next_conn: 1,
        }
    }

    fn default() -> Self {
        Self::new(test_config(), 8, |_| {})
    }

    fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            self.log.push(o);
        }
    }

    fn handle(&mut self, input: EngineInput) {
        self.engine.handle(input);
        self.pump();
    }

    /// Connects and returns the session id.
    fn connect(&mut self) -> SessionId {
        let conn = ConnId(self.next_conn);
        self.next_conn += 1;
        self.handle(EngineInput::Connected {
            conn,
            caps: Caps::default(),
        });
        self.log
            .iter()
            .rev()
            .find_map(|o| match o {
                EngineOutput::Opened { conn: c, id } if *c == conn => Some(*id),
                _ => None,
            })
            .expect("opened")
    }

    fn login(&mut self, id: SessionId, character: Option<Character>) {
        self.login_as(id, AccountId(i64::from(id.0) + 1), character);
    }

    /// The account a session is logged in to.
    fn account(&self, id: SessionId) -> AccountId {
        self.engine
            .world
            .session(id)
            .unwrap()
            .account
            .as_ref()
            .unwrap()
            .id
    }

    fn login_as(&mut self, id: SessionId, account: AccountId, character: Option<Character>) {
        let name = character
            .as_ref()
            .map_or("Newbie".to_string(), |c| c.firstname.clone());
        self.handle(EngineInput::LoggedIn {
            id,
            login: Box::new(Login {
                account: AccountRef {
                    id: account,
                    username: name,
                    sysop: false,
                    gameop: false,
                    sex: b'M',
                },
                character: character.map(Box::new),
                may_gossip: true,
                may_auction: true,
                sysop_keys: silvermere_engine::commands::sysop::Keys::empty(),
            }),
        });
    }

    fn line(&mut self, id: SessionId, text: &str) {
        self.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
    }

    fn ticks(&mut self, n: u32) {
        self.clock.advance_ticks(n, &mut self.engine);
        self.pump();
    }

    /// Everything written to `id` since output index `from`.
    fn written_since(&self, from: usize, id: SessionId) -> Vec<u8> {
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

    fn text_since(&self, from: usize, id: SessionId) -> String {
        String::from_utf8_lossy(&self.written_since(from, id)).into_owned()
    }

    fn closed(&self, id: SessionId) -> bool {
        self.log
            .iter()
            .any(|o| matches!(o, EngineOutput::Close { id: i } if *i == id))
    }

    fn last_save(&self) -> Option<Character> {
        self.log.iter().rev().find_map(|o| match o {
            EngineOutput::Persist(PersistRequest::Character { character, .. }) => {
                Some((**character).clone())
            }
            _ => None,
        })
    }

    fn state(&self, id: SessionId) -> Option<SessionState> {
        self.engine.world.session(id).map(|s| s.state)
    }

    /// Connect, log in and select MajorMUD at the shim: at the `[MAJORMUD]:` menu.
    fn at_menu(&mut self, character: Option<Character>) -> SessionId {
        let id = self.connect();
        self.login(id, character);
        self.line(id, "M");
        id
    }

    /// Connect and enter the realm.
    fn playing(&mut self, character: Character) -> SessionId {
        let id = self.at_menu(Some(character));
        self.line(id, "E");
        assert_eq!(self.state(id), Some(SessionState::Playing));
        id
    }
}

/// The tiny world plus a hang-up room on the start room and six droppable items. The
/// start room is the capture's: room 1/2140 of the stock `WCCMP001`, the Newhaven
/// Village Entrance, with its two items.
fn snapshot() -> silvermere_engine::WorldSnapshot {
    let mut s = tiny_snapshot();
    let [start, reinc, ..] = special_rooms();
    for r in &mut s.rooms {
        if r.key == RoomKey::new(1, start) {
            r.huproom = RoomId(reinc);
            r.name = "Newhaven, Village Entrance".into();
            for (i, l) in [
                "Welcome to Newhaven! You are standing at the crude wooden gates of the",
                "village entrance, in the middle of a dusty path. A low wooden palisade",
                "surrounds the village, to protect against creatures of the night and",
                "random raiders. You can see small buildings to the north and south,",
                "and a dusty path that leads to the west and southeast.",
            ]
            .into_iter()
            .enumerate()
            {
                r.desc[i] = l.into();
            }
            for (d, to) in [(0, 2141), (1, 2142), (3, 2143), (6, 2148)] {
                r.exits[d].target = RoomId(to);
            }
            r.items[0].item = ItemNo(1098);
            r.items[1].item = ItemNo(11);
        }
    }
    for (n, name) in [(11, "large sign"), (1098, "newbie manual")] {
        let mut item = silvermere_engine::world::blank::clear_item(ItemNo(n));
        item.name = name.into();
        s.items.push(item);
    }
    for n in 3..=8 {
        let mut item = silvermere_engine::world::blank::clear_item(ItemNo(n));
        item.weight = 1;
        s.items.push(item);
    }
    // Item 8 is a Loyal Item: kept on a hang-up.
    let loyal = s.items.iter_mut().find(|i| i.number == ItemNo(8)).unwrap();
    loyal.abilities[0] = (Ability::LoyalItem, 0);
    s
}

/// Aldric of the capture: level 1, 35 hit points, in the start room.
fn aldric() -> Character {
    character("Aldric", special_rooms()[0])
}

fn character(name: &str, room: u32) -> Character {
    let mut c = clear_player();
    c.firstname = name.into();
    c.userid = name.to_ascii_uppercase();
    c.map = MapId(1);
    c.room = RoomId(room);
    c.msglevel = MessageLevel::High;
    c.hp = 35;
    c.maxhp = 35;
    c.class = ClassNo(1);
    c.race = RaceNo(1);
    c.sex = b'M';
    c.statline = StatlineMode::Full;
    c.verbose = 1;
    // A stored row as `load_player` (T-023) accepts it: record version 22, level 1, stats
    // inside the zeroed test race's range (5..=50), the name validated, and `hpbonus`
    // making the recomputed maximum (50/2 + 10) the 35 hit points the prompt shows.
    c.version = 22;
    c.level = 1;
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (50, 50, 50, 50, 50, 50);
    (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (50, 50, 50, 50, 50, 50);
    c.hpbonus = 10;
    c.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
    // Made by the original's character creation: the race change is spent, so
    // the menu has no CHANGERACE line, as in the capture.
    c.flags |= silvermere_types::flags::PlayerFlags::RACECHANGED;
    c
}

/// Every `X\b` pair dropped: the random letter `dirnames[]` carries.
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

/// The bytes of the capture from the end of the first `after` at or past `from` up to
/// and including the first `until` after that.
fn capture_between(from: usize, after: &[u8], until: &[u8]) -> (usize, Vec<u8>) {
    let a = find(CAPTURE, after, from).expect("start marker in the capture") + after.len();
    let u = find(CAPTURE, until, a).expect("end marker in the capture") + until.len();
    (u, CAPTURE[a..u].to_vec())
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn esc(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

const MENU_PROMPT: &[u8] = b"\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m";
const BBS_PROMPT: &[u8] = b"Make your selection (X to exit): ";

// logon, SUBST_START/ENTER, USRFOLED; cmd_quit; fast_update_character; the MBBSEmu BBS
// menu. Compared byte for byte with the capture, the client's echo and the parts other
// cards draw (the login prompts: silvermere-auth; the room: T-025) left out.
#[test]
fn entry_and_exit_match_the_capture() {
    let mut h = Harness::default();
    let id = h.connect();
    let mut stream = Vec::new();

    // Logged in: USRLOGIN and the BBS menu.
    let mark = h.log.len();
    h.login(id, Some(aldric()));
    let got = h.written_since(mark, id);
    let (pos, want) = capture_between(0, b"*******\r\n", BBS_PROMPT);
    assert_eq!(esc(&got), esc(&want), "greeting and BBS menu");
    stream.extend_from_slice(&got);

    // M: MBBSEmu's clear screen, then the module's menu.
    let mark = h.log.len();
    h.line(id, "M");
    let got = h.written_since(mark, id);
    let (pos, want) = capture_between(pos, b"M\r\n", MENU_PROMPT);
    assert_eq!(esc(&got), esc(&want), "the [MAJORMUD]: menu");
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    stream.extend_from_slice(&got);

    // E: the banner, the room with its description (Aldric is verbose), the prompt.
    // The direction names' random letters are dropped on both sides.
    let mark = h.log.len();
    h.line(id, "E");
    let got = h.written_since(mark, id);
    let (pos, want) = capture_between(pos, b"E\r\n", b"]:");
    assert_eq!(
        esc(&drop_backspace_pairs(&got)),
        esc(&drop_backspace_pairs(&want)),
        "the banner, the room, the prompt"
    );
    assert_eq!(h.state(id), Some(SessionState::Playing));
    stream.extend_from_slice(&got);

    // An empty line: the short room under the full status line, then the prompt.
    let mark = h.log.len();
    h.line(id, "");
    let got = h.written_since(mark, id);
    let (pos, want) = capture_between(pos, b"\r\n", b"]:");
    assert_eq!(
        esc(&drop_backspace_pairs(&got)),
        esc(&drop_backspace_pairs(&want)),
        "the room again"
    );
    stream.extend_from_slice(&got);

    // x: the meditation, a `...` per fast cycle, then USREXIT and [MAJORMUD]:.
    let mark = h.log.len();
    h.line(id, "x");
    h.ticks(leave::EXIT_DELAY.into());
    let got = h.written_since(mark, id);
    let (pos, want) = capture_between(pos - 2, b"]:x\r\n", MENU_PROMPT);
    assert_eq!(esc(&got), esc(&want), "the exit");
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    stream.extend_from_slice(&got);

    // X at the menu: the BBS menu again.
    let mark = h.log.len();
    h.line(id, "X");
    let got = h.written_since(mark, id);
    let (pos, want) = capture_between(pos, b"X\r\n", BBS_PROMPT);
    assert_eq!(esc(&got), esc(&want), "back at the BBS menu");
    stream.extend_from_slice(&got);

    // X, Y at the BBS menu: the confirmation and the goodbye, then the line drops.
    let mark = h.log.len();
    h.line(id, "X");
    h.line(id, "Y");
    let got = h.written_since(mark, id);
    let (_, want) = capture_between(pos, b"X\r\n", b"Have a nice day...\r\n\r\n");
    let want: Vec<u8> = String::from_utf8_lossy(&want)
        .replace("? Y\r\n", "? ")
        .into_bytes();
    assert_eq!(esc(&got), esc(&want), "logoff");
    assert!(h.closed(id));
    stream.extend_from_slice(&got);

    // The build stamp masked, as the capture's `mask` list does for `mud_title`.
    let stream = esc(&stream).replace("(Dec 30 2005 14:20:05)", "(<build stamp>)");
    insta::assert_snapshot!("entry_existing_stream", stream);
}

// the realm takes `maxingame` players, counted apart from the
// session slots; the next one is refused at the menu.
#[test]
fn game_full_at_max_in_game() {
    let mut h = Harness::new(test_config(), 8, |w| w.max_in_game = 1);
    h.playing(aldric());
    let id = h.at_menu(Some(character("Bob", special_rooms()[0])));
    let mark = h.log.len();
    h.line(id, "E");
    let t = h.text_since(mark, id);
    assert!(
        t.starts_with("\x1b[1;37m\r\nWe're sorry, but the game is currently full.\r\n"),
        "{t:?}"
    );
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    // with the default (`server.max_sessions`) both get in
    let mut h = Harness::default();
    h.playing(aldric());
    h.playing(character("Bob", special_rooms()[0]));
}

// SUBST_MAIN_MENU: P and E both enter, in either case.
#[test]
fn p_e_and_lower_case_enter() {
    for key in ["E", "P", "e", "play", "enter now"] {
        let mut h = Harness::default();
        let id = h.at_menu(Some(aldric()));
        let mark = h.log.len();
        h.line(id, key);
        assert_eq!(h.state(id), Some(SessionState::Playing), "{key}");
        assert!(h.text_since(mark, id).contains("[HP=35"), "{key}");
    }
}

// an empty line shows the menu again; any other word is refused.
#[test]
fn unknown_input_at_the_menu() {
    let mut h = Harness::default();
    let id = h.at_menu(Some(aldric()));
    let mark = h.log.len();
    h.line(id, "");
    let t = h.text_since(mark, id);
    assert!(
        t.contains("Enter the Realm") && t.ends_with("\x1b[0;37;40m"),
        "{t:?}"
    );
    let mark = h.log.len();
    h.line(id, "zork");
    assert_eq!(
        h.written_since(mark, id),
        b"\r\nInvalid Option.\r\n\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m"
    );
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
}

// The session states: with compat.bbs_menu off the login goes
// straight to the [MAJORMUD]: menu, and X at the menu logs off.
#[test]
fn bbs_menu_off_goes_straight_to_the_menu() {
    let mut h = Harness::new(test_config(), 8, |w| w.compat.bbs_menu = false);
    let id = h.connect();
    h.login(id, Some(aldric()));
    let t = h.text_since(0, id);
    assert!(t.contains("Fantasy awaits you in"));
    assert!(!t.contains("Make your selection"));
    assert!(t.contains("Enter the Realm"));
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    h.line(id, "x");
    assert!(h.closed(id));
}

// The shim's R re-logs on without hanging up; N goes back to the BBS menu.
#[test]
fn bbs_menu_log_on_again_and_no() {
    let mut h = Harness::default();
    let id = h.connect();
    h.login(id, Some(aldric()));
    h.line(id, "X");
    let mark = h.log.len();
    h.line(id, "N");
    assert!(h.text_since(mark, id).contains("Make your selection"));
    h.line(id, "X");
    h.line(id, "R");
    assert!(
        h.log
            .iter()
            .any(|o| matches!(o, EngineOutput::ReturnToLogin { id: i } if *i == id))
    );
    assert!(!h.closed(id));
    assert!(h.engine.world.session(id).unwrap().account.is_none());
}

// A new account (signup with NEW) has no character: it reaches the module menu, and E
// starts character creation with the race list (T-022).
#[test]
fn new_account_reaches_the_menu() {
    let mut h = Harness::default();
    let id = h.at_menu(None);
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    let mark = h.log.len();
    h.line(id, "E");
    assert_eq!(
        h.state(id),
        Some(SessionState::NewChar(
            silvermere_types::enums::NewCharStep::Race
        ))
    );
    let text = h.text_since(mark, id);
    assert!(
        text.contains("Please choose a race from the following list:"),
        "{text:?}"
    );
    assert!(
        text.contains("Please choose your race [ ? for help ] : "),
        "{text:?}"
    );
    assert!(
        h.engine.world.session(id).unwrap().player().is_some(),
        "create_player"
    );
}

// the entrance and exit go to the game,
// filtered by the PF_NOENTRYMSG setting.
#[test]
fn entered_and_left_go_to_the_game_by_entrance_setting() {
    let mut h = Harness::default();
    let watcher = h.playing(character("Wanda", special_rooms()[1]));
    let mut deaf = character("Deaf", special_rooms()[1]);
    deaf.flags.insert(PlayerFlags::NOENTRYMSG);
    let deaf = h.playing(deaf);
    let mark = h.log.len();
    let id = h.playing(aldric());
    let w = h.text_since(mark, watcher);
    assert!(
        w.contains("\x1b[79D\x1b[KAldric just entered the Realm.\r\n"),
        "{w:?}"
    );
    assert!(w.contains("[HP="), "the listener's prompt follows: {w:?}");
    assert!(!h.text_since(mark, deaf).contains("entered"));
    assert!(!h.text_since(mark, id).contains("Aldric just entered"));

    let mark = h.log.len();
    h.line(id, "x");
    h.ticks(leave::EXIT_DELAY.into());
    assert!(
        h.text_since(mark, watcher)
            .contains("Aldric just left the Realm.")
    );
    assert!(!h.text_since(mark, deaf).contains("left the Realm"));
}

// cmd_quit: no exit at 0 hit points or below; the prompt follows.
#[test]
fn x_while_mortally_wounded_is_refused() {
    let mut h = Harness::default();
    let mut c = aldric();
    c.hp = 0;
    let id = h.playing(c);
    let mark = h.log.len();
    h.line(id, "x");
    let t = h.text_since(mark, id);
    assert!(
        t.starts_with("You may not exit MajorMud while you are mortally wounded!\r\n"),
        "{t:?}"
    );
    assert!(t.contains("[HP=\x1b[1;31m0\x1b[0;37m]:"), "{t:?}");
    h.ticks(10);
    assert_eq!(h.state(id), Some(SessionState::Playing));
}

// the room sees the meditation; while waiting
// every command is refused.
#[test]
fn commands_while_exiting_are_refused() {
    let mut h = Harness::default();
    let other = h.playing(character("Bob", special_rooms()[0]));
    let id = h.playing(aldric());
    let mark = h.log.len();
    h.line(id, "x");
    assert!(
        h.text_since(mark, other)
            .contains("\x1b[79D\x1b[KAldric sits down and meditates.\r\n")
    );
    let mark = h.log.len();
    h.line(id, "look");
    assert_eq!(
        h.text_since(mark, id),
        "You may not perform any commands while waiting to exit!\r\n"
    );
    h.ticks(leave::EXIT_DELAY.into());
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    let saved = h.last_save().expect("saved on exit");
    assert!(!saved.status.contains(PlayerStatus::EXITING));
}

// stop_users_exit: an attack (the monster attack path, T-033)
// interrupts the meditation; the exit does not happen.
#[test]
fn an_attack_interrupts_the_exit() {
    let mut h = Harness::default();
    let id = h.playing(aldric());
    h.line(id, "x");
    h.ticks(2);
    let mark = h.log.len();
    h.handle(EngineInput::Custom {
        session: Some(id),
        run: Box::new(move |w| leave::stop_users_exit(w, id)),
    });
    let t = h.text_since(mark, id);
    assert!(
        t.starts_with("Your meditation has been interrupted - you may not exit now!\r\n"),
        "{t:?}"
    );
    h.ticks(10);
    assert_eq!(h.state(id), Some(SessionState::Playing));
    // BREAK does the same.
    h.line(id, "x");
    let mark = h.log.len();
    h.line(id, "break");
    assert!(h.text_since(mark, id).contains("interrupted"));
}

// hangup, load_player: drop the line in the realm, reconnect, and the punishment shows on the next
// entry.
#[test]
fn disconnect_in_the_realm_is_punished_and_reported() {
    let config = GameConfig {
        pun_hup: PunHup::High,
        min_hup_hp: 10,
        max_hup_hp: 40,
        max_hup_items: 2,
        ..test_config()
    };
    let mut h = Harness::new(config, 8, |_| {});
    let watcher = h.playing(character("Wanda", special_rooms()[1]));
    let mut c = aldric();
    // Entry recomputes the maximum (`load_player`, T-023): 50/2 + 75.
    c.hpbonus = 75;
    c.maxhp = 100;
    c.hp = 100;
    for (slot, n) in [(0, 3), (1, 4), (2, 8), (4, 5), (6, 6)] {
        c.items[slot] = ItemNo(n);
        c.itemuses[slot] = 0;
    }
    let carried = c.items.iter().filter(|i| i.0 != 0).count();
    let id = h.playing(c);

    let mark = h.log.len();
    h.handle(EngineInput::Disconnect { id });
    assert!(
        h.text_since(mark, watcher)
            .contains("\x1b[79D\x1b[K\x1b[1;37mAldric just disconnected!!!\r\n")
    );
    let saved = h.last_save().expect("saved at the hang-up");
    assert!(saved.status.contains(PlayerStatus::HUNGUP));
    let lost = 100 - i32::from(saved.hp);
    assert!((10..=40).contains(&lost), "lost {lost}");
    let left = saved.items.iter().filter(|i| i.0 != 0).count();
    assert!(carried - left <= 2, "{carried} -> {left}");
    // Slots 0, 2, 4, ...: item 3 drops, item 8 is Loyal and stays, item 5 drops.
    assert_eq!(saved.items[0], ItemNo(0));
    assert_eq!(saved.items[1], ItemNo(4));
    assert_eq!(saved.items[2], ItemNo(8));
    assert_eq!(saved.items[4], ItemNo(0));
    assert_eq!(saved.items[6], ItemNo(6));
    // The start room's hang-up room.
    assert_eq!(saved.room, RoomId(special_rooms()[1]));

    // Back again: the notice pair before the banner, then the realm.
    let id = h.at_menu(Some(saved));
    let mark = h.log.len();
    h.line(id, "E");
    let t = h.text_since(mark, id);
    let note = t
        .find("\x1b[1;37m\r\nLast time you were on, you disconnected while playing.\r\n")
        .expect("the hang-up notice");
    let punished = t
        .find("The gods have punished you appropriately.\r\n")
        .expect("the second line");
    let banner = t.find("MajorMUD - ROL").expect("the banner");
    assert!(note < punished && punished < banner, "{t:?}");
    let plr = h.engine.world.session(id).unwrap().player().unwrap();
    assert!(plr.status.is_empty());
    assert_eq!(plr.room, RoomId(special_rooms()[1]));
}

// in a protected room nothing is taken.
#[test]
fn disconnect_in_a_safe_room_is_not_punished() {
    let config = GameConfig {
        pun_hup: PunHup::High,
        min_hup_hp: 50,
        max_hup_hp: 50,
        ..test_config()
    };
    let start = special_rooms()[0];
    let mut h = Harness::new(config, 8, |w| {
        let key = RoomKey::new(1, start);
        w.rooms
            .get_mut(&key)
            .unwrap()
            .flags
            .insert(silvermere_types::flags::RoomFlags::PROTECTED);
    });
    let id = h.playing(aldric());
    h.handle(EngineInput::Disconnect { id });
    let saved = h.last_save().unwrap();
    assert_eq!(saved.hp, 35);
    assert!(!saved.status.contains(PlayerStatus::HUNGUP));
    assert_eq!(saved.room, RoomId(special_rooms()[1]));
}

/// How the player stands when the line drops, for the `PUNHUP` matrix.
#[derive(Clone, Copy, Debug)]
enum Fight {
    /// Nobody fights anybody.
    Calm,
    /// Queued against a monster (`is_inside_autocombat`).
    Monster,
    /// Another player is queued against him (`is_being_attacked`, and so
    /// `is_in_pvp_combat`); he does not fight back.
    AttackedByPlayer,
}

/// Aldric drops the line in the protected start room in the given fight, Wanda in the
/// same room; returns the saved record, Aldric's hit points before, and what Wanda saw.
fn hang_up_in_a_protected_room(pun_hup: PunHup, fight: Fight) -> (Character, i16, String) {
    let config = GameConfig {
        pun_hup,
        min_hup_hp: 20,
        max_hup_hp: 20,
        max_hup_items: 1,
        ..test_config()
    };
    let start = special_rooms()[0];
    let mut h = Harness::new(config, 8, |w| {
        w.rooms
            .get_mut(&RoomKey::new(1, start))
            .unwrap()
            .flags
            .insert(silvermere_types::flags::RoomFlags::PROTECTED);
    });
    let wanda = h.playing(character("Wanda", start));
    let mut c = aldric();
    c.items[0] = ItemNo(3);
    c.itemuses[0] = 0;
    let id = h.playing(c);
    h.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| match fight {
            Fight::Calm => {}
            Fight::Monster => silvermere_engine::combat::engage_autocombat(
                w,
                id,
                AutoTarget::Monster(ActiveMonsterId(1)),
                Move::Attack,
            ),
            Fight::AttackedByPlayer => silvermere_engine::combat::engage_autocombat(
                w,
                wanda,
                AutoTarget::Player(id),
                Move::Attack,
            ),
        }),
    });
    let hp = h.engine.world.session(id).unwrap().player().unwrap().hp;
    let mark = h.log.len();
    h.handle(EngineInput::Disconnect { id });
    let seen = h.text_since(mark, wanda);
    (h.last_save().expect("saved at the hang-up"), hp, seen)
}

// hangup: a protected room is safe only out of combat
// (`is_inside_autocombat`, `is_being_attacked`); outside it, PUN_HIGH always punishes,
// PUN_MEDIUM in auto-combat or under attack, PUN_LOW in player combat, and ST_HUNGUP is
// set whether or not the hang-up was punished.
#[test]
fn hang_up_in_a_protected_room_follows_punhup_and_the_fight() {
    use Fight::*;
    use PunHup::*;
    // (level, fight, punished, hung up)
    let cases = [
        (High, Calm, false, false),
        (Medium, Calm, false, false),
        (Low, Calm, false, false),
        (High, Monster, true, true),
        (Medium, Monster, true, true),
        (Low, Monster, false, true),
        (None, Monster, false, true),
        (High, AttackedByPlayer, true, true),
        (Medium, AttackedByPlayer, true, true),
        (Low, AttackedByPlayer, true, true),
        (None, AttackedByPlayer, false, true),
    ];
    for (level, fight, punished, hungup) in cases {
        let (saved, hp, _) = hang_up_in_a_protected_room(level, fight);
        let case = format!("{level:?} {fight:?}");
        // 20% of 35 is 7
        let want_hp = if punished { hp - 7 } else { hp };
        assert_eq!(saved.hp, want_hp, "{case}");
        // the one item in slot 0 drops when punished
        let want_item = if punished { ItemNo(0) } else { ItemNo(3) };
        assert_eq!(saved.items[0], want_item, "{case}");
        assert_eq!(
            saved.status.contains(PlayerStatus::HUNGUP),
            hungup,
            "{case}"
        );
        // the hang-up room either way
        assert_eq!(saved.room, RoomId(special_rooms()[1]), "{case}");
    }
}

// the room sees the same line in a fight as out of one.
#[test]
fn hang_up_in_a_fight_tells_the_game() {
    let (_, _, seen) = hang_up_in_a_protected_room(PunHup::High, Fight::AttackedByPlayer);
    assert!(
        seen.starts_with("\x1b[79D\x1b[K\x1b[1;37mAldric just disconnected!!!\r\n"),
        "{seen:?}"
    );
}

// a Colliseum room in the arena's safe mode (`arenasafe`, the
// default) is safe even in a fight; with SYSOP ARENA COMBAT it is not.
#[test]
fn hang_up_in_a_safe_arena_is_not_punished_even_in_a_fight() {
    for arena_safe in [true, false] {
        let config = GameConfig {
            pun_hup: PunHup::High,
            min_hup_hp: 20,
            max_hup_hp: 20,
            ..test_config()
        };
        let start = special_rooms()[0];
        let mut h = Harness::new(config, 8, |w| {
            w.rooms.get_mut(&RoomKey::new(1, start)).unwrap().room_type =
                silvermere_types::enums::RoomType::Colliseum;
            w.display.arena_safe = arena_safe;
        });
        let id = h.playing(aldric());
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                silvermere_engine::combat::engage_autocombat(
                    w,
                    id,
                    AutoTarget::Monster(ActiveMonsterId(1)),
                    Move::Attack,
                )
            }),
        });
        h.handle(EngineInput::Disconnect { id });
        let saved = h.last_save().unwrap();
        assert_eq!(saved.hp, if arena_safe { 35 } else { 28 }, "{arena_safe}");
        assert_eq!(
            saved.status.contains(PlayerStatus::HUNGUP),
            !arena_safe,
            "{arena_safe}"
        );
    }
}

// T-075: the gate reads a login's character from the store before the engine sees the
// login, and the hang-up's save may not have landed yet. A login that brings the record
// from before the hang-up starts from the one the hang-up saved: the notice, the lost
// hit points and the dropped items are all there.
#[test]
fn a_quick_login_after_a_hang_up_sees_the_hang_up() {
    let config = GameConfig {
        pun_hup: PunHup::High,
        min_hup_hp: 20,
        max_hup_hp: 20,
        max_hup_items: 2,
        ..test_config()
    };
    let mut h = Harness::new(config, 8, |_| {});
    let mut c = aldric();
    c.items[0] = ItemNo(3);
    c.itemuses[0] = 0;
    c.items[2] = ItemNo(5);
    c.itemuses[2] = 0;
    let stale = c.clone();
    let id = h.playing(c);
    let account = h.account(id);
    h.handle(EngineInput::Disconnect { id });
    let saved = h.last_save().unwrap();
    assert!(saved.status.contains(PlayerStatus::HUNGUP));

    // the store has not caught up: the gate brings the old row
    let id = h.connect();
    h.login_as(id, account, Some(stale.clone()));
    h.line(id, "M");
    let mark = h.log.len();
    h.line(id, "E");
    let t = h.text_since(mark, id);
    assert!(
        t.starts_with(
            "\x1b[1;37m\r\nLast time you were on, you disconnected while playing.\r\n\
             The gods have punished you appropriately.\r\n"
        ),
        "{t:?}"
    );
    let plr = h.engine.world.session(id).unwrap().player().unwrap();
    assert_eq!(plr.hp, saved.hp);
    assert_eq!(plr.items[0], ItemNo(0));
    assert_eq!(plr.items[2], ItemNo(0));
    assert_eq!(plr.room, RoomId(special_rooms()[1]));

    // taken once: after a clean logoff the next login reads the queued logoff record
    h.line(id, "x");
    h.ticks(10);
    h.line(id, "X");
    let id = h.connect();
    h.login_as(id, account, Some(stale));
    h.line(id, "M");
    let mark = h.log.len();
    h.line(id, "E");
    assert!(
        !h.text_since(mark, id)
            .contains("disconnected while playing")
    );
    let plr = h.engine.world.session(id).unwrap().player().unwrap();
    assert_eq!(plr.items[0], ItemNo(0));
}

// T-075: a character deleted while the deletion is still queued does not come back from
// a stale read.
#[test]
fn a_quick_login_after_a_deletion_has_no_character() {
    let mut h = Harness::default();
    let c = aldric();
    let id = h.playing(c.clone());
    let account = h.account(id);
    h.handle(EngineInput::Custom {
        session: Some(id),
        run: Box::new(move |w| silvermere_engine::player::delete_player(w, id)),
    });
    h.handle(EngineInput::Disconnect { id });
    let id = h.connect();
    h.login_as(id, account, Some(c));
    assert!(h.engine.world.session(id).unwrap().character.is_none());
}

/// The config of the T-075 hang-up tests: every hang-up punished, 20% of the hit points
/// and up to two items.
fn punishing() -> GameConfig {
    GameConfig {
        pun_hup: PunHup::High,
        min_hup_hp: 20,
        max_hup_hp: 20,
        max_hup_items: 2,
        ..test_config()
    }
}

/// The store acknowledges the first `written` character writes (T-088).
fn store_wrote(h: &mut Harness, written: u64) {
    h.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| silvermere_engine::player::on_characters_written(w, written)),
    });
}

// T-088: once the store acknowledges the hang-up's save its row is current, so the queued
// record goes and the next login takes the gate's read.
#[test]
fn the_store_acknowledgement_drops_the_queued_record() {
    let mut h = Harness::new(punishing(), 8, |_| {});
    let id = h.playing(aldric());
    let account = h.account(id);
    h.handle(EngineInput::Disconnect { id });
    assert!(h.engine.world.queued_characters.contains_key(&account));
    let written = h.engine.world.character_writes;
    store_wrote(&mut h, written);
    assert!(h.engine.world.queued_characters.is_empty());

    let mut row = h.last_save().unwrap();
    row.hp = 11;
    let id = h.connect();
    h.login_as(id, account, Some(row));
    assert_eq!(
        h.engine
            .world
            .session(id)
            .unwrap()
            .character
            .as_ref()
            .unwrap()
            .hp,
        11
    );
}

// T-088: the gate may have read a login's row before the store committed the hang-up's
// save and still be on its way when the acknowledgement comes. Such a login belongs to a
// connection in the login dialogue at that moment, so the acknowledgement waits for it:
// the login still starts from the hang-up's record, and once nobody it waited on is
// logging in (here, a connection that gives up), the next tick drops the records.
#[test]
fn an_acknowledgement_waits_for_the_logins_in_progress() {
    let mut h = Harness::new(punishing(), 8, |_| {});
    let stale = aldric();
    let id = h.playing(stale.clone());
    let account = h.account(id);
    h.handle(EngineInput::Disconnect { id });
    let comer = h.connect();
    let quitter = h.connect();
    let written = h.engine.world.character_writes;
    store_wrote(&mut h, written);
    assert!(h.engine.world.queued_characters.contains_key(&account));

    h.login_as(comer, account, Some(stale));
    let plr = h.engine.world.session(comer).unwrap().player().unwrap();
    assert!(plr.status.contains(PlayerStatus::HUNGUP));
    assert_eq!(plr.hp, 28);

    // a second account's save, acknowledged while the quitter is still at the dialogue
    let other = h.playing(character("Wanda", special_rooms()[0]));
    let other_account = h.account(other);
    h.handle(EngineInput::Disconnect { id: other });
    let written = h.engine.world.character_writes;
    store_wrote(&mut h, written);
    h.ticks(1);
    assert!(
        h.engine
            .world
            .queued_characters
            .contains_key(&other_account)
    );
    h.handle(EngineInput::Disconnect { id: quitter });
    h.ticks(1);
    assert!(h.engine.world.queued_characters.is_empty());
    assert!(h.engine.world.characters_written.is_empty());
}

// T-088: an acknowledgement covers only the writes before it: a save queued after them
// keeps its record until its own acknowledgement, and a login before that still starts
// from it.
#[test]
fn an_older_acknowledgement_keeps_a_newer_queued_record() {
    let mut h = Harness::new(punishing(), 8, |_| {});
    let stale = aldric();
    let id = h.playing(stale.clone());
    let account = h.account(id);
    h.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            silvermere_engine::player::save_player(w, id);
        }),
    });
    let entered = h.engine.world.character_writes;
    assert!(entered > 0);
    h.handle(EngineInput::Disconnect { id });
    let hung = h.engine.world.character_writes;
    assert!(hung > entered);
    store_wrote(&mut h, entered);
    let queued = &h.engine.world.queued_characters[&account];
    assert_eq!(queued.seq, hung);
    assert!(
        queued
            .record
            .as_ref()
            .unwrap()
            .status
            .contains(PlayerStatus::HUNGUP)
    );

    let id = h.connect();
    h.login_as(id, account, Some(stale));
    let plr = h
        .engine
        .world
        .session(id)
        .unwrap()
        .character
        .as_ref()
        .unwrap();
    assert!(plr.status.contains(PlayerStatus::HUNGUP));
    assert_eq!(plr.hp, 28);
    // the login took the record; the late acknowledgement finds nothing to drop
    assert!(h.engine.world.queued_characters.is_empty());
    store_wrote(&mut h, hung);
    assert!(h.engine.world.queued_characters.is_empty());
}

// T-088: the store answered a gang lookup with Aldric's row, then he came in and dropped
// the line before the answer reached the engine. The promotion is made on the record the
// hang-up saved, as `save_player` had already stored it: the penalty, the dropped items
// and ST_HUNGUP stay.
#[test]
fn a_gang_offline_edit_after_a_hang_up_keeps_the_hang_up() {
    use silvermere_engine::gangs::{self, GangAnswer, GangQuery, OfflineOp};
    let mut h = Harness::new(punishing(), 8, |w| {
        w.gangs.insert(
            "WOLVES".into(),
            silvermere_types::records::Gang {
                key: "WOLVES".into(),
                name: "Wolves".into(),
                exp: 0,
                leader: "Wanda".into(),
                created: 0,
                members: 2,
                flags: silvermere_types::flags::GangFlags::empty(),
                overexp: 0,
                overflows: 0,
            },
        );
    });
    let mut wanda = character("Wanda", special_rooms()[0]);
    wanda.gang = "Wolves".into();
    let wanda = h.playing(wanda);
    let mut c = aldric();
    c.gang = "Wolves".into();
    c.items[0] = ItemNo(3);
    c.itemuses[0] = 0;
    let stale = c.clone();
    let id = h.playing(c);
    let account = h.account(id);
    h.handle(EngineInput::Disconnect { id });

    let mark = h.log.len();
    h.handle(EngineInput::Custom {
        session: Some(wanda),
        run: Box::new(move |w| {
            gangs::on_query_answer(
                w,
                wanda,
                GangQuery::Offline {
                    name: "aldric".into(),
                    gang: "Wolves".into(),
                    op: OfflineOp::Promote,
                },
                GangAnswer::Offline(Some((account, Box::new(stale)))),
            )
        }),
    });
    // the notice, then the prompt
    assert_eq!(
        h.text_since(mark, wanda),
        "Gang member Aldric will be notified of their promotion next time they log on.\r\n\
         \x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:"
    );
    let saved = h.last_save().unwrap();
    assert!(saved.flags.contains(PlayerFlags::PROMOTED));
    assert!(saved.status.contains(PlayerStatus::HUNGUP));
    assert_eq!(saved.hp, 28);
    assert_eq!(saved.items[0], ItemNo(0));
    assert_eq!(saved.room, RoomId(special_rooms()[1]));
}

// inactivity_check: two strikes, a typed line clears the mark.
#[test]
fn inactivity_hangs_up_after_two_periods() {
    let config = GameConfig {
        inactive_secs: 60,
        ..test_config()
    };
    let mut h = Harness::new(config, 8, |_| {});
    let id = h.playing(aldric());
    h.ticks(60);
    assert!(!h.closed(id), "one period: marked only");
    let mark = h.log.len();
    h.ticks(60);
    assert!(h.closed(id), "two periods: hung up");
    assert_eq!(
        h.text_since(mark, id),
        "\x1b[0m\r\nYou have been inactive for too long.   Bye!\r\n\r\n"
    );
    // Set to the main menu first, so the hang-up is no punishment.
    assert!(!h.last_save().unwrap().status.contains(PlayerStatus::HUNGUP));
}

// any typed line clears the mark between the checks.
#[test]
fn a_line_between_the_checks_resets_the_strike() {
    let config = GameConfig {
        inactive_secs: 60,
        ..test_config()
    };
    let mut h = Harness::new(config, 8, |_| {});
    let id = h.at_menu(Some(aldric()));
    h.ticks(60);
    h.line(id, "");
    h.ticks(60);
    assert!(!h.closed(id));
    h.ticks(60);
    assert!(h.closed(id));
}

/// `n` searchers counted in the start room with `foundmask` 1, Aldric one of them
/// (`ST_SEARCHED`, as `search_room` leaves it); a second one is Wanda when `n` is 2.
fn searching(h: &mut Harness, ids: &[SessionId], n: u8) {
    let ids = ids.to_vec();
    h.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            let key = RoomKey::new(1, special_rooms()[0]);
            let st = w.room_state.get_mut(&key).unwrap();
            st.searchers = n;
            st.foundmask = 1;
            for id in ids {
                w.session_mut(id)
                    .unwrap()
                    .character
                    .as_mut()
                    .unwrap()
                    .status
                    .insert(PlayerStatus::SEARCHED);
            }
        }),
    });
}

fn start_state(h: &Harness) -> (u8, u32) {
    let st = &h.engine.world.room_state[&RoomKey::new(1, special_rooms()[0])];
    (st.searchers, st.foundmask)
}

// hangup calls clear_search_flags: the last searcher gone hides the found slots again; with
// another searcher left the count drops by one and the slots stay found.
#[test]
fn hang_up_clears_the_search_flags() {
    let mut h = Harness::default();
    let id = h.playing(aldric());
    searching(&mut h, &[id], 1);
    h.handle(EngineInput::Disconnect { id });
    assert_eq!(start_state(&h), (0, 0));
    assert!(
        !h.last_save()
            .unwrap()
            .status
            .contains(PlayerStatus::SEARCHED)
    );

    let mut h = Harness::default();
    let wanda = h.playing(character("Wanda", special_rooms()[0]));
    let id = h.playing(aldric());
    searching(&mut h, &[wanda, id], 2);
    h.handle(EngineInput::Disconnect { id });
    assert_eq!(start_state(&h), (1, 1));
}

// X, the exit's wait over: the same, before the character is saved.
#[test]
fn quitting_clears_the_search_flags() {
    let mut h = Harness::default();
    let id = h.playing(aldric());
    searching(&mut h, &[id], 1);
    h.line(id, "x");
    h.ticks(leave::EXIT_DELAY.into());
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    assert_eq!(start_state(&h), (0, 0));
    assert!(
        !h.last_save()
            .unwrap()
            .status
            .contains(PlayerStatus::SEARCHED)
    );

    let mut h = Harness::default();
    let wanda = h.playing(character("Wanda", special_rooms()[0]));
    let id = h.playing(aldric());
    searching(&mut h, &[wanda, id], 2);
    h.line(id, "x");
    h.ticks(leave::EXIT_DELAY.into());
    assert_eq!(start_state(&h), (1, 1));
}

/// Exit with auto-combat on or off; the bytes from the `x` echo to the end of the wait,
/// and whether the player is still in the ring afterwards.
fn exit_bytes(auto: bool) -> (Vec<u8>, bool) {
    let mut h = Harness::default();
    let id = h.playing(aldric());
    if auto {
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                silvermere_engine::combat::engage_autocombat(
                    w,
                    id,
                    AutoTarget::Monster(ActiveMonsterId(1)),
                    Move::Attack,
                )
            }),
        });
    }
    let mark = h.log.len();
    h.line(id, "x");
    let queued = silvermere_engine::combat::is_inside_autocombat(&h.engine.world, id);
    h.ticks(leave::EXIT_DELAY.into());
    assert_eq!(h.state(id), Some(SessionState::MainMenu));
    assert!(!silvermere_engine::combat::is_inside_autocombat(
        &h.engine.world,
        id
    ));
    (h.written_since(mark, id), queued)
}

// cmd_quit, display_autocombat_broken: with
// auto-combat on, "*Combat Off*" and a prompt come before the meditation line, the player
// leaves the ring at once (so no queued attack or spell fires during the dots), and the
// exit's 16 dots follow. Without it there is no Combat Off.
#[test]
fn exit_breaks_autocombat_before_the_meditation() {
    let (on, queued) = exit_bytes(true);
    assert!(!queued, "out of the ring as soon as x is typed");
    let (off, _) = exit_bytes(false);
    let tail = "You will exit after a period of silent meditation.\r\n................\x1b[0m\r\n\
Your character has been saved. If you have any comments or suggestions, please\r\n\
leave them in E-mail to Sysop. Thanks.\r\n\r\n\
\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m";
    let combat_off =
        "\x1b[79D\x1b[K\x1b[0;33m*Combat Off*\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=35\x1b[0;37m]:";
    assert_eq!(esc(&on), esc(format!("{combat_off}{tail}").as_bytes()));
    assert_eq!(esc(&off), esc(tail.as_bytes()));
}
