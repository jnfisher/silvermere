//! The text block script engine (T-054): the parser and the wildcard table,
//! one test per keyword of `perform_matched_action` on a synthetic world, the three
//! block runners, the named teleports, ASK and GREET, the Remote Action exits and
//! `run_room_exit_action`. The tests over the stock records (every reachable block parsed,
//! the Newhaven skiff, the crypt quest, monster talk, random text) are
//! `silvermere-testkit/tests/stock_scripts.rs`, which builds them from the download.
//!
//! Every test lives in `mod scripts`, so `cargo test -p silvermere-engine scripts::`
//! selects them with the unit tests of `src/scripts`.

use crate::common;

mod scripts {
    use std::collections::BTreeSet;

    use super::common::{enter, test_config, tiny_snapshot};
    use silvermere_engine::commands::input::execute_input;

    use silvermere_engine::movement::exits::{LOCK_LOCKED, LOCK_OPEN, live_exit};

    use silvermere_engine::scripts::{
        Clause, KEYWORDS, LineResult, STOCK_TELEPORTS, pattern_lines, perform_matched_action,
        perform_special_command, run_text_block, whole_lines, wildcard_match,
    };
    use silvermere_engine::tell::palette_of;
    use silvermere_engine::world::blank;
    use silvermere_engine::{SeededRng, SessionId, World};
    use silvermere_types::enums::{
        Ability, Alignment, DebugLevel, Direction, ExitType, GroupRole, ItemType, Magery,
        MessageLevel, MonsterType,
    };
    use silvermere_types::flags::{ActiveMonsterFlags, ExitFlags, PlayerFlags, SpellFlags};
    use silvermere_types::ids::{
        ActiveMonsterId, ItemNo, MessageNo, MonsterNo, RoomId, RoomKey, SpellNo, TextBlockNo,
    };
    use silvermere_types::records::{
        ActiveMonster, ActiveSpell, Coins, Exit, Message, Spell, TextBlock,
    };
    use silvermere_types::sizes::{NAMTRAIL, NMONITEMS, NMONSPELLS};

    // -----------------------------------------------------------------------------------
    // The synthetic world.

    /// The room the keyword tests stand in.
    const HERE: u32 = 6000;
    /// A teleport target.
    const THERE: u32 = 6001;
    /// A room with a locked door north to `DOOR_FAR`, which has the door back south.
    const DOOR_ROOM: u32 = 6002;
    const DOOR_FAR: u32 = 6003;
    /// A room whose north exit is Hidden behind two blocks, any order.
    const HIDDEN_ROOM: u32 = 6004;
    /// "You do it." / "%s does it."
    const MSG_DO: i32 = 900;
    /// "No."
    const MSG_NO: i32 = 901;
    const SWORD: i32 = 51;
    const ANVIL: i32 = 52;
    const KEY: i32 = 53;
    const SPELL: i16 = 7;

    fn exit(target: u32, kind: ExitType) -> Exit {
        Exit {
            target: RoomId(target),
            kind,
            par1: 0,
            par2: 0,
            par3: 0,
            par4: 0,
        }
    }

    fn message(n: i32, texts: [&str; 3]) -> Message {
        Message {
            number: MessageNo(n),
            msgs: texts.map(String::from),
        }
    }

    fn spell(n: i16, name: &str) -> Spell {
        Spell {
            number: SpellNo(n),
            name: name.into(),
            descs: [String::new(), String::new()],
            resist: 0,
            levelcap: 0,
            flags: SpellFlags::empty(),
            abilities: std::array::from_fn(|_| (Ability::None, 0)),
            energy: 0,
            level: 0,
            minval: 0,
            maxval: 0,
            spell_type: silvermere_types::enums::SpellType::from_code(0),
            resmode: silvermere_types::enums::ResistMode::from_code(0),
            diff: 0,
            durlvl: 0,
            target: silvermere_types::enums::SpellTarget::from_code(0),
            duration: 0,
            element: silvermere_types::enums::Element::from_code(0),
            resstat: silvermere_types::enums::ResistStat::from_code(0),
            magery: Magery::None,
            msgnum: MessageNo(0),
            mana: 0,
            maxinc: 0,
            maxlvls: 0,
            magelvl: 0,
            mininc: 0,
            minlvls: 0,
            durinc: 0,
            durlvls: 0,
            shortnm: "vine".into(),
        }
    }

    fn active_monster(number: i32, known: i32, name: &str, room: u32) -> ActiveMonster {
        ActiveMonster {
            number: ActiveMonsterId(number),
            knownnum: MonsterNo(known),
            level: 0,
            map: 1.into(),
            room: room.into(),
            poison: 0,
            energy: 0,
            hp: 10,
            following: String::new(),
            trail: [0.into(); NAMTRAIL],
            hunters: [ActiveMonsterId(0); 10],
            hunting: ActiveMonsterId(0),
            name: name.into(),
            wornitem: ItemNo(0),
            weapon: ItemNo(0),
            items: [(ItemNo(0), 0); NMONITEMS],
            cash: Coins::default(),
            maxhp: 10,
            alignment: Alignment::from_code(0),
            followpct: 0,
            dr: 0,
            ac: 0,
            exp: 0,
            maxenergy: 0,
            friendly: 0,
            genroom: room.into(),
            inactive: 0,
            flags: ActiveMonsterFlags::empty(),
            montype: MonsterType::from_code(0),
            target: 0,
            hpregen: 0,
            lastdir: 0,
            lastupd: 0,
            grouprole: GroupRole::from_code(0),
            spells: [ActiveSpell {
                spell: SpellNo(0),
                value: 0,
                rounds: 0,
            }; NMONSPELLS],
            downrounds: 0,
            created: 0,
        }
    }

