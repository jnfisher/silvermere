//! Experience, training and the top list (T-039) through the engine: `add_experience`,
//! TRAIN (`cmd_train`; `train_level`) and the top list.
//!
//! The rooms, shops, race and class are the stock records (WCCMP001, WCCSHOPS, WCCRACE,
//! WCCCLASS): room 1/2147 "Newhaven, Adventurer's Guild" is shop 38, the universal trainer
//! for levels 1-10 at no markup; room 1/498 is shop 19, the Witchunter trainer for levels
//! 1-10 at 300%; shop 49 is the Super Warrior Trainer for level 11 only. Kang (race 11,
//! exp 35) Witchunter (class 2, exp 30, 4 hit points a level) is the +65% character whose
//! level 2 needs 1,650.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, special_rooms, test_config, tiny_snapshot};
use silvermere_engine::player::profile::{self, Retire};
use silvermere_engine::progression::{ExpOutcome, TopTen, add_experience};
use silvermere_engine::world::blank;
use silvermere_engine::{
    AccountRef, Engine, EngineInput, EngineOutput, GameEvent, SeededRng, SessionId, World,
};
use silvermere_rules::progression::{kill_exp, new_calc_exp_needed};
use silvermere_text::strip_ansi_str;
use silvermere_types::enums::{MessageLevel, RoomType, ShopType};
use silvermere_types::flags::{PlayerSettings, PlayerStatus};
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, RaceNo, RoomKey, ShopNo};
use silvermere_types::records::{BigExp, Character, Coins, Shop, ShopSlot};

const GUILD: u32 = 2147;
const WITCHUNTER_TRAINER: u32 = 498;
const SUPER_TRAINER: u32 = 1301;
const KANG: RaceNo = RaceNo(11);
const WITCHUNTER: ClassNo = ClassNo(2);
const WARRIOR: ClassNo = ClassNo(1);

fn shop(n: i32, name: &str, minlevel: u16, maxlevel: u16, markup: u16, trainclass: u16) -> Shop {
    Shop {
        number: ShopNo(n),
        name: name.to_string(),
        desc: Default::default(),
        shop_type: ShopType::Training,
        minlevel,
        maxlevel,
        markup,
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

/// The tiny world plus the three trainers' rooms, their shops, Kang and the two classes.
fn world() -> World {
    crate::common::install_text();
    let mut s = tiny_snapshot();
    for (room, shopnum, name) in [
        (GUILD, 38, "Newhaven, Adventurer's Guild"),
        (WITCHUNTER_TRAINER, 19, "Witchunter Training Room"),
        (SUPER_TRAINER, 49, "Super Warrior Trainer"),
    ] {
        let mut r = blank::clear_room(RoomKey::new(1, room));
        r.name = name.to_string();
        r.room_type = RoomType::Shop;
        r.shopnum = ShopNo(shopnum);
        s.rooms.push(r);
    }
    s.shops.push(shop(38, "Training Room", 1, 10, 0, 0));
    s.shops
        .push(shop(19, "Witchunter Training Room", 1, 10, 300, 2));
    s.shops
        .push(shop(49, "Super Warrior Trainer", 11, 11, 500, 1));
    s.classes.retain(|c| c.number != WARRIOR);
    let mut warrior = blank::clear_class(WARRIOR);
    warrior.name = "Warrior".into();
    warrior.hplevel = 4;
    warrior.basehp = 6;
    let mut witchunter = blank::clear_class(WITCHUNTER);
    witchunter.name = "Witchunter".into();
    witchunter.exp = 30;
    witchunter.hplevel = 4;
    witchunter.basehp = 6;
    let mut kang = blank::clear_race(KANG);
    kang.name = "Kang".into();
    kang.exp = 35;
    s.classes.extend([warrior, witchunter]);
    s.races.push(kang);
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
        w.rng = SeededRng::new(39);
        Self {
            engine: Engine::new(w, TestClock::new(), SeededRng::new(7), tx),
            rx,
            log: Vec::new(),
        }
    }

    fn world(&mut self) -> &mut World {
        &mut self.engine.world
    }

    /// A Kang Witchunter at level 1 in `room`, 48 hit points, nine lives, no coins.
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
        c.race = KANG;
        c.class = WITCHUNTER;
        c.level = 1;
        (c.hp, c.maxhp) = (48, 48);
        c.lives = 9;
        c.coins = Coins::default();
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

    /// Types `text`; returns the raw bytes the session received.
    fn bytes(&mut self, id: SessionId, text: &str) -> Vec<u8> {
        let from = self.log.len();
        self.engine.handle(EngineInput::Line {
            id,
            text: text.into(),
        });
        self.pump();
        self.written(from, id)
    }

    /// Types `text`; returns what the session received, ANSI stripped, `\r\n` as `\n`.
    fn line(&mut self, id: SessionId, text: &str) -> String {
        let b = self.bytes(id, text);
        plain(&b)
    }

    /// Runs `f` on the world as one engine event would and returns the session's output.
    fn during(&mut self, id: SessionId, f: impl FnOnce(&mut World)) -> String {
        plain(&self.during_bytes(id, f))
    }

    fn during_bytes(&mut self, id: SessionId, f: impl FnOnce(&mut World)) -> Vec<u8> {
        f(&mut self.engine.world);
        let from = self.log.len();
        self.engine.flush();
        self.pump();
        self.written(from, id)
    }

    fn events(&self) -> Vec<&GameEvent> {
        self.log
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Event(e) => Some(e),
                _ => None,
            })
            .collect()
    }
}

