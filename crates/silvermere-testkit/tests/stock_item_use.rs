//! Item use (T-053) through the engine: USE and READ (`cmd_use`), EAT and
//! DRINK, OPEN {item}, the five `use_*_target`
//! functions and the Auto Use hook of a player's blow.
//!
//! The stock records come from the downloaded distribution ([`items::use_item`]'s
//! `fixture`): the scroll of vine strike and the Newhaven Spell Shop that sells it, the
//! minor healing potion, the torch, the bone key and the Key exit it fits (room 1309,
//! north), the iron ration, the waterskin, the obsidian claymore (Auto Use 100, One Time
//! Cast 172 dark fire), their spells and messages, the Druid and Witchunter classes. The
//! rest is made up for the cases the stock data has no item for: Heal, Heal Mana, a
//! container, a wrong key.
//!
//! Every test sits in `mod items::use_item`, so `cargo test -p silvermere-testkit
//! items::use_item` runs them.

use crate::engine_common as common;

mod items {
    pub mod use_item {
        use proptest::prelude::*;
        use serde_json::Value;

        use super::super::common::{enter, test_config, tiny_snapshot};
        use silvermere_engine::combat::Mode;
        use silvermere_engine::combat::attack_user_monster;
        use silvermere_engine::commands::input::execute_input;
        use silvermere_engine::items::inventory::{
            Charge, add_item_to_inventory, deduct_item_charge,
        };
        use silvermere_engine::items::use_item::{UseTarget, use_item};
        use silvermere_engine::world::{World, blank};
        use silvermere_engine::{SeededRng, SessionId};
        use silvermere_rules::combat::Move;
        use silvermere_text::strip_ansi;
        use silvermere_types::enums::{
            Ability, Alignment, ArmourType, Currency, Element, ExitType, GroupRole, ItemType,
            Magery, MessageLevel, MonsterType, ResistMode, ResistStat, RoomType, ShopType,
            SpellTarget, SpellType, WeaponClass, WeaponType, WornLocation,
        };
        use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings, SpellFlags};
        use silvermere_types::ids::{
            ActiveMonsterId, ClassNo, ItemNo, MapId, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey,
            ShopNo, SpellNo,
        };
        use silvermere_types::records::{
            ActiveMonster, ActiveSpell, Character, Class, Coins, Exit, Item, Message, ShopSlot,
            Spell,
        };
        use silvermere_types::sizes::NSHOPITEMS;

        // Stock numbers.
        const SPELL_SHOP: u32 = 2144;
        const TOMB: u32 = 1309;
        const SCROLL: ItemNo = ItemNo(142);
        const TORCH: ItemNo = ItemNo(175);
        const BONE_KEY: ItemNo = ItemNo(177);
        const POTION: ItemNo = ItemNo(223);
        const RATION: ItemNo = ItemNo(282);
        const WATERSKIN: ItemNo = ItemNo(283);
        const CLAYMORE: ItemNo = ItemNo(541);
        const VINE: SpellNo = SpellNo(25);
        const DARK_FIRE: SpellNo = SpellNo(172);
        const WITCHUNTER: ClassNo = ClassNo(2);
        const DRUID: ClassNo = ClassNo(13);
        // Made up.
        const ROOM: u32 = 3000;
        const ELIXIR: ItemNo = ItemNo(900);
        const MANA_DRAUGHT: ItemNo = ItemNo(901);
        const CHEST: ItemNo = ItemNo(902);
        const IRON_KEY: ItemNo = ItemNo(903);
        const WAND: ItemNo = ItemNo(904);
        const RING: ItemNo = ItemNo(905);
        const SWORD: ItemNo = ItemNo(906);
        const RAT: MonsterNo = MonsterNo(1);

        /// The stock records the tests read, by kind, each as `--json` writes it; items'
        /// and spells' abilities as `[number, value]` pairs.
        fn fixture() -> Value {
            use crate::stock_records::{by_number, room, with_ability_pairs};
            let rows = |table: &str, ns: &[i64]| -> Vec<Value> {
                ns.iter()
                    .map(|&n| Value::Object(by_number(table, n).clone()))
                    .collect()
            };
            let paired = |table: &str, ns: &[i64]| -> Vec<Value> {
                ns.iter()
                    .map(|&n| with_ability_pairs(by_number(table, n)))
                    .collect()
            };
            let rooms: Vec<Value> = [1309, 1377, 2144]
                .iter()
                .map(|&n| Value::Object(room(n).clone()))
                .collect();
            serde_json::json!({
                "items": paired("WCCITEMS", &[142, 175, 177, 223, 282, 283, 541]),
                "spells": paired("WCCSPELS", &[25, 50, 172, 316, 317]),
                "shops": rows("WCCSHOPS", &[48]),
                "rooms": rooms,
                "classes": rows("WCCCLASS", &[2, 13]),
                "messages": rows("WCCMSG", &[14, 8376, 8420, 8603, 8610, 139, 645, 1398, 2014]),
            })
        }

        fn int(v: &Value) -> i64 {
            v.as_i64().unwrap()
        }

        fn abilities<const N: usize>(v: &Value) -> [(Ability, i16); N] {
            let mut a: [(Ability, i16); N] = std::array::from_fn(|_| (Ability::None, 0));
            for (slot, ab) in a.iter_mut().zip(v.as_array().unwrap()) {
                *slot = (Ability::from_code(int(&ab[0]) as u16), int(&ab[1]) as i16);
            }
            a
        }

