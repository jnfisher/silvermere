//! The command pipeline on the engine (T-024): the typed-ahead queue and flood control,
//! the command delay and the fast-tick drain, the empty-line rule, the mortally-wounded
//! gate, the text exits, speech and the key during a paged listing. Every tick is a `Tick`
//! input from the `TestClock`.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, special_rooms, test_config, tiny_snapshot};
use silvermere_engine::commands::dispatch::{WOUNDED_ALLOWED, dispatch};
use silvermere_engine::commands::pending::{add_delay, commands_pending};
use silvermere_engine::commands::{Cmd, CommandLine, Handled};
use silvermere_engine::output::Item;
use silvermere_engine::{EchoMode, Engine, EngineInput, EngineOutput, SeededRng, SessionId, World};
use silvermere_text::Msg;
use silvermere_types::enums::{ExitType, MessageLevel, SessionState, StatlineMode};
use silvermere_types::flags::{PlayerStatus, PlayerStatus2, SessionFlags};
use silvermere_types::ids::{MessageNo, RoomId, RoomKey};
use silvermere_types::records::Message;

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
}

impl Harness {
    fn new(edit: impl FnOnce(&mut World)) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let mut world = World::load(test_config(), 8, snapshot()).expect("world loads");
        edit(&mut world);
        let engine = Engine::new(world, clock.clone(), SeededRng::new(7), tx);
        Self {
            engine,
            clock,
            rx,
            log: Vec::new(),
        }
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

    /// A playing character in the start room, with `hp` hit points and `statline`.
    fn playing(&mut self, name: &str, statline: StatlineMode) -> SessionId {
        let id = enter(
            &mut self.engine.world,
            name,
            special_rooms()[0],
            MessageLevel::High,
        );
        let plr = self
            .engine
            .world
            .session_mut(id)
            .unwrap()
            .character
            .as_mut()
            .unwrap();
        plr.statline = statline;
        // load_player (which `enter` bypasses): no move pending
        plr.movedir = -1;
        id
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

    fn echoes_since(&self, from: usize, id: SessionId) -> Vec<EchoMode> {
        self.log[from..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::SetEcho { id: i, mode } if *i == id => Some(*mode),
                _ => None,
            })
            .collect()
    }

    fn queued(&self, id: SessionId) -> Vec<String> {
        self.engine
            .world
            .session(id)
            .unwrap()
            .pending()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn delay(&mut self, id: SessionId, ticks: u8) {
        self.handle(EngineInput::Custom {
            session: Some(id),
            run: Box::new(move |w| add_delay(w, id, ticks)),
        });
    }
}

/// The tiny world with a text exit (`borrow skiff` / `go skiff` / `row skiff`, like
/// Newhaven Docks' exit to the Small Pier, WCCMSG 8564) from the start room to the
/// second room.
fn snapshot() -> silvermere_engine::WorldSnapshot {
    let mut s = tiny_snapshot();
    let [start, second, ..] = special_rooms();
    for r in &mut s.rooms {
        if r.key == RoomKey::new(1, start) {
            r.exits[0].target = RoomId(second);
            r.exits[0].kind = ExitType::Text;
            r.exits[0].par1 = 8564;
        }
    }
    s.messages.push(Message {
        number: MessageNo(8564),
        msgs: ["borrow skiff".into(), "go skiff".into(), "row skiff".into()],
    });
    s
}

/// The test palette's prompt under `statline on` at 10 hit points.
const PROMPT: &str = "\x1b[79D\x1b[K\x1b[0;37m[HP=10\x1b[0;37m]:";
/// `ANSI_NORMAL` before every command routine.
const NORMAL: &str = "\x1b[0;37;40m";
const SLOW_DOWN: &str = "Why don't you slow down for a few seconds?\r\n";
const TOO_FAST: &str = "You are typing too quickly - command ignored\r\n";
const NO_EFFECT: &str = "Your command had no effect.\r\n";

// add_delayed_command: the walk counts links; the 8th link (nine lines
// already waiting) warns, the 12th (thirteen waiting) drops the line with both texts.
#[test]
fn flood_control_warns_at_the_tenth_line_and_drops_from_the_fourteenth() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    h.delay(id, 100);
    for i in 1..=9 {
        let mark = h.log.len();
        h.line(id, &format!("version {i}"));
        assert_eq!(h.text_since(mark, id), "", "line {i} queues silently");
    }
    assert_eq!(h.queued(id).len(), 9);
    for i in 10..=13 {
        let mark = h.log.len();
        h.line(id, &format!("version {i}"));
        assert_eq!(
            h.text_since(mark, id),
            SLOW_DOWN,
            "line {i} warns and queues"
        );
        assert_eq!(h.queued(id).len(), i);
    }
    let mark = h.log.len();
    h.line(id, "version 14");
    assert_eq!(h.text_since(mark, id), format!("{SLOW_DOWN}{TOO_FAST}"));
    assert_eq!(h.queued(id).len(), 13, "the fourteenth line is dropped");
    let mark = h.log.len();
    h.line(id, "version 15");
    assert_eq!(h.text_since(mark, id), format!("{SLOW_DOWN}{TOO_FAST}"));
    assert_eq!(h.queued(id).len(), 13);
    insta::assert_snapshot!("flood_stream", h.text_since(0, id));
}

