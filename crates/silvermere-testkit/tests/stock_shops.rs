//! Shops (T-041): LIST, BUY, SELL and APPRAISE through the whole input pipeline
//! (`cmd_list`/`cmd_buy`/`cmd_sell`, `cmd_appraise`, `buy_item`/`sell_item`,
//! `display_shop_items` and the restock), against the original's visible behaviour and the
//! T-005 `shop` capture of the original (Newhaven weapon, armour and general stores).
//!
//! The shops and items are the stock records (shops 5, 45, 46 and 47, their map 1 rooms
//! and the items they list, plus the quarterstaff and the rapier), built from the
//! downloaded distribution. Every test sits in `mod shop`, so `cargo test -p
//! silvermere-testkit shop::` runs them.

use crate::engine_common as common;
use crate::stock_records;

mod shop {
    use std::sync::mpsc::Receiver;

    use proptest::prelude::*;
    use serde_json::Value;

    use super::common::{TestClock, enter, test_config, tiny_snapshot};
    use silvermere_engine::scheduler::Rng;
    use silvermere_engine::shops::{self, RestockNode};
    use silvermere_engine::world::blank;
    use silvermere_engine::{
        Engine, EngineInput, EngineOutput, PersistRequest, SeededRng, SessionId, World,
    };
    use silvermere_text::strip_ansi_str;
    use silvermere_types::enums::{
        Ability, ArmourType, Currency, ItemType, MessageLevel, RoomType, ShopType, WeaponClass,
        WeaponType, WornLocation,
    };
    use silvermere_types::flags::PlayerFlags;
    use silvermere_types::ids::{ClassNo, ItemNo, RoomKey, ShopNo};
    use silvermere_types::records::{Coins, Gang, Shop, ShopSlot};
    use silvermere_types::sizes::NSHOPITEMS;

    const SWORD_SHOP: ShopNo = ShopNo(5);
    const WEAPON_SHOP: ShopNo = ShopNo(45);
    const ARMOUR_SHOP: ShopNo = ShopNo(46);
    const GENERAL_STORE: ShopNo = ShopNo(47);
    /// Rooms of the fixture shops (map 1).
    const SWORD_ROOM: u32 = 355;
    const WEAPON_ROOM: u32 = 2141;
    const ARMOUR_ROOM: u32 = 2142;
    const STORE_ROOM: u32 = 2145;
    /// Rooms this test adds: a hospital, a gang shop, a deed shop, a plain room.
    const HOSPITAL_ROOM: u32 = 9001;
    const GANG_ROOM: u32 = 9002;
    const DEED_ROOM: u32 = 9003;
    const PLAIN_ROOM: u32 = 9004;
    const HOSPITAL: ShopNo = ShopNo(4);
    const GANG_SHOP: ShopNo = ShopNo(90);
    const DEED_SHOP: ShopNo = ShopNo(91);
    const QUARTERSTAFF: ItemNo = ItemNo(100);
    const RAPIER: ItemNo = ItemNo(369);
    /// A gang house deed (`Ability::GangDeed`) the test adds.
    const DEED: ItemNo = ItemNo(9100);

    /// The stock shops this file tests and the items they and the tests name: `shops`
    /// with each one's map 1 `room` and its `slots` (`[item, maxqty, qty, regentime,
    /// regennum, regenpct]`), and `items` with their abilities as `[number, value]` pairs and the classes and
    /// races they are limited to.
    fn fixture() -> Value {
        use super::stock_records::{by_number, ints, shop_room, with_ability_pairs};
        let mut items = std::collections::BTreeSet::from([QUARTERSTAFF.0 as i64, RAPIER.0 as i64]);
        let shops: Vec<Value> = [SWORD_SHOP, WEAPON_SHOP, ARMOUR_SHOP, GENERAL_STORE]
            .iter()
            .map(|n| {
                let r = by_number("WCCSHOPS", i64::from(n.0));
                let room = shop_room(i64::from(n.0));
                let cols = [
                    "items",
                    "maxqty",
                    "qty",
                    "regentime",
                    "regennum",
                    "regenpct",
                ]
                .map(|k| ints(r, k));
                let slots: Vec<Value> = (0..cols[0].len())
                    .map(|i| Value::from(cols.iter().map(|c| c[i]).collect::<Vec<_>>()))
                    .collect();
                items.extend(cols[0].iter().copied().filter(|&i| i != 0));
                serde_json::json!({
                    "number": r["number"], "name": r["name"], "type": r["type"],
                    "markup": r["markup"], "room": room["roomnum"], "slots": slots,
                })
            })
            .collect();
        let items: Vec<Value> = items
            .iter()
            .map(|&n| {
                let r = by_number("WCCITEMS", n);
                let mut item = with_ability_pairs(r);
                for k in ["classes", "races"] {
                    item[k] = ints(r, k).into_iter().filter(|&c| c != 0).collect();
                }
                item
            })
            .collect();
        serde_json::json!({ "shops": shops, "items": items })
    }

    fn int(v: &Value) -> i64 {
        v.as_i64().expect("an integer")
    }