        fn item(v: &Value) -> Item {
            let mut i = blank::clear_item(ItemNo(int(&v["number"]) as i32));
            i.name = v["name"].as_str().unwrap().into();
            i.item_type = ItemType::from_code(int(&v["type"]) as u16);
            i.worn = WornLocation::from_code(int(&v["worn"]) as u16);
            i.charges = int(&v["charges"]) as i16;
            i.retain = int(&v["retain"]) as u8;
            i.destroymsg = MessageNo(int(&v["destroymsg"]) as i32);
            i.weight = int(&v["weight"]) as i16;
            i.cost = int(&v["cost"]) as u16;
            i.currency = Currency::from_code(int(&v["currency"]) as u16);
            i.level = int(&v["level"]) as u16;
            i.limit = int(&v["limit"]) as u16;
            i.gettable = int(&v["gettable"]) as i16;
            i.wpntype = WeaponType::from_code(int(&v["wpntype"]) as u16);
            i.armtype = ArmourType::from_code(int(&v["armtype"]) as u16);
            i.mindmg = int(&v["mindmg"]) as i16;
            i.maxdmg = int(&v["maxdmg"]) as i16;
            i.speed = int(&v["speed"]) as i16;
            i.strreq = int(&v["strreq"]) as i16;
            i.accuracy = int(&v["accuracy"]) as i16;
            i.abilities = abilities(&v["abilities"]);
            i
        }

        fn spell(v: &Value) -> Spell {
            Spell {
                number: SpellNo(int(&v["number"]) as i16),
                name: v["name"].as_str().unwrap().into(),
                descs: [String::new(), String::new()],
                resist: int(&v["resist"]) as i32,
                levelcap: int(&v["levelcap"]) as i16,
                flags: SpellFlags::from_bits_retain(int(&v["flags"]) as u8),
                abilities: abilities(&v["abilities"]),
                energy: int(&v["energy"]) as u16,
                level: int(&v["level"]) as u16,
                minval: int(&v["minval"]) as i16,
                maxval: int(&v["maxval"]) as i16,
                spell_type: SpellType::from_code(int(&v["type"]) as u16),
                resmode: ResistMode::from_code(int(&v["resmode"]) as u16),
                diff: int(&v["diff"]) as i16,
                durlvl: 0,
                target: SpellTarget::from_code(int(&v["target"]) as u16),
                duration: int(&v["duration"]) as i16,
                element: Element::from_code(int(&v["element"]) as u16),
                resstat: ResistStat::from_code(0),
                magery: Magery::from_code(int(&v["magery"]) as u16),
                msgnum: MessageNo(int(&v["msgnum"]) as i32),
                mana: int(&v["mana"]) as i16,
                maxinc: int(&v["maxinc"]) as u8,
                maxlvls: int(&v["maxlvls"]) as u8,
                magelvl: int(&v["magelvl"]) as i16,
                mininc: 0,
                minlvls: 0,
                durinc: 0,
                durlvls: int(&v["durlvls"]) as u8,
                shortnm: v["shortnm"].as_str().unwrap().into(),
            }
        }

        fn class(v: &Value) -> Class {
            let mut c = blank::clear_class(ClassNo(int(&v["number"]) as u16));
            c.name = v["name"].as_str().unwrap().into();
            let codes = v["abilities"].as_array().unwrap();
            let vals = v["abilvals"].as_array().unwrap();
            for (i, slot) in c.abilities.iter_mut().enumerate() {
                *slot = (
                    Ability::from_code(int(&codes[i]) as u16),
                    int(&vals[i]) as i16,
                );
            }
            c.magery = Magery::from_code(int(&v["magery"]) as u16);
            c.magelvl = int(&v["magelvl"]) as i16;
            c.weapon = WeaponClass::from_code(int(&v["weapon"]) as u16);
            c.armour = ArmourType::from_code(int(&v["armour"]) as u16);
            c.combat = int(&v["combat"]) as i16;
            c.basehp = int(&v["basehp"]) as i16;
            c.hplevel = int(&v["hplevel"]) as i16;
            c
        }

        fn room(v: &Value) -> silvermere_types::records::Room {
            let n = int(&v["roomnum"]) as u32;
            let mut r = blank::clear_room(RoomKey::new(1, n));
            r.name = v["name"].as_str().unwrap().into();
            r.room_type = RoomType::from_code(int(&v["type"]) as u16);
            r.shopnum = ShopNo(int(&v["shopnum"]) as i32);
            for (d, e) in r.exits.iter_mut().enumerate() {
                *e = Exit {
                    target: RoomId(int(&v["exits"][d]) as u32),
                    kind: ExitType::from_code(int(&v["exittypes"][d]) as u16),
                    par1: int(&v["exitpar1s"][d]) as u32,
                    par2: int(&v["exitpar2s"][d]) as i16,
                    par3: int(&v["exitpar3s"][d]) as i32,
                    par4: int(&v["exitpar4s"][d]) as i32,
                };
            }
            r
        }

