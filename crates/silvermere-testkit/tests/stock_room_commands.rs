//! Room commands (T-144) on the stock world over telnet: the Iceforge's summons after a kill
//! in the room (public issue #5), the Ice Sorceress's throne and the Blood Champion's gem;
//! and a sweep of every stock room command text, each clause parsed and every number it
//! names looked up, then each room's commands run once.
//!
//! Stock records: 3:632 The Iceforge (command text 476: `touch hydra` and `touch giant`
//! clear the frozen hydra 767 or the frozen giant 768 from the floor, message 1114 when it
//! is gone, and summon the frost hydra 316 or the frost giant 756, both `maxalive` 1), 10:245
//! Tower, Open Chamber (text 4700, specific monster 108), 7:142 Large Clearing, Stone Altar
//! (text 4703, specific monster 122).

use crate::common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use silvermere_engine::scripts::{Clause, Keyword, Teleport, clauses, pattern_lines, whole_lines};
use silvermere_engine::world::World;
use silvermere_types::ids::{
    ClassNo, ItemNo, MessageNo, MonsterNo, RaceNo, RoomKey, SpellNo, TextBlockNo,
};

const WARRIOR: u16 = 1;

/// A warrior of level `level` with hit points to spare, placed in `map:room`.
async fn hero(
    r: &silvermere::Running,
    url: &str,
    name: &str,
    level: u16,
    (map, room): (u32, u32),
) -> silvermere_testkit::TelnetClient {
    let c = new_character(r, url, name, WARRIOR, &[]).await;
    edit_player(r, name, move |p| {
        p.level = level;
        p.maxhp = 30000;
        p.hp = 30000;
    })
    .await;
    place(r, name, map, room).await;
    c
}

/// The active monsters of known monster `monster` in `map:room`.
async fn here(r: &silvermere::Running, (map, room): (u32, u32), monster: i32) -> usize {
    on_world(r, move |w| {
        w.room_state[&RoomKey::new(map, room)]
            .monsters
            .iter()
            .filter(|m| m.0 != 0)
            .filter_map(|m| w.active_monsters.by_number(*m))
            .filter(|a| a.knownnum == MonsterNo(monster))
            .count()
    })
    .await
}

/// Item `item` lies in `map:room`, visible or hidden.
async fn lies(r: &silvermere::Running, (map, room): (u32, u32), item: i32) -> bool {
    on_world(r, move |w| {
        let s = &w.room_state[&RoomKey::new(map, room)];
        s.items
            .iter()
            .chain(s.hiditems.iter())
            .any(|i| i.item == ItemNo(item))
    })
    .await
}

/// Line 0 of message `n`, cut before its first argument: the part a reply shows verbatim.
async fn message(r: &silvermere::Running, n: i32) -> String {
    on_world(r, move |w| {
        let t = w.messages[&MessageNo(n)].msgs[0].clone();
        let t = t.split('%').next().unwrap_or_default().trim().to_owned();
        t.chars().take(30).collect::<String>().trim().to_owned()
    })
    .await
}

/// `name`'s quest experience (`expbil` billions and `exprem`) and the value of ability
/// `ability` in the record's own slots.
async fn exp_and(r: &silvermere::Running, name: &str, ability: u16) -> (u64, Option<i16>) {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        let p = w.session(id).unwrap().character.as_ref().unwrap();
        let v = p.abils.iter().find(|s| s.0.code() == ability).map(|s| s.1);
        (u64::from(p.expbil) * 1_000_000_000 + u64::from(p.exprem), v)
    })
    .await
}

/// The one monster of known monster `monster` in `at` killed by `name` through the
/// game's death path (`check_kill_monster`): the room's last kill, its counters and the
/// monster's own count as after a fight.
async fn kill(r: &silvermere::Running, name: &str, (map, room): (u32, u32), monster: i32) {
    let name = name.to_owned();
    on_world(r, move |w| {
        let id = session_of(w, &name);
        let a = w
            .active_monsters
            .values_mut()
            .find(|a| a.knownnum == MonsterNo(monster) && a.map.0 == map && a.room.0 == room)
            .expect("the monster to kill");
        a.hp = 0;
        let n = a.number;
        assert!(silvermere_engine::combat::kill_monster::check_kill_monster(
            w,
            n,
            Some(id)
        ));
    })
    .await;
}