// A line typed during a delay of
// three ticks waits, unshown, and runs on the third tick, echoed as if typed then, with
// the input echo back on.
#[test]
fn a_delay_of_three_ticks_holds_a_line_until_the_third_tick() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    let mark = h.log.len();
    h.delay(id, 3);
    assert_eq!(h.echoes_since(mark, id), vec![EchoMode::Off]);
    let mark = h.log.len();
    h.line(id, "version");
    assert_eq!(h.text_since(mark, id), "");
    assert_eq!(h.queued(id), vec!["version".to_string()]);
    h.ticks(2);
    assert_eq!(h.text_since(mark, id), "", "ticks 1 and 2 hold the line");
    h.ticks(1);
    let t = h.text_since(mark, id);
    assert!(
        t.starts_with(&format!("version\r\n{NORMAL}\x1b[0;37mMajorMUD v ")),
        "the line is echoed, then runs: {t:?}"
    );
    assert!(t.ends_with(PROMPT), "the prompt follows: {t:?}");
    assert!(!commands_pending(&h.engine.world, id));
    assert_eq!(h.echoes_since(mark, id), vec![EchoMode::On]);
}

// With no delay running, the queue drains one line per fast tick,
// in typed order, and a line typed while lines wait joins the queue.
#[test]
fn queued_lines_run_one_per_tick_in_order() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    h.delay(id, 1);
    h.line(id, "version");
    h.line(id, "quiet");
    h.line(id, "talk fast");
    assert_eq!(h.queued(id), vec!["version", "quiet", "talk fast"]);
    let mark = h.log.len();
    h.ticks(1);
    let t = h.text_since(mark, id);
    assert!(t.starts_with("version\r\n"), "{t:?}");
    assert!(!t.contains("quiet"), "{t:?}");
    assert_eq!(h.queued(id), vec!["quiet", "talk fast"]);
    // a fourth line typed now goes behind the two still waiting
    h.line(id, "verbose");
    assert_eq!(h.queued(id), vec!["quiet", "talk fast", "verbose"]);
    let mark = h.log.len();
    h.ticks(1);
    assert_eq!(
        h.text_since(mark, id),
        format!("quiet\r\n{NORMAL}Quiet mode set\r\n{PROMPT}")
    );
    let mark = h.log.len();
    h.ticks(1);
    assert_eq!(
        h.text_since(mark, id),
        format!("talk fast\r\n{NORMAL}{PROMPT}")
    );
    let plr = h.engine.world.session(id).unwrap().player().unwrap();
    assert_eq!(plr.fasttalk, 1);
    let mark = h.log.len();
    h.ticks(1);
    assert_eq!(
        h.text_since(mark, id),
        format!("verbose\r\n{NORMAL}Verbose mode set\r\n{PROMPT}")
    );
    assert!(!commands_pending(&h.engine.world, id));
    assert_eq!(
        h.engine
            .world
            .session(id)
            .unwrap()
            .player()
            .unwrap()
            .verbose,
        1
    );
}

