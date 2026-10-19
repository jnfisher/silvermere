//! T-056: the daily cleanup with players on line, and the leaving callbacks (hang-up and
//! logoff).
//!
//! The harness plays `bin`'s store: it keeps the last saved record of every account and
//! answers `EngineOutput::CleanupScan` with them, as `store.rs` does from the database.

use crate::common;

use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;

use common::TestClock;
use silvermere_engine::cleanup::{self as job, CleanupReport};
use silvermere_engine::state::login::Login;
use silvermere_engine::world::blank;
use silvermere_engine::{
    AccountRef, AuditValue, Caps, ConnId, Engine, EngineInput, EngineOutput, GameEvent,
    PersistRequest, SeededRng, SessionId, World, WorldSnapshot, clear_player,
};
use silvermere_types::config::{GameConfig, PunHup};
use silvermere_types::enums::{
    Ability, Currency, GangHouse, MessageLevel, SessionState, StatlineMode,
};
use silvermere_types::flags::{GangFlags, PlayerFlags, PlayerSettings, PlayerStatus, RoomFlags};
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, MapId, RaceNo, RoomId, RoomKey, ShopNo};
use silvermere_types::records::{BankBook, Character, Gang, Item, RoomItem};

// The fixture world: map 1, rooms 1..=6 numbered without gaps so that
// `room_walk` (50 missing numbers end a map) reaches them all.
const START: u32 = 1;
const REINC: u32 = 2;
const EVIL_REINC: u32 = 3;
const JAIL: u32 = 4;
/// `RF_CLEANUP`.
const CLEAN: u32 = 5;
/// `RF_GANGHOUSE`, the Red Gang House.
const HOUSE: u32 = 6;

const DEED: ItemNo = ItemNo(140);
const RECHARGE: ItemNo = ItemNo(150);
const GANG_ITEM: ItemNo = ItemNo(160);

const LARS: AccountId = AccountId(100);
const EVAN: AccountId = AccountId(101);

fn key(room: u32) -> RoomKey {
    RoomKey::new(1, room)
}

fn config() -> GameConfig {
    let mut c = GameConfig {
        sec_oth: 60,
        start_room: START,
        reinc_room: REINC,
        evil_reinc_room: EVIL_REINC,
        jail_room: JAIL,
        ..GameConfig::default()
    };
    c.cleanup.at = "00:20".into();
    c
}

fn item(n: i32, name: &str, cost: u16, edit: impl FnOnce(&mut Item)) -> Item {
    let mut i = blank::clear_item(ItemNo(n));
    i.name = name.into();
    i.cost = cost;
    i.gettable = 1;
    i.weight = 1;
    edit(&mut i);
    i
}

fn slot(n: i32, uses: i16, count: i16) -> RoomItem {
    RoomItem {
        item: ItemNo(n),
        uses,
        count,
    }
}

fn gang(name: &str, members: i16, flags: GangFlags) -> Gang {
    Gang {
        key: name.to_ascii_uppercase(),
        name: name.into(),
        exp: 0,
        leader: "Lars".into(),
        created: 0,
        members,
        flags,
        overexp: 0,
        overflows: 0,
    }
}

/// The cleanup world of T-056: a normal room with 10 floor items and
/// coins, a `RF_CLEANUP` room, and the Red Gang House, whose deed Lars holds.
fn snapshot() -> WorldSnapshot {
    let mut s = WorldSnapshot::default();
    for (n, name) in [
        (START, "Square"),
        (REINC, "Temple"),
        (EVIL_REINC, "Pit"),
        (JAIL, "Jail"),
        (CLEAN, "Midden"),
        (HOUSE, "Red House"),
    ] {
        let mut r = blank::clear_room(key(n));
        r.name = name.into();
        s.rooms.push(r);
    }
    {
        let r = s.rooms.iter_mut().find(|r| r.key == key(START)).unwrap();
        r.huproom = RoomId(REINC);
        r.placed[0] = ItemNo(108);
        for (i, sl) in [
            slot(101, -1, 0),
            slot(102, -1, 0),
            slot(103, -1, 0),
            slot(104, -1, 0),
            slot(105, -1, 0),
            slot(106, -1, 0),
            slot(107, -1, 2),
            slot(108, -1, 0),
            slot(109, -1, 0),
            slot(110, 5, 0),
        ]
        .into_iter()
        .enumerate()
        {
            r.items[i] = sl;
        }
        r.hiditems[0] = slot(107, -1, 1);
        r.hiditems[1] = slot(111, -1, 0);
        r.coins.copper = 7;
        r.coins.gold = 3;
    }
    {
        let r = s.rooms.iter_mut().find(|r| r.key == key(CLEAN)).unwrap();
        r.flags |= RoomFlags::CLEANUP;
        r.items[0] = slot(120, -1, 0);
        r.hiditems[0] = slot(121, -1, 0);
    }
    {
        let r = s.rooms.iter_mut().find(|r| r.key == key(HOUSE)).unwrap();
        r.flags |= RoomFlags::GANGHOUSE;
        r.ganghouse = GangHouse::Red;
        r.items[0] = slot(130, -1, 0);
        r.hiditems[0] = slot(131, -1, 0);
        r.coins.gold = 5;
        r.hidcoins.gold = 2;
    }
    s.items = vec![
        item(101, "torch", 5, |_| {}),
        item(102, "rock", 0, |_| {}),
        item(103, "scrap", 10, |i| {
            i.abilities[0] = (Ability::DelCleanup, 0);
        }),
        item(104, "statue", 100, |i| {
            i.currency = Currency::Gold;
            i.abilities[0] = (Ability::KeepVisible, 0);
        }),
        item(105, "pebble", 1, |i| {
            i.abilities[0] = (Ability::KeepVisible, 0);
        }),
        item(106, "fountain", 10, |i| i.gettable = 0),
        item(107, "rope", 3, |_| {}),
        item(108, "sign", 3, |_| {}),
        item(109, "gem", 1, |i| i.currency = Currency::Platinum),
        item(110, "wand", 20, |_| {}),
        item(111, "junk", 0, |_| {}),
        item(120, "bone", 10, |_| {}),
        item(121, "rag", 10, |_| {}),
        item(130, "banner", 10, |_| {}),
        item(131, "chest", 10, |_| {}),
        item(140, "red deed", 10, |i| {
            i.abilities[0] = (Ability::GangDeed, 1);
            i.abilities[1] = (Ability::GangTax, 10);
        }),
        item(150, "lantern", 10, |i| {
            i.abilities[0] = (Ability::RechargeCleanup, 10);
        }),
        item(160, "red key", 10, |i| {
            i.abilities[0] = (Ability::GangItem, 1);
        }),
    ];
    s.classes.push(blank::clear_class(ClassNo(1)));
    s.races.push(blank::clear_race(RaceNo(1)));
    s.gangs = vec![
        gang("Wolves", 2, GangFlags::empty()),
        gang("Empty", 0, GangFlags::empty()),
        gang("Doomed", 3, GangFlags::DELETE),
    ];
    // Lars's book at the gang-house bank holds 500 copper; the tax is 10 silver = 1000.
    s.bank_books.push((
        LARS,
        BankBook {
            userid: "LARS".into(),
            bank: ShopNo(8),
            balance: 500,
            loan: 0,
        },
    ));
    s
}