/// Every monster in `map:room` goes.
async fn clear_monsters(r: &silvermere::Running, (map, room): (u32, u32)) {
    on_world(r, move |w| {
        let gone: Vec<_> = w.room_state[&RoomKey::new(map, room)]
            .monsters
            .iter()
            .copied()
            .filter(|m| m.0 != 0)
            .collect();
        for n in gone {
            silvermere_engine::monsters::active::remove_active_monster(w, n, false);
        }
    })
    .await;
}

const ICEFORGE: (u32, u32) = (3, 632);
const FROZEN_HYDRA: i32 = 767;
const FROZEN_GIANT: i32 = 768;
const FROST_HYDRA: i32 = 316;
const FROST_GIANT: i32 = 756;

/// Issue #5: in the Iceforge, `touch giant` summons the frost giant; the party kills it,
/// and `touch hydra` then takes the frozen hydra and summons the frost hydra: a summon
/// ignores the room's regeneration delay after a kill (and its cap and area limit), as the
/// original's `summon` generates forced.
#[tokio::test(flavor = "multi_thread")]
async fn iceforge_summons_after_a_kill_in_the_room() {
    let db = stock_db();
    let r = start(&db.url, 144).await;
    let mut c = hero(&r, &db.url, "Brann", 20, ICEFORGE).await;
    assert!(
        lies(&r, ICEFORGE, FROZEN_HYDRA).await,
        "the frozen hydra at start"
    );
    assert!(
        lies(&r, ICEFORGE, FROZEN_GIANT).await,
        "the frozen giant at start"
    );

    // the giant, first: no kill in the room yet
    let giant = cmd(&mut c, "touch giant").await;
    assert_eq!(here(&r, ICEFORGE, FROST_GIANT).await, 1, "{giant}");
    assert!(!lies(&r, ICEFORGE, FROZEN_GIANT).await, "{giant}");

    // the giant is killed: the room's own spawns now wait out the regeneration delay
    kill(&r, "Brann", ICEFORGE, FROST_GIANT).await;
    let refused = on_world(&r, |w| {
        use silvermere_engine::monsters::generate::{Request, generate_monster};
        let key = RoomKey::new(3, 632);
        // (a kill stamped at minute 0 reads as no kill, as on the original)
        w.room_state[&key].lastkill == 0
            || generate_monster(
                w,
                key,
                Request {
                    areatype: 0,
                    monnum: MonsterNo(FROST_HYDRA),
                    minlevel: 0,
                    maxlevel: 0x7FFF,
                    owner: None,
                    ally: false,
                    force: false,
                },
            )
            .is_none()
    })
    .await;
    assert!(refused, "an unforced spawn right after the kill");

    // the hydra, right after the kill
    let hydra = cmd(&mut c, "touch hydra").await;
    assert!(!lies(&r, ICEFORGE, FROZEN_HYDRA).await, "{hydra}");
    assert_eq!(
        here(&r, ICEFORGE, FROST_HYDRA).await,
        1,
        "the frost hydra after a kill in the room:\n{hydra}"
    );
    r.stop().await.unwrap();
}

/// With the frozen hydra gone from the floor, `touch hydra` shows message 1114 and
/// summons nothing.
#[tokio::test(flavor = "multi_thread")]
async fn iceforge_summons_nothing_without_the_item() {
    let db = stock_db();
    let r = start(&db.url, 145).await;
    let mut c = hero(&r, &db.url, "Brann", 20, ICEFORGE).await;
    on_world(&r, |w| {
        silvermere_engine::scripts::actions::erase_item_from_room(
            w,
            RoomKey::new(3, 632),
            ItemNo(FROZEN_HYDRA),
        );
    })
    .await;
    assert!(!lies(&r, ICEFORGE, FROZEN_HYDRA).await);
    let gone = message(&r, 1114).await;
    let out = cmd(&mut c, "touch hydra").await;
    assert!(out.contains(&gone), "message 1114 ({gone:?}):\n{out}");
    assert_eq!(here(&r, ICEFORGE, FROST_HYDRA).await, 0, "{out}");
    r.stop().await.unwrap();
}

const THRONE: (u32, u32) = (10, 245);
const ICE_SORCERESS: i32 = 108;