// A delay that runs out with nothing waiting shows the prompt again.
#[test]
fn a_delay_ending_with_nothing_queued_reprompts() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    h.delay(id, 2);
    let mark = h.log.len();
    h.ticks(1);
    assert_eq!(h.text_since(mark, id), "");
    h.ticks(1);
    assert_eq!(h.text_since(mark, id), PROMPT);
    assert_eq!(h.echoes_since(mark, id), vec![EchoMode::On]);
}

// An empty line shows the room again only under the full status line
// (modes 2 and 4), then the prompt; ON, OFF and CUSTOM show
// only the prompt.
#[test]
fn an_empty_line_gives_the_prompt_and_the_room_only_under_full() {
    let mut h = Harness::new(|_| {});
    for mode in [
        StatlineMode::On,
        StatlineMode::Off,
        StatlineMode::Custom,
        StatlineMode::Full,
        StatlineMode::FullCustom,
    ] {
        let id = h.playing("Aldric", mode);
        let mark = h.log.len();
        h.line(id, "");
        let t = h.text_since(mark, id);
        if matches!(mode, StatlineMode::Full | StatlineMode::FullCustom) {
            // the short room display (T-025), then the prompt
            assert!(
                t.starts_with("\x1b[79D\x1b[K\x1b[1;36mRoom 1\r\n")
                    && t.contains("Obvious exits: NONE!!!\r\n\x1b[79D\x1b[K\x1b[0;37m"),
                "{mode:?}: {t:?}"
            );
        } else {
            assert!(t.starts_with("\x1b[79D\x1b[K\x1b[0;37m"), "{mode:?}: {t:?}");
            assert!(
                !t.contains("\r\n"),
                "{mode:?}: nothing before the prompt: {t:?}"
            );
        }
        h.engine.world.sessions.remove(&id);
        h.engine.world.ids.free(id);
    }
}

// The gate over all 138 codes; exactly the 24 listed pass, JOIN does not.
#[test]
fn the_mortally_wounded_gate_lets_exactly_the_listed_commands_through() {
    let mut world = common::tiny_world(4);
    // SUICIDE and REROLL pass the gate; in a tournament they refuse instead of deleting
    // the character (T-068)
    world.config.tournament_play = true;
    let id = enter(&mut world, "Aldric", special_rooms()[0], MessageLevel::High);
    let mut refused = Vec::new();
    let mut passed = Vec::new();
    for cmd in Cmd::ALL.iter().copied() {
        if cmd == Cmd::None {
            continue;
        }
        for wounded in [(0, false), (10, true)] {
            {
                let plr = world.session_mut(id).unwrap().character.as_mut().unwrap();
                plr.hp = wounded.0;
                plr.status.set(PlayerStatus::PARALYSED, wounded.1);
            }
            world.session_mut(id).unwrap().output.clear();
            let handled = dispatch(&mut world, id, cmd, &CommandLine::parse("word"));
            let items = world.session(id).unwrap().output.items().to_vec();
            let gated = items.iter().any(|i| {
                matches!(
                    i,
                    Item::Msg {
                        msg: Msg::MortallyWounded,
                        ..
                    }
                )
            });
            if gated {
                assert_eq!(items.len(), 1, "{cmd:?}: the refusal is all that prints");
                refused.push(cmd);
            } else if handled == Handled::NotCommand || cmd == Cmd::Search {
                // the word is not (yet) a command, or its tell_room empties the output
                // buffer: the dispatcher's ANSI_NORMAL never reaches the player
                passed.push(cmd);
            } else {
                assert!(
                    items.first().is_some_and(|i| matches!(
                        i,
                        Item::Msg {
                            msg: Msg::CmdNormal,
                            ..
                        }
                    )),
                    "{cmd:?}: ANSI_NORMAL precedes a command that runs"
                );
                passed.push(cmd);
            }
        }
    }
    refused.dedup();
    passed.dedup();
    assert_eq!(passed.len(), 24);
    assert_eq!(refused.len(), 137 - 24);
    for cmd in WOUNDED_ALLOWED {
        assert!(passed.contains(&cmd), "{cmd:?} passes");
    }
    assert!(refused.contains(&Cmd::Join));
    // Healthy and unparalysed, every command runs.
    {
        let plr = world.session_mut(id).unwrap().character.as_mut().unwrap();
        plr.hp = 10;
        plr.status.remove(PlayerStatus::PARALYSED);
    }
    world.session_mut(id).unwrap().output.clear();
    dispatch(&mut world, id, Cmd::Join, &CommandLine::parse("join bob"));
    assert!(
        !world
            .session(id)
            .unwrap()
            .output
            .items()
            .iter()
            .any(|i| matches!(
                i,
                Item::Msg {
                    msg: Msg::MortallyWounded,
                    ..
                }
            ))
    );
}