    /// Puts an active monster of kind `known` into `room` (map 1).
    fn place(w: &mut World, number: i32, known: i32, name: &str, room: u32) {
        w.active_monsters
            .insert(active_monster(number, known, name, room));
        let st = w.room_state.get_mut(&RoomKey::new(1, room)).unwrap();
        let slot = st.monsters.iter().position(|m| m.0 == 0).unwrap();
        st.monsters[slot] = ActiveMonsterId(number);
    }

    fn block(w: &mut World, n: i32, text: &str) {
        w.text_blocks.insert(
            (0, TextBlockNo(n)),
            TextBlock {
                part: 0,
                number: TextBlockNo(n),
                link: TextBlockNo(0),
                text: text.into(),
            },
        );
    }

    struct T {
        w: World,
        /// The player the scripts run for.
        me: SessionId,
        /// Another player in the same room.
        bob: SessionId,
    }

    fn world() -> T {
        let mut s = tiny_snapshot();
        for n in [
            HERE,
            THERE,
            DOOR_ROOM,
            DOOR_FAR,
            HIDDEN_ROOM,
            HIDDEN_ROOM + 1,
        ] {
            let mut r = blank::clear_room(RoomKey::new(1, n));
            r.name = format!("Room {n}");
            s.rooms.push(r);
        }
        let door = |target| Exit {
            par1: LOCK_LOCKED,
            par3: 3,
            ..exit(target, ExitType::Door)
        };
        s.rooms.iter_mut().for_each(|r| match r.key.room.0 {
            DOOR_ROOM => r.exits[0] = door(DOOR_FAR),
            DOOR_FAR => r.exits[1] = door(DOOR_ROOM),
            HIDDEN_ROOM => {
                r.exits[0] = Exit {
                    par1: (ExitFlags::BLOCKED1 | ExitFlags::BLOCKED2).bits(),
                    par2: -2,
                    ..exit(HIDDEN_ROOM + 1, ExitType::Hidden)
                }
            }
            _ => {}
        });
        s.messages
            .push(message(MSG_DO, ["You do it.", "%s does it.", ""]));
        s.messages.push(message(MSG_NO, ["No.", "", ""]));
        for (n, name, weight) in [
            (SWORD, "sword", 1),
            (ANVIL, "anvil", 30000),
            (KEY, "key", 0),
        ] {
            let mut it = blank::clear_item(ItemNo(n));
            it.name = name.into();
            it.weight = weight;
            it.gettable = 1;
            it.charges = 3;
            if n == KEY {
                it.item_type = ItemType::Key;
            }
            s.items.push(it);
        }
        s.spells.push(spell(SPELL, "vine strike"));
        let mut w = World::load(test_config(), 8, s).expect("the world loads");
        w.rng = SeededRng::new(1);
        let me = enter(&mut w, "Kang", HERE, MessageLevel::High);
        let bob = enter(&mut w, "Bob", HERE, MessageLevel::High);
        {
            let p = w.session_mut(me).unwrap().character.as_mut().unwrap();
            p.level = 10;
            p.agi = 50;
            p.evil = 10;
            p.str = 50;
            p.flags.insert(PlayerFlags::NEWEXP);
            p.nspellslots = 10;
            p.maxwgt = 1000;
        }
        T { w, me, bob }
    }

    impl T {
        fn run(&mut self, line: &str) -> LineResult {
            perform_matched_action(&mut self.w, self.me, line)
        }

        /// What `id` would receive now, `<P>` for a prompt; the batch emptied.
        fn out_of(&mut self, id: SessionId) -> String {
            let palette = palette_of(&self.w, id);
            let s = self.w.session_mut(id).unwrap();
            let out = s.output.render(palette, b"<P>");
            s.output.clear();
            String::from_utf8_lossy(&out).replace("\x1b[79D\x1b[K", "")
        }

        fn out(&mut self) -> String {
            self.out_of(self.me)
        }

        fn plr(&mut self) -> &mut silvermere_types::records::Character {
            self.w
                .session_mut(self.me)
                .unwrap()
                .character
                .as_mut()
                .unwrap()
        }

        fn room(&self, n: u32) -> &silvermere_engine::RoomState {
            &self.w.room_state[&RoomKey::new(1, n)]
        }
    }

    use LineResult::{Nothing, Ok as OK, Stop};

    // -----------------------------------------------------------------------------------
    // The keywords, in the chain's order. The names list below is
    // checked against KEYWORDS so a keyword without a test fails the build of the list.

    const TESTED: [&str; 42] = [
        "price",
        "cast",
        "giveability",
        "addability",
        "teleport",
        "removeability",
        "nomonsters",
        "monsters",
        "needmonster",
        "summon",
        "message",
        "takeitem",
        "addexp",
        "giveitem",
        "hideitem",
        "checkability",
        "class",
        "race",
        "goodaligned",
        "evilaligned",
        "minlevel",
        "maxlevel",
        "checkitem",
        "failitem",
        "text",
        "roomtext",
        "roomitem",
        "failroomitem",
        "clearitem",
        "addevil",
        "testability",
        "failability",
        "remoteaction",
        "random",
        "adddelay",
        "testskill",
        "checkskill",
        "givecoins",
        "test_tournament",
        "checkspell",
        "flag",
        "learnspell",
    ];

