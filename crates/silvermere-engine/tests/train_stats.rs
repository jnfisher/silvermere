//! TRAIN STATS and the one-time race and class change (T-047) through the engine:
//! `cmd_train`'s stats branch, the line-mode editor with the first name locked
//! (`edit_character_stats`, with its validation and save), the `SUBST_CHG_*` states and
//! `can_user_change_race`/`one_time_race_change`/`one_time_class_change`.
//!
//! The scripts drive the editor directly through the engine, as a player at a terminal
//! would; no bot ever types TRAIN STATS (CLAUDE.md).
//!
//! Rooms and shops: 1/1376 is the universal trainer (trainclass 0), 1/2 a weapons shop,
//! 1/498 the Witchunter trainer (trainclass 2). Kang (race 11) has the stock ranges of
//! the stock race; Elf (race 5) the minimums of the `creation-line-mode`
//! capture.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, special_rooms, test_config, tiny_snapshot};
use silvermere_engine::world::blank;
use silvermere_engine::{
    AccountRef, Engine, EngineInput, EngineOutput, PersistRequest, SeededRng, SessionId, World,
};
use silvermere_text::strip_ansi_str;
use silvermere_types::enums::{
    Ability, InGame, Magery, MessageLevel, NewCharStep, RaceChangeStep, RoomType, SessionState,
    ShopType,
};
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus2, SpellFlags};
use silvermere_types::ids::{
    AccountId, ClassNo, ItemNo, MessageNo, RaceNo, RoomKey, ShopNo, SpellNo,
};
use silvermere_types::records::{Character, Shop, ShopSlot, Spell};

const TRAINER: u32 = 1376;
const WEAPONS: u32 = 2;
const WITCHUNTER_TRAINER: u32 = 498;
const KANG: RaceNo = RaceNo(11);
const ELF: RaceNo = RaceNo(5);
const WARRIOR: ClassNo = ClassNo(1);
const WITCHUNTER: ClassNo = ClassNo(2);
const MAGE: ClassNo = ClassNo(12);
const BULLSTRENGTH: SpellNo = SpellNo(40);

fn shop(n: i32, name: &str, shop_type: ShopType, trainclass: u16) -> Shop {
    Shop {
        number: ShopNo(n),
        name: name.to_string(),
        desc: Default::default(),
        shop_type,
        minlevel: 1,
        maxlevel: 10,
        markup: 0,
        ownable: 0,
        expandable: 0,
        trainclass: ClassNo(trainclass),
        stock: std::array::from_fn(|_| ShopSlot {
            item: ItemNo(0),
            maxqty: 0,
            qty: 0,
            regentime: 0,
            regennum: 0,
            regenpct: 0,
        }),
    }
}

/// A spell whose first ability slot is Alter Strength (`ABL_STRENGTH`).
fn strength_spell() -> Spell {
    let mut abilities = std::array::from_fn(|_| (Ability::None, 0));
    abilities[0] = (Ability::Strength, 10);
    Spell {
        number: BULLSTRENGTH,
        name: "bullstrength".into(),
        descs: [String::new(), String::new()],
        resist: 0,
        levelcap: 0,
        flags: SpellFlags::empty(),
        abilities,
        energy: 0,
        level: 1,
        minval: 10,
        maxval: 10,
        spell_type: silvermere_types::enums::SpellType::from_code(0),
        resmode: silvermere_types::enums::ResistMode::from_code(0),
        diff: 0,
        durlvl: 0,
        target: silvermere_types::enums::SpellTarget::from_code(0),
        duration: 10,
        element: silvermere_types::enums::Element::from_code(0),
        resstat: silvermere_types::enums::ResistStat::from_code(0),
        magery: Magery::Mage,
        msgnum: MessageNo(0),
        mana: 0,
        maxinc: 0,
        maxlvls: 0,
        magelvl: 0,
        mininc: 0,
        minlvls: 0,
        durinc: 0,
        durlvls: 0,
        shortnm: "bull".into(),
    }
}