/// A stored character `load_player` accepts (as `login_flow.rs`'s).
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
    c.version = 22;
    c.level = 1;
    (c.str, c.intel, c.wil, c.agi, c.hea, c.cha) = (50, 50, 50, 50, 50, 50);
    (c.basstr, c.basint, c.baswil, c.basagi, c.bashea, c.bascha) = (50, 50, 50, 50, 50, 50);
    c.hpbonus = 10;
    c.settings |= PlayerSettings::VALIDATED;
    c.flags |= PlayerFlags::RACECHANGED;
    c
}

/// Lars: Wolves, the Red deed and a Red house item, played 5 minutes.
fn lars() -> Character {
    let mut c = character("Lars", START);
    c.gang = "Wolves".into();
    c.items[0] = DEED;
    c.itemuses[0] = -1;
    c.items[1] = GANG_ITEM;
    c.itemuses[1] = -1;
    c.sesstime = 10;
    c
}

/// Evan: level 5 with 50 evil points after 120 minutes of play, and a lantern with 2
/// charges left that recharges at cleanup.
fn evan() -> Character {
    let mut c = character("Evan", REINC);
    c.level = 5;
    c.evil = 50;
    c.sesstime = 240;
    c.lifemins = 1000;
    c.items[0] = RECHARGE;
    c.itemuses[0] = 2;
    c
}

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
    next_conn: u64,
    /// The store: the last saved record of every account.
    stored: BTreeMap<AccountId, Character>,
}

