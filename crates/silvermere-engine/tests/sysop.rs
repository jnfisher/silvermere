//! SYSOP (T-055) through the whole input pipeline: `cmd_sysop`, its gates per role and key, the
//! reports, the `[MAJORMUD Sysop]` menu and the audit events. The tests sit in `mod sysop` so
//! `cargo test -p silvermere-engine sysop::` runs them.

use crate::common;

mod sysop {
    use std::sync::mpsc::Receiver;

    use super::common::{TestClock, enter, special_rooms, test_config, tiny_snapshot};
    use silvermere_engine::commands::sysop::{Disposition, KEYWORDS, Key, Keys};
    use silvermere_engine::player_session::AccountRef;
    use silvermere_engine::world::blank;
    use silvermere_engine::{
        Engine, EngineInput, EngineOutput, GameEvent, SeededRng, SessionId, World,
    };
    use silvermere_text::strip_ansi_str;
    use silvermere_types::enums::OwnerKind;
    use silvermere_types::enums::{MessageLevel, RoomType, SessionState, SysopStep};
    use silvermere_types::flags::{GangFlags, PlayerSettings};
    use silvermere_types::ids::{
        AccountId, ActiveMonsterId, ItemNo, MapId, RoomId, RoomKey, SessionId as Sid, TextBlockNo,
    };
    use silvermere_types::records::{Character, EvilRecord, Gang, ItemOwnership, TextBlock};

    const NEWHAVEN: u32 = 2150;
    const SUPPORT: u32 = 164;
    const COLLISEUM: u32 = 3000;

    struct Harness {
        engine: Engine<TestClock, SeededRng>,
        rx: Receiver<EngineOutput>,
        log: Vec<EngineOutput>,
        next_account: i64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Role {
        Player,
        GameOp,
        Sysop,
    }

    /// Every key, the developer gate included.
    fn all_keys() -> Keys {
        Key::ALL.iter().copied().collect()
    }

    fn world() -> World {
        crate::common::install_text();
        let mut snap = tiny_snapshot();
        snap.items[0].name = "Ancient Sword".into();
        snap.items[0].limit = 1;
        for n in [NEWHAVEN, SUPPORT] {
            let mut r = blank::clear_room(RoomKey::new(1, n));
            r.name = format!("Room {n}");
            snap.rooms.push(r);
        }
        let mut c = blank::clear_room(RoomKey::new(1, COLLISEUM));
        c.room_type = RoomType::Colliseum;
        snap.rooms.push(c);
        snap.gangs.push(Gang {
            key: "OLD GUARD".into(),
            name: "Old Guard".into(),
            exp: 0,
            leader: "Bob".into(),
            created: 0,
            members: 3,
            flags: GangFlags::empty(),
            overexp: 0,
            overflows: 0,
        });
        World::load(test_config(), 8, snap).expect("world loads")
    }

    impl Harness {
        fn new() -> Self {
            let (tx, rx) = std::sync::mpsc::channel();
            let engine = Engine::new(world(), TestClock::new(), SeededRng::new(7), tx);
            Self {
                engine,
                rx,
                log: Vec::new(),
                next_account: 1,
            }
        }

        /// A playing character with an account of `role`, the role's default keys.
        fn who(&mut self, name: &str, role: Role) -> SessionId {
            let keys = Keys::for_roles(role == Role::Sysop, role == Role::GameOp);
            self.with_keys(name, role, keys)
        }

        fn with_keys(&mut self, name: &str, role: Role, keys: Keys) -> SessionId {
            let w = &mut self.engine.world;
            let id = enter(w, name, special_rooms()[0], MessageLevel::High);
            let s = w.session_mut(id).unwrap();
            s.flags.sysop = role == Role::Sysop;
            s.flags.gameop = role == Role::GameOp;
            s.sysop_keys = keys;
            s.account = Some(AccountRef {
                id: AccountId(self.next_account),
                username: name.to_ascii_uppercase(),
                sysop: role == Role::Sysop,
                gameop: role == Role::GameOp,
                sex: b'M',
            });
            self.next_account += 1;
            let c = s.character.as_mut().unwrap();
            (c.hp, c.maxhp) = (40, 40);
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

        fn pump(&mut self) -> usize {
            let from = self.log.len();
            while let Ok(o) = self.rx.try_recv() {
                self.log.push(o);
            }
            from
        }

        fn type_line(&mut self, id: SessionId, text: &str) -> usize {
            let from = self.pump();
            self.engine.handle(EngineInput::Line {
                id,
                text: text.into(),
            });
            self.pump();
            from
        }

        fn raw_since(&self, from: usize, id: SessionId) -> String {
            let b: Vec<u8> = self.log[from..]
                .iter()
                .filter_map(|o| match o {
                    EngineOutput::Write { id: i, bytes } if *i == id => Some(bytes.as_slice()),
                    _ => None,
                })
                .flatten()
                .copied()
                .collect();
            String::from_utf8_lossy(&b).into_owned()
        }

        fn got(&self, from: usize, id: SessionId) -> String {
            strip_ansi_str(&self.raw_since(from, id)).replace("\r\n", "\n")
        }

        fn line(&mut self, id: SessionId, text: &str) -> String {
            let from = self.type_line(id, text);
            self.got(from, id)
        }

        fn events_since(&self, from: usize) -> Vec<GameEvent> {
            self.log[from..]
                .iter()
                .filter_map(|o| match o {
                    EngineOutput::Event(e) => Some(e.clone()),
                    _ => None,
                })
                .collect()
        }

        /// One fast tick, answering a roster request as `bin` would.
        fn tick(&mut self) -> usize {
            let from = self.pump();
            self.engine.handle(EngineInput::Tick);
            self.pump();
            from
        }

        /// Answers every `Roster` request since `from` with `chars`.
        fn answer_roster(&mut self, from: usize, chars: Vec<(AccountId, Character)>) {
            let ids: Vec<SessionId> = self.log[from..]
                .iter()
                .filter_map(|o| match o {
                    EngineOutput::Roster { id } => Some(*id),
                    _ => None,
                })
                .collect();
            for id in ids {
                let chars = chars.clone();
                self.engine.handle(EngineInput::Custom {
                    session: Some(id),
                    run: Box::new(move |w| {
                        silvermere_engine::state::sysop_menu::on_roster(w, id, chars);
                    }),
                });
            }
            self.pump();
        }

        fn room(&self, id: SessionId) -> u32 {
            self.plr(id).room.0
        }
    }