fn world() -> World {
    let mut s = tiny_snapshot();
    for (room, shopnum, name) in [
        (TRAINER, 38, "Silvermere Universal Trainer"),
        (WEAPONS, 3, "Newhaven, Weapons Shop"),
        (WITCHUNTER_TRAINER, 19, "Witchunter Training Room"),
    ] {
        let mut r = blank::clear_room(RoomKey::new(1, room));
        r.name = name.to_string();
        r.room_type = RoomType::Shop;
        r.shopnum = ShopNo(shopnum);
        s.rooms.push(r);
    }
    s.shops
        .push(shop(38, "Universal Trainer", ShopType::Training, 0));
    s.shops.push(shop(3, "Weapons Shop", ShopType::Weapons, 0));
    s.shops
        .push(shop(19, "Witchunter Training Room", ShopType::Training, 2));
    s.classes.retain(|c| c.number != WARRIOR);
    for (n, name) in [
        (WARRIOR, "Warrior"),
        (WITCHUNTER, "Witchunter"),
        (MAGE, "Mage"),
    ] {
        let mut c = blank::clear_class(n);
        c.name = name.into();
        c.hplevel = 4;
        c.basehp = 6;
        s.classes.push(c);
    }
    let mut kang = blank::clear_race(KANG);
    kang.name = "Kang".into();
    (kang.minstr, kang.minint, kang.minwil) = (55, 30, 45);
    (kang.minagi, kang.minhea, kang.mincha) = (30, 50, 30);
    (kang.maxstr, kang.maxint, kang.maxwil) = (120, 90, 110);
    (kang.maxagi, kang.maxhea, kang.maxcha) = (90, 110, 85);
    kang.basecp = 100;
    let mut elf = blank::clear_race(ELF);
    elf.name = "Elf".into();
    (elf.minstr, elf.minint, elf.minwil) = (35, 50, 40);
    (elf.minagi, elf.minhea, elf.mincha) = (50, 30, 50);
    (elf.maxstr, elf.maxint, elf.maxwil) = (90, 120, 100);
    (elf.maxagi, elf.maxhea, elf.maxcha) = (120, 70, 120);
    elf.basecp = 100;
    s.races.extend([kang, elf]);
    s.spells.push(strength_spell());
    World::load(test_config(), 8, s).expect("world loads")
}

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
}

impl Harness {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = world();
        w.rng = SeededRng::new(47);
        Self {
            engine: Engine::new(w, TestClock::new(), SeededRng::new(7), tx),
            rx,
            log: Vec::new(),
        }
    }

    fn world(&mut self) -> &mut World {
        &mut self.engine.world
    }

    /// A level-2 Kang Warrior in `room` with 15 CP to spend: Strength 60 (5 CP over the
    /// minimum), the rest at the minimums, `totalcp` 20 (10 at creation, 10 for level 2).
    fn player(&mut self, name: &str, room: u32) -> SessionId {
        let w = &mut self.engine.world;
        let id = enter(w, name, room, MessageLevel::High);
        // 79 columns: the line-mode stat editor (the full-screen one is
        // tests/fullscreen_editor.rs, T-063)
        w.session_mut(id).unwrap().caps.columns = 79;
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
        c.settings |= PlayerSettings::VALIDATED;
        c.flags |= PlayerFlags::RACECHANGED;
        c.version = 22;
        c.lastname = "Stonefist".into();
        c.suicpw = "secret".into();
        c.race = KANG;
        c.class = WARRIOR;
        c.level = 2;
        // experience above 0: the save works out the secondary stats keeping HP; at 0 it
        // would refill them
        (c.exp, c.exprem) = (1700, 1700);
        (c.hp, c.maxhp) = (31, 40);
        set_stats(c, [60, 30, 45, 30, 50, 30]);
        (c.totalcp, c.cp) = (20, 15);
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

    fn state(&self, id: SessionId) -> SessionState {
        self.engine.world.session(id).unwrap().state
    }

    /// Puts the session at the `[MAJORMUD]:` menu, with its row loaded from
    /// the stored record.
    fn at_menu(&mut self, id: SessionId) {
        let s = self.engine.world.session_mut(id).unwrap();
        s.state = SessionState::MainMenu;
        s.ingame = InGame::Outside;
    }

    fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            self.log.push(o);
        }
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

    /// Types `text` on `id`; returns the raw bytes every session received, by session.
    fn send(&mut self, id: SessionId, text: &str) -> usize {
        let from = self.log.len();
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.pump();
        from
    }

    fn bytes(&mut self, id: SessionId, text: &str) -> Vec<u8> {
        let from = self.send(id, text);
        self.written(from, id)
    }

    fn line(&mut self, id: SessionId, text: &str) -> String {
        plain(&self.bytes(id, text))
    }

    /// The character records saved since `from`, oldest first.
    fn saves(&self, from: usize) -> Vec<Character> {
        self.log[from..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Persist(PersistRequest::Character { character, .. }) => {
                    Some((**character).clone())
                }
                _ => None,
            })
            .collect()
    }
}

