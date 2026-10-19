//! Communications (T-045) through the whole input pipeline: telepath with its allowance
//! and receipts, say-to, broadcast, gossip and auction, the forget list, say and yell.
//! The tests sit in `mod comms` so `cargo test -p silvermere-engine comms::` runs them.

use crate::common;

mod comms {
    use std::sync::mpsc::Receiver;

    use super::common::{TestClock, enter, special_rooms, tiny_world};
    use silvermere_engine::comms::allowance::DEFAULT_MAX;
    use silvermere_engine::{Engine, EngineInput, EngineOutput, SeededRng, SessionId};
    use silvermere_text::strip_ansi_str;
    use silvermere_types::enums::MessageLevel;
    use silvermere_types::flags::{PlayerFlags, PlayerSettings, PlayerStatus};
    use silvermere_types::ids::{RoomId, RoomKey};
    use silvermere_types::records::Character;

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
            let world = tiny_world(8);
            let engine = Engine::new(world, clock.clone(), SeededRng::new(7), tx);
            Self {
                engine,
                clock,
                rx,
                log: Vec::new(),
            }
        }

        /// A playing character in the start room with 48 hit points, three of each
        /// allowance (`create_player`) and the statline ON.
        fn player(&mut self, name: &str) -> SessionId {
            self.player_in(name, special_rooms()[0])
        }

        fn player_in(&mut self, name: &str, room: u32) -> SessionId {
            let w = &mut self.engine.world;
            let id = enter(w, name, room, MessageLevel::High);
            let s = w.session_mut(id).unwrap();
            s.flags.hasgoss = true;
            s.flags.hasauct = true;
            let c = s.character.as_mut().unwrap();
            (c.hp, c.maxhp) = (48, 48);
            (c.pagemax, c.gossmax, c.auctmax) = (DEFAULT_MAX, DEFAULT_MAX, DEFAULT_MAX);
            id
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

        /// Types `text` as `id`; what everyone received is read with [`Harness::got`].
        fn type_line(&mut self, id: SessionId, text: &str) -> usize {
            let from = self.pump();
            self.engine.handle(EngineInput::Line {
                id,
                text: text.into(),
            });
            self.pump();
            from
        }

        fn bytes_since(&self, from: usize, id: SessionId) -> Vec<u8> {
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

        /// What `id` received since `from`, ANSI stripped, `\r\n` as `\n`.
        fn got(&self, from: usize, id: SessionId) -> String {
            let b = self.bytes_since(from, id);
            strip_ansi_str(&String::from_utf8_lossy(&b)).replace("\r\n", "\n")
        }

        /// Types `text` and returns what `id` itself received.
        fn line(&mut self, id: SessionId, text: &str) -> String {
            let from = self.type_line(id, text);
            self.got(from, id)
        }

        /// One medium cycle (3 s, `secmed`): the allowances start again.
        fn next_period(&mut self) {
            self.clock.advance_ticks(3, &mut self.engine);
            self.pump();
        }
    }

    const P: &str = "[HP=48]:";

    fn has(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    // ---------------------------------------------------------------------------------
    // Telepath (cmd_whisper)

    // The receiver's line in green with clrtbl[13], then the
    // receiver's prompt; the sender gets the grey receipt, then the prompt
    #[test]
    fn telepath_delivers_with_receipt_and_colours() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        let from = h.type_line(ann, "/brynja @ok");
        assert_eq!(
            h.got(from, ann),
            format!("--- Telepath Sent to Brynja ---\n{P}")
        );
        assert_eq!(h.got(from, bry), format!("Ann telepaths: @ok\n{P}"));
        let wire = h.bytes_since(from, bry);
        let tele = silvermere_text::colour::tell_colour(13, Default::default());
        let mut want = b"\x1B[79D\x1B[K\x1B[32mAnn telepaths: ".to_vec();
        want.extend_from_slice(tele);
        want.extend_from_slice(b"@ok\r\n");
        assert!(
            wire.starts_with(&want),
            "{:?}",
            String::from_utf8_lossy(&wire)
        );
        let sent = h.bytes_since(from, ann);
        assert!(has(&sent, b"\x1B[0;37m--- Telepath Sent to Brynja ---\r\n"));
        insta::assert_snapshot!(
            "telepath_wire",
            format!(
                "receiver: {:?}\nsender: {:?}",
                String::from_utf8_lossy(&wire),
                String::from_utf8_lossy(&sent)
            )
        );
    }

    // clrtbl[COL_TELEPATH] in the receiver's palette: grey for 0 to 2, light cyan for 3
    #[test]
    fn telepath_colour_follows_the_receiver_palette() {
        use silvermere_text::colour::{Palette, tell_colour};
        for pal in 0..4u8 {
            let mut h = Harness::new();
            let ann = h.player("Ann");
            let bry = h.player("Brynja");
            h.plr_mut(bry).palette = pal;
            let from = h.type_line(ann, "/brynja hi");
            let mut want = b"Ann telepaths: ".to_vec();
            want.extend_from_slice(tell_colour(13, Palette::repaired(pal)));
            want.extend_from_slice(b"hi\r\n");
            assert!(has(&h.bytes_since(from, bry), &want), "palette {pal}");
        }
    }

    // The word form, the spacing kept (the rest of the line from the third word)
    #[test]
    fn telepath_by_command_word_keeps_the_spacing() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        let from = h.type_line(ann, "whisper bry  two  spaces");
        assert_eq!(h.got(from, bry), format!("Ann telepaths: two  spaces\n{P}"));
    }

    #[test]
    fn telepath_refusals() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        assert_eq!(
            h.line(ann, "/brynja"),
            format!("You have to telepath something!\n{P}")
        );
        assert_eq!(
            h.line(ann, "/ hello"),
            format!("You have to specify a person to telepath to.\n{P}")
        );
        assert_eq!(
            h.line(ann, "/ann hi"),
            format!("Why are you telepathing to yourself?\n{P}")
        );
        assert_eq!(h.line(ann, "/zed hi"), format!("Cannot find user!\n{P}"));
        // a name but no message: the page is counted all the same (sic)
        let from = h.type_line(ann, "whisper brynja");
        assert_eq!(h.got(from, ann), format!("What do you really mean?\n{P}"));
        assert_eq!(h.got(from, bry), "");
        assert_eq!(
            h.engine
                .world
                .session(bry)
                .unwrap()
                .player()
                .unwrap()
                .pagecnt,
            1
        );
        // a sender who blocks telepaths may not send one
        h.plr_mut(ann).settings |= PlayerSettings::NOTELEPATH;
        assert_eq!(
            h.line(ann, "/brynja hi"),
            format!("You may not telepath while you are blocking telepaths.\n{P}")
        );
    }

    // `/P`-style input (the host's /P, /#, /SYS are gone, 05 section 2) is a telepath to
    // whoever the name P fits
    #[test]
    fn host_slash_commands_are_telepaths() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        assert_eq!(
            h.line(ann, "/P Brynja hello"),
            format!("Cannot find user!\n{P}")
        );
        let pia = h.player("Pia");
        let from = h.type_line(ann, "/P Brynja hello");
        assert_eq!(
            h.got(from, pia),
            format!("Ann telepaths: Brynja hello\n{P}")
        );
        assert_eq!(h.line(ann, "/SYS x"), format!("Cannot find user!\n{P}"));
    }

    // An abbreviation fitting two players lists them; a full name
    // wins outright
    #[test]
    fn ambiguous_names_list_the_candidates() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let _bo = h.player("Bo");
        let bob = h.player("Bob");
        assert_eq!(
            h.line(ann, "/b hi"),
            format!(
                "Please be more specific.  You could have meant any of these:\n-- Bo\n-- Bob\n{P}"
            )
        );
        let from = h.type_line(ann, "/bob hi");
        assert_eq!(h.got(from, bob), format!("Ann telepaths: hi\n{P}"));
    }

    // get_user_number with several matches answers UserMatch::Several, and the dispatcher's
    // ANSI_NORMAL and the list are held for the same send. Telepath and say-to then count
    // as handled (Handled::Done), and the pending-command step adds the prompt and sends all
    // at once, so the player gets reset, list, prompt in one piece.
    #[test]
    fn several_matches_send_the_reset_and_the_list_with_the_prompt() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        h.player("Bo");
        h.player("Bob");
        let prompt =
            String::from_utf8(silvermere_engine::tell::prompt_bytes(&h.engine.world, ann)).unwrap();
        let want = format!(
            "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;31m\
             Please be more specific.  You could have meant any of these:\r\n\
             \x1b[0;37m-- Bo\r\n-- Bob\r\n{prompt}"
        );
        for line in ["/b hi", "whisper b hi", ">b hi"] {
            let from = h.type_line(ann, line);
            assert_eq!(
                String::from_utf8_lossy(&h.bytes_since(from, ann)),
                want,
                "{line}"
            );
        }
    }

    // Four telepaths a period with the default
    // three (`<=`), the fifth is Not Sent and the receiver sees nothing; next period it
    // goes through
    #[test]
    fn telepath_allowance_per_medium_cycle() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        let mut delivered = 0;
        for i in 0..5 {
            let from = h.type_line(ann, &format!("/brynja n{i}"));
            let a = h.got(from, ann);
            if h.got(from, bry).contains("telepaths:") {
                delivered += 1;
                assert_eq!(a, format!("--- Telepath Sent to Brynja ---\n{P}"), "{i}");
            } else {
                assert_eq!(a, format!("--- Telepath Not Sent ---\n{P}"), "{i}");
                assert_eq!(h.got(from, bry), "");
            }
        }
        assert_eq!(delivered, 4);
        h.next_period();
        let from = h.type_line(ann, "/brynja again");
        assert_eq!(h.got(from, bry), format!("Ann telepaths: again\n{P}"));
        // SET MAXPAGES 30: thirty in a row all get through
        h.next_period();
        assert_eq!(
            h.line(bry, "set maxpages 30"),
            format!("Maxpages set to 30\n{P}")
        );
        for i in 0..30 {
            let from = h.type_line(ann, &format!("/brynja m{i}"));
            assert_eq!(
                h.got(from, ann),
                format!("--- Telepath Sent to Brynja ---\n{P}")
            );
        }
    }

    // Property: within one period, delivered = min(sent, pagemax + 1) and every attempt
    // gets exactly one receipt
    #[test]
    fn delivered_is_min_of_sent_and_pagemax_plus_one() {
        for pagemax in [0u8, 1, 3, 7] {
            for sent in 0..12usize {
                let mut h = Harness::new();
                let ann = h.player("Ann");
                let bry = h.player("Brynja");
                h.plr_mut(bry).pagemax = pagemax;
                let (mut delivered, mut receipts) = (0, 0);
                for _ in 0..sent {
                    let from = h.type_line(ann, "/brynja x");
                    delivered += h.got(from, bry).matches("telepaths:").count();
                    receipts += h.got(from, ann).matches("--- Telepath").count();
                }
                assert_eq!(
                    delivered,
                    sent.min(usize::from(pagemax) + 1),
                    "{pagemax} {sent}"
                );
                assert_eq!(receipts, sent);
            }
        }
    }

    // The paging check run against the original under MBBSEmu, in-engine: two players with
    // `set maxpages 30` exchange 50 numbered @-messages each way, one fast tick per round as
    // the bots' pacing gives; every one arrives and every one is receipted
    #[test]
    fn partycheck_hundred_telepaths() {
        let mut h = Harness::new();
        let a = h.player("Brynja");
        let b = h.player("Isolde");
        h.line(a, "set maxpages 30");
        h.line(b, "set maxpages 30");
        let (mut got_a, mut got_b, mut sent) = (Vec::new(), Vec::new(), 0);
        for i in 0..50 {
            for (from_id, to_name, inbox) in [(a, "isolde", &mut got_b), (b, "brynja", &mut got_a)]
            {
                let to_id = if from_id == a { b } else { a };
                let from = h.type_line(from_id, &format!("/{to_name} @ping {i}"));
                sent += h.got(from, from_id).matches("--- Telepath Sent to").count();
                for l in h.got(from, to_id).lines() {
                    if let Some(n) = l.split("@ping ").nth(1) {
                        inbox.push(n.parse::<u32>().unwrap());
                    }
                }
            }
            h.clock.advance_ticks(1, &mut h.engine);
            h.pump();
        }
        assert_eq!(sent, 100);
        assert_eq!(got_a, (0..50).collect::<Vec<_>>());
        assert_eq!(got_b, (0..50).collect::<Vec<_>>());
    }

    // Blocked and forgotten look the same to the sender as out of pages
    #[test]
    fn blocked_and_forgotten_are_not_sent() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        h.plr_mut(bry).settings |= PlayerSettings::NOTELEPATH;
        assert_eq!(
            h.line(ann, "/brynja hi"),
            format!("--- Telepath Not Sent ---\n{P}")
        );
        h.plr_mut(bry).settings -= PlayerSettings::NOTELEPATH;
        assert_eq!(
            h.line(bry, "forget ann"),
            format!("You have now forgotten ann\n{P}")
        );
        let from = h.type_line(ann, "/brynja hi");
        assert_eq!(h.got(from, ann), format!("--- Telepath Not Sent ---\n{P}"));
        assert_eq!(h.got(from, bry), "");
        assert_eq!(
            h.line(bry, "forget ANN"),
            format!("You have no longer forgotten ANN\n{P}")
        );
        let from = h.type_line(ann, "/brynja hi");
        assert_eq!(h.got(from, bry), format!("Ann telepaths: hi\n{P}"));
    }

    // ---------------------------------------------------------------------------------
    // Say-to (cmd_direct_message)

    #[test]
    fn say_to_reaches_target_and_room() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        let cai = h.player("Cai");
        let away = h.player_in("Dov", special_rooms()[1]);
        let from = h.type_line(ann, ">bry hello  there");
        assert_eq!(
            h.got(from, ann),
            format!("--- Message Directed to Brynja ---\n{P}")
        );
        assert_eq!(
            h.got(from, bry),
            format!("Ann says (to you) \"hello  there\"\n{P}")
        );
        assert_eq!(
            h.got(from, cai),
            format!("Ann says (to Brynja) \"hello  there\"\n{P}")
        );
        assert_eq!(h.got(from, away), "");
        insta::assert_snapshot!(
            "say_to_wire",
            format!("{:?}", String::from_utf8_lossy(&h.bytes_since(from, bry)))
        );
        // another room: not sent
        assert_eq!(
            h.line(ann, ">dov hi"),
            format!("--- Message Not Sent ---\n{P}")
        );
        // a page is counted, the allowance is not checked
        assert_eq!(
            h.engine
                .world
                .session(bry)
                .unwrap()
                .player()
                .unwrap()
                .pagecnt,
            1
        );
    }

    // The target blocking telepaths or having forgotten the sender does not
    // stop a say-to; the target being hidden does
    #[test]
    fn say_to_ignores_blocking_and_forget_but_not_hiding() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        h.plr_mut(bry).settings |= PlayerSettings::NOTELEPATH;
        h.line(bry, "forget Ann");
        let from = h.type_line(ann, ">brynja hi");
        assert_eq!(h.got(from, bry), format!("Ann says (to you) \"hi\"\n{P}"));
        h.plr_mut(bry).hidden = 1;
        assert_eq!(
            h.line(ann, ">brynja hi"),
            format!("--- Message Not Sent ---\n{P}")
        );
    }

    #[test]
    fn say_to_refusals() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let _bry = h.player("Brynja");
        assert_eq!(
            h.line(ann, ">brynja"),
            format!("You have to direct something!\n{P}")
        );
        assert_eq!(
            h.line(ann, "> hi"),
            format!("You have to specify a person to direct to.\n{P}")
        );
        assert_eq!(
            h.line(ann, ">ann hi"),
            format!("Why are you directing messages to yourself?\n{P}")
        );
        assert_eq!(h.line(ann, ">zed hi"), format!("Cannot find user!\n{P}"));
        h.plr_mut(ann).status |= PlayerStatus::MUTE;
        assert_eq!(
            h.line(ann, ">brynja hi"),
            format!("You may not direct any messages right now.\n{P}")
        );
    }

    // ---------------------------------------------------------------------------------
    // Say and yell

    #[test]
    fn say_and_its_echo() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        let from = h.type_line(ann, ".hello all");
        assert_eq!(h.got(from, ann), format!("You say \"hello all\"\n{P}"));
        assert_eq!(h.got(from, bry), format!("Ann says \"hello all\"\n{P}"));
        // PS_BRIEFTALK only changes the speaker's echo
        h.plr_mut(ann).settings |= PlayerSettings::BRIEFTALK;
        let from = h.type_line(ann, ".again");
        assert_eq!(h.got(from, ann), format!("Well Spoken...\n{P}"));
        assert_eq!(h.got(from, bry), format!("Ann says \"again\"\n{P}"));
        // alone in the room, slow talk: nobody heard
        let solo = h.player_in("Solo", special_rooms()[3]);
        assert_eq!(
            h.line(solo, ".anyone"),
            format!("Your command had no effect.\n{P}")
        );
        h.plr_mut(ann).status |= PlayerStatus::MUTE;
        assert_eq!(h.line(ann, ".hi"), format!("You cannot speak!\n{P}"));
    }

    // Every room one exit away hears the yell from the listener's
    // side (revdir), above and below for up and down
    #[test]
    fn yell_reaches_adjacent_rooms_with_the_reverse_direction() {
        let mut h = Harness::new();
        let [r0, r1, r2, r3] = special_rooms();
        {
            let rm = h.engine.world.rooms.get_mut(&RoomKey::new(1, r0)).unwrap();
            rm.exits[0].target = RoomId(r1); // north
            rm.exits[3].target = RoomId(r2); // west
            rm.exits[8].target = RoomId(r3); // up
        }
        let ann = h.player("Ann");
        let here = h.player("Hal");
        let north = h.player_in("Nia", r1);
        let west = h.player_in("Wes", r2);
        let up = h.player_in("Uma", r3);
        let from = h.type_line(ann, "\"help me");
        assert_eq!(h.got(from, ann), format!("You yell \"help me\"\n{P}"));
        assert_eq!(
            h.got(from, here),
            format!("From Ann (Yelling): \"help me\"\n{P}")
        );
        // the yeller is south of the room to the north
        let n = h.got(from, north);
        assert!(n.starts_with("Someone yells from the s"), "{n:?}");
        assert!(n.ends_with(&format!(" \"help me\"\n{P}")), "{n:?}");
        let w = h.got(from, west);
        assert!(w.starts_with("Someone yells from the e"), "{w:?}");
        assert_eq!(
            h.got(from, up),
            format!("Someone yells from below \"help me\"\n{P}")
        );
    }

    // ---------------------------------------------------------------------------------
    // Gossip and auction (through tell_game)

    #[test]
    fn gossip_reaches_everyone_with_the_broadcast_colour() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let far = h.player_in("Far", special_rooms()[2]);
        let from = h.type_line(ann, "gossip hello  world");
        assert_eq!(h.got(from, far), format!("Ann gossips: hello  world\n{P}"));
        // the speaker hears it too (tell_game(-1)), then gets the command's prompt
        assert_eq!(
            h.got(from, ann),
            format!("Ann gossips: hello  world\n{P}{P}")
        );
        let wire = h.bytes_since(from, far);
        let col = silvermere_text::colour::tell_colour(14, Default::default());
        // tell_game sends the whole output buffer: the dispatcher's ANSI_NORMAL,
        // still queued, goes to every listener ahead of the line
        let mut want = b"\x1B[0;37;40m\x1B[79D\x1B[KAnn gossips: ".to_vec();
        want.extend_from_slice(col);
        want.extend_from_slice(b"hello  world\r\n");
        assert!(
            wire.starts_with(&want),
            "{:?}",
            String::from_utf8_lossy(&wire)
        );
        let from = h.type_line(ann, "auction a sword");
        assert_eq!(h.got(from, far), format!("Ann auctions: a sword\n{P}"));
    }

    // GOSSIP alone toggles; switched off, one's own gossip is refused with
    // the two lines and others' gossip is not received
    #[test]
    fn gossip_off_refuses_and_hears_nothing() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        assert_eq!(
            h.line(ann, "gossip"),
            format!("You will no longer receive gossip messages.\n{P}")
        );
        assert!(
            h.engine
                .world
                .session(ann)
                .unwrap()
                .player()
                .unwrap()
                .flags
                .contains(PlayerFlags::NOGOSSIP)
        );
        assert_eq!(
            h.line(ann, "gossip hi"),
            format!(
                "You may not use gossip while your gossip is turned off.\nType 'gossip' to turn gossip on again.\n{P}"
            )
        );
        let from = h.type_line(bry, "gossip hi");
        assert_eq!(h.got(from, ann), "");
        // auction is a separate switch
        let from = h.type_line(bry, "auction hi");
        assert_eq!(h.got(from, ann), format!("Brynja auctions: hi\n{P}"));
        assert_eq!(
            h.line(ann, "gossip"),
            format!("You will now receive gossip messages.\n{P}")
        );
        assert_eq!(
            h.line(ann, "auction"),
            format!("You will no longer receive auction messages.\n{P}")
        );
        assert_eq!(
            h.line(ann, "auction hi"),
            format!(
                "You may not use auction while your auction is turned off.\nType 'auction' to turn auction on again.\n{P}"
            )
        );
    }

    // GOSSKEY/AUCTKEY as permissions: without them, refused; staff always may
    #[test]
    fn gossip_permission() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        h.engine.world.session_mut(ann).unwrap().flags.hasgoss = false;
        h.engine.world.session_mut(ann).unwrap().flags.hasauct = false;
        assert_eq!(
            h.line(ann, "gossip hi"),
            format!("You may not use gossip - please contact your sysop.\n{P}")
        );
        assert_eq!(
            h.line(ann, "auction hi"),
            format!("You may not use auction - please contact your sysop.\n{P}")
        );
        h.engine.world.session_mut(ann).unwrap().flags.sysop = true;
        assert!(h.line(ann, "gossip hi").starts_with("Ann gossips: hi"));
    }

    // The gossip count with gossip's two tell_game calls: each gossip costs a
    // listener two counts, so with gossmax 3 a listener hears two whole gossips a period
    // (prefix 0->1, text 1->2, prefix 2->3, text 3->4, then 4 > 3); with gossmax 4 the
    // third gossip's prefix gets through without its text. Others still hear it.
    #[test]
    fn gossip_allowance_is_per_listener_and_counts_both_halves() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player("Brynja");
        let cai = h.player("Cai");
        h.plr_mut(cai).gossmax = 255;
        let mut heard = String::new();
        let mut cai_heard = 0;
        for i in 0..3 {
            let from = h.type_line(ann, &format!("gossip g{i}"));
            heard.push_str(&h.got(from, bry));
            cai_heard += h.got(from, cai).matches("gossips:").count();
        }
        assert_eq!(heard, format!("Ann gossips: g0\n{P}Ann gossips: g1\n{P}"));
        assert_eq!(cai_heard, 3);
        h.next_period();
        h.plr_mut(bry).gossmax = 4;
        let mut heard = String::new();
        for i in 0..3 {
            let from = h.type_line(ann, &format!("gossip h{i}"));
            heard.push_str(&h.got(from, bry));
        }
        assert_eq!(
            heard,
            format!("Ann gossips: h0\n{P}Ann gossips: h1\n{P}Ann gossips: ")
        );
        h.next_period();
        assert_eq!(
            h.line(bry, "set maxgossip 9"),
            format!("Max gossips set to 9\n{P}")
        );
        assert_eq!(
            h.line(bry, "set maxauction 9"),
            format!("Max auctions set to 9\n{P}")
        );
    }

    // ---------------------------------------------------------------------------------
    // Broadcast (cmd_broadcast)

    #[test]
    fn broadcast_on_a_channel() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        let bry = h.player_in("Brynja", special_rooms()[1]);
        let off = h.player("Off");
        h.plr_mut(ann).bchannel = 7;
        h.plr_mut(bry).bchannel = 7;
        let from = h.type_line(ann, "'hi there");
        assert_eq!(
            h.got(from, bry),
            format!("Broadcast from Ann \"hi there\"\n{P}")
        );
        assert_eq!(
            h.got(from, ann),
            format!("Broadcast from Ann \"hi there\"\n{P}{P}")
        );
        assert_eq!(h.got(from, off), "");
        let from = h.type_line(ann, "br hello");
        assert_eq!(
            h.got(from, bry),
            format!("Broadcast from Ann \"hello\"\n{P}")
        );
        // a quote alone lists the channel
        assert_eq!(
            h.line(ann, "'"),
            format!("The following users are on channel 7:\nAnn, Brynja\n{P}")
        );
        // off any channel it is not a command: the line is speech
        assert_eq!(
            h.line(off, "-x"),
            format!("Your command had no effect.\n{P}")
        );
    }

    // ---------------------------------------------------------------------------------
    // The forget list

    #[test]
    fn forget_list_lines() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        assert_eq!(
            h.line(ann, "forget"),
            format!("You have not forgotten any users.\n{P}")
        );
        assert_eq!(
            h.line(ann, "forget Bob"),
            format!("You have now forgotten Bob\n{P}")
        );
        h.line(ann, "forget Cai");
        assert_eq!(
            h.line(ann, "forget"),
            format!("Forgotten Users:\nCai\nBob\n{P}")
        );
        assert_eq!(
            h.line(ann, "forget bob"),
            format!("You have no longer forgotten bob\n{P}")
        );
        // the `\n` of the original goes out as CR LF
        let from = h.type_line(ann, "forget Dee");
        assert!(has(
            &h.bytes_since(from, ann),
            b"You have now forgotten Dee\r\n"
        ));
        // eleven bytes or more: not a command, the line is speech
        assert_eq!(
            h.line(ann, "forget abcdefghijk"),
            format!("Your command had no effect.\n{P}")
        );
        // max(terminal count, 20) names
        for i in 0..18 {
            h.line(ann, &format!("forget n{i}"));
        }
        assert_eq!(
            h.line(ann, "forget zz"),
            format!("Your forget list is too long.\n{P}")
        );
    }

    // The list is per session and emptied when the player leaves the game
    #[test]
    fn forget_list_is_cleared_at_logoff() {
        let mut h = Harness::new();
        let ann = h.player("Ann");
        h.line(ann, "forget Bob");
        h.engine.handle(EngineInput::Disconnect { id: ann });
        h.pump();
        assert!(
            h.engine
                .world
                .session(ann)
                .is_none_or(|s| s.ust.forgets.is_empty())
        );
    }
}
