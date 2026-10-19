//! `SUICIDE`, `SUICIDE NOW` and `REROLL` (T-068): `cmd_suicide`, the `SUBST_SUICIDE`
//! password arm and `commit_suicide`, through the whole input pipeline: the refusals, the
//! password,
//! a life lost, the character deleted, the kept experience on the next `E`, and the log
//! events.

use crate::common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, special_rooms, tiny_world};
use silvermere_engine::player::profile;
use silvermere_engine::state::login::menu_bytes;
use silvermere_engine::{
    AccountRef, EchoMode, Engine, EngineInput, EngineOutput, GameEvent, PersistRequest, SeededRng,
    SessionId, World,
};
use silvermere_text::{Msg, strip_ansi_str};
use silvermere_types::enums::{
    Ability, MessageLevel, NewCharStep, RoomType, SessionState, SuicideStep,
};
use silvermere_types::flags::{PlayerSettings, PlayerStatus};
use silvermere_types::ids::{AccountId, ClassNo, ItemNo, MapId, RoomId, RoomKey};
use silvermere_types::records::{Character, Coins};

struct Harness {
    engine: Engine<TestClock, SeededRng>,
    clock: TestClock,
    rx: Receiver<EngineOutput>,
    log: Vec<EngineOutput>,
}

/// The room every test starts in, and the normal reincarnation room.
fn start() -> RoomKey {
    RoomKey::new(1, special_rooms()[0])
}

impl Harness {
    fn new() -> Self {
        Self::with(|_| {})
    }