fn set_stats(c: &mut Character, v: [u16; 6]) {
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (v[0], v[1], v[2], v[3], v[4], v[5]);
    (c.basstr, c.basint, c.baswil) = (v[0], v[1], v[2]);
    (c.basagi, c.bashea, c.bascha) = (v[3], v[4], v[5]);
}

fn stats(c: &Character) -> [u16; 6] {
    [c.str, c.intel, c.wil, c.agi, c.hea, c.cha]
}

fn plain(b: &[u8]) -> String {
    strip_ansi_str(&String::from_utf8_lossy(b)).replace("\r\n", "\n")
}

fn esc(b: &[u8]) -> String {
    String::from_utf8_lossy(b)
        .replace('\x1b', "\\x1b")
        .replace("\r\n", "\\r\\n\n")
}

/// Answers every field after the family name with Enter, then `exit` at the last field.
/// Returns the output of the last answer.
fn finish(h: &mut Harness, id: SessionId, exit: &str) -> String {
    // Intellect .. Charm, hair length, hair colour, eye colour
    for _ in 0..8 {
        h.line(id, "");
    }
    h.line(id, exit)
}

mod training {
    pub mod stats {
        use super::super::*;

        // cmd_train; edit_character_stats with the first name locked; the points used; the
        // save.
        // Strength 60 -> 70 over Kang's minimum 55 crosses the first band at 65: 5 points
        // at 1 CP and 5 at 2 CP, 15 CP. (Ten points across a band boundary cannot cost
        // 10 CP, so the level-2 character carries 5 CP left over from creation.)
        #[test]
        fn universal_trainer_raises_strength_across_a_band() {
            let mut h = Harness::new();
            let id = h.player("Kraag", TRAINER);
            let watcher = h.player("Watcher", special_rooms()[0]);
            let room_before = h.plr(id).room;
            let from = h.send(id, "train stats");
            let out = plain(&h.written(from, id));
            assert!(out.contains("     Character Generator\n"));
            assert!(out.contains("\nYou have 15 CP to distribute.\n"));
            // no first-name field: the cursor starts at the family name
            assert!(out.ends_with("Family name [Stonefist]: "), "{out}");
            assert_eq!(
                plain(&h.written(from, watcher)),
                "Kraag just left the Realm.\n[HP=31]:"
            );
            assert_eq!(h.saves(from).len(), 1, "saved before the editor");
            assert_eq!(h.state(id), SessionState::NewChar(NewCharStep::Stats));
            assert!(!h.world().is_in_game(id), "out of the realm in the editor");

            assert_eq!(
                h.line(id, ""),
                "\nYou have 15 CP to distribute.\nStrength (55-120) [60]: "
            );
            assert_eq!(
                h.line(id, "70"),
                "\nYou have 0 CP to distribute.\nIntellect (30-90) [30]: "
            );
            let from = h.log.len();
            let out = finish(&mut h, id, "SAVE");
            assert!(out.contains("Kraag"), "show_status: {out}");
            assert!(out.ends_with("[HP=31]:"), "{out}");
            assert_eq!(
                plain(&h.written(from, watcher)),
                "Kraag just entered the Realm.\n[HP=31]:"
            );
            let saved = h.saves(from);
            assert_eq!(saved.len(), 1);
            let rec = &saved[0];
            assert_eq!(stats(rec), [70, 30, 45, 30, 50, 30]);
            assert_eq!(rec.basstr, 70);
            assert_eq!((rec.cp, rec.totalcp), (0, 20));
            assert_eq!(h.state(id), SessionState::Playing);
            assert!(h.world().is_in_game(id));
            assert_eq!(h.plr(id).room, room_before);
            assert_eq!(h.plr(id).hp, 31);
        }

