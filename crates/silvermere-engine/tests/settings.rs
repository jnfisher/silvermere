//! `SET` (T-028): every option of `cmd_set` through the whole input
//! pipeline, with its confirmation, its field and its `Valid ...` line; the PROFILE screen
//! (`display_profile`); the prompt each statline mode gives; where the
//! settings bite (Enter under FULL, `messages`, `entrances`, the new `utf8`); and what
//! survives logoff and death.

mod common;

use std::sync::mpsc::Receiver;
use std::time::Instant;

use common::{TestClock, enter, special_rooms, tiny_world};
use silvermere_engine::commands::settings::{Change, Form, SetError, parse};
use silvermere_engine::commands::{Cmd, CommandLine};
use silvermere_engine::player::profile::{self, Retire};
use silvermere_engine::player::{create_player, load_player};
use silvermere_engine::{
    AccountRef, Caps, Engine, EngineInput, EngineOutput, Noise, PersistRequest, PlayerSession,
    SeededRng, SessionId, TellBlind, World, tell_room,
};
use silvermere_text::{Msg, strip_ansi_str};
use silvermere_types::enums::{InGame, MessageLevel, SessionState, StatlineMode};
use silvermere_types::flags::{PlayerFlags, PlayerSettings};
use silvermere_types::ids::{AccountId, RoomKey};
use silvermere_types::records::{Character, Coins};

const TODAY: i64 = 1_760_000_000;

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
}

impl Harness {
    fn new() -> Self {
        Self::with(|_| {})
    }

    fn with(edit: impl FnOnce(&mut World)) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut world = tiny_world(8);
        edit(&mut world);
        let engine = Engine::new(world, TestClock::new(), SeededRng::new(7), tx);
        Self {
            engine,
            rx,
            log: Vec::new(),
        }
    }

    /// A playing character with an account, 48 hit points and the statline ON.
    fn player(&mut self, name: &str) -> SessionId {
        let w = &mut self.engine.world;
        let id = enter(w, name, special_rooms()[0], MessageLevel::High);
        let s = w.session_mut(id).unwrap();
        s.account = Some(AccountRef {
            id: AccountId(i64::from(id.0) + 1),
            username: name.to_ascii_uppercase(),
            sysop: false,
            gameop: false,
            sex: b'M',
        });
        s.created = true;
        let c = s.character.as_mut().unwrap();
        // a validated name, so save_player writes it
        c.settings |= PlayerSettings::VALIDATED;
        c.version = silvermere_engine::player::PLAYER_VERSION;
        (c.hp, c.maxhp) = (48, 48);
        c.movedir = -1;
        id
    }

    fn plr(&self, id: SessionId) -> &Character {
        self.engine.world.session(id).unwrap().player().unwrap()
    }

    fn plr_mut(&mut self, id: SessionId) -> &mut Character {
        self.engine
            .world
            .session_mut(id)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
    }

    fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            self.log.push(o);
        }
    }

    /// Types `text` and returns what the session received, as bytes.
    fn bytes(&mut self, id: SessionId, text: &str) -> Vec<u8> {
        let from = self.log.len();
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.pump();
        self.written(from, id)
    }

    fn written(&self, from: usize, id: SessionId) -> Vec<u8> {
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

    /// Types `text` and returns what the session received, ANSI stripped, `\r\n` as `\n`.
    fn line(&mut self, id: SessionId, text: &str) -> String {
        let b = self.bytes(id, text);
        strip_ansi_str(&String::from_utf8_lossy(&b)).replace("\r\n", "\n")
    }

    fn persisted(&self) -> Vec<&PersistRequest> {
        self.log
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Persist(p) => Some(p),
                _ => None,
            })
            .collect()
    }
}

/// The text a message renders to, ANSI stripped, `\r\n` as `\n`.
fn text(msg: Msg) -> String {
    let r = silvermere_text::render(&msg, Default::default());
    strip_ansi_str(&String::from_utf8_lossy(&r.bytes)).replace("\r\n", "\n")
}

/// `out` is `msgs` then the `[HP=48]:` prompt.
#[track_caller]
fn says(out: &str, msgs: &[&str]) {
    let mut want: String = msgs.concat();
    want.push_str("[HP=48]:");
    assert_eq!(out, want);
}