impl Harness {
    fn new(config: GameConfig, edit: impl FnOnce(&mut World)) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let clock = TestClock::new();
        let mut config = config;
        config.cleanup.tz = "UTC".into();
        let mut world = World::load(config, 8, snapshot()).expect("world loads");
        edit(&mut world);
        let engine = Engine::new(world, clock.clone(), SeededRng::new(7), tx);
        Self {
            engine,
            clock,
            rx,
            log: Vec::new(),
            next_conn: 1,
            stored: BTreeMap::new(),
        }
    }

    fn w(&mut self) -> &mut World {
        &mut self.engine.world
    }

    fn pump(&mut self) {
        while let Ok(o) = self.rx.try_recv() {
            if let EngineOutput::Persist(PersistRequest::Character { account, character }) = &o {
                self.stored.insert(*account, (**character).clone());
            }
            self.log.push(o);
        }
    }

    fn handle(&mut self, input: EngineInput) {
        self.engine.handle(input);
        self.pump();
    }

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

    fn account(id: SessionId) -> AccountId {
        AccountId(i64::from(id.0) + 1)
    }

    fn at_menu(&mut self, character: Character) -> SessionId {
        let id = self.connect();
        let name = character.firstname.clone();
        self.handle(EngineInput::LoggedIn {
            id,
            login: Box::new(Login {
                account: AccountRef {
                    id: Self::account(id),
                    username: name,
                    sysop: false,
                    gameop: false,
                    sex: b'M',
                },
                character: Some(Box::new(character)),
                may_gossip: true,
                may_auction: true,
                sysop_keys: silvermere_engine::commands::sysop::Keys::empty(),
            }),
        });
        self.line(id, "M");
        assert_eq!(self.state(id), Some(SessionState::MainMenu));
        id
    }

    fn playing(&mut self, character: Character) -> SessionId {
        let id = self.at_menu(character);
        self.line(id, "E");
        assert_eq!(self.state(id), Some(SessionState::Playing));
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

    fn state(&self, id: SessionId) -> Option<SessionState> {
        self.engine.world.session(id).map(|s| s.state)
    }

    fn plr(&self, id: SessionId) -> &Character {
        self.engine.world.session(id).unwrap().player().unwrap()
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

    fn scans_since(&self, from: usize) -> usize {
        self.log[from..]
            .iter()
            .filter(|o| matches!(o, EngineOutput::CleanupScan))
            .count()
    }

    /// The store's answer to `CleanupScan`: every stored record (`store.rs`).
    fn try_answer_scan(&mut self) -> Option<CleanupReport> {
        let rows: Vec<(AccountId, Character)> =
            self.stored.iter().map(|(a, c)| (*a, c.clone())).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        self.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                let _ = tx.send(job::on_scan(w, rows));
            }),
        });
        rx.recv().expect("the answer was handled")
    }

    fn answer_scan(&mut self) -> CleanupReport {
        self.try_answer_scan().expect("a run was waiting")
    }

    /// `EngineInput::DailyCleanup` and the store's answer.
    fn run_cleanup(&mut self) -> CleanupReport {
        let mark = self.log.len();
        self.handle(EngineInput::DailyCleanup);
        assert_eq!(self.scans_since(mark), 1);
        self.answer_scan()
    }

    fn audits_since(&self, from: usize) -> Vec<(&'static str, Vec<(&'static str, AuditValue)>)> {
        self.log[from..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Event(GameEvent::Audit { kind, fields }) => {
                    Some((*kind, fields.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

mod cleanup {
    use super::*;

    fn offline_harness() -> Harness {
        let mut h = Harness::new(config(), |_| {});
        h.stored.insert(LARS, lars());
        h.stored.insert(EVAN, evan());
        h
    }

    // The four passes on the fixture world
    #[test]
    fn fixture_world() {
        let mut h = offline_harness();
        let mark = h.log.len();
        let report = h.run_cleanup();

        // pass 1: Lars holds house 1 with a tax of 10 silver; his 500 copper cannot pay
        let red = &report.houses[1];
        assert_eq!(red.leader, "LARS");
        assert_eq!(red.gang, "Wolves");
        assert_eq!(red.tax, 1000);
        assert!(!red.occupied);
        assert_eq!(red.flags & job::EVICTED, job::EVICTED);
        assert!(red.closed());
        assert_eq!(h.w().bank_books[&(LARS, ShopNo(8))].balance, 500);

        // pass 2, the normal room
        let rs = h.w().room_state[&key(START)].clone();
        let visible: Vec<i32> = rs
            .items
            .iter()
            .map(|s| s.item.0)
            .filter(|&n| n != 0)
            .collect();
        // statue (Remain Visible, 100 gold), fountain (not gettable) and the room's own
        // sign stay; the Remain Visible pebble goes with the crowd (10 items > 8)
        assert_eq!(visible, [104, 106, 108]);
        // the rope joins its hidden stack (only the extra copies: sic), torch, gem and
        // wand take free hidden slots in order; the free junk is deleted
        assert_eq!(rs.hiditems[0], slot(107, -1, 3));
        assert_eq!(rs.hiditems[1].item, ItemNo(0));
        assert_eq!(rs.hiditems[2], slot(101, -1, 0));
        assert_eq!(rs.hiditems[3], slot(109, -1, 0));
        assert_eq!(rs.hiditems[4], slot(110, 5, 0));
        assert_eq!(report.items_hidden, 4);
        // coins become hidden coins
        assert_eq!((rs.coins.copper, rs.coins.gold), (0, 0));
        assert_eq!((rs.hidcoins.copper, rs.hidcoins.gold), (7, 3));

        // the RF_CLEANUP room loses everything
        let rs = h.w().room_state[&key(CLEAN)].clone();
        assert!(rs.items.iter().all(|s| s.item == ItemNo(0)));
        assert!(rs.hiditems.iter().all(|s| s.item == ItemNo(0)));

        // the closed house is emptied, coins and hidden coins too
        let rs = h.w().room_state[&key(HOUSE)].clone();
        assert!(rs.items.iter().all(|s| s.item == ItemNo(0)));
        assert!(rs.hiditems.iter().all(|s| s.item == ItemNo(0)));
        assert_eq!((rs.coins.gold, rs.hidcoins.gold), (0, 0));
        assert_eq!((red.nitems, red.nhidden), (1, 1));

        // pass 3: 50 evil points at level 5 after 120 minutes: (120/60)*1+3 = 5 forgiven
        let e = &h.stored[&EVAN];
        assert_eq!(e.evil, 45);
        assert_eq!(e.lifemins, 1120);
        assert_eq!(e.sesstime, 0);
        // the lantern is back at its "Recharge at cleanup" value
        assert_eq!((e.items[0], e.itemuses[0]), (RECHARGE, 10));
        // Lars loses the Red house item and is told at his next entry
        let l = &h.stored[&LARS];
        assert_eq!(l.items[1], ItemNo(0));
        assert_eq!(l.items[0], DEED);
        assert!(l.flags.contains(PlayerFlags::HOUSECLOSED));
        assert!(l.flags.contains(PlayerFlags::HOUSEITEMS));
        assert_eq!(l.lifemins, 5);
        assert_eq!(report.users, 2);
        assert_eq!(report.minutes, 125);
        assert_eq!(report.evil_forgiven, 5);

        // pass 4: the gang with no members and the one marked for deletion go
        assert_eq!(report.gangs_deleted, 2);
        let mut gangs: Vec<&String> = h.engine.world.gangs.keys().collect();
        gangs.sort();
        assert_eq!(gangs, ["WOLVES"]);
        let deleted: Vec<&String> = h.log[mark..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Persist(PersistRequest::DeleteGang(k)) => Some(k),
                _ => None,
            })
            .collect();
        assert_eq!(deleted, ["DOOMED", "EMPTY"]);

        // the original's operator broadcast and log lines, as audit events in order
        let audits = h.audits_since(mark);
        let kinds: Vec<&str> = audits.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            kinds,
            [
                "cleanup.start",
                "ganghouse.evicted",
                "cleanup.items",
                "cleanup.users",
                "cleanup.complete"
            ]
        );
        assert_eq!(audits[2].1, [("hidden", AuditValue::Int(4))]);
        assert_eq!(
            audits[3].1,
            [
                ("users", AuditValue::Int(2)),
                ("examined", AuditValue::Int(2)),
                ("minutes", AuditValue::Int(125)),
                ("evil_forgiven", AuditValue::Int(5)),
            ]
        );
        // every changed room went to the persistence worker
        let rooms: Vec<RoomKey> = h.log[mark..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Persist(PersistRequest::RoomState(s)) => Some(s.key),
                _ => None,
            })
            .collect();
        for r in [START, CLEAN, HOUSE] {
            assert!(rooms.contains(&key(r)), "room {r} saved");
        }
    }

    // A holder who can pay keeps the house, the tax comes out of his
    // book, and the house keeps its contents
    #[test]
    fn a_paid_tax_keeps_the_house() {
        let mut h = offline_harness();
        h.w()
            .bank_books
            .get_mut(&(LARS, ShopNo(8)))
            .unwrap()
            .balance = 1500;
        let mark = h.log.len();
        let report = h.run_cleanup();
        assert!(!report.houses[1].closed());
        assert_eq!(report.houses[1].flags, job::TAX_PAID);
        assert_eq!(h.w().bank_books[&(LARS, ShopNo(8))].balance, 500);
        let rs = h.w().room_state[&key(HOUSE)].clone();
        assert_eq!(rs.items[0].item, ItemNo(130));
        assert_eq!(rs.hiditems[0].item, ItemNo(131));
        assert_eq!(rs.coins.gold, 5);
        let l = &h.stored[&LARS];
        assert_eq!(l.items[1], GANG_ITEM);
        assert!(!l.flags.contains(PlayerFlags::HOUSEITEMS));
        assert!(h.log[mark..].iter().any(|o| matches!(
            o,
            EngineOutput::Persist(PersistRequest::BankBook { account, book })
                if *account == LARS && book.balance == 500
        )));
        assert!(
            h.audits_since(mark)
                .iter()
                .any(|(k, _)| *k == "ganghouse.tax")
        );
    }

    // A character who has not played since the last cleanup is passed
    // over, and running again changes nothing (idempotent on a world with nothing to do)
    #[test]
    fn a_second_run_has_nothing_to_do() {
        let mut h = offline_harness();
        h.run_cleanup();
        let rooms = h.engine.world.room_state.clone();
        let stored = h.stored.clone();
        let gangs = h.engine.world.gangs.clone();
        let books = h.engine.world.bank_books.clone();
        let mark = h.log.len();
        let report = h.run_cleanup();
        assert_eq!(report.items_hidden, 0);
        assert_eq!(report.users, 0);
        assert_eq!(report.gangs_deleted, 0);
        assert_eq!(h.engine.world.room_state, rooms);
        assert_eq!(h.stored, stored);
        assert_eq!(h.engine.world.gangs, gangs);
        assert_eq!(h.engine.world.bank_books, books);
        assert!(!h.log[mark..].iter().any(|o| matches!(
            o,
            EngineOutput::Persist(PersistRequest::Character { .. })
                | EngineOutput::Persist(PersistRequest::RoomState(_))
        )));
    }

    // (sic) the walk of a map ends at its 50th missing
    // room number, so a room numbered past that is never swept
    #[test]
    fn the_room_walk_stops_after_fifty_gaps() {
        let mut h = offline_harness();
        let far = RoomKey::new(1, 60);
        let mut r = blank::clear_room(far);
        r.items[0] = slot(102, -1, 0);
        h.w()
            .room_state
            .insert(far, silvermere_engine::RoomState::of(&r));
        h.w().rooms.insert(far, r);
        assert!(!job::walk_order(&h.engine.world).contains(&far));
        h.run_cleanup();
        assert_eq!(h.w().room_state[&far].items[0].item, ItemNo(102));
    }

    // The scheduled run (`Routine::DailyCleanup`) at `game.cleanup.at`, with two players on
    // line: one idle in the square, one in the house that closes. Nobody is disconnected
    // or told anything but the closed-house notice and the new room; both keep playing;
    // a third player logs in while the store answers and after.
    #[test]
    fn online_players_stay_connected() {
        let mut h = Harness::new(config(), |_| {});
        h.stored.insert(LARS, lars());
        let ann = h.playing(character("Ann", START));
        let mut b = character("Bob", HOUSE);
        b.gang = "Wolves".into();
        b.items[0] = GANG_ITEM;
        b.itemuses[0] = -1;
        let bob = h.playing(b);
        // 00:20 UTC from the clock's start: 1200 fast ticks
        h.ticks(1199);
        assert_eq!(h.scans_since(0), 0);
        let mark = h.log.len();
        h.ticks(1);
        assert_eq!(h.scans_since(mark), 1, "the routine fired at 00:20");
        // a third client logs in while the job waits for the store
        let cid = h.at_menu(character("Cid", START));
        let (ann_before, bob_before) = (h.plr(ann).clone(), h.plr(bob).clone());
        assert!(ann_before.sesstime > 0 && bob_before.sesstime > 0);
        let report = h.answer_scan();
        assert_eq!(report.users, 3, "Ann, Bob and the stored Lars");
        // nobody was closed or logged off
        assert!(!h.log[mark..].iter().any(|o| matches!(
            o,
            EngineOutput::Close { .. } | EngineOutput::ReturnToLogin { .. }
        )));
        for id in [ann, bob] {
            assert_eq!(h.state(id), Some(SessionState::Playing));
        }
        assert_eq!(h.state(cid), Some(SessionState::MainMenu));
        // Ann sees nothing at all; Bob the notice, then the temple and his prompt
        assert_eq!(h.text_since(mark, ann), "");
        let t = h.text_since(mark, bob);
        let notice = t
            .find("Gang house items have dissappeared from your inventory!")
            .expect("the house items notice");
        let room = t.find("Temple").expect("the new room");
        assert!(notice < room, "{t:?}");
        assert_eq!(h.plr(bob).room, RoomId(REINC));
        assert_eq!(h.plr(bob).items[0], ItemNo(0));
        assert!(!h.plr(bob).flags.contains(PlayerFlags::HOUSEITEMS));
        // the minutes are folded into lifemins and not counted again
        for (id, before) in [(ann, &ann_before), (bob, &bob_before)] {
            let p = h.plr(id);
            assert_eq!(p.lifemins, before.lifemins + before.sesstime / 2);
            assert_eq!(p.sesstime, 0);
        }
        assert!(h.stored[&Harness::account(ann)].sesstime == 0);
        // everyone carries on
        let m = h.log.len();
        h.line(ann, "look");
        assert!(h.text_since(m, ann).contains("Square"));
        let m = h.log.len();
        h.line(bob, "look");
        assert!(h.text_since(m, bob).contains("Temple"));
        h.line(cid, "E");
        assert_eq!(h.state(cid), Some(SessionState::Playing));
        let late = h.at_menu(character("Dee", START));
        assert_eq!(h.state(late), Some(SessionState::MainMenu));
        // and the next run is armed for the same time tomorrow
        let next = h
            .engine
            .world
            .scheduler
            .pending()
            .find(|k| k.routine == silvermere_engine::Routine::DailyCleanup)
            .expect("re-armed")
            .when;
        assert_eq!(next, 1200 + 86_400);
    }

    // A session that ends while the store is answering: the stored row may be older than
    // its last save, so the run uses the record it saved, even when the row is stale or
    // not written yet
    #[test]
    fn a_player_leaving_during_the_scan_is_cleaned_from_his_last_save() {
        let mut h = Harness::new(config(), |_| {});
        let ann = h.playing(character("Ann", START));
        h.ticks(300);
        h.handle(EngineInput::DailyCleanup);
        h.handle(EngineInput::Disconnect { id: ann });
        let acct = Harness::account(ann);
        let saved = h.stored[&acct].clone();
        assert_eq!(saved.sesstime, 10);
        // the store has no row for him yet
        h.stored.remove(&acct);
        let report = h.answer_scan();
        assert_eq!(report.users, 1);
        let after = &h.stored[&acct];
        assert_eq!((after.sesstime, after.lifemins), (0, saved.lifemins + 5));
    }

    // Carried from the T-047 review:
    // the session minutes are stored and build up across logins; the midnight pass folds them into
    // lifemins, and the race-change undo window closes
    #[test]
    fn the_race_change_window_closes_after_the_midnight_pass() {
        use silvermere_engine::state::newchar::race_change::{can_user_change_race, change_open};
        let mut cfg = config();
        cfg.race_change_duration = 60;
        cfg.cleanup.at = "12:00".into();
        let mut h = Harness::new(cfg, |_| {});
        let mut c = character("Rae", START);
        c.lifemins = 100;
        c.racechg = 100;
        let id = h.playing(c);
        assert_eq!(change_open(&h.engine.world, id), Some(60));
        // 12 minutes of play, then the line drops
        h.ticks(12 * 60);
        h.handle(EngineInput::Disconnect { id });
        let acct = Harness::account(id);
        assert_eq!(h.stored[&acct].sesstime, 24);
        assert_eq!(h.stored[&acct].lifemins, 100);
        // the stored half-minutes count at the next login, before any cleanup: 48 left
        let cfg = h.engine.world.config.clone();
        let rec = h.stored[&acct].clone();
        assert_eq!(can_user_change_race(&rec, rec.sesstime, &cfg), Some(48));
        // the midnight pass folds them in: the same 48 minutes left
        h.run_cleanup();
        let rec = h.stored[&acct].clone();
        assert_eq!((rec.sesstime, rec.lifemins), (0, 112));
        assert_eq!(can_user_change_race(&rec, rec.sesstime, &cfg), Some(48));
        // and after the next login
        let back = h.at_menu(rec);
        assert_eq!(change_open(&h.engine.world, back), Some(48));
        // the window closes once 60 minutes of play are spent, across the logins
        h.w()
            .session_mut(back)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
            .sesstime = 98;
        assert_eq!(change_open(&h.engine.world, back), None);
    }

    // T-088: an offline edit queued while the store answers (here a gang promotion of a
    // hung-up character) is the record the run reads, the record as `save_player` last
    // stored it; a deletion queued
    // meanwhile leaves the account out
    #[test]
    fn an_offline_edit_during_the_scan_is_the_record_cleaned() {
        let mut h = Harness::new(config(), |_| {});
        let mut stale = evan();
        stale.status.insert(PlayerStatus::HUNGUP);
        h.stored.insert(EVAN, stale.clone());
        h.stored.insert(LARS, lars());
        h.handle(EngineInput::DailyCleanup);
        let mut edited = stale.clone();
        edited.flags.insert(PlayerFlags::PROMOTED);
        edited.hp = 3;
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                silvermere_engine::player::queue_character(w, EVAN, Box::new(edited));
                silvermere_engine::player::queue_character_delete(w, LARS);
            }),
        });
        // the store read before either landed
        h.stored.insert(EVAN, stale);
        h.stored.insert(LARS, lars());
        let report = h.answer_scan();
        assert_eq!(report.users, 1);
        let after = &h.stored[&EVAN];
        assert!(after.flags.contains(PlayerFlags::PROMOTED));
        assert!(after.status.contains(PlayerStatus::HUNGUP));
        assert_eq!(after.hp, 3);
        // the run did its work on that record
        assert_eq!((after.sesstime, after.lifemins), (0, 1120));
    }

    // review of #62: a deed holder who leaves while the store is answering keeps his
    // house; the run uses the record he just saved, not the stored row
    #[test]
    fn a_deed_holder_leaving_during_the_scan_keeps_the_house() {
        let mut h = Harness::new(config(), |_| {});
        let mut c = lars();
        c.room = RoomId(START);
        let id = h.playing(c);
        let acct = Harness::account(id);
        h.w().bank_books.insert(
            (acct, ShopNo(8)),
            BankBook {
                userid: "LARS".into(),
                bank: ShopNo(8),
                balance: 1500,
                loan: 0,
            },
        );
        h.handle(EngineInput::DailyCleanup);
        h.handle(EngineInput::Disconnect { id });
        // the stored row has not caught up: it still lacks the deed
        let mut stale = lars();
        stale.items[0] = ItemNo(0);
        h.stored.insert(acct, stale);
        let report = h.answer_scan();
        assert!(!report.houses[1].closed(), "{:?}", report.houses[1]);
        assert_eq!(report.houses[1].leader, "LARS");
        let rs = h.w().room_state[&key(HOUSE)].clone();
        assert_eq!(rs.items[0].item, ItemNo(130));
        assert_eq!(h.w().bank_books[&(acct, ShopNo(8))].balance, 500);
        // and the record written back is the one he left with, cleaned
        assert_eq!(h.stored[&acct].items[0], DEED);
        assert_eq!(h.stored[&acct].sesstime, 0);
    }

    // review of #62: a scan the store could not answer does not stop the cleanup for good
    #[test]
    fn a_failed_scan_is_retried_by_the_next_run() {
        let mut h = Harness::new(config(), |_| {});
        h.stored.insert(EVAN, evan());
        h.handle(EngineInput::DailyCleanup);
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(job::on_scan_failed),
        });
        assert!(h.engine.world.cleanup.pending.is_none());
        let mark = h.log.len();
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(job::on_due),
        });
        assert_eq!(h.scans_since(mark), 1);
        let report = h.answer_scan();
        assert_eq!(report.users, 1);
        assert_eq!(h.stored[&EVAN].evil, 45);
    }

    // a scan that was lost: the next start asks again, and only the first answer runs
    #[test]
    fn an_unanswered_scan_is_asked_again() {
        let mut h = Harness::new(config(), |_| {});
        h.stored.insert(EVAN, evan());
        let mark = h.log.len();
        h.handle(EngineInput::DailyCleanup);
        h.handle(EngineInput::DailyCleanup);
        assert_eq!(h.scans_since(mark), 2);
        assert_eq!(h.answer_scan().users, 1);
        assert!(h.try_answer_scan().is_none());
        assert_eq!(h.stored[&EVAN].evil, 45);
    }
}