    fn with(edit: impl FnOnce(&mut World)) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut world = tiny_world(8);
        edit(&mut world);
        let clock = TestClock::new();
        let engine = Engine::new(world, clock.clone(), SeededRng::new(7), tx);
        Self {
            engine,
            clock,
            rx,
            log: Vec::new(),
        }
    }

    /// A playing character with an account, 48 hit points, three lives.
    fn player(&mut self, name: &str) -> SessionId {
        let w = &mut self.engine.world;
        let id = enter(w, name, start().room.0, MessageLevel::High);
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
        c.settings |= PlayerSettings::VALIDATED;
        // made by 1.11p's create_player: no CHANGERACE menu line
        c.flags |= silvermere_types::flags::PlayerFlags::RACECHANGED;
        c.version = silvermere_engine::player::PLAYER_VERSION;
        (c.hp, c.maxhp) = (48, 48);
        c.lives = 3;
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

    fn state(&self, id: SessionId) -> SessionState {
        self.engine.world.session(id).unwrap().state
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
        let got = self.written(from, id);
        // every reply in the realm starts with the batch's colour reset
        let reset = bytes(Msg::AnsiNormal);
        match got.strip_prefix(reset.as_slice()) {
            Some(rest) => rest.to_vec(),
            None => got,
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

    /// Types `text` and returns what the session received, ANSI stripped, `\r\n` as `\n`.
    fn line(&mut self, id: SessionId, text: &str) -> String {
        let b = self.bytes(id, text);
        plain(&b)
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

    fn events(&self) -> Vec<&GameEvent> {
        self.log
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Event(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    fn echo(&self) -> Vec<EchoMode> {
        self.log
            .iter()
            .filter_map(|o| match o {
                EngineOutput::SetEcho { mode, .. } => Some(*mode),
                _ => None,
            })
            .collect()
    }
}

fn plain(b: &[u8]) -> String {
    strip_ansi_str(&String::from_utf8_lossy(b)).replace("\r\n", "\n")
}

/// The bytes a message renders to (default palette).
fn bytes(msg: Msg) -> Vec<u8> {
    silvermere_text::render(&msg, Default::default()).bytes
}

/// The text a message renders to, ANSI stripped, `\r\n` as `\n`.
fn text(msg: Msg) -> String {
    plain(&bytes(msg))
}

const PROMPT: &str = "[HP=48]:";

// ---------------------------------------------------------------------------------------
// The refusals

#[test]
fn refused_during_a_tournament() {
    let mut h = Harness::with(|w| w.config.tournament_play = true);
    let id = h.player("Ann");
    for cmd in ["suicide", "suicide now", "reroll"] {
        let out = h.line(id, cmd);
        assert_eq!(
            out,
            format!("You may not suicide or reroll during a tournament.\n{PROMPT}")
        );
        assert_eq!(out, format!("{}{PROMPT}", text(Msg::SuicideTournament)));
    }
    assert_eq!(h.plr(id).lives, 3);
    assert_eq!(h.state(id), SessionState::Playing);
}

#[test]
fn refused_in_a_colliseum_while_the_arena_is_safe() {
    let mut h = Harness::with(|w| {
        w.rooms.get_mut(&start()).unwrap().room_type = RoomType::Colliseum;
    });
    let id = h.player("Ann");
    h.plr_mut(id).status.insert(PlayerStatus::SNEAKING);
    for cmd in ["suicide", "suicide now", "reroll"] {
        let out = h.line(id, cmd);
        assert_eq!(
            out,
            format!("You may not suicide or reroll in this room!\n{PROMPT}")
        );
        assert_eq!(out, format!("{}{PROMPT}", text(Msg::SuicideRoomRefused)));
    }
    // refused before the flags are touched
    assert!(h.plr(id).status.contains(PlayerStatus::SNEAKING));
    assert_eq!(h.plr(id).lives, 3);

    // with the arena switch off the room is like any other
    h.engine.world.display.arena_safe = false;
    let out = h.line(id, "suicide");
    assert!(out.contains("You now have 2 lives remaining."), "{out:?}");
}

// More words than SUICIDE x, or any after REROLL: no command of ours
#[test]
fn extra_words_are_not_the_command() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    for cmd in ["suicide now please", "reroll now"] {
        let out = h.line(id, cmd);
        assert!(!out.contains("LONG thought"), "{cmd}: {out:?}");
        assert!(!out.contains("suicide password"), "{cmd}: {out:?}");
    }
    assert_eq!(h.plr(id).lives, 3);
    assert_eq!(h.state(id), SessionState::Playing);
    assert!(h.events().is_empty());
}

// ---------------------------------------------------------------------------------------
// SUICIDE with lives to spare: one life lost

#[test]
fn suicide_costs_a_life_and_wakes_up_in_the_reincarnation_room() {
    let mut h = Harness::with(|w| {
        // a room of its own away from the reincarnation room
        let mut r = silvermere_engine::world::blank::clear_room(RoomKey::new(1, 77));
        r.name = "Field".into();
        w.room_state
            .insert(r.key, silvermere_engine::world::RoomState::of(&r));
        w.rooms.insert(r.key, r);
        w.items.get_mut(&ItemNo(1)).unwrap().name = "ruby".into();
        let loyal = w.items.get_mut(&ItemNo(2)).unwrap();
        loyal.name = "ring".into();
        loyal.abilities[0] = (Ability::LoyalItem, 1);
    });
    let id = h.player("Ann");
    let bob = h.player("Bob");
    let field = RoomKey::new(1, 77);
    for who in [id, bob] {
        let c = h.plr_mut(who);
        c.map = field.map;
        c.room = field.room;
    }
    {
        let c = h.plr_mut(id);
        c.hp = 5;
        c.coins = Coins {
            gold: 7,
            copper: 3,
            ..Coins::default()
        };
        c.items[0] = ItemNo(1);
        c.itemuses[0] = 4;
        c.items[1] = ItemNo(2);
        c.keys[0] = ItemNo(1);
        c.keyuses[0] = 2;
        c.worn[0] = ItemNo(2);
        c.weapon = ItemNo(1);
        c.spells[0] = silvermere_types::ids::SpellNo(3);
        c.poison = 9;
        c.hidden = 1;
        c.status
            .insert(PlayerStatus::DRAGGING | PlayerStatus::SNEAKING);
    }
    // Ann drags Bob (session state: `Party::dragging`)
    h.engine.world.session_mut(id).unwrap().party.dragging = Some(bob);
    let mark = h.log.len();
    let got = h.bytes(id, "suicide");
    let mut want = bytes(Msg::SuicideLifeLost { lives: 2 });
    want.extend_from_slice(b"\r\n");
    let out = plain(&got);
    assert_eq!(
        out,
        format!(
            "\nAfter a LONG thought, you take your own life.\nYou now have 2 lives remaining.\n{PROMPT}"
        )
    );
    assert!(got.starts_with(&bytes(Msg::SuicideLifeLost { lives: 2 })));
    // the room hears it, with its prompt
    let heard = plain(&h.written(mark, bob));
    assert_eq!(heard, format!("Ann took their own life!\n{PROMPT}"));
    assert!(heard.starts_with(&text(Msg::SuicideRoom { who: "Ann".into() })));

    let c = h.plr(id);
    assert_eq!(c.lives, 2);
    // hp = maxhp (48) before the secondary-stat recalculation (CALC_KEEPCURRENT)
    assert_eq!(c.hp, 48);
    assert_eq!(
        (c.map, c.room),
        (MapId(1), RoomId(h.engine.world.config.reinc_room))
    );
    assert_eq!(c.coins, Coins::default());
    // the loyal ring stays, the ruby and the key are on the floor
    assert_eq!(c.items[0], ItemNo(0));
    assert_eq!(c.items[1], ItemNo(2));
    assert_eq!(c.keys[0], ItemNo(0));
    assert_eq!(c.keyuses[0], -2);
    assert_eq!(c.worn[0], ItemNo(0));
    assert_eq!(c.weapon, ItemNo(0));
    assert_eq!(c.spells[0].0, 0);
    assert_eq!(c.poison, 0);
    assert_eq!(c.hidden, 0);
    // dragging nobody
    assert_eq!(h.engine.world.session(id).unwrap().party.dragging, None);
    assert!(
        !c.status
            .intersects(PlayerStatus::DRAGGING | PlayerStatus::SNEAKING | PlayerStatus::SUICIDE)
    );
    let floor = &h.engine.world.room_state[&field];
    assert_eq!(floor.coins.gold, 7);
    assert_eq!(floor.coins.copper, 3);
    let on_floor: Vec<_> = floor
        .items
        .iter()
        .filter(|i| i.item.0 != 0)
        .map(|i| (i.item, i.uses))
        .collect();
    assert_eq!(on_floor, vec![(ItemNo(1), 4), (ItemNo(1), 2)]);
    // logged; not deleted
    assert_eq!(h.events(), vec![&GameEvent::Suicided { id }]);
    assert!(
        !h.persisted()
            .iter()
            .any(|p| matches!(p, PersistRequest::DeleteCharacter { .. }))
    );
    assert_eq!(h.state(id), SessionState::Playing);
}

// On the last life the loyal items go too; a destroy-on-death item
// "returns to its rightful place"
#[test]
fn rightful_place_and_loyal_items_on_the_last_life() {
    let mut h = Harness::with(|w| {
        let ruby = w.items.get_mut(&ItemNo(1)).unwrap();
        ruby.name = "ruby".into();
        ruby.destroy = 1;
        let loyal = w.items.get_mut(&ItemNo(2)).unwrap();
        loyal.name = "ring".into();
        loyal.abilities[0] = (Ability::LoyalItem, 1);
    });
    let id = h.player("Ann");
    {
        let c = h.plr_mut(id);
        c.lives = 1;
        c.items[0] = ItemNo(1);
        c.items[1] = ItemNo(2);
    }
    let got = h.bytes(id, "suicide");
    let returned = bytes(Msg::SuicideItemReturned {
        item: "ruby".into(),
    });
    assert!(got.starts_with(&returned));
    assert!(plain(&got).starts_with("Your ruby has returned to its rightful place.\n"));
    let floor = &h.engine.world.room_state[&start()];
    assert!(floor.items.iter().any(|i| i.item == ItemNo(2)));
    assert!(!floor.items.iter().any(|i| i.item == ItemNo(1)));
}

// ---------------------------------------------------------------------------------------
// The character deleted

/// What the deleting branch prints: the line, `ANSI_NORMAL`, the menu, `ANSI_NORMAL`.
fn deleted_bytes(h: &Harness, id: SessionId) -> Vec<u8> {
    let mut want = bytes(Msg::SuicideDeleted);
    want.extend(menu_bytes(&h.engine.world, id));
    want.extend(bytes(Msg::AnsiNormal));
    want
}

#[track_caller]
fn assert_deleted(h: &Harness, id: SessionId) {
    let s = h.engine.world.session(id).unwrap();
    assert_eq!(s.state, SessionState::MainMenu);
    assert!(s.character.is_none());
    assert!(!s.created);
    let account = s.account.as_ref().unwrap().id;
    assert!(h.persisted().iter().any(|p| matches!(
        p,
        PersistRequest::DeleteCharacter { account: a } if *a == account
    )));
    assert!(h.persisted().iter().any(|p| matches!(
        p,
        PersistRequest::Profile { account: a, .. } if *a == account
    )));
    assert!(h.events().contains(&&GameEvent::Rerolled { id }));
}

#[test]
fn suicide_on_the_last_life_deletes_the_character() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    h.plr_mut(id).lives = 1;
    let want = deleted_bytes(&h, id);
    let got = h.bytes(id, "suicide");
    assert_eq!(got, want);
    assert!(plain(&got).starts_with("\nAfter a LONG thought, you take your own life\n"));
    assert_deleted(&h, id);
}

#[test]
fn suicide_now_deletes_the_character_with_lives_left() {
    for cmd in ["suicide now", "SUICIDE N"] {
        let mut h = Harness::new();
        let id = h.player("Ann");
        let want = deleted_bytes(&h, id);
        assert_eq!(h.bytes(id, cmd), want, "{cmd}");
        assert_deleted(&h, id);
    }
}

// REROLL keeps the permrec share and the next `E` starts creation again with it
#[test]
fn reroll_keeps_the_experience_share_and_starts_creation_again() {
    let mut h = Harness::with(|w| {
        w.config.keep_exp = true;
        w.classes.get_mut(&ClassNo(1)).unwrap().exp = 100;
    });
    let id = h.player("Ann");
    {
        let c = h.plr_mut(id);
        c.level = 10;
        c.exprem = 5_000_000;
        c.settings |= PlayerSettings::KEEPEXP;
    }
    let kept = {
        let w = &h.engine.world;
        let c = h.plr(id);
        profile::kept_exp(c, &w.classes[&c.class], &w.races[&c.race], true)
    };
    assert!(kept.exprem > 0);
    let want = deleted_bytes(&h, id);
    assert_eq!(h.bytes(id, "reroll"), want);
    assert_deleted(&h, id);
    let account = h
        .engine
        .world
        .session(id)
        .unwrap()
        .account
        .as_ref()
        .unwrap()
        .id;
    let rec = profile::lookup(&h.engine.world, account).unwrap();
    assert_eq!((rec.expbil, rec.exprem), (kept.expbil, kept.exprem));

    // E: no character, so creation starts, with the kept experience
    let out = h.line(id, "E");
    assert!(
        out.contains("Please choose a race from the following list:"),
        "{out:?}"
    );
    assert_eq!(h.state(id), SessionState::NewChar(NewCharStep::Race));
    let c = h.plr(id);
    assert_eq!((c.expbil, c.exprem), (kept.expbil, kept.exprem));
}

// without KEEP the profile keeps nothing
#[test]
fn reroll_without_keep_keeps_no_experience() {
    let mut h = Harness::with(|w| w.config.keep_exp = true);
    let id = h.player("Ann");
    {
        let c = h.plr_mut(id);
        c.level = 10;
        c.exprem = 5_000_000;
        c.settings.remove(PlayerSettings::KEEPEXP);
    }
    h.line(id, "reroll");
    let account = h
        .engine
        .world
        .session(id)
        .unwrap()
        .account
        .as_ref()
        .unwrap()
        .id;
    let rec = profile::lookup(&h.engine.world, account).unwrap();
    assert_eq!((rec.expbil, rec.exprem), (0, 0));
}

// ---------------------------------------------------------------------------------------
// The suicide password

#[test]
fn a_password_is_asked_for_with_stars() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    h.plr_mut(id).suicpw = "secret".into();
    for cmd in ["suicide", "suicide now", "reroll"] {
        let mark = h.log.len();
        let got = h.bytes(id, cmd);
        assert_eq!(
            got,
            bytes(Msg::SuicideEnterPassword),
            "{cmd}: no prompt follows"
        );
        assert_eq!(plain(&got), "Enter your suicide password:");
        assert_eq!(h.state(id), SessionState::Suicide(SuicideStep::Confirm));
        assert!(h.log[mark..].iter().any(|o| matches!(
            o,
            EngineOutput::SetEcho {
                mode: EchoMode::Secure { ch: b'*', width: 8 },
                ..
            }
        )));
        // a wrong one: back to the realm, nothing done
        let out = h.line(id, "wrong");
        assert_eq!(out, format!("\nInvalid password specified.\n{PROMPT}"));
        assert_eq!(h.state(id), SessionState::Playing);
        assert_eq!(h.echo().last(), Some(&EchoMode::On));
    }
    // nothing typed is wrong too
    h.line(id, "suicide");
    let out = h.line(id, "");
    assert_eq!(out, format!("\nInvalid password specified.\n{PROMPT}"));
    assert_eq!(h.plr(id).lives, 3);
    assert!(h.events().is_empty());
}

#[test]
fn the_right_password_carries_it_out() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    h.plr_mut(id).suicpw = "secret".into();
    // SUICIDE: a life lost, the prompt
    h.line(id, "suicide");
    let out = h.line(id, "SECRET");
    assert_eq!(
        out,
        format!(
            "\nAfter a LONG thought, you take your own life.\nYou now have 2 lives remaining.\n{PROMPT}"
        )
    );
    assert_eq!(h.state(id), SessionState::Playing);
    assert_eq!(h.plr(id).lives, 2);
    // REROLL: deleted, the menu
    h.line(id, "reroll");
    let want = deleted_bytes(&h, id);
    assert_eq!(h.bytes(id, "secret"), want);
    assert_deleted(&h, id);
}

// commit_suicide: he no longer counts as having searched the room.
#[test]
fn suicide_clears_the_search_flags() {
    let mut h = Harness::new();
    let id = h.player("Ann");
    h.plr_mut(id).status.insert(PlayerStatus::SEARCHED);
    let st = h.engine.world.room_state.get_mut(&start()).unwrap();
    st.searchers = 1;
    st.foundmask = 1;
    h.line(id, "suicide now");
    let st = &h.engine.world.room_state[&start()];
    assert_eq!((st.searchers, st.foundmask), (0, 0));
}

// ---------------------------------------------------------------------------------------
// cleanup_when_user_leaves, before anything else

impl Harness {
    /// `leader` invites each follower, who follows him (INVITE, FOLLOW).
    fn party(&mut self, leader: SessionId, followers: &[SessionId]) {
        let lname = self.plr(leader).firstname.clone();
        for f in followers {
            let fname = self.plr(*f).firstname.clone();
            self.line(leader, &format!("invite {fname}"));
            self.line(*f, &format!("follow {lname}"));
        }
    }
}

fn esc(b: &[u8]) -> String {
    b.escape_ascii().to_string()
}

/// A field away from the reincarnation room (`start()`), for the tests where it matters
/// that the player wakes up somewhere else.
fn field_harness() -> Harness {
    Harness::with(|w| {
        let mut r = silvermere_engine::world::blank::clear_room(RoomKey::new(1, 77));
        r.name = "Field".into();
        w.room_state
            .insert(r.key, silvermere_engine::world::RoomState::of(&r));
        w.rooms.insert(r.key, r);
    })
}

/// The prompt as written after a line (from the start of the line).
const PROMPT_BYTES: &str = "\x1b[79D\x1b[K\x1b[0;37m[HP=48\x1b[0;37m]:";

// stop_following for the leaver: every follower of the leader
// leaves the party and is told "You are no longer following Ann." in light blue from a
// cleared line, with his prompt, in channel order; only then does the room hear the
// suicide
#[test]
fn a_leader_s_followers_are_told_he_is_gone() {
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    let cid = h.player("Cid");
    h.party(ann, &[bob, cid]);
    let mark = h.log.len();
    h.bytes(ann, "suicide");
    let told = format!(
        "\x1b[79D\x1b[K\x1b[1;34mYou are no longer following Ann.\r\n{PROMPT_BYTES}\
         \x1b[79D\x1b[KAnn took their own life!\r\n{PROMPT_BYTES}"
    );
    // the first write of the tick carries the batch's colour reset
    assert_eq!(
        esc(&h.written(mark, bob)),
        esc(format!("\x1b[0;37;40m{told}").as_bytes())
    );
    assert_eq!(esc(&h.written(mark, cid)), esc(told.as_bytes()));
    for f in [bob, cid] {
        assert_eq!(h.engine.world.session(f).unwrap().party.leader, None);
    }
}

// stop_following for a follower: nobody follows him, so
// nobody is told; remove_from_group crosses him off every list. (sic) His
// own `leader` is left as it is: only the -1 loop clears it, and he is not his own
// follower.
#[test]
fn a_follower_is_crossed_off_the_party() {
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    let cid = h.player("Cid");
    h.party(ann, &[bob, cid]);
    let mark = h.log.len();
    h.bytes(bob, "suicide");
    let heard = format!("\x1b[79D\x1b[KBob took their own life!\r\n{PROMPT_BYTES}");
    assert_eq!(esc(&h.written(mark, ann)), esc(heard.as_bytes()));
    assert_eq!(esc(&h.written(mark, cid)), esc(heard.as_bytes()));
    assert_eq!(
        esc(&h.bytes(ann, "par")),
        esc(format!(
            "\x1b[79D\x1b[K\x1b[0;37mThe following people are in your travel party:\r\n\
             \x20 Cid                            ()                    [H:100%]   - Midrank\r\n\
             \x20 Ann                            ()                    [H:100%]   - Frontrank\r\n\
             {PROMPT_BYTES}"
        )
        .as_bytes())
    );
    assert_eq!(
        esc(&h.bytes(cid, "par")),
        esc(format!(
            "\x1b[79D\x1b[K\x1b[1m\x1b[36mYou are following Ann.\r\n\
             \x1b[0;37mThe following people are in your travel party:\r\n\
             \x20 Ann                            ()                    [H:100%]   - Frontrank\r\n\
             \x20 Cid                            ()                    [H:100%]   - Midrank\r\n\
             {PROMPT_BYTES}"
        )
        .as_bytes())
    );
    assert_eq!(h.engine.world.session(bob).unwrap().party.leader, Some(ann));
    assert!(
        h.engine
            .world
            .session(bob)
            .unwrap()
            .party
            .group
            .iter()
            .all(Option::is_none)
    );
}

// kill_autocombat_against and kill_autocombat: whoever fights him, and he himself, leave
// the ring untold, so no round later breaks off with *Combat Off*; his energy above one
// round goes
#[test]
fn the_fights_end_untold() {
    use silvermere_engine::combat::AutoTarget;
    use silvermere_engine::combat::autocombat::{engage, is_inside_autocombat};
    use silvermere_rules::combat::Move;
    let mut h = field_harness();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    let cid = h.player("Cid");
    for who in [ann, bob, cid] {
        h.plr_mut(who).room = RoomId(77);
    }
    (h.plr_mut(ann).energy, h.plr_mut(ann).maxenergy) = (3000, 1000);
    engage(
        &mut h.engine.world,
        bob,
        Some(AutoTarget::Player(ann)),
        Move::Attack,
        None,
    );
    engage(
        &mut h.engine.world,
        ann,
        Some(AutoTarget::Player(cid)),
        Move::Attack,
        None,
    );
    h.bytes(ann, "suicide");
    assert!(!is_inside_autocombat(&h.engine.world, bob));
    assert!(!is_inside_autocombat(&h.engine.world, ann));
    assert_eq!(h.plr(ann).energy, 1000);
    // four energy rounds: a fight left running would break off
    let mark = h.log.len();
    h.clock.advance_ticks(20, &mut h.engine);
    h.pump();
    for who in [ann, bob, cid] {
        assert_eq!(esc(&h.written(mark, who)), "");
    }
}

// His pending commands go: the lines typed
// behind the SUICIDE while he waited are thrown away
#[test]
fn the_lines_typed_behind_it_are_thrown_away() {
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    h.plr_mut(ann).delay = 2;
    assert_eq!(h.bytes(ann, "suicide"), b"");
    assert_eq!(h.bytes(ann, "say hi"), b"");
    let mark = h.log.len();
    h.clock.advance_ticks(20, &mut h.engine);
    h.pump();
    assert_eq!(
        esc(&h.written(mark, ann)),
        esc(format!(
            "suicide\r\n\x1b[0;37;40m\x1b[1;31;40m\r\nAfter a LONG thought, you take your own life.\r\n\
             You now have 2 lives remaining.\r\n{PROMPT_BYTES}"
        )
        .as_bytes())
    );
    assert_eq!(
        esc(&h.written(mark, bob)),
        esc(format!("\x1b[79D\x1b[KAnn took their own life!\r\n{PROMPT_BYTES}").as_bytes())
    );
    assert!(h.engine.world.session(ann).unwrap().ust.pending.is_empty());
}

// clear_gang_invitations: his invitations
// are gone
#[test]
fn the_gang_invitations_go() {
    let mut h = Harness::with(|w| {
        w.gangs.insert(
            "WOLVES".into(),
            silvermere_types::records::Gang {
                key: "WOLVES".into(),
                name: "Wolves".into(),
                exp: 0,
                leader: "Bob".into(),
                created: 0,
                members: 1,
                flags: silvermere_types::flags::GangFlags::empty(),
                overexp: 0,
                overflows: 0,
            },
        );
    });
    let ann = h.player("Ann");
    h.engine
        .world
        .gang_invites
        .push(silvermere_engine::world::GangInvite {
            session: ann,
            gang: "Wolves".into(),
        });
    h.bytes(ann, "suicide");
    assert_eq!(
        esc(&h.bytes(ann, "join gang wolves")),
        esc(format!("You have not been invited to join that gang!\r\n{PROMPT_BYTES}").as_bytes())
    );
}

// stop_dragging_user: a downed player who takes his
// life is let go by whoever drags him, so DRAG alone has nobody to release
#[test]
fn whoever_drags_him_lets_go() {
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    h.plr_mut(ann).hp = -2;
    h.plr_mut(bob).status.insert(PlayerStatus::DRAGGING);
    h.engine.world.session_mut(bob).unwrap().party.dragging = Some(ann);
    h.bytes(ann, "suicide");
    assert_eq!(h.engine.world.session(bob).unwrap().party.dragging, None);
    assert!(!h.plr(bob).status.contains(PlayerStatus::DRAGGING));
    assert_eq!(esc(&h.bytes(bob, "drag")), esc(PROMPT_BYTES.as_bytes()));
}

/// The field (77) with the meadow (78) to its north, and back. Ann leads Bob and Cid
/// there; Bob is created first when `bob_first` (a lower session id than Ann's).
fn walk_party(bob_first: bool) -> (Harness, SessionId, SessionId, SessionId) {
    use silvermere_types::enums::ExitType;
    use silvermere_types::records::Exit;
    let mut h = Harness::with(|w| {
        let exit = |to: u32| Exit {
            target: RoomId(to),
            kind: ExitType::Normal,
            par1: 0,
            par2: 0,
            par3: 0,
            par4: 0,
        };
        let mut field = silvermere_engine::world::blank::clear_room(RoomKey::new(1, 77));
        field.name = "Field".into();
        field.exits[0] = exit(78);
        let mut meadow = silvermere_engine::world::blank::clear_room(RoomKey::new(1, 78));
        meadow.name = "Meadow".into();
        meadow.exits[1] = exit(77);
        for r in [field, meadow] {
            w.room_state
                .insert(r.key, silvermere_engine::world::RoomState::of(&r));
            w.rooms.insert(r.key, r);
        }
    });
    let (ann, bob) = if bob_first {
        let bob = h.player("Bob");
        (h.player("Ann"), bob)
    } else {
        let ann = h.player("Ann");
        (ann, h.player("Bob"))
    };
    let cid = h.player("Cid");
    for who in [ann, bob, cid] {
        let c = h.plr_mut(who);
        c.room = RoomId(77);
        // standing in the field, as move_user leaves the trail for the followers to read
        c.trailrooms[0] = RoomId(77);
        c.trailmaps[0] = MapId(1);
    }
    h.party(ann, &[bob, cid]);
    (h, ann, bob, cid)
}

/// Cid, on a higher session id than Ann, follows her north.
fn cid_follows() -> String {
    format!(
        "\x1b[79D\x1b[K\x1b[1;31mAnn\x1b[0;32m just left to  \x08the no \x08rth.\r\n{PROMPT_BYTES}\
         \x1b[79D\x1b[K\x1b[0;37m -- Following your Party leader n \x08orth --\r\n\
         \x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;36mMeadow\r\n\x1b[0;35mAlso here: \x1b[1;35mAnn\x1b[0;35m.\r\n\
         \x1b[0m\x1b[0;32mObvious exits: s \x08outh\r\n{PROMPT_BYTES}"
    )
}

// The follower who took his life keeps `leader` (see
// a_follower_is_crossed_off_the_party). On a higher channel than his leader he only
// follows when her trail leads out of his own room
// (direction_player_travelling): it never touches the reincarnation room,
// so her walk north does nothing to him and he hears nothing; he still names her
#[test]
fn a_higher_follower_who_took_his_life_stays_behind_untold() {
    let (mut h, ann, bob, cid) = walk_party(false);
    h.bytes(bob, "suicide");
    let mark = h.log.len();
    h.bytes(ann, "n");
    h.clock.advance_ticks(10, &mut h.engine);
    h.pump();
    assert_eq!(esc(&h.written(mark, bob)), "");
    assert_eq!(esc(&h.written(mark, cid)), esc(cid_follows().as_bytes()));
    assert_eq!(
        esc(&h.written(mark, ann)),
        esc(format!(
            "\x1b[0;37;40m\x1b[79D\x1b[K\x1b[1;36mMeadow\r\n\x1b[0;32mObvious exits: s \x08outh\r\n\
             {PROMPT_BYTES}\x1b[79D\x1b[K\x1b[1;31mCid\x1b[0;32m walks into the room from the s \x08outh.\r\n\
             {PROMPT_BYTES}"
        )
        .as_bytes())
    );
    assert_eq!(h.plr(bob).room, RoomId(h.engine.world.config.reinc_room));
    assert_eq!(h.engine.world.session(bob).unwrap().party.leader, Some(ann));
}

// On a lower channel the leader's own update walks him the same way from wherever he
// is (move_user as a follower's move): the reincarnation room has no
// exit north, so he reads "There is no exit in that direction!" from a cleared line,
// then his prompt, and leaves the party line (leader -1)
#[test]
fn a_lower_follower_who_took_his_life_finds_no_exit_and_lets_go() {
    let (mut h, ann, bob, cid) = walk_party(true);
    h.bytes(bob, "suicide");
    let mark = h.log.len();
    h.bytes(ann, "n");
    h.clock.advance_ticks(10, &mut h.engine);
    h.pump();
    assert_eq!(
        esc(&h.written(mark, bob)),
        esc(
            format!("\x1b[79D\x1b[KThere is no exit in that direction!\r\n{PROMPT_BYTES}")
                .as_bytes()
        )
    );
    assert_eq!(esc(&h.written(mark, cid)), esc(cid_follows().as_bytes()));
    assert_eq!(h.plr(bob).room, RoomId(h.engine.world.config.reinc_room));
    assert_eq!(h.engine.world.session(bob).unwrap().party.leader, None);
}

// cleanup_when_user_leaves and the deletion each call stop_following, which tells nobody
// without followers: nothing is sent before the player's own lines (or the deletion's),
// so the dispatcher's ANSI_NORMAL is still in the output buffer and leads them; the room's
// line comes after, without it.
#[test]
fn without_followers_the_reset_leads_the_players_own_lines() {
    const RESET: &[u8] = b"\x1b[0;37;40m";
    let mut h = Harness::new();
    let ann = h.player("Ann");
    let bob = h.player("Bob");
    let mark = h.log.len();
    h.bytes(ann, "suicide");
    let prompt = silvermere_engine::tell::prompt_bytes(&h.engine.world, ann);
    assert_eq!(
        h.written(mark, ann),
        [RESET, &bytes(Msg::SuicideLifeLost { lives: 2 }), &prompt].concat()
    );
    let prompt = silvermere_engine::tell::prompt_bytes(&h.engine.world, bob);
    assert_eq!(
        h.written(mark, bob),
        [
            b"\x1b[79D\x1b[KAnn took their own life!\r\n".as_slice(),
            &prompt
        ]
        .concat()
    );
    // the last life: deleted, the reset ahead of the farewell
    h.plr_mut(bob).lives = 1;
    let want = deleted_bytes(&h, bob);
    let mark = h.log.len();
    h.bytes(bob, "suicide");
    assert_eq!(h.written(mark, bob), [RESET, &want].concat());
}