/// Types `line`, checks the confirmation, and returns the character.
#[track_caller]
fn set(h: &mut Harness, id: SessionId, line: &str, msg: &str) {
    let out = h.line(id, line);
    says(&out, &[msg]);
}

// ---------------------------------------------------------------------------------------
// SET alone, unknown options, the parser's order

// The T-005 capture tests/captures/status-screens/plain.txt:90-92
#[test]
fn set_alone_lists_the_options() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let out = h.line(id, "set");
    let capture = include_str!("../../../tests/captures/status-screens/plain.txt");
    let line = capture
        .lines()
        .skip_while(|l| *l != ">>> 'set'")
        .nth(2)
        .expect("the capture has the SET reply");
    assert_eq!(out, format!("{line}\n[HP=48]:"));
    insta::assert_snapshot!("set_options", out);
}

// Rc = 0: one word that is no option is no command; two words are
// ignored without a word; more words, unless a custom statline, likewise
#[test]
fn unknown_options() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    assert_eq!(
        parse(&CommandLine::parse("set xyzzy"), h.plr(id)),
        Err(SetError::NotOption)
    );
    let out = h.line(id, "set xyzzy");
    assert!(!out.contains("Valid"), "{out}");
    says(&h.line(id, "set xyzzy on"), &[]);
    says(&h.line(id, "set gossip on now"), &[]);
    says(&h.line(id, "set statline full nonsense here"), &[]);
    says(&h.line(id, "set statline full custom"), &[]);
}

// The `starts_with_ignore_case` prefix and the table order: SET S is SUICIDE, SET M is
// MAXPAGES, SET MESS HIGH works, and SPEECH/TALK share one arm
#[test]
fn abbreviations_follow_the_original_order() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let p = |l: &str, h: &Harness| parse(&CommandLine::parse(l), h.plr(id));
    assert_eq!(p("set s", &h), Ok(Change::Suicide));
    assert_eq!(p("SET M", &h), Ok(Change::MaxpagesSyntax));
    assert_eq!(p("set m 9", &h), Ok(Change::Maxpages(9)));
    assert_eq!(
        p("set mess high", &h),
        Ok(Change::Messages(MessageLevel::High))
    );
    // the cleared character talks slowly, so the toggle makes it fast
    assert_eq!(p("set sp", &h), Ok(Change::FastTalk(true, Form::Toggle)));
    // SPEECH/TALK come before TELEPATH
    assert_eq!(p("set t", &h), Ok(Change::FastTalk(true, Form::Toggle)));
    // PALETTE comes before PAGE, so SET P OK is a bad palette
    assert_eq!(p("set p 2", &h), Ok(Change::Palette(2)));
    assert_eq!(
        p("set p ok", &h),
        Err(SetError::Invalid(Box::new(Msg::SetPaletteOptions)))
    );
    // the value's own order: SET INTERRUPT O is ON (ON is tried before OFF)
    assert_eq!(p("set int o", &h), Ok(Change::Interrupt(true)));
}

// ---------------------------------------------------------------------------------------
// The two-state options: one test per keyword, the toggle both ways and
// every value of the two-word form, then a bad value

/// Runs a two-state option: from the cleared character (`get` false), `SET {option}`
/// twice gives `on` then `off`; each `(value, state, msg)` sets and confirms; a bad value
/// prints `invalid` and changes nothing.
#[track_caller]
fn two_state(
    option: &str,
    get: fn(&Character) -> bool,
    on: Msg,
    off: Msg,
    values: &[(&str, bool, Msg)],
    invalid: Msg,
) {
    let mut h = Harness::new();
    let id = h.player("Ann");
    assert!(!get(h.plr(id)), "{option} starts off");
    set(&mut h, id, &format!("set {option}"), &text(on));
    assert!(get(h.plr(id)), "{option} toggled on");
    set(&mut h, id, &format!("set {option}"), &text(off));
    assert!(!get(h.plr(id)), "{option} toggled off");
    for (value, state, msg) in values {
        set(
            &mut h,
            id,
            &format!("set {option} {value}"),
            &text(msg.clone()),
        );
        assert_eq!(get(h.plr(id)), *state, "set {option} {value}");
    }
    let before = h.plr(id).clone();
    set(&mut h, id, &format!("set {option} bogus"), &text(invalid));
    assert_eq!(*h.plr(id), before, "a bad value changes nothing");
}