mod time_zone {
    use super::*;
    use silvermere_engine::scheduler::Routine;

    fn ts(s: &str) -> i64 {
        s.parse::<jiff::Timestamp>().unwrap().as_second()
    }

    /// Seconds to the armed cleanup with `tz` (None: the default) at `wall`.
    fn armed(tz: Option<&str>, wall: &str) -> u64 {
        let mut cfg = config();
        cfg.cleanup.at = "03:00".into();
        match tz {
            Some(z) => cfg.cleanup.tz = z.into(),
            None => cfg.cleanup = silvermere_types::config::CleanupConfig::default(),
        }
        let mut world = World::load(cfg, 8, snapshot()).expect("world loads");
        world.set_wall(ts(wall));
        job::arm(&mut world);
        let k = world
            .scheduler
            .pending()
            .find(|k| k.routine == Routine::DailyCleanup)
            .expect("armed");
        k.when - world.scheduler.tick()
    }

    #[test]
    fn default_is_america_detroit() {
        assert_eq!(config().cleanup.tz, "America/Detroit");
    }

    // 2026 DST change in Detroit: Sunday 8 March, 02:00 EST -> 03:00 EDT
    #[test]
    fn default_zone_each_side_of_the_dst_change() {
        // Sat 7 Mar 12:00 EST: 03:00 on the 8th is EDT (07:00Z), 14 hours on
        assert_eq!(armed(None, "2026-03-07T17:00:00Z"), 14 * 3600);
        // Fri 6 Mar 12:00 EST: 03:00 on the 7th is EST (08:00Z), 15 hours on
        assert_eq!(armed(None, "2026-03-06T17:00:00Z"), 15 * 3600);
        // Sun 8 Mar 12:00 EDT: 03:00 on the 9th is EDT, 15 hours on
        assert_eq!(armed(None, "2026-03-08T16:00:00Z"), 15 * 3600);
    }

