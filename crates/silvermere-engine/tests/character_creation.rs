//! Character creation end to end (T-022): the race and class lists, the lawful question,
//! the line-mode stat editor, the name check and the suicide password prompts, driven
//! through the engine one line at a time.
//!
//! The world is the tiny fixture plus the races and classes the tests name: Kang
//! (WCCRACE 11) and Witchunter (WCCCLASS 2) of the fixed test sheet, and two known
//! monsters, `giant rat` and `Kobold`. The registry regexes for every printed line are
//! checked in `silvermere-text/tests/patterns_snapshot.rs` (each catalogue entry names its
//! pattern); here the lines are compared with the literal texts.

mod common;

use std::sync::mpsc::Receiver;
use std::time::Instant;

use silvermere_engine::player_session::{AccountRef, Caps};
use silvermere_engine::state::newchar::{self, suicide_pw, validate_name};
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, PersistRequest, SeededRng, SessionId, World, session,
};
use silvermere_rules::progression::cp_cost;
use silvermere_text::cp437;
use silvermere_text::strip::strip_ansi;
use silvermere_types::enums::{MessageLevel, NewCharStep, SessionState, SuicideStep};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{AccountId, ClassNo, MapId, MonsterNo, RaceNo, RoomId};

use common::TestClock;

/// Kang (WCCRACE 11): min I30 W45 S55 H50 A30 C30, max I90 W110 S120 H110 A90 C85,
/// base CP 100 (the stock race).
fn kang() -> silvermere_types::records::Race {
    let mut r = blank::clear_race(RaceNo(11));
    r.name = "Kang".into();
    r.minint = 30;
    r.minwil = 45;
    r.minstr = 55;
    r.minhea = 50;
    r.minagi = 30;
    r.mincha = 30;
    r.maxint = 90;
    r.maxwil = 110;
    r.maxstr = 120;
    r.maxhea = 110;
    r.maxagi = 90;
    r.maxcha = 85;
    r.basecp = 100;
    r
}

fn race(n: u16, name: &str) -> silvermere_types::records::Race {
    let mut r = blank::clear_race(RaceNo(n));
    r.name = name.into();
    for m in [
        &mut r.minint,
        &mut r.minwil,
        &mut r.minstr,
        &mut r.minhea,
        &mut r.minagi,
        &mut r.mincha,
    ] {
        *m = 40;
    }
    for m in [
        &mut r.maxint,
        &mut r.maxwil,
        &mut r.maxstr,
        &mut r.maxhea,
        &mut r.maxagi,
        &mut r.maxcha,
    ] {
        *m = 100;
    }
    r.basecp = 80;
    r
}

fn class(n: u16, name: &str, hplevel: i16) -> silvermere_types::records::Class {
    let mut c = blank::clear_class(ClassNo(n));
    c.name = name.into();
    c.hplevel = hplevel;
    c
}

/// The tiny world with the stock-like races, classes and two monsters.
fn snapshot() -> silvermere_engine::WorldSnapshot {
    let mut s = common::tiny_snapshot();
    s.races = vec![
        race(1, "Human"),
        race(2, "Dwarf"),
        race(3, "Gnome"),
        race(4, "Halfling"),
        race(5, "Elf"),
        race(6, "Half-Elf"),
        race(7, "Dark-Elf"),
        race(8, "Half-Orc"),
        race(9, "Goblin"),
        race(10, "Half-Ogre"),
        kang(),
        race(12, "Nekojin"),
    ];
    s.classes = vec![
        class(1, "Warrior", 10),
        class(2, "Witchunter", 4),
        class(10, "Mage", 2),
    ];
    let mut rat = blank::clear_known_monster(MonsterNo(1));
    rat.name = "giant rat".into();
    let mut kobold = blank::clear_known_monster(MonsterNo(2));
    kobold.name = "Kobold".into();
    s.monsters = vec![rat, kobold];
    s
}

type TestEngine = Engine<TestClock, SeededRng>;

fn new_engine(max_sessions: u16) -> (TestEngine, Receiver<EngineOutput>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let world = World::load(common::test_config(), max_sessions, snapshot()).expect("loads");
    let engine = Engine::new(world, TestClock::new(), SeededRng::new(1), tx);
    (engine, rx)
}

/// A connected session with an account, at the race list (the menu's `E` with no
/// character). Returns the id and what the player received.
fn start(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    username: &str,
) -> (SessionId, Vec<u8>) {
    let now = Instant::now();
    let name = username.to_string();
    let id = SessionId(engine.world.ids.in_use() as u16);
    engine.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            // 79 columns, as the creation-line-mode capture: the line-mode editor
            // (the full-screen one is tests/fullscreen_editor.rs, T-063)
            let caps = Caps {
                columns: 79,
                ..Caps::default()
            };
            let got = session::on_connect(w, caps, now).unwrap();
            assert_eq!(got, id);
            w.session_mut(id).unwrap().account = Some(AccountRef {
                id: AccountId(7),
                username: name,
                sysop: false,
                gameop: false,
                sex: b'M',
            });
            newchar::begin(w, id, 0);
        }),
    });
    let bytes = writes(&common::drain(rx), id);
    (id, bytes)
}