// (keepexp on)
#[test]
fn keep() {
    fn get(c: &Character) -> bool {
        c.settings.contains(PlayerSettings::KEEPEXP)
    }
    let mut h = Harness::with(|w| w.config.keep_exp = true);
    let id = h.player("Ann");
    set(&mut h, id, "set keep", &text(Msg::SetKeepOn));
    assert!(get(h.plr(id)));
    set(&mut h, id, "set keep", &text(Msg::SetKeepOff));
    assert!(!get(h.plr(id)));
    set(&mut h, id, "set keep on", &text(Msg::SetKeepOn));
    assert!(get(h.plr(id)));
    set(&mut h, id, "set keep off", &text(Msg::SetKeepOff));
    assert!(!get(h.plr(id)));
    set(&mut h, id, "set keep maybe", &text(Msg::SetKeepOptions));
}

// With the sysop's KEEPEXP off the flag is still set, with a note
#[test]
fn keep_disabled_on_this_system() {
    let mut h = Harness::with(|w| w.config.keep_exp = false);
    let id = h.player("Ann");
    let out = h.line(id, "set keep");
    says(&out, &[&text(Msg::SetKeepOn), &text(Msg::SetKeepDisabled)]);
    assert_eq!(
        out.lines().nth(1),
        Some("(Currently disabled on this system)")
    );
    assert!(h.plr(id).settings.contains(PlayerSettings::KEEPEXP));
    let out = h.line(id, "set keep on");
    says(&out, &[&text(Msg::SetKeepOn), &text(Msg::SetKeepDisabled)]);
    set(&mut h, id, "set keep off", &text(Msg::SetKeepOff));
}

// The sic `techincal` of the two-word form
#[test]
fn style() {
    two_state(
        "style",
        |c| c.settings.contains(PlayerSettings::TECHNICAL),
        Msg::SetStyleTechnical,
        Msg::SetStyleFantasy,
        &[
            ("technical", true, Msg::SetStyleTechincal),
            ("fantasy", false, Msg::SetStyleFantasy),
            ("tech", true, Msg::SetStyleTechincal),
        ],
        Msg::SetStyleOptions,
    );
    assert_eq!(
        text(Msg::SetStyleTechincal),
        "You will now receive techincal style messages.\n"
    );
}

// The flag is PF_NOGOSSIP, so "on" clears it
#[test]
fn gossip() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let blocked = |h: &Harness| h.plr(id).flags.contains(PlayerFlags::NOGOSSIP);
    set(&mut h, id, "set gossip", &text(Msg::SetGossipOff));
    assert!(blocked(&h));
    set(&mut h, id, "set gossip", &text(Msg::SetGossipOn));
    assert!(!blocked(&h));
    set(&mut h, id, "set gossip off", &text(Msg::SetGossipOff));
    assert!(blocked(&h));
    set(&mut h, id, "set gossip on", &text(Msg::SetGossipOn));
    assert!(!blocked(&h));
    set(&mut h, id, "set gossip x", &text(Msg::SetGossipOptions));
}

#[test]
fn auction() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let blocked = |h: &Harness| h.plr(id).flags.contains(PlayerFlags::NOAUCTION);
    set(&mut h, id, "set auction", &text(Msg::SetAuctionOff));
    assert!(blocked(&h));
    set(&mut h, id, "set auction", &text(Msg::SetAuctionOn));
    assert!(!blocked(&h));
    set(&mut h, id, "set auction off", &text(Msg::SetAuctionOff));
    assert!(blocked(&h));
    set(&mut h, id, "set auction on", &text(Msg::SetAuctionOn));
    assert!(!blocked(&h));
    set(&mut h, id, "set auction x", &text(Msg::SetAuctionOptions));
}

// The toggle words and the two-word lines (TALKCHAR)
#[test]
fn speech() {
    two_state(
        "speech",
        |c| c.fasttalk != 0,
        Msg::SetTalkFast,
        Msg::SetTalkSlow,
        &[
            ("fast", true, Msg::SetTalkFastLine),
            ("slow", false, Msg::SetTalkSlowLine { ch: '.' }),
        ],
        Msg::SetTalkOptions,
    );
}