    #[test]
    fn utc_fires_at_the_hour_in_utc() {
        assert_eq!(armed(Some("UTC"), "2026-03-07T17:00:00Z"), 10 * 3600);
        // Detroit at the same instant is 14 hours
        assert_eq!(armed(None, "2026-03-07T17:00:00Z"), 14 * 3600);
    }

    #[test]
    fn an_unknown_zone_is_a_config_error_naming_the_key() {
        let mut cfg = config();
        cfg.cleanup.tz = "Mars/Olympus".into();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("game.cleanup.tz"), "{err}");
        let mut ok = config();
        ok.cleanup.tz = "UTC".into();
        assert!(ok.validate().is_ok());
    }
}

mod hangup {
    use super::*;

    fn punishing(pun_hup: PunHup) -> GameConfig {
        GameConfig {
            pun_hup,
            min_hup_hp: 20,
            max_hup_hp: 40,
            max_hup_items: 3,
            ..config()
        }
    }

    /// Wes, carrying the floor-test items 101..=106 in slots 0..=5, at 100 hit points.
    fn wes() -> Character {
        let mut c = character("Wes", START);
        c.hpbonus = 75;
        c.maxhp = 100;
        c.hp = 100;
        for (i, n) in (101..=106).enumerate() {
            c.items[i] = ItemNo(n);
            c.itemuses[i] = -1;
        }
        c
    }