/// One typed line; what the session received for it.
fn line(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    text: &str,
) -> Vec<u8> {
    engine.handle(EngineInput::Line {
        id,
        text: text.to_string(),
    });
    writes(&common::drain(rx), id)
}

/// The same, keeping every output.
fn line_all(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    text: &str,
) -> Vec<EngineOutput> {
    engine.handle(EngineInput::Line {
        id,
        text: text.to_string(),
    });
    common::drain(rx)
}

/// The DB's answer to `NameCheck`.
fn name_checked(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    taken: bool,
) -> Vec<EngineOutput> {
    engine.handle(EngineInput::Custom {
        session: Some(id),
        run: Box::new(move |w| validate_name::on_checked(w, id, taken)),
    });
    common::drain(rx)
}

fn writes(outs: &[EngineOutput], id: SessionId) -> Vec<u8> {
    outs.iter()
        .filter_map(|o| match o {
            EngineOutput::Write { id: i, bytes } if *i == id => Some(bytes.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

/// The bytes as text: ANSI stripped, CP437 decoded.
fn plain(bytes: &[u8]) -> String {
    cp437::decode(&strip_ansi(bytes))
}

fn escape(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|&b| std::ascii::escape_default(b))
        .map(char::from)
        .collect()
}

fn state(engine: &TestEngine, id: SessionId) -> SessionState {
    engine.world.session(id).unwrap().state
}

fn name_check(outs: &[EngineOutput]) -> Option<(SessionId, String)> {
    outs.iter().find_map(|o| match o {
        EngineOutput::NameCheck { id, name } => Some((*id, name.clone())),
        _ => None,
    })
}

/// Through race, class and the lawful question to the editor's first prompt.
fn to_editor(engine: &mut TestEngine, rx: &Receiver<EngineOutput>) -> SessionId {
    let (id, _) = start(engine, rx, "KASIMIR");
    line(engine, rx, id, "11");
    line(engine, rx, id, "2");
    let out = line(engine, rx, id, "y");
    assert!(
        plain(&out).contains("You have 100 CP to distribute."),
        "{}",
        plain(&out)
    );
    assert_eq!(state(engine, id), SessionState::NewChar(NewCharStep::Stats));
    id
}

/// The §0 stats and looks; the answer to the SAVE prompt is left to the caller.
fn fill_kasimir(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    last: &str,
) -> Vec<u8> {
    let mut all = Vec::new();
    for answer in [
        "Kasimir", last, "79", "30", "45", "42", "74", "30", "short", "black", "hazel",
    ] {
        all.extend(line(engine, rx, id, answer));
    }
    all
}

// display_race_list, display_class_list; WCCTEXT CHSRACE/CHSCLASS
#[test]
fn race_and_class_lists() {
    let (mut engine, rx) = new_engine(4);
    let (id, race_list) = start(&mut engine, &rx, "KASIMIR");
    let text = plain(&race_list);
    // clear_player zeroed the record, `dragging` too; no load_player reads another one before
    // he plays, so his first marched step is refused
    let sess = engine.world.session(id).unwrap();
    assert_eq!(sess.party.dragging, Some(silvermere_engine::SessionId(0)));
    assert!(sess.party.loaded);
    assert_eq!(sess.player().unwrap().dragging, 0);
    assert!(text.starts_with("Please choose a race from the following list:\r\n"));
    assert!(
        text.contains("[1]  Human\r\n"),
        "two blanks after a one-digit number"
    );
    assert!(text.contains("[11] Kang\r\n"), "one blank from 10 up");
    assert!(text.ends_with("Please choose your race [ ? for help ] : "));
    // the row's bytes
    assert!(
        race_list
            .windows(40)
            .any(|w| w.starts_with(b"\x1b[1;30m[\x1b[1;37m1\x1b[1;30m]\x1b[0;37m  Human\r\n")),
        "{}",
        escape(&race_list)
    );
    insta::assert_snapshot!("race_list", escape(&race_list));
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Race));
    let class_list = line(&mut engine, &rx, id, "11");
    let text = plain(&class_list);
    assert!(text.starts_with("Please choose a class from the following list:\r\n"));
    assert!(text.contains("[2]  Witchunter\r\n"));
    assert!(text.contains("[10] Mage\r\n"));
    assert!(text.ends_with("Please choose your class [ ? for help ] : "));
    insta::assert_snapshot!("class_list", escape(&class_list));
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Class)
    );
    // WCCTEXT GOODTEXT + GOODPMPT
    let lawful = line(&mut engine, &rx, id, "2");
    let text = plain(&lawful);
    assert!(text.contains("You must now choose if you want to be a truly 'lawful' citizen"));
    assert!(text.ends_with("Do you want to be Lawful?  [Yes/No] "));
    insta::assert_snapshot!("lawful_prompt", escape(&lawful));
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Lawful)
    );
}