// The mortally-wounded gate through the pipeline: the refusal counts as handled and the
// prompt follows.
#[test]
fn a_wounded_player_is_refused_with_the_exact_line() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    h.engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .hp = 0;
    let mark = h.log.len();
    h.line(id, "join bob");
    let t = h.text_since(mark, id);
    assert_eq!(
        t,
        "\x1b[1;31;40mYou may not do that while you are mortally wounded!\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=\x1b[1;31m0\x1b[0;37m]:"
    );
}

// A line that is no command is speech; without talk fast nobody heard
// it; with the talk character the room hears it; the yell character yells.
#[test]
fn speech_and_no_effect() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    let other = h.playing("Bob", StatlineMode::On);
    let mark = h.log.len();
    h.line(id, "hello there");
    assert_eq!(
        h.text_since(mark, id),
        format!("\x1b[0;37m{NO_EFFECT}{PROMPT}")
    );
    assert_eq!(h.text_since(mark, other), "");
    let mark = h.log.len();
    h.line(id, ".hi all");
    assert!(
        h.text_since(mark, other)
            .starts_with("\x1b[79D\x1b[K\x1b[0;32mAldric says \"hi all\"\r\n"),
        "{:?}",
        h.text_since(mark, other)
    );
    assert!(
        h.text_since(mark, id)
            .starts_with("\x1b[0;32mYou say \"hi all\"\r\n"),
        "{:?}",
        h.text_since(mark, id)
    );
    // talk fast: any plain line is said
    h.line(id, "talk fast");
    let mark = h.log.len();
    h.line(id, "hello there");
    assert!(
        h.text_since(mark, other)
            .contains("Aldric says \"hello there\"")
    );
    assert!(h.text_since(mark, id).contains("You say \"hello there\""));
    // a yell reaches the room and, with its direction, the room beyond the text exit
    let far = enter(
        &mut h.engine.world,
        "Cara",
        special_rooms()[1],
        MessageLevel::High,
    );
    let mark = h.log.len();
    h.line(id, "\"help");
    assert!(
        h.text_since(mark, other)
            .contains("From Aldric (Yelling): \"help\"")
    );
    assert!(
        h.text_since(mark, far)
            .contains("Someone yells from the s \x08outh \"help\""),
        "{:?}",
        h.text_since(mark, far)
    );
    assert!(h.text_since(mark, id).contains("You yell \"help\""));
    // mute: nothing is said
    h.engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .status |= PlayerStatus::MUTE;
    let mark = h.log.len();
    h.line(id, "hello");
    assert_eq!(
        h.text_since(mark, id),
        format!("\x1b[1;31;40mYou cannot speak!\r\n{PROMPT}")
    );
    assert_eq!(h.text_since(mark, other), "");
}

