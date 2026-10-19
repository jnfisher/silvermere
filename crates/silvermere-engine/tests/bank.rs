//! Banks (T-042): DEPOSIT, WITHDRAW, BANKBOOK/BALANCE, BORROW and REPAY through the whole
//! input pipeline: `cmd_deposit`/`cmd_withdraw`/`cmd_bankbook`, `deposit_gold`/`withdraw_gold`
//! and `display_bank_balance`, as the original behaves.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, special_rooms, tiny_world};
use silvermere_engine::commands::bank::BankBooks;
use silvermere_engine::{
    AccountRef, Engine, EngineInput, EngineOutput, PersistRequest, SeededRng, SessionId, World,
};
use silvermere_text::strip_ansi_str;
use silvermere_types::enums::{MessageLevel, RoomType, ShopType};
use silvermere_types::flags::PlayerSettings;
use silvermere_types::ids::{AccountId, MapId, RoomId, RoomKey, ShopNo};
use silvermere_types::records::BankBook;
use silvermere_types::records::{Character, Coins, Shop, ShopSlot};

/// Shop 7 is the bank in the first special room, shop 9 a second bank in the second room,
/// shop 8 a store in the third.
const BANK: ShopNo = ShopNo(7);
const BANK2: ShopNo = ShopNo(9);

fn shop(number: ShopNo, name: &str, shop_type: ShopType) -> Shop {
    Shop {
        number,
        name: name.into(),
        desc: Default::default(),
        shop_type,
        minlevel: 0,
        maxlevel: 0,
        markup: 0,
        ownable: 0,
        expandable: 0,
        trainclass: silvermere_types::ids::ClassNo(0),
        stock: std::array::from_fn(|_| ShopSlot {
            item: silvermere_types::ids::ItemNo(0),
            maxqty: 0,
            qty: 0,
            regentime: 0,
            regennum: 0,
            regenpct: 0,
        }),
    }
}

/// A world with banks in the first two special rooms, a store in the third and a plain
/// fourth room.
fn bank_world(w: &mut World) {
    let rooms = special_rooms();
    let table = [
        (rooms[0], BANK, "First Bank", ShopType::Bank),
        (rooms[1], BANK2, "Second Bank", ShopType::Bank),
        (rooms[2], ShopNo(8), "The Store", ShopType::from_code(1)),
    ];
    for (room, no, name, kind) in table {
        w.shops.insert(no, shop(no, name, kind));
        let r = w
            .rooms
            .get_mut(&RoomKey {
                map: MapId(1),
                room: RoomId(room),
            })
            .unwrap();
        r.room_type = RoomType::Shop;
        r.shopnum = no;
    }
}

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
}