// the refusals reprint exactly
#[test]
fn invalid_and_empty_answers_at_the_lists_and_the_lawful_question() {
    let (mut engine, rx) = new_engine(4);
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    let out = line(&mut engine, &rx, id, "");
    assert_eq!(plain(&out), "You must choose a race. [ ? for help ]");
    insta::assert_snapshot!("must_choose_race", escape(&out));
    let out = line(&mut engine, &rx, id, "99");
    assert_eq!(plain(&out), "You must choose a valid race. [ ? for help ]");
    let out = line(&mut engine, &rx, id, "abc");
    assert_eq!(plain(&out), "You must choose a valid race. [ ? for help ]");
    // `?` prints the list again
    let out = line(&mut engine, &rx, id, "?");
    assert!(plain(&out).contains("[11] Kang"));
    assert!(
        !plain(&out).contains("Please choose your race"),
        "the prompt is not reprinted"
    );
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Race));
    line(&mut engine, &rx, id, "11");
    let out = line(&mut engine, &rx, id, "");
    assert_eq!(plain(&out), "You must choose a class. [ ? for help ]");
    assert!(out.starts_with(b"\x1b[1;37m"), "ANSI_WHITE first");
    insta::assert_snapshot!("must_choose_class", escape(&out));
    let out = line(&mut engine, &rx, id, "7");
    assert_eq!(plain(&out), "You must choose a valid class. [ ? for help ]");
    insta::assert_snapshot!("must_choose_valid_class", escape(&out));
    line(&mut engine, &rx, id, "2");
    for bad in ["", "maybe", "y n", "yes please"] {
        let out = line(&mut engine, &rx, id, bad);
        assert_eq!(
            plain(&out),
            // GOODPMPT itself starts with a line end (the .MSG text)
            "You must respond with a Y or an N.\r\n\r\nDo you want to be Lawful?  [Yes/No] ",
            "{bad:?}"
        );
        assert_eq!(
            state(&engine, id),
            SessionState::NewChar(NewCharStep::Lawful)
        );
    }
    insta::assert_snapshot!(
        "lawful_y_or_n",
        escape(&line(&mut engine, &rx, id, "maybe"))
    );
}

// Y forces lawful for life, N takes the force off
#[test]
fn lawful_answer_is_stored() {
    let (mut engine, rx) = new_engine(4);
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "11");
    line(&mut engine, &rx, id, "2");
    line(&mut engine, &rx, id, "Yes");
    let plr = engine.world.session(id).unwrap().player().unwrap().clone();
    assert_eq!((plr.lawful, plr.evil), (-10, -51));
    let (id, _) = start(&mut engine, &rx, "BRYNJA");
    line(&mut engine, &rx, id, "11");
    line(&mut engine, &rx, id, "2");
    line(&mut engine, &rx, id, "n");
    let plr = engine.world.session(id).unwrap().player().unwrap().clone();
    assert_eq!((plr.lawful, plr.evil), (0, 0));
}

// without ENABLAWF the editor opens straight after the class
#[test]
fn no_lawful_question_when_disabled() {
    let (mut engine, rx) = new_engine(4);
    engine.world.config.enable_lawful = false;
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "11");
    let out = line(&mut engine, &rx, id, "2");
    assert!(plain(&out).contains("Character Generator"));
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Stats)
    );
}

// edit_character_stats, the line-mode preamble; roll_stats
#[test]
fn editor_preamble() {
    let (mut engine, rx) = new_engine(4);
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "11");
    line(&mut engine, &rx, id, "2");
    let out = line(&mut engine, &rx, id, "y");
    let text = plain(&out);
    assert!(text.starts_with("\r\n     M A J O R   M U D\r\n\r\n     Character Generator\r\n\r\n+-    Point Cost Chart    -+\r\n"));
    assert!(text.contains("| 3rd 10 points: 3 CP each |\r\n"));
    assert!(text.contains("| +10 to base stat:  10 CP |\r\n"));
    assert!(text.contains("| +40 to base stat: 100 CP |\r\n"));
    // (sic) the maximum prints as 0, as in the original
    assert!(text.contains("Strength  55 - 0\r\nIntellect 30 - 0\r\nWillpower 45 - 0\r\nAgility   30 - 0\r\nHealth    50 - 0\r\nCharm     30 - 0\r\n"));
    assert!(text.contains("\r\nYou have 100 CP to distribute.\r\n"));
    assert!(text.ends_with("Given name [KASIMIR]: "));
    insta::assert_snapshot!("editor_preamble", escape(&out));
    // roll_stats: the stats start at the race minimums, cp = totalcp = basecp
    let plr = engine.world.session(id).unwrap().player().unwrap().clone();
    assert_eq!(
        [plr.str, plr.intel, plr.wil, plr.agi, plr.hea, plr.cha],
        [55, 30, 45, 30, 50, 30]
    );
    assert_eq!([plr.basstr, plr.basint], [55, 30]);
    assert_eq!((plr.cp, plr.totalcp), (100, 100));
    assert_eq!(plr.randomhp, 4, "the class's hplevel");
}

