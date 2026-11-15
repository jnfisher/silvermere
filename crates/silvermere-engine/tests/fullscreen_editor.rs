//! The full-screen character editor (T-063): the EDITCHA1 form on an ANSI terminal of
//! at least 80x23, driven one key at a time through the engine.
//!
//! The expectations follow the host's form library (FSD.C, FSDBBS.C, AIN.C) and the
//! game's `edit_character_stats`, `ljnvfy` and `ljndun`: the background is the template
//! with its answers filled in, the field under the cursor is in the cursor colour
//! (attribute 0x70, `ESC[0;7m`), a refusal goes to the `!` help area on row 23 in the
//! area's colour (bright red, `ESC[0;1;31;40m`) with a bell, and the CP display is the
//! game's own at row 20, column 33. The T-005 capture `tests/captures/creation-fsd` is an
//! MBBSEmu session, whose form library differs; its comparisons are kept, ignored.

use crate::common;

use std::sync::mpsc::Receiver;
use std::time::Instant;

use common::{TestClock, enter, special_rooms, test_config, tiny_snapshot};
use silvermere_engine::player_session::{AccountRef, Caps};
use silvermere_engine::state::newchar::{self, validate_name};
use silvermere_engine::world::blank;
use silvermere_engine::{Engine, EngineInput, EngineOutput, SeededRng, SessionId, World, session};
use silvermere_text::cp437;
use silvermere_text::strip::strip_ansi;
use silvermere_types::enums::{MessageLevel, NewCharStep, RoomType, SessionState, ShopType};
use silvermere_types::flags::{PlayerFlags, PlayerSettings};
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, RaceNo, RoomKey, ShopNo};
use silvermere_types::names::{EYE_COLOURS, HAIR_COLOURS, HAIR_LENGTHS};
use silvermere_types::records::{Character, Shop, ShopSlot};

const RAW: &[u8] = include_bytes!("../../../tests/captures/creation-fsd/raw.bin");
const SCRIPT: &str = include_str!("../../../tests/captures/creation-fsd/script.json");
const TRAINER: u32 = 1376;

type TestEngine = Engine<TestClock, SeededRng>;

/// Human (40-100 in every stat, 100 CP) and Warrior, as in the capture; a universal
/// trainer for TRAIN STATS.
fn snapshot() -> silvermere_engine::WorldSnapshot {
    crate::common::install_text();
    let mut s = tiny_snapshot();
    let mut human = blank::clear_race(RaceNo(1));
    human.name = "Human".into();
    (human.minstr, human.minint, human.minwil) = (40, 40, 40);
    (human.minagi, human.minhea, human.mincha) = (40, 40, 40);
    (human.maxstr, human.maxint, human.maxwil) = (100, 100, 100);
    (human.maxagi, human.maxhea, human.maxcha) = (100, 100, 100);
    human.basecp = 100;
    s.races = vec![human];
    let mut warrior = blank::clear_class(ClassNo(1));
    warrior.name = "Warrior".into();
    warrior.hplevel = 10;
    warrior.basehp = 6;
    s.classes = vec![warrior];
    let mut r = blank::clear_room(RoomKey::new(1, TRAINER));
    r.name = "Silvermere Universal Trainer".into();
    r.room_type = RoomType::Shop;
    r.shopnum = ShopNo(38);
    s.rooms.push(r);
    s.shops.push(Shop {
        number: ShopNo(38),
        name: "Universal Trainer".into(),
        desc: Default::default(),
        shop_type: ShopType::Training,
        minlevel: 1,
        maxlevel: 10,
        markup: 0,
        ownable: 0,
        expandable: 0,
        trainclass: ClassNo(0),
        stock: std::array::from_fn(|_| ShopSlot {
            item: ItemNo(0),
            maxqty: 0,
            qty: 0,
            regentime: 0,
            regennum: 0,
            regenpct: 0,
        }),
    });
    s
}