        fn made_up(
            n: ItemNo,
            name: &str,
            ty: ItemType,
            charges: i16,
            abl: &[(Ability, i16)],
        ) -> Item {
            let mut i = blank::clear_item(n);
            i.name = name.into();
            i.item_type = ty;
            i.weight = 10;
            i.gettable = 1;
            i.charges = charges;
            for (slot, a) in i.abilities.iter_mut().zip(abl) {
                *slot = *a;
            }
            i
        }

        /// The stock records, a plain room, the made-up items and a rat to fight.
        fn world() -> World {
            // built once per test process: the property tests load a world per case
            static SNAPSHOT: std::sync::OnceLock<silvermere_engine::WorldSnapshot> =
                std::sync::OnceLock::new();
            let s = SNAPSHOT.get_or_init(snapshot).clone();
            let mut w = World::load(test_config(), 8, s).expect("world loads");
            w.rng = SeededRng::new(7);
            w
        }

        fn snapshot() -> silvermere_engine::WorldSnapshot {
            let fx = fixture();
            let mut s = tiny_snapshot();
            s.items
                .extend(fx["items"].as_array().unwrap().iter().map(item));
            s.spells
                .extend(fx["spells"].as_array().unwrap().iter().map(spell));
            s.classes
                .extend(fx["classes"].as_array().unwrap().iter().map(class));
            s.rooms
                .extend(fx["rooms"].as_array().unwrap().iter().map(room));
            for m in fx["messages"].as_array().unwrap() {
                let t = |i: usize| m["msgs"][i].as_str().unwrap().to_owned();
                s.messages.push(Message {
                    number: MessageNo(int(&m["number"]) as i32),
                    msgs: [t(0), t(1), t(2)],
                });
            }
            for sh in fx["shops"].as_array().unwrap() {
                let mut shop = silvermere_types::records::Shop {
                    number: ShopNo(int(&sh["number"]) as i32),
                    name: sh["name"].as_str().unwrap().into(),
                    desc: Default::default(),
                    shop_type: ShopType::from_code(int(&sh["type"]) as u16),
                    minlevel: int(&sh["minlevel"]) as u16,
                    maxlevel: int(&sh["maxlevel"]) as u16,
                    markup: int(&sh["markup"]) as u16,
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
                };
                for (i, slot) in shop.stock.iter_mut().enumerate() {
                    *slot = ShopSlot {
                        item: ItemNo(int(&sh["items"][i]) as i32),
                        maxqty: int(&sh["maxqty"][i]) as u16,
                        qty: int(&sh["qty"][i]) as u16,
                        regentime: int(&sh["regentime"][i]) as u16,
                        regennum: int(&sh["regennum"][i]) as i16,
                        regenpct: int(&sh["regenpct"][i]) as u8,
                    };
                }
                s.shops.push(shop);
            }
            // only the spell shop's own scroll is in the item list; the others it sells
            // are not needed
            let mut plain = blank::clear_room(RoomKey::new(1, ROOM));
            plain.name = "Plain Room".into();
            s.rooms.push(plain);
            s.items.push(made_up(
                ELIXIR,
                "elixir",
                ItemType::Drink,
                1,
                &[(Ability::Heal, 10)],
            ));
            s.items.push(made_up(
                MANA_DRAUGHT,
                "mana draught",
                ItemType::Drink,
                -1,
                &[(Ability::HealMana, 7)],
            ));
            let mut chest = made_up(
                CHEST,
                "small chest",
                ItemType::Container,
                1,
                &[(Ability::GenLight, 0)],
            );
            chest.cash = Coins {
                gold: 5,
                ..Coins::default()
            };
            s.items.push(chest);
            s.items
                .push(made_up(IRON_KEY, "iron key", ItemType::Key, 3, &[]));
            s.items.push(made_up(
                WAND,
                "wand of mending",
                ItemType::General,
                2,
                &[(Ability::OneTimeCast, 900)],
            ));
            s.items
                .push(made_up(SWORD, "rusty sword", ItemType::Weapon, -1, &[]));
            let mut ring = made_up(RING, "silver ring", ItemType::Armour, -1, &[]);
            ring.worn = WornLocation::from_code(4);
            s.items.push(ring);
            // a spell with a cost, for the One Time Cast mode test: Druid magery,
            // a 2-6 heal, 3 mana, 200 energy, difficulty -100
            let mut mend = spell(&fixture()["spells"][0]);
            mend.number = SpellNo(900);
            mend.name = "item mend".into();
            mend.shortnm = "imend".into();
            mend.spell_type = SpellType::General;
            mend.target = SpellTarget::from_code(1);
            mend.abilities = std::array::from_fn(|_| (Ability::None, 0));
            mend.abilities[0] = (Ability::Heal, 0);
            (mend.minval, mend.maxval) = (2, 6);
            (mend.mana, mend.energy, mend.diff, mend.level) = (3, 200, -100, 1);
            mend.msgnum = MessageNo(645);
            s.spells.push(mend);
            let mut rat = blank::clear_known_monster(RAT);
            rat.name = "giant rat".into();
            rat.level = 1;
            rat.maxhp = 500;
            rat.energy = 1000;
            rat.alignment = Alignment::Neutral;
            rat.montype = MonsterType::Lair;
            s.monsters[0] = rat;
            s
        }

        fn plr(w: &mut World, id: SessionId) -> &mut Character {
            w.session_mut(id).unwrap().character.as_mut().unwrap()
        }