// stat validation: over budget, under the minimum, over the maximum
#[test]
fn stat_refusals_in_the_editor() {
    let (mut engine, rx) = new_engine(4);
    let id = to_editor(&mut engine, &rx);
    line(&mut engine, &rx, id, "Kasimir");
    let out = line(&mut engine, &rx, id, "Stonefist");
    assert_eq!(
        plain(&out),
        "\r\nYou have 100 CP to distribute.\r\nStrength (55-120) [55]: "
    );
    let under = line(&mut engine, &rx, id, "40");
    assert_eq!(
        plain(&under),
        "Strength may not be lower than 55.\r\nStrength (55-120) [55]: "
    );
    insta::assert_snapshot!("stat_under_minimum", escape(&under));
    let over = line(&mut engine, &rx, id, "121");
    assert_eq!(
        plain(&over),
        "Strength may not be higher than 120.\r\nStrength (55-120) [55]: "
    );
    insta::assert_snapshot!("stat_over_maximum", escape(&over));
    let budget = line(&mut engine, &rx, id, "110");
    assert_eq!(
        plain(&budget),
        "You may not assign that much to Strength\r\nStrength (55-120) [55]: "
    );
    insta::assert_snapshot!("stat_over_budget", escape(&budget));
    // accepted: the CP line is reprinted
    let ok = line(&mut engine, &rx, id, "79");
    assert_eq!(
        plain(&ok),
        "\r\nYou have 58 CP to distribute.\r\nIntellect (30-90) [30]: "
    );
    insta::assert_snapshot!("stat_accepted", escape(&ok));
    // the budget is what is left after the other stats: Health 74 costs 42, 16 left
    for a in ["30", "45", "42"] {
        line(&mut engine, &rx, id, a);
    }
    let out = line(&mut engine, &rx, id, "74");
    assert_eq!(
        plain(&out),
        "\r\nYou have 2 CP to distribute.\r\nCharm (30-85) [30]: "
    );
    let out = line(&mut engine, &rx, id, "33");
    assert_eq!(
        plain(&out),
        "You may not assign that much to Charm\r\nCharm (30-85) [30]: "
    );
}

// finishing creation and the name-validation poll: the happy path
#[test]
fn happy_path_creates_a_kang_witchunter() {
    let (mut engine, rx) = new_engine(4);
    let ann = common::enter(
        &mut engine.world,
        "Ann",
        common::special_rooms()[0],
        MessageLevel::High,
    );
    let id = to_editor(&mut engine, &rx);
    let out = fill_kasimir(&mut engine, &rx, id, "Stonefist");
    assert!(plain(&out).ends_with("SAVE or QUIT? [SAVE]: "));
    // SAVE: the suicide notice, then the name check starts
    let outs = line_all(&mut engine, &rx, id, "");
    let text = plain(&writes(&outs, id));
    assert_eq!(
        text,
        "\r\nTo prevent accidental suicide or reroll, these commands\r\nhave been password protected.  You have not yet entered\r\nyour suicide password, so please do so soon using the\r\nSET SUICIDE command.\r\nPlease wait - Validating your name.\r\n "
    );
    insta::assert_snapshot!("save_suicide_notice_validating", escape(&writes(&outs, id)));
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Validating)
    );
    assert_eq!(name_check(&outs), Some((id, "Kasimir".to_string())));
    assert!(
        !outs.iter().any(|o| matches!(o, EngineOutput::Persist(_))),
        "not saved before validation"
    );
    // typing while the check runs
    let out = line(&mut engine, &rx, id, "look");
    assert_eq!(plain(&out), "Validating your name, please wait.\r\n ");
    // the DB says the name is free
    let outs = name_checked(&mut engine, &rx, id, false);
    assert_eq!(state(&engine, id), SessionState::Playing);
    assert!(engine.world.is_in_game(id));
    let mine = plain(&writes(&outs, id));
    // the secondary-stat recalculation on entry (T-023) has
    // replaced `newplrhp` with the rolled maximum, and refilled.
    assert!(mine.contains("[HP=42]:"), "{mine:?}");
    assert!(
        mine.starts_with('\u{8}'),
        "the line starts with a backspace"
    );
    // the other player hears it, with a prompt
    let theirs = plain(&writes(&outs, ann));
    assert!(
        theirs.starts_with("Kasimir just entered the Realm.\r\n"),
        "{theirs:?}"
    );
    assert!(theirs.contains("[HP=10]:"));
    // save_player: the first save
    let saved = outs
        .iter()
        .find_map(|o| match o {
            EngineOutput::Persist(PersistRequest::Character { account, character }) => {
                Some((*account, character.as_ref().clone()))
            }
            _ => None,
        })
        .expect("a save request");
    let plr = engine.world.session(id).unwrap().player().unwrap().clone();
    assert_eq!(saved.0, AccountId(7));
    assert_eq!(saved.1, plr, "the saved record is the live one");
    // the §0 character: 98 of 100 CP spent
    assert_eq!(plr.firstname, "Kasimir");
    assert_eq!(plr.lastname, "Stonefist");
    assert_eq!(plr.userid, "KASIMIR");
    assert_eq!(plr.race, RaceNo(11));
    assert_eq!(plr.class, ClassNo(2));
    assert_eq!(
        [plr.str, plr.intel, plr.wil, plr.agi, plr.hea, plr.cha],
        [79, 30, 45, 42, 74, 30]
    );
    assert_eq!(
        [
            plr.basstr, plr.basint, plr.baswil, plr.basagi, plr.bashea, plr.bascha
        ],
        [79, 30, 45, 42, 74, 30]
    );
    assert_eq!((plr.cp, plr.totalcp), (2, 100));
    assert_eq!((plr.hairlen, plr.haircol, plr.eyecol), (1, 0, 13));
    assert_eq!((plr.lawful, plr.evil), (-10, -51));
    // the new character state (create_player)
    assert_eq!(plr.level, 1);
    assert_eq!(plr.exp, 0);
    assert_eq!(plr.coins, Default::default());
    assert_eq!(plr.lives, 9);
    assert_eq!(plr.map, MapId(1));
    assert_eq!(plr.room, RoomId(common::test_config().start_room));
    assert_eq!(plr.statline, silvermere_types::enums::StatlineMode::Full);
    assert_eq!(plr.verbose, 1);
    assert_eq!((plr.hunger, plr.thirst), (1000, 1000));
    assert_eq!((plr.maxenergy, plr.energy), (1000, 1000));
    assert!(plr.settings.contains(PlayerSettings::VALIDATED));
    assert!(engine.world.session(id).unwrap().editor.is_none());
}