    const P: &str = "[HP=40]:";

    // -------------------------------------------------------------------------------
    // The keyword table (§2 of the card)

    /// Every keyword of the card's table, with the dropped ones.
    const CARD_KEYWORDS: [&str; 29] = [
        "lance",
        "debug",
        "monster",
        "dump",
        "reset",
        "buffers",
        "reload",
        "initialize",
        "summon",
        "express_train",
        "goto",
        "lightning",
        "transfer",
        "values",
        "status",
        "disband",
        "speak",
        "arena",
        "disable",
        "enable",
        "gangsize",
        "god",
        "EMULATE",
        "NOEMULATE",
        "list",
        "report",
        "map",
        "configure",
        "tweak",
    ];

    // every keyword of the table maps to a handler or to Dropped with a reason
    #[test]
    fn every_keyword_is_kept_or_dropped() {
        for kw in CARD_KEYWORDS {
            let d = KEYWORDS
                .iter()
                .find(|(k, _)| *k == kw)
                .unwrap_or_else(|| panic!("{kw} not in KEYWORDS"));
            match d.1 {
                Disposition::Kept(gate) => assert!(!gate.is_empty()),
                Disposition::Dropped(why) => assert!(!why.is_empty(), "{kw}"),
            }
        }
        let dropped: Vec<&str> = KEYWORDS
            .iter()
            .filter(|(_, d)| matches!(d, Disposition::Dropped(_)))
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(
            dropped,
            [
                "lance",
                "initialize",
                "EMULATE",
                "NOEMULATE",
                "menu U",
                "menu B",
                "menu E",
                "menu A"
            ]
        );
    }

    // a kept keyword reaches its handler for a sysop; a dropped one is not recognised
    #[test]
    fn kept_keywords_have_handlers_and_dropped_ones_do_not() {
        for (kw, d) in KEYWORDS.iter().filter(|(k, _)| !k.starts_with("menu ")) {
            let mut h = Harness::new();
            let s = h.with_keys("Sid", Role::Sysop, all_keys());
            let out = h.line(s, &typed(kw));
            let refused = out.contains("Command not recognized.");
            match d {
                Disposition::Kept(_) => assert!(!refused, "{kw}: {out:?}"),
                Disposition::Dropped(_) => assert!(refused, "{kw}: {out:?}"),
            }
        }
    }

    // -------------------------------------------------------------------------------
    // The permission matrix: who may use each sub-command (§2)

    /// Who may use a top-level sub-command: the sysop flag required, the key, and
    /// DISBAND's "sysop, or a game operator holding the key".
    fn gate(kw: &str) -> (bool, Option<Key>) {
        match kw {
            "debug" | "monster" | "dump" | "summon" | "express_train" | "values" | "tweak" => {
                (true, Some(Key::Debug))
            }
            "reset" => (true, Some(Key::SysResetRooms)),
            "buffers" => (false, Some(Key::SysBuffers)),
            "goto" => (false, Some(Key::SysGoto)),
            "lightning" => (false, Some(Key::SysLightning)),
            "transfer" | "arena" | "gangsize" | "configure" => (true, None),
            "status" => (false, Some(Key::SysStatus)),
            "speak" => (false, Some(Key::SysSpeak)),
            "disable" | "enable" => (true, Some(Key::SysDisableGangs)),
            "report" => (false, Some(Key::SysReport)),
            "map" => (false, Some(Key::SysMap)),
            _ => (false, None),
        }
    }