        // A shop that is not a trainer
        #[test]
        fn weapon_shop_refuses() {
            let mut h = Harness::new();
            let id = h.player("Kraag", WEAPONS);
            let from = h.log.len();
            let bytes = h.bytes(id, "train stats");
            assert_eq!(
                plain(&bytes),
                "You may not train your stats here!\n[HP=31]:"
            );
            assert!(esc(&bytes).contains("\\x1b[79D\\x1b[K\\x1b[1;31mYou may not train"));
            assert!(h.saves(from).is_empty());
            assert_eq!(h.state(id), SessionState::Playing);
        }

        // An Alter Strength spell on the character
        #[test]
        fn altered_stats_refuse() {
            let mut h = Harness::new();
            let id = h.player("Kraag", TRAINER);
            h.plr_mut(id).spells[0] = BULLSTRENGTH;
            h.plr_mut(id).spellvals[0] = 10;
            assert_eq!(
                h.line(id, "train stats"),
                "Your stats are unnaturally altered!  You may not train stats now.\n[HP=31]:"
            );
            assert_eq!(h.state(id), SessionState::Playing);
        }

        // The refusal order, each fixture failing every test after
        // the one it shows: tournament on (a buffed player in a trainer of another class),
        // a non-shop room (buffed), a buffed player in a trainer, a trainer of another
        // class.
        #[test]
        fn refusal_order() {
            let mut out = String::new();
            for (what, room, tournament, buffed) in [
                ("tournament", WITCHUNTER_TRAINER, true, true),
                ("not a shop", special_rooms()[0], false, true),
                ("altered", WITCHUNTER_TRAINER, false, true),
                ("another class", WITCHUNTER_TRAINER, false, false),
            ] {
                let mut h = Harness::new();
                h.world().config.tournament_play = tournament;
                let id = h.player("Kraag", room);
                if buffed {
                    h.plr_mut(id).spells[0] = BULLSTRENGTH;
                }
                out.push_str(&format!("{what}:\n{}\n", esc(&h.bytes(id, "train stats"))));
            }
            insta::assert_snapshot!("train_stats_refusal_order", out);
        }

        // With cp 0 the editor opens and SAVE
        // leaves the record as it was; the room hears the left/entered pair.
        #[test]
        fn no_cp_opens_and_leaves_unchanged() {
            let mut h = Harness::new();
            let id = h.player("Kraag", TRAINER);
            let watcher = h.player("Watcher", TRAINER);
            h.plr_mut(id).cp = 0;
            set_stats(h.plr_mut(id), [65, 30, 45, 30, 50, 30]);
            h.plr_mut(id).totalcp = 10;
            let from = h.send(id, "train stats");
            assert!(plain(&h.written(from, id)).contains("You have 0 CP to distribute."));
            assert!(plain(&h.written(from, watcher)).contains("Kraag just left the Realm."));
            // the room no longer shows the player
            let look = h.line(watcher, "look");
            assert!(!look.contains("Kraag"), "{look}");
            let from = h.log.len();
            h.line(id, "");
            h.line(id, "");
            finish(&mut h, id, "save");
            assert!(plain(&h.written(from, watcher)).contains("Kraag just entered the Realm."));
            let rec = h.saves(from).pop().expect("saved");
            assert_eq!(stats(&rec), [65, 30, 45, 30, 50, 30]);
            assert_eq!((rec.cp, rec.totalcp), (0, 10));
        }