/// The Ice Sorceress's throne (text 4700): `sit throne` is refused while a monster is in
/// the room (message 1093) and below level 15 (message 3383); with her killed, a level-15
/// character gains 750000 experience (as quest experience), +1 Alter Defence Value
/// (ability 2) and the quest mark (ability 125 at 2), with message 3384; a second
/// `sit throne` gives nothing more.
#[tokio::test(flavor = "multi_thread")]
async fn ice_sorceress_throne_rewards_once() {
    let db = stock_db();
    let r = start(&db.url, 146).await;
    let mut c = hero(&r, &db.url, "Mira", 15, THRONE).await;
    spawn(&r, THRONE.0, THRONE.1, ICE_SORCERESS as u16).await;
    let (busy, low, gain) = (
        message(&r, 1093).await,
        message(&r, 3383).await,
        message(&r, 3384).await,
    );

    // she is here
    let before = exp_and(&r, "Mira", 2).await;
    let out = cmd(&mut c, "sit throne").await;
    assert!(out.contains(&busy), "message 1093 ({busy:?}):\n{out}");
    assert_eq!(exp_and(&r, "Mira", 2).await, before, "{out}");

    // she is killed
    kill(&r, "Mira", THRONE, ICE_SORCERESS).await;

    // below level 15
    edit_player(&r, "Mira", |p| p.level = 14).await;
    let before = exp_and(&r, "Mira", 2).await;
    let out = cmd(&mut c, "sit throne").await;
    assert!(out.contains(&low), "message 3383 ({low:?}):\n{out}");
    assert_eq!(exp_and(&r, "Mira", 2).await, before, "{out}");
    assert_eq!(exp_and(&r, "Mira", 125).await.1, None, "{out}");

    // level 15: the reward
    edit_player(&r, "Mira", |p| p.level = 15).await;
    let (exp0, ac0) = exp_and(&r, "Mira", 2).await;
    let out = cmd(&mut c, "sit throne").await;
    let (exp1, ac1) = exp_and(&r, "Mira", 2).await;
    assert!(out.contains(&gain), "message 3384 ({gain:?}):\n{out}");
    assert_eq!(exp1, exp0 + 750_000, "experience:\n{out}");
    assert_eq!(ac1.unwrap_or(0), ac0.unwrap_or(0) + 1, "ability 2:\n{out}");
    assert_eq!(
        exp_and(&r, "Mira", 125).await.1,
        Some(2),
        "ability 125:\n{out}"
    );

    // once only
    let out = cmd(&mut c, "sit throne").await;
    assert_eq!(
        exp_and(&r, "Mira", 2).await,
        (exp1, ac1),
        "a second sit:\n{out}"
    );
    assert!(!out.contains(&gain), "{out}");
    r.stop().await.unwrap();
}

const ALTAR: (u32, u32) = (7, 142);

/// The Blood Champion's gem (text 4703): with a monster in the room `touch gem` gives
/// message 289 and nothing else; with none it casts spell 214 and teleports to 7:20, and a
/// level-15 character without ability 129 gains +1 Spellcasting (ability 70), the quest
/// mark (129 at 2) and 750000 quest experience, once.
#[tokio::test(flavor = "multi_thread")]
async fn blood_champion_gem_rewards_once() {
    let db = stock_db();
    let r = start(&db.url, 147).await;
    let mut c = hero(&r, &db.url, "Odo", 15, ALTAR).await;
    spawn(&r, ALTAR.0, ALTAR.1, 122).await;
    let (busy, moved, gain) = (
        message(&r, 289).await,
        message(&r, 291).await,
        message(&r, 3388).await,
    );
    // the spell blinds, so no room display follows the teleport: message 291 ends it
    let after = regex::escape(&moved);

    let before = exp_and(&r, "Odo", 70).await;
    let out = cmd(&mut c, "touch gem").await;
    assert!(out.contains(&busy), "message 289 ({busy:?}):\n{out}");
    assert_eq!(room_of(&r, "Odo").await, ALTAR.1, "no teleport:\n{out}");
    assert_eq!(exp_and(&r, "Odo", 70).await, before, "{out}");

    clear_monsters(&r, ALTAR).await;
    let (exp0, sc0) = exp_and(&r, "Odo", 70).await;
    let out = cmd_until(&mut c, "touch gem", &after).await;
    let out = out + &cmd(&mut c, "").await;
    let (exp1, sc1) = exp_and(&r, "Odo", 70).await;
    let at = on_world(&r, |w| {
        let id = session_of(w, "Odo");
        w.session(id).unwrap().room_key().unwrap()
    })
    .await;
    assert_eq!(at, RoomKey::new(7, 20), "the teleport:\n{out}");
    assert!(out.contains(&gain), "message 3388 ({gain:?}):\n{out}");
    assert_eq!(exp1, exp0 + 750_000, "experience:\n{out}");
    assert_eq!(sc1.unwrap_or(0), sc0.unwrap_or(0) + 1, "ability 70:\n{out}");
    assert_eq!(
        exp_and(&r, "Odo", 129).await.1,
        Some(2),
        "ability 129:\n{out}"
    );

    // again: the cast and the teleport, no second reward
    place(&r, "Odo", ALTAR.0, ALTAR.1).await;
    clear_monsters(&r, ALTAR).await;
    let out = cmd_until(&mut c, "touch gem", &after).await;
    let out = out + &cmd(&mut c, "").await;
    assert_eq!(room_of(&r, "Odo").await, 20, "the teleport again:\n{out}");
    assert_eq!(
        exp_and(&r, "Odo", 70).await,
        (exp1, sc1),
        "a second touch:\n{out}"
    );
    assert!(!out.contains(&gain), "{out}");
    r.stop().await.unwrap();
}