fn new_engine() -> (TestEngine, Receiver<EngineOutput>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let world = World::load(test_config(), 8, snapshot()).expect("loads");
    (
        Engine::new(world, TestClock::new(), SeededRng::new(1), tx),
        rx,
    )
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

fn char_mode(outs: &[EngineOutput], id: SessionId) -> Option<bool> {
    outs.iter().rev().find_map(|o| match o {
        EngineOutput::SetCharMode { id: i, on } if *i == id => Some(*on),
        _ => None,
    })
}

fn plain(bytes: &[u8]) -> String {
    cp437::decode(&strip_ansi(bytes))
}

fn escape(bytes: &[u8]) -> String {
    cp437::decode(bytes)
        .replace('\x1b', "\\e")
        .replace("\r\n", "\\r\\n\n")
}

/// A session on a terminal of `caps`, with an account `Aldric`, at the race list.
fn start(engine: &mut TestEngine, rx: &Receiver<EngineOutput>, caps: Caps) -> SessionId {
    let now = Instant::now();
    let id = SessionId(engine.world.ids.in_use() as u16);
    engine.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            let got = session::on_connect(w, caps, now).unwrap();
            assert_eq!(got, id);
            w.session_mut(id).unwrap().account = Some(AccountRef {
                id: AccountId(7),
                username: "Aldric".into(),
                sysop: false,
                gameop: false,
                sex: b'M',
            });
            newchar::begin(w, id, 0);
        }),
    });
    common::drain(rx);
    id
}

fn line(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    text: &str,
) -> Vec<EngineOutput> {
    engine.handle(EngineInput::Line {
        id,
        text: text.into(),
    });
    common::drain(rx)
}

/// Keys typed in character mode; everything the engine emitted.
fn keys(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    bytes: &[u8],
) -> Vec<EngineOutput> {
    for &byte in bytes {
        engine.handle(EngineInput::Key { id, byte });
    }
    common::drain(rx)
}

fn key_bytes(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    id: SessionId,
    k: &[u8],
) -> Vec<u8> {
    writes(&keys(engine, rx, id, k), id)
}

/// Race 1, class 1, not lawful: the form opens. Returns what the `N` produced.
fn to_form(
    engine: &mut TestEngine,
    rx: &Receiver<EngineOutput>,
    caps: Caps,
) -> (SessionId, Vec<EngineOutput>) {
    let id = start(engine, rx, caps);
    line(engine, rx, id, "1");
    line(engine, rx, id, "1");
    let outs = line(engine, rx, id, "N");
    (id, outs)
}

/// The capture's steps: what was sent (escapes decoded) and the offset of its answer.
fn steps() -> Vec<(Vec<u8>, usize)> {
    let script: serde_json::Value = serde_json::from_str(SCRIPT).unwrap();
    script["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let send = s["send"].as_str().unwrap_or("");
            let send = send.replace("\\r", "\r").replace("\\e", "\x1b");
            (
                send.into_bytes(),
                s["result"]["offset"].as_u64().unwrap() as usize,
            )
        })
        .collect()
}

/// The original's answer to step `i`, without the transport's echo of a typed line.
fn answer(i: usize) -> &'static [u8] {
    let st = steps();
    &RAW[st[i].1..st[i + 1].1]
}

fn default_caps() -> Caps {
    Caps::default()
}

fn line_mode_caps() -> Caps {
    Caps {
        columns: 79,
        ..Caps::default()
    }
}

/// The help area (row 23) showing a refusal: its colour, the text and the bell.
fn error_line(msg: &str) -> Vec<u8> {
    let mut b = b"\x1b[23;1f\x1b[0;1;31;40m".to_vec();
    b.extend(msg.as_bytes());
    b.push(0x07);
    b
}

