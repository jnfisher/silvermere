//! Gangs (T-044) through the whole input pipeline, against the original's gang commands
//! (create, invite/uninvite member, join/leave gang, broadgang, disband,
//! promote, demote, stock/unstock/markup), the member lists,
//! `SUBST_DISBAND` and the gang part of `add_experience`.
//!
//! Every test sits in `mod gang`, so `cargo test -p silvermere-engine gang::` runs them.

use crate::common;

mod gang {
    use std::sync::mpsc::Receiver;

    use proptest::prelude::*;

    use super::common::{TestClock, enter, test_config, tiny_snapshot};
    use silvermere_engine::display::topten::display_top_gangs;
    use silvermere_engine::gangs::{
        self, GangAnswer, GangQuery, OfflineOp, add_kill_exp, gang_shop_owner,
    };
    use silvermere_engine::world::blank;
    use silvermere_engine::{
        Engine, EngineInput, EngineOutput, PersistRequest, SeededRng, SessionId, World,
    };
    use silvermere_text::strip_ansi_str;
    use silvermere_types::enums::{
        Ability, Currency, GangHouse, MessageLevel, RoomType, SessionState, ShopType,
    };
    use silvermere_types::flags::{GangFlags, PlayerFlags, SessionFlags};
    use silvermere_types::ids::{AccountId, ClassNo, ItemNo, RoomKey, ShopNo};
    use silvermere_types::records::{BankBook, Character, Gang, Shop, ShopSlot};
    use silvermere_types::sizes::NSHOPITEMS;

    /// The first room of the tiny world.
    const ROOM: u32 = 1;
    /// A gang shop of gang house 1, added to the tiny world.
    const GANG_ROOM: u32 = 9002;
    const GANG_SHOP: ShopNo = ShopNo(90);
    /// The gang house 1 controller (`ABL_GANGSHOPITEM` 1) and a dagger to sell.
    const CONTROLLER: ItemNo = ItemNo(50);
    const DAGGER: ItemNo = ItemNo(51);
    const DAGNABIT: ItemNo = ItemNo(52);

    fn world() -> World {
        let mut snap = tiny_snapshot();
        let mut ctl = blank::clear_item(CONTROLLER);
        ctl.name = "gang shop key".into();
        ctl.abilities[0] = (Ability::GangShopItem, 1);
        snap.items.push(ctl);
        let mut dagger = blank::clear_item(DAGGER);
        dagger.name = "dagger".into();
        dagger.cost = 12;
        dagger.currency = Currency::Silver;
        dagger.weight = 1;
        dagger.gettable = 1;
        snap.items.push(dagger);
        let mut dagnabit = blank::clear_item(DAGNABIT);
        dagnabit.name = "dagnabit".into();
        dagnabit.cost = 3;
        dagnabit.currency = Currency::Copper;
        dagnabit.weight = 1;
        dagnabit.gettable = 1;
        snap.items.push(dagnabit);
        let mut r = blank::clear_room(RoomKey::new(1, GANG_ROOM));
        r.name = "Gang Shop".into();
        r.room_type = RoomType::Shop;
        r.shopnum = GANG_SHOP;
        r.ganghouse = GangHouse::from_code(1);
        snap.rooms.push(r);
        snap.shops.push(Shop {
            number: GANG_SHOP,
            name: "Gang Shop".into(),
            desc: Default::default(),
            shop_type: ShopType::GangShop,
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
        });
        let mut config = test_config();
        config.exp_in_top_gang = true;
        World::load(config, 8, snap).expect("the gang world loads")
    }

    struct Harness {
        engine: Engine<TestClock, SeededRng>,
        rx: Receiver<EngineOutput>,
        log: Vec<EngineOutput>,
    }

    impl Harness {
        fn new() -> Self {
            Self::over(world())
        }

        fn over(world: World) -> Self {
            let (tx, rx) = std::sync::mpsc::channel();
            let engine = Engine::new(world, TestClock::new(), SeededRng::new(7), tx);
            Self {
                engine,
                rx,
                log: Vec::new(),
            }
        }

        /// A playing character with enough experience to found a gang.
        fn player(&mut self, name: &str, last: &str) -> SessionId {
            let w = &mut self.engine.world;
            let id = enter(w, name, ROOM, MessageLevel::High);
            let c = w.session_mut(id).unwrap().character.as_mut().unwrap();
            c.lastname = last.into();
            c.exprem = 150_000;
            c.str = 50;
            c.maxwgt = 2400;
            c.movedir = -1;
            // a fully created character: `cleanup_when_user_leaves`
            // clears its invitations when it leaves
            c.settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
            id
        }