/// A number a clause names that the world does not have.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Missing {
    /// The text block and the clause.
    pub block: i32,
    pub clause: String,
    /// What is missing: `item 12`, `room 7:20`, ...
    pub what: String,
}

/// What [`sweep`] found.
#[derive(Debug, Default)]
pub struct Sweep {
    /// Rooms with a command text, and the blocks walked from them.
    pub rooms: usize,
    pub blocks: usize,
    pub lines: usize,
    pub clauses: usize,
    pub keywords: BTreeMap<Keyword, usize>,
    /// Clauses with no keyword (skipped, the line going on), by text: the blocks that
    /// hold them.
    pub unknown: BTreeMap<String, BTreeSet<i32>>,
    /// Numbers that name nothing.
    pub missing: BTreeSet<Missing>,
}

/// Every room command text of `w` and every block its clauses reach (`random` blocks,
/// `checkspell` and `testskill` blocks run as whole blocks, and the rooms and exits a
/// `remoteaction` reaches), each clause parsed and each number it names looked up: items,
/// monsters, messages, spells, rooms, exits, classes, races and text blocks. Works on any
/// world, so an imported realm can be checked the same way.
pub fn sweep(w: &World) -> Sweep {
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Kind {
        Pattern,
        Whole,
    }
    let parts = |n: i32| -> Vec<String> {
        (0..silvermere_engine::scripts::MAX_RECORDS)
            .map_while(|p| {
                w.text_blocks
                    .get(&(p, TextBlockNo(n)))
                    .map(|t| t.text.clone())
            })
            .collect()
    };
    let teleports = &silvermere_engine::scripts::STOCK_TELEPORTS;
    let mut s = Sweep::default();
    let mut todo: Vec<(Kind, i32, i16)> = Vec::new();
    let mut rooms: Vec<&silvermere_types::records::Room> =
        w.rooms.values().filter(|r| r.cmdtext.0 != 0).collect();
    rooms.sort_by_key(|r| (r.key.map.0, r.key.room.0));
    s.rooms = rooms.len();
    for rm in rooms {
        todo.push((Kind::Pattern, rm.cmdtext.0, rm.key.map.0 as i16));
    }
    let mut seen = BTreeSet::new();
    while let Some((kind, n, map)) = todo.pop() {
        if !seen.insert((kind, n, map)) {
            continue;
        }
        s.blocks += 1;
        let text = parts(n);
        let mut miss = |clause: &str, what: String| {
            s.missing.insert(Missing {
                block: n,
                clause: clause.trim().to_owned(),
                what,
            });
        };
        if text.is_empty() {
            miss("", format!("text block {n}"));
            continue;
        }
        for t in text {
            let lines: Vec<String> = match kind {
                Kind::Whole => whole_lines(&t).into_iter().map(String::from).collect(),
                Kind::Pattern => pattern_lines(&t).into_iter().map(|p| p.action).collect(),
            };
            for line in lines {
                s.lines += 1;
                for (text, _) in clauses(&line) {
                    let c = Clause::parse(text, teleports);
                    s.clauses += 1;
                    if let Some(k) = c.keyword() {
                        *s.keywords.entry(k).or_default() += 1;
                    }
                    let item = |n: i32| {
                        (n != 0 && !w.items.contains_key(&ItemNo(n))).then(|| format!("item {n}"))
                    };
                    let msg = |n: i32| {
                        (n != 0 && !w.messages.contains_key(&MessageNo(n)))
                            .then(|| format!("message {n}"))
                    };
                    let block = |n: i32| {
                        (n != 0 && !w.text_blocks.contains_key(&(0, TextBlockNo(n))))
                            .then(|| format!("text block {n}"))
                    };
                    let spell = |n: i32| {
                        (n != 0 && !w.spells.contains_key(&SpellNo(n as i16)))
                            .then(|| format!("spell {n}"))
                    };
                    let monster = |n: i32| {
                        (!w.monsters.contains_key(&MonsterNo(n))).then(|| format!("monster {n}"))
                    };
                    let room = |r: i32, m: i32| {
                        (!w.rooms.contains_key(&RoomKey::new(m as u32, r as u32)))
                            .then(|| format!("room {m}:{r}"))
                    };
                    let found: Vec<Option<String>> = match &c {
                        Clause::Unknown(u) => {
                            if !u.trim().is_empty() {
                                s.unknown.entry(u.trim().to_owned()).or_default().insert(n);
                            }
                            vec![]
                        }
                        Clause::Price { msg: m, .. }
                        | Clause::NoMonsters { msg: m }
                        | Clause::Monsters { msg: m }
                        | Clause::Message { msg: m }
                        | Clause::GoodAligned { msg: m, .. }
                        | Clause::EvilAligned { msg: m, .. }
                        | Clause::MinLevel { msg: m, .. }
                        | Clause::MaxLevel { msg: m, .. }
                        | Clause::Flag { msg: m, .. } => vec![msg(*m)],
                        Clause::FailAbility { msg: m, .. } | Clause::CheckSkill { msg: m, .. } => {
                            vec![msg(i32::from(*m))]
                        }
                        Clause::Cast { spell: n } => vec![spell(*n)],
                        Clause::LearnSpell { spell: n } => vec![spell(i32::from(*n))],
                        Clause::NeedMonster { monster: n, msg: m } => vec![monster(*n), msg(*m)],
                        Clause::Summon { monster: n } => vec![monster(*n)],
                        Clause::GiveItem { item: n } | Clause::HideItem { item: n } => {
                            vec![item(*n)]
                        }
                        Clause::TakeItem { item: n, msg: m }
                        | Clause::CheckItem { item: n, msg: m }
                        | Clause::FailItem { item: n, msg: m }
                        | Clause::RoomItem { item: n, msg: m }
                        | Clause::FailRoomItem { item: n, msg: m }
                        | Clause::ClearItem { item: n, msg: m } => vec![item(*n), msg(*m)],
                        Clause::Text { block: n } | Clause::RoomText { block: n } => {
                            vec![block(*n)]
                        }
                        Clause::Class { class } => vec![
                            (!w.classes.contains_key(&ClassNo(*class as _)))
                                .then(|| format!("class {class}")),
                        ],
                        Clause::Race { race } => vec![
                            (!w.races.contains_key(&RaceNo(*race as _)))
                                .then(|| format!("race {race}")),
                        ],
                        Clause::Teleport(Teleport::Room { room: r, map: m }) => {
                            vec![room(*r, *m)]
                        }
                        Clause::RemoteAction {
                            room: r,
                            msg: m,
                            exit,
                            ..
                        } => {
                            let target = RoomKey::new(map as u32, *r as u32);
                            let exit_ok = w.rooms.get(&target).is_none_or(|rm| {
                                (0..=10).contains(exit)
                                    && rm
                                        .exits
                                        .get(*exit as usize)
                                        .is_some_and(|e| e.target.0 != 0)
                            });
                            vec![
                                room(*r, i32::from(map)),
                                msg(*m),
                                (!exit_ok).then(|| format!("exit {exit} of {map}:{r}")),
                            ]
                        }
                        Clause::Random { block: n } => {
                            todo.push((Kind::Pattern, *n, map));
                            vec![]
                        }
                        Clause::CheckSpell {
                            spell: sp,
                            block: n,
                        } => {
                            todo.push((Kind::Whole, *n, map));
                            vec![spell(*sp)]
                        }
                        Clause::TestSkill { block: n, .. } => {
                            todo.push((Kind::Whole, i32::from(*n), map));
                            vec![]
                        }
                        _ => vec![],
                    };
                    for what in found.into_iter().flatten() {
                        miss(text, what);
                    }
                }
            }
        }
    }
    s
}