fn plain(b: &[u8]) -> String {
    strip_ansi_str(&String::from_utf8_lossy(b)).replace("\r\n", "\n")
}

/// Raw bytes with escapes and line ends visible, for the snapshots.
fn esc(b: &[u8]) -> String {
    String::from_utf8_lossy(b)
        .replace('\x1b', "\\x1b")
        .replace("\r\n", "\\r\\n\n")
}

const PROMPT: &str = "[HP=48]:";

// ---------------------------------------------------------------------------------------
// add_experience

// exp*level, the gain line and the prompt.
#[test]
fn a_kill_pays_exp_times_level_with_the_gain_line() {
    let mut h = Harness::new();
    let id = h.player("Kraag", special_rooms()[0]);
    let exp = kill_exp(15, 3);
    assert_eq!(exp, 45);
    let mut outcome = None;
    let bytes = h.during_bytes(id, |w| {
        outcome = Some(add_experience(w, id, exp as i32, true));
    });
    insta::assert_snapshot!("exp_gain", esc(&bytes));
    let out = plain(&bytes);
    assert_eq!(
        outcome,
        Some(ExpOutcome {
            added: true,
            capped: false
        })
    );
    assert_eq!(out, format!("You gain 45 experience.\n{PROMPT}"));
    assert_eq!(
        (h.plr(id).expbil, h.plr(id).exprem, h.plr(id).exp),
        (0, 45, 45)
    );
    // A silent gain (verbose 0) adds without a word.
    let out = h.during(id, |w| {
        add_experience(w, id, 5, false);
    });
    assert_eq!(out, "");
    assert_eq!(h.plr(id).exprem, 50);
}

// The cap is new_calc_exp_needed(level + xtralvl): for a level-1 Kang
// Witchunter with XTRALVL 1, 3,300 (the experience of level 3). Exactly the cap may still
// earn; past it, the gain is refused with the line and the prompt and adds nothing.
#[test]
fn the_cap_refuses_past_the_experience_for_level_plus_xtralvl() {
    let mut h = Harness::new();
    let id = h.player("Kraag", special_rooms()[0]);
    // XTRALVL 1, the module's own default (the stock WCCMMUD.MSG sets 2).
    h.world().config.xtra_lvl = 1;
    assert_eq!(
        new_calc_exp_needed(2, 65),
        BigExp {
            expbil: 0,
            exprem: 3_300
        }
    );
    h.plr_mut(id).exprem = 3_300;
    let out = h.during(id, |w| {
        add_experience(w, id, 10, true);
    });
    assert_eq!(out, format!("You gain 10 experience.\n{PROMPT}"));
    assert_eq!(h.plr(id).exprem, 3_310);
    let mut outcome = None;
    let bytes = h.during_bytes(id, |w| {
        outcome = Some(add_experience(w, id, 10, true));
    });
    insta::assert_snapshot!("exp_capped", esc(&bytes));
    assert_eq!(
        plain(&bytes),
        format!(
            "You have progressed too far without training to acquire anymore experience!\n{PROMPT}"
        )
    );
    assert_eq!(
        outcome,
        Some(ExpOutcome {
            added: false,
            capped: true
        })
    );
    assert_eq!(h.plr(id).exprem, 3_310);
    // A silent call is refused too (only the billions-alone case lets it through), and
    // says nothing more than the refusal line does; a loss (0 or less) is never capped.
    let out = h.during(id, |w| {
        add_experience(w, id, 10, false);
    });
    assert!(out.contains("too far without training"), "{out}");
    assert_eq!(h.plr(id).exprem, 3_310);
}

