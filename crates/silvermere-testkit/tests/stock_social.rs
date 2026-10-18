//! Social actions (T-046): `handle_social_actions` and `display_list_of_actions`
//! through the whole input pipeline, against the stock verbs: the 64 records of the stock
//! actions file (name, 11 texts and target mask of each), decoded from the downloaded
//! distribution.

#[path = "../../silvermere-engine/tests/common/mod.rs"]
mod common;
#[path = "support/stock_records.rs"]
mod stock_records;

mod social {
    use super::common::{enter, special_rooms, tiny_world};
    use silvermere_engine::commands::execute_input;
    use silvermere_engine::commands::social::{
        TXT_FLOORROOM, TXT_FLOORSELF, TXT_INVROOM, TXT_INVSELF, TXT_MONROOM, TXT_MONSELF,
        TXT_SOLOROOM, TXT_SOLOSELF, TXT_USERROOM, TXT_USERSELF, TXT_USERTARGET, render,
    };
    use silvermere_engine::output::{PromptText, render_items};
    use silvermere_engine::{SessionId, World};
    use silvermere_text::colour::{ColRow, Palette, tell_colour};
    use silvermere_types::enums::MessageLevel;
    use silvermere_types::flags::{ActionTargets, PlayerSettings, PlayerStatus};
    use silvermere_types::ids::{ActiveMonsterId, ItemNo, MonsterNo, RoomKey};
    use silvermere_types::records::{Action, ActiveMonster};

    const ROW: u8 = ColRow::Action as u8;

    /// The stock verbs, in record order.
    fn stock() -> Vec<Action> {
        super::stock_records::content()
            .actions
            .iter()
            .map(|d| silvermere_import::decode::to_action(&d.record))
            .collect()
    }

    fn start() -> RoomKey {
        RoomKey::new(1, special_rooms()[0])
    }