        // QUIT re-enters unchanged, with the entered line, unsaved
        #[test]
        fn quit_reenters_unchanged() {
            let mut h = Harness::new();
            let id = h.player("Kraag", TRAINER);
            let watcher = h.player("Watcher", special_rooms()[0]);
            h.line(id, "train stats");
            h.line(id, "");
            h.line(id, "70");
            let from = h.log.len();
            let out = finish(&mut h, id, "quit");
            assert_eq!(out, "[HP=31]:");
            assert_eq!(
                plain(&h.written(from, watcher)),
                "Kraag just entered the Realm.\n[HP=31]:"
            );
            assert!(h.saves(from).is_empty());
            assert_eq!(stats(h.plr(id)), [60, 30, 45, 30, 50, 30]);
            assert_eq!(h.plr(id).cp, 15);
            assert_eq!(h.state(id), SessionState::Playing);
        }

        // The validation: never below the current value, CP kept when unspent
        #[test]
        fn never_lower_and_unspent_cp_kept() {
            let mut h = Harness::new();
            let id = h.player("Kraag", TRAINER);
            h.line(id, "train stats");
            h.line(id, "");
            assert_eq!(
                h.line(id, "59"),
                "Strength may not be lower than 60.\nStrength (55-120) [60]: "
            );
            assert_eq!(
                h.line(id, "71"),
                "You may not assign that much to Strength\nStrength (55-120) [60]: "
            );
            h.line(id, "62");
            let from = h.log.len();
            finish(&mut h, id, "save");
            let rec = h.saves(from).pop().expect("saved");
            assert_eq!(rec.str, 62);
            assert_eq!(rec.cp, 13, "15 - 2: 60 -> 62 at 1 CP each");
        }

        // The SET SUICIDE notice reaches an existing character too
        #[test]
        fn suicide_notice_for_an_existing_character() {
            let mut h = Harness::new();
            let id = h.player("Kraag", TRAINER);
            h.plr_mut(id).suicpw.clear();
            h.line(id, "train stats");
            h.line(id, "");
            h.line(id, "");
            let out = finish(&mut h, id, "save");
            let pos = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s}: {out}"));
            assert!(pos("SET SUICIDE command.") < pos("[HP=31]:"));
        }

        // T-023: a
        // character whose stats were reset meets the editor at E; SAVE clears the mark and
        // enters, QUIT goes back to the menu.
        #[test]
        fn stat_reset_goes_through_the_editor_at_entry() {
            for (exit, menu) in [("save", false), ("quit", true)] {
                let mut h = Harness::new();
                let id = h.player("Kraag", TRAINER);
                h.plr_mut(id).cp = -1;
                h.at_menu(id);
                let out = h.line(id, "e");
                assert!(
                    out.contains("Please TRAIN STATS as soon as possible"),
                    "{out}"
                );
                assert!(out.contains("Character Generator"), "{out}");
                assert!(out.ends_with("Family name [Stonefist]: "), "{out}");
                assert!(
                    h.plr(id).status2.contains(PlayerStatus2::STATRESET),
                    "load_player marks it"
                );
                h.line(id, "");
                h.line(id, "");
                let out = finish(&mut h, id, exit);
                if menu {
                    assert!(out.contains("[MAJORMUD]:"), "{out}");
                    assert_eq!(h.state(id), SessionState::MainMenu);
                    assert!(h.plr(id).status2.contains(PlayerStatus2::STATRESET));
                } else {
                    assert!(out.ends_with("]:"), "{out}");
                    assert_eq!(h.state(id), SessionState::Playing);
                    assert!(!h.plr(id).status2.contains(PlayerStatus2::STATRESET));
                }
            }
        }