// Every stock room command text and every block it reaches parses, and the numbers they name
// exist. The clauses with no keyword are the stock data's own faults, listed here; they are
// skipped and the line goes on, as on the original:
// - blocks 237, 1607 and 4085 are talk blocks (`topic:answer` lines) used as the command
//   texts of rooms 1:157, 1:193 and 15:1343, so their "clauses" are block numbers;
// - block 4251, the failed theft of blocks 4247 to 4250's `testskill`, is a prose line, so
//   the failure shows nothing;
// (block 9578's `go nightblack portal` line, which lacked `teleport`, is fixed as the
// content loads; see `stock_faults`);
// - `delay 5`, `add delay 5`, `check class`, `levelcheck` and `goodability -51 3154`
//   name no clause.
#[tokio::test(flavor = "multi_thread")]
async fn stock_room_commands_parse_and_name_what_exists() {
    let db = stock_db();
    let r = start(&db.url, 148).await;
    let s = on_world(&r, |w| sweep(w)).await;
    r.stop().await.unwrap();
    eprintln!(
        "{} rooms, {} blocks, {} lines, {} clauses; keywords: {:?}",
        s.rooms, s.blocks, s.lines, s.clauses, s.keywords
    );
    assert_eq!(s.missing, BTreeSet::new(), "numbers that name nothing");
    // block 4251's prose line, by its block and its length
    let mut unknown = s.unknown.clone();
    unknown
        .retain(|text, blocks| !(blocks.len() == 1 && blocks.contains(&4251) && text.len() > 60));
    assert_eq!(
        unknown.len() + 1,
        s.unknown.len(),
        "block 4251's prose line"
    );
    let want: BTreeMap<String, BTreeSet<i32>> = [
        ("0", &[237][..]),
        ("666", &[1607, 4085]),
        ("add delay 5", &[9136]),
        ("check class", &[2903, 9228]),
        (
            "delay 5",
            &[4230, 4231, 4232, 4233, 4234, 4236, 4237, 4238, 4239, 4240],
        ),
        ("goodability -51 3154", &[4355]),
        ("levelcheck", &[2903]),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.iter().copied().collect()))
    .collect();
    assert_eq!(unknown, want, "clauses with no keyword");
    assert_eq!(s.rooms, 810);
}