// TALK is the same arm as SPEECH
#[test]
fn talk() {
    two_state(
        "talk",
        |c| c.fasttalk != 0,
        Msg::SetTalkFast,
        Msg::SetTalkSlow,
        &[
            ("slow", false, Msg::SetTalkSlowLine { ch: '.' }),
            ("fast", true, Msg::SetTalkFastLine),
        ],
        Msg::SetTalkOptions,
    );
    let mut h = Harness::with(|w| w.config.talk_char = '\'');
    let id = h.player("Ann");
    set(
        &mut h,
        id,
        "set talk slow",
        "To talk to the room, you must begin your line with a '\n",
    );
}

// The sic INTERRUPT in its options line
#[test]
fn responses() {
    two_state(
        "responses",
        |c| c.settings.contains(PlayerSettings::BRIEFTALK),
        Msg::SetResponsesBrief,
        Msg::SetResponsesVerbose,
        &[
            ("brief", true, Msg::SetResponsesBrief),
            ("verbose", false, Msg::SetResponsesVerbose),
        ],
        Msg::SetResponsesOptions,
    );
    assert_eq!(
        text(Msg::SetResponsesOptions),
        "Valid interrupt options: BRIEF, VERBOSE\n"
    );
}

#[test]
fn interrupt() {
    two_state(
        "interrupt",
        |c| c.settings.contains(PlayerSettings::ATKINTERRUPT),
        Msg::SetInterruptOn,
        Msg::SetInterruptOff,
        &[
            ("on", true, Msg::SetInterruptOn),
            ("off", false, Msg::SetInterruptOff),
        ],
        Msg::SetInterruptOptions,
    );
}

#[test]
fn warning() {
    two_state(
        "warning",
        |c| c.settings.contains(PlayerSettings::EVILWARN),
        Msg::SetWarningOn,
        Msg::SetWarningOff,
        &[
            ("on", true, Msg::SetWarningOn),
            ("off", false, Msg::SetWarningOff),
        ],
        Msg::SetWarningOptions,
    );
}

// The flag is PS_NOTELEPATH, so "on" allows them
#[test]
fn telepath() {
    two_state(
        "telepath",
        |c| c.settings.contains(PlayerSettings::NOTELEPATH),
        Msg::SetTelepathBlocked,
        Msg::SetTelepathAllowed,
        &[
            ("off", true, Msg::SetTelepathBlocked),
            ("on", false, Msg::SetTelepathAllowed),
        ],
        Msg::SetTelepathOptions,
    );
}

#[test]
fn entrances() {
    two_state(
        "entrances",
        |c| c.flags.contains(PlayerFlags::NOENTRYMSG),
        Msg::SetEntrancesOff,
        Msg::SetEntrancesOn,
        &[
            ("off", true, Msg::SetEntrancesOff),
            ("on", false, Msg::SetEntrancesOn),
        ],
        Msg::SetEntranceOptions,
    );
}

#[test]
fn follow() {
    two_state(
        "follow",
        |c| c.settings.contains(PlayerSettings::BLINDFOLLOW),
        Msg::SetFollowBlind,
        Msg::SetFollowNormal,
        &[
            ("blind", true, Msg::SetFollowBlind),
            ("normal", false, Msg::SetFollowNormal),
        ],
        Msg::SetFollowOptions,
    );
}

// The sic INTERRUPT options line
#[test]
fn receive() {
    two_state(
        "receive",
        |c| c.settings.contains(PlayerSettings::NORECEIVE),
        Msg::SetReceiveOff,
        Msg::SetReceiveOn,
        &[
            ("off", true, Msg::SetReceiveOff),
            ("on", false, Msg::SetReceiveOn),
        ],
        Msg::SetInterruptOptions,
    );
}

// ON and OK both allow paging
#[test]
fn page() {
    let ok = || Msg::SetPaging { state: "Ok".into() };
    let off = || Msg::SetPaging {
        state: "Off".into(),
    };
    two_state(
        "page",
        |c| c.paging != 0,
        ok(),
        off(),
        &[
            ("on", true, ok()),
            ("off", false, off()),
            ("ok", true, ok()),
        ],
        Msg::SetPageOptions,
    );
    // the PAGE command's own line has no full stop
    assert_eq!(text(ok()), "Paging is now set to Ok.\n");
    assert_eq!(
        text(Msg::PagingSet { state: "Ok".into() }),
        "Paging is now set to Ok\n"
    );
}