    /// The line typed for a keyword: arguments where the gate needs them (more than two words).
    fn typed(kw: &str) -> String {
        match kw {
            "summon" => "sysop summon nothing".into(),
            "express_train" => format!("sysop express_train {}", special_rooms()[0]),
            "goto" => "sysop goto nowhere".into(),
            "disband" | "disable" | "enable" => format!("sysop {kw} Nobody"),
            _ => format!("sysop {kw}"),
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Cell {
        /// `Handled::NotCommand`: the line went on through `execute_input` (exits, spells, speech).
        Speech,
        /// The chain fell through.
        NotRecognized,
        /// Handled.
        Ran,
    }

    fn outcome(out: &str) -> Cell {
        // not a command: execute_input goes on to exits, spells and speech; none takes the word
        if out.contains("Your command had no effect.") {
            Cell::Speech
        } else if out.contains("Command not recognized.") {
            Cell::NotRecognized
        } else {
            Cell::Ran
        }
    }

    fn expected(kw: &str, role: Role, keys: Keys) -> Cell {
        if role == Role::Player {
            return Cell::Speech;
        }
        let sysop = role == Role::Sysop;
        let ok = match kw {
            "disband" => sysop || keys.contains(Key::SysDisband),
            _ => {
                let (needs_sysop, key) = gate(kw);
                (!needs_sysop || sysop) && key.is_none_or(|k| keys.contains(k))
            }
        };
        if ok { Cell::Ran } else { Cell::NotRecognized }
    }

    #[test]
    fn permission_matrix() {
        let kept: Vec<&str> = KEYWORDS
            .iter()
            .filter(|(k, d)| !k.starts_with("menu ") && matches!(d, Disposition::Kept(_)))
            .map(|(k, _)| *k)
            .collect();
        let mut rows: Vec<(Role, Keys, String)> = vec![
            (Role::Player, Keys::empty(), "player".into()),
            (Role::GameOp, Keys::for_roles(false, true), "gameop".into()),
            (Role::Sysop, Keys::for_roles(true, false), "sysop".into()),
            (Role::Sysop, all_keys(), "sysop + DEBUG".into()),
        ];
        for k in Key::ALL {
            let mut keys = all_keys();
            keys.remove(*k);
            rows.push((Role::Sysop, keys, format!("sysop - {}", k.ini_name())));
        }
        for (role, keys, label) in rows {
            for kw in &kept {
                let mut h = Harness::new();
                let id = h.with_keys("Sid", role, keys);
                let out = h.line(id, &typed(kw));
                assert_eq!(
                    outcome(&out),
                    expected(kw, role, keys),
                    "{label} / {kw}: {out:?}"
                );
            }
        }
    }

    // the gameop's subset with the default keys: exactly these, the rest refused
    #[test]
    fn gameop_subset() {
        let keys = Keys::for_roles(false, true);
        let ran: Vec<&str> = KEYWORDS
            .iter()
            .filter(|(k, d)| !k.starts_with("menu ") && matches!(d, Disposition::Kept(_)))
            .map(|(k, _)| *k)
            .filter(|kw| expected(kw, Role::GameOp, keys) == Cell::Ran)
            .collect();
        assert_eq!(
            ran,
            [
                "buffers",
                "reload",
                "goto",
                "lightning",
                "status",
                "speak",
                "god",
                "list",
                "report",
                "map"
            ]
        );
    }

    // STATUS and GOD: get_user_number answers UserMatch::Several and SYSOP counts as handled,
    // so the dispatcher's ANSI_NORMAL and the list go out with the prompt
    #[test]
    fn several_matches_answer_with_the_reset_and_the_list() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        h.who("Bo", Role::Player);
        h.who("Bob", Role::Player);
        let prompt =
            String::from_utf8(silvermere_engine::tell::prompt_bytes(&h.engine.world, s)).unwrap();
        let want = format!(
            "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;31m\
             Please be more specific.  You could have meant any of these:\r\n\
             \x1b[0;37m-- Bo\r\n-- Bob\r\n{prompt}"
        );
        for line in ["sysop status b", "sysop god b dance"] {
            let from = h.type_line(s, line);
            assert_eq!(h.raw_since(from, s), want, "{line}");
        }
    }

    // a GOD option's key revoked: the option's "No ... done."
    #[test]
    fn god_option_key_revoked() {
        let mut h = Harness::new();
        let mut keys = Keys::for_roles(true, false);
        keys.remove(Key::SysAddLife);
        let s = h.with_keys("Sid", Role::Sysop, keys);
        let bob = h.who("Bob", Role::Player);
        h.plr_mut(bob).lives = 3;
        assert_eq!(
            h.line(s, "sysop god bob add life"),
            format!("No add command done.\n{P}")
        );
        assert_eq!(h.plr(bob).lives, 3);
    }

    // LANCE answers a player too, then the word is speech
    #[test]
    fn lance_for_a_player_and_an_operator() {
        let mut h = Harness::new();
        let p = h.who("Pat", Role::Player);
        let out = h.line(p, "sysop lance");
        assert!(out.starts_with("Lcon 8\nDeath Setting: "), "{out:?}");
        assert!(out.contains("Your command had no effect."), "{out:?}");
        let s = h.who("Sid", Role::Sysop);
        let out = h.line(s, "sysop lance");
        assert!(out.contains("Lcon 8\n"));
        assert!(out.contains("Command not recognized."), "(sic) {out:?}");
        assert!(!out.contains("Reg#"));
    }

    // -------------------------------------------------------------------------------
    // Snapshots

    fn snap(text: &str) -> String {
        strip_ansi_str(text).replace("\r\n", "\n")
    }

    #[test]
    fn snapshot_help_lists() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let g = h.who("Gil", Role::GameOp);
        insta::assert_snapshot!(
            "help_sysop",
            crate::common::redact(&snap(&h.line(s, "sysop")))
        );
        insta::assert_snapshot!(
            "help_gameop",
            crate::common::redact(&snap(&h.line(g, "sysop")))
        );
    }

    #[test]
    fn snapshot_texts() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let _bob = h.who("Bob", Role::Player);
        insta::assert_snapshot!("goto_locations", h.line(s, "sysop goto nowhere"));
        insta::assert_snapshot!("god_syntax", crate::common::redact(&h.line(s, "sysop god")));
        insta::assert_snapshot!(
            "god_valid_options",
            crate::common::redact(&h.line(s, "sysop god bob dance"))
        );
        insta::assert_snapshot!("valid_lists", h.line(s, "sysop list"));
        let dev = h.with_keys("Dev", Role::Sysop, all_keys());
        insta::assert_snapshot!("valid_lists_dev", h.line(dev, "sysop list"));
        insta::assert_snapshot!(
            "valid_edits",
            crate::common::redact(&h.line(dev, "sysop tweak"))
        );
        insta::assert_snapshot!(
            "name_change",
            crate::common::redact(&h.line(s, "sysop god bob change name Robert"))
        );
    }