        /// A druid of level 5 with an empty spellbook of 100 slots.
        fn druid(w: &mut World, name: &str, room: u32) -> SessionId {
            let id = enter(w, name, room, MessageLevel::Low);
            let c = plr(w, id);
            c.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
            c.class = DRUID;
            c.race = RaceNo(1);
            c.level = 5;
            (c.hp, c.maxhp) = (30, 40);
            (c.mana, c.maxmana) = (10, 20);
            (c.energy, c.maxenergy) = (1000, 1000);
            c.spellcast = 60;
            c.maxwgt = 2400;
            c.str = 50;
            c.nkeyslots = 10;
            c.readied = -1;
            c.nspellslots = 100;
            c.spellbook = vec![SpellNo(0); 100];
            id
        }

        fn give(w: &mut World, id: SessionId, item: ItemNo) {
            let charges = w.items.get(&item).unwrap().charges;
            assert!(add_item_to_inventory(w, id, item, charges));
        }

        fn carried(w: &mut World, id: SessionId, item: ItemNo) -> usize {
            let c = plr(w, id);
            c.items
                .iter()
                .chain(c.keys.iter())
                .filter(|&&i| i == item)
                .count()
        }

        /// The screen lines written to `id` since the last call: ANSI off, prompts and
        /// blank lines dropped (a line written after a prompt keeps its text).
        fn lines(w: &mut World, id: SessionId) -> Vec<String> {
            let prompt = silvermere_engine::tell::prompt_bytes(w, id);
            let palette = silvermere_engine::tell::palette_of(w, id);
            let s = w.session_mut(id).unwrap();
            let b = s.output.render(palette, &prompt);
            s.output.clear();
            let text = String::from_utf8_lossy(&b).replace("\x1b[79D\x1b[K", "\r\n");
            String::from_utf8_lossy(&strip_ansi(text.as_bytes()))
                .split("\r\n")
                .map(|l| {
                    let l = l.trim_matches([' ', '\u{7}']);
                    let l = match l.strip_prefix("[HP=") {
                        Some(rest) => rest.split_once("]:").map_or("", |(_, t)| t),
                        None => l,
                    };
                    l.trim().to_owned()
                })
                .filter(|l| !l.is_empty())
                .collect()
        }

        fn run(w: &mut World, id: SessionId, line: &str) -> Vec<String> {
            lines(w, id);
            execute_input(w, id, line, false);
            lines(w, id)
        }

        fn spawn(w: &mut World, room: u32) -> ActiveMonsterId {
            let k = w.monsters.get(&RAT).unwrap().clone();
            let number = ActiveMonsterId(1);
            w.active_monsters.insert(ActiveMonster {
                number,
                knownnum: k.number,
                level: k.level,
                map: MapId(1),
                room: RoomId(room),
                poison: 0,
                energy: k.energy,
                hp: k.maxhp,
                following: String::new(),
                trail: std::array::from_fn(|_| RoomId::default()),
                hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
                hunting: ActiveMonsterId::default(),
                name: k.name.clone(),
                wornitem: ItemNo::default(),
                weapon: k.weapon,
                items: std::array::from_fn(|_| (ItemNo::default(), 0)),
                cash: Coins::default(),
                maxhp: k.maxhp,
                alignment: k.alignment,
                followpct: k.followpct,
                dr: k.dr,
                ac: k.ac,
                exp: k.exp,
                maxenergy: k.energy as i16,
                friendly: 0,
                genroom: RoomId(room),
                inactive: 0,
                flags: ActiveMonsterFlags::empty(),
                montype: k.montype,
                target: -1,
                hpregen: 0,
                lastdir: 0,
                lastupd: 0,
                grouprole: GroupRole::Solo,
                spells: std::array::from_fn(|_| ActiveSpell {
                    spell: Default::default(),
                    value: 0,
                    rounds: 0,
                }),
                downrounds: 0,
                created: 0,
            });
            let rs = w.room_state.get_mut(&RoomKey::new(1, room)).unwrap();
            *rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap() = number;
            number
        }

        fn mon_hp(w: &World, n: ActiveMonsterId) -> i16 {
            w.active_monsters
                .values()
                .find(|m| m.number == n)
                .map_or(0, |m| m.hp)
        }

        // BUY, then READ and the spell typed by name: the druid plan character buys the
        // scroll of vine
        // strike in the Newhaven Spell Shop and reads it; the scroll is used up (its
        // destroy message, WCCMSG 8420, is empty: an empty line), SPELLS lists the spell
        // and it can be cast; a second scroll is already known and stays.
        #[test]
        fn learn_vine_strike_from_a_scroll_then_cast_it() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", SPELL_SHOP);
            let bought = run(&mut w, id, "buy scroll of vine strike");
            assert_eq!(carried(&mut w, id, SCROLL), 1, "{bought:?}");
            let out = run(&mut w, id, "use scroll of vine strike");
            assert_eq!(
                out,
                ["You read scroll of vine strike and learn the spell vine strike."]
            );
            assert_eq!(carried(&mut w, id, SCROLL), 0);
            assert!(plr(&mut w, id).spellbook.contains(&VINE));
            let spells = run(&mut w, id, "spells").join("\n");
            assert!(spells.contains("vine strike"), "{spells}");