#[test]
fn gang() {
    two_state(
        "gang",
        |c| c.flags.contains(PlayerFlags::GANGONLINE),
        Msg::SetGangOnline,
        Msg::SetGangAll,
        &[
            ("online", true, Msg::SetGangOnline),
            ("all", false, Msg::SetGangAll),
        ],
        Msg::SetGangOptions,
    );
}

// ---------------------------------------------------------------------------------------
// Numbers, levels, palette, gender, suicide

#[test]
fn maxpages() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    set(
        &mut h,
        id,
        "set maxpages",
        "Please specify the maximum number of pages: SET MAXPAGES <n>\n",
    );
    for (typed, stored) in [("5", 5), ("300", 255), ("0", 0), ("abc", 0), ("-1", 255)] {
        set(
            &mut h,
            id,
            &format!("set maxpages {typed}"),
            &format!("Maxpages set to {stored}\n"),
        );
        assert_eq!(h.plr(id).pagemax, stored, "{typed}");
    }
}

#[test]
fn maxgossip() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    set(&mut h, id, "set maxgossip 7", "Max gossips set to 7\n");
    assert_eq!(h.plr(id).gossmax, 7);
    set(&mut h, id, "set maxgossip 999", "Max gossips set to 255\n");
    assert_eq!(h.plr(id).gossmax, 255);
}

#[test]
fn maxauction() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    set(&mut h, id, "set maxauction 2", "Max auctions set to 2\n");
    assert_eq!(h.plr(id).auctmax, 2);
}

// The cycle High -> Low -> Medium -> High, and the values
#[test]
fn messages() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let level = |h: &Harness| h.plr(id).msglevel;
    set(&mut h, id, "set messages", &text(Msg::SetMessagesLow));
    assert_eq!(level(&h), MessageLevel::Low);
    set(&mut h, id, "set messages", &text(Msg::SetMessagesMedium));
    assert_eq!(level(&h), MessageLevel::Medium);
    set(&mut h, id, "set messages", &text(Msg::SetMessagesHigh));
    assert_eq!(level(&h), MessageLevel::High);
    for (value, want, msg) in [
        ("low", MessageLevel::Low, Msg::SetMessagesLow),
        ("medium", MessageLevel::Medium, Msg::SetMessagesMedium),
        ("high", MessageLevel::High, Msg::SetMessagesHigh),
        ("l", MessageLevel::Low, Msg::SetMessagesLow),
    ] {
        set(&mut h, id, &format!("set messages {value}"), &text(msg));
        assert_eq!(level(&h), want, "{value}");
    }
    set(
        &mut h,
        id,
        "set messages loud",
        "Valid messaging options: High, Medium, Low\n",
    );
}

#[test]
fn palette() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    for n in 0..4u8 {
        set(
            &mut h,
            id,
            &format!("set palette {n}"),
            &format!("You will now use colour palette number {n}.\n"),
        );
        assert_eq!(h.plr(id).palette, n);
    }
    set(
        &mut h,
        id,
        "set palette 4",
        "Valid palettes are: 0, 1, 2, 3\n",
    );
    assert_eq!(h.plr(id).palette, 3);
}

// GENDERGO gold, paid with the coins listed; free at 0; forbidden below 0
#[test]
fn gender() {
    let mut h = Harness::with(|w| w.config.gender_gold = 10);
    let id = h.player("Ann");
    h.plr_mut(id).coins = Coins {
        gold: 12,
        ..Coins::default()
    };
    set(
        &mut h,
        id,
        "set gender female",
        "You hand over 10 gold crowns and you are now female!\n",
    );
    assert_eq!((h.plr(id).sex, h.plr(id).coins.gold), (b'F', 2));
    // the price is checked before the word
    set(
        &mut h,
        id,
        "set gender male",
        "You cannot afford to do that!\n",
    );
    assert_eq!(h.plr(id).sex, b'F');
    h.plr_mut(id).coins.gold = 20;
    set(
        &mut h,
        id,
        "set gender it",
        "Valid gender options: MALE, FEMALE\n",
    );
    assert_eq!(h.plr(id).coins.gold, 20);
    set(
        &mut h,
        id,
        "set gender m",
        "You hand over 10 gold crowns and you are now male!\n",
    );
    assert_eq!(h.plr(id).sex, b'M');

    let mut h = Harness::with(|w| w.config.gender_gold = 0);
    let id = h.player("Bob");
    set(&mut h, id, "set gender female", "You are now female!\n");
    assert_eq!(h.plr(id).sex, b'F');

    let mut h = Harness::with(|w| w.config.gender_gold = -1);
    let id = h.player("Cid");
    set(
        &mut h,
        id,
        "set gender female",
        "You may not do that here!\n",
    );
    assert_eq!(h.plr(id).sex, b'M');
}