        fn w(&mut self) -> &mut World {
            &mut self.engine.world
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

        fn gang(&self, key: &str) -> Gang {
            self.engine.world.gangs[key].clone()
        }

        fn pump(&mut self) {
            while let Ok(o) = self.rx.try_recv() {
                self.log.push(o);
            }
        }

        fn text(&self, from: usize, id: SessionId) -> String {
            let b: Vec<u8> = self.log[from..]
                .iter()
                .filter_map(|o| match o {
                    EngineOutput::Write { id: i, bytes } if *i == id => Some(bytes.as_slice()),
                    _ => None,
                })
                .flatten()
                .copied()
                .collect();
            strip_ansi_str(&String::from_utf8_lossy(&b)).replace("\r\n", "\n")
        }

        /// Types `text` as `id`; what `id` and `other` received, ANSI stripped.
        fn both(&mut self, id: SessionId, text: &str, other: SessionId) -> (String, String) {
            let from = self.log.len();
            self.engine.handle(EngineInput::Line {
                id,
                text: text.into(),
            });
            self.pump();
            (self.text(from, id), self.text(from, other))
        }

        /// What `id` got, its prompts cut out.
        fn line(&mut self, id: SessionId, text: &str) -> String {
            cut_prompts(&self.both(id, text, id).0)
        }

        /// Runs `f` on the world as the store's answer would; what `id` received.
        fn answer(&mut self, id: SessionId, f: impl FnOnce(&mut World) + Send + 'static) -> String {
            let from = self.log.len();
            self.engine.handle(EngineInput::Custom {
                session: Some(id),
                run: Box::new(f),
            });
            self.pump();
            cut_prompts(&self.text(from, id))
        }

        /// The store acknowledges every character write queued so far (T-088), so the
        /// next answer's row is the one on file.
        fn store_caught_up(&mut self) {
            let written = self.engine.world.character_writes;
            silvermere_engine::player::on_characters_written(&mut self.engine.world, written);
        }

        /// The queries asked since `from`.
        fn queries(&self, from: usize) -> Vec<(SessionId, GangQuery)> {
            self.log[from..]
                .iter()
                .filter_map(|o| match o {
                    EngineOutput::GangQuery { id, query } => Some((*id, query.clone())),
                    _ => None,
                })
                .collect()
        }

        /// The last gang record saved under `key`.
        fn saved_gang(&self, key: &str) -> Option<Gang> {
            self.log.iter().rev().find_map(|o| match o {
                EngineOutput::Persist(PersistRequest::Gang(g)) if g.key == key => Some(g.clone()),
                _ => None,
            })
        }
    }

    /// The text with every prompt (`[HP=...]:` and the `^A` after it) removed.
    fn cut_prompts(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find("[HP=") {
            out.push_str(&rest[..i]);
            rest = match rest[i..].find(']') {
                Some(j) => {
                    let after = &rest[i + j + 1..];
                    let after = after.strip_prefix(':').unwrap_or(after);
                    after.strip_prefix('\u{1}').unwrap_or(after)
                }
                None => "",
            };
        }
        out.push_str(rest);
        out
    }

    fn wolves(h: &mut Harness) -> (SessionId, SessionId) {
        let bob = h.player("Bob", "Smith");
        let ann = h.player("Ann", "Lee");
        assert_eq!(h.line(bob, "create gang Wolves"), "Gang created.\n");
        (bob, ann)
    }

    // §5: create, invite, join, promote, demote, broadgang, leave, disband
    #[test]
    fn the_whole_script() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        let g = h.gang("WOLVES");
        assert_eq!(
            (
                g.name.as_str(),
                g.leader.as_str(),
                g.members,
                g.exp,
                g.flags
            ),
            ("Wolves", "Bob", 1, 0, GangFlags::empty())
        );
        assert_eq!(h.plr(bob).gang, "Wolves");
        assert_eq!(h.saved_gang("WOLVES"), Some(g));

        let (me, her) = h.both(bob, "invite member ann", ann);
        assert_eq!(
            cut_prompts(&me),
            "You have invited Ann to join your gang.\n"
        );
        assert_eq!(
            cut_prompts(&her),
            "Gang leader Bob has invited you to join Wolves.\n"
        );
        assert!(her.contains("[HP="), "the invited player gets a prompt");
        assert_eq!(h.w().gang_invites.len(), 1);

        let (her, me) = h.both(ann, "join gang wolves", bob);
        assert_eq!(
            cut_prompts(&her),
            "You have joined the gang Wolves.\nAnn just joined your gang.\n"
        );
        assert_eq!(cut_prompts(&me), "Ann just joined your gang.\n");
        assert_eq!(h.plr(ann).gang, "Wolves");
        assert_eq!(h.gang("WOLVES").members, 2);
        assert!(h.w().gang_invites.is_empty());