    #[test]
    fn snapshot_menu() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        h.engine.world.session_mut(s).unwrap().state = SessionState::MainMenu;
        let from = h.type_line(s, "sysop");
        insta::assert_snapshot!("menu", crate::common::redact(&h.raw_since(from, s)));
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::SysopMenu(SysopStep::Menu)
        );
    }

    // -------------------------------------------------------------------------------
    // GOTO

    #[test]
    fn goto_silvermere_arrives_silently() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let out = h.line(s, "sysop goto silvermere");
        assert_eq!(out, P, "no message on success");
        assert_eq!(h.room(s), special_rooms()[1], "reincroom");
        h.line(s, "sysop goto newhaven");
        assert_eq!(h.room(s), NEWHAVEN);
        h.line(s, "sysop goto sup");
        assert_eq!(h.room(s), SUPPORT);
    }

    // SUPPORT is the sysop's alone
    #[test]
    fn goto_support_is_sysop_only() {
        let mut h = Harness::new();
        let g = h.who("Gil", Role::GameOp);
        let out = h.line(g, "sysop goto support");
        assert!(out.starts_with("Current GOTO locations"), "{out:?}");
        assert_eq!(h.room(g), special_rooms()[0]);
    }

    // the add-on towns go without the add-on test; a missing room keeps the player
    #[test]
    fn goto_addon_town_has_no_purchase_gate() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let out = h.line(s, "sysop goto rhudaur");
        assert!(!out.contains("purchase"), "{out:?}");
    }

    // the developer gate is off for a stock sysop (CODEC_OF_TWINKS unset)
    #[test]
    fn developer_commands_need_the_debug_grant() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        for line in [
            "sysop tweak",
            "sysop debug",
            "sysop express_train 1",
            "sysop summon x",
        ] {
            assert_eq!(
                h.line(s, line),
                format!("Command not recognized.\n{P}"),
                "{line}"
            );
        }
    }

    // SYSOP CONFIGURE SHOW, the whole block as the wire carries it (ANSI and
    // CRLF included). Recorded divergences (docs/differences.md): the three credit lines
    // are not printed, pollrate/initrate/Logfile print 0, 0 and `tracing`, and the
    // developer pair shows for the DEBUG grant. `Logfile: %s\r\n` ends in one
    // line end, not two: the host turns `\r\n` into one CRLF (`normalise_line_ends`), and the
    // original's live output has no blank line before the prompt either (m4 sysop step 25).
    #[test]
    fn configure_show_whole_output() {
        const COLOUR: &str = "\u{1b}[0;37;40m";
        // the prompt redraw that follows every answer
        const PROMPT: &str = "\u{1b}[79D\u{1b}[K\u{1b}[0;37m[HP=40\u{1b}[0;37m]:";
        let mut h = Harness::new();
        let dev = h.with_keys("Dev", Role::Sysop, all_keys());
        let (secs, mins, max) = {
            let w = &h.engine.world;
            (w.config.sec_create, w.config.regen_mins, w.max_in_game)
        };
        let from = h.type_line(dev, "sysop configure show");
        assert_eq!(
            h.raw_since(from, dev),
            format!(
                "{COLOUR}Current Configuration Settings:\r\n\
                 Monster Generation Rate:     {secs} seconds\r\n\
                 Minutes before regenerating: {mins} minutes\r\n\
                 Maximum users in game:       {max}\r\n\
                 Current pollrate:            0\r\n\
                 Current initrate:            0\r\n\
                 Logfile: tracing\r\n{PROMPT}"
            )
        );
        // without the grant: no developer pair
        let s = h.who("Sid", Role::Sysop);
        let from = h.type_line(s, "sysop configure show");
        assert_eq!(
            h.raw_since(from, s),
            format!(
                "{COLOUR}Current Configuration Settings:\r\n\
                 Maximum users in game:       {max}\r\n\
                 Current pollrate:            0\r\n\
                 Current initrate:            0\r\n\
                 Logfile: tracing\r\n{PROMPT}"
            )
        );
    }

    // SYSOP MONSTER calls generate_monster forced, so the
    // room's spawn rules do not stop it: neither its monster limit nor
    // the regeneration delay after a kill there
    #[test]
    fn sysop_monster_is_forced_past_the_room_limit_and_the_regen_delay() {
        let mut h = Harness::new();
        let s = h.with_keys("Sid", Role::Sysop, all_keys());
        let here = RoomKey::new(1, h.room(s));
        let w = &mut h.engine.world;
        let max = w.room(here).unwrap().maxmonsters;
        let now = w.minute_stamp();
        let rs = w.room_state.get_mut(&here).unwrap();
        rs.nummonsters = max;
        rs.lastkill = now.max(1);
        let before = h.engine.world.active_monsters.len();
        h.line(s, "sysop monster 1");
        assert_eq!(h.engine.world.active_monsters.len(), before + 1);
    }

    /// A combat state for `goto_with` until T-034 fills `movement::CombatHooks`.
    struct Combat {
        auto: bool,
        attacked: bool,
        retaliation: bool,
    }

    impl silvermere_engine::movement::CombatHooks for Combat {
        fn in_autocombat(&self, _w: &World, _id: Sid) -> bool {
            self.auto
        }
        fn being_attacked(&self, _w: &World, _id: Sid) -> bool {
            self.attacked
        }
        fn in_retaliation(&self, _w: &World, _id: Sid) -> bool {
            self.retaliation
        }
    }

    // autocombat, being attacked, a retaliation window; no move
    #[test]
    fn goto_refusals_with_a_combat_stub() {
        use silvermere_engine::commands::CommandLine;
        use silvermere_engine::commands::sysop::goto_with;
        let cases = [
            (
                Combat {
                    auto: true,
                    attacked: false,
                    retaliation: false,
                },
                "You may not be in autocombat when you do that.\n",
            ),
            (
                Combat {
                    auto: false,
                    attacked: true,
                    retaliation: false,
                },
                "You may not be in autocombat when you do that.\n",
            ),
            (
                Combat {
                    auto: false,
                    attacked: false,
                    retaliation: true,
                },
                "You may not teleport during a PVP retaliation period.\n",
            ),
        ];
        for (combat, text) in cases {
            let mut h = Harness::new();
            let s = h.who("Sid", Role::Sysop);
            let w = &mut h.engine.world;
            goto_with(
                w,
                s,
                true,
                &CommandLine::parse("sysop goto newhaven"),
                &combat,
            );
            let bytes = w
                .session(s)
                .unwrap()
                .output
                .render(silvermere_text::colour::Palette::default(), b"");
            let out = strip_ansi_str(&String::from_utf8_lossy(&bytes)).replace("\r\n", "\n");
            assert_eq!(out, text);
            assert_eq!(h.room(s), special_rooms()[0], "did not move");
        }
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let calm = Combat {
            auto: false,
            attacked: false,
            retaliation: false,
        };
        let w = &mut h.engine.world;
        goto_with(
            w,
            s,
            true,
            &CommandLine::parse("sysop goto newhaven"),
            &calm,
        );
        assert_eq!(h.room(s), NEWHAVEN);
    }

    /// An active monster named `rat` (known monster 1) in `room`, in the room's list.
    fn rat(h: &mut Harness, room: u32) {
        use silvermere_types::enums::GroupRole;
        use silvermere_types::flags::ActiveMonsterFlags;
        use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins};
        let w = &mut h.engine.world;
        let k = w
            .monsters
            .get(&silvermere_types::ids::MonsterNo(1))
            .unwrap()
            .clone();
        let number = ActiveMonsterId(1);
        w.active_monsters.insert(ActiveMonster {
            number,
            knownnum: k.number,
            level: 1,
            map: MapId(1),
            room: RoomId(room),
            poison: 0,
            energy: 1000,
            hp: 500,
            following: String::new(),
            trail: std::array::from_fn(|_| RoomId::default()),
            hunters: std::array::from_fn(|_| ActiveMonsterId::default()),
            hunting: ActiveMonsterId::default(),
            name: "rat".into(),
            wornitem: ItemNo::default(),
            weapon: k.weapon,
            items: std::array::from_fn(|_| (ItemNo::default(), 0)),
            cash: Coins::default(),
            maxhp: 500,
            alignment: k.alignment,
            followpct: 0,
            dr: 0,
            ac: 0,
            exp: 1,
            maxenergy: 1000,
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
    }

    // through T-034's real auto-combat: the sysop attacks a monster
    // (`*Combat Engaged*`), then GOTO is refused and he stays
    #[test]
    fn goto_refused_in_autocombat() {
        let mut h = Harness::new();
        let start = special_rooms()[0];
        rat(&mut h, start);
        let s = h.who("Sid", Role::Sysop);
        let out = h.line(s, "a rat");
        assert!(out.contains("*Combat Engaged*"), "{out:?}");
        assert!(silvermere_engine::combat::autocombat::is_inside_autocombat(
            &h.engine.world,
            s
        ));
        let out = h.line(s, "sysop goto silvermere");
        assert!(
            out.starts_with("You may not be in autocombat when you do that.\n"),
            "{out:?}"
        );
        assert_eq!(h.room(s), start, "did not move");
    }

    // -------------------------------------------------------------------------------
    // GOD, LIGHTNING and the audit events

    fn sysop_events(h: &Harness, from: usize) -> Vec<(&'static str, String)> {
        h.events_since(from)
            .into_iter()
            .filter_map(|e| match e {
                GameEvent::Sysop { kind, user, .. } => Some((kind, user)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn god_add_life() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let bob = h.who("Bob", Role::Player);
        h.plr_mut(bob).lives = 3;
        let from = h.type_line(s, "sysop god bob add life");
        assert_eq!(h.got(from, s), format!("Done\n{P}"));
        assert_eq!(h.plr(bob).lives, 4);
        assert_eq!(sysop_events(&h, from), [("sysop.god", "SID".to_string())]);
        // not above 9: nothing at all
        h.plr_mut(bob).lives = 9;
        assert_eq!(h.line(s, "sysop god bob add life"), P);
        assert_eq!(h.plr(bob).lives, 9);
    }

    #[test]
    fn lightning_strikes_with_both_lines() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let bob = h.who("Bob", Role::Player);
        let ann = h.who("Ann", Role::Player);
        {
            let b = h.plr_mut(bob);
            b.evil = 250; // legal level 5
            (b.hp, b.maxhp) = (1, 40);
        }
        let from = h.type_line(s, "sysop lightning bob");
        let you = h.got(from, bob);
        assert!(
            you.starts_with("A bolt of lightning from the heavens strikes you for "),
            "{you:?}"
        );
        let dmg: i16 = you["A bolt of lightning from the heavens strikes you for ".len()..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!((10..=20).contains(&dmg), "25% to 50% of 40: {dmg}");
        assert_eq!(h.plr(bob).hp, 1 - dmg);
        let room = h.got(from, ann);
        assert!(
            room.starts_with("A bolt of lightning just struck Bob!\n"),
            "{room:?}"
        );
        assert!(
            room.contains("Bob drops to the ground!"),
            "knocked out: {room:?}"
        );
        assert!(h.got(from, s).ends_with(&format!("Done\n{P}")));
        assert_eq!(
            sysop_events(&h, from),
            [("sysop.lightning", "SID".to_string())]
        );
    }

    // a victim who is not evil enough
    #[test]
    fn lightning_refused_for_a_good_player() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let _bob = h.who("Bob", Role::Player);
        h.engine.world.config.light_evy = false;
        assert_eq!(
            h.line(s, "sysop lightning bob"),
            format!("You may not lightning that user.\n{P}")
        );
    }

    #[test]
    fn gang_commands_audit_once_each() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let from = h.type_line(s, "sysop gangsize 5 Old Guard");
        assert_eq!(
            h.got(from, s),
            format!("Gang Old Guard has been resized to 5.\n{P}")
        );
        assert_eq!(
            sysop_events(&h, from),
            [("sysop.gangsize", "SID".to_string())]
        );
        let from = h.type_line(s, "sysop disband Old Guard");
        assert_eq!(
            h.got(from, s),
            format!("Gang Old Guard has been disbanded and will be deleted at cleanup.\n{P}")
        );
        assert_eq!(
            sysop_events(&h, from),
            [("sysop.disband", "SID".to_string())]
        );
        let g = &h.engine.world.gangs["OLD GUARD"];
        assert!(g.flags.contains(GangFlags::DISBANDED | GangFlags::DELETE));
        assert_eq!(g.members, 5);
        assert_eq!(
            h.line(s, "sysop disband Nobody"),
            format!("Gang Nobody not found.\n{P}")
        );
    }

    // LIST GANG <gang>: T-044's paged member list, no prompt yet
    #[test]
    fn list_gang_uses_the_gang_member_list() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let from = h.type_line(s, "sysop list gang Old Guard");
        let out = h.got(from, s);
        assert!(out.starts_with("Old Guard members (3)\n"), "{out:?}");
        assert!(
            !out.ends_with(P),
            "the list ends with its own prompt: {out:?}"
        );
        assert!(
            h.log[from..]
                .iter()
                .any(|o| matches!(o, EngineOutput::GangQuery { id, .. } if *id == s)),
            "the members are asked of the store"
        );
    }

    #[test]
    fn arena_combat_audits_once() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        assert_eq!(
            h.line(s, "sysop arena"),
            format!("Arena rooms are in normal mode (death does not count)\n{P}")
        );
        let from = h.type_line(s, "sysop arena combat");
        assert_eq!(
            h.got(from, s),
            format!("Arena rooms are now in combat mode (death DOES count)\n{P}")
        );
        assert_eq!(sysop_events(&h, from), [("sysop.arena", "SID".to_string())]);
        assert!(!h.engine.world.display.arena_safe);
        // somebody in a colliseum room: no switching
        let bob = h.who("Bob", Role::Player);
        h.plr_mut(bob).room = RoomId(COLLISEUM);
        assert_eq!(
            h.line(s, "sysop arena normal"),
            format!("You cannot switch arena mode while users are currently in arena rooms!\n{P}")
        );
    }

    // GOD user SPEAK: the target gets the gods' line and his prompt
    // GOD user JAIL runs text block 1510 for the target, then Done
    #[test]
    fn god_jail_runs_block_1510() {
        let mut h = Harness::new();
        let jail = special_rooms()[3];
        h.engine.world.text_blocks.insert(
            (0, TextBlockNo(1510)),
            TextBlock {
                part: 0,
                number: TextBlockNo(1510),
                link: TextBlockNo(0),
                text: format!("teleport {jail} 1"),
            },
        );
        let s = h.who("Sid", Role::Sysop);
        let bob = h.who("Bob", Role::Player);
        let from = h.type_line(s, "sysop god bob jail");
        assert!(h.got(from, s).ends_with(&format!("Done\n{P}")));
        assert_eq!(h.room(bob), jail, "the block teleported him");
        assert_eq!(h.room(s), special_rooms()[0]);
    }

    // LIST LIMITED_ITEMS / NEWLIMITED: floor, shelf (T-041, OWN_ROOM + #SHELF) and user
    // copies; shelf and floor print the same line
    #[test]
    fn limited_lists_with_a_shelf_copy() {
        let mut h = Harness::new();
        let room = special_rooms()[0];
        let own = |id: i32, kind: OwnerKind, userid: &str| ItemOwnership {
            id,
            itemnum: ItemNo(1),
            owntype: kind,
            mapnum: MapId(1),
            roomnum: RoomId(room),
            userid: userid.into(),
            monnum: ActiveMonsterId(0),
        };
        for o in [
            own(1, OwnerKind::Room, ""),
            own(
                2,
                OwnerKind::Room,
                silvermere_engine::items::limited::SHELF_MARK,
            ),
            own(3, OwnerKind::User, "BOB"),
        ] {
            h.engine.world.item_ownership.insert(o.id, o);
        }
        let s = h.who("Sid", Role::Sysop);
        let line = format!("Item Ancient Sword Map 1 Room {room}\n");
        assert_eq!(
            h.line(s, "sysop list limited_items"),
            format!("{line}{line}Item Ancient Sword User BOB\n{P}")
        );
        assert_eq!(
            h.line(s, "sysop list newlimited"),
            format!("{line}{line}Item Ancient Sword User BOB\n\n{P}")
        );
    }

    #[test]
    fn god_speak() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let bob = h.who("Bob", Role::Player);
        let from = h.type_line(s, "sysop god bob speak behave  now");
        assert_eq!(
            h.got(from, bob),
            format!("The gods tell you: behave  now\n{P}")
        );
        assert_eq!(h.got(from, s), format!("You have spoken.\n{P}"));
    }

    // GOD user GOOD / LAWFUL / UNLAWFUL / RETRAIN / DISABLE TOPTEN / CHANGE SEX
    #[test]
    fn god_options_change_the_record() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let bob = h.who("Bob", Role::Player);
        h.line(s, "sysop god bob lawful");
        assert_eq!((h.plr(bob).lawful, h.plr(bob).evil), (-10, -51));
        h.line(s, "sysop god bob unlawful");
        assert_eq!((h.plr(bob).lawful, h.plr(bob).evil), (0, 40));
        assert_eq!(
            h.line(s, "sysop god bob retrain"),
            format!(
                "The next time Bob logs into MajorMUD they will have to retrain their stats.\n{P}"
            )
        );
        assert_eq!(h.plr(bob).cp, -1);
        assert_eq!(h.line(s, "sysop god bob disable topten"), P, "(sic) silent");
        assert!(h.plr(bob).settings.contains(PlayerSettings::NOTOPTEN));
        assert_eq!(
            h.line(s, "sysop god bob change sex"),
            format!("Bob has been changed to a female.\n{P}")
        );
        assert_eq!(h.plr(bob).sex, b'F');
        assert_eq!(
            h.line(s, "sysop god nobody status"),
            format!("Cannot find user nobody\n{P}")
        );
    }

    // -------------------------------------------------------------------------------
    // Paged report and its X

    fn stored(n: usize) -> Vec<(AccountId, Character)> {
        (0..n)
            .map(|i| {
                let mut c = silvermere_engine::clear_player();
                c.userid = format!("USER{i}");
                c.firstname = format!("Name{i}");
                c.exp = (n - i) as u32;
                (AccountId(100 + i as i64), c)
            })
            .collect()
    }

    #[test]
    fn report_users_pages_and_ends_with_the_prompt() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let from = h.type_line(s, "sysop report users");
        assert!(h.got(from, s).starts_with("Users of the Realm\nUserid "));
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::UserReport
        );
        h.answer_roster(from, stored(3));
        let from = h.tick();
        let out = h.got(from, s);
        let lines: Vec<&str> = out.lines().collect();
        assert!(
            lines[0].starts_with("USER2 "),
            "lowest experience first: {out:?}"
        );
        assert!(lines[2].starts_with("USER0 "));
        assert!(out.ends_with(P));
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::Playing
        );
    }

    #[test]
    fn report_users_x_interrupts() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let from = h.type_line(s, "sysop report users");
        assert_eq!(
            h.line(s, "x"),
            "Sysop user report interrupted\n",
            "no prompt follows"
        );
        h.answer_roster(from, stored(3));
        let from = h.tick();
        assert_eq!(h.got(from, s), "");
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::Playing
        );
    }

    // LIST USERS is the sysop's WHO (display_scan_of_users(1)): no paging
    #[test]
    fn list_users() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        let _bob = h.who("Bob", Role::Player);
        let out = h.line(s, "sysop list users");
        assert!(out.starts_with("Userid           Name"), "{out:?}");
        assert!(out.contains("\nBOB              Bob "), "{out:?}");
        assert!(
            out.contains("There are 2 users in the game.\n\n"),
            "{out:?}"
        );
    }

    // -------------------------------------------------------------------------------
    // The sysop menu

    fn at_menu(h: &mut Harness) -> SessionId {
        let s = h.who("Sid", Role::Sysop);
        h.engine.world.session_mut(s).unwrap().state = SessionState::SysopMenu(SysopStep::Menu);
        s
    }

    // X: back to the main menu
    #[test]
    fn menu_x_returns_to_the_main_menu() {
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        let out = h.line(s, "x");
        assert!(out.contains("[MAJORMUD]:"), "{out:?}");
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::MainMenu
        );
    }

    // L walks the three purge-limit prompts
    #[test]
    fn menu_l_walks_the_limits() {
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        let out = h.line(s, "l");
        assert!(out.starts_with("Currently all users who have played less than 60 minutes,\n"));
        assert!(out.ends_with("New limit on minutes of play: "));
        assert_eq!(
            h.line(s, "30"),
            "Limit on minutes of play: 30\nNew days without playing: "
        );
        assert_eq!(
            h.line(s, ""),
            "Days without playing: 14\nNew experience limit: "
        );
        assert_eq!(
            h.line(s, "-5"),
            "Experience limit to check: 1000\n[MAJORMUD Sysop]: "
        );
        assert_eq!(h.engine.world.sysop.purge_mins, 30);
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::SysopMenu(SysopStep::Menu)
        );
    }

    // C: one kept evil record a tick, `Cleared:` lines, then Done and the prompt
    #[test]
    fn menu_c_clears_the_evil_points() {
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        for (a, u) in [(5, "AL"), (6, "BO")] {
            h.engine.world.evil_points.insert(
                AccountId(a),
                EvilRecord {
                    userid: u.into(),
                    evil: 30,
                },
            );
        }
        assert_eq!(h.line(s, "c"), "Clearing:  ");
        let from = h.tick();
        assert_eq!(h.got(from, s), "\u{8}-\u{8}Cleared: AL\nClearing:  ");
        h.tick();
        let from = h.tick();
        assert_eq!(h.got(from, s), "\u{8}|\u{8}Done\n\n[MAJORMUD Sysop]: ");
        assert!(h.engine.world.evil_points.is_empty());
        let deletes = h
            .log
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    EngineOutput::Persist(silvermere_engine::PersistRequest::DeleteEvilPoints(_))
                )
            })
            .count();
        assert_eq!(deletes, 2);
    }

    // X cancels a clearing
    #[test]
    fn menu_c_cancelled_with_x() {
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        h.engine.world.evil_points.insert(
            AccountId(5),
            EvilRecord {
                userid: "AL".into(),
                evil: 30,
            },
        );
        h.line(s, "c");
        assert_eq!(h.line(s, "x"), "Cancelled\n\n[MAJORMUD Sysop]: ");
        assert!(h.engine.world.sysop.jobs.is_empty());
    }

    // D: a defunct character is shown and deleted on Y
    #[test]
    fn menu_d_deletes_a_defunct_character() {
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        let from = h.type_line(s, "d");
        assert_eq!(h.got(from, s), "Checking for deleted users:  ");
        let mut chars = stored(1);
        chars[0].1.lifemins = 5;
        chars[0].1.exp = 5000;
        h.answer_roster(from, chars);
        let from = h.tick();
        let out = h.got(from, s);
        assert!(out.contains("User USER0, game-id Name0\n"), "{out:?}");
        assert!(out.ends_with("Delete this user? (THERE IS NO UNDO) :"));
        assert_eq!(h.line(s, "y"), " ");
        let from = h.tick();
        assert!(h.got(from, s).contains("Deleting USER0\n"));
        assert!(h.log.iter().any(|o| matches!(
            o,
            EngineOutput::Persist(silvermere_engine::PersistRequest::DeleteCharacter {
                account: AccountId(100)
            })
        )));
        let from = h.tick();
        let _ = h.got(from, s);
        assert_eq!(
            h.engine.world.session(s).unwrap().state,
            SessionState::SysopMenu(SysopStep::Menu)
        );
    }

    // H: the menu reads the hoarder's record again by user-id when the sysop
    // answers Y, so a write queued since the roster was read (a hang-up's save) is the
    // record stripped: the penalty and ST_HUNGUP stay (T-088)
    #[test]
    fn menu_h_strips_the_record_queued_since_the_roster() {
        use silvermere_types::flags::PlayerStatus;
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        let from = h.type_line(s, "h");
        let mut chars = stored(1);
        chars[0].1.items[0] = ItemNo(1);
        chars[0].1.itemuses[0] = 0;
        chars[0].1.hp = 35;
        let account = chars[0].0;
        let mut hung = chars[0].1.clone();
        h.answer_roster(from, chars);
        let from = h.tick();
        assert!(
            h.got(from, s)
                .ends_with("Remove ALL limited items from this user? (THERE IS NO UNDO) :")
        );
        // the hang-up's save lands in the queue while the sysop reads the question
        hung.hp = 28;
        hung.status.insert(PlayerStatus::HUNGUP);
        hung.items[1] = ItemNo(2);
        hung.itemuses[1] = 0;
        h.engine.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| {
                silvermere_engine::player::queue_character(w, account, Box::new(hung))
            }),
        });
        assert_eq!(h.line(s, "y"), " ");
        let from = h.tick();
        assert_eq!(
            h.got(from, s),
            "Removing limited items from USER0\nChecking for users with limited items: \n \
             \u{8}\\\u{8}Done Checking Users\n\n[MAJORMUD Sysop]: "
        );
        let saved = h
            .log
            .iter()
            .rev()
            .find_map(|o| match o {
                EngineOutput::Persist(silvermere_engine::PersistRequest::Character {
                    account: a,
                    character,
                }) if *a == account => Some(character.clone()),
                _ => None,
            })
            .expect("the stripped record is saved");
        assert_eq!(saved.items[0], ItemNo(0));
        assert_eq!(saved.items[1], ItemNo(2));
        assert_eq!(saved.hp, 28);
        assert!(saved.status.contains(PlayerStatus::HUNGUP));
    }

    // H: a record deleted since the roster was read is left alone after the notice
    // ("no such character any more"; T-088)
    #[test]
    fn menu_h_leaves_a_record_deleted_since_the_roster() {
        let mut h = Harness::new();
        let s = at_menu(&mut h);
        let from = h.type_line(s, "h");
        let mut chars = stored(1);
        chars[0].1.items[0] = ItemNo(1);
        chars[0].1.itemuses[0] = 0;
        let account = chars[0].0;
        h.answer_roster(from, chars);
        h.tick();
        h.engine.handle(EngineInput::Custom {
            session: None,
            run: Box::new(move |w| silvermere_engine::player::queue_character_delete(w, account)),
        });
        let from = h.log.len();
        assert_eq!(h.line(s, "y"), " ");
        let tick = h.tick();
        assert_eq!(
            h.got(tick, s),
            "Removing limited items from USER0\nChecking for users with limited items: \n \
             \u{8}\\\u{8}Done Checking Users\n\n[MAJORMUD Sysop]: "
        );
        assert!(!h.log[from..].iter().any(|o| matches!(
            o,
            EngineOutput::Persist(silvermere_engine::PersistRequest::Character { .. })
        )));
    }

    // the hidden word works only for the sysop role
    #[test]
    fn hidden_word_needs_the_sysop_role() {
        let mut h = Harness::new();
        let g = h.who("Gil", Role::GameOp);
        h.engine.world.session_mut(g).unwrap().state = SessionState::MainMenu;
        h.line(g, "sysop");
        assert_eq!(
            h.engine.world.session(g).unwrap().state,
            SessionState::MainMenu
        );
    }

    // -------------------------------------------------------------------------------
    // SYSOP BUFFERS (T-101): display_buffer_statistics
    // -------------------------------------------------------------------------------

    /// The statistics after `heading`: every cache line, `\r` ends turned to newlines.
    fn stat_lines(out: &str) -> Vec<&str> {
        out.lines().filter(|l| l.contains("Size: ")).collect()
    }

    // the heading, the label column of 17 and `%-5u`
    #[test]
    fn buffers_prints_the_heading_and_a_padded_line_per_cache() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        h.line(s, "sysop buffers clear");
        h.engine.clock.advance(std::time::Duration::from_secs(10));
        let w = &mut h.engine.world;
        {
            let mut c = w.active_monsters.cache_mut();
            c.get(7, true);
            c.get(7, true);
            c.get(8, true);
        }
        let out = h.line(s, "sysop buffers");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "Buffer Statistics: (655360 - 10 Sec)");
        assert_eq!(
            lines[3],
            "Active Monsters: Size: 301   Used: 2     Hits: 1     Miss: 2     Mod: 0    "
        );
        assert_eq!(
            lines[1],
            format!(
                "Rooms:           Size: 2001  Used: {:<5} Hits: 0     Miss: 0     Mod: 0    ",
                h.engine.world.rooms.len()
            )
        );
        assert_eq!(stat_lines(&out).len(), 11);
        for (l, label) in lines[1..12].iter().zip([
            "Rooms:",
            "Items:",
            "Active Monsters:",
            "Known Monsters:",
            "Spells:",
            "Classes:",
            "Races:",
            "Shops:",
            "Bankbooks:",
            "Messages:",
            "Gangs:",
        ]) {
            assert_eq!(&l[..17], format!("{label:<17}"));
            assert!(l.ends_with("Mod: 0    "), "{l}");
        }
    }

    // CLEAR zeroes hits and misses, keeps used,
    // prints "Cleared" and restarts the clock, then shows the statistics
    #[test]
    fn buffers_clear_resets_the_counters() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        h.engine.clock.advance(std::time::Duration::from_secs(500));
        {
            let mut c = h.engine.world.active_monsters.cache_mut();
            c.get(7, true);
            c.get(7, true);
        }
        let out = h.line(s, "sysop buffers clear");
        assert!(
            out.starts_with("Cleared\nBuffer Statistics: (0 - 0 Sec)\n"),
            "{out}"
        );
        let mons: Vec<&str> = stat_lines(&out)
            .into_iter()
            .filter(|l| l.starts_with("Active"))
            .collect();
        assert_eq!(
            mons,
            ["Active Monsters: Size: 301   Used: 1     Hits: 0     Miss: 0     Mod: 0    "]
        );
        assert_eq!(h.engine.world.sysop.stats_start, h.engine.world.wall);
    }

    // through the world's look-up: a load is a miss, a cached find a
    // hit, and the shown line follows (the test's rat is inserted unsaved, so it counts as
    // modified)
    #[test]
    fn a_monster_lookup_moves_hits_or_miss() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        rat(&mut h, NEWHAVEN);
        let n = ActiveMonsterId(1);
        let w = &h.engine.world;
        assert!(w.active_monsters.by_number(n).is_some());
        assert_eq!(
            (
                w.active_monsters.cache().hits(),
                w.active_monsters.cache().misses()
            ),
            (0, 1)
        );
        assert!(w.active_monsters.by_number(n).is_some());
        assert!(w.active_monsters.by_number(n).is_some());
        assert_eq!(
            (
                w.active_monsters.cache().hits(),
                w.active_monsters.cache().misses()
            ),
            (2, 1)
        );
        let out = h.line(s, "sysop buffers");
        assert!(
            out.contains(
                "Active Monsters: Size: 301   Used: 1     Hits: 2     Miss: 1     Mod: 1    \n"
            ),
            "{out}"
        );
    }

    // before any CLEAR the clock runs from start-up
    #[test]
    fn buffers_before_a_clear_counts_from_start_up() {
        let mut h = Harness::new();
        let s = h.who("Sid", Role::Sysop);
        h.engine.clock.advance(std::time::Duration::from_secs(3));
        let out = h.line(s, "sysop buffers");
        assert!(
            out.starts_with("Buffer Statistics: (196608 - 3 Sec)\n"),
            "{out}"
        );
    }
}