// T-022's password prompts, no prompt after (RC_NOPROMPT)
#[test]
fn suicide() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let out = h.line(id, "set suicide");
    assert_eq!(out, "\nEnter New Password:");
    assert!(matches!(
        h.engine.world.session(id).unwrap().state,
        SessionState::Suicide(_)
    ));
    let mut h = Harness::new();
    let id = h.player("Bob");
    h.plr_mut(id).suicpw = "secret".into();
    assert_eq!(h.line(id, "set s"), "Enter the current password:");
}

// ---------------------------------------------------------------------------------------
// The statline: modes, custom templates, the prompt, Enter

// ON/OFF/FULL set the statline with no confirmation; the prompt that
// follows is the new mode's
#[test]
fn statline_modes_show_in_the_next_prompt() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    assert_eq!(h.line(id, "set statline off"), ">");
    assert_eq!(h.plr(id).statline, StatlineMode::Off);
    assert_eq!(h.line(id, "set statline on"), "[HP=48]:");
    assert_eq!(h.plr(id).statline, StatlineMode::On);
    assert_eq!(h.line(id, "set statline full"), "[HP=48]:");
    assert_eq!(h.plr(id).statline, StatlineMode::Full);
    says(
        &h.line(id, "set statline custom"),
        &["Valid statline options: ON, OFF, FULL, CUSTOM xxx, FULL CUSTOM xxx\n"],
    );
    says(
        &h.line(id, "set statline bright"),
        &[&text(Msg::SetStatlineOptions)],
    );
    assert_eq!(h.plr(id).statline, StatlineMode::Full);
}

// The rest of the line, cut to 60 characters, in the session and the profile
#[test]
fn statline_custom_templates() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    assert_eq!(h.line(id, "set statline custom [%h/%H]:"), "[48/48]:");
    assert_eq!(h.plr(id).statline, StatlineMode::Custom);
    let s = h.engine.world.session(id).unwrap();
    assert_eq!(s.ust.statline, "[%h/%H]:");
    // the words keep their spacing; `%%` prints two percent signs
    assert_eq!(
        h.line(id, "set statline custom HP  %h 100%%"),
        "HP  48 100%%"
    );
    // FULL CUSTOM, and the 60-character cut
    let long = format!("{}XYZ", "a".repeat(58));
    assert_eq!(
        h.line(id, &format!("set statline full custom {long}")),
        format!("{}XY", "a".repeat(58))
    );
    assert_eq!(h.plr(id).statline, StatlineMode::FullCustom);
    let s = h.engine.world.session(id).unwrap();
    assert_eq!(s.ust.statline.chars().count(), 60);
    // the profile keeps it (the account-level save)
    let account = s.account.as_ref().unwrap().id;
    assert_eq!(h.engine.world.profiles[&account].statfmt, s.ust.statline);
    assert!(h.persisted().iter().any(|p| matches!(
        p,
        PersistRequest::Profile { record, .. } if record.statfmt == "[%h/%H]:"
    )));
}

// STATLINE as a command takes ON/OFF/FULL only
#[test]
fn statline_command() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    assert_eq!(h.line(id, "statline off"), ">");
    assert_eq!(h.plr(id).statline, StatlineMode::Off);
    assert_eq!(
        h.line(id, "statline custom"),
        "Syntax: STATLINE {ON/OFF/FULL}\n>"
    );
    assert_eq!(h.plr(id).statline, StatlineMode::Off);
}