impl Harness {
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

const PROMPT: &str = "[HP=48]:";

fn coins(runic: u32, platinum: u32, gold: u32, silver: u32, copper: u32) -> Coins {
    Coins {
        runic,
        platinum,
        gold,
        silver,
        copper,
    }
}

fn bank() -> Harness {
    Harness::with(bank_world)
}

fn purse(h: &Harness, id: SessionId) -> Coins {
    h.plr(id).coins
}

fn balance(h: &Harness, id: SessionId, bank: ShopNo) -> Option<u32> {
    let acct = h
        .engine
        .world
        .session(id)
        .unwrap()
        .account
        .as_ref()
        .unwrap()
        .id;
    h.engine
        .world
        .bank_books
        .book(acct, bank)
        .map(|b| b.balance)
}

fn move_to(h: &mut Harness, id: SessionId, room: u32) {
    h.plr_mut(id).room = RoomId(room);
}

#[test]
fn deposit_bankbook_withdraw_round_trip() {
    let mut h = bank();
    let id = h.player("Ann");
    h.plr_mut(id).coins = coins(0, 0, 10, 0, 0); // 1000 copper
    // the coins handed over are listed
    assert_eq!(
        h.line(id, "deposit 523"),
        format!("You deposit 5 gold crowns, 2 silver nobles, 3 copper farthings.\n{PROMPT}")
    );
    assert_eq!(balance(&h, id, BANK), Some(523));
    // 1000 - 523 = 477 copper, tidied into larger coins (the change-up into larger coins)
    assert_eq!(purse(&h, id), coins(0, 0, 4, 7, 7));
    // the balance lines
    assert_eq!(
        h.line(id, "bankbook"),
        format!(
            "Your balance at First Bank (#7) is:\nOn deposit: 523 copper farthings [5.23 gold crowns]\n{PROMPT}"
        )
    );
    assert_eq!(
        h.line(id, "withdraw 500"),
        format!("You withdrew 500 copper farthings.\n{PROMPT}")
    );
    assert_eq!(balance(&h, id, BANK), Some(23));
    // 477 + 500 = 977 copper tidied: 9 gold, 7 silver, 7 copper
    assert_eq!(purse(&h, id), coins(0, 0, 9, 7, 7));
    assert_eq!(
        h.line(id, "balance"),
        format!(
            "Your balance at First Bank (#7) is:\nOn deposit: 23 copper farthings [0.23 gold crowns]\n{PROMPT}"
        )
    );
}

#[test]
fn singular_and_commas() {
    let mut h = bank();
    let id = h.player("Ann");
    h.engine.world.bank_books.insert(
        (AccountId(i64::from(id.0) + 1), BANK),
        BankBook {
            userid: "ANN".into(),
            bank: BANK,
            balance: 123_456_701,
            loan: 0,
        },
    );
    assert_eq!(
        h.line(id, "bankbook"),
        format!(
            "Your balance at First Bank (#7) is:\nOn deposit: 123456701 copper farthings [1,234,567.01 gold crowns]\n{PROMPT}"
        )
    );
    h.plr_mut(id).coins = coins(0, 0, 0, 0, 1);
    assert_eq!(
        h.line(id, "deposit 1"),
        format!("You deposit 1 copper farthing.\n{PROMPT}")
    );
    assert_eq!(
        h.line(id, "withdraw 1"),
        format!("You withdrew 1 copper farthing.\n{PROMPT}")
    );
}

#[test]
fn a_book_is_made_by_the_first_deposit_and_each_bank_has_its_own() {
    let mut h = bank();
    let id = h.player("Ann");
    h.plr_mut(id).coins = coins(0, 0, 0, 0, 900);
    // no book on file shows as a balance of 0, and is not stored
    assert_eq!(
        h.line(id, "bankbook"),
        format!(
            "Your balance at First Bank (#7) is:\nOn deposit: 0 copper farthings [0.00 gold crowns]\n{PROMPT}"
        )
    );
    assert_eq!(balance(&h, id, BANK), None);
    h.line(id, "deposit 100");
    move_to(&mut h, id, special_rooms()[1]);
    h.line(id, "deposit 250");
    assert_eq!(balance(&h, id, BANK), Some(100));
    assert_eq!(balance(&h, id, BANK2), Some(250));
    // two rows for one account reach the persistence worker, userid kept
    h.pump();
    let books: Vec<_> = h
        .persisted()
        .into_iter()
        .filter_map(|p| match p {
            PersistRequest::BankBook { account, book } => {
                Some((*account, book.bank, book.balance, book.userid.clone()))
            }
            _ => None,
        })
        .collect();
    let acct = AccountId(i64::from(id.0) + 1);
    assert_eq!(
        books,
        vec![
            (acct, BANK, 100, "ANN".to_string()),
            (acct, BANK2, 250, "ANN".to_string())
        ]
    );
    // outside a bank, the plain word lists the books in bank order
    move_to(&mut h, id, special_rooms()[3]);
    assert_eq!(
        h.line(id, "bankbook"),
        format!(
            "Your balance at First Bank (#7) is:\nOn deposit: 100 copper farthings [1.00 gold crowns]\nYour balance at Second Bank (#9) is:\nOn deposit: 250 copper farthings [2.50 gold crowns]\n{PROMPT}"
        )
    );
    // BANKBOOK {n}: one bank; a shop that is no bank, or none, shows nothing
    assert_eq!(
        h.line(id, "bankbook 9"),
        format!(
            "Your balance at Second Bank (#9) is:\nOn deposit: 250 copper farthings [2.50 gold crowns]\n{PROMPT}"
        )
    );
    assert_eq!(h.line(id, "bankbook 8"), PROMPT);
    assert_eq!(h.line(id, "bankbook 77"), PROMPT);
    assert_eq!(h.line(id, "bankbook -7"), PROMPT);
    // the leading number wrapped to a signed 16-bit value: 65543 is 7
    assert!(h.line(id, "bankbook 65543").contains("First Bank (#7)"));
}

#[test]
fn list_in_a_bank_is_the_rate_card_only() {
    let mut h = bank();
    let id = h.player("Ann");
    assert_eq!(
        h.line(id, "list"),
        format!("Banking services:\nDeposit Rate: ??\nLoan Rate: ??\n{PROMPT}")
    );
}

#[test]
fn nobody_banked_lists_nothing() {
    let mut h = bank();
    let id = h.player("Ann");
    move_to(&mut h, id, special_rooms()[3]);
    assert_eq!(h.line(id, "bankbook"), PROMPT);
}

#[test]
fn refusals_in_the_original_order() {
    let mut h = bank();
    let id = h.player("Ann");
    h.plr_mut(id).coins = coins(0, 0, 0, 0, 100);
    // deposit / withdraw
    assert_eq!(
        h.line(id, "deposit"),
        format!("Syntax: DEPOSIT {{Amount to deposit in copper}}\n{PROMPT}")
    );
    assert_eq!(
        h.line(id, "withdraw"),
        format!("Syntax: WITHDRAW {{Amount to withdraw in copper}}\n{PROMPT}")
    );
    // DEPOSIT takes two or more words, WITHDRAW exactly 2
    assert!(h.line(id, "withdraw 1 2").starts_with("Syntax: WITHDRAW"));
    assert!(h.line(id, "bankbook 1").starts_with("Syntax: BANKBOOK"));
    // Nothing, text or a bad number reads as 0; ten digits count as 0
    for bad in ["0", "abc", "1234567890"] {
        assert_eq!(
            h.line(id, &format!("deposit {bad}")),
            format!("Please specify a more reasonable amount.\n{PROMPT}"),
            "{bad}"
        );
        assert_eq!(
            h.line(id, &format!("withdraw {bad}")),
            format!("Please specify a more reasonable amount.\n{PROMPT}")
        );
    }
    // a store and a plain room: the same text, DEPOSIT and WITHDRAW worded alike
    for room in [special_rooms()[2], special_rooms()[3]] {
        move_to(&mut h, id, room);
        assert_eq!(
            h.line(id, "deposit 5"),
            format!("You cannot DEPOSIT if you are not in a bank!\n{PROMPT}")
        );
        assert_eq!(
            h.line(id, "withdraw 5"),
            format!("You cannot WITHDRAW if you are not in a bank!\n{PROMPT}")
        );
        // the check comes before the argument count
        assert_eq!(
            h.line(id, "deposit"),
            format!("You cannot DEPOSIT if you are not in a bank!\n{PROMPT}")
        );
    }
    assert_eq!(purse(&h, id), coins(0, 0, 0, 0, 100));
}

#[test]
fn money_that_cannot_move_is_silent_but_the_room_still_looks() {
    let mut h = bank();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    h.plr_mut(ann).coins = coins(0, 0, 0, 0, 50);
    // more than carried (the purse check fails inside deposit_gold): no text, no change
    assert_eq!(h.line(ann, "deposit 51"), PROMPT);
    assert_eq!(purse(&h, ann), coins(0, 0, 0, 0, 50));
    assert_eq!(balance(&h, ann, BANK), None);
    // withdraw with no book: the same
    assert_eq!(h.line(ann, "withdraw 5"), PROMPT);
    // and no ANSI_NORMAL reaches the user: the buffer it waited in went to the room (m3/bank
    // against the original: these four steps differed only in colour)
    for line in ["deposit 51", "withdraw 5", "deposit 99", "withdraw 99"] {
        let from = h.log.len();
        h.line(ann, line);
        h.pump();
        let bytes = h.written(from, ann);
        assert_eq!(
            bytes,
            b"\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:",
            "{line}: {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }
    // the others see it either way, with their prompt
    let seen = h.line(bob, "look");
    assert!(!seen.is_empty());
    let from = h.log.len();
    h.line(ann, "withdraw 5");
    h.pump();
    // nothing was told to Ann, so the buffer, the dispatcher's ANSI_NORMAL first, goes to the room
    assert_eq!(
        String::from_utf8_lossy(&h.written(from, bob)),
        "\x1b[0;37;40m\x1b[79D\x1b[KYou see Ann making a withdrawal.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
    h.line(ann, "deposit 20");
    let from = h.log.len();
    h.line(ann, "deposit 10");
    h.pump();
    // the money moved: Ann was told, and the buffer with her; the room gets the line alone
    assert_eq!(
        String::from_utf8_lossy(&h.written(from, bob)),
        "\x1b[79D\x1b[KYou see Ann making a deposit.\r\n\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:"
    );
    // a withdrawal larger than the balance: silent
    assert_eq!(h.line(ann, "withdraw 31"), PROMPT);
    assert_eq!(balance(&h, ann, BANK), Some(30));
    // a negative number is a huge unsigned amount: it can be neither paid nor drawn
    assert_eq!(h.line(ann, "deposit -5"), PROMPT);
    assert_eq!(h.line(ann, "withdraw -5"), PROMPT);
    assert_eq!(balance(&h, ann, BANK), Some(30));
}

#[test]
fn a_balance_never_wraps() {
    let mut h = bank();
    let id = h.player("Ann");
    let acct = AccountId(i64::from(id.0) + 1);
    h.engine.world.bank_books.insert(
        (acct, BANK),
        BankBook {
            userid: "ANN".into(),
            bank: BANK,
            balance: u32::MAX - 10,
            loan: 0,
        },
    );
    h.plr_mut(id).coins = coins(0, 0, 0, 0, 50);
    assert_eq!(
        h.line(id, "deposit 11"),
        format!("The bank cannot accept your deposit at this time.\n{PROMPT}")
    );
    // the coins stay, but the purse is still tidied up: 50 copper is 5 silver
    assert_eq!(purse(&h, id), coins(0, 0, 0, 5, 0));
    assert_eq!(balance(&h, id, BANK), Some(u32::MAX - 10));
    assert!(
        h.line(id, "deposit 10")
            .starts_with("You deposit 1 silver noble.")
    );
    assert_eq!(balance(&h, id, BANK), Some(u32::MAX));
}

#[test]
fn deposits_and_withdrawals_save_the_character() {
    let mut h = bank();
    let id = h.player("Ann");
    h.plr_mut(id).coins = coins(0, 0, 0, 0, 80);
    h.line(id, "deposit 30");
    h.line(id, "withdraw 10");
    h.pump();
    let saves = h
        .persisted()
        .into_iter()
        .filter(|p| matches!(p, PersistRequest::Character { .. }))
        .count();
    assert_eq!(saves, 2);
}

#[test]
fn borrow_and_repay_are_no_commands() {
    let mut h = bank();
    let id = h.player("Ann");
    // the stubs are not commands, so the word is handled as any unknown one
    let unknown = h.line(id, "frobnicate 5");
    assert_eq!(h.line(id, "borrow 5"), unknown);
    assert_eq!(h.line(id, "repay 5"), unknown);
    assert_eq!(h.line(id, "borrow"), unknown);
    move_to(&mut h, id, special_rooms()[3]);
    assert_eq!(h.line(id, "repay"), unknown);
}

#[test]
fn the_sysops_maximum_cuts_a_balance_when_it_is_read() {
    // MAXIMUM_BANK_BALANCE caps a book as it is loaded
    let mut h = Harness::with(|w| {
        bank_world(w);
        w.config.maximum_bank_balance = 1000;
    });
    let id = h.player("Ann");
    h.engine.world.bank_books.insert(
        (AccountId(i64::from(id.0) + 1), BANK),
        BankBook {
            userid: "ANN".into(),
            bank: BANK,
            balance: 5000,
            loan: 0,
        },
    );
    assert!(h.line(id, "bankbook").contains("On deposit: 1000 copper"));
}

#[test]
fn debit_for_the_gang_tax() {
    let mut h = bank();
    let id = h.player("Ann");
    let acct = AccountId(i64::from(id.0) + 1);
    h.plr_mut(id).coins = coins(0, 0, 0, 0, 200);
    h.line(id, "deposit 200");
    let books = &mut h.engine.world.bank_books;
    assert_eq!(books.debit(acct, BANK, 150), Ok(50));
    assert_eq!(
        books.debit(acct, BANK, 51),
        Err(silvermere_engine::commands::bank::WithdrawError::Short)
    );
    assert_eq!(
        books.debit(acct, BANK2, 1),
        Err(silvermere_engine::commands::bank::WithdrawError::NoBook)
    );
}

#[test]
fn purse_plus_balance_is_conserved() {
    // a fixed pseudo-random run of deposits, withdrawals and refusals
    let mut h = bank();
    let id = h.player("Ann");
    h.plr_mut(id).coins = coins(1, 3, 12, 4, 77);
    let total = |h: &Harness| {
        silvermere_rules::economy::to_copper(purse(h, id)) + balance(h, id, BANK).unwrap_or(0)
    };
    let start = total(&h);
    let mut x: u32 = 12345;
    for _ in 0..200 {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let amount = (x >> 8) % 200_000;
        let cmd = if x & 1 == 0 { "deposit" } else { "withdraw" };
        h.line(id, &format!("{cmd} {amount}"));
        assert_eq!(total(&h), start, "{cmd} {amount}");
    }
}