// the field and SAVE: a blank last name is refused
// unless BLANK_LASTNAME_ALLOWED
#[test]
fn blank_last_name_refused_then_allowed_by_config() {
    let (mut engine, rx) = new_engine(4);
    let id = to_editor(&mut engine, &rx);
    line(&mut engine, &rx, id, "Kasimir");
    let out = line(&mut engine, &rx, id, "");
    assert_eq!(
        plain(&out),
        "You must enter a last name!\r\nFamily name []: "
    );
    insta::assert_snapshot!("last_name_required", escape(&out));
    // the SAVE-time check for a family name that is blank when SAVE runs
    for a in ["Stonefist", "", "", "", "", "", "", "", "", ""] {
        line(&mut engine, &rx, id, a);
    }
    engine.handle(EngineInput::Custom {
        session: Some(id),
        run: Box::new(move |w| {
            w.session_mut(id)
                .unwrap()
                .editor
                .as_mut()
                .unwrap()
                .family_name
                .clear();
        }),
    });
    common::drain(&rx);
    let out = line(&mut engine, &rx, id, "save");
    assert_eq!(
        plain(&out),
        "You MUST have a last name!\r\nYou will have to start over again.\r\nPress Enter."
    );
    insta::assert_snapshot!("last_name_must_start_over", escape(&out));
    assert_eq!(state(&engine, id), SessionState::MainMenu);
    assert!(
        engine.world.session(id).unwrap().player().is_none(),
        "clear_player"
    );

    let (mut engine, rx) = new_engine(4);
    engine.world.config.blank_lastname_allowed = true;
    let id = to_editor(&mut engine, &rx);
    fill_kasimir(&mut engine, &rx, id, "");
    let outs = line_all(&mut engine, &rx, id, "save");
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Validating)
    );
    assert!(name_check(&outs).is_some());
    assert_eq!(
        engine.world.session(id).unwrap().player().unwrap().lastname,
        ""
    );
}

// letters only in the family name
#[test]
fn family_name_rules() {
    let (mut engine, rx) = new_engine(4);
    let id = to_editor(&mut engine, &rx);
    line(&mut engine, &rx, id, "Kasimir");
    let out = line(&mut engine, &rx, id, "Stone-fist");
    assert_eq!(
        plain(&out),
        "Only ALPHA characters are allowed in a name!\r\nFamily name []: "
    );
    let out = line(&mut engine, &rx, id, "Abcdefghijklmnopqrs");
    assert_eq!(
        plain(&out),
        "Your family name can only be 18 characters long!\r\nFamily name []: "
    );
    let out = line(&mut engine, &rx, id, "stonefist");
    assert!(plain(&out).starts_with("\r\nYou have 100 CP to distribute.\r\n"));
    assert_eq!(
        engine
            .world
            .session(id)
            .unwrap()
            .editor
            .as_ref()
            .unwrap()
            .family_name,
        "Stonefist"
    );
}

// a given name that parses as a command bounces to the menu
#[test]
fn given_name_sysop_bounces_to_the_menu() {
    let (mut engine, rx) = new_engine(4);
    let id = to_editor(&mut engine, &rx);
    for a in ["sysop", "Stonefist", "", "", "", "", "", "", "", "", ""] {
        line(&mut engine, &rx, id, a);
    }
    let out = line(&mut engine, &rx, id, "SAVE");
    assert_eq!(
        plain(&out),
        "The name you have chosen is already in use!\r\nYou will have to start over again.\r\nPress Enter."
    );
    assert!(out.starts_with(b"\x1b[1;31m"), "ANSI_LTRED");
    insta::assert_snapshot!("given_name_is_a_command", escape(&out));
    assert_eq!(state(&engine, id), SessionState::MainMenu);
    assert!(engine.world.session(id).unwrap().player().is_none());
}