    fn floor(h: &Harness, room: u32) -> Vec<i32> {
        h.engine.world.room_state[&key(room)]
            .items
            .iter()
            .map(|s| s.item.0)
            .filter(|&n| n != 0)
            .collect()
    }

    // The hang-up punishment with PUNHUP=HIGH, then `load_player`
    #[test]
    fn a_dropped_line_is_punished() {
        let mut h = Harness::new(punishing(PunHup::High), |w| {
            w.room_state.get_mut(&key(START)).unwrap().items = [RoomItem {
                item: ItemNo(0),
                uses: -1,
                count: 0,
            };
                silvermere_types::sizes::NROOMITEMS];
        });
        let watcher = h.playing(character("Wanda", REINC));
        let id = h.playing(wes());
        // in a fight: a monster's round has just hit him (the combat state itself is
        // T-034's; PUNHUP=HIGH punishes whatever he was doing)
        h.ticks(3);
        let mark = h.log.len();
        h.handle(EngineInput::Disconnect { id });
        assert!(
            h.text_since(mark, watcher)
                .contains("\x1b[79D\x1b[K\x1b[1;37mWes just disconnected!!!\r\n")
        );
        let acct = Harness::account(id);
        let saved = h.stored[&acct].clone();
        // 20..=40 per cent of 100 hit points
        let lost = 100 - i32::from(saved.hp);
        assert!((20..=40).contains(&lost), "lost {lost}");
        // every second slot, at most MAXHUPIT: 101, 103, 105 on the old room's floor
        assert_eq!(floor(&h, START), [101, 103, 105]);
        let kept: Vec<i32> = saved
            .items
            .iter()
            .map(|i| i.0)
            .filter(|&n| n != 0)
            .collect();
        assert_eq!(kept, [102, 104, 106]);
        assert_eq!(saved.room, RoomId(REINC), "the room's hang-up room");
        assert!(saved.status.contains(PlayerStatus::HUNGUP));
        // the next entry: the two lines, then the room
        let back = h.at_menu(saved);
        let mark = h.log.len();
        h.line(back, "E");
        let t = h.text_since(mark, back);
        let a = t
            .find("Last time you were on, you disconnected while playing.\r\n")
            .expect("first line");
        let b = t
            .find("The gods have punished you appropriately.\r\n")
            .expect("second line");
        let room = t.find("Temple").expect("the room");
        assert!(a < b && b < room, "{t:?}");
    }

