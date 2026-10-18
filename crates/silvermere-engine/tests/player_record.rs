//! T-023: the player record. Create, save, load, the secondary stats, the save points and
//! the profiles, each against the original's behaviour.

mod common;

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use silvermere_engine::player::profile::{self, Retire};
use silvermere_engine::player::save::strip_level_gated;
use silvermere_engine::player::secondary::StealthSource;
use silvermere_engine::player::{
    CalcMode, LoadError, SavePoint, class_title, create_player, load_player, recalculate,
    roll_stats, save_at, save_player, secondary,
};
use silvermere_engine::player_session::{AccountRef, Caps};
use silvermere_engine::world::blank;
use silvermere_engine::{
    Engine, EngineInput, EngineOutput, PersistRequest, PlayerSession, SeededRng, SessionId, World,
};
use silvermere_rules::abilities::{AbilityLookup, CharacterAbilities};
use silvermere_rules::progression::new_calc_exp_needed;
use silvermere_text::Msg;
use silvermere_types::enums::{Ability, InGame, Magery, MessageLevel, SessionState, StatlineMode};
use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{AccountId, ClassNo, MapId, RaceNo, RoomId, SpellNo};
use silvermere_types::records::{BigExp, Character, Class, Race, TextBlock};

use common::TestClock;

const TODAY: i64 = 1_700_006_400; // a UTC midnight, as the database stores days

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

/// WCCRACE 11, Kang (the stock record).
fn kang() -> Race {
    let mut r = blank::clear_race(RaceNo(11));
    r.name = "Kang".into();
    (r.minint, r.minwil, r.minstr, r.minhea, r.minagi, r.mincha) = (30, 45, 55, 50, 30, 30);
    (r.maxint, r.maxwil, r.maxstr, r.maxhea, r.maxagi, r.maxcha) = (90, 110, 120, 110, 90, 85);
    r.basecp = 100;
    r.hpbonus = 0;
    r.exp = 35;
    r.abilities[0] = (Ability::from_code(21), 100);
    r.abilities[1] = (Ability::from_code(7), 10);
    r.abilities[2] = (Ability::from_code(2), 5);
    r
}

/// WCCCLASS 2, Witchunter (the stock record).
fn witchunter() -> Class {
    let mut c = blank::clear_class(ClassNo(2));
    c.name = "Witchunter".into();
    c.basehp = 6;
    c.hplevel = 4;
    c.exp = 30;
    c.combat = 7;
    c.magery = Magery::None;
    c.magelvl = 0;
    c.abilities[0] = (Ability::from_code(31), 0);
    c.abilities[1] = (Ability::from_code(51), 0);
    c.abilities[2] = (Ability::from_code(142), 10);
    c
}

/// A caster class as in the stock data (mage, priest and druid: magery level 3; basehp 3, 3 and 4).
fn caster(number: u16, magery: Magery, basehp: i16) -> Class {
    let mut c = blank::clear_class(ClassNo(number));
    c.basehp = basehp;
    c.hplevel = 3;
    c.magery = magery;
    c.magelvl = 3;
    c
}

fn plain_race(hpbonus: i16) -> Race {
    let mut r = blank::clear_race(RaceNo(1));
    r.hpbonus = hpbonus;
    r
}

fn world() -> World {
    let mut w = common::tiny_world(16);
    w.races.insert(RaceNo(11), kang());
    w.classes.insert(ClassNo(2), witchunter());
    w
}

fn account(n: i64) -> AccountRef {
    AccountRef {
        id: AccountId(n),
        username: format!("USER{n}"),
        sysop: false,
        gameop: false,
        sex: b'M',
    }
}

/// A session in the game for account `n`, with nothing loaded.
fn session(w: &mut World, n: i64) -> SessionId {
    let id = w.ids.alloc().expect("a free id");
    let mut s = PlayerSession::new(id, Caps::default(), Instant::now());
    s.account = Some(account(n));
    s.state = SessionState::Playing;
    s.ingame = InGame::Realm;
    w.sessions.insert(id, s);
    id
}

/// The creation menus' choices (T-022): race 11, class 2 and, unless blank, the name.
fn named(mut c: Character, name: &str) -> Character {
    c.race = RaceNo(11);
    c.class = ClassNo(2);
    if !name.is_empty() {
        c.firstname = name.into();
    }
    c
}