// a character already has the name; SUBST_NEW_NAME
#[test]
fn clash_with_an_existing_character_loops_to_new_name() {
    let (mut engine, rx) = new_engine(4);
    // online: found at once, no DB query
    common::enter(
        &mut engine.world,
        "Kasimir",
        common::special_rooms()[1],
        MessageLevel::High,
    );
    let id = to_editor(&mut engine, &rx);
    fill_kasimir(&mut engine, &rx, id, "Stonefist");
    let outs = line_all(&mut engine, &rx, id, "");
    assert!(name_check(&outs).is_none());
    assert!(
        !outs.iter().any(|o| matches!(o, EngineOutput::Persist(_))),
        "not saved before validation"
    );
    let text = plain(&writes(&outs, id));
    assert!(text.ends_with("Please wait - Validating your name.\r\n \u{8}You may not use Kasimir as your name.\r\nPlease enter a new name: "), "{text:?}");
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Name));
    // the new-name prompt's own checks
    let out = line(&mut engine, &rx, id, "");
    assert_eq!(plain(&out), "Please enter a new name or X to exit:");
    let out = line(&mut engine, &rx, id, "Bartholomew");
    assert_eq!(
        plain(&out),
        "Your name may only be 10 characters long.\r\nPlease try another one: "
    );
    let out = line(&mut engine, &rx, id, "north");
    assert_eq!(
        plain(&out),
        "You may not use that name.\r\nPlease try another one: "
    );
    let out = line(&mut engine, &rx, id, "br1nja");
    assert_eq!(
        plain(&out),
        "The name you have chosen is invalid!\r\nPlease try another one: "
    );
    assert!(out.starts_with(b"\x1b[1;31m"));
    // stored: found by the DB
    let outs = line_all(&mut engine, &rx, id, "brynja");
    assert_eq!(
        plain(&writes(&outs, id)),
        "Please wait - Validating your name.\r\n "
    );
    assert_eq!(name_check(&outs), Some((id, "Brynja".to_string())));
    assert!(
        !outs.iter().any(|o| matches!(o, EngineOutput::Persist(_))),
        "not saved before validation"
    );
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Validating)
    );
    let outs = name_checked(&mut engine, &rx, id, true);
    assert_eq!(
        plain(&writes(&outs, id)),
        "\u{8}You may not use Brynja as your name.\r\nPlease enter a new name: "
    );
    insta::assert_snapshot!("name_taken", escape(&writes(&outs, id)));
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Name));
    // free at last
    let outs = line_all(&mut engine, &rx, id, "Solveig");
    assert_eq!(name_check(&outs), Some((id, "Solveig".to_string())));
    assert!(
        !outs.iter().any(|o| matches!(o, EngineOutput::Persist(_))),
        "not saved before validation"
    );
    name_checked(&mut engine, &rx, id, false);
    assert_eq!(state(&engine, id), SessionState::Playing);
    assert_eq!(
        engine
            .world
            .session(id)
            .unwrap()
            .player()
            .unwrap()
            .firstname,
        "Solveig"
    );
}

// a known monster has the name
#[test]
fn clash_with_a_monster_name_loops_to_new_name() {
    let (mut engine, rx) = new_engine(4);
    let id = to_editor(&mut engine, &rx);
    // `giant rat` cannot be a given name at all: a blank is not a letter
    let out = line(&mut engine, &rx, id, "giant rat");
    assert_eq!(
        plain(&out),
        "Only ALPHA characters are allowed in a name!\r\nGiven name [KASIMIR]: "
    );
    for a in [
        "kobold",
        "Stonefist",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
    ] {
        line(&mut engine, &rx, id, a);
    }
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Validating)
    );
    let outs = name_checked(&mut engine, &rx, id, false);
    assert_eq!(
        plain(&writes(&outs, id)),
        "\u{8}You may not use Kobold as your name.\r\nPlease enter a new name: "
    );
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Name));
    // at the new-name prompt a word of the monster's name is checked the same way
    let outs = line_all(&mut engine, &rx, id, "giant rat");
    assert_eq!(name_check(&outs), Some((id, "Giant".to_string())));
}

// X and QUIT return to the menu
#[test]
fn x_at_each_stage_returns_to_the_menu() {
    let menu_head = "\u{1b}[0m\u{1b}[2J\r\n \u{1b}[1mM A J O R  M U D";
    let (mut engine, rx) = new_engine(8);
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    let out = line(&mut engine, &rx, id, "x");
    assert!(cp437::decode(&out).starts_with(menu_head), "prf_menu");
    assert!(
        out.ends_with(b"MAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m"),
        "the prompt, then ANSI_NORMAL"
    );
    assert_eq!(state(&engine, id), SessionState::MainMenu);

    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "11");
    let out = line(&mut engine, &rx, id, "Xyz");
    assert!(cp437::decode(&out).starts_with(menu_head));
    assert_eq!(state(&engine, id), SessionState::MainMenu);

    // the lawful question has no X: only Y or N
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "11");
    line(&mut engine, &rx, id, "2");
    let out = line(&mut engine, &rx, id, "x");
    assert!(plain(&out).starts_with("You must respond with a Y or an N."));
    assert_eq!(
        state(&engine, id),
        SessionState::NewChar(NewCharStep::Lawful)
    );

    // QUIT in the editor: the menu, twice (sic)
    line(&mut engine, &rx, id, "y");
    line(&mut engine, &rx, id, "");
    line(&mut engine, &rx, id, "Stonefist");
    for _ in 0..9 {
        line(&mut engine, &rx, id, "");
    }
    let out = line(&mut engine, &rx, id, "quit");
    let text = cp437::decode(&out);
    assert_eq!(text.matches(menu_head).count(), 2);
    assert_eq!(state(&engine, id), SessionState::MainMenu);
    assert!(engine.world.session(id).unwrap().editor.is_none());

    // X at the new-name prompt: the prompt line only, no character
    common::enter(
        &mut engine.world,
        "Kasimir",
        common::special_rooms()[1],
        MessageLevel::High,
    );
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "11");
    line(&mut engine, &rx, id, "2");
    line(&mut engine, &rx, id, "y");
    fill_kasimir(&mut engine, &rx, id, "Stonefist");
    line(&mut engine, &rx, id, "");
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Name));
    let out = line(&mut engine, &rx, id, "X");
    assert_eq!(
        out,
        b"\r\n\x1b[1;30m[\x1b[0mMAJORMUD\x1b[1;30m]\x1b[0m: \x1b[0;37;40m"
    );
    assert_eq!(state(&engine, id), SessionState::MainMenu);
    assert!(engine.world.session(id).unwrap().player().is_none());
}