    // 42 keywords, each with a test named `kw_<keyword>` below.
    #[test]
    fn every_keyword_has_a_test() {
        let names: Vec<&str> = KEYWORDS.iter().map(|(k, _)| *k).collect();
        assert_eq!(names, TESTED);
        assert_eq!(KEYWORDS.len(), 42);
    }

    // The price is paid, or the line stops; the KID_GLOVES refund
    #[test]
    fn kw_price() {
        let mut t = world();
        t.plr().coins.copper = 50;
        assert_eq!(t.run(&format!("price 30 {MSG_NO}:message {MSG_DO}")), OK);
        assert_eq!(t.plr().coins.copper, 20);
        assert_eq!(t.out(), "You do it.\r\n");
        assert_eq!(t.run(&format!("price 30 {MSG_NO}:message {MSG_DO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        // a refund of what the line took, with KID_GLOVES on
        t.w.config.kid_gloves = true;
        t.run(&format!("giveitem {SWORD}"));
        assert!(t.plr().items.contains(&ItemNo(SWORD)));
        assert_eq!(t.run(&format!("takeitem {SWORD}:price 999")), Stop);
        assert!(
            t.plr().items.contains(&ItemNo(SWORD)),
            "given back: {:?}",
            t.out()
        );
        t.w.config.kid_gloves = false;
        assert_eq!(t.run(&format!("takeitem {SWORD}:price 999")), Stop);
        assert!(!t.plr().items.contains(&ItemNo(SWORD)), "kept");
    }

    // No such spell passes; a spell goes to an item-mode `cast_no_target` (T-050), whose 0
    // stops the line: this one is a Single spell, which needs a target
    #[test]
    fn kw_cast() {
        let mut t = world();
        assert_eq!(t.run("cast 99:message 900"), OK);
        assert_eq!(t.run(&format!("cast {SPELL}:message 900")), Stop);
        assert_eq!(
            t.out(),
            "You do it.\r\n\u{1b}[1;31;40mYou must specify a target for that spell!\r\n"
        );
    }

    fn abil(t: &mut T, code: u16) -> Option<i16> {
        t.plr()
            .abils
            .iter()
            .find(|(a, _)| a.code() == code)
            .map(|(_, v)| *v)
    }

    // A new slot, or raised to the value when lower
    #[test]
    fn kw_giveability() {
        let mut t = world();
        assert_eq!(t.run("giveability 50 3"), OK);
        assert_eq!(abil(&mut t, 50), Some(3));
        assert_eq!(t.run("giveability 50 2"), OK);
        assert_eq!(abil(&mut t, 50), Some(3));
        assert_eq!(t.run("giveability 50 5"), OK);
        assert_eq!(abil(&mut t, 50), Some(5));
        // Give Spell (160) also writes the spell into the book
        assert_eq!(t.run(&format!("giveability 160 {SPELL}")), OK);
        assert!(t.plr().spellbook.contains(&SpellNo(SPELL)));
        // no free slot stops the line
        for slot in t.plr().abils.iter_mut() {
            if slot.0.code() == 0 {
                *slot = (Ability::from_code(1), 1);
            }
        }
        assert_eq!(t.run("giveability 51 1"), Stop);
    }

    // Added to; Give Spell refused
    #[test]
    fn kw_addability() {
        let mut t = world();
        assert_eq!(t.run("addability 50 2"), OK);
        assert_eq!(t.run("addability 50 2"), OK);
        assert_eq!(abil(&mut t, 50), Some(4));
        assert_eq!(t.run("addability 160 7"), Stop);
    }

    // The plain form
    #[test]
    fn kw_teleport() {
        let mut t = world();
        assert_eq!(t.run("teleport"), OK, "no room: nothing");
        assert_eq!(t.run(&format!("teleport {THERE}")), OK, "no map: nothing");
        assert_eq!(t.plr().room, RoomId(HERE));
        assert_eq!(t.run(&format!("teleport {THERE} 1")), OK);
        assert_eq!(t.plr().room, RoomId(THERE));
        assert_eq!(t.out(), "", "the room is not shown");
        assert_eq!(t.run("teleport 9999 1:message 900"), Stop, "no such room");
        assert_eq!(t.plr().room, RoomId(THERE));
    }

    // Each named teleport lands in one of its ranges on its map
    #[test]
    fn named_teleports_land_on_their_maps() {
        let mut s = tiny_snapshot();
        for area in STOCK_TELEPORTS {
            for &(lo, hi) in area.ranges {
                for n in lo..=hi {
                    s.rooms
                        .push(blank::clear_room(RoomKey::new(area.map, n as u32)));
                }
            }
        }
        s.rooms.push(blank::clear_room(RoomKey::new(1, HERE)));
        let mut w = World::load(test_config(), 2, s).unwrap();
        w.rng = SeededRng::new(3);
        let me = enter(&mut w, "Kang", HERE, MessageLevel::High);
        assert_eq!(STOCK_TELEPORTS.len(), 12);
        for area in STOCK_TELEPORTS {
            let mut seen = BTreeSet::new();
            for _ in 0..400 {
                assert_eq!(perform_matched_action(&mut w, me, area.keyword), OK);
                let p = w.session(me).unwrap().player().unwrap();
                assert_eq!(p.map.0, area.map, "{}", area.keyword);
                let r = i64::from(p.room.0);
                let i = area
                    .ranges
                    .iter()
                    .position(|&(lo, hi)| (lo..=hi).contains(&r))
                    .unwrap_or_else(|| panic!("{} landed in {r}", area.keyword));
                seen.insert(i);
            }
            assert_eq!(
                seen.len(),
                area.ranges.len(),
                "{}: every range",
                area.keyword
            );
        }
    }

    #[test]
    fn kw_removeability() {
        let mut t = world();
        t.run("giveability 50 3");
        assert_eq!(t.run("removeability 50"), OK);
        assert_eq!(abil(&mut t, 50), None);
        assert_eq!(t.run("removeability 50"), Stop);
        // Give Spell takes its spell out of the book
        t.run(&format!("giveability 160 {SPELL}"));
        assert_eq!(t.run("removeability 160"), OK);
        assert!(!t.plr().spellbook.contains(&SpellNo(SPELL)));
    }

    // With the (sic): as the last clause it always passes
    #[test]
    fn kw_nomonsters() {
        let mut t = world();
        assert_eq!(t.run(&format!("nomonsters {MSG_NO}:message {MSG_DO}")), OK);
        place(&mut t.w, 5, 1, "orc", HERE);
        t.out();
        assert_eq!(
            t.run(&format!("nomonsters {MSG_NO}:message {MSG_DO}")),
            Stop
        );
        assert_eq!(t.out(), "No.\r\n");
        assert_eq!(
            t.run(&format!("nomonsters {MSG_NO}")),
            OK,
            "(sic) last clause"
        );
        assert_eq!(t.out(), "");
    }

    #[test]
    fn kw_monsters() {
        let mut t = world();
        assert_eq!(t.run(&format!("monsters {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        place(&mut t.w, 5, 1, "orc", HERE);
        assert_eq!(t.run(&format!("monsters {MSG_NO}")), OK);
    }

    #[test]
    fn kw_needmonster() {
        let mut t = world();
        place(&mut t.w, 5, 1, "orc", HERE);
        assert_eq!(t.run(&format!("needmonster 1 {MSG_NO}")), OK);
        assert_eq!(t.run(&format!("needmonster 2 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
    }

    // Forced, as the original: the room's cap (0 in a cleared room) and its regeneration
    // delay after a kill do not stop it; with none made (no such monster) the line stops
    #[test]
    fn kw_summon() {
        let mut t = world();
        let key = RoomKey::new(1, HERE);
        let now = t.w.minute_stamp().max(1);
        t.w.room_state.get_mut(&key).unwrap().lastkill = now;
        assert_eq!(t.w.room(key).unwrap().maxmonsters, 0);
        assert_eq!(t.run(&format!("summon 1:message {MSG_DO}")), OK);
        let here = t.w.room_state[&key]
            .monsters
            .iter()
            .filter(|m| m.0 != 0)
            .count();
        assert_eq!(here, 1);
        assert!(t.out().ends_with("You do it.\r\n"));
        assert_eq!(t.run(&format!("summon 999:message {MSG_DO}")), Stop);
        assert_eq!(t.out(), "");
    }

    // Line 0 to the player, line 1 with his name to the room
    #[test]
    fn kw_message() {
        let mut t = world();
        assert_eq!(t.run(&format!("message {MSG_DO}")), OK);
        assert_eq!(t.out(), "You do it.\r\n");
        assert_eq!(t.out_of(t.bob), "Kang does it.\r\n<P>");
        assert_eq!(t.run("message 12345"), OK, "no such message");
        assert_eq!(t.out(), "");
    }

    #[test]
    fn kw_takeitem() {
        let mut t = world();
        t.run(&format!("giveitem {SWORD}"));
        assert_eq!(t.run(&format!("takeitem {SWORD}")), OK);
        assert!(!t.plr().items.contains(&ItemNo(SWORD)));
        assert_eq!(t.run(&format!("takeitem {SWORD} {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
    }

    #[test]
    fn kw_addexp() {
        let mut t = world();
        t.plr().exprem = 600_000_000;
        assert_eq!(t.run("addexp 1500000000"), OK);
        assert_eq!((t.plr().expbil, t.plr().exprem), (2, 100_000_000));
        assert_eq!(t.out(), "", "the clause shows nothing");
        t.plr().flags.remove(PlayerFlags::NEWEXP);
        assert_eq!(t.run("addexp 5"), OK);
        assert_eq!(
            t.plr().exprem,
            100_000_000,
            "not on the 1.11 system: nothing"
        );
    }

    // Too heavy, the item falls to the floor, the refusal is dropped
    // and the line ends with ACT_OK
    #[test]
    fn kw_giveitem() {
        let mut t = world();
        assert_eq!(t.run(&format!("giveitem {SWORD}")), OK);
        assert_eq!(t.plr().items[0], ItemNo(SWORD));
        assert_eq!(t.run(&format!("giveitem {ANVIL}:message {MSG_DO}")), OK);
        assert_eq!(t.out(), "", "no refusal, no message");
        assert_eq!(t.room(HERE).items[0].item, ItemNo(ANVIL));
        assert_eq!(t.room(HERE).items[0].uses, 3);
    }

    // Hidden in the room, and the line always ends
    #[test]
    fn kw_hideitem() {
        let mut t = world();
        assert_eq!(t.run(&format!("hideitem {SWORD}:message {MSG_DO}")), OK);
        assert_eq!(t.out(), "");
        assert_eq!(t.room(HERE).hiditems[0].item, ItemNo(SWORD));
    }

    #[test]
    fn kw_checkability() {
        let mut t = world();
        assert_eq!(t.run("checkability 50"), Stop);
        t.run("giveability 50 3");
        assert_eq!(t.run("checkability 50"), OK);
        assert_eq!(t.run("checkability 50 3"), OK);
        assert_eq!(t.run("checkability 50 4"), Stop);
    }

    #[test]
    fn kw_class_and_kw_race() {
        let mut t = world();
        assert_eq!(t.run("class 1:race 1"), OK);
        assert_eq!(t.run("class 2"), Stop);
        assert_eq!(t.run("race 2"), Stop);
    }

    #[test]
    fn kw_class() {
        kw_class_and_kw_race();
    }

    #[test]
    fn kw_race() {
        kw_class_and_kw_race();
    }

    // (evil points 10)
    #[test]
    fn kw_goodaligned() {
        let mut t = world();
        assert_eq!(t.run(&format!("goodaligned 5 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        assert_eq!(t.run("goodaligned 10"), OK);
    }

    #[test]
    fn kw_evilaligned() {
        let mut t = world();
        assert_eq!(t.run(&format!("evilaligned 20 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        assert_eq!(t.run("evilaligned 10"), OK);
    }

    // (level 10)
    #[test]
    fn kw_minlevel() {
        let mut t = world();
        assert_eq!(t.run(&format!("minlevel 11 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        assert_eq!(t.run("minlevel 10"), OK);
    }

    #[test]
    fn kw_maxlevel() {
        let mut t = world();
        assert_eq!(t.run(&format!("maxlevel 9 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        assert_eq!(t.run("maxlevel 10"), OK);
    }

    // The inventory or the key ring
    #[test]
    fn kw_checkitem() {
        let mut t = world();
        assert_eq!(t.run(&format!("checkitem {KEY} {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        t.plr().keys = vec![ItemNo(KEY)];
        assert_eq!(t.run(&format!("checkitem {KEY} {MSG_NO}")), OK);
        t.plr().items = vec![ItemNo(SWORD)];
        assert_eq!(t.run(&format!("checkitem {SWORD}")), OK);
    }

    #[test]
    fn kw_failitem() {
        let mut t = world();
        assert_eq!(t.run(&format!("failitem {SWORD} {MSG_NO}")), OK);
        t.plr().items = vec![ItemNo(SWORD)];
        assert_eq!(t.run(&format!("failitem {SWORD} {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
    }

    #[test]
    fn kw_text() {
        let mut t = world();
        block(&mut t.w, 950, "The wall glows.\r");
        assert_eq!(t.run("text 950"), OK);
        assert_eq!(t.out(), "\x1b[0;32mThe wall glows.\r\n");
        assert_eq!(t.out_of(t.bob), "");
    }

    #[test]
    fn kw_roomtext() {
        let mut t = world();
        block(&mut t.w, 950, "The ground shakes.");
        assert_eq!(t.run("roomtext 950"), OK);
        let bob = t.out_of(t.bob);
        assert_eq!(bob, "\x1b[0;32mThe ground shakes.\r\n<P>");
        assert_eq!(t.out(), bob, "the actor too");
    }

    // Visible or hidden
    #[test]
    fn kw_roomitem() {
        let mut t = world();
        assert_eq!(t.run(&format!("roomitem {SWORD} {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        t.run(&format!("hideitem {SWORD}"));
        assert_eq!(t.run(&format!("roomitem {SWORD} {MSG_NO}")), OK);
    }

    #[test]
    fn kw_failroomitem() {
        let mut t = world();
        assert_eq!(t.run(&format!("failroomitem {SWORD} {MSG_NO}")), OK);
        t.run(&format!("hideitem {SWORD}"));
        assert_eq!(t.run(&format!("failroomitem {SWORD} {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
    }

    #[test]
    fn kw_clearitem() {
        let mut t = world();
        t.run(&format!("hideitem {SWORD}"));
        t.run(&format!("giveitem {ANVIL}"));
        assert_eq!(t.run(&format!("clearitem {SWORD} {MSG_NO}")), OK);
        assert_eq!(t.room(HERE).hiditems[0].item, ItemNo(0));
        assert_eq!(t.room(HERE).hiditems[0].uses, -1);
        assert_eq!(t.run(&format!("clearitem {SWORD} {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        // item 0: everything, and it counts as found
        assert_eq!(t.run("clearitem 0"), OK);
        assert_eq!(t.room(HERE).items[0].item, ItemNo(0));
    }

    #[test]
    fn kw_addevil() {
        let mut t = world();
        assert_eq!(t.run("addevil -15"), OK);
        assert_eq!(t.plr().evil, -5);
    }

    // A negative maximum means "must not have it"
    #[test]
    fn kw_testability() {
        let mut t = world();
        assert_eq!(t.run("testability 50 0"), Stop);
        assert_eq!(t.run("testability 50 -1"), OK);
        t.run("giveability 50 3");
        assert_eq!(t.run("testability 50 2"), Stop);
        assert_eq!(t.run("testability 50 3"), OK);
    }

    #[test]
    fn kw_failability() {
        let mut t = world();
        assert_eq!(t.run(&format!("failability 50 {MSG_NO}")), OK);
        t.run("giveability 50 3");
        assert_eq!(t.run(&format!("failability 50 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
    }

    // The exit action (Door: swapped on both sides; Hidden: the blocks go, then the passage
    // opens and the room is told)
    #[test]
    fn kw_remoteaction() {
        let mut t = world();
        let room = RoomKey::new(1, DOOR_ROOM);
        let far = RoomKey::new(1, DOOR_FAR);
        assert_eq!(
            t.run(&format!(
                "remoteaction {DOOR_ROOM} {MSG_DO} 0 0:message {MSG_NO}"
            )),
            OK
        );
        assert_eq!(
            live_exit(&t.w, room, Direction::North).unwrap().par1,
            LOCK_OPEN
        );
        assert_eq!(
            live_exit(&t.w, far, Direction::South).unwrap().par1,
            LOCK_OPEN
        );
        assert_eq!(t.out(), "You do it.\r\n<P>No.\r\n");
        assert_eq!(t.out_of(t.bob), "Kang does it.\r\n<P>");
        t.run(&format!("remoteaction {DOOR_ROOM} 0 0 0"));
        assert_eq!(
            live_exit(&t.w, room, Direction::North).unwrap().par1,
            LOCK_LOCKED
        );
        // exit 11 is out of range; room 9999 does not exist: nothing at all
        t.out();
        assert_eq!(
            t.run(&format!("remoteaction {DOOR_ROOM} {MSG_DO} 0 11")),
            OK
        );
        assert_eq!(t.run(&format!("remoteaction 9999 {MSG_DO} 0 0")), OK);
        assert_eq!(t.out(), "");
        // the Hidden exit: block 1, then block 2 (any order: exitpar2s -2)
        let watcher = enter(&mut t.w, "Wat", HIDDEN_ROOM, MessageLevel::High);
        let hidden = RoomKey::new(1, HIDDEN_ROOM);
        t.run(&format!("remoteaction {HIDDEN_ROOM} 0 1 0"));
        let flags =
            ExitFlags::from_bits_retain(live_exit(&t.w, hidden, Direction::North).unwrap().par1);
        assert_eq!(flags, ExitFlags::BLOCKED2);
        assert_eq!(t.out_of(watcher), "");
        t.run(&format!("remoteaction {HIDDEN_ROOM} 0 2 0"));
        let flags =
            ExitFlags::from_bits_retain(live_exit(&t.w, hidden, Direction::North).unwrap().par1);
        assert_eq!(flags, ExitFlags::VISBLOCKED);
        insta::assert_snapshot!("concealed_passage", format!("{:?}", t.out_of(watcher)));
    }

    // One roll 0-99, the first line whose percentage is above it
    #[test]
    fn kw_random() {
        let mut t = world();
        block(
            &mut t.w,
            960,
            &format!("0:message {MSG_NO}\n101:message {MSG_DO}\n"),
        );
        assert_eq!(t.run("random 960"), OK);
        assert_eq!(t.out(), "You do it.\r\n");
        block(&mut t.w, 961, &format!("0:message {MSG_NO}\n"));
        assert_eq!(t.run("random 961"), Nothing, "no line chosen");
        assert_eq!(t.run("random 961:message 900"), OK);
        assert_eq!(t.out(), "You do it.\r\n", "the line goes on");
        block(&mut t.w, 962, &format!("100:checkitem 1 {MSG_NO}\n"));
        assert_eq!(t.run(&format!("random 962:message {MSG_DO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        // the split follows the roll
        block(
            &mut t.w,
            963,
            &format!("50:message {MSG_NO}\n100:message {MSG_DO}\n"),
        );
        let (mut lo, mut hi) = (0, 0);
        for _ in 0..200 {
            t.run("random 963");
            match t.out().as_str() {
                "No.\r\n" => lo += 1,
                "You do it.\r\n" => hi += 1,
                o => panic!("{o:?}"),
            }
        }
        assert!(lo > 60 && hi > 60, "{lo} {hi}");
    }

    // A byte, wrapping
    #[test]
    fn kw_adddelay() {
        let mut t = world();
        t.plr().delay = 250;
        assert_eq!(t.run("adddelay 10"), OK);
        assert_eq!(t.plr().delay, 4);
    }

    // Mode 0: roll 0..range plus the modifier against the stat; above it, the block runs
    // and the line stops
    #[test]
    fn kw_testskill() {
        let mut t = world();
        block(&mut t.w, 970, &format!("message {MSG_NO}"));
        assert_eq!(
            t.run(&format!("testskill agility -1000 970:message {MSG_DO}")),
            OK
        );
        assert_eq!(t.out(), "You do it.\r\n");
        assert_eq!(
            t.run(&format!("testskill agility 1000 970:message {MSG_DO}")),
            Stop
        );
        assert_eq!(t.out(), "No.\r\n");
    }

    // Mode 1; an unknown stat is 0
    #[test]
    fn kw_checkskill() {
        let mut t = world();
        assert_eq!(t.run(&format!("checkskill agility 50 {MSG_NO}")), OK);
        assert_eq!(t.run(&format!("checkskill agility 51 {MSG_NO}")), Stop);
        assert_eq!(t.out(), "No.\r\n");
        assert_eq!(t.run(&format!("checkskill strength 50 {MSG_NO}")), OK);
        assert_eq!(t.run(&format!("checkskill juggling 1 {MSG_NO}")), Stop);
        assert_eq!(t.run("checkskill juggling 0"), OK);
    }

    #[test]
    fn kw_givecoins() {
        let mut t = world();
        for line in [
            "givecoins 5 g",
            "givecoins 6",
            "givecoins 7 x",
            "givecoins 8 Runic",
            "givecoins 9 p",
            "givecoins 1 s",
        ] {
            assert_eq!(t.run(line), OK);
        }
        let c = t.plr().coins;
        assert_eq!(
            (c.runic, c.platinum, c.gold, c.silver, c.copper),
            (8, 9, 5, 1, 13)
        );
    }

    #[test]
    fn kw_test_tournament() {
        let mut t = world();
        assert_eq!(t.run("test_tournament:message 900"), Stop);
        t.w.config.tournament_play = true;
        assert_eq!(t.run("test_tournament:message 900"), OK);
    }

    #[test]
    fn kw_checkspell() {
        let mut t = world();
        block(&mut t.w, 970, &format!("message {MSG_NO}"));
        assert_eq!(
            t.run(&format!("checkspell {SPELL} 970:message {MSG_DO}")),
            Stop
        );
        assert_eq!(t.out(), "No.\r\n");
        t.plr().spells[0] = SpellNo(SPELL);
        assert_eq!(
            t.run(&format!("checkspell {SPELL} 970:message {MSG_DO}")),
            OK
        );
        assert_eq!(t.out(), "You do it.\r\n");
    }

    // (sic) The verb is read from the flag number's
    // word: only the range test runs, and no flag ever changes.
    #[test]
    fn kw_flag() {
        let mut t = world();
        assert_eq!(t.run("flag 0 set"), Stop);
        assert_eq!(t.run("flag 65 set"), Stop);
        assert_eq!(t.run(&format!("flag 5 set:message {MSG_DO}")), OK);
        assert_eq!(
            t.run(&format!("flag 40 check {MSG_NO}:message {MSG_DO}")),
            OK
        );
        assert_eq!((t.plr().qflags, t.plr().qflags2), (0, 0));
        assert_eq!(t.out(), "You do it.\r\nYou do it.\r\n");
        assert!(matches!(
            Clause::parse("flag 5 clear 77", &STOCK_TELEPORTS),
            Clause::Flag {
                flag: 5,
                msg: 77,
                ..
            }
        ));
    }

    #[test]
    fn kw_learnspell() {
        let mut t = world();
        assert_eq!(t.run(&format!("learnspell {SPELL}")), OK);
        assert!(t.plr().spellbook.contains(&SpellNo(SPELL)));
        assert_eq!(t.run(&format!("learnspell {SPELL}")), OK, "known: silent");
        assert_eq!(t.run("learnspell 0"), Stop);
        assert_eq!(t.run("learnspell 99"), Stop);
        insta::assert_snapshot!("learn_spell", format!("{:?}", t.out()));
        // a spell above the player's level
        t.w.spells.get_mut(&SpellNo(SPELL)).unwrap().level = 50;
        t.plr().spellbook.fill(SpellNo(0));
        assert_eq!(t.run(&format!("learnspell {SPELL}")), Stop);
        assert_eq!(t.out(), "You don't know what to do with this!\r\n");
    }

    // A clause with no keyword is skipped and the line goes on
    #[test]
    fn unknown_keywords_are_skipped() {
        let mut t = world();
        assert_eq!(t.run("levelcheck 5:message 900"), OK);
        assert_eq!(t.run("levelcheck 5"), Nothing);
        assert_eq!(t.run(""), Nothing);
        assert_eq!(t.out(), "You do it.\r\n");
    }

    // -----------------------------------------------------------------------------------
    // The runners.

    // The matching lines run until one runs through; nothing matched is 0
    #[test]
    fn special_command_runs_matching_lines_in_turn() {
        let mut t = world();
        block(
            &mut t.w,
            980,
            &format!(
                "pull lever:message {MSG_DO}\npush button:checkitem 1 {MSG_NO}\npush button:message {MSG_DO}\n"
            ),
        );
        let me = t.me;
        assert_eq!(
            perform_special_command(&mut t.w, me, TextBlockNo(980), "PUSH BUTTON"),
            OK
        );
        assert_eq!(t.out(), "No.\r\nYou do it.\r\n");
        assert_eq!(
            perform_special_command(&mut t.w, me, TextBlockNo(980), "dance"),
            Nothing
        );
        assert_eq!(
            perform_special_command(&mut t.w, me, TextBlockNo(980), "push"),
            Nothing
        );
        // sysop debugging shows each test
        t.w.session_mut(me).unwrap().flags.debug = DebugLevel::On;
        perform_special_command(&mut t.w, me, TextBlockNo(980), "pull lever");
        assert_eq!(
            t.out(),
            "Checking for pull lever in pull lever\r\nYou do it.\r\n"
        );
    }

    // At most 50 records a walk
    #[test]
    fn a_walk_reads_at_most_50_records() {
        let mut t = world();
        for part in 0..60i16 {
            let text = if part == 55 {
                "magic:message 900\n"
            } else {
                "x:message 901\n"
            };
            t.w.text_blocks.insert(
                (part, TextBlockNo(990)),
                TextBlock {
                    part,
                    number: TextBlockNo(990),
                    link: TextBlockNo(0),
                    text: text.into(),
                },
            );
        }
        let me = t.me;
        assert_eq!(
            perform_special_command(&mut t.w, me, TextBlockNo(990), "magic"),
            Nothing
        );
        t.w.text_blocks
            .get_mut(&(49, TextBlockNo(990)))
            .unwrap()
            .text = "magic:message 900\n".into();
        assert_eq!(
            perform_special_command(&mut t.w, me, TextBlockNo(990), "magic"),
            OK
        );
    }

    // Every line until one runs through; a line runs only while a
    // ':' follows its start
    #[test]
    fn a_whole_block_runs_line_by_line() {
        let mut t = world();
        let me = t.me;
        block(
            &mut t.w,
            991,
            &format!("checkitem 1 {MSG_NO}\nmessage {MSG_DO}:adddelay 0\nmessage {MSG_NO}\n"),
        );
        assert_eq!(run_text_block(&mut t.w, me, TextBlockNo(991)), OK);
        assert_eq!(t.out(), "No.\r\nYou do it.\r\n");
        block(
            &mut t.w,
            992,
            &format!("checkitem 1 {MSG_NO}\nmessage {MSG_DO}\n"),
        );
        assert_eq!(run_text_block(&mut t.w, me, TextBlockNo(992)), Stop);
        assert_eq!(t.out(), "No.\r\n", "the second line has no ':' after it");
        t.w.session_mut(me).unwrap().flags.debug = DebugLevel::On;
        run_text_block(&mut t.w, me, TextBlockNo(991));
        assert_eq!(
            t.out(),
            "Line 1:checkitem 1 901\r\nNo.\r\nLine 2:message 900:adddelay 0\r\nYou do it.\r\n"
        );
    }

    // A block that runs itself is cut, not a stack overflow.
    #[test]
    fn a_block_that_runs_itself_is_cut() {
        let mut t = world();
        let me = t.me;
        block(&mut t.w, 993, "checkspell 1 993\n");
        assert_eq!(run_text_block(&mut t.w, me, TextBlockNo(993)), Stop);
    }

    // Stage 8 of the input; the prompt follows
    #[test]
    fn the_room_command_text_takes_a_typed_line() {
        let mut t = world();
        block(&mut t.w, 980, &format!("pull lever:message {MSG_DO}\n"));
        t.w.rooms.get_mut(&RoomKey::new(1, HERE)).unwrap().cmdtext = TextBlockNo(980);
        let me = t.me;
        execute_input(&mut t.w, me, "pull lever", false);
        assert!(t.out().ends_with("You do it.\r\n<P>"));
        // LOOK at something not here reaches it too
        block(&mut t.w, 980, &format!("look lever:message {MSG_DO}\n"));
        execute_input(&mut t.w, me, "look lever", false);
        assert!(t.out().ends_with("You do it.\r\n<P>"));
    }

    // -----------------------------------------------------------------------------------
    // The parser and the wildcard table.

    // Stock patterns (none uses '*' or '|') and the forms the original describes
    #[test]
    fn wildcard_table() {
        let rows: [(&str, &str, bool); 30] = [
            ("borrow skiff", "borrow skiff", true),
            ("borrow skiff", "BORROW SKIFF", true),
            ("borrow skiff", "borrow", false),
            ("borrow skiff", "borrow skiff now", false),
            ("pull lever", "pull lever", true),
            ("pull lever", "pull  lever", false),
            ("dig grave", "dig grave", true),
            ("dig grave", "dig", false),
            ("climb tree", "Climb Tree", true),
            ("go grey portal", "go grey portal", true),
            ("go grey portal", "go portal", false),
            ("give crane totem to kuel", "give crane totem to kuel", true),
            (
                "put three keys in gleaming steel device",
                "put three keys in gleaming steel device",
                true,
            ),
            ("place eye in socket", "place eye", false),
            ("push v", "push v", true),
            ("push v", "push", false),
            ("read thin grey book", "read thin grey book", true),
            ("evil3", "EVIL3", true),
            ("learnspell", "learnspell", true),
            ("say time", "say time", true),
            ("lift lid", "lift lid", true),
            // The described forms: '|' alternatives, '*' abbreviation
            ("pull lever|yank lever", "yank lever", true),
            ("pull lever|yank lever", "yank", false),
            ("pul*l lever", "pul lev", true),
            ("pul*l lever", "pull lever", true),
            ("pul*l lever", "pu lever", false),
            ("get *sword", "get sw", true),
            ("get*", "get", true),
            ("get*", "get sword", false),
            ("*pull lever|*yank lever", "yank", true),
        ];
        for (p, typed, want) in rows {
            assert_eq!(wildcard_match(p, typed), want, "{p:?} vs {typed:?}");
        }
    }

    // How a record splits into lines
    #[test]
    fn records_split_into_lines() {
        let l = pattern_lines("a:b\nc d:e:f\nno colon\nx:y");
        let pairs: Vec<(&str, &str)> = l
            .iter()
            .map(|p| (p.pattern.as_str(), p.action.as_str()))
            .collect();
        assert_eq!(pairs, [("a", "b"), ("c d", "e:f"), ("no colon\nx", "y")]);
        assert!(pattern_lines("just text\n").is_empty());
        assert_eq!(whole_lines("a\nb:c\nd\n"), ["a", "b:c"]);
        assert_eq!(whole_lines("text 5"), ["text 5"]);
    }
}