/// What one room command did when run.
#[derive(Debug, Default)]
struct Ran {
    /// Commands typed, and how many the room took as its own.
    commands: usize,
    handled: usize,
    /// `map:room input` of each command the room did not take, and of each that panicked.
    untaken: Vec<String>,
    panics: Vec<String>,
}

// Each room's commands run once against a fresh stock world: in each room with a command
// text, every line's first wording (its `*` dropped) typed by a level-20 character with
// plenty of hit points and coins, put back in the room each time. No command panics, and
// every one but the three of the talk-block rooms is taken by its room.
#[tokio::test(flavor = "multi_thread")]
async fn every_stock_room_command_runs() {
    let db = stock_db();
    let r = start(&db.url, 149).await;
    let _c = hero(&r, &db.url, "Sweep", 20, (1, 1)).await;
    let keys = on_world(&r, |w| {
        let mut k: Vec<RoomKey> = w
            .rooms
            .values()
            .filter(|r| r.cmdtext.0 != 0)
            .map(|r| r.key)
            .collect();
        k.sort_by_key(|k| (k.map.0, k.room.0));
        k
    })
    .await;
    let mut ran = Ran::default();
    for key in keys {
        let (commands, handled, untaken, panics) = on_world(&r, move |w| {
            let id = session_of(w, "Sweep");
            let inputs: Vec<String> = {
                let rm = &w.rooms[&key];
                (0..silvermere_engine::scripts::MAX_RECORDS)
                    .map_while(|p| w.text_blocks.get(&(p, rm.cmdtext)))
                    .flat_map(|t| pattern_lines(&t.text))
                    .filter_map(|pl| {
                        let first = pl.pattern.split('|').next()?.replace('*', "");
                        let first = first.trim();
                        (!first.is_empty()).then(|| first.to_owned())
                    })
                    .collect()
            };
            let (mut handled, mut untaken, mut panics) = (0, Vec::new(), Vec::new());
            for input in &inputs {
                silvermere_engine::movement::move_user::set_user_location(w, id, key, true);
                if let Some(p) = w.session_mut(id).and_then(|s| s.character.as_mut()) {
                    p.hp = p.maxhp;
                    p.coins.runic = 1000;
                    p.coins.platinum = 1000;
                }
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    silvermere_engine::scripts::room_command(w, id, input)
                }));
                match run {
                    Ok(true) => handled += 1,
                    Ok(false) => untaken.push(format!("{}:{} {input}", key.map.0, key.room.0)),
                    Err(_) => panics.push(format!("{}:{} {input}", key.map.0, key.room.0)),
                }
                if let Some(s) = w.session_mut(id) {
                    s.output.clear();
                }
            }
            (inputs.len(), handled, untaken, panics)
        })
        .await;
        ran.commands += commands;
        ran.handled += handled;
        ran.untaken.extend(untaken);
        ran.panics.extend(panics);
    }
    r.stop().await.unwrap();
    eprintln!(
        "{} commands, {} taken by the room; not taken: {:?}; panics: {:?}",
        ran.commands, ran.handled, ran.untaken, ran.panics
    );
    assert!(ran.panics.is_empty(), "{:#?}", ran.panics);
    // the three talk blocks used as command texts (see the sweep above) take nothing
    assert_eq!(
        ran.untaken,
        ["1:157 nope.topic 239", "1:193 woohoo", "15:1343 heh"],
        "{ran:?}"
    );
}