    fn rat() -> ActiveMonster {
        use silvermere_types::enums::{Alignment, GroupRole, MonsterType};
        use silvermere_types::flags::ActiveMonsterFlags;
        use silvermere_types::ids::SpellNo;
        use silvermere_types::records::{ActiveSpell, Coins};
        use silvermere_types::sizes::{NAMTRAIL, NMONITEMS, NMONSPELLS};
        ActiveMonster {
            number: ActiveMonsterId(5),
            knownnum: MonsterNo(1),
            level: 0,
            map: 1.into(),
            room: start().room,
            poison: 0,
            energy: 0,
            hp: 5,
            following: String::new(),
            trail: [0.into(); NAMTRAIL],
            hunters: [ActiveMonsterId(0); 10],
            hunting: ActiveMonsterId(0),
            name: "giant rat".into(),
            wornitem: ItemNo(0),
            weapon: ItemNo(0),
            items: [(ItemNo(0), 0); NMONITEMS],
            cash: Coins::default(),
            maxhp: 5,
            alignment: Alignment::from_code(0),
            followpct: 0,
            dr: 0,
            ac: 0,
            exp: 0,
            maxenergy: 0,
            friendly: 0,
            genroom: 0.into(),
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

    /// A world with the stock verbs, a giant rat, a stone on the floor and a torch in
    /// Alice's pack, with Alice (female) and Bob (male) in the start room.
    struct Scene {
        w: World,
        alice: SessionId,
        bob: SessionId,
    }

    fn scene() -> Scene {
        let mut w = tiny_world(8);
        let verbs = stock();
        w.action_order = verbs.iter().map(|a| a.name.clone()).collect();
        w.actions = verbs
            .into_iter()
            .map(|a| (a.name.to_ascii_uppercase(), a))
            .collect();
        w.items.get_mut(&ItemNo(1)).unwrap().name = "torch".into();
        w.items.get_mut(&ItemNo(2)).unwrap().name = "stone".into();
        w.active_monsters.insert(rat());
        let st = w.room_state.get_mut(&start()).unwrap();
        st.monsters[0] = ActiveMonsterId(5);
        st.items[0].item = ItemNo(2);
        let alice = enter(&mut w, "Alice", start().room.0, MessageLevel::High);
        let bob = enter(&mut w, "Bob", start().room.0, MessageLevel::High);
        for (id, sex) in [(alice, b'F'), (bob, b'M')] {
            w.session_mut(id).unwrap().character.as_mut().unwrap().sex = sex;
        }
        let c = w.session_mut(alice).unwrap().character.as_mut().unwrap();
        c.items = vec![ItemNo(1)];
        Scene { w, alice, bob }
    }

    /// What `id` is sent so far as text: the escape sequences of the action lines named,
    /// `\r\n` as `|` and the prompt as `<P>`.
    fn out(w: &mut World, id: SessionId) -> String {
        let s = w.session_mut(id).unwrap();
        let palette = Palette::repaired(s.player().unwrap().palette);
        let (items, _) = s.output.take();
        let mut bytes = Vec::new();
        render_items(&mut bytes, &items, palette, PromptText::Marker(b"<P>"));
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        let col = String::from_utf8_lossy(tell_colour(ROW, palette)).into_owned();
        text = text.replace("\u{1b}[79D\u{1b}[K", "<CL>");
        if !col.is_empty() {
            text = text.replace(&col, "<A>");
        }
        text.replace("\r\n", "|")
    }

    /// `who` is `'a'` for Alice, `'b'` for Bob.
    /// Typed lines that fall through are spoken only with FASTTALK on: the
    /// speech tests turn it on for Alice.
    fn fasttalk(s: &mut Scene) {
        s.w.session_mut(s.alice)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
            .fasttalk = 1;
    }

    fn type_line(s: &mut Scene, who: char, line: &str) {
        let who = if who == 'a' { s.alice } else { s.bob };
        for id in [s.alice, s.bob] {
            s.w.session_mut(id).unwrap().output.clear();
        }
        execute_input(&mut s.w, who, line, false);
    }

    // The verb alone.
    #[test]
    fn verb_alone_texts_0_and_1() {
        let mut s = scene();
        type_line(&mut s, 'a', "smile");
        assert_eq!(out(&mut s.w, s.alice), "<A>You smile happily!|<P>");
        assert_eq!(out(&mut s.w, s.bob), "<A><CL>Alice smiles happily!|<P>");
    }

    // At a player: texts 2, 3, 4.
    #[test]
    fn verb_at_player_texts_2_3_4() {
        let mut s = scene();
        let carol = enter(&mut s.w, "Carol", start().room.0, MessageLevel::High);
        type_line(&mut s, 'a', "smile Bob");
        assert_eq!(out(&mut s.w, s.alice), "<A>You smile happily at Bob!|<P>");
        assert_eq!(
            out(&mut s.w, s.bob),
            "<CL><A>Alice smiles happily at you!|<P>"
        );
        assert_eq!(
            out(&mut s.w, carol),
            "<A><CL>Alice smiles happily at Bob!|<P>"
        );
    }

    // At a monster: texts 5 and 6; `%s` takes the monster's whole name.
    #[test]
    fn verb_at_monster_texts_5_6() {
        let mut s = scene();
        type_line(&mut s, 'a', "hug rat");
        assert_eq!(out(&mut s.w, s.alice), "<A>You hug giant rat!|<P>");
        assert_eq!(
            out(&mut s.w, s.bob),
            "<A><CL>Alice hugs giant rat close!|<P>"
        );
    }

    // A carried item: texts 7 and 8 with the possessive pronoun in the middle.
    #[test]
    fn verb_at_carried_item_texts_7_8() {
        let mut s = scene();
        type_line(&mut s, 'a', "kiss torch");
        assert_eq!(out(&mut s.w, s.alice), "<A>You kiss your torch!|<P>");
        assert_eq!(out(&mut s.w, s.bob), "<A><CL>Alice kisses her torch!|<P>");
    }

    // A floor item: texts 9 and 10.
    #[test]
    fn verb_at_floor_item_texts_9_10() {
        let mut s = scene();
        type_line(&mut s, 'a', "kiss stone");
        assert_eq!(out(&mut s.w, s.alice), "<A>You kiss the stone!|<P>");
        assert_eq!(out(&mut s.w, s.bob), "<A><CL>Alice kisses the stone!|<P>");
    }

    // A player's own name: texts 2, 3 and 4 all go to the actor; the
    // room (which excludes the actor twice) hears text 4.
    #[test]
    fn verb_at_oneself() {
        let mut s = scene();
        type_line(&mut s, 'a', "smile alice");
        assert_eq!(
            out(&mut s.w, s.alice),
            "<A>You smile happily at Alice!|<P><CL><A>Alice smiles happily at you!|<P>"
        );
        assert_eq!(
            out(&mut s.w, s.bob),
            "<A><CL>Alice smiles happily at Alice!|<P>"
        );
    }

    // The mask decides what a name can be: `wave` (7) has no floor-item text, so the line is
    // speech; a target the verb cannot take is never searched for.
    #[test]
    fn a_target_the_mask_does_not_allow_falls_through_to_speech() {
        let mut s = scene();
        fasttalk(&mut s);
        type_line(&mut s, 'a', "wave stone");
        let a = out(&mut s.w, s.alice);
        assert!(a.contains("You say \"wave stone\""), "{a}");
        // `hug` alone has no solo text (mask 30): speech too
        type_line(&mut s, 'a', "hug");
        assert!(out(&mut s.w, s.alice).contains("You say \"hug\""));
    }

    // `set actions off`, then the verb is spoken.
    #[test]
    fn actions_off_makes_the_verb_speech() {
        let mut s = scene();
        fasttalk(&mut s);
        s.w.session_mut(s.alice)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
            .settings
            .insert(PlayerSettings::NOACTIONS);
        type_line(&mut s, 'a', "smile");
        assert!(out(&mut s.w, s.alice).contains("You say \"smile\""));
        assert!(out(&mut s.w, s.bob).contains("Alice says \"smile\""));
    }

    // Acting gives a hidden or sneaking player away.
    #[test]
    fn performing_an_action_reveals_the_actor() {
        let mut s = scene();
        let c =
            s.w.session_mut(s.alice)
                .unwrap()
                .character
                .as_mut()
                .unwrap();
        c.hidden = 1;
        c.status.insert(PlayerStatus::SNEAKING);
        type_line(&mut s, 'a', "smile");
        let c = s.w.session(s.alice).unwrap().player().unwrap();
        assert_eq!(c.hidden, 0);
        assert!(!c.status.contains(PlayerStatus::SNEAKING));
    }

    // Another player who is hidden or sneaking can't be seen. The search skips a
    // hidden player altogether; a sneaking one is found and then refused, so the line
    // is spoken and the actor stays hidden.
    #[test]
    fn a_hidden_or_sneaking_target_is_not_acted_at() {
        let mut s = scene();
        fasttalk(&mut s);
        s.w.session_mut(s.bob)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
            .hidden = 1;
        type_line(&mut s, 'a', "smile Bob");
        assert!(out(&mut s.w, s.alice).contains("You say \"smile Bob\""));
    }

    #[test]
    fn a_sneaking_target_is_not_acted_at() {
        let mut s = scene();
        fasttalk(&mut s);
        s.w.session_mut(s.bob)
            .unwrap()
            .character
            .as_mut()
            .unwrap()
            .status
            .insert(PlayerStatus::SNEAKING);
        type_line(&mut s, 'a', "smile Bob");
        assert!(out(&mut s.w, s.alice).contains("You say \"smile Bob\""));
    }

    // Each reader's line starts with the colour table's action row in their own palette. Row 12 is
    // the same green in all four palettes, so two palettes cannot give
    // different bytes here; the test pins that each reader is served from the table with
    // his own palette.
    #[test]
    fn the_colour_is_the_readers_palette() {
        let mut s = scene();
        for p in 0..4u8 {
            s.w.session_mut(s.bob)
                .unwrap()
                .character
                .as_mut()
                .unwrap()
                .palette = p;
            type_line(&mut s, 'a', "smile Bob");
            let pal = Palette::repaired(p);
            let (items, _) = s.w.session_mut(s.bob).unwrap().output.take();
            let mut bytes = Vec::new();
            render_items(&mut bytes, &items, pal, PromptText::Marker(b""));
            let col = tell_colour(ROW, pal);
            assert!(!col.is_empty());
            let want: Vec<u8> = [
                &b"\x1b[79D\x1b[K"[..],
                col,
                b"Alice smiles happily at you!\r\n",
            ]
            .concat();
            assert_eq!(bytes, want, "palette {p}");
            s.w.session_mut(s.alice).unwrap().output.clear();
        }
    }

    // The whole word, no abbreviation; case does not matter.
    #[test]
    fn the_verb_is_matched_whole_and_in_any_case() {
        let mut s = scene();
        fasttalk(&mut s);
        type_line(&mut s, 'a', "SMILE");
        assert!(out(&mut s.w, s.alice).contains("You smile happily!"));
        type_line(&mut s, 'a', "smil");
        assert!(out(&mut s.w, s.alice).contains("You say \"smil\""));
    }

    // The `ACTION` list: every verb and one space, in WCCACTS
    // record order (Step First/Next), not alphabetical; the original's list was checked
    // against MBBSEmu.
    #[test]
    fn action_lists_every_verb_in_record_order() {
        let mut s = scene();
        type_line(&mut s, 'a', "action");
        let a = silvermere_text::strip_ansi_str(&out(&mut s.w, s.alice));
        assert!(
            a.starts_with("hug kiss wave grin bow bleed nod laugh shake "),
            "{a}"
        );
        assert!(a.ends_with("<P>"), "{a}");
        let verbs: Vec<&str> = a
            .trim_end_matches("<P>")
            .split(['|', ' '])
            .filter(|v| !v.is_empty())
            .collect();
        assert_eq!(verbs.len(), 64);
        // wrapped at 79 columns by the transport
        assert!(a.split('|').all(|l| l.len() <= 79), "{a}");
    }

    // The ACTION list reaches the player through `help actions` too.
    #[test]
    fn help_actions_lists_the_verbs() {
        let mut s = scene();
        type_line(&mut s, 'a', "help actions");
        let a = out(&mut s.w, s.alice);
        assert!(a.contains("hug kiss wave grin"), "{a}");
    }

    /// `%s` count the code supplies per text index.
    const SUPPLIED: [usize; 11] = [0, 2, 1, 1, 2, 1, 2, 1, 3, 1, 2];

    /// The text indices a mask uses.
    fn used(targets: ActionTargets) -> Vec<usize> {
        let mut v = Vec::new();
        for (bit, idx) in [
            (ActionTargets::SINGLE, &[TXT_SOLOSELF, TXT_SOLOROOM][..]),
            (
                ActionTargets::USER,
                &[TXT_USERSELF, TXT_USERTARGET, TXT_USERROOM][..],
            ),
            (ActionTargets::MONSTER, &[TXT_MONSELF, TXT_MONROOM][..]),
            (ActionTargets::INVENTORY, &[TXT_INVSELF, TXT_INVROOM][..]),
            (ActionTargets::FLOOR, &[TXT_FLOORSELF, TXT_FLOORROOM][..]),
        ] {
            if targets.contains(bit) {
                v.extend_from_slice(idx);
            }
        }
        v
    }

    // Done-when: every text of every stock verb renders without a leftover `%s`, and no text
    // asks for more names than the code supplies. The exceptions (fewer `%s` than supplied,
    // which print nothing extra) are the table below.
    #[test]
    fn every_stock_text_renders_cleanly() {
        let s = scene();
        let verbs = stock();
        println!("stock verbs: {}", verbs.len());
        assert_eq!(verbs.len(), 64);
        let mut fewer: Vec<(String, usize, usize)> = Vec::new();
        let mut empty_user_target = Vec::new();
        for a in &verbs {
            #[allow(clippy::needless_range_loop)]
            for i in 0..11 {
                let n = a.texts[i].matches("%s").count();
                let line = render(
                    &s.w,
                    s.alice,
                    &a.texts[i],
                    Some("Bob"),
                    &["Bob", "his", "torch"],
                );
                assert!(
                    !line.contains("%s") && !line.contains("%p"),
                    "{} {i}: {line}",
                    a.name
                );
                if used(a.targets).contains(&i) {
                    assert!(
                        n <= SUPPLIED[i],
                        "{} text {i} wants {n} names, code gives {}",
                        a.name,
                        SUPPLIED[i]
                    );
                    if n < SUPPLIED[i] {
                        fewer.push((a.name.clone(), i, n));
                    }
                }
            }
            if a.targets.contains(ActionTargets::USER) && a.texts[TXT_USERTARGET].is_empty() {
                empty_user_target.push(a.name.clone());
            }
        }
        // every exception is a one-name SOLOROOM (the second name, `his/her`, is unused),
        // except `shake`'s INVROOM, which has `%p` in the slot where the code's second name
        // is: "Alice shakes her head at her." (a data quirk kept)
        for (name, i, n) in &fewer {
            let ok = (*i == TXT_SOLOROOM && *n == 1)
                || (name == "shake" && *i == TXT_INVROOM && *n == 2);
            assert!(ok, "unexpected exception {name} {i} {n}");
        }
        assert_eq!(fewer.iter().filter(|e| e.1 != TXT_SOLOROOM).count(), 1);
        // aimed at a player with an empty text 3: the original prints the empty line; none in stock
        assert!(empty_user_target.is_empty(), "{empty_user_target:?}");
        // the keep flag is never set in stock
        assert!(verbs.iter().all(|a| a.targets.bits() & 0x40 == 0));
    }

    // `shake` aimed at a carried item: the data's second `%s` receives the pronoun.
    #[test]
    fn shake_at_an_item_shows_the_data_quirk() {
        let mut s = scene();
        type_line(&mut s, 'a', "shake torch");
        assert_eq!(
            out(&mut s.w, s.bob),
            "<A><CL>Alice shakes her head at her.|<P>"
        );
    }

    // The verb table does not shadow commands (stage 6 comes first): `bow` is no command,
    // but `look` is; and an action never starts for a word the verbs lack.
    #[test]
    fn commands_come_before_actions() {
        let mut s = scene();
        s.w.actions.insert(
            "LOOK".into(),
            Action {
                name: "look".into(),
                texts: std::array::from_fn(|_| "ACTION LOOK".to_string()),
                targets: ActionTargets::SINGLE,
            },
        );
        type_line(&mut s, 'a', "look");
        let a = out(&mut s.w, s.alice);
        assert!(!a.contains("ACTION LOOK"), "{a}");
    }
}