        let (me, her) = h.both(bob, "promote ann", ann);
        assert_eq!(
            cut_prompts(&me),
            "Gang member Ann has been notified of their promotion.\n"
        );
        assert_eq!(
            cut_prompts(&her),
            "Your gang leader has promoted you to the rank of lieutenant.\n"
        );
        assert!(h.plr(ann).flags.contains(PlayerFlags::LIEUTENANT));

        // the online list: leader, lieutenants, members
        h.plr_mut(bob).flags.insert(PlayerFlags::GANGONLINE);
        insta::assert_snapshot!("online_members", h.line(bob, "gb"));

        let (me, her) = h.both(bob, "demote ann", ann);
        assert_eq!(
            cut_prompts(&me),
            "Gang member Ann has been notified of their demotion.\n"
        );
        assert_eq!(cut_prompts(&her), "Your gang leader has demoted you.\n");
        assert!(!h.plr(ann).flags.contains(PlayerFlags::LIEUTENANT));

        // tell_gang reaches the speaker too
        let (me, her) = h.both(bob, "gb hi", ann);
        assert_eq!(cut_prompts(&me), "Bob gangpaths: hi\n");
        assert_eq!(cut_prompts(&her), "Bob gangpaths: hi\n");
        insta::assert_snapshot!(
            "gangpaths_bytes",
            format!("{:?}", h.both(ann, "bg hey", bob).1)
        );

        let (her, me) = h.both(ann, "leave gang", bob);
        assert_eq!(cut_prompts(&her), "You have left Wolves.\n");
        assert_eq!(cut_prompts(&me), "Ann has left Wolves.\n");
        assert_eq!(h.plr(ann).gang, "");
        assert_eq!(h.gang("WOLVES").members, 1);
        assert_eq!(
            h.line(ann, "leave gang"),
            "You are not currently in a gang.\n"
        );
        assert_eq!(
            h.line(bob, "leave gang"),
            "You are the leader - you may not leave your gang. Use DISBAND GANG\n"
        );

        // DISBAND asks; any answer but Y keeps the gang
        assert_eq!(
            h.line(bob, "disband gang"),
            "Are you sure you want to disband Wolves? "
        );
        assert_eq!(h.w().session(bob).unwrap().state, SessionState::Disband);
        assert_eq!(h.line(bob, "no"), "Your gang has not been disbanded.\n");
        assert_eq!(h.w().session(bob).unwrap().state, SessionState::Playing);
        h.line(bob, "disband gang");
        assert_eq!(
            h.line(bob, "y"),
            "The gang Wolves has now been disbanded.\nThe name may not be used again until all members have entered the game!\n"
        );
        let g = h.gang("WOLVES");
        assert!(g.flags.contains(GangFlags::DISBANDED));
        assert_eq!(g.members, 0);
        assert_eq!(h.plr(bob).gang, "");
        assert_eq!(h.saved_gang("WOLVES"), Some(g));
        // a disbanded name stays taken, without the leader line
        h.plr_mut(bob).gang.clear();
        assert_eq!(
            h.line(bob, "create gang wolves"),
            "The name you have chosen is already being used!\n"
        );
    }