// the game is full when the name is accepted
#[test]
fn game_full_at_validation_returns_to_the_menu() {
    let (mut engine, rx) = new_engine(2);
    common::enter(
        &mut engine.world,
        "Ann",
        common::special_rooms()[0],
        MessageLevel::High,
    );
    common::enter(
        &mut engine.world,
        "Bob",
        common::special_rooms()[0],
        MessageLevel::High,
    );
    let id = SessionId(2);
    engine.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            // a third channel beyond max_sessions, as a bigger licence would have had
            let mut s = silvermere_engine::PlayerSession::new(id, Caps::default(), Instant::now());
            s.account = Some(AccountRef {
                id: AccountId(9),
                username: "CARL".into(),
                sysop: false,
                gameop: false,
                sex: b'M',
            });
            s.character = Some(silvermere_engine::player::create_player(
                w,
                AccountId(9),
                "CARL",
                0,
            ));
            s.character.as_mut().unwrap().firstname = "Carl".into();
            s.state = SessionState::NewChar(NewCharStep::Validating);
            w.sessions.insert(id, s);
        }),
    });
    let outs = name_checked(&mut engine, &rx, id, false);
    let text = plain(&writes(&outs, id));
    assert_eq!(
        text,
        "\r\nWe're sorry, but the game is currently full.\r\n\r\n[MAJORMUD]: "
    );
    assert_eq!(state(&engine, id), SessionState::MainMenu);
    assert!(!engine.world.is_in_game(id));
    assert!(
        outs.iter().any(|o| matches!(o, EngineOutput::Persist(_))),
        "saved first"
    );
}

// cmd_set suicide; the two password arms
#[test]
fn set_suicide_password() {
    let (mut engine, rx) = new_engine(4);
    let id = common::enter(
        &mut engine.world,
        "Ann",
        common::special_rooms()[0],
        MessageLevel::High,
    );
    let set = |engine: &mut TestEngine| {
        engine.handle(EngineInput::Custom {
            session: Some(id),
            run: Box::new(move |w| suicide_pw::start(w, id)),
        });
        common::drain(&rx)
    };
    // no password yet: the new one is asked at once, echoed as `*`
    let outs = set(&mut engine);
    assert_eq!(plain(&writes(&outs, id)), "\r\nEnter New Password:");
    assert!(outs.iter().any(|o| matches!(
        o,
        EngineOutput::SetEcho {
            mode: silvermere_engine::EchoMode::Secure { ch: b'*', width: 8 },
            ..
        }
    )));
    assert_eq!(
        state(&engine, id),
        SessionState::Suicide(SuicideStep::NewPw)
    );
    let out = line(&mut engine, &rx, id, "ninechars");
    assert_eq!(
        plain(&out),
        "\r\nThe password may not be longer than 8 characters.\r\nEnter New Password:"
    );
    insta::assert_snapshot!("suicide_password_too_long", escape(&out));
    let outs = line_all(&mut engine, &rx, id, "secret");
    let text = plain(&writes(&outs, id));
    assert!(text.starts_with("\r\nPassword changed\r\n"));
    assert!(text.contains("[HP=10]:"));
    assert!(outs.iter().any(|o| matches!(
        o,
        EngineOutput::SetEcho {
            mode: silvermere_engine::EchoMode::On,
            ..
        }
    )));
    assert_eq!(state(&engine, id), SessionState::Playing);
    assert_eq!(
        engine.world.session(id).unwrap().player().unwrap().suicpw,
        "secret"
    );
    // with a password: the current one first
    let outs = set(&mut engine);
    assert_eq!(plain(&writes(&outs, id)), "Enter the current password:");
    assert_eq!(
        state(&engine, id),
        SessionState::Suicide(SuicideStep::OldPw)
    );
    let out = line(&mut engine, &rx, id, "");
    assert!(plain(&out).starts_with("\r\nPassword NOT changed\r\n"));
    assert_eq!(state(&engine, id), SessionState::Playing);
    set(&mut engine);
    let out = line(&mut engine, &rx, id, "wrong");
    assert!(plain(&out).starts_with("\r\nInvalid password!"));
    insta::assert_snapshot!("suicide_invalid_password", escape(&out));
    assert_eq!(state(&engine, id), SessionState::Playing);
    set(&mut engine);
    let out = line(&mut engine, &rx, id, "SECRET");
    assert_eq!(plain(&out), "Enter New Password:");
    assert!(out.starts_with(b"\x1b[79D\x1b[K"), "ANSI_CLRLINE");
    assert_eq!(
        state(&engine, id),
        SessionState::Suicide(SuicideStep::NewPw)
    );
    let out = line(&mut engine, &rx, id, "");
    assert!(plain(&out).starts_with("\r\nPassword NOT changed\r\n"));
    assert_eq!(
        engine.world.session(id).unwrap().player().unwrap().suicpw,
        "secret"
    );
}