// Enter shows the room again under FULL and FULL CUSTOM only
#[test]
fn enter_redisplays_the_room_only_in_full_modes() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    for (line, room) in [
        ("set statline on", false),
        ("set statline full", true),
        ("set statline custom >", false),
        ("set statline full custom >", true),
        ("set statline off", false),
    ] {
        h.line(id, line);
        let out = h.line(id, "");
        assert_eq!(out.contains("Room 1"), room, "{line}: {out:?}");
    }
}

// ---------------------------------------------------------------------------------------
// Where the settings bite

// Others' misses (TRN_HIGH) reach only players at `messages high`
#[test]
fn messages_low_hides_others_misses() {
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    h.line(ann, "set messages low");
    h.line(bob, "set messages medium");
    let from = h.log.len();
    let room = RoomKey::new(1, special_rooms()[0]);
    let miss = Msg::PlayerEntered { who: "Cid".into() };
    let w = &mut h.engine.world;
    tell_room(
        w,
        room,
        [None, None],
        Noise::High,
        TellBlind::All,
        false,
        None,
        &miss,
    );
    tell_room(
        w,
        room,
        [None, None],
        Noise::Medium,
        TellBlind::All,
        false,
        None,
        &miss,
    );
    h.engine.flush();
    h.pump();
    let seen = |h: &Harness, id| {
        strip_ansi_str(&String::from_utf8_lossy(&h.written(from, id)))
            .matches("Cid")
            .count()
    };
    assert_eq!((seen(&h, ann), seen(&h, bob)), (0, 1));
    h.line(ann, "set messages high");
    let from = h.log.len();
    let w = &mut h.engine.world;
    tell_room(
        w,
        room,
        [None, None],
        Noise::High,
        TellBlind::All,
        false,
        None,
        &miss,
    );
    h.engine.flush();
    h.pump();
    assert_eq!(
        strip_ansi_str(&String::from_utf8_lossy(&h.written(from, ann)))
            .matches("Cid")
            .count(),
        1
    );
}

// `tell_game` with PF_NOENTRYMSG: `set entrances off` hides the
// broadcast from a second session's entry
#[test]
fn entrances_off_hides_entries() {
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    h.line(ann, "set entrances off");
    let from = h.log.len();
    let w = &mut h.engine.world;
    let cid = w.ids.alloc().unwrap();
    let mut s = PlayerSession::new(cid, Caps::default(), Instant::now());
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    w.sessions.insert(cid, s);
    silvermere_engine::tell_game(
        w,
        Some(cid),
        PlayerFlags::NOENTRYMSG,
        false,
        None,
        &Msg::PlayerEntered { who: "Cid".into() },
    );
    h.engine.flush();
    h.pump();
    let got = |h: &Harness, id| {
        String::from_utf8_lossy(&h.written(from, id)).contains("Cid just entered the Realm.")
    };
    assert!(!got(&h, ann));
    assert!(got(&h, bob));
    h.line(ann, "set entrances on");
    assert!(!h.plr(ann).flags.contains(PlayerFlags::NOENTRYMSG));
}

// [new] SET UTF8: the session's CP437 output as UTF-8 from the next write on
#[test]
fn utf8() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    let boxed = |h: &mut Harness| {
        let from = h.log.len();
        let s = h.engine.world.session_mut(id).unwrap();
        s.output.push_raw(vec![0xC4]); // CP437 '─'
        h.engine.flush();
        h.pump();
        h.written(from, id)
    };
    assert_eq!(boxed(&mut h), [0xC4]);
    let out = h.bytes(id, "set utf8 on");
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("UTF-8 output is now on.")
    );
    assert!(h.engine.world.session(id).unwrap().caps.utf8);
    assert_eq!(boxed(&mut h), "─".as_bytes());
    set(&mut h, id, "set utf8 off", "UTF-8 output is now off.\n");
    assert_eq!(boxed(&mut h), [0xC4]);
    set(
        &mut h,
        id,
        "set utf8 maybe",
        "Valid utf8 options: ON, OFF\n",
    );
    // the default comes from compat.charset
    let mut h = Harness::with(|w| w.compat.utf8 = true);
    h.engine.handle(EngineInput::Connected {
        conn: silvermere_engine::ConnId(1),
        caps: Caps::default(),
    });
    let id = *h.engine.world.sessions.keys().next().unwrap();
    assert!(h.engine.world.session(id).unwrap().caps.utf8);
}

// ---------------------------------------------------------------------------------------
// The PROFILE screen