// (sic) The amount is compared with a billion unsigned, so -5 counts
// as 4,294,967,291: four billions and 294,967,291, and the line prints that remainder.
#[test]
fn a_negative_amount_wraps_as_the_original_did() {
    let mut h = Harness::new();
    let id = h.player("Kraag", special_rooms()[0]);
    let out = h.during(id, |w| {
        add_experience(w, id, -5, true);
    });
    assert_eq!(out, format!("You gain 294967291 experience.\n{PROMPT}"));
    assert_eq!((h.plr(id).expbil, h.plr(id).exprem), (4, 294_967_291));
}

// cmd_experience shows new_calc_exp_needed(level, race+class): the T-027
// snapshot `info_commands__exp_aldric` has a level-1 Human Warrior (+0) with no
// experience at "1000 (1000) [0%]"; the Kang Witchunter needs 1,650.
#[test]
fn exp_figures_match_the_exp_display() {
    assert_eq!(
        new_calc_exp_needed(1, 0),
        BigExp {
            expbil: 0,
            exprem: 1_000
        }
    );
    assert_eq!(new_calc_exp_needed(1, 65).exprem, 1_650);
}

// ---------------------------------------------------------------------------------------
// TRAIN

// At the Adventurer's Guild: 5 silver for level 1 at no markup, the
// receipt in order, level 2, +10 CP, 0-4 hit points, lives already at 9 so no lives line.
#[test]
fn train_at_the_guild_from_level_1() {
    let mut h = Harness::new();
    let id = h.player("Kraag", GUILD);
    h.plr_mut(id).exprem = 1_650;
    h.plr_mut(id).coins.silver = 5;
    let (cp, totalcp) = (h.plr(id).cp, h.plr(id).totalcp);
    let bytes = h.bytes(id, "train");
    insta::assert_snapshot!("train_receipt", esc(&bytes));
    assert_eq!(
        plain(&bytes),
        format!(
            "You hand over 5 silver nobles and you receive training to attain level 2.\n\
             You receive the following:\n\
             10 additional character points\n{PROMPT}"
        )
    );
    let p = h.plr(id);
    assert_eq!(p.level, 2);
    assert_eq!(p.cp, cp + 10);
    assert_eq!(p.totalcp, totalcp + 10);
    assert!(p.randomhp <= 4, "randomhp {}", p.randomhp);
    assert_eq!(p.lives, 9);
    assert_eq!(p.coins, Coins::default());
    assert!(h.events().contains(&&GameEvent::Level { id, level: 2 }));
    assert!(h.engine.world.topten.iter().any(|e| e.userid == "KRAAG"));
}

// A roll of 0 to hplevel+1: 0..=4 for 4 hit points a level, every value
// reached over many trainings.
#[test]
fn train_hit_points_are_0_to_hplevel() {
    let mut h = Harness::new();
    let id = h.player("Kraag", GUILD);
    let mut seen = [false; 5];
    for _ in 0..200 {
        let p = h.plr_mut(id);
        (p.level, p.exprem, p.randomhp) = (1, 1_650, 0);
        p.coins.silver = 5;
        h.line(id, "train");
        let hp = usize::from(h.plr(id).randomhp);
        assert!(hp <= 4, "{hp}");
        seen[hp] = true;
    }
    assert_eq!(seen, [true; 5]);
}

// +5 lives, capped at 9, the line giving the change.
#[test]
fn train_lives_line_only_when_they_changed() {
    for (before, after, line) in [(3, 8, Some(5)), (6, 9, Some(3)), (9, 9, None)] {
        let mut h = Harness::new();
        let id = h.player("Kraag", GUILD);
        let p = h.plr_mut(id);
        (p.exprem, p.lives) = (1_650, before);
        p.coins.silver = 5;
        let out = h.line(id, "train");
        assert_eq!(h.plr(id).lives, after);
        match line {
            Some(n) => assert!(
                out.ends_with(&format!(
                    "10 additional character points\n{n} additional lives\n{PROMPT}"
                )),
                "{out}"
            ),
            None => assert!(!out.contains("additional lives"), "{out}"),
        }
    }
}

// A class trainer marks up 300%: 20 silver at level 1; 5 is short.
#[test]
fn a_class_trainer_charges_its_markup() {
    let mut h = Harness::new();
    let id = h.player("Kraag", WITCHUNTER_TRAINER);
    h.plr_mut(id).exprem = 1_650;
    h.plr_mut(id).coins.silver = 5;
    let out = h.line(id, "train");
    assert_eq!(
        out,
        format!("You do not have the money required for your training.\n{PROMPT}")
    );
    assert_eq!(h.plr(id).level, 1);
    h.plr_mut(id).coins = Coins {
        gold: 1,
        silver: 15,
        ..Coins::default()
    };
    let out = h.line(id, "train");
    assert!(
        out.starts_with(
            "You hand over 1 gold crown, 10 silver nobles and you receive training to attain level 2.\n"
        ),
        "{out}"
    );
    assert_eq!(h.plr(id).coins.silver, 5);
}