    // §5: the gang row survives a restart unchanged; the invitations do not
    #[test]
    fn restart_keeps_the_gang_and_loses_invitations() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        let cid = h.player("Cid", "Vale");
        h.line(bob, "invite member ann");
        h.line(ann, "join gang wolves");
        h.line(bob, "invite member cid");
        add_kill_exp(h.w().gangs.get_mut("WOLVES").unwrap(), 1234);
        gangs::save_gang(h.w(), "WOLVES");
        h.engine.handle(EngineInput::Tick);
        h.pump();
        assert_eq!(h.w().gang_invites.len(), 1);
        let saved = h.saved_gang("WOLVES").expect("saved");
        assert_eq!(saved, h.gang("WOLVES"));
        // a new world from what the store holds
        let mut snap = tiny_snapshot();
        snap.gangs.push(saved.clone());
        let w = World::load(test_config(), 8, snap).unwrap();
        assert_eq!(w.gangs["WOLVES"], saved);
        assert_eq!(
            (saved.members, saved.exp, saved.leader.as_str()),
            (2, 1234, "Bob")
        );
        assert!(w.gang_invites.is_empty());
        let _ = cid;
    }

    // invitations go when the invited player leaves the game
    #[test]
    fn invitations_are_cleared_by_clear_gang_invitations() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        h.line(bob, "invite member ann");
        // (sic) only the newest invitation is compared
        h.line(bob, "invite member ann");
        assert_eq!(h.w().gang_invites.len(), 1);
        gangs::clear_gang_invitations(h.w(), ann, None);
        assert!(h.w().gang_invites.is_empty());
        assert_eq!(
            h.line(ann, "join gang wolves"),
            "You have not been invited to join that gang!\n"
        );
        assert_eq!(h.line(ann, "join gang owls"), "That gang doesn't exist!\n");
    }

    #[test]
    fn create_refusals() {
        let mut h = Harness::new();
        let bob = h.player("Bob", "Smith");
        let ann = h.player("Ann", "Lee");
        h.plr_mut(ann).exprem = 99_999;
        assert_eq!(
            h.line(ann, "create gang Owls"),
            "You are not experienced enough to start your own gang!\n"
        );
        assert_eq!(
            h.line(bob, "create gang The Very Long Gang Name"),
            "The name you have chosen is too LONG: The Very Long Gang Name\n"
        );
        assert_eq!(
            h.line(bob, "create gang none"),
            "You may not use 'None' as a gang name.\n"
        );
        assert_eq!(
            h.line(bob, "create gang Caf\u{e9}"),
            "You have specified an invalid character in your gang name.\n"
        );
        assert_eq!(h.line(bob, "create guild Wolves"), "Gang created.\n");
        assert_eq!(
            h.line(bob, "create gang Owls"),
            "You are already in one gang.  You cannot create another one.\n"
        );
        h.plr_mut(ann).exprem = 0;
        h.plr_mut(ann).expbil = 1;
        assert_eq!(
            h.line(ann, "create gang WOLVES"),
            "The name you have chosen is already being used!\nBob is the leader of Wolves.\n"
        );
        assert_eq!(
            h.line(ann, "create room north"),
            "If you are a gang leader you may lease a Gang House.\n"
        );
        assert_eq!(h.line(ann, "create thing x"), "");
    }

    #[test]
    fn invite_and_uninvite_refusals() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        let cid = h.player("Cid", "Vale");
        h.line(bob, "invite member ann");
        h.line(ann, "join gang wolves");
        assert_eq!(
            h.line(ann, "invite member cid"),
            "You must be the leader or a lieutenant of your gang to invite new members!\n"
        );
        assert_eq!(
            h.line(bob, "invite member bob"),
            "Why would you invite yourself?\n"
        );
        assert_eq!(
            h.line(bob, "invite member zed"),
            "You don't see member zed here!\n"
        );
        assert_eq!(
            h.line(cid, "join gang wolves"),
            "You have not been invited to join that gang!\n"
        );
        h.line(bob, "promote ann");
        // a lieutenant may invite, and is named so
        let (_, c) = h.both(ann, "invite member cid", cid);
        assert_eq!(
            cut_prompts(&c),
            "Lieutenant Ann has invited you to join Wolves.\n"
        );
        h.line(cid, "join gang wolves");
        assert_eq!(
            h.line(ann, "uninvite member bob"),
            "You are not able to uninvite the gang leader.\n"
        );
        h.line(bob, "promote cid");
        assert_eq!(
            h.line(ann, "uninvite member cid"),
            "You must be the gang leader to uninvite a lieutenant!\n"
        );
        h.line(bob, "demote cid");
        let (a, c) = h.both(ann, "uninvite member cid", cid);
        assert_eq!(cut_prompts(&a), "You have removed Cid from your gang.\n");
        assert_eq!(
            cut_prompts(&c),
            "Lieutenant Ann has exiled you from Wolves.\n"
        );
        assert_eq!(
            h.line(ann, "uninvite member cid"),
            "Cid is not in your gang.\n"
        );
        assert_eq!(
            h.line(cid, "uninvite member ann"),
            "UNINVITE {user name}\n".replace("UNINVITE", "Syntax: UNINVITE")
        );
        assert_eq!(
            h.line(cid, "join gang wolves"),
            "You have not been invited to join that gang!\n"
        );
        assert_eq!(h.gang("WOLVES").members, 2);
        assert_eq!(
            h.line(ann, "join gang owls"),
            "You may not join another gang!  You are already a member of one.\n"
        );
    }

    #[test]
    fn promote_and_demote_refusals() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        let cid = h.player("Cid", "Vale");
        h.line(bob, "invite member ann");
        h.line(ann, "join gang wolves");
        assert_eq!(h.line(bob, "promote"), "Syntax: PROMOTE {user name}\n");
        assert_eq!(
            h.line(bob, "promote bob"),
            "You may not demote yourself to lieutenant. Your gang needs a leader!\n"
        );
        assert_eq!(
            h.line(bob, "promote cid"),
            "You may not promote somebody who is not in your gang!.\n"
        );
        assert_eq!(h.line(ann, "promote bob"), "");
        assert_eq!(
            h.line(bob, "demote bob"),
            "You wish to demote yourself from leader of your gang?\n"
        );
        assert_eq!(
            h.line(bob, "demote cid"),
            "You may not demote someone who is not in your gang!\n"
        );
        assert_eq!(h.line(bob, "disband"), "Syntax: DISBAND {Party/Gang}\n");
        assert_eq!(
            h.line(bob, "disband club"),
            "Syntax: DISBAND {Party/Gang}\n"
        );
        assert_eq!(
            h.line(ann, "disband gang"),
            "You are not the leader of the gang; You may not disband it!\n"
        );
        assert_eq!(h.line(cid, "disband gang"), "You are not in a gang!\n");
        assert_eq!(
            h.line(cid, "gb hi"),
            "You are not in a gang at the present!\n"
        );
    }

    fn stored(name: &str, gang: &str, exp: u32, flags: PlayerFlags) -> Character {
        let mut c = silvermere_engine::clear_player();
        c.firstname = name.into();
        c.lastname = "Stored".into();
        c.gang = gang.into();
        c.exp = exp;
        c.flags = flags;
        c
    }

    // members off line: UNINVITE MEMBER, PROMOTE and DEMOTE ask the store
    #[test]
    fn offline_members_through_the_store() {
        let mut h = Harness::new();
        let (bob, _ann) = wolves(&mut h);
        h.w().gangs.get_mut("WOLVES").unwrap().members = 3;
        let from = h.log.len();
        assert_eq!(h.line(bob, "promote dan"), "");
        let q = h.queries(from);
        assert_eq!(
            q,
            [(
                bob,
                GangQuery::Offline {
                    name: "dan".into(),
                    gang: "Wolves".into(),
                    op: OfflineOp::Promote
                }
            )]
        );
        let dan = stored("Dan", "Wolves", 10, PlayerFlags::empty());
        let account = AccountId(77);
        let (query, d) = (q[0].1.clone(), dan.clone());
        let got = h.answer(bob, move |w| {
            gangs::on_query_answer(
                w,
                bob,
                query,
                GangAnswer::Offline(Some((account, Box::new(d)))),
            )
        });
        assert_eq!(
            got,
            "Gang member Dan will be notified of their promotion next time they log on.\n"
        );
        let saved = h.log.iter().rev().find_map(|o| match o {
            EngineOutput::Persist(PersistRequest::Character {
                account: a,
                character,
            }) => Some((*a, character.flags)),
            _ => None,
        });
        assert_eq!(saved, Some((account, PlayerFlags::PROMOTED)));
        h.store_caught_up();

        // already a lieutenant; an outsider; nobody (the Syntax line is never sent, sic)
        let lt = stored("Dan", "Wolves", 10, PlayerFlags::LIEUTENANT);
        let q = GangQuery::Offline {
            name: "dan".into(),
            gang: "Wolves".into(),
            op: OfflineOp::Promote,
        };
        let q2 = q.clone();
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q2,
                GangAnswer::Offline(Some((account, Box::new(lt))))
            )),
            "Gang member Dan is already a lieutenant in your gang.\n"
        );
        let out = stored("Dan", "Owls", 10, PlayerFlags::empty());
        let q2 = q.clone();
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q2,
                GangAnswer::Offline(Some((account, Box::new(out))))
            )),
            "Perhaps you should invite Dan into your gang first.\n"
        );
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q,
                GangAnswer::Offline(None)
            )),
            // goes out with the prompt
            "Syntax: PROMOTE {user name}\n"
        );

        h.store_caught_up();
        // DEMOTE: (sic) every member found gets the notice
        let q = GangQuery::Offline {
            name: "dan".into(),
            gang: "Wolves".into(),
            op: OfflineOp::Demote,
        };
        let d = dan.clone();
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q,
                GangAnswer::Offline(Some((account, Box::new(d))))
            )),
            "Gang member Dan will be notified of their demotion next time they log on.\n"
        );

        h.store_caught_up();
        // UNINVITE MEMBER of Dan
        let from = h.log.len();
        assert_eq!(h.line(bob, "uninvite member dan"), "");
        let q = h.queries(from)[0].1.clone();
        let d = dan.clone();
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q.clone(),
                GangAnswer::Offline(Some((account, Box::new(d))))
            )),
            "You removed Dan from your gang.\n"
        );
        assert_eq!(h.gang("WOLVES").members, 2);
        let saved = h.log.iter().rev().find_map(|o| match o {
            EngineOutput::Persist(PersistRequest::Character { character, .. }) => {
                Some(character.gang.clone())
            }
            _ => None,
        });
        assert_eq!(saved.as_deref(), Some(""));
        let q = GangQuery::Offline {
            name: "zed".into(),
            gang: "Wolves".into(),
            op: OfflineOp::Uninvite,
        };
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q,
                GangAnswer::Offline(None)
            )),
            "You don't see Zed here!\n"
        );
    }

    // the full list with a 35-character name cut to 29
    #[test]
    fn full_member_list() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        h.line(bob, "invite member ann");
        h.line(ann, "join gang wolves");
        h.line(bob, "promote ann");
        h.plr_mut(ann).lastname = "Abcdefghijklmnopqrstuvwxyzabcdefgh".into();
        h.w().gangs.get_mut("WOLVES").unwrap().members = 4;
        let from = h.log.len();
        let head = h.line(bob, "gb");
        assert_eq!(h.w().session(bob).unwrap().state, SessionState::Listing);
        let q = h.queries(from)[0].1.clone();
        assert_eq!(
            q,
            GangQuery::Members {
                gang: "Wolves".into()
            }
        );
        // lowest experience first: Eve (off line), the leader, Ann, Dan (lieutenant); every
        // line from the stored record, Online from who is in the game
        let rows = vec![
            stored("Eve", "Wolves", 1, PlayerFlags::empty()),
            stored("Bob", "Wolves", 5, PlayerFlags::empty()),
            Character {
                lastname: "Abcdefghijklmnopqrstuvwxyzabcdefgh".into(),
                ..stored("Ann", "Wolves", 6, PlayerFlags::LIEUTENANT)
            },
            stored("Dan", "wolves", 9, PlayerFlags::LIEUTENANT),
            stored("Fay", "Wolves", 12, PlayerFlags::empty()),
        ];
        let rest = h.answer(bob, move |w| {
            gangs::on_query_answer(w, bob, q, GangAnswer::Members(rows))
        });
        assert_eq!(h.w().session(bob).unwrap().state, SessionState::Playing);
        assert!(
            !h.w()
                .session(bob)
                .unwrap()
                .ust
                .flags
                .contains(SessionFlags::GANGLIST)
        );
        insta::assert_snapshot!("full_members", format!("{head}{rest}"));
        // a disbanded gang says so, and an absent leader has no Online
        let mut g = h.gang("WOLVES");
        g.flags.insert(GangFlags::DISBANDED);
        g.leader = "Zed".into();
        h.w().gangs.insert("WOLVES".into(), g);
        insta::assert_snapshot!("full_members_disbanded", h.line(ann, "gb"));
    }

    // the gang's share through add_experience
    #[test]
    fn kill_exp_goes_to_the_gang() {
        let mut h = Harness::new();
        let (bob, _) = wolves(&mut h);
        h.plr_mut(bob).level = 99;
        silvermere_engine::progression::experience::add_experience(h.w(), bob, 250, false);
        assert_eq!(h.gang("WOLVES").exp, 250);
        h.engine.handle(EngineInput::Tick);
        h.pump();
        assert_eq!(h.saved_gang("WOLVES").map(|g| g.exp), Some(250));
    }

    // u32::MAX worth of kills, by hand: 4 kills of 2^30 reach 2^32 - 1 + 1
    #[test]
    fn gang_exp_overflow_matches_a_hand_computation() {
        let mut g = h_gang();
        for _ in 0..4 {
            add_kill_exp(&mut g, 1 << 30);
        }
        // 3 * 2^30 = 3221225472; the fourth leaves 1 over the cap
        assert_eq!((g.exp, g.overexp, g.overflows), (u32::MAX, 1, 0));
        assert!(g.flags.contains(GangFlags::OVERFLOW));
        // another u32::MAX: overexp 1 + (2^32 - 1) wraps to 0 with room 2^32 - 2
        add_kill_exp(&mut g, u32::MAX);
        assert_eq!((g.exp, g.overexp, g.overflows), (u32::MAX, 1, 1));
    }

    fn h_gang() -> Gang {
        Gang {
            key: "WOLVES".into(),
            name: "Wolves".into(),
            exp: 0,
            leader: "Bob".into(),
            created: 0,
            members: 1,
            flags: GangFlags::empty(),
            overexp: 0,
            overflows: 0,
        }
    }

    proptest! {
        // wrapping on purpose: no panic for any sequence
        #[test]
        fn gang_exp_never_panics(start in any::<u32>(), kills in proptest::collection::vec(any::<u32>(), 0..40)) {
            let mut g = h_gang();
            g.exp = start;
            for k in kills {
                add_kill_exp(&mut g, k);
            }
        }
    }

    // the top gangs header and a row past the cap
    #[test]
    fn top_gang_overflow_row() {
        let mut h = Harness::new();
        let bob = h.player("Bob", "Smith");
        let mut g = h_gang();
        g.exp = u32::MAX;
        g.overexp = 77;
        g.overflows = 2;
        g.flags = GangFlags::OVERFLOW;
        g.created = 1_791_936_000;
        h.w().gangs.insert("WOLVES".into(), g);
        display_top_gangs(h.w(), bob, 10);
        h.engine.handle(EngineInput::Tick);
        h.pump();
        insta::assert_snapshot!("top_gang_overflow", cut_prompts(&h.text(0, bob)));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Invite(usize),
        Join(usize),
        Leave(usize),
        Uninvite(usize),
        Disband,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (1usize..4).prop_map(Op::Invite),
            (1usize..4).prop_map(Op::Join),
            (1usize..4).prop_map(Op::Leave),
            (1usize..4).prop_map(Op::Uninvite),
            Just(Op::Disband),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        // the member count equals the characters whose gang is the name
        #[test]
        fn member_count_matches_the_characters(ops in proptest::collection::vec(op(), 0..25)) {
            let mut h = Harness::new();
            let names = ["Bob", "Ann", "Cid", "Dan"];
            let ids: Vec<SessionId> = names.iter().map(|n| h.player(n, "X")).collect();
            h.line(ids[0], "create gang Wolves");
            for o in ops {
                match o {
                    Op::Invite(i) => { h.line(ids[0], &format!("invite member {}", names[i])); }
                    Op::Join(i) => { h.line(ids[i], "join gang wolves"); }
                    Op::Leave(i) => { h.line(ids[i], "leave gang"); }
                    Op::Uninvite(i) => { h.line(ids[0], &format!("uninvite member {}", names[i])); }
                    Op::Disband => { h.line(ids[0], "disband gang"); h.line(ids[0], "y"); }
                }
                let members = ids
                    .iter()
                    .filter(|&&i| h.plr(i).gang.eq_ignore_ascii_case("wolves"))
                    .count() as i16;
                prop_assert_eq!(h.gang("WOLVES").members, members);
            }
        }
    }

    fn controller_holder(h: &mut Harness, name: &str) -> SessionId {
        let id = h.player(name, "Smith");
        let c = h.plr_mut(id);
        c.room = silvermere_types::ids::RoomId(GANG_ROOM);
        c.items[0] = CONTROLLER;
        c.items[1] = DAGGER;
        c.items[2] = DAGGER;
        // the two daggers' weight
        c.wgt = 2;
        id
    }

    // STOCK, UNSTOCK, MARKUP and the takings
    #[test]
    fn gang_shop_commands() {
        let mut h = Harness::new();
        let bob = controller_holder(&mut h, "Bob");
        let ann = h.player("Ann", "Lee");
        h.plr_mut(ann).room = silvermere_types::ids::RoomId(GANG_ROOM);
        assert_eq!(
            h.line(ann, "stock dagger"),
            "You do not have the correct item to stock this shop.\n"
        );
        assert_eq!(
            h.line(ann, "stock"),
            "Syntax: STOCK {item} {price} {currency}\n"
        );
        let (me, her) = h.both(bob, "stock dagger 7 gold", ann);
        assert_eq!(
            cut_prompts(&me),
            "You add the dagger to your shops stock.\n"
        );
        assert_eq!(
            cut_prompts(&her),
            "You see Bob add a dagger to the shops stock.\n"
        );
        let shop = h.w().shops[&GANG_SHOP].clone();
        assert_eq!(
            (
                shop.stock[0].item,
                shop.stock[0].qty,
                shop.stock[0].regentime,
                shop.stock[0].regennum
            ),
            (DAGGER, 1, 7, Currency::Gold.code() as i16)
        );
        assert_eq!(gang_shop_owner(&shop), "BOB");
        // no price: the item's own; the same slot takes the copy
        h.line(bob, "stock dagger");
        let s = h.w().shops[&GANG_SHOP].stock[0];
        assert_eq!(
            (s.qty, s.regentime, s.regennum),
            (2, 12, Currency::Silver.code() as i16)
        );
        assert_eq!(
            h.line(bob, "markup 2000"),
            "New gang shop markup value set to 1000 percent.\n"
        );
        assert_eq!(
            h.line(bob, "markup -5"),
            "New gang shop markup value set to 0 percent.\n"
        );
        assert_eq!(
            h.line(ann, "markup 10"),
            "You do not have the correct item to set the markup value for this shop.\n"
        );
        assert_eq!(
            h.line(bob, "unstock dagger"),
            "You remove dagger from the shops stock.\n"
        );
        assert_eq!(h.w().shops[&GANG_SHOP].stock[0].qty, 1);
        // a sale pays the owner's book at the gang bank
        h.w().bank_books.insert(
            (AccountId(5), gangs::GANGBANK),
            BankBook {
                userid: "bob".into(),
                bank: gangs::GANGBANK,
                balance: 100,
                loan: 0,
            },
        );
        h.plr_mut(ann).coins.silver = 50;
        assert!(
            h.line(ann, "buy dagger")
                .starts_with("You just bought dagger for")
        );
        let book = &h.w().bank_books[&(AccountId(5), gangs::GANGBANK)];
        // the price paid (the cost is deposited): 12 silver, no markup, Charm 0
        let paid = silvermere_rules::economy::buy_price(12, Currency::Silver, 0, 0) as u32;
        assert_eq!(book.balance, 100 + paid);
        let s = h.w().shops[&GANG_SHOP].stock[0];
        assert_eq!((s.item, s.qty), (ItemNo(0), 0));
        assert!(h.log.iter().any(|o| matches!(
            o,
            EngineOutput::Persist(PersistRequest::GangShop(s)) if s.number == GANG_SHOP
        )));
        assert_eq!(
            h.line(bob, "unstock all"),
            "There are no stocked items to remove.\n"
        );
        h.plr_mut(bob).room = silvermere_types::ids::RoomId(ROOM);
        assert_eq!(
            h.line(bob, "unstock dagger"),
            "You cannot UNSTOCK if you are not in your gangs shop!\n"
        );
    }

    // cleanup_when_user_leaves: a logoff or a hang-up drops the
    // player's invitations, so the next player given the same session id has none
    #[test]
    fn invitations_go_at_logoff_and_hangup() {
        let mut h = Harness::new();
        let (bob, ann) = wolves(&mut h);
        let cid = h.player("Cid", "Vale");
        h.line(bob, "invite member ann");
        h.line(bob, "invite member cid");
        assert_eq!(h.w().gang_invites.len(), 2);
        silvermere_engine::session::on_logoff(h.w(), ann);
        assert!(h.w().gang_invites.iter().all(|g| g.session != ann));
        h.engine.handle(EngineInput::Disconnect { id: cid });
        h.pump();
        assert!(h.w().gang_invites.is_empty());
        // the freed id goes to a newcomer, who was invited to nothing
        let eve = h.player("Eve", "Moss");
        assert!(eve == ann || eve == cid);
        assert_eq!(
            h.line(eve, "join gang wolves"),
            "You have not been invited to join that gang!\n"
        );
    }

    // UNINVITE MEMBER of a name nobody has: the rewritten name is shown
    #[test]
    fn offline_uninvite_shows_the_rewritten_name() {
        let mut h = Harness::new();
        let (bob, _) = wolves(&mut h);
        let q = GangQuery::Offline {
            name: "bOB jR".into(),
            gang: "Wolves".into(),
            op: OfflineOp::Uninvite,
        };
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q,
                GangAnswer::Offline(None)
            )),
            "You don't see Bob jr here!\n"
        );
        let q = GangQuery::Offline {
            name: "zed".into(),
            gang: "Wolves".into(),
            op: OfflineOp::Demote,
        };
        assert_eq!(
            h.answer(bob, move |w| gangs::on_query_answer(
                w,
                bob,
                q,
                GangAnswer::Offline(None)
            )),
            "Syntax: DEMOTE {user name}\n"
        );
    }

    // STOCK drops the candidate list (clearing the output after an ambiguous match);
    // UNSTOCK sends it; prices and markup are 16-bit
    #[test]
    fn gang_shop_lists_and_16_bit_numbers() {
        let mut h = Harness::new();
        let bob = controller_holder(&mut h, "Bob");
        h.plr_mut(bob).items[3] = DAGNABIT;
        h.plr_mut(bob).wgt = 3;
        let got = h.line(bob, "stock dag 70000");
        assert!(!got.contains("specific"), "{got}");
        assert!(got.starts_with("You add the dag"), "{got}");
        // (unsigned)70000 = 4464
        assert_eq!(h.w().shops[&GANG_SHOP].stock[0].regentime, 4464);
        // the other one at 65536: 0 and not "free", so the item's own cost
        let got = h.line(bob, "stock dagnabit 65536");
        assert_eq!(got, "You add the dagnabit to your shops stock.\n");
        let s = h.w().shops[&GANG_SHOP].stock[1];
        assert_eq!((s.item, s.regentime), (DAGNABIT, 3));
        let got = h.line(bob, "unstock dag");
        assert!(got.contains("dagger") && got.contains("dagnabit"), "{got}");
        assert!(
            got.ends_with("This item is not currently in stock.\n"),
            "{got}"
        );
        // wrapped to signed 16 bits: 40000 is -25536, so 0; 65636 is 100
        assert_eq!(
            h.line(bob, "markup 40000"),
            "New gang shop markup value set to 0 percent.\n"
        );
        assert_eq!(
            h.line(bob, "markup 65636"),
            "New gang shop markup value set to 100 percent.\n"
        );
    }
}