            // a second scroll: already known, not used up
            run(&mut w, id, "buy scroll of vine strike");
            let out = run(&mut w, id, "read scroll of vine strike");
            assert_eq!(out, ["You realize that you already know this scroll!"]);
            assert_eq!(carried(&mut w, id, SCROLL), 1);

            // and the spell learned from it is cast at a rat: by hand an instant attack
            // spell starts the fight
            let rat = spawn(&mut w, SPELL_SHOP);
            let out = run(&mut w, id, "cast vine giant rat");
            assert!(out.iter().any(|l| l == "*Combat Engaged*"), "{out:?}");
            plr(&mut w, id).energy = 1000;
            plr(&mut w, id).settings.insert(PlayerSettings::CANCAST);
            let before = mon_hp(&w, rat);
            let r = silvermere_engine::spells::autocombat_cast(
                &mut w,
                id,
                VINE,
                Some(silvermere_engine::combat::AutoTarget::Monster(rat)),
            );
            assert_ne!(r.outcome, silvermere_engine::spells::CastOutcome::Refused);
            let out = lines(&mut w, id);
            assert!(
                mon_hp(&w, rat) < before || out.iter().any(|l| l.contains("fail")),
                "{out:?}"
            );
        }

        /// The bytes written to `id` for `line`, prompts left out (rendered empty).
        fn raw(w: &mut World, id: SessionId, line: &str) -> Vec<u8> {
            lines(w, id);
            execute_input(w, id, line, false);
            let palette = silvermere_engine::tell::palette_of(w, id);
            let s = w.session_mut(id).unwrap();
            let b = s.output.render(palette, b"");
            s.output.clear();
            b
        }

        // USE's and READ's ambiguous-match handling clears the output buffer, which drops
        // the dispatcher's colour reset and the scroll's
        // clear-line sequence, so a known scroll's line comes bare (m4 run 38823959731,
        // `read scroll`); the torch lights and the key turns with no reset in front
        // (for LIGHT, for USE of a light and for a key alike).
        #[test]
        fn find_helpers_clear_the_pending_reset() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", SPELL_SHOP);
            run(&mut w, id, "buy scroll of vine strike");
            // A new scroll: the learn line, then the (empty) destroy message
            assert_eq!(
                raw(&mut w, id, "read scroll of vine strike"),
                b"You read scroll of vine strike and learn the spell vine strike.\r\n\r\n"
            );
            run(&mut w, id, "buy scroll of vine strike");
            // A known one, by READ and by USE
            for line in ["read scroll of vine strike", "use scroll of vine strike"] {
                assert_eq!(
                    raw(&mut w, id, line),
                    b"You realize that you already know this scroll!\r\n",
                    "{line}"
                );
            }
            give(&mut w, id, TORCH);
            give(&mut w, id, TORCH);
            assert_eq!(raw(&mut w, id, "light torch"), b"You lit the torch.\r\n");
            assert_eq!(
                raw(&mut w, id, "light torch"),
                b"You already have something lit!\r\n"
            );
            // USE of a light goes to LIGHT
            assert_eq!(
                raw(&mut w, id, "use torch"),
                b"You already have something lit!\r\n"
            );
            // a key on the Key exit
            let k = druid(&mut w, "Kai", TOMB);
            give(&mut w, k, BONE_KEY);
            assert_eq!(
                raw(&mut w, k, "use bone key n"),
                b"You successfully unlocked the door.\r\n"
            );
            assert_eq!(
                raw(&mut w, k, "use bone key n"),
                b"The door was not locked.\r\n"
            );
        }

        // A Witchunter (Anti-Magic) may not
        // use a Magical potion; DRINK refuses silently.
        #[test]
        fn witchunter_refuses_a_magical_potion() {
            let mut w = world();
            let id = druid(&mut w, "Hal", ROOM);
            plr(&mut w, id).class = WITCHUNTER;
            give(&mut w, id, POTION);
            assert_eq!(
                run(&mut w, id, "use minor healing potion"),
                ["You may not use that item!"]
            );
            assert_eq!(carried(&mut w, id, POTION), 1);
        }

        // A potion with two
        // uses is drunk twice and goes with its (empty) destroy message; each drink is a
        // One Time Cast of spell 50.
        #[test]
        fn a_two_use_potion_drunk_twice_disappears() {
            let mut w = world();
            w.items.get_mut(&POTION).unwrap().charges = 2;
            let id = druid(&mut w, "Rowan", ROOM);
            give(&mut w, id, POTION);
            plr(&mut w, id).hp = 10;
            let first = run(&mut w, id, "drink minor healing potion");
            assert_eq!(first[0], "You drink the minor healing potion.");
            assert!(
                first.iter().any(|l| l.contains("healing warmth")),
                "{first:?}"
            );
            assert!(plr(&mut w, id).hp > 10);
            assert_eq!(carried(&mut w, id, POTION), 1);
            let second = run(&mut w, id, "drink minor healing potion");
            assert_eq!(second[0], "You drink the minor healing potion.");
            assert_eq!(carried(&mut w, id, POTION), 0);
            // a third: nothing by that name, DRINK does not handle it
            let third = run(&mut w, id, "drink minor healing potion");
            assert!(
                !third.iter().any(|l| l.starts_with("You drink")),
                "{third:?}"
            );
        }

        // The bone key opens the Key exit north of
        // the sealed tomb (exitpar1 177, locked); used again it was not locked; a key
        // that does not fit says so and keeps its use; with no direction nothing happens.
        #[test]
        fn bone_key_unlocks_the_key_exit() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", TOMB);
            give(&mut w, id, BONE_KEY);
            give(&mut w, id, IRON_KEY);
            let tomb = RoomKey::new(1, TOMB);
            let north = silvermere_types::enums::Direction::North;
            let state = |w: &World| {
                silvermere_engine::movement::exits::live_exit(w, tomb, north)
                    .unwrap()
                    .par2
            };
            assert_eq!(state(&w), 2);
            assert_eq!(
                run(&mut w, id, "use iron key n"),
                ["The iron key doesn't seem to fit that lock."]
            );
            assert_eq!(
                plr(&mut w, id).keyuses.iter().filter(|&&u| u == 3).count(),
                2
            );
            assert!(run(&mut w, id, "use bone key").is_empty());
            assert_eq!(
                run(&mut w, id, "use bone key n"),
                ["You successfully unlocked the door."]
            );
            assert_eq!(state(&w), 1);
            assert_eq!(
                run(&mut w, id, "use bone key north"),
                ["The door was not locked."]
            );
            // the key fits both times: two of its three uses are gone
            let c = plr(&mut w, id);
            let i = c.keys.iter().position(|&k| k == BONE_KEY).unwrap();
            assert_eq!(c.keyuses[i], 1);
            // the third use breaks it (WCCMSG 1398)
            assert_eq!(
                run(&mut w, id, "use bone key n"),
                [
                    "The door was not locked.",
                    "The key breaks and crumbles apart."
                ]
            );
            assert_eq!(carried(&mut w, id, BONE_KEY), 0);
        }

        // USE of a light source lights it; LIGHT alone
        // then tells the light level.
        #[test]
        fn light_a_torch_then_ask_the_level() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            give(&mut w, id, TORCH);
            give(&mut w, id, TORCH);
            assert_eq!(run(&mut w, id, "light torch"), ["You lit the torch."]);
            let level = run(&mut w, id, "light");
            assert_eq!(level.len(), 1);
            assert!(
                level[0].starts_with("The current light level is"),
                "{level:?}"
            );
            assert_eq!(
                run(&mut w, id, "use torch"),
                ["You already have something lit!"]
            );
        }

        // EAT a stock iron ration: the line, the room's line, its
        // One Time Cast (spell 316 food, WCCMSG 645) and its destroy message (8610).
        #[test]
        fn eat_an_iron_ration() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            let other = druid(&mut w, "Bob", ROOM);
            give(&mut w, id, RATION);
            plr(&mut w, id).hp = 10;
            lines(&mut w, other);
            let out = run(&mut w, id, "eat iron ration");
            assert_eq!(out[0], "You eat the iron ration.");
            assert!(
                out.contains(&"You feel a little better.".to_owned()),
                "{out:?}"
            );
            assert_eq!(
                out.last().unwrap(),
                "The iron ration tastes bland, but is nutritious.",
                "{out:?}"
            );
            assert_eq!(carried(&mut w, id, RATION), 0);
            assert!(lines(&mut w, other).contains(&"Rowan eats iron ration.".to_owned()));
            // nothing edible by that name: not handled, no line of EAT's
            let again = run(&mut w, id, "eat iron ration");
            assert!(!again.iter().any(|l| l.starts_with("You eat")), "{again:?}");
        }

        // The obsidian claymore (Auto Use 100,
        // One Time Cast 172 dark fire) fires dark fire at the rat it hit.
        #[test]
        fn auto_use_weapon_casts_on_the_victim() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            give(&mut w, id, CLAYMORE);
            plr(&mut w, id).weapon = CLAYMORE;
            plr(&mut w, id).class = ClassNo(1);
            let rat = spawn(&mut w, ROOM);
            let mut fired = false;
            for _ in 0..40 {
                plr(&mut w, id).energy = 1000;
                attack_user_monster(&mut w, id, rat, Move::Attack, Move::Attack, Mode::Auto);
                let out = lines(&mut w, id);
                if out
                    .iter()
                    .any(|l| l.starts_with("Dark flame sears giant rat"))
                {
                    fired = true;
                    break;
                }
            }
            assert!(fired, "no dark fire in 40 rounds");
        }

        // A One Time Cast is cast
        // in item mode: no success roll (spellcast 0 and difficulty -100 still work),
        // but the spell's energy and mana are needed and spent; a class without the
        // magery is refused silently and the item keeps its charge.
        #[test]
        fn one_time_cast_mode_one() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            give(&mut w, id, WAND);
            {
                let c = plr(&mut w, id);
                c.spellcast = 0;
                c.hp = 10;
            }
            let out = run(&mut w, id, "use wand of mending");
            assert!(
                out.contains(&"You feel a little better.".to_owned()),
                "{out:?}"
            );
            let c = plr(&mut w, id);
            assert!(c.hp > 10);
            assert_eq!((c.mana, c.energy), (7, 800));
            // out of mana: nothing is cast, nothing spent
            plr(&mut w, id).mana = 2;
            run(&mut w, id, "use wand of mending");
            let c = plr(&mut w, id);
            assert_eq!(c.mana, 2);
            let i = c.items.iter().position(|&n| n == WAND).unwrap();
            assert_eq!(c.itemuses[i], 1);
            // no Druid magery: refused, nothing printed, the charge kept
            plr(&mut w, id).class = WITCHUNTER;
            plr(&mut w, id).mana = 20;
            assert!(run(&mut w, id, "use wand of mending").is_empty());
            let c = plr(&mut w, id);
            let i = c.items.iter().position(|&n| n == WAND).unwrap();
            assert_eq!(c.itemuses[i], 1);
        }

        // Heal and Heal Mana, capped at the maximum, on the
        // user and on another player (who is told).
        #[test]
        fn heal_and_heal_mana() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            let bob = druid(&mut w, "Bob", ROOM);
            give(&mut w, id, ELIXIR);
            give(&mut w, id, MANA_DRAUGHT);
            plr(&mut w, id).hp = 35;
            plr(&mut w, id).mana = 18;
            assert_eq!(
                run(&mut w, id, "use elixir"),
                [
                    "You receive 10 hit points.",
                    "It's uses gone, elixir disappears from your inventory!"
                ]
            );
            assert_eq!(plr(&mut w, id).hp, 40);
            assert_eq!(run(&mut w, id, "use mana draught"), ["You receive 7 mana."]);
            assert_eq!(plr(&mut w, id).mana, 20);
            // on another player: Bob is told, the user sees nothing
            plr(&mut w, bob).mana = 0;
            lines(&mut w, bob);
            assert!(run(&mut w, id, "use mana draught bob").is_empty());
            assert_eq!(plr(&mut w, bob).mana, 7);
            assert_eq!(lines(&mut w, bob), ["You receive 7 mana."]);
            // DRINK runs the same abilities
            assert_eq!(
                run(&mut w, id, "drink mana draught"),
                ["You drink the mana draught.", "You receive 7 mana."]
            );
        }

        // The refusals and the fallbacks.
        #[test]
        fn use_refusals() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            assert_eq!(
                run(&mut w, id, "use"),
                ["Syntax: USE {Item to use} [{target}]"]
            );
            assert_eq!(run(&mut w, id, "read"), ["Syntax: Read {Item}"]);
            assert_eq!(
                run(&mut w, id, "use frobnitz thing"),
                ["You don't have frobnitz thing."]
            );
            w.classes.get_mut(&DRUID).unwrap().weapon = WeaponClass::All;
            give(&mut w, id, SWORD);
            assert_eq!(
                run(&mut w, id, "use rusty sword"),
                ["You must have that item readied to use it!"]
            );
            give(&mut w, id, RING);
            assert_eq!(
                run(&mut w, id, "use silver ring"),
                ["You must be wearing that item to use it!"]
            );
            give(&mut w, id, WATERSKIN);
            let c = plr(&mut w, id);
            let i = c.items.iter().position(|&n| n == WATERSKIN).unwrap();
            c.itemuses[i] = 0;
            assert_eq!(
                run(&mut w, id, "use waterskin"),
                ["There are no more uses in waterskin."]
            );
            assert_eq!(
                run(&mut w, id, "drink waterskin"),
                ["There is nothing more that waterskin can do for you."]
            );
        }

        // OPEN {container} rolls its coins and takes a charge; an
        // empty one says so.
        #[test]
        fn open_a_container() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            give(&mut w, id, CHEST);
            run(&mut w, id, "open small chest");
            assert!(plr(&mut w, id).coins.gold <= 5);
            let c = plr(&mut w, id);
            let i = c.items.iter().position(|&n| n == CHEST);
            assert!(i.is_none(), "the chest's one use is gone");
            w.items.get_mut(&CHEST).unwrap().retain = 1;
            give(&mut w, id, CHEST);
            run(&mut w, id, "open small chest");
            assert_eq!(
                run(&mut w, id, "open small chest"),
                ["There is nothing more that small chest can do for you."]
            );
        }

        // Through wear/remove: Give Spell and Alter HP while worn.
        #[test]
        fn worn_item_gives_a_spell_and_hit_points() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            {
                let ring = w.items.get_mut(&RING).unwrap();
                ring.abilities[0] = (Ability::GiveSpell, VINE.0);
                ring.abilities[1] = (Ability::AlterHp, 5);
            }
            {
                let ring = w.items.get_mut(&RING).unwrap();
                ring.abilities[0] = (Ability::GiveSpell, VINE.0);
                ring.abilities[1] = (Ability::AlterHp, 5);
            }
            give(&mut w, id, RING);
            let out = run(&mut w, id, "wear silver ring");
            assert!(plr(&mut w, id).spellbook.contains(&VINE), "{out:?}");
            let worn = plr(&mut w, id).maxhp;
            run(&mut w, id, "remove silver ring");
            assert!(!plr(&mut w, id).spellbook.contains(&VINE));
            assert_eq!(plr(&mut w, id).maxhp, worn - 5);
        }

        // An item used on a spell does nothing.
        #[test]
        fn use_on_a_spell_does_nothing() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", ROOM);
            give(&mut w, id, ELIXIR);
            assert_eq!(use_item(&mut w, id, ELIXIR, UseTarget::Spell(DARK_FIRE)), 0);
            assert_eq!(use_item(&mut w, id, ELIXIR, UseTarget::None), 1);
        }

        /// The sixteen screen-pattern rows of the card, each as
        /// the engine writes it.
        #[test]
        fn screen_lines() {
            let mut w = world();
            let id = druid(&mut w, "Rowan", SPELL_SHOP);
            let bob = druid(&mut w, "Bob", SPELL_SHOP);
            let mut rows: Vec<(&str, Vec<String>)> = Vec::new();
            let mut row = |name, out: Vec<String>| rows.push((name, out));

            give(&mut w, id, SCROLL);
            give(&mut w, id, SCROLL);
            row("learn_scroll", run(&mut w, id, "use scroll of vine strike"));
            row(
                "already_known",
                run(&mut w, id, "use scroll of vine strike"),
            );
            plr(&mut w, id).level = 0;
            plr(&mut w, id).spellbook = vec![SpellNo(0); 100];
            plr(&mut w, id).class = ClassNo(1);
            let scroll = w.items.get(&SCROLL).unwrap().clone();
            silvermere_engine::items::use_item::use_no_target(&mut w, id, &scroll, None);
            row("use_unknown", lines(&mut w, id));
            plr(&mut w, id).class = DRUID;
            plr(&mut w, id).level = 5;
            give(&mut w, id, ELIXIR);
            row("receive_hp", run(&mut w, id, "use elixir"));
            row("uses_gone", vec![]);
            give(&mut w, id, WATERSKIN);
            {
                let c = plr(&mut w, id);
                let i = c.items.iter().position(|&n| n == WATERSKIN).unwrap();
                c.itemuses[i] = 0;
            }
            row("no_uses", run(&mut w, id, "use waterskin"));
            plr(&mut w, id).class = WITCHUNTER;
            give(&mut w, id, POTION);
            row("use_forbidden", run(&mut w, id, "use minor healing potion"));
            plr(&mut w, id).class = DRUID;
            row("dont_have", run(&mut w, id, "use feather"));
            give(&mut w, id, RATION);
            lines(&mut w, bob);
            row("eat_drink", run(&mut w, id, "eat iron ration"));
            row("other_eat_drink", lines(&mut w, bob));
            give(&mut w, id, TORCH);
            row("light_ok", run(&mut w, id, "light torch"));
            give(&mut w, id, TORCH);
            row("light_fail", run(&mut w, id, "light torch"));
            row("light_level", run(&mut w, id, "light"));
            // the torch burns down on the medium cycle (T-037): its own destroy message
            // (WCCMSG 8603, no room line); a torch without one gets the standard pair
            let burn_out = |w: &mut World| {
                let c = plr(w, id);
                let i = c.readied as usize;
                c.itemuses[i] = 1;
                silvermere_engine::cycles::medium::burn_light(w, id);
            };
            burn_out(&mut w);
            row("light_out", lines(&mut w, id));
            w.items.get_mut(&TORCH).unwrap().destroymsg = MessageNo(0);
            give(&mut w, id, TORCH);
            run(&mut w, id, "light torch");
            lines(&mut w, bob);
            burn_out(&mut w);
            let standard = lines(&mut w, id);
            row("light_out_other", lines(&mut w, bob));
            row("light_out (standard)", standard);
            // REMOVE of a light source in use that is not the readied one (T-030's
            // remove_armour): here a torch held in a worn slot
            give(&mut w, id, TORCH);
            plr(&mut w, id).worn[0] = TORCH;
            row("not_lit", run(&mut w, id, "remove torch"));
            drop(row);

            // the uses_gone row: the elixir's line (no destroy message)
            let elixir = rows.iter().find(|r| r.0 == "receive_hp").unwrap().1.clone();
            let mut text = String::new();
            for (name, out) in &rows {
                let out = if *name == "uses_gone" {
                    elixir[1..].to_vec()
                } else if *name == "receive_hp" {
                    elixir[..1].to_vec()
                } else {
                    out.clone()
                };
                text.push_str(&format!("{name}: {}\n", out.join(" | ")));
            }
            insta::assert_snapshot!(text);
        }

        proptest! {
            // charges never go below 0, an item kept when empty
            // (`retain`) never disappears, -1 never changes.
            #[test]
            fn charges_never_negative(start in -1i16..6, uses in 0usize..10, retain in any::<bool>()) {
                let mut w = world();
                w.items.get_mut(&WAND).unwrap().retain = u8::from(retain);
                w.items.get_mut(&WAND).unwrap().charges = start;
                let id = druid(&mut w, "Rowan", ROOM);
                prop_assume!(start != 0);
                give(&mut w, id, WAND);
                let mut charges = start;
                for _ in 0..uses {
                    if charges == 0 {
                        break;
                    }
                    match deduct_item_charge(&mut w, id, WAND, charges) {
                        Charge::Left(n) => {
                            prop_assert!(n >= 0 || n == -1);
                            charges = n;
                        }
                        Charge::Gone => {
                            prop_assert!(!retain);
                            charges = 0;
                        }
                    }
                }
                let held = carried(&mut w, id, WAND);
                if retain || start == -1 {
                    prop_assert_eq!(held, 1);
                }
                let c = plr(&mut w, id);
                prop_assert!(c.itemuses.iter().all(|&u| u >= -2));
            }
        }
    }
}