        // Against tests/captures/creation-line-mode (the original's
        // line-mode editor, NAWS 79x24): the chart, the six range lines with the (sic)
        // maximum of 0, and the CP line, byte for byte, one CR LF per line. An Elf with
        // 100 CP, as the capture's Corvin.
        #[test]
        fn chart_matches_the_line_mode_capture() {
            const CAPTURE: &[u8] =
                include_bytes!("../../../tests/captures/creation-line-mode/raw.bin");
            const START: &[u8] = b"\r\n     M A J O R   M U D";
            const END: &[u8] = b"CP to distribute.\r\n";
            fn block(b: &[u8]) -> &[u8] {
                let a = b
                    .windows(START.len())
                    .position(|w| w == START)
                    .expect("chart start");
                let e = b[a..]
                    .windows(END.len())
                    .position(|w| w == END)
                    .expect("chart end")
                    + a
                    + END.len();
                &b[a..e]
            }
            let mut h = Harness::new();
            let id = h.player("Corvin", TRAINER);
            let c = h.plr_mut(id);
            c.race = ELF;
            c.class = MAGE;
            set_stats(c, [35, 50, 40, 50, 30, 50]);
            (c.totalcp, c.cp) = (100, 100);
            let out = h.bytes(id, "train stats");
            let got = block(&out);
            assert_eq!(esc(got), esc(block(CAPTURE)));
            // CR LF exactly once per line: no bare CR or LF, no doubled CR
            for (i, w) in got.windows(2).enumerate() {
                assert_eq!(w[0] == b'\r', w[1] == b'\n', "at {i}");
            }
            insta::assert_snapshot!("train_stats_editor_screen", esc(&out));
        }
    }
}

mod race_change {
    use super::*;

    /// A Kang Warrior at the menu who never changed race (a pre-1.11 record).
    fn unchanged(h: &mut Harness) -> SessionId {
        let id = h.player("Kraag", special_rooms()[0]);
        h.plr_mut(id).flags.remove(PlayerFlags::RACECHANGED);
        h.plr_mut(id).lifemins = 1000;
        h.at_menu(id);
        id
    }

    fn questions(h: &mut Harness, id: SessionId) -> Vec<String> {
        vec![
            h.line(id, "changerace"),
            h.line(id, "yes"),
            h.line(id, "YES"),
            h.line(id, "yes"),
        ]
    }

    #[test]
    fn three_questions_then_the_race() {
        let mut h = Harness::new();
        let id = unchanged(&mut h);
        let q = questions(&mut h, id);
        assert_eq!(
            q[0],
            "\nHave you read all the help files on the races and classes?\n(You must answer YES to continue): "
        );
        assert_eq!(
            q[1],
            "\nDo you understand that you have only a short time to change the decision you are making?\n(You must answer YES to continue): "
        );
        assert_eq!(
            q[2],
            "\nDo you understand that your sysop can never change your race or class for you?\n(You must answer YES to continue): "
        );
        assert!(q[3].starts_with("Please choose a race from the following list:\n"));
        assert!(q[3].contains("[5]  Elf\n"), "{}", q[3]);
        assert_eq!(h.state(id), SessionState::RaceChange(RaceChangeStep::Race));
        let from = h.log.len();
        let done = h.line(id, "5");
        assert_eq!(
            done,
            "You have now completed your race change.\nDon't forget that there is a time limit after which you cannot change again.\nPress <enter> to continue.\n"
        );
        assert_eq!(h.state(id), SessionState::MainMenu);
        let rec = h.saves(from).pop().expect("the record is written");
        assert_eq!(rec.race, ELF);
        assert!(rec.flags.contains(PlayerFlags::RACECHANGED));
        assert_eq!((rec.cp, rec.racechg), (-1, 1000));
        // Enter draws the menu, with the undo offer
        assert!(
            h.line(id, "")
                .contains("[CHANGERACE] . Race Change [300 game minutes left]")
        );
    }

    // Undo within racechgdur minutes of play, refused after
    #[test]
    fn undo_within_the_window_then_refused() {
        let mut h = Harness::new();
        let id = unchanged(&mut h);
        questions(&mut h, id);
        h.line(id, "5");
        // 299 minutes later: still open, back to Kang; the change time is kept from the first
        h.plr_mut(id).lifemins += 299;
        questions(&mut h, id);
        assert!(h.line(id, "11").contains("completed your race change"));
        assert_eq!((h.plr(id).race, h.plr(id).racechg), (KANG, 1000));
        assert!(h.line(id, "").contains("[1 game minutes left]"));
        // one more minute and a half: closed; CHANGERACE is the menu's C
        h.plr_mut(id).lifemins += 2;
        let menu = h.line(id, "");
        assert!(!menu.contains("CHANGERACE"), "{menu}");
        let out = h.line(id, "changerace");
        assert!(!out.contains("Have you read"), "{out}");
        assert_eq!(h.state(id), SessionState::MainMenu);
    }