// the SUICIDE password prompt
#[test]
fn suicide_confirm_checks_the_password() {
    let (mut engine, rx) = new_engine(4);
    let id = common::enter(
        &mut engine.world,
        "Ann",
        common::special_rooms()[0],
        MessageLevel::High,
    );
    // three lives, so a carried-out suicide costs one and the game goes on (T-068)
    engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .lives = 3;
    let confirm = |engine: &mut TestEngine| {
        engine.handle(EngineInput::Custom {
            session: Some(id),
            run: Box::new(move |w| {
                w.session_mut(id).unwrap().state = SessionState::Suicide(SuicideStep::Confirm);
            }),
        });
        common::drain(&rx);
    };
    // no password set: a typed one is wrong, nothing typed goes through
    confirm(&mut engine);
    let out = line(&mut engine, &rx, id, "anything");
    assert!(plain(&out).starts_with("\r\nInvalid password specified.\r\n"));
    assert!(plain(&out).contains("[HP=10]:"));
    insta::assert_snapshot!("suicide_invalid_password_specified", escape(&out));
    assert_eq!(state(&engine, id), SessionState::Playing);
    confirm(&mut engine);
    let outs = line_all(&mut engine, &rx, id, "");
    assert!(outs.iter().any(|o| matches!(
        o,
        EngineOutput::SetEcho {
            mode: silvermere_engine::EchoMode::On,
            ..
        }
    )));
    assert_eq!(
        state(&engine, id),
        SessionState::Playing,
        "commit_suicide: a life lost"
    );
    // a password set: it must match, case does not matter
    engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .suicpw = "secret".into();
    confirm(&mut engine);
    let out = line(&mut engine, &rx, id, "");
    assert!(plain(&out).starts_with("\r\nInvalid password specified.\r\n"));
    confirm(&mut engine);
    line(&mut engine, &rx, id, "SECRET");
    assert_eq!(state(&engine, id), SessionState::Playing);
    assert_eq!(engine.world.session(id).unwrap().player().unwrap().lives, 1);
}

// CP cost: every band, the cumulative totals, 10 past 90
#[test]
fn cp_cost_table() {
    // (points above the minimum, CP): the end of each band
    for (over, want) in [
        (0, 0),
        (1, 1),
        (10, 10),
        (11, 12),
        (20, 30),
        (21, 33),
        (30, 60),
        (40, 100),
        (50, 150),
        (60, 210),
        (70, 280),
        (80, 360),
        (90, 450),
        (91, 460),
        (100, 550),
    ] {
        assert_eq!(cp_cost(55, 55 + over), want, "+{over}");
    }
    // band b costs b per point
    for band in 1..=9u32 {
        let start = (band - 1) * 10;
        for p in 1..=10u32 {
            let over = start + p;
            let full: u32 = (1..band).map(|b| b * 10).sum();
            assert_eq!(cp_cost(30, 30 + over as u16), full + p * band, "+{over}");
        }
    }
    // the §0 character
    assert_eq!(cp_cost(55, 79) + cp_cost(30, 42) + cp_cost(50, 74), 98);
}

// a creation left at `X` was never written, so the
// next `E` finds no character and starts creation again (no phantom character is loaded)
#[test]
fn e_after_leaving_creation_starts_creation_again() {
    let (mut engine, rx) = new_engine(4);
    let (id, _) = start(&mut engine, &rx, "KASIMIR");
    line(&mut engine, &rx, id, "X");
    assert_eq!(state(&engine, id), SessionState::MainMenu);
    assert!(!engine.world.session(id).unwrap().created);
    let out = line(&mut engine, &rx, id, "E");
    let text = plain(&out);
    assert!(
        text.contains("Please choose a race from the following list:\r\n"),
        "{text:?}"
    );
    assert!(text.ends_with("Please choose your race [ ? for help ] : "));
    assert_eq!(state(&engine, id), SessionState::NewChar(NewCharStep::Race));
    let s = engine.world.session(id).unwrap();
    assert!(!s.created);
    let plr = s.player().unwrap();
    assert_eq!(
        (plr.race.0, plr.class.0, plr.str),
        (0, 0, 5),
        "a fresh create_player row"
    );
}