/// The field at `row;col`, `width` wide, holding `value`, under the cursor.
fn selected(row: u16, col: u16, value: &str, width: usize, number: bool) -> Vec<u8> {
    let at = format!("\x1b[{row};{col}f");
    let value = if number {
        format!("{value:>width$}")
    } else {
        format!("{value:<width$}")
    };
    format!("{at}\x1b[0;7m{value}{at}").into_bytes()
}

/// The game's CP display.
fn cp_display(cp: i32) -> Vec<u8> {
    format!("\x1b[20;33f\x1b[0m{cp:4}").into_bytes()
}

/// The EDITCHA1 template as the host hands it over (CR LF line ends) with each field's
/// answer in place (text left, numbers right, cut to the width) and the help area blank,
/// after the clear screen of `fsdbkg`.
fn filled_template(answers: &[String]) -> Vec<u8> {
    let raw = crate::common::install_text().editcha1_bytes();
    let mut t = Vec::new();
    for b in raw {
        if b == b'\r' {
            t.extend(b"\r\n");
        } else {
            t.push(b);
        }
    }
    let mut out = b"\x1b[0m\x1b[2J\x1b[0m".to_vec();
    let (mut i, mut n) = (0, 0);
    while i < t.len() {
        let c = t[i];
        let run = t[i..].iter().take_while(|&&b| b == c).count();
        if matches!(c, b'?' | b'$' | b'!') && run >= 2 {
            if c == b'!' {
                out.extend(std::iter::repeat_n(b' ', run));
            } else {
                let a: String = answers[n].chars().take(run).collect();
                let a = if c == b'$' {
                    format!("{a:>run$}")
                } else {
                    format!("{a:<run$}")
                };
                out.extend(cp437::encode(&a));
                n += 1;
            }
            i += run;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// `ESC[row;colf` of the last cursor move in `bytes`.
fn last_cursor(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let re = regex::Regex::new(r"\x1b\[(\d+;\d+)f").unwrap();
    re.captures_iter(&text)
        .last()
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

// The EDITCHA1 form with its answers and background drawn; the capture's step 7 (the
// lawful answer) after the transport's `N\r\n`
#[test]
#[ignore = "MBBSEmu capture: the real library draws the form filled in; re-recorded from the C"]
fn first_frame_equals_the_capture() {
    let (mut engine, rx) = new_engine();
    let (id, outs) = to_form(&mut engine, &rx, default_caps());
    let got = writes(&outs, id);
    let want = &answer(7)[3..];
    assert_eq!(answer(7)[..3], *b"N\r\n");
    assert_eq!(escape(&got), escape(want));
}

// fsdbkg, fsdego, then the game's CP display: the template with every answer in place
// (the race, the class and the race limits too, as they are avoided only after the
// background), the given name under the cursor, and the cursor left at the CP display
#[test]
fn first_frame_is_the_filled_template() {
    let (mut engine, rx) = new_engine();
    let (id, outs) = to_form(&mut engine, &rx, default_caps());
    let got = writes(&outs, id);
    let p = engine.world.session(id).unwrap().player().unwrap().clone();
    let mut answers = vec![
        "Aldric".to_string(),
        String::new(),
        "Human".into(),
        "Warrior".into(),
    ];
    for cur in [p.str, p.intel, p.wil, p.agi, p.hea, p.cha] {
        answers.extend(["40".into(), "100".into(), cur.to_string()]);
    }
    answers.push(HAIR_LENGTHS[usize::from(p.hairlen)].into());
    answers.push(HAIR_COLOURS[usize::from(p.haircol)].into());
    answers.push(EYE_COLOURS[usize::from(p.eyecol)].into());
    answers.push("SAVE".into());
    let mut want = filled_template(&answers);
    want.extend(selected(4, 20, "Aldric", 18, false));
    want.extend(cp_display(100));
    assert_eq!(escape(&got), escape(&want));
    assert_eq!(last_cursor(&got), "20;33");
    assert_eq!(char_mode(&outs, id), Some(true));
    assert_eq!(
        engine.world.session(id).unwrap().state,
        SessionState::NewChar(NewCharStep::Stats)
    );
    insta::assert_snapshot!("new_character_form", crate::common::redact(&escape(&got)));
}

// Character creation through the form, with its quirks (the active field varies between
// sessions, the name check answers late): the capture's steps 8-27, one key at a time,
// each answer byte for byte
#[test]
#[ignore = "MBBSEmu capture: the real library draws the form filled in; re-recorded from the C"]
fn scripted_session_equals_the_capture() {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    let st = steps();
    let mut cursors = Vec::new();
    for (i, (send, _)) in st.iter().enumerate().take(27).skip(8) {
        let got = key_bytes(&mut engine, &rx, id, send);
        assert_eq!(escape(&got), escape(answer(i)), "step {i} {send:?}");
        cursors.push(last_cursor(&got));
    }
    // given name 4;20 .. Exit 20;14 (and back up with the arrows)
    assert_eq!(
        cursors
            .iter()
            .filter(|c| !c.is_empty())
            .cloned()
            .collect::<Vec<_>>(),
        [
            "5;20", "9;34", "9;34", "10;34", "9;34", "10;34", "11;34", "12;34", "13;34", "13;34",
            "14;34", "16;22", "16;22", "17;22", "18;22", "20;14"
        ]
    );
    // step 27, SAVE: the form is left, then the save's own output
    let outs = keys(&mut engine, &rx, id, b"\r");
    let got = writes(&outs, id);
    let leave = b"\x1b[24;0f\x1b[0m";
    let at = got.windows(leave.len()).position(|w| w == leave).unwrap() + leave.len();
    let want = answer(27);
    let want_at = want.windows(leave.len()).position(|w| w == leave).unwrap() + leave.len();
    assert_eq!(escape(&got[..at]), escape(&want[..want_at]));
    assert!(plain(&got[at..]).contains("To prevent accidental suicide or reroll"));
    assert!(plain(&got[at..]).contains("Please wait - Validating your name."));
    assert_eq!(char_mode(&outs, id), Some(false));
    assert_eq!(
        engine.world.session(id).unwrap().state,
        SessionState::NewChar(NewCharStep::Validating)
    );
    // the capture's `st` screen: Str 50, Hea 50, 80 of 100 CP left
    let plr = engine.world.session(id).unwrap().player().unwrap();
    assert_eq!(
        [plr.str, plr.intel, plr.wil, plr.agi, plr.hea, plr.cha],
        [50, 40, 40, 40, 50, 40],
        "{plr:?}"
    );
}

/// Drives a new Human Warrior with Str 69 and Hea 64, the rest at the minimum, through
/// either editor, then lets the name check pass. Returns the character.
fn create_69_64(fullscreen: bool) -> Character {
    let (mut engine, rx) = new_engine();
    let caps = if fullscreen {
        default_caps()
    } else {
        line_mode_caps()
    };
    let (id, _) = to_form(&mut engine, &rx, caps);
    if fullscreen {
        for k in [
            &b"\r"[..],
            b"Stonebrook\r",
            b"69\r",
            b"\r\r\r",
            b"64\r",
            b"\r",
            b"\r\r\r",
            b"\r",
        ] {
            keys(&mut engine, &rx, id, k);
        }
    } else {
        for answer in ["", "Stonebrook", "69", "", "", "", "64", "", "", "", "", ""] {
            line(&mut engine, &rx, id, answer);
        }
    }
    assert_eq!(
        engine.world.session(id).unwrap().state,
        SessionState::NewChar(NewCharStep::Validating)
    );
    engine.handle(EngineInput::Custom {
        session: Some(id),
        run: Box::new(move |w| validate_name::on_checked(w, id, false)),
    });
    common::drain(&rx);
    assert_eq!(
        engine.world.session(id).unwrap().state,
        SessionState::Playing
    );
    engine.world.session(id).unwrap().player().unwrap().clone()
}

// The same record whichever editor was used
#[test]
fn both_editors_end_with_the_same_record() {
    let fsd = create_69_64(true);
    let lined = create_69_64(false);
    assert_eq!(
        [fsd.str, fsd.intel, fsd.wil, fsd.agi, fsd.hea, fsd.cha],
        [69, 40, 40, 40, 64, 40]
    );
    // stat_amount_used: 69 over 40 is 57 CP, 64 over 40 is 42 CP
    assert_eq!((fsd.cp, fsd.totalcp), (1, 100));
    assert_eq!(fsd.lastname, "Stonebrook");
    assert_eq!(fsd, lined);
}

/// The answer to the keys `k` typed into a fresh form, after moving to field `skip`
/// with CRs.
fn refusal(skip: usize, k: &[u8]) -> (String, Vec<u8>) {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    keys(&mut engine, &rx, id, &b"\r".repeat(skip));
    let got = key_bytes(&mut engine, &rx, id, k);
    (plain(&got), got)
}

// Every ljnvfy message reachable on the form, shown in the help area (23;1) with the
// bell, and the cursor back on the field it was typed in. The first key typed replaces
// the answer, so a blank given name is a space.
#[test]
fn every_verifier_message() {
    let names: [(usize, &[u8], &str, &str); 5] = [
        (0, b" \r", "You have entered an invalid name!", "4;20"),
        (
            0,
            b"Bartholomew\r",
            "Your given name can only be 10 characters long!",
            "4;20",
        ),
        (
            0,
            b"Al4ric\r",
            "Only ALPHA characters are allowed in a name!",
            "4;20",
        ),
        (1, b"\r", "You must enter a last name!", "5;20"),
        (
            1,
            b"Stone-Brook\r",
            "Only ALPHA characters are allowed in a name!",
            "5;20",
        ),
    ];
    for (skip, k, msg, cursor) in names {
        let (text, bytes) = refusal(skip, k);
        assert!(text.contains(msg), "{msg}: {text:?}");
        assert!(contains(&bytes, &error_line(msg)), "{msg}");
        assert_eq!(last_cursor(&bytes), cursor, "{msg}");
    }
    let stats = [
        "Strength",
        "Intellect",
        "Willpower",
        "Agility",
        "Health",
        "Charm",
    ];
    // the stats: the family name first, then the stat
    for (i, stat) in stats.iter().enumerate() {
        let row = 9 + i;
        let to_stat = [&b"\rStonebrook\r"[..], &b"\r".repeat(i)].concat();
        for (typed, msg) in [
            (&b"39\r"[..], format!("{stat} may not be lower than 40.")),
            (b"101\r", format!("{stat} may not be higher than 100.")),
            // MIN=10 makes the field non-negative: the minus is not taken
            (b"-3\r", format!("{stat} may not be lower than 40.")),
            (b"100\r", format!("You may not assign that much to {stat}")),
        ] {
            let (mut engine, rx) = new_engine();
            let (id, _) = to_form(&mut engine, &rx, default_caps());
            keys(&mut engine, &rx, id, &to_stat);
            let got = key_bytes(&mut engine, &rx, id, typed);
            let text = plain(&got);
            assert!(text.contains(&msg), "{msg}: {text:?}");
            assert_eq!(last_cursor(&got), format!("{row};34"), "{msg}");
            assert!(contains(&got, &error_line(&msg)), "{msg}");
            // the field under the cursor again with its old answer
            assert!(
                got.ends_with(&selected(row as u16, 34, "40", 4, true)),
                "{msg}"
            );
        }
    }
}

// The form's own MIN=10: below it, under the race minimum, the game's message comes first;
// an accepted stat redraws the CP display, then the field in its own colour, then the next
// field under the cursor
#[test]
fn an_accepted_stat_shows_the_cp_left() {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    keys(&mut engine, &rx, id, b"\rStonebrook\r");
    let got = key_bytes(&mut engine, &rx, id, b"65\r");
    // 65 over 40: 10 + 20 + 15 = 45 CP
    let mut want = cp_display(55);
    want.extend(b"\x1b[9;34f\x1b[0;1;37;40m  65");
    want.extend(selected(10, 34, "40", 4, true));
    assert!(got.ends_with(&want), "{}", escape(&got));
}

// Layer 2: the help area as the form shows it
#[test]
fn error_message_snapshot() {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    keys(&mut engine, &rx, id, b"\rStonebrook\r");
    let got = key_bytes(&mut engine, &rx, id, b"101\r");
    insta::assert_snapshot!("strength_too_high", escape(&got));
}

fn opens_line_mode(caps: Caps) {
    let (mut engine, rx) = new_engine();
    let (id, outs) = to_form(&mut engine, &rx, caps);
    let text = plain(&writes(&outs, id));
    assert!(
        crate::common::holds_moved(&text, "stat_editor_preamble"),
        "{text}"
    );
    // the line by line form, its first field with its answer
    assert!(text.ends_with("Given Name  Aldric"), "{text}");
    assert_eq!(char_mode(&outs, id), Some(true));
    let s = engine.world.session(id).unwrap();
    let form = s.editor.as_ref().unwrap().form.as_ref().unwrap();
    assert!(!form.is_full_screen());
}

// 80 columns are needed
#[test]
fn seventy_nine_columns_get_the_line_editor() {
    opens_line_mode(Caps {
        columns: 79,
        rows: 25,
        ..Caps::default()
    });
}

// 23 rows are needed
#[test]
fn twenty_two_rows_get_the_line_editor() {
    opens_line_mode(Caps {
        columns: 80,
        rows: 22,
        ..Caps::default()
    });
}

// ANSI is needed
#[test]
fn ansi_off_gets_the_line_editor() {
    opens_line_mode(Caps {
        ansi: false,
        ..Caps::default()
    });
}

// A MULTICHOICE field: Space shows the next ALT= choice as the specification spells it,
// blanking the field first and leaving the cursor after it, and wraps after the last;
// Return keeps it, and the stored index is the choice's (fsdord)
#[test]
fn description_fields_cycle_through_every_value() {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    keys(&mut engine, &rx, id, b"\rStonebrook\r\r\r\r\r\r\r");
    let lengths = [
        "Short",
        "Shoulder-Length",
        "Long",
        "Waist-length",
        "Ankle-Length",
        "None",
    ];
    let hair = [
        "White",
        "Silver",
        "Red",
        "Brown",
        "Dark-Brown",
        "Blonde",
        "Green",
        "Blue",
        "Grey",
        "Black",
    ];
    let eyes = [
        "Crimson",
        "Yellow",
        "Pale-Blue",
        "Sea-Blue",
        "Dark-Blue",
        "Grey-Blue",
        "Slate-Grey",
        "Bright-Green",
        "Forest-Green",
        "Pale-Green",
        "Chesnut-Brown",
        "Dark-Brown",
        "Hazel",
        "Violet",
        "Lavender",
        "Golden",
        "Black",
    ];
    for (row, values) in [(16, &lengths[..]), (17, &hair[..]), (18, &eyes[..])] {
        for v in values {
            let got = key_bytes(&mut engine, &rx, id, b" ");
            let back = 15 - v.len();
            let mut want =
                format!("\x1b[{row};22f\x1b[0;7m{:15}\x1b[{row};22f{v:<15}", "").into_bytes();
            if back > 0 {
                want.extend(
                    format!(
                        "\x1b[{}D",
                        if back > 1 {
                            back.to_string()
                        } else {
                            String::new()
                        }
                    )
                    .into_bytes(),
                );
            }
            assert_eq!(escape(&got), escape(&want));
        }
        // one more: the second value again, then on to the next field
        keys(&mut engine, &rx, id, b" \r");
    }
    // SAVE
    keys(&mut engine, &rx, id, b"\r");
    let plr = engine.world.session(id).unwrap().player().unwrap();
    assert_eq!((plr.hairlen, plr.haircol, plr.eyecol), (1, 1, 1));
}

// The SAVE(ALT=SAVE ALT=EDIT ALT=QUIT) field: EDIT goes back to the first field and the
// field keeps SAVE (vfyadn's VFYDEF); QUIT on a new character is the menu
#[test]
fn edit_then_quit() {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    keys(&mut engine, &rx, id, b"\rStonebrook\r69\r\r\r\r\r\r\r\r\r");
    let got = key_bytes(&mut engine, &rx, id, b" ");
    assert!(got.ends_with(b"\x1b[20;14fEDIT"), "{}", escape(&got));
    let got = key_bytes(&mut engine, &rx, id, b"\r");
    assert_eq!(last_cursor(&got), "4;20");
    // the exit field shows SAVE again, the given name is under the cursor
    assert!(
        contains(&got, b"\x1b[20;14f\x1b[0;1;37;40mSAVE"),
        "{}",
        escape(&got)
    );
    assert!(got.ends_with(&selected(4, 20, "Aldric", 18, false)));
    // the answers are kept: back to the exit field, QUIT
    let got = key_bytes(&mut engine, &rx, id, b"\r\r\r\r\r\r\r\r\r\r\r");
    assert_eq!(last_cursor(&got), "20;14");
    assert!(contains(&got, b"\x1b[9;34f\x1b[0;1;37;40m  69"));
    keys(&mut engine, &rx, id, b"  ");
    let outs = keys(&mut engine, &rx, id, b"\r");
    let got = writes(&outs, id);
    // goback: below the form (row maxy + 1), then the colours
    assert!(
        contains(&got, b"\x1b[23;1f\x1b[0;1;32m"),
        "{}",
        escape(&got)
    );
    let text = plain(&got);
    assert!(text.contains("[MAJORMUD]:"), "{text}");
    assert_eq!(char_mode(&outs, id), Some(false));
    assert_eq!(
        engine.world.session(id).unwrap().state,
        SessionState::MainMenu
    );
}

// AIN.C: a sequence it does not know ends with no key, and F1 is a key the form ignores;
// ESC and another byte is that byte, so `ESC x` types an x
#[test]
fn ansi_keys_as_the_host_translates_them() {
    let (mut engine, rx) = new_engine();
    let (id, _) = to_form(&mut engine, &rx, default_caps());
    for seq in [&b"\x1b[2~"[..], b"\x1bOP", b"\x1b[5;1R"] {
        let got = key_bytes(&mut engine, &rx, id, seq);
        assert!(got.is_empty(), "{seq:?}: {}", escape(&got));
    }
    // still on the given name: x replaces it
    let got = key_bytes(&mut engine, &rx, id, b"\x1bx");
    assert_eq!(
        escape(&got),
        escape(format!("\x1b[4;20f\x1b[0;7m{:18}\x1b[4;20fx", "").as_bytes())
    );
}

/// A level-2 Human Warrior at the trainer: Str 60, the rest at 40, 15 of 35 CP left.
fn trainee(engine: &mut TestEngine) -> SessionId {
    let w = &mut engine.world;
    let id = enter(w, "Kraag", TRAINER, MessageLevel::High);
    let s = w.session_mut(id).unwrap();
    s.account = Some(AccountRef {
        id: AccountId(9),
        username: "KRAAG".into(),
        sysop: false,
        gameop: false,
        sex: b'M',
    });
    s.created = true;
    let c = s.character.as_mut().unwrap();
    c.settings |= PlayerSettings::VALIDATED;
    c.flags |= PlayerFlags::RACECHANGED;
    c.version = 22;
    c.lastname = "Stonefist".into();
    c.suicpw = "secret".into();
    c.level = 2;
    (c.exp, c.exprem) = (1700, 1700);
    (c.hp, c.maxhp) = (31, 40);
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (60, 40, 40, 40, 40, 40);
    (c.basstr, c.basint, c.baswil) = (60, 40, 40);
    (c.basagi, c.bashea, c.bascha) = (40, 40, 40);
    // Str 60 over 40: 10 + 2*10 = 30 CP spent
    (c.totalcp, c.cp) = (45, 15);
    id
}

// `train stats` opens the form with the first name locked, so the cursor starts at the
// family name (and is left at the CP display); the answers are checked and saved as at
// creation
#[test]
fn train_stats_on_the_form() {
    let (mut engine, rx) = new_engine();
    let id = trainee(&mut engine);
    let watcher = enter(
        &mut engine.world,
        "Watcher",
        special_rooms()[0],
        MessageLevel::High,
    );
    let outs = line(&mut engine, &rx, id, "train stats");
    let got = writes(&outs, id);
    assert_eq!(char_mode(&outs, id), Some(true));
    assert!(plain(&got).contains("Character Creation"));
    let mut tail = selected(5, 20, "Stonefist", 18, false);
    tail.extend(cp_display(15));
    assert!(got.ends_with(&tail), "{}", escape(&got));
    insta::assert_snapshot!(
        "existing_character_form",
        crate::common::redact(&escape(&got))
    );
    // Up from the family name: the first name is avoided, so the cursor goes round to the
    // exit field (movfld wraps); Down goes round again to the family name
    let got = key_bytes(&mut engine, &rx, id, b"\x1b[A");
    let mut want = b"\x1b[5;20f\x1b[0;1;37;40mStonefist         ".to_vec();
    want.extend(selected(20, 14, "SAVE", 4, false));
    assert_eq!(escape(&got), escape(&want));
    let got = key_bytes(&mut engine, &rx, id, b"\x1b[B");
    assert_eq!(last_cursor(&got), "5;20");
    // to Strength: lowering it is refused
    keys(&mut engine, &rx, id, b"\r");
    let got = key_bytes(&mut engine, &rx, id, b"59\r");
    assert!(contains(
        &got,
        &error_line("Strength may not be lower than 60.")
    ));
    assert_eq!(last_cursor(&got), "9;34");
    // stat_amount_used over the minimum 40: 60 costs 30, 65 costs 45 (all 45 CP), 66 48
    let got = key_bytes(&mut engine, &rx, id, b"66\r");
    assert!(contains(
        &got,
        &error_line("You may not assign that much to Strength")
    ));
    assert_eq!(last_cursor(&got), "9;34");
    let got = key_bytes(&mut engine, &rx, id, b"65\r");
    assert!(contains(&got, &cp_display(0)), "{}", escape(&got));
    assert_eq!(last_cursor(&got), "10;34");
    // nothing left for Intellect: 41 costs 1
    let got = key_bytes(&mut engine, &rx, id, b"41\r");
    assert!(contains(
        &got,
        &error_line("You may not assign that much to Intellect")
    ));
    // the rest as they are, SAVE
    let outs = keys(&mut engine, &rx, id, b"40\r\r\r\r\r\r\r\r\r");
    let got = writes(&outs, id);
    assert!(contains(&got, b"\x1b[23;1f\x1b[0;1;32m"));
    assert_eq!(char_mode(&outs, id), Some(false));
    let s = engine.world.session(id).unwrap();
    assert_eq!(s.state, SessionState::Playing);
    let plr = s.player().unwrap();
    assert_eq!(
        [plr.str, plr.intel, plr.wil, plr.agi, plr.hea, plr.cha],
        [65, 40, 40, 40, 40, 40]
    );
    assert_eq!((plr.cp, plr.totalcp), (0, 45));
    assert_eq!(plr.lastname, "Stonefist");
    assert!(plain(&writes(&outs, watcher)).contains("Kraag just entered the Realm."));
}