// The PROFILE screen for a new character, then with every setting changed
#[test]
fn profile_screen() {
    let mut h = Harness::with(|w| w.config.keep_exp = false);
    let id = h.player("Ann");
    h.plr_mut(id).lifemins = 125;
    h.plr_mut(id).sesstime = 9;
    let fresh = h.line(id, "profile");
    insta::assert_snapshot!("profile_fresh", fresh);
    assert!(fresh.starts_with("Life for this CHAR  129 minutes\n"));
    assert!(fresh.contains("Keep some exp:      No(Disabled on this system)\n"));
    assert!(!fresh.contains("View gang members"));
    for line in [
        "set statline custom [%h]",
        "set speech",
        "set follow blind",
        "set interrupt on",
        "set receive off",
        "set warning on",
        "set telepath off",
        "set entrances off",
        "set gossip off",
        "set maxgossip 4",
        "set auction off",
        "set maxauction 5",
        "set gang online",
        "set keep on",
        "set messages medium",
        "set page off",
        "set maxpages 6",
        "set responses brief",
        "set style technical",
        "set palette 2",
        "quiet",
    ] {
        h.line(id, line);
    }
    let s = h.engine.world.session_mut(id).unwrap();
    s.flags.sysop = true;
    let c = s.character.as_mut().unwrap();
    c.gang = "Rats".into();
    c.suicpw = "pw".into();
    c.bchannel = 3;
    let changed = h.line(id, "profile");
    insta::assert_snapshot!("profile_changed", changed);
    assert!(changed.contains("Statusline:         [%h]\n"));
    assert!(changed.contains("View gang members:  Online\n"));
    assert!(!changed.contains("suicide password"));
    assert!(changed.contains("SYSOP ACCESS ENABLED\n"));
}

// ---------------------------------------------------------------------------------------
// Persistence

// The settings live in the character, so the logoff save (T-023) carries them and
// `load_player` brings them back
#[test]
fn settings_survive_logoff_and_login() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    for line in [
        "set messages low",
        "set maxpages 9",
        "set follow blind",
        "set interrupt on",
        "set palette 3",
        "set statline off",
        "set entrances off",
        "set talk slow",
    ] {
        h.line(id, line);
    }
    let want = h.plr(id).clone();
    h.engine.world.logoff(id);
    h.engine.flush();
    h.pump();
    let row = h
        .persisted()
        .into_iter()
        .find_map(|p| match p {
            PersistRequest::Character { character, .. } => Some((**character).clone()),
            _ => None,
        })
        .expect("logoff saves the character");
    let w = &mut h.engine.world;
    let again = w.ids.alloc().unwrap();
    let mut s = PlayerSession::new(again, Caps::default(), Instant::now());
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    w.sessions.insert(again, s);
    load_player(w, again, row, TODAY).expect("loads");
    let got = w.session(again).unwrap().player().unwrap();
    assert_eq!(got.msglevel, MessageLevel::Low);
    assert_eq!(
        (got.pagemax, got.palette, got.statline, got.fasttalk),
        (9, 3, StatlineMode::Off, 0)
    );
    assert_eq!(got.settings, want.settings);
    assert_eq!(got.flags, want.flags);
}

// The custom template lives only in the profile; a last-life death keeps it and the next
// character of the account gets it back
#[test]
fn custom_template_survives_death() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    h.line(id, "set statline full custom <%h>");
    let account = h
        .engine
        .world
        .session(id)
        .unwrap()
        .account
        .as_ref()
        .unwrap()
        .id;
    profile::retire(&mut h.engine.world, id, TODAY, Retire::LastLife);
    let w = &mut h.engine.world;
    let c = create_player(w, account, "ANN", TODAY);
    assert_eq!(c.statline, StatlineMode::FullCustom);
    let rec = w.profiles[&account].clone();
    let s = w.sessions.get_mut(&id).unwrap();
    s.ust.statline.clear();
    profile::restore_session(&mut s.ust, &rec, c.deathdate, TODAY);
    assert_eq!(s.ust.statline, "<%h>");
}

// The dispatch arm is T-024's
#[test]
fn set_is_dispatched() {
    assert_eq!(
        silvermere_engine::commands::parse_command("set"),
        Some(Cmd::Set)
    );
}