    // PUNHUP=NONE takes nothing. (He is still moved to the hang-up
    // room, marked hung up and announced to the game: only the hit points
    // and the items are spared.)
    #[test]
    fn punhup_none_takes_nothing() {
        let mut h = Harness::new(punishing(PunHup::None), |_| {});
        let id = h.playing(wes());
        let before = floor(&h, START);
        h.handle(EngineInput::Disconnect { id });
        let saved = h.stored[&Harness::account(id)].clone();
        assert_eq!(saved.hp, 100);
        assert_eq!(saved.items[..6], wes().items[..6]);
        assert_eq!(floor(&h, START), before);
    }

    // The logoff callback: x, the meditation, X at the menu: nothing is
    // punished
    #[test]
    fn a_clean_logoff_punishes_nothing() {
        let mut h = Harness::new(punishing(PunHup::High), |_| {});
        let id = h.playing(wes());
        let before = floor(&h, START);
        h.line(id, "x");
        h.ticks(10);
        assert_eq!(h.state(id), Some(SessionState::MainMenu));
        h.line(id, "x");
        h.ticks(1);
        let saved = h.stored[&Harness::account(id)].clone();
        assert_eq!(saved.hp, 100);
        assert_eq!(saved.items[..6], wes().items[..6]);
        assert!(!saved.status.contains(PlayerStatus::HUNGUP));
        assert_eq!(floor(&h, START), before);
    }

    // `cleanup_when_user_leaves`: his followers stop following him
    // and his pending commands go
    #[test]
    fn leaving_breaks_up_the_party() {
        let mut h = Harness::new(punishing(PunHup::None), |_| {});
        let lead = h.playing(character("Lea", START));
        let fol = h.playing(character("Fay", START));
        h.w().session_mut(fol).unwrap().party.leader = Some(lead);
        h.w().session_mut(lead).unwrap().party.group[0] = Some(fol);
        let mark = h.log.len();
        h.handle(EngineInput::Disconnect { id: lead });
        assert_eq!(h.engine.world.session(fol).unwrap().party.leader, None);
        assert!(h.text_since(mark, fol).contains("no longer following"));
    }

    // deleting an offline character: the gang loses a member (and is
    // disbanded when he led it), his limited copies are released, the row is deleted
    #[test]
    fn a_deleted_account_takes_its_character() {
        let mut h = Harness::new(config(), |w| {
            w.items.get_mut(&ItemNo(101)).unwrap().limit = 1;
            w.items.get_mut(&ItemNo(101)).unwrap().active = 1;
        });
        let mut c = lars();
        c.items[2] = ItemNo(101);
        let mark = h.log.len();
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                silvermere_engine::session::on_account_deleted(w, LARS, Some(c))
            }),
        });
        let g = &h.engine.world.gangs["WOLVES"];
        assert_eq!(g.members, 1);
        assert!(g.flags.contains(GangFlags::DISBANDED));
        assert_eq!(h.engine.world.items[&ItemNo(101)].active, 0);
        assert!(!h.engine.world.bank_books.contains_key(&(LARS, ShopNo(8))));
        assert!(h.log[mark..].iter().any(|o| matches!(
            o,
            EngineOutput::Persist(PersistRequest::DeleteCharacter { account }) if *account == LARS
        )));
    }
}

// T-069: `DELETE /v1/accounts/{id}` through the engine. The harness plays the store's
// half: the answer to `AccountDeleteScan` is `session::on_account_scan` with what it read.
mod account_delete {
    use std::sync::mpsc::{Receiver, channel};

    use silvermere_engine::AdminRequest;
    use silvermere_engine::session::{AccountDeletion, AccountScan, on_account_scan};

    use super::*;

    fn limited() -> Harness {
        Harness::new(config(), |w| {
            w.items.get_mut(&ItemNo(101)).unwrap().limit = 1;
            w.items.get_mut(&ItemNo(101)).unwrap().active = 1;
        })
    }

    fn ask(h: &mut Harness, account: AccountId) -> Receiver<AccountDeletion> {
        let (reply, rx) = channel();
        h.handle(EngineInput::Admin(AdminRequest::DeleteAccount {
            account,
            reply,
        }));
        rx
    }