// Each refusal, in the original's order.
#[test]
fn every_refusal() {
    let mut h = Harness::new();
    // Not a shop room
    let id = h.player("Kraag", special_rooms()[0]);
    h.plr_mut(id).exprem = 1_650;
    h.plr_mut(id).coins.silver = 100;
    assert_eq!(
        h.line(id, "train"),
        format!("You must be in an appropriate training room to train!\n{PROMPT}")
    );
    // A shop room whose shop is not a trainer
    h.world().shops.get_mut(&ShopNo(38)).unwrap().shop_type = ShopType::General;
    h.plr_mut(id).room = silvermere_types::ids::RoomId(GUILD);
    assert_eq!(
        h.line(id, "train"),
        format!("You must be in an appropriate training room to train!\n{PROMPT}")
    );
    h.world().shops.get_mut(&ShopNo(38)).unwrap().shop_type = ShopType::Training;
    // Another class's trainer
    h.plr_mut(id).room = silvermere_types::ids::RoomId(SUPER_TRAINER);
    assert_eq!(
        h.line(id, "train"),
        format!("This shop is not suitable for your training.\n{PROMPT}")
    );
    // Below the trainer's minimum (level 11 only; a level-1 Warrior)
    h.plr_mut(id).class = WARRIOR;
    assert_eq!(
        h.line(id, "train"),
        format!("You have not progressed far enough to use the training provided here.\n{PROMPT}")
    );
    // A level 11 at the guild (levels 1-10)
    let p = h.plr_mut(id);
    (p.class, p.room, p.level) = (WITCHUNTER, silvermere_types::ids::RoomId(GUILD), 11);
    assert_eq!(
        h.line(id, "train"),
        format!("You have progressed too far to use the training provided here.\n{PROMPT}")
    );
    // Not enough experience: refused before the money is looked at
    let p = h.plr_mut(id);
    (p.level, p.exprem, p.coins) = (1, 1_649, Coins::default());
    assert_eq!(
        h.line(id, "train"),
        format!("You do not have the required experience to train yet!\n{PROMPT}")
    );
    // The experience but not the money
    h.plr_mut(id).exprem = 1_650;
    assert_eq!(
        h.line(id, "train"),
        format!("You do not have the money required for your training.\n{PROMPT}")
    );
    assert_eq!(h.plr(id).level, 1);
}

// The range is checked against the next level: a level 4 at the
// guild (1-10) trains; so does a level 9 (to 10); a level 10 (to 11) is refused.
#[test]
fn the_trainer_range_is_the_next_level() {
    for (level, ok) in [(4u16, true), (9, true), (10, false)] {
        let mut h = Harness::new();
        let id = h.player("Kraag", GUILD);
        let need = new_calc_exp_needed(level as i16, 65);
        let p = h.plr_mut(id);
        (p.level, p.expbil, p.exprem) = (level, need.expbil, need.exprem);
        p.coins.gold = 100;
        let out = h.line(id, "train");
        assert_eq!(h.plr(id).level, if ok { level + 1 } else { level }, "{out}");
        if ok {
            assert!(out.contains(&format!("training to attain level {}.", level + 1)));
        } else {
            assert!(
                out.starts_with("You have progressed too far to use"),
                "{out}"
            );
        }
    }
}

// Tournament mode refuses TRAIN and TRAIN STATS,
// resting ends, TRAIN STATS opens the stat editor (T-047).
#[test]
fn cmd_train_tournament_rest_and_stats() {
    let mut h = Harness::new();
    let id = h.player("Kraag", GUILD);
    h.plr_mut(id).status.insert(PlayerStatus::RESTING);
    h.line(id, "train");
    assert!(!h.plr(id).status.contains(PlayerStatus::RESTING));
    h.world().config.tournament_play = true;
    assert_eq!(
        h.line(id, "train"),
        format!("You may not do any training while in tournament mode!\n{PROMPT}")
    );
    assert_eq!(
        h.line(id, "train stats"),
        format!("You may not do any training while in tournament mode!\n{PROMPT}")
    );
    // T-047: out of tournament mode the stat editor opens (tests/train_stats.rs).
    h.world().config.tournament_play = false;
    assert!(crate::common::holds_moved(
        &h.line(id, "train stats"),
        "stat_editor_preamble"
    ));
}

// ---------------------------------------------------------------------------------------
// The top list