const NEGATIVE_PLANE: (u32, u32) = (8, 984);
const PORTAL_ITEM: i32 = 1419;

/// The portal phrases of the Negative Power Plane all move a level-40 character, with the
/// portal item lying in the room (it does at start), to 1:2683 with messages 1375 and 1376, the long one included (its stock line lacked
/// `teleport`).
#[tokio::test(flavor = "multi_thread")]
async fn nightblack_portal_phrases_all_teleport() {
    let db = stock_db();
    let r = start(&db.url, 146).await;
    let (m1, m2) = (message(&r, 1375).await, message(&r, 1376).await);
    assert!(
        lies(&r, NEGATIVE_PLANE, PORTAL_ITEM).await,
        "the portal at start"
    );
    for (name, phrase) in [
        ("Aaa", "go portal"),
        ("Bbb", "enter portal"),
        ("Ccc", "go nightblack portal"),
        ("Ddd", "enter nightblack portal"),
    ] {
        let mut c = hero(&r, &db.url, name, 40, NEGATIVE_PLANE).await;
        let out = cmd(&mut c, phrase).await;
        assert_eq!(room_of(&r, name).await, 2683, "{phrase}:\n{out}");
        assert!(out.contains(&m1) && out.contains(&m2), "{phrase}:\n{out}");
    }
    r.stop().await.unwrap();
}

/// Below level 40 the long phrase shows message 1374, and with the portal gone from the room
/// message 1373; it moves nobody then either.
#[tokio::test(flavor = "multi_thread")]
async fn nightblack_portal_checks_are_unchanged() {
    let db = stock_db();
    let r = start(&db.url, 147).await;
    let (m_item, m_level) = (message(&r, 1373).await, message(&r, 1374).await);
    let mut c = hero(&r, &db.url, "Low", 39, NEGATIVE_PLANE).await;
    let out = cmd(&mut c, "go nightblack portal").await;
    assert!(out.contains(&m_level), "{out}");
    assert_eq!(room_of(&r, "Low").await, NEGATIVE_PLANE.1);
    on_world(&r, |w| {
        let key = RoomKey::new(NEGATIVE_PLANE.0, NEGATIVE_PLANE.1);
        silvermere_engine::scripts::actions::erase_item_from_room(w, key, ItemNo(PORTAL_ITEM));
    })
    .await;
    let mut c = hero(&r, &db.url, "High", 40, NEGATIVE_PLANE).await;
    let out = cmd(&mut c, "go nightblack portal").await;
    assert!(out.contains(&m_item), "{out}");
    assert_eq!(room_of(&r, "High").await, NEGATIVE_PLANE.1);
    r.stop().await.unwrap();
}