/// A Kasimir the Kang Witchunter as character creation leaves him: created, rolled, the
/// sheet's stats spent, the name validated.
fn kasimir(w: &World, n: i64) -> Character {
    let mut c = named(
        create_player(w, AccountId(n), &format!("USER{n}"), TODAY),
        "Kasimir",
    );
    roll_stats(w, &mut c, b'M');
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (79, 30, 45, 42, 74, 30);
    (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (79, 30, 45, 42, 74, 30);
    c.cp = 2;
    c.settings |= PlayerSettings::VALIDATED;
    recalculate(w, &mut c, CalcMode::Refill);
    c
}

fn put(w: &mut World, id: SessionId, c: Character) {
    let s = w.sessions.get_mut(&id).unwrap();
    // as load_player starts the session from the record's `dragging`
    s.party = silvermere_engine::party::Party::from_record(c.dragging);
    s.character = Some(c);
    s.created = true;
}

fn engine_over(
    w: World,
) -> (
    Engine<TestClock, SeededRng>,
    TestClock,
    Receiver<EngineOutput>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let clock = TestClock::new();
    (
        Engine::new(w, clock.clone(), SeededRng::new(7), tx),
        clock,
        rx,
    )
}

fn persists(rx: &Receiver<EngineOutput>) -> Vec<PersistRequest> {
    common::drain(rx)
        .into_iter()
        .filter_map(|o| match o {
            EngineOutput::Persist(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn saved_names(p: &[PersistRequest]) -> Vec<String> {
    p.iter()
        .filter_map(|r| match r {
            PersistRequest::Character { character, .. } => Some(character.firstname.clone()),
            _ => None,
        })
        .collect()
}

fn notices(w: &World, id: SessionId) -> Vec<Msg> {
    w.sessions[&id]
        .output
        .items()
        .iter()
        .filter_map(|i| match i {
            silvermere_engine::output::Item::Msg { msg, .. } => Some(msg.clone()),
            _ => None,
        })
        .collect()
}

/// Ability totals by hand, for the pure formulas.
#[derive(Default)]
struct Stub(HashMap<Ability, i32>);

impl Stub {
    fn with(pairs: &[(Ability, i32)]) -> Self {
        Self(pairs.iter().copied().collect())
    }
}

impl AbilityLookup for Stub {
    fn value(&self, a: Ability) -> i32 {
        self.0.get(&a).copied().unwrap_or(0)
    }
    fn has(&self, a: Ability) -> bool {
        self.0.contains_key(&a)
    }
    fn pair(&self, a: Ability, b: Ability) -> (i32, i32) {
        (self.value(a), self.value(b))
    }
    fn best(&self, a: Ability) -> Option<i32> {
        self.0.get(&a).copied()
    }
}

/// A character with the given level and stats (base = current).
fn sheet(level: u16, [s, i, w, a, h, c]: [u16; 6], randomhp: u8) -> Character {
    let mut ch = silvermere_engine::clear_player();
    ch.level = level;
    (ch.str, ch.intel, ch.wil, ch.agi, ch.hea, ch.cha) = (s, i, w, a, h, c);
    (
        ch.basstr, ch.basint, ch.baswil, ch.basagi, ch.bashea, ch.bascha,
    ) = (s, i, w, a, h, c);
    ch.randomhp = randomhp;
    ch
}

// ---------------------------------------------------------------------------------------
// Secondary stats
// ---------------------------------------------------------------------------------------

// the secondary stats of the stock Kang Witchunter, as measured on the original
#[test]
fn kang_witchunter_sheet() {
    let w = world();
    let c = kasimir(&w, 1);
    let spells = |n| w.spells.get(&n);
    let items = |n| w.items.get(&n);
    let lookup = CharacterAbilities {
        character: &c,
        race: w.races.get(&c.race),
        class: w.classes.get(&c.class),
        spells: &spells,
        items: &items,
    };
    let s = secondary(&c, &witchunter(), &kang(), &lookup);
    assert_eq!(s.maxwgt, 3792); // 79*48
    assert_eq!(s.maxmr, 41); // (30+135)/4
    assert_eq!(s.mabase, 1); // -2 clamped
    assert_eq!(s.maxmana, 0); // MAGERY_NONE
    // The record holds 1*2 - 150 + 0; the sheet does not show it for a
    // class without magery, which is why it reads as 0 there.
    assert_eq!(s.spellcast, -148);
    // 74/2 + 1*6 + 0 + (74-50)*1/16 + randomhp (hplevel 4 at level 1).
    assert_eq!(c.randomhp, 4);
    assert_eq!(s.maxhp, 37 + 6 + 1 + 4);
    assert_eq!(s.perception, 33); // (150+90+30)/8
    assert_eq!(s.martial, 1 + 3 + 8);
    assert_eq!(s.stealth_source, StealthSource::None);
    // The wrapper wrote the same into the record, with full hit points (mode 0).
    assert_eq!(
        (c.maxwgt, c.maxmr, c.mr, c.mabase, c.maxmana),
        (3792, 41, 41, 1, 0)
    );
    assert_eq!((c.spellcast, c.maxhp, c.hp), (-148, 48, 48));
}

// secondary stats: mana, spellcasting, MR, HP by magery
#[test]
fn caster_sheets() {
    struct Row {
        name: &'static str,
        class: Class,
        race: Race,
        c: Character,
        want: [i32; 8], // maxwgt, perception, maxmana, spellcast, maxmr, maxhp, mabase, martial
    }
    let mut druid = sheet(30, [110, 60, 70, 20, 60, 20], 50);
    druid.hpbonus = 5;
    druid.manabonus = 4;
    let rows = [
        // castbase (270+50)/6=53; maxhp 22+60+0+(-5*20)/16=-6+30; mabase 2+4+(-8)/20=0+(-12)/30=0.
        Row {
            name: "Mage",
            class: caster(5, Magery::Mage, 3),
            race: plain_race(0),
            c: sheet(20, [40, 90, 50, 42, 45, 38], 30),
            want: [1920, 73, 126, 108, 60, 106, 6, 21],
        },
        // castbase (240+40)/6=46; maxhp 24+21+(-2*7)/16=0+12; mabase -1+(-17)/20=0 -> 1.
        Row {
            name: "Priest",
            class: caster(12, Magery::Priest, 3),
            race: plain_race(0),
            c: sheet(7, [50, 40, 80, 33, 48, 42], 12),
            want: [2400, 50, 48, 75, 70, 57, 1, 12],
        },
        // maxwgt 5280+360; mana (90+3)*2+4; castbase 130/3=43; maxhp 30+120+30+18+50+5;
        // mabase 3+1+(-30)/20=-1+(-30)/30=-1.
        Row {
            name: "Druid",
            class: caster(13, Magery::Druid, 4),
            race: plain_race(1),
            c: druid,
            want: [5640, 57, 190, 118, 67, 253, 2, 14],
        },
    ];
    for r in rows {
        let s = secondary(&r.c, &r.class, &r.race, &Stub::default());
        let got = [
            s.maxwgt,
            s.perception,
            s.maxmana,
            s.spellcast,
            s.maxmr,
            s.maxhp,
            s.mabase,
            s.martial,
        ];
        assert_eq!(got, r.want, "{}", r.name);
    }
}

// secondary stats: Bard and Kai
#[test]
fn bard_and_kai() {
    let c = sheet(10, [50, 40, 60, 50, 50, 90], 0);
    let s = secondary(
        &c,
        &caster(9, Magery::Bard, 4),
        &plain_race(0),
        &Stub::default(),
    );
    // (3*10+3)*2 = 66; castbase (270+60)/6 = 55; 20+55+15.
    assert_eq!((s.maxmana, s.spellcast), (66, 90));
    let mut kai = caster(15, Magery::Kai, 4);
    kai.magelvl = 1;
    let s = secondary(&c, &kai, &plain_race(0), &Stub::default());
    // maxmana = level-1, castbase 500: 20+500+5.
    assert_eq!((s.maxmana, s.spellcast), (9, 525));
}

// secondary stats: maxmana floored as a signed short
#[test]
fn negative_mana_floors_at_zero() {
    let mut c = sheet(1, [50; 6], 0);
    c.manabonus = -100;
    let s = secondary(
        &c,
        &caster(5, Magery::Mage, 3),
        &plain_race(0),
        &Stub::default(),
    );
    assert_eq!(s.maxmana, 0);
}

// secondary stats: the thief skills
#[test]
fn skills_from_abilities() {
    let c = sheet(20, [50, 50, 40, 80, 50, 40], 0);
    let class = caster(8, Magery::None, 4);
    let all = Stub::with(&[
        (Ability::ClassStealth, 0),
        (Ability::Stealth, 5),
        (Ability::Thievery, 0),
        (Ability::FindTraps, 0),
        (Ability::Picklocks, 0),
        (Ability::Tracking, 0),
    ]);
    let s = secondary(&c, &class, &plain_race(0), &all);
    // stealth: 80/4 + (5*2/2+30) + 40/6 + 50/8 + 20 = 87, class only, + Stealth 5.
    assert_eq!((s.stealth, s.stealth_source), (92, StealthSource::Class));
    // skill level past 15: (20-15)/2+15 = 17.
    assert_eq!(s.thievery, (80 + 50 + 40 + 17 * 24) / 6); // 96
    assert_eq!((s.traps, s.disarm), (98, 98)); // (50+80+80+476)/7
    assert_eq!(s.picklocks, 85); // (80+50+170)*2/7
    assert_eq!(s.tracking, 107); // (100+40+40+680)/8

    let race_only = Stub::with(&[(Ability::RaceStealth, 0)]);
    let s = secondary(&c, &class, &plain_race(0), &race_only);
    assert_eq!(
        (s.stealth, s.stealth_source),
        (87 - 15, StealthSource::Race)
    );
    let both = Stub::with(&[(Ability::RaceStealth, 0), (Ability::ClassStealth, 0)]);
    let s = secondary(&c, &class, &plain_race(0), &both);
    assert_eq!(
        (s.stealth, s.stealth_source),
        (87 + 10, StealthSource::Both)
    );
    let hopeless = Stub::with(&[(Ability::RaceStealth, 0), (Ability::Stealth, -500)]);
    assert_eq!(secondary(&c, &class, &plain_race(0), &hopeless).stealth, 0);
    // Without the abilities, nothing.
    let none = secondary(&c, &class, &plain_race(0), &Stub::default());
    assert_eq!(
        [
            none.stealth,
            none.thievery,
            none.traps,
            none.picklocks,
            none.tracking
        ],
        [0; 5]
    );
    // Jumpkick doubles martial arts plus level.
    let plain = none.martial;
    let jk = secondary(
        &c,
        &class,
        &plain_race(0),
        &Stub::with(&[(Ability::Jumpkick, 0)]),
    );
    assert_eq!(jk.martial, (plain + 20) * 2);
}

// secondary stats: weight limit and HP over levels
#[test]
fn maxwgt_positive_and_maxhp_grows_with_level() {
    let class = caster(1, Magery::None, 6);
    for str_ in [1u16, 5, 50, 100, 101, 150, 255] {
        for bashea in [5u16, 30, 50, 74, 110, 150] {
            let mut prev = i32::MIN;
            for level in 1..=255u16 {
                let c = sheet(level, [str_, 50, 50, 50, bashea, 50], 0);
                let s = secondary(&c, &class, &plain_race(0), &Stub::default());
                assert!(s.maxwgt >= 1, "str {str_}");
                assert!(s.maxhp > prev, "str {str_} hea {bashea} level {level}");
                prev = s.maxhp;
            }
        }
    }
}

// recalculate: modes 0, 1, 2
#[test]
fn modes() {
    let w = world();
    let mut c = kasimir(&w, 1);
    c.hp = 5;
    c.mana = 0;
    recalculate(&w, &mut c, CalcMode::KeepCurrent);
    assert_eq!((c.hp, c.maxhp), (5, 48));
    recalculate(&w, &mut c, CalcMode::Refill);
    assert_eq!(c.hp, 48);
    c.maxhp = 99;
    c.mana = 7;
    recalculate(&w, &mut c, CalcMode::KeepMaxHp);
    assert_eq!((c.maxhp, c.maxmana, c.hp), (99, 7, 48));
}

// the dynamic-stat update: the status bits abilities set are rebuilt
#[test]
fn status_words_merge() {
    let w = world();
    let mut c = kasimir(&w, 1);
    c.status = PlayerStatus::FROMABILS | PlayerStatus::HUNGUP;
    c.status2 = PlayerStatus2::FROMABILS | PlayerStatus2::STATRESET;
    recalculate(&w, &mut c, CalcMode::KeepCurrent);
    assert_eq!(c.status, PlayerStatus::HUNGUP);
    assert_eq!(c.status2, PlayerStatus2::STATRESET);
}

// ---------------------------------------------------------------------------------------
// Level-gated abilities
// ---------------------------------------------------------------------------------------

// load_player; save_player
#[test]
fn level_gated_abilities_by_class_and_level() {
    let table: [(Ability, &[(u16, u16)]); 3] = [
        (
            Ability::Smash,
            &[(1, 22), (2, 20), (3, 25), (4, 27), (11, 27), (14, 22)],
        ),
        (
            Ability::PerfectStealth,
            &[
                (6, 25),
                (7, 20),
                (8, 20),
                (9, 25),
                (10, 25),
                (14, 20),
                (15, 27),
            ],
        ),
        (
            Ability::Meditate,
            &[
                (3, 27),
                (4, 23),
                (5, 20),
                (6, 23),
                (9, 27),
                (10, 23),
                (11, 23),
                (12, 20),
                (13, 20),
                (14, 27),
            ],
        ),
    ];
    let kept = |ability: Ability, class: u16, level: u16| {
        let mut c = silvermere_engine::clear_player();
        c.class = ClassNo(class);
        c.level = level;
        c.abils[3] = (ability, 7);
        strip_level_gated(&mut c);
        c.abils[3].0 == ability
    };
    for (ability, rows) in table {
        for class in 1..=16u16 {
            let from = rows.iter().find(|r| r.0 == class).map(|r| r.1);
            for level in [1u16, 19, 20, 21, 22, 23, 25, 27, 50] {
                let want = from.is_some_and(|f| level >= f);
                assert_eq!(
                    kept(ability, class, level),
                    want,
                    "{ability:?} class {class} level {level}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// load_player
// ---------------------------------------------------------------------------------------

// load_player: create -> save -> load gives the same record
#[test]
fn create_save_load_round_trip() {
    let mut w = world();
    let id = session(&mut w, 1);
    let made = kasimir(&w, 1);
    put(&mut w, id, made.clone());
    let (mut engine, _clock, rx) = engine_over(w);
    assert!(save_player(&mut engine.world, id));
    engine.flush();
    let saved = match persists(&rx).as_slice() {
        [PersistRequest::Character { account, character }] => {
            assert_eq!(*account, AccountId(1));
            (**character).clone()
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(saved, made);

    // A later session (the server restarted) loads the row.
    let mut w = world();
    let id = session(&mut w, 1);
    load_player(&mut w, id, saved.clone(), TODAY).unwrap();
    let loaded = w.sessions[&id].character.clone().unwrap();
    assert!(w.sessions[&id].created);
    assert_eq!(loaded, saved);
    assert!(notices(&w, id).is_empty());
}

// load_player: only version 22 is loaded (the conversions are the importer's)
#[test]
fn old_versions_are_refused() {
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.version = 21;
    assert_eq!(
        load_player(&mut w, id, c, TODAY),
        Err(LoadError::Version(21))
    );
}

// load_player: stats out of range are reset in the reincarnation room
#[test]
fn out_of_range_stats_reset_and_move_to_reincroom() {
    for (evil, room) in [(0i16, 2189u32), (39, 2189), (40, 142)] {
        let mut w = world();
        let id = session(&mut w, 1);
        let mut c = kasimir(&w, 1);
        c.level = 12;
        c.exp = 0;
        c.expbil = 1_000;
        c.str = 171; // Kang maximum 120, +50 allowed
        c.evil = evil;
        c.map = MapId(3);
        c.room = RoomId(77);
        c.spells[0] = SpellNo(5);
        load_player(&mut w, id, c, TODAY).unwrap();
        let c = w.sessions[&id].character.clone().unwrap();
        assert_eq!((c.map, c.room), (MapId(1), RoomId(room)), "evil {evil}");
        assert!(c.status2.contains(PlayerStatus2::STATRESET));
        assert_eq!(c.spells[0], SpellNo(0));
        // roll_stats: the race minimums ...
        assert_eq!(
            (c.str, c.intel, c.wil, c.hea, c.agi, c.cha),
            (55, 30, 45, 50, 30, 30)
        );
        // ... and the points of levels 2-12 back: 100 + 9*10 + 2*15.
        assert_eq!((c.cp, c.totalcp), (220, 220));
        assert_eq!(notices(&w, id), vec![Msg::EnterStatsReset]);
    }
    // In range at the edges: nothing happens.
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    (c.str, c.intel) = (170, 25); // 120+50, max(30-30,5)=5 is the floor
    load_player(&mut w, id, c, TODAY).unwrap();
    assert!(notices(&w, id).is_empty());
}

// load_player: the entry notices, in order
#[test]
fn entry_notices_in_order() {
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.status = PlayerStatus::HUNGUP;
    c.flags |= PlayerFlags::PROMOTED | PlayerFlags::HOUSECLOSED;
    c.gang = "Vanished".into();
    c.wil = 10; // below Kang's 45-30
    c.lives = 1;
    load_player(&mut w, id, c, TODAY).unwrap();
    assert_eq!(
        notices(&w, id),
        vec![
            Msg::EnterHangupNote,
            Msg::EnterHangupPunished,
            Msg::EnterPromoted,
            Msg::EnterHouseClosed,
            Msg::EnterGangRankStripped,
            Msg::EnterGangGone {
                gang: "Vanished".into()
            },
            Msg::EnterStatsReset,
            Msg::EnterLifeEbbing,
        ]
    );
    let c = w.sessions[&id].character.clone().unwrap();
    assert!(c.gang.is_empty());
    assert!(
        !c.flags
            .intersects(PlayerFlags::PROMOTED | PlayerFlags::LIEUTENANT | PlayerFlags::HOUSECLOSED)
    );
    assert!(c.status.is_empty());
}

// load_player: the texts
#[test]
fn notice_texts() {
    assert_eq!(
        Msg::EnterHangupNote.text(),
        "\x1b[1;37m\rLast time you were on, you disconnected while playing.\r"
    );
    assert_eq!(
        Msg::EnterHangupPunished.text(),
        "The gods have punished you appropriately.\r"
    );
    assert_eq!(
        Msg::EnterStatsReset.text(),
        "\x1b[1;37m\rYour stats have been reset due to an earlier bug.\r\
         Please TRAIN STATS as soon as possible to re-apply your CPs\r\
         (Unfortunately, if any spells were cast on you they will be terminated.\r"
    );
}

// load_player: the runtime repairs
#[test]
fn runtime_repairs() {
    let mut w = world();
    w.races.insert(RaceNo(10), kang());
    w.classes.insert(ClassNo(14), witchunter());
    let mut titles = TextBlock {
        part: 0,
        number: Default::default(),
        link: Default::default(),
        text: String::new(),
    };
    titles.number.0 = 900;
    titles.text = "Novice|Novicia\r\n\r\nHunter\r\n".into();
    w.text_blocks.insert((0, titles.number), titles.clone());
    w.classes.get_mut(&ClassNo(14)).unwrap().titles = titles.number;

    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.race = RaceNo(99);
    c.class = ClassNo(99);
    c.pagemax = 0;
    c.palette = 6;
    c.abils[0] = (Ability::SeeHidden, 1);
    c.spellbook[0] = SpellNo(0);
    c.spellbook[1] = SpellNo(0);
    c.spellbook[2] = SpellNo(4);
    c.spellbook[3] = SpellNo(0);
    c.spellbook[4] = SpellNo(9);
    c.settings -= PlayerSettings::VALIDATED;
    c.sex = b'F';
    load_player(&mut w, id, c, TODAY).unwrap();
    let c = w.sessions[&id].character.clone().unwrap();
    assert_eq!((c.race, c.class), (RaceNo(10), ClassNo(14)));
    assert_eq!(c.pagemax, 1); // the default chat rate
    assert_eq!(c.palette, 0);
    assert_eq!(c.abils[0].0, Ability::None);
    assert_eq!(&c.spellbook[..3], &[SpellNo(4), SpellNo(9), SpellNo(0)]);
    assert!(c.settings.contains(PlayerSettings::VALIDATED));
    assert_eq!(c.lastdate, TODAY);
    assert_eq!(c.title, "Novicia");
    assert_eq!(class_title(&w, ClassNo(14), 2, b'M'), "Hunter"); // blank lines skipped
    assert_eq!(class_title(&w, ClassNo(14), 3, b'M'), "Adventurer");
    assert_eq!(class_title(&w, ClassNo(2), 9, b'F'), "Lady");
    assert_eq!(class_title(&w, ClassNo(2), 12, b'M'), "Master");
}

// reduce_level_to_exp, via load_player
#[test]
fn level_reduced_to_what_the_exp_pays_for() {
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.level = 5;
    let needed4 = new_calc_exp_needed(3, 65);
    c.expbil = needed4.expbil;
    c.exprem = needed4.exprem; // enough for level 4, not 5
    load_player(&mut w, id, c, TODAY).unwrap();
    let c = w.sessions[&id].character.clone().unwrap();
    assert_eq!(c.level, 4);
    assert!(c.status2.contains(PlayerStatus2::STATRESET)); // cp=-1 retrains
}

// ---------------------------------------------------------------------------------------
// save_player and the save points
// ---------------------------------------------------------------------------------------

// save_player: never an unfinished or unvalidated character
#[test]
fn save_guards() {
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    w.sessions.get_mut(&id).unwrap().character = Some(c.clone());
    assert!(!save_player(&mut w, id), "not created");
    w.sessions.get_mut(&id).unwrap().created = true;
    c.settings -= PlayerSettings::VALIDATED;
    w.sessions.get_mut(&id).unwrap().character = Some(c);
    assert!(!save_player(&mut w, id), "not validated");
    w.sessions.get_mut(&id).unwrap().character = None;
    assert!(!save_player(&mut w, id), "empty");
}

// save_player at each of our 30 save points and the two race/class change writes;
// stripping
#[test]
fn each_save_point_saves_once() {
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.abils[5] = (Ability::Smash, 3); // Witchunter keeps it only from 20
    put(&mut w, id, c.clone());
    let (mut engine, _clock, rx) = engine_over(w);
    let mut expected = c;
    expected.abils[5].0 = Ability::None;
    for point in SavePoint::ALL {
        assert!(save_at(&mut engine.world, id, point), "{point:?}");
        engine.flush();
        let p = persists(&rx);
        assert_eq!(
            p,
            vec![PersistRequest::Character {
                account: AccountId(1),
                character: Box::new(expected.clone()),
            }],
            "{point:?}"
        );
    }
}

// logoff and hang-up saves
#[test]
fn logoff_and_hangup_save() {
    let mut w = world();
    let a = session(&mut w, 1);
    let b = session(&mut w, 2);
    let mut ca = kasimir(&w, 1);
    ca.firstname = "Ann".into();
    let mut cb = kasimir(&w, 2);
    cb.firstname = "Bob".into();
    put(&mut w, a, ca);
    put(&mut w, b, cb);
    let (mut engine, _clock, rx) = engine_over(w);
    engine.world.logoff(a);
    engine.flush();
    assert_eq!(saved_names(&persists(&rx)), ["Ann"]);
    // A hang-up passes two save points: the hang-up's, after the punishment
    // (T-021's `leave::hangup`), and then the logoff's.
    engine.handle(EngineInput::Disconnect { id: b });
    assert_eq!(saved_names(&persists(&rx)), ["Bob", "Bob"]);
}

// save_characters: five per sweep, round-robin, restart at 0
#[test]
fn sweep_saves_five_round_robin() {
    let mut w = common::tiny_world(12);
    w.races.insert(RaceNo(11), kang());
    w.classes.insert(ClassNo(2), witchunter());
    for n in 0..12 {
        let id = session(&mut w, n);
        let mut c = kasimir(&w, n);
        c.firstname = format!("P{n}");
        put(&mut w, id, c);
    }
    let (mut engine, clock, rx) = engine_over(w);
    let names = |r: &[&str]| r.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // The save routine runs every 60 s (`sec_oth` 60 in the test config).
    clock.advance_ticks(59, &mut engine);
    assert!(saved_names(&persists(&rx)).is_empty());
    clock.advance_ticks(1, &mut engine);
    assert_eq!(
        saved_names(&persists(&rx)),
        names(&["P0", "P1", "P2", "P3", "P4"])
    );
    clock.advance_ticks(60, &mut engine);
    assert_eq!(
        saved_names(&persists(&rx)),
        names(&["P5", "P6", "P7", "P8", "P9"])
    );
    clock.advance_ticks(60, &mut engine);
    assert_eq!(saved_names(&persists(&rx)), names(&["P10", "P11"]));
    clock.advance_ticks(60, &mut engine);
    assert_eq!(
        saved_names(&persists(&rx)),
        names(&["P0", "P1", "P2", "P3", "P4"])
    );
}

// save_characters: empty slots are skipped, not counted
#[test]
fn sweep_skips_sessions_not_in_game() {
    let mut w = common::tiny_world(12);
    w.races.insert(RaceNo(11), kang());
    w.classes.insert(ClassNo(2), witchunter());
    for n in 0..8 {
        let id = session(&mut w, n);
        if n % 2 == 0 {
            let mut c = kasimir(&w, n);
            c.firstname = format!("P{n}");
            put(&mut w, id, c);
        }
    }
    let (mut engine, _clock, rx) = engine_over(w);
    engine.world.save_cycle();
    engine.flush();
    assert_eq!(saved_names(&persists(&rx)), ["P0", "P2", "P4", "P6"]);
}

// ---------------------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------------------

fn retired(keep_setting: bool, keep_option: bool) -> (World, Vec<PersistRequest>) {
    let mut w = world();
    w.config.keep_exp = keep_option;
    w.config.evil_keep = 50;
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.level = 10;
    c.expbil = 3;
    c.exprem = 500_000_000;
    c.evil = 30;
    c.deathdate = TODAY - 86_400;
    c.statline = StatlineMode::Custom;
    c.verbose = 0;
    c.palette = 2;
    c.msglevel = MessageLevel::Low;
    c.lastname = "Ironhide".into();
    c.settings.set(PlayerSettings::KEEPEXP, keep_setting);
    c.settings |= PlayerSettings::NOTOPTEN;
    put(&mut w, id, c);
    w.sessions.get_mut(&id).unwrap().ust.statline = "HP=%h".into();
    let (mut engine, _clock, rx) = engine_over(w);
    profile::retire(&mut engine.world, id, TODAY, Retire::LastLife);
    assert!(engine.world.sessions[&id].character.is_none());
    engine.flush();
    let p = persists(&rx);
    (engine.world, p)
}

// check_kill_user; profile::save
#[test]
fn last_life_death_stores_the_profile() {
    let (w, p) = retired(true, true);
    let rec = w.profiles[&AccountId(1)].clone();
    assert_eq!(
        p,
        vec![
            PersistRequest::EvilPoints {
                account: AccountId(1),
                record: silvermere_types::records::EvilRecord {
                    userid: "USER1".into(),
                    evil: 15, // 30*50/100 on another day
                },
            },
            PersistRequest::Profile {
                account: AccountId(1),
                record: Box::new(rec.clone()),
            },
            PersistRequest::DeleteCharacter {
                account: AccountId(1)
            },
        ]
    );
    assert_eq!(rec.deathdate, TODAY);
    assert_eq!(
        (rec.statline, rec.verbose, rec.palette),
        (StatlineMode::Custom, 0, 2)
    );
    assert_eq!(rec.msglevel, MessageLevel::Low);
    assert_eq!(rec.statfmt, "HP=%h");
    assert_eq!(
        (rec.firstname.as_str(), rec.lastname.as_str()),
        ("Kasimir", "Ironhide")
    );
}

// profile::save: kept experience only with both switches on
#[test]
fn kept_exp_needs_both_switches() {
    // A tenth of 3,500,000,000 is 350,000,000, more than level 5 needs.
    let (w, _) = retired(true, true);
    let rec = &w.profiles[&AccountId(1)];
    assert_eq!((rec.expbil, rec.exprem, rec.exp), (0, 350_000_000, 0));
    for (setting, option) in [(true, false), (false, true), (false, false)] {
        let (w, _) = retired(setting, option);
        let rec = &w.profiles[&AccountId(1)];
        assert_eq!((rec.expbil, rec.exprem), (0, 0), "{setting} {option}");
    }
    // Little experience: the half-level requirement plus one wins.
    let w = world();
    let mut c = kasimir(&w, 1);
    c.level = 10;
    c.exprem = 100;
    let needed = new_calc_exp_needed(5, 65);
    assert_eq!(
        profile::kept_exp(&c, &witchunter(), &kang(), true),
        BigExp {
            expbil: needed.expbil,
            exprem: needed.exprem + 1
        }
    );
    c.level = 1;
    assert_eq!(
        profile::kept_exp(&c, &witchunter(), &kang(), true),
        BigExp {
            expbil: 0,
            exprem: 0
        }
    );
}

// profile::restore, via create_player
#[test]
fn next_character_inherits_the_profile() {
    let (mut w, _) = retired(true, true);
    let id = session(&mut w, 1);
    let c = named(create_player(&w, AccountId(1), "USER1", TODAY), "");
    assert_eq!(
        (c.statline, c.verbose, c.palette),
        (StatlineMode::Custom, 0, 2)
    );
    assert_eq!(c.msglevel, MessageLevel::Low);
    assert_eq!(
        (c.firstname.as_str(), c.lastname.as_str()),
        ("Kasimir", "Ironhide")
    );
    assert_eq!((c.expbil, c.exprem), (0, 350_000_000));
    assert_eq!(c.evil, 15); // the saved evil points
    // PS_VALIDATED and PS_NOTOPTEN are not carried.
    assert!(
        !c.settings
            .intersects(PlayerSettings::VALIDATED | PlayerSettings::NOTOPTEN)
    );
    assert!(c.settings.contains(PlayerSettings::KEEPEXP));
    let rec = w.profiles[&AccountId(1)].clone();
    let ust = &mut w.sessions.get_mut(&id).unwrap().ust;
    profile::restore_session(ust, &rec, c.deathdate, TODAY);
    assert_eq!(ust.statline, "HP=%h");
    // Another account inherits nothing.
    let other = named(create_player(&w, AccountId(2), "USER2", TODAY), "Zed");
    assert_eq!(
        (other.statline, other.verbose, other.evil),
        (StatlineMode::Full, 1, 0)
    );
}

// create_player: the defaults
#[test]
fn new_character_defaults() {
    let w = world();
    let c = create_player(&w, AccountId(9), "LONGUSERNAME", TODAY);
    assert_eq!(c.firstname, "LONGUSERNA");
    assert_eq!((c.level, c.str, c.basstr), (1, 5, 5));
    assert_eq!(
        (c.maxhp, c.hp, c.maxenergy, c.energy, c.maxwgt),
        (20, 20, 1000, 1000, 500)
    );
    assert_eq!((c.map, c.room), (MapId(1), RoomId(2140)));
    assert_eq!(
        (
            c.hunger,
            c.thirst,
            c.lives,
            c.autoengage,
            c.readied,
            c.movedir
        ),
        (1000, 1000, 9, 1, -1, -1)
    );
    assert!(c.itemuses.iter().all(|&u| u == -1) && c.keyuses.iter().all(|&u| u == -2));
    assert!(c.wornuses.iter().all(|&u| u == -1));
    assert_eq!(
        (c.statline, c.verbose, c.fasttalk, c.paging),
        (StatlineMode::Full, 1, 1, 1)
    );
    assert_eq!((c.pagemax, c.gossmax, c.auctmax), (3, 3, 3));
    assert_eq!(
        c.settings,
        PlayerSettings::EVILWARN | PlayerSettings::KEEPEXP
    );
    assert_eq!(c.flags, PlayerFlags::RACECHANGED | PlayerFlags::NEWEXP);
    assert_eq!(
        (c.title.as_str(), c.version, c.lastdate),
        ("Apprentice", 22, TODAY)
    );
    // Race and class come from the creation menus.
    assert_eq!((c.race, c.class), (RaceNo(0), ClassNo(0)));
}

// the main menu: `P`/`E` runs load_player on the held row, every time
#[test]
fn entering_the_realm_runs_load_player_each_time() {
    let mut w = world();
    let id = session(&mut w, 1);
    let mut c = kasimir(&w, 1);
    c.status = PlayerStatus::HUNGUP;
    c.pagemax = 0;
    put(&mut w, id, c);
    w.sessions.get_mut(&id).unwrap().state = SessionState::MainMenu;
    let (mut engine, clock, rx) = engine_over(w);
    clock.advance(std::time::Duration::from_secs(3 * 86_400 + 5_000));
    engine.handle(EngineInput::Line {
        id,
        text: "E".into(),
    });
    let text: String = common::drain(&rx)
        .into_iter()
        .filter_map(|o| match o {
            EngineOutput::Write { id: to, bytes } if to == id => {
                Some(String::from_utf8_lossy(&bytes).into_owned())
            }
            _ => None,
        })
        .collect();
    // The hang-up pair once, before the banner.
    assert_eq!(text.matches("disconnected while playing").count(), 1);
    assert!(text.find("punished").unwrap() < text.find("MajorMUD - ROL").unwrap());
    let s = &engine.world.sessions[&id];
    let c = s.character.clone().unwrap();
    assert_eq!(s.state, SessionState::Playing);
    assert!(c.status.is_empty());
    assert_eq!(c.pagemax, 1);
    assert_eq!(c.lastdate, 3 * 86_400); // the day, at UTC midnight
    // Back at the menu, the next entry loads the held row again.
    let s = engine.world.sessions.get_mut(&id).unwrap();
    s.state = SessionState::MainMenu;
    s.character.as_mut().unwrap().gossmax = 0;
    engine.handle(EngineInput::Line {
        id,
        text: "e".into(),
    });
    assert_eq!(
        engine.world.sessions[&id]
            .character
            .as_ref()
            .unwrap()
            .gossmax,
        1
    );
}