    fn answer(h: &mut Harness, account: AccountId, scan: AccountScan) {
        h.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| on_account_scan(w, account, scan)),
        });
    }

    fn scans(h: &Harness, from: usize, account: AccountId) -> Vec<usize> {
        (from..h.log.len())
            .filter(|&i| h.log[i] == EngineOutput::AccountDeleteScan { account })
            .collect()
    }

    // a character in the game is logged off by the normal path and saved; the store is
    // asked after that save, and the deletion runs on what it read
    #[test]
    fn a_player_in_the_game_is_logged_off_then_deleted() {
        let mut h = limited();
        let mut c = lars();
        c.items[2] = ItemNo(101);
        let id = h.playing(c);
        let account = Harness::account(id);
        let mark = h.log.len();
        let first = ask(&mut h, account);
        assert!(h.engine.world.session(id).is_none(), "logged off");
        assert!(h.log[mark..].contains(&EngineOutput::Close { id }));
        let save = (mark..h.log.len())
            .find(|&i| {
                matches!(&h.log[i],
                    EngineOutput::Persist(PersistRequest::Character { account: a, .. }) if *a == account)
            })
            .expect("the logoff save");
        let asked = scans(&h, mark, account);
        assert_eq!(asked.len(), 1);
        assert!(asked[0] > save, "the store is asked after the save");
        assert!(first.try_recv().is_err(), "no answer before the store's");
        // a second request waits for the same answer
        let second = ask(&mut h, account);
        assert_eq!(scans(&h, mark, account).len(), 1);

        let stored = h.stored[&account].clone();
        let mark = h.log.len();
        answer(&mut h, account, AccountScan::Found(Some(Box::new(stored))));
        let queued = AccountDeletion::Queued {
            character: Some("Lars".into()),
        };
        assert_eq!(first.try_recv().unwrap(), queued);
        assert_eq!(second.try_recv().unwrap(), queued);
        let g = &h.engine.world.gangs["WOLVES"];
        assert_eq!(g.members, 1);
        assert_eq!(h.engine.world.items[&ItemNo(101)].active, 0);
        assert!(h.engine.world.account_deletes.is_empty());
        let persisted: Vec<&PersistRequest> = h.log[mark..]
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Persist(p) => Some(p),
                _ => None,
            })
            .collect();
        let at = |want: &PersistRequest| persisted.iter().position(|p| *p == want);
        let character = at(&PersistRequest::DeleteCharacter { account }).expect("row");
        let row = at(&PersistRequest::DeleteAccount(account)).expect("account row");
        assert!(character < row);
        assert!(
            persisted[..row]
                .iter()
                .any(|p| matches!(p, PersistRequest::Gang(g) if g.key == "WOLVES")),
            "the gang's write goes before the account row"
        );
        assert!(
            !persisted[row..]
                .iter()
                .any(|p| matches!(p, PersistRequest::Character { .. })),
            "nothing saves the character after its deletion"
        );
    }

    // an account nobody plays on is read at once; one that logs in while the store reads
    // is logged off and read again
    #[test]
    fn a_login_during_the_read_is_logged_off_and_read_again() {
        let mut h = limited();
        let account = AccountId(1);
        let rx = ask(&mut h, account);
        assert_eq!(scans(&h, 0, account).len(), 1, "asked at once");
        let id = h.playing(lars());
        assert_eq!(Harness::account(id), account, "the fixture's first account");
        let mark = h.log.len();
        answer(&mut h, account, AccountScan::Found(None));
        assert!(rx.try_recv().is_err());
        assert!(h.engine.world.session(id).is_none());
        assert_eq!(scans(&h, mark, account).len(), 1, "asked again");
        let stored = h.stored[&account].clone();
        answer(&mut h, account, AccountScan::Found(Some(Box::new(stored))));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AccountDeletion::Queued { character: Some(n) } if n == "Lars"
        ));
    }

    // T-072: the gate read the character before the deletion and hands the login in after
    // it: the session is closed before it reaches the game, and the limited item it
    // carries stays released
    #[test]
    fn a_login_racing_the_deletion_never_reaches_the_game() {
        let mut h = limited();
        let mut c = lars();
        c.items[2] = ItemNo(101);
        let id = h.playing(c);
        let account = Harness::account(id);
        let rx = ask(&mut h, account);
        let stale = h.stored[&account].clone();
        answer(
            &mut h,
            account,
            AccountScan::Found(Some(Box::new(stale.clone()))),
        );
        assert!(matches!(
            rx.try_recv().unwrap(),
            AccountDeletion::Queued { .. }
        ));
        assert_eq!(h.engine.world.items[&ItemNo(101)].active, 0);

        let id = h.connect();
        let mark = h.log.len();
        h.handle(EngineInput::LoggedIn {
            id,
            login: Box::new(Login {
                account: AccountRef {
                    id: account,
                    username: "Lars".into(),
                    sysop: false,
                    gameop: false,
                    sex: b'M',
                },
                character: Some(Box::new(stale)),
                may_gossip: true,
                may_auction: true,
                sysop_keys: silvermere_engine::commands::sysop::Keys::empty(),
            }),
        });
        assert!(h.log[mark..].contains(&EngineOutput::Close { id }));
        assert!(h.engine.world.session(id).is_none(), "closed");
        assert!(h.text_since(mark, id).contains("has been deleted"));
        // typing at it does nothing either
        h.line(id, "M");
        h.line(id, "E");
        assert!(
            h.engine
                .world
                .sessions
                .values()
                .all(|s| s.account.as_ref().is_none_or(|a| a.id != account))
        );
        assert_eq!(h.engine.world.items[&ItemNo(101)].active, 0);
        assert!(
            !h.engine
                .world
                .item_ownership
                .values()
                .any(|o| o.itemnum == ItemNo(101)),
            "no copy of the limited item is owned"
        );
        assert!(
            !h.log[mark..].iter().any(|o| matches!(
                o,
                EngineOutput::Persist(PersistRequest::Character { account: a, .. }) if *a == account
            )),
            "nothing saves the deleted account's character"
        );
    }

    #[test]
    fn an_unknown_account_or_a_failed_read_deletes_nothing() {
        let mut h = limited();
        for (scan, want) in [
            (AccountScan::NoAccount, AccountDeletion::NotFound),
            (
                AccountScan::Failed("down".into()),
                AccountDeletion::Failed("down".into()),
            ),
        ] {
            let rx = ask(&mut h, AccountId(77));
            let mark = h.log.len();
            answer(&mut h, AccountId(77), scan);
            assert_eq!(rx.try_recv().unwrap(), want);
            assert!(
                !h.log[mark..]
                    .iter()
                    .any(|o| matches!(o, EngineOutput::Persist(_))),
                "nothing persisted"
            );
        }
        assert_eq!(h.engine.world.gangs["WOLVES"].members, 2);
    }
}