    // PF_RACECHANGED with no change time, as every character made since
    #[test]
    fn a_spent_change_is_refused() {
        let mut h = Harness::new();
        let id = h.player("Kraag", special_rooms()[0]);
        h.at_menu(id);
        assert!(!h.line(id, "").contains("CHANGERACE"));
        assert!(!h.line(id, "changerace").contains("Have you read"));
        assert_eq!(h.state(id), SessionState::MainMenu);
    }

    // Anything but YES is the menu
    #[test]
    fn not_yes_is_the_menu() {
        for answer in ["", "no", "y"] {
            let mut h = Harness::new();
            let id = unchanged(&mut h);
            h.line(id, "changerace");
            let out = h.line(id, answer);
            assert!(out.contains("[MAJORMUD]:"), "{answer:?}: {out}");
            assert_eq!(h.state(id), SessionState::MainMenu);
        }
    }

    // No word, X, an unknown race
    #[test]
    fn race_prompt_refusals() {
        let mut h = Harness::new();
        let id = unchanged(&mut h);
        questions(&mut h, id);
        assert_eq!(h.line(id, ""), "You must choose a race. [ ? for help ]");
        assert_eq!(
            h.line(id, "99"),
            "You must choose a valid race. [ ? for help ]"
        );
        assert_eq!(h.plr(id).race, KANG);
        assert!(h.line(id, "x").contains("[MAJORMUD]:"));
        assert!(!h.plr(id).flags.contains(PlayerFlags::RACECHANGED));
    }

    // ENABLE_CLASS_CHANGE on
    #[test]
    fn class_change_when_enabled() {
        let mut h = Harness::new();
        h.world().config.enable_class_change = true;
        let id = unchanged(&mut h);
        h.plr_mut(id).randomhp = 4;
        assert!(h.line(id, "").contains("[CHANGERACE] . Race/Class Change"));
        questions(&mut h, id);
        let out = h.line(id, "5");
        assert!(
            out.starts_with("Please choose a class from the following list:\n"),
            "{out}"
        );
        assert_eq!(h.state(id), SessionState::RaceChange(RaceChangeStep::Class));
        let bad = h.bytes(id, "99");
        assert_eq!(bad, b"You must choose a valid class. [ ? for help ]");
        let from = h.log.len();
        assert_eq!(
            h.line(id, "12"),
            "You have now completed your class change.\nDon't forget that there is a time limit after which you cannot change again.\nPress <enter> to continue.\n"
        );
        let rec = h.saves(from).pop().expect("saved");
        assert_eq!(
            (rec.race, rec.class, rec.cp, rec.randomhp),
            (ELF, MAGE, -1, 0)
        );
        assert_eq!(h.state(id), SessionState::MainMenu);
    }

    // The change sets cp to -1, so (T-023) the next E
    // rolls the stats for the new race and opens the editor
    #[test]
    fn the_next_entry_retrains_for_the_new_race() {
        let mut h = Harness::new();
        let id = unchanged(&mut h);
        questions(&mut h, id);
        h.line(id, "5");
        let out = h.line(id, "e");
        assert!(out.contains("Your stats have been reset"), "{out}");
        assert!(out.contains("Strength  35 - 0\n"), "{out}");
        assert_eq!(h.state(id), SessionState::NewChar(NewCharStep::Stats));
    }

    // The prompts and the done texts, as bytes.
    #[test]
    fn prompts_snapshot() {
        let mut h = Harness::new();
        h.world().config.enable_class_change = true;
        let id = unchanged(&mut h);
        let mut out = String::new();
        for line in ["changerace", "yes", "yes", "yes", "", "5", "", "12"] {
            out.push_str(&format!("> {line}\n{}\n", esc(&h.bytes(id, line))));
        }
        insta::assert_snapshot!("race_change_prompts", out);
    }
}