    fn empty_shop(number: ShopNo, name: &str, shop_type: ShopType) -> Shop {
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
            trainclass: ClassNo(0),
            stock: [ShopSlot {
                item: ItemNo(0),
                maxqty: 0,
                qty: 0,
                regentime: 0,
                regennum: 0,
                regenpct: 0,
            }; NSHOPITEMS],
        }
    }

    fn shop_room(snap: &mut silvermere_engine::WorldSnapshot, room: u32, shop: ShopNo) {
        let mut r = blank::clear_room(RoomKey::new(1, room));
        r.name = format!("Room {room}");
        r.room_type = RoomType::Shop;
        r.shopnum = shop;
        snap.rooms.push(r);
    }

    /// The tiny world plus the fixture shops, a hospital, a gang shop, a deed shop and a
    /// plain room; class 1 may use every weapon and armour.
    fn shop_world() -> World {
        let fx = fixture();
        let mut snap = tiny_snapshot();
        snap.classes[0].weapon = WeaponClass::All;
        snap.classes[0].armour = ArmourType::Platemail;
        for it in fx["items"].as_array().unwrap() {
            let mut item = blank::clear_item(ItemNo(int(&it["number"]) as i32));
            item.name = it["name"].as_str().unwrap().into();
            item.cost = int(&it["cost"]) as u16;
            item.currency = Currency::from_code(int(&it["currency"]) as u16);
            item.limit = int(&it["limit"]) as u16;
            item.level = int(&it["level"]) as u16;
            item.item_type = ItemType::from_code(int(&it["type"]) as u16);
            item.weight = 10;
            // every fixture item can be picked up (`gettable`, as in the stock data)
            item.gettable = 1;
            item.wpntype = WeaponType::from_code(int(&it["wpntype"]) as u16);
            item.armtype = ArmourType::from_code(int(&it["armtype"]) as u16);
            item.worn = WornLocation::from_code(int(&it["worn"]) as u16);
            item.charges = int(&it["charges"]) as i16;
            for (slot, ab) in item
                .abilities
                .iter_mut()
                .zip(it["abilities"].as_array().unwrap())
            {
                *slot = (Ability::from_code(int(&ab[0]) as u16), int(&ab[1]) as i16);
            }
            snap.items.push(item);
        }
        let mut deed = blank::clear_item(DEED);
        deed.name = "gang house deed".into();
        deed.cost = 5;
        deed.currency = Currency::Gold;
        deed.abilities[0] = (Ability::GangDeed, 1);
        snap.items.push(deed);
        for s in fx["shops"].as_array().unwrap() {
            let number = ShopNo(int(&s["number"]) as i32);
            let mut shop = empty_shop(
                number,
                s["name"].as_str().unwrap(),
                ShopType::from_code(int(&s["type"]) as u16),
            );
            shop.markup = int(&s["markup"]) as u16;
            for (slot, row) in shop.stock.iter_mut().zip(s["slots"].as_array().unwrap()) {
                *slot = ShopSlot {
                    item: ItemNo(int(&row[0]) as i32),
                    maxqty: int(&row[1]) as u16,
                    qty: int(&row[2]) as u16,
                    regentime: int(&row[3]) as u16,
                    regennum: int(&row[4]) as i16,
                    regenpct: int(&row[5]) as u8,
                };
            }
            shop_room(&mut snap, int(&s["room"]) as u32, number);
            snap.shops.push(shop);
        }
        snap.shops
            .push(empty_shop(HOSPITAL, "Healer", ShopType::Hospital));
        shop_room(&mut snap, HOSPITAL_ROOM, HOSPITAL);
        let mut gang = empty_shop(GANG_SHOP, "Gang Shop", ShopType::GangShop);
        gang.markup = 50;
        // a listing of two quarterstaffs at 4 silver nobles each
        gang.stock[0] = ShopSlot {
            item: QUARTERSTAFF,
            maxqty: 0,
            qty: 2,
            regentime: 4,
            regennum: Currency::Silver.code() as i16,
            regenpct: 0,
        };
        snap.shops.push(gang);
        shop_room(&mut snap, GANG_ROOM, GANG_SHOP);
        let mut deeds = empty_shop(DEED_SHOP, "Deed Shop", ShopType::DeedShop);
        deeds.stock[0] = ShopSlot {
            item: DEED,
            maxqty: 1,
            qty: 1,
            regentime: 0,
            regennum: 0,
            regenpct: 0,
        };
        snap.shops.push(deeds);
        shop_room(&mut snap, DEED_ROOM, DEED_SHOP);
        let mut plain = blank::clear_room(RoomKey::new(1, PLAIN_ROOM));
        plain.name = "Plain".into();
        snap.rooms.push(plain);
        World::load(test_config(), 8, snap).expect("the shop world loads")
    }

    struct Harness {
        engine: Engine<TestClock, SeededRng>,
        clock: TestClock,
        rx: Receiver<EngineOutput>,
        log: Vec<EngineOutput>,
    }

    impl Harness {
        fn new() -> Self {
            let (tx, rx) = std::sync::mpsc::channel();
            let clock = TestClock::new();
            let engine = Engine::new(shop_world(), clock.clone(), SeededRng::new(7), tx);
            Self {
                engine,
                clock,
                rx,
                log: Vec::new(),
            }
        }

        /// A playing character in `room`, Charm 50, 48 hit points, room to carry.
        fn player(&mut self, name: &str, room: u32) -> SessionId {
            let w = &mut self.engine.world;
            let id = enter(w, name, room, MessageLevel::High);
            let c = w.session_mut(id).unwrap().character.as_mut().unwrap();
            (c.hp, c.maxhp) = (48, 48);
            c.cha = 50;
            // Strength 50 carries 2400 (secondary stats are worked out again on a drop)
            c.str = 50;
            c.maxwgt = 2400;
            c.movedir = -1;
            id
        }

        fn world(&mut self) -> &mut World {
            &mut self.engine.world
        }

        fn plr(&self, id: SessionId) -> &silvermere_types::records::Character {
            self.engine.world.session(id).unwrap().player().unwrap()
        }

        fn plr_mut(&mut self, id: SessionId) -> &mut silvermere_types::records::Character {
            self.engine
                .world
                .session_mut(id)
                .unwrap()
                .character
                .as_mut()
                .unwrap()
        }

        fn qty(&self, shop: ShopNo, slot: usize) -> u16 {
            self.engine.world.shops[&shop].stock[slot].qty
        }

        fn carried(&self, id: SessionId, item: ItemNo) -> usize {
            self.plr(id).items.iter().filter(|&&n| n == item).count()
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

        /// Types `text`; what `id` and `other` received, as bytes.
        fn both(&mut self, id: SessionId, text: &str, other: SessionId) -> (Vec<u8>, Vec<u8>) {
            let from = self.log.len();
            self.engine.handle(EngineInput::Line {
                id,
                text: text.into(),
            });
            self.pump();
            (self.written(from, id), self.written(from, other))
        }

        fn bytes(&mut self, id: SessionId, text: &str) -> Vec<u8> {
            self.both(id, text, id).0
        }

        /// Types `text`; what `id` received, ANSI stripped, `\r\n` as `\n`, prompt cut.
        fn line(&mut self, id: SessionId, text: &str) -> String {
            let b = self.bytes(id, text);
            let s = strip_ansi_str(&String::from_utf8_lossy(&b)).replace("\r\n", "\n");
            match s.rfind("[HP=") {
                Some(i) => s[..i].to_string(),
                None => s,
            }
        }

        fn stock_persists(&self) -> Vec<(ShopNo, [u16; NSHOPITEMS])> {
            self.log
                .iter()
                .filter_map(|o| match o {
                    EngineOutput::Persist(PersistRequest::ShopStock { shop, qty }) => {
                        Some((*shop, *qty))
                    }
                    _ => None,
                })
                .collect()
        }
    }

    const NORMAL: &[u8] = b"\x1b[0;37;40m";
    const CLRLINE: &[u8] = b"\x1b[79D\x1b[K";

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// The LIST screen of the T-005 `shop` capture (raw.bin), Newhaven Weapons Shop, after
    /// one quarterstaff had been sold that session.
    const CAPTURE_WEAPON_LIST: &[u8] = b"\x1b[0;37;40m\x1b[0;37;40m\x1b[79D\x1b[KThe following items are for sale here:\r\n\r\n\x1b[0;32mItem                          \x1b[36mQuantity    Price\r\n------------------------------------------------------\r\n\x1b[32mquarterstaff                  \x1b[36m30           Free\r\n\x1b[32mclub                          \x1b[36m26           Free\r\n";

    // The shop list against the capture, byte for byte up to the prompt
    #[test]
    fn list_weapon_shop_equals_the_capture() {
        let mut h = Harness::new();
        let a = h.player("Aldric", WEAPON_ROOM);
        h.world().shops.get_mut(&WEAPON_SHOP).unwrap().stock[0].qty = 30;
        let out = h.bytes(a, "list");
        assert!(
            out.starts_with(CAPTURE_WEAPON_LIST),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
    }

    // the armour shop and general store screens of the same capture (ANSI stripped)
    #[test]
    fn list_armour_shop_and_general_store_equal_the_capture() {
        let mut h = Harness::new();
        let a = h.player("Aldric", ARMOUR_ROOM);
        let rows = |items: &[&str]| {
            let mut s = String::from(
                "The following items are for sale here:\n\nItem                          Quantity    Price\n------------------------------------------------------\n",
            );
            for r in items {
                s.push_str(r);
                s.push('\n');
            }
            s
        };
        assert_eq!(
            h.line(a, "list"),
            rows(&[
                "padded vest                   35           Free",
                "padded pants                  35           Free",
                "padded helm                   35           Free",
                "padded gloves                 35           Free",
                "padded boots                  35           Free",
            ])
        );
        let s = h.player("Sasha", STORE_ROOM);
        assert_eq!(
            h.line(s, "list"),
            rows(&[
                "torch                         820          Free",
                "lantern                       63           2 gold crowns",
            ])
        );
    }

    // duplicate buys are allowed, as in the capture: the second `buy quarterstaff` is accepted
    // and its line
    // is byte-equal to the first
    #[test]
    fn buying_a_quarterstaff_twice() {
        let mut h = Harness::new();
        let a = h.player("Aldric", WEAPON_ROOM);
        let b = h.player("Betram", WEAPON_ROOM);
        let (first, seen) = h.both(a, "buy quarterstaff", b);
        let (second, _) = h.both(a, "buy quarterstaff", b);
        let bought = b"You just bought quarterstaff for nothing.\r\n";
        assert!(
            first.starts_with(bought),
            "{:?}",
            String::from_utf8_lossy(&first)
        );
        assert_eq!(first, second);
        // the room got what the output buffer held: the dispatcher's colour reset first
        let see = cat(&[NORMAL, CLRLINE, b"You see Aldric buy a quarterstaff.\r\n"]);
        assert!(
            seen.starts_with(&see),
            "{:?}",
            String::from_utf8_lossy(&seen)
        );
        assert_eq!(h.carried(a, QUARTERSTAFF), 2);
        assert_eq!(h.qty(WEAPON_SHOP, 0), 29);
        assert_eq!(h.stock_persists().last().unwrap().0, WEAPON_SHOP);
    }

    // the capture's `sell quarterstaff` and `appraise quarterstaff` (Charm 50: half of 0)
    #[test]
    fn selling_and_appraising() {
        let mut h = Harness::new();
        let a = h.player("Aldric", WEAPON_ROOM);
        let b = h.player("Betram", WEAPON_ROOM);
        h.bytes(a, "buy quarterstaff");
        h.bytes(a, "buy quarterstaff");
        let (sold, seen) = h.both(a, "sell quarterstaff", b);
        assert!(
            sold.starts_with(b"You sold quarterstaff for 0 copper farthings.\r\n"),
            "{:?}",
            String::from_utf8_lossy(&sold)
        );
        let see = cat(&[CLRLINE, b"You see Aldric sell a quarterstaff.\r\n"]);
        assert!(
            seen.starts_with(&see),
            "{:?}",
            String::from_utf8_lossy(&seen)
        );
        // back on the shelf: 31 - 2 + 1
        assert_eq!(h.qty(WEAPON_SHOP, 0), 30);
        let appraised = h.bytes(a, "appraise quarterstaff");
        assert!(
            appraised.starts_with(b"You would get 0 copper farthings for your quarterstaff.\r\n"),
            "{:?}",
            String::from_utf8_lossy(&appraised)
        );
        assert_eq!(h.qty(WEAPON_SHOP, 0), 30);
        assert_eq!(h.carried(a, QUARTERSTAFF), 1);
    }

    // the general store part of the capture: what it cannot afford, what it does not sell
    #[test]
    fn general_store_refusals_equal_the_capture() {
        let mut h = Harness::new();
        let a = h.player("Aldric", STORE_ROOM);
        let poor = h.bytes(a, "buy lantern");
        assert!(poor.starts_with(&cat(&[NORMAL, b"You cannot afford lantern.\r\n"])));
        let unknown = h.bytes(a, "buy quarterstaff");
        assert!(unknown.starts_with(&cat(&[NORMAL, b"quarterstaff is not a known item.\r\n"])));
        assert_eq!(h.qty(GENERAL_STORE, 1), 63);
    }

    // Buy and sell prices through the commands: the Sword Shop's markup 100
    #[test]
    fn prices_follow_markup_and_charm() {
        let mut h = Harness::new();
        let a = h.player("Aldric", SWORD_ROOM);
        h.plr_mut(a).coins = Coins {
            gold: 30,
            ..Coins::default()
        };
        // 12 gold crowns, doubled by the markup, at Charm 50
        assert_eq!(
            h.line(a, "buy longsword"),
            "You just bought longsword for 24 gold crowns.\n"
        );
        assert_eq!(h.plr(a).coins.gold, 6);
        // Charm 30 pays 104%: 2400 * 104 / 100 = 2496 copper
        h.plr_mut(a).cha = 30;
        assert_eq!(h.line(a, "buy longsword"), "You cannot afford longsword.\n");
        h.plr_mut(a).coins = Coins {
            gold: 25,
            ..Coins::default()
        };
        assert_eq!(
            h.line(a, "buy longsword"),
            "You just bought longsword for 24 gold crowns, 9 silver nobles, 6 copper farthings.\n"
        );
        // selling: (Charm/2 + 25)% of 1200 copper at Charm 30 is 480
        assert_eq!(
            h.line(a, "appraise longsword"),
            "You would get 480 copper farthings for your longsword.\n"
        );
        assert_eq!(
            h.line(a, "sell longsword"),
            "You sold longsword for 480 copper farthings.\n"
        );
        // paid in copper, then changed up (the change-up into larger coins)
        assert_eq!(h.plr(a).coins.gold, 4);
        assert_eq!(h.plr(a).coins.silver, 8);
        assert_eq!(h.plr(a).coins.copper, 4);
    }

    // stock 0 is "cannot buy here"; a shop that does not list an item refuses to buy it
    #[test]
    fn out_of_stock_and_not_listed() {
        let mut h = Harness::new();
        let a = h.player("Aldric", SWORD_ROOM);
        assert_eq!(
            h.line(a, "buy serrated scimitar"),
            "You cannot buy serrated scimitar here!\n"
        );
        h.plr_mut(a).items[0] = ItemNo(90); // a club
        assert_eq!(h.line(a, "sell club"), "You cannot sell club here.\n");
        assert_eq!(h.line(a, "sell spoon"), "You don't have spoon to sell!\n");
        h.plr_mut(a).items[0] = ItemNo(0);
    }

    // Several fitting names give the list, the second first
    #[test]
    fn several_names_give_the_list() {
        let mut h = Harness::new();
        let a = h.player("Aldric", SWORD_ROOM);
        // a word of the name must start with what was typed (matches_any_word)
        let out = h.line(a, "buy s");
        assert_eq!(
            out,
            "Please be more specific.  You could have meant any of these:\n-- scimitar\n-- stiletto\n-- bastard sword\n-- shortsword\n-- serrated scimitar\n-- khopesh sword\n-- sickle\n"
        );
    }

    // The refusals outside a shop, syntax, and LIST with an argument
    #[test]
    fn refusals_outside_a_shop() {
        let mut h = Harness::new();
        let a = h.player("Aldric", PLAIN_ROOM);
        let list = h.bytes(a, "list");
        assert!(list.starts_with(&cat(&[
            NORMAL,
            b"\x1b[1;31;40mYou cannot LIST if you are not in a shop!\r\n"
        ])));
        assert_eq!(
            h.line(a, "buy torch"),
            "You cannot BUY if you are not in a shop!\n"
        );
        assert_eq!(
            h.line(a, "sell torch"),
            "You cannot SELL if you are not in a shop!\n"
        );
        assert_eq!(h.line(a, "buy"), "Syntax: BUY {item}\n");
        assert_eq!(h.line(a, "sell"), "Syntax: SELL {item}\n");
        assert_eq!(
            h.line(a, "buy room"),
            "If you are a gang leader you may lease a Gang House.\n"
        );
    }

    // Resting ends first, then LIST with a word is no command
    #[test]
    fn list_ends_resting_and_takes_no_argument() {
        use silvermere_types::flags::{PlayerStatus, PlayerStatus2};
        let mut h = Harness::new();
        let a = h.player("Aldric", WEAPON_ROOM);
        h.plr_mut(a).status.insert(PlayerStatus::RESTING);
        h.plr_mut(a).status2.insert(PlayerStatus2::MEDITATING);
        let out = h.line(a, "list quarterstaff");
        assert!(!out.contains("for sale"), "{out:?}");
        assert!(!h.plr(a).status.contains(PlayerStatus::RESTING));
        assert!(!h.plr(a).status2.contains(PlayerStatus2::MEDITATING));
    }

    // A scripted wound, then healing and curing at the hospital
    #[test]
    fn hospital_heals_and_cures() {
        let mut h = Harness::new();
        let a = h.player("Aldric", HOSPITAL_ROOM);
        // nothing in stock: LIST shows nothing at all (the capture's healer)
        assert_eq!(h.line(a, "list"), "");
        h.plr_mut(a).hp = 30;
        assert_eq!(
            h.line(a, "buy healing"),
            "You do not have sufficient funds to buy full healing!\n"
        );
        h.plr_mut(a).coins = Coins {
            silver: 25,
            ..Coins::default()
        };
        let healed = h.bytes(a, "buy heal");
        assert!(healed.starts_with(&cat(&[
            NORMAL,
            CLRLINE,
            b"\x1b[1;34mYou hand over 3 silver nobles, 6 copper farthings and all your wounds are healed.\r\n"
        ])), "{:?}", String::from_utf8_lossy(&healed));
        assert_eq!(h.plr(a).hp, 48);
        // 25 silver less 36 copper, then 15 more although he was not poisoned
        assert_eq!(
            h.line(a, "buy curing"),
            "You hand over 15 silver nobles and find that you were not poisoned!\n"
        );
        h.plr_mut(a).poison = 4;
        assert_eq!(
            h.line(a, "buy cure poison"),
            "You do not have sufficient funds to buy curing!\n"
        );
        h.plr_mut(a).coins.silver += 25;
        assert_eq!(
            h.line(a, "buy cure poison"),
            "You hand over 25 silver nobles and your poisoning is cured.\n"
        );
        assert_eq!(h.plr(a).poison, 0);
    }

    // gang shops: the gang's price in its
    // currency with the markup (4 silver + 50%) and nothing for Charm 50, ten slots, no selling,
    // cleared when sold out
    #[test]
    fn gang_shop_listing() {
        let mut h = Harness::new();
        let a = h.player("Aldric", GANG_ROOM);
        h.plr_mut(a).coins = Coins {
            silver: 12,
            ..Coins::default()
        };
        assert_eq!(
            h.line(a, "list"),
            "The following items are for sale here:\n\nItem                          Quantity    Price\n------------------------------------------------------\nquarterstaff                  2            6 silver nobles\n"
        );
        assert_eq!(
            h.line(a, "buy quarterstaff"),
            "You just bought quarterstaff for 6 silver nobles.\n"
        );
        assert_eq!(
            h.line(a, "sell quarterstaff"),
            "You may not sell items to a gang shop.\n"
        );
        assert_eq!(
            h.line(a, "buy quarterstaff"),
            "You just bought quarterstaff for 6 silver nobles.\n"
        );
        let slot = h.engine.world.shops[&GANG_SHOP].stock[0];
        assert_eq!(
            (slot.item, slot.qty, slot.regentime, slot.regennum),
            (ItemNo(0), 0, 0, 5)
        );
        assert_eq!(
            h.line(a, "buy quarterstaff"),
            "quarterstaff is not a known item.\n"
        );
    }

    // the deed shop's refusals in their order
    #[test]
    fn deed_shop_refusals() {
        let mut h = Harness::new();
        let a = h.player("Aldric", DEED_ROOM);
        assert_eq!(
            h.line(a, "buy deed"),
            "You must be a gang leader to purchase a gang house deed.\n"
        );
        h.world().gangs.insert(
            "WOLVES".into(),
            Gang {
                key: "WOLVES".into(),
                name: "Wolves".into(),
                exp: 10,
                leader: "Aldric".into(),
                created: 0,
                members: 1,
                flags: silvermere_types::flags::GangFlags::empty(),
                overexp: 0,
                overflows: 0,
            },
        );
        h.plr_mut(a).gang = "Wolves".into();
        h.world().config.gang_exp = 1;
        assert_eq!(
            h.line(a, "buy deed"),
            "Your gang does not have enough experience for you to purchase a gang house now.\n"
        );
        h.world().gangs.get_mut("WOLVES").unwrap().exp = 10_000;
        h.plr_mut(a).flags.insert(PlayerFlags::SOLDDEED);
        assert_eq!(
            h.line(a, "buy deed"),
            "Due to outstanding paper-work we are unable to provide you with another\nproperty today. Please call back tomorrow!\n"
        );
        h.plr_mut(a).flags.remove(PlayerFlags::SOLDDEED);
        h.plr_mut(a).coins.gold = 5;
        assert_eq!(
            h.line(a, "buy deed"),
            "You just bought gang house deed for 5 gold crowns.\n"
        );
        // (sic) the owner of a deed is told his gang lacks experience
        h.world().shops.get_mut(&DEED_SHOP).unwrap().stock[0].qty = 1;
        assert_eq!(
            h.line(a, "buy deed"),
            "Your gang does not have enough experience for you to purchase a gang house now.\n"
        );
        // selling the deed back marks him (`PlayerFlags::SOLDDEED`); so does APPRAISE (sic)
        h.world().shops.get_mut(&DEED_SHOP).unwrap().stock[0].qty = 0;
        h.line(a, "appraise deed");
        assert!(h.plr(a).flags.contains(PlayerFlags::SOLDDEED));
    }

    // LOOK at goods (display_item_in_shop): an item in
    // stock is described, a sold-out one is not there, several names give the list
    #[test]
    fn looking_at_goods() {
        let mut h = Harness::new();
        let a = h.player("Aldric", SWORD_ROOM);
        let seen = h.line(a, "look longsword");
        assert!(!seen.contains("You do not see"), "{seen:?}");
        assert_eq!(
            h.line(a, "look serrated"),
            "You do not see serrated here!\n"
        );
        // ("look s" would be LOOK SOUTH)
        assert_eq!(
            h.line(a, "look sc"),
            "Please be more specific.  You could have meant any of these:\n-- serrated scimitar\n-- scimitar\n"
        );
    }

    /// A script of rolls, each clamped into the range asked for.
    struct Script(Vec<i32>, usize);

    impl Rng for Script {
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            let v = self.0[self.1 % self.0.len()];
            self.1 += 1;
            v.clamp(lo, (hi - 1).max(lo))
        }

        fn long(&mut self, lo: i64, _hi: i64) -> i64 {
            lo
        }
    }

    // By hand: the stock after one restock_items of the three shops'
    // due slots, with the rolls in queue order (the list's head first)
    #[test]
    fn restock_equals_the_hand_computed_table() {
        let mut h = Harness::new();
        let w = h.world();
        w.restock.clear();
        let set = |w: &mut World, shop: ShopNo, slot: usize, qty: u16| {
            w.shops.get_mut(&shop).unwrap().stock[slot].qty = qty;
        };
        set(w, GENERAL_STORE, 0, 800); // torch 800/820, +10 at 100%
        set(w, GENERAL_STORE, 1, 60); // lantern 60/63, +2 at 100%
        set(w, SWORD_SHOP, 0, 30); // longsword 30/32, +2 at 100%
        set(w, SWORD_SHOP, 4, 10); // knife 10/15, +1 at 35%
        set(w, SWORD_SHOP, 11, 9); // fine broadsword 9/10, +1 at 25%
        set(w, SWORD_SHOP, 18, 1); // sickle 1/5, +1 at 33%
        set(w, WEAPON_SHOP, 0, 5); // quarterstaff 5/31, +10 at 100%
        set(w, WEAPON_SHOP, 1, 20); // club 20/26, +8 at 100%
        for shop in [GENERAL_STORE, SWORD_SHOP, WEAPON_SHOP] {
            shops::check_initiate_restocking(w, shop, &mut Script(vec![50], 0));
        }
        // 2 + 13 + 2 timed slots; the untimed Sword Shop slots rolled 50, never below 0
        assert_eq!(w.restock.len(), 17);
        assert_eq!(w.shops[&SWORD_SHOP].stock[13].qty, 0);
        // every first restock falls due
        for n in &mut w.restock {
            n.due = 0;
        }
        // rolls are drawn only for slots below their ceiling, head first: club,
        // quarterstaff, sickle, fine broadsword, knife, longsword, lantern, torch
        let mut rolls = Script(vec![10, 99, 40, 30, 34, 1, 99, 50], 0);
        shops::restock_items(w, &mut rolls);
        assert_eq!(rolls.1, 8);
        let table: [(ShopNo, usize, u16); 12] = [
            (WEAPON_SHOP, 1, 26),   // 20 + 8, cut to 26
            (WEAPON_SHOP, 0, 15),   // 5 + 10
            (SWORD_SHOP, 18, 1),    // 40 is not below 33
            (SWORD_SHOP, 11, 9),    // 30 is not below 25
            (SWORD_SHOP, 10, 24),   // full: no roll
            (SWORD_SHOP, 4, 11),    // 34 is below 35
            (SWORD_SHOP, 0, 32),    // 30 + 2
            (SWORD_SHOP, 12, 0),    // the recycler slot is never restocked
            (SWORD_SHOP, 13, 0),    // untimed, regennum 0
            (GENERAL_STORE, 1, 62), // 60 + 2
            (GENERAL_STORE, 0, 810),
            (SWORD_SHOP, 2, 30), // full
        ];
        for (shop, slot, qty) in table {
            assert_eq!(
                w.shops[&shop].stock[slot].qty, qty,
                "shop {} slot {slot}",
                shop.0
            );
        }
        // each node is next due one full interval later, success or not
        let node = |shop: ShopNo, slot: usize| {
            *w.restock
                .iter()
                .find(|n| n.shop == shop && n.slot == slot)
                .unwrap()
        };
        assert_eq!(
            node(SWORD_SHOP, 11),
            RestockNode {
                shop: SWORD_SHOP,
                slot: 11,
                due: 720 * 60
            }
        );
        assert_eq!(node(WEAPON_SHOP, 0).due, 24 * 60);
        // a second run now finds nothing due
        let mut none = Script(vec![1], 0);
        shops::restock_items(w, &mut none);
        assert_eq!(none.1, 0);
    }

    // An untimed slot is topped up at load (or cut to its ceiling); a
    // timed slot's first restock is 1..regentime minutes away; gang shops are skipped
    #[test]
    fn initiating_tops_up_untimed_slots_and_queues_timed_ones() {
        let mut h = Harness::new();
        let w = h.world();
        w.restock.clear();
        {
            let s = &mut w.shops.get_mut(&SWORD_SHOP).unwrap().stock;
            s[13].regennum = 2; // serrated scimitar 0/3, +2 at 50%
            s[13].regenpct = 50;
            s[14].qty = 9; // khopesh 9/5: cut back
            s[0].qty = 40; // longsword 40/32, timed: cut back too
        }
        shops::check_initiate_restocking(w, SWORD_SHOP, &mut Script(vec![49, 1000], 0));
        let s = w.shops[&SWORD_SHOP].stock;
        assert_eq!((s[13].qty, s[14].qty, s[0].qty), (2, 5, 32));
        assert!(w.restock.iter().all(|n| n.shop == SWORD_SHOP));
        // regentime 240: the roll is at most 239 minutes
        let longsword = w.restock.iter().find(|n| n.slot == 0).unwrap();
        assert!(longsword.due <= 239 * 60, "{longsword:?}");
        let before = w.restock.len();
        shops::check_initiate_restocking(w, GANG_SHOP, &mut Script(vec![1], 0));
        assert_eq!(w.restock.len(), before);
    }

    // the scheduler: restock_items runs on the 21st slow cycle
    #[test]
    fn restock_after_21_slow_cycles() {
        let mut h = Harness::new();
        h.world().shops.get_mut(&ARMOUR_SHOP).unwrap().stock[0].qty = 3;
        // padded vest: regentime 2, so due after one minute (a roll in [1, 2) is 1)
        assert!(
            h.engine
                .world
                .restock
                .iter()
                .any(|n| n.shop == ARMOUR_SHOP && n.slot == 0 && n.due == 60)
        );
        let clock = h.clock.clone();
        clock.advance_ticks(20 * 30 + 10, &mut h.engine);
        assert_eq!(h.qty(ARMOUR_SHOP, 0), 3);
        clock.advance_ticks(30, &mut h.engine);
        assert_eq!(h.qty(ARMOUR_SHOP, 0), 13);
        h.pump();
        assert!(
            h.stock_persists()
                .iter()
                .any(|(s, q)| *s == ARMOUR_SHOP && q[0] == 13)
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum Op {
        Buy(bool),
        Drop(bool),
        Get(bool),
        Give(bool),
        Sell(bool),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            any::<bool>().prop_map(Op::Buy),
            any::<bool>().prop_map(Op::Drop),
            any::<bool>().prop_map(Op::Get),
            any::<bool>().prop_map(Op::Give),
            any::<bool>().prop_map(Op::Sell),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        // item_ownership never holds more copies of a limited item than its limit, and
        // its rows are the item's count in play (`active`), whatever two players do
        #[test]
        fn limited_copies_never_exceed_the_limit(ops in proptest::collection::vec(op(), 1..40)) {
            let mut h = Harness::new();
            {
                // the emerald-hilted rapier (limit 3) on a shelf of 2, ceiling 3
                let s = &mut h.world().shops.get_mut(&SWORD_SHOP).unwrap().stock[16];
                s.maxqty = 3;
                s.qty = 2;
            }
            let a = h.player("Ann", SWORD_ROOM);
            let b = h.player("Bob", SWORD_ROOM);
            for c in [a, b] {
                h.plr_mut(c).coins.platinum = 100;
            }
            for op in ops {
                let who = |first: bool| if first { (a, "Bob") } else { (b, "Ann") };
                let line = match op {
                    Op::Buy(f) => { let (w, _) = who(f); (w, "buy emerald-hilted rapier".to_string()) }
                    Op::Drop(f) => { let (w, _) = who(f); (w, "drop rapier".to_string()) }
                    Op::Get(f) => { let (w, _) = who(f); (w, "get rapier".to_string()) }
                    Op::Give(f) => { let (w, o) = who(f); (w, format!("give rapier to {o}")) }
                    Op::Sell(f) => { let (w, _) = who(f); (w, "sell rapier".to_string()) }
                };
                h.bytes(line.0, &line.1);
                let w = &h.engine.world;
                let rows = shops_rows(w);
                let limit = usize::from(w.items[&RAPIER].limit);
                prop_assert!(rows <= limit, "{rows} copies after {op:?}");
                prop_assert_eq!(rows, usize::from(w.items[&RAPIER].active));
            }
        }
    }

    fn shops_rows(w: &World) -> usize {
        silvermere_engine::items::limited::holders_of(w, RAPIER).len()
    }

    // the operations the property test draws, one by one, with the ledger after each
    #[test]
    fn a_limited_copy_through_buy_drop_get_give_and_sell() {
        let mut h = Harness::new();
        {
            let s = &mut h.world().shops.get_mut(&SWORD_SHOP).unwrap().stock[16];
            s.maxqty = 3;
            s.qty = 2;
        }
        let a = h.player("Ann", SWORD_ROOM);
        let b = h.player("Bob", SWORD_ROOM);
        h.plr_mut(a).coins.platinum = 10;
        let kinds = |h: &Harness| {
            silvermere_engine::items::limited::holders_of(&h.engine.world, RAPIER)
                .iter()
                .map(|o| o.owntype.code())
                .collect::<Vec<_>>()
        };
        let (user, room) = (
            silvermere_types::enums::OwnerKind::User.code(),
            silvermere_types::enums::OwnerKind::Room.code(),
        );
        h.line(a, "buy emerald-hilted rapier");
        assert_eq!(kinds(&h), [user]);
        let dropped = h.line(a, "drop rapier");
        assert_eq!(dropped, "You dropped emerald-hilted rapier.\n");
        assert_eq!(kinds(&h), [room]);
        let got = h.line(a, "get rapier");
        assert_eq!(kinds(&h), [user], "{got:?}");
        h.line(a, "give rapier to Bob");
        assert_eq!(h.carried(b, RAPIER), 1);
        assert_eq!(kinds(&h), [user]);
        assert_eq!(
            h.line(b, "sell rapier"),
            "You sold emerald-hilted rapier for 5000 copper farthings.\n"
        );
        // back on the shelf: an ownership record of the shop's room, marked as the shelf
        let shelf: Vec<_> = silvermere_engine::items::limited::holders_of(&h.engine.world, RAPIER)
            .iter()
            .map(|o| (o.owntype.code(), o.roomnum, o.userid.clone()))
            .collect();
        assert_eq!(
            shelf,
            [(
                room,
                silvermere_types::ids::RoomId(SWORD_ROOM),
                silvermere_engine::items::limited::SHELF_MARK.to_string()
            )]
        );
        assert_eq!(h.engine.world.items[&RAPIER].active, 1);
    }

    // the collision the property test found: a copy dropped on a shop's floor is not a
    // shelf copy, so buying from the shelf leaves the floor record alone
    #[test]
    fn a_floor_copy_in_a_shop_is_not_a_shelf_copy() {
        let mut h = Harness::new();
        {
            let s = &mut h.world().shops.get_mut(&SWORD_SHOP).unwrap().stock[16];
            s.maxqty = 3;
            s.qty = 2;
        }
        let a = h.player("Ann", SWORD_ROOM);
        let b = h.player("Bob", SWORD_ROOM);
        for c in [a, b] {
            h.plr_mut(c).coins.platinum = 10;
        }
        h.line(a, "buy emerald-hilted rapier");
        h.line(a, "sell rapier");
        h.line(a, "buy emerald-hilted rapier"); // the shelf record: count unchanged
        assert_eq!(h.engine.world.items[&RAPIER].active, 1);
        h.line(a, "drop rapier");
        h.line(b, "buy emerald-hilted rapier"); // unrecorded stock: a new copy
        assert_eq!(h.engine.world.items[&RAPIER].active, 2);
        h.line(b, "get rapier");
        let w = &h.engine.world;
        assert_eq!(shops_rows(w), 2);
        assert_eq!(w.items[&RAPIER].active, 2);
        assert_eq!(h.carried(b, RAPIER), 2);
    }

    // reason 1: only stock no one sold here (unrecorded) needs a copy
    // free under the limit; a recorded shelf copy is bought whatever the count, and goes
    // back on the shelf when the buyer cannot pay
    #[test]
    fn a_limited_item_needs_a_free_copy() {
        let mut h = Harness::new();
        {
            let s = &mut h.world().shops.get_mut(&SWORD_SHOP).unwrap().stock[16];
            s.maxqty = 5;
            s.qty = 5;
        }
        let a = h.player("Ann", SWORD_ROOM);
        h.plr_mut(a).coins.platinum = 100;
        for _ in 0..3 {
            assert!(
                h.line(a, "buy emerald-hilted rapier")
                    .starts_with("You just bought")
            );
        }
        assert_eq!(h.engine.world.items[&RAPIER].active, 3);
        // all three copies are out and none is on the shelf: refused, one copy less
        assert_eq!(
            h.line(a, "buy emerald-hilted rapier"),
            "You cannot buy emerald-hilted rapier here!\n"
        );
        assert_eq!(h.qty(SWORD_SHOP, 16), 1);
        // sold back: the copy is recorded on the shelf and still counts
        h.line(a, "sell rapier");
        assert_eq!(h.qty(SWORD_SHOP, 16), 2);
        assert_eq!(h.engine.world.items[&RAPIER].active, 3);
        assert_eq!(shops_rows(&h.engine.world), 3);
        // a buyer who cannot pay puts the shelf record back
        let coins = h.plr(a).coins;
        h.plr_mut(a).coins = Coins::default();
        assert_eq!(
            h.line(a, "buy emerald-hilted rapier"),
            "You cannot afford emerald-hilted rapier.\n"
        );
        assert_eq!(h.engine.world.items[&RAPIER].active, 3);
        assert_eq!(shops_rows(&h.engine.world), 3);
        h.plr_mut(a).coins = coins;
        // the shelf copy is bought although all three are in play: the count nets out
        assert!(
            h.line(a, "buy emerald-hilted rapier")
                .starts_with("You just bought")
        );
        assert_eq!(h.engine.world.items[&RAPIER].active, 3);
        assert_eq!(shops_rows(&h.engine.world), 3);
        assert_eq!(h.qty(SWORD_SHOP, 16), 1);
    }
}