fn order(list: &TopTen) -> Vec<&str> {
    list.iter().map(|e| e.userid.as_str()).collect()
}

// update_top_list after each gain: highest first, billions before the
// remainder; a tie stays below the one already there.
#[test]
fn top_list_order_after_three_characters_gain() {
    let mut h = Harness::new();
    let a = h.player("Ann", special_rooms()[0]);
    let b = h.player("Bob", special_rooms()[0]);
    let c = h.player("Cid", special_rooms()[0]);
    h.during(a, |w| {
        add_experience(w, a, 100, false);
        add_experience(w, b, 300, false);
        add_experience(w, c, 200, false);
    });
    assert_eq!(order(&h.engine.world.topten), ["BOB", "CID", "ANN"]);
    h.during(a, |w| {
        add_experience(w, a, 250, false);
    });
    assert_eq!(order(&h.engine.world.topten), ["ANN", "BOB", "CID"]);
    // Cid draws level with Bob: he stays below him.
    h.during(c, |w| {
        add_experience(w, c, 100, false);
    });
    assert_eq!(order(&h.engine.world.topten), ["ANN", "BOB", "CID"]);
    // A billion beats any remainder.
    h.plr_mut(c).expbil = 1;
    h.engine.world.update_top_list(c);
    assert_eq!(order(&h.engine.world.topten), ["CID", "ANN", "BOB"]);
    assert_eq!(h.engine.world.topten[0].exphi, 1);
}

// A reroll, and the last-life deletion: the
// deleted character leaves the list and the rest move up.
#[test]
fn removal_on_deletion() {
    for why in [Retire::Suicide, Retire::LastLife] {
        let mut h = Harness::new();
        let a = h.player("Ann", special_rooms()[0]);
        let b = h.player("Bob", special_rooms()[0]);
        h.during(a, |w| {
            add_experience(w, a, 100, false);
            add_experience(w, b, 300, false);
        });
        assert_eq!(order(&h.engine.world.topten), ["BOB", "ANN"]);
        profile::retire(&mut h.engine.world, b, 1_760_000_000, why);
        assert_eq!(order(&h.engine.world.topten), ["ANN"], "{why:?}");
    }
}

// Building the top list: from every stored character, PS_NOTOPTEN left out,
// immortals marked EDITED, highest first, at most 100.
#[test]
fn rebuild_from_the_stored_characters() {
    let mk = |name: &str, rem: u32| {
        let mut c = silvermere_engine::clear_player();
        c.userid = name.to_ascii_uppercase();
        c.firstname = name.into();
        c.exprem = rem;
        c
    };
    let mut chars: Vec<Character> = (0..120).map(|i| mk(&format!("P{i:03}"), i)).collect();
    chars[119].settings |= PlayerSettings::NOTOPTEN;
    chars[118].immortal = 1;
    let list = TopTen::rebuild(&chars);
    assert_eq!(list.len(), 100);
    assert_eq!(list[0].userid, "P118");
    assert_eq!(list[0].edited, "EDITED");
    assert_eq!(list[1].userid, "P117");
    assert_eq!(list[1].edited, "");
    assert!(list.iter().all(|e| e.userid != "P119"));
    // Ties keep the order the characters came in.
    let tied = TopTen::rebuild(&[mk("Xa", 5), mk("Xb", 5)]);
    assert_eq!(order(&tied), ["XA", "XB"]);
}

// A newcomer joins a full list only with more than its last entry,
// which drops off; a character taken off TOPTEN leaves it.
#[test]
fn update_on_a_full_list_and_notopten() {
    let chars: Vec<Character> = (0..100)
        .map(|i| {
            let mut c = silvermere_engine::clear_player();
            c.userid = format!("P{i:03}");
            c.exprem = 10 + i;
            c
        })
        .collect();
    let mut list = TopTen::rebuild(&chars);
    let mut low = silvermere_engine::clear_player();
    low.userid = "LOW".into();
    low.exprem = 10;
    list.update(&low);
    assert!(list.iter().all(|e| e.userid != "LOW"));
    let mut high = low.clone();
    high.userid = "HIGH".into();
    high.exprem = 55;
    list.update(&high);
    assert_eq!(list.len(), 100);
    assert!(list.iter().all(|e| e.userid != "P000"));
    let at = list.iter().position(|e| e.userid == "HIGH").unwrap();
    // Below P045 (also 55), above P044 (54).
    assert_eq!(list[at - 1].userid, "P045");
    assert_eq!(list[at + 1].userid, "P044");
    high.settings |= PlayerSettings::NOTOPTEN;
    list.update(&high);
    assert!(list.iter().all(|e| e.userid != "HIGH"));
}