// One of the exit's three texts, as the whole line, is the exit; held
// and stunned players are refused; the line is dealt with either way (no speech).
#[test]
fn a_text_exit_takes_the_whole_line() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    let mark = h.log.len();
    // BORROW is a command word, and cmd_borrow always answers 0, so the
    // line reaches the exits (and the output buffer holding the dispatcher's ANSI_NORMAL is
    // dropped, as the room-walk capture shows). move_user (T-026) walks the player:
    // ANSI_NORMAL after the room messages, the new room, and the prompt a
    // typed exit ends with; nothing is said.
    h.line(id, "Borrow Skiff");
    let out = h.text_since(mark, id);
    assert!(
        out.starts_with(&format!("{NORMAL}\x1b[79D\x1b[K\x1b[1;36mRoom 2\r\n")),
        "{out:?}"
    );
    assert!(out.ends_with(PROMPT), "{out:?}");
    assert!(!out.contains("says"), "{out:?}");
    // he is in the second room now; back to the first for the refusals below
    let [start, second, ..] = special_rooms();
    let plr = h
        .engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap();
    assert_eq!(plr.room, RoomId(second));
    plr.room = RoomId(start);
    h.engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .status2 |= PlayerStatus2::HELD;
    let mark = h.log.len();
    h.line(id, "row skiff");
    assert_eq!(
        h.text_since(mark, id),
        "You can't seem to move anywhere!\r\n"
    );
    let plr = h
        .engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap();
    plr.status2.remove(PlayerStatus2::HELD);
    plr.status2 |= PlayerStatus2::KNOCKEDDOWN;
    let mark = h.log.len();
    h.line(id, "go skiff");
    assert_eq!(
        h.text_since(mark, id),
        "You are too stunned to move anywhere!\r\n"
    );
    // not the whole line: speech
    let mark = h.log.len();
    h.line(id, "borrow skiff now");
    assert!(
        h.text_since(mark, id).contains(NO_EFFECT),
        "{:?}",
        h.text_since(mark, id)
    );
    // the dead do not use exits: speech. (`borrow skiff` at hp 0 is the
    // BORROW command's mortally-wounded refusal first.)
    h.engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .hp = 0;
    let mark = h.log.len();
    h.line(id, "row skiff");
    assert!(
        h.text_since(mark, id).contains(NO_EFFECT),
        "{:?}",
        h.text_since(mark, id)
    );
    let mark = h.log.len();
    h.line(id, "borrow skiff");
    assert!(h.text_since(mark, id).contains("mortally wounded"));
}

// commands::input::on_key: a key in character mode is the stat editor's (T-039); a paged
// listing is stopped by a one-word `X` line (listing::on_line,
// T-027), not by a key, which changes nothing here.
#[test]
fn a_key_in_character_mode_during_a_listing_does_not_end_it() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    {
        let s = h.engine.world.session_mut(id).unwrap();
        s.state = SessionState::Listing;
        s.ust.flags |= SessionFlags::TOPTEN | SessionFlags::CHARMODE;
    }
    let mark = h.log.len();
    h.handle(EngineInput::Key { id, byte: b'x' });
    assert_eq!(h.text_since(mark, id), "");
    let s = h.engine.world.session(id).unwrap();
    assert_eq!(s.state, SessionState::Listing);
    assert!(s.ust.flags.contains(SessionFlags::TOPTEN));
}

// A display command with an argument, and a code with no
// routine, fall through to speech.
#[test]
fn display_commands_with_arguments_and_unrouted_codes_are_speech() {
    let mut h = Harness::new(|_| {});
    let id = h.playing("Aldric", StatlineMode::On);
    let mark = h.log.len();
    h.line(id, "status now");
    assert!(h.text_since(mark, id).contains(NO_EFFECT));
    let mark = h.log.len();
    h.line(id, "throw rock");
    assert!(h.text_since(mark, id).contains(NO_EFFECT));
    let mark = h.log.len();
    h.line(id, "version");
    assert!(h.text_since(mark, id).contains("MajorMUD v "));
}
