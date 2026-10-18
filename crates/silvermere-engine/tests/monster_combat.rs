//! The monsters' round (T-036): `do_monster_combat` and `monster_can_see_user` inside the
//! energy round (`background_energy`), with the blows of T-033's `attack_monster_user`.
//!
//! The monsters are the stock giant rat (1), acid slime (5) and kobold thief (7) with their
//! WCCMSG texts and the kobold's shortsword (item 67), copied from the vendored export
//! `crates/silvermere-import/tests/fixtures/world/{WCCKNMSR,WCCMSG,WCCITEMS}.jsonl.gz`
//! (MANIFEST of 2026-10-14). The fighters are the test Kang Witchunter of `autocombat.rs`.
//! The monster-text patterns are built the way the reference test harness's
//! `template_regex` builds them from those WCCMSG records (its ROLE_ARGS and the
//! hit/def/miss mapping of the message records).

mod common;

use std::sync::mpsc::Receiver;

use common::{TestClock, enter, test_config, tiny_snapshot};
use silvermere_engine::combat::autocombat::{
    background_energy, energy_update_monsters, engage, is_inside_autocombat,
};
use silvermere_engine::combat::{AutoTarget, Mode, attack_monster_user, attack_user_monster};
use silvermere_engine::monsters::combat_ai::do_monster_combat;
use silvermere_engine::monsters::generate::{Request, generate_monster};
use silvermere_engine::monsters::visibility::monster_can_see_user;
use silvermere_engine::movement::move_user;
use silvermere_engine::tell::palette_of;
use silvermere_engine::world::blank;
use silvermere_engine::{Engine, EngineInput, EngineOutput, SeededRng, SessionId, World};
use silvermere_rules::combat::Move;
use silvermere_text::strip_ansi;
use silvermere_types::enums::{
    Ability, Alignment, ExitType, GroupRole, ItemType, MessageLevel, MonsterAttackType,
    MonsterType, WeaponType,
};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{
    ActiveMonsterId, ClassNo, ItemNo, MessageNo, MonsterNo, RaceNo, RoomId, RoomKey, SpellNo,
};
use silvermere_types::records::{Exit, KnownMonster, Message, MonsterAttack};

const ARENA: u32 = 2150;
/// Three rooms in a row, west to east (the hunter's walk).
const WEST: u32 = 3001;
const MIDDLE: u32 = 3002;
const EAST: u32 = 3003;
const KANG: RaceNo = RaceNo(11);
const WITCHUNTER: ClassNo = ClassNo(2);
const QUARTERSTAFF: ItemNo = ItemNo(100);
const SHORTSWORD: ItemNo = ItemNo(67);
/// A test armour (not stock): AC 4 and DR 30, so the kobold thief's 15 accuracy misses at times.
const VEST: ItemNo = ItemNo(102);
const RAT: i32 = 1;
const SLIME: i32 = 5;
const KOBOLD: i32 = 7;

/// The 0.001 points of chi-square for 1 and 2 degrees of freedom.
const CHI1_0001: f64 = 10.828;
const CHI2_0001: f64 = 13.816;

fn msg(n: i32, a: &str, b: &str, c: &str) -> Message {
    Message {
        number: MessageNo(n),
        msgs: [a.into(), b.into(), c.into()],
    }
}

/// The stock WCCMSG records of the three monsters and the two weapons.
fn messages() -> Vec<Message> {
    vec![
        msg(
            27,
            "The %s bites you for %d damage!",
            "The %s bites %s for %s damage!",
            "The giant rat falls to the ground with a tortured squeak.",
        ),
        msg(
            35,
            "The %s whips you with its pseudopod for %d damage!",
            "The %s whips %s with its pseudopod for %s damage!",
            "The acid slime dissolves into a puddle of bluish goo.",
        ),
        msg(
            41,
            "The %s stabs you for %d damage!",
            "The %s stabs %s for %s damage!",
            "The kobold thief falls to the ground with a shrill cry.",
        ),
        msg(
            8294,
            "The %s %slunges at %s, %sbut %s dodges out of the way!",
            "The %s %slunges at %syou!",
            "The %s %slunges at %s! %s",
        ),
        msg(
            8296,
            "The %s %slashes at %s, %sbut %s dodges out of the way!",
            "The %s %sflails at you!",
            "The %s %sflails at %s!",
        ),
        msg(
            8297,
            "The %s %s you, but your armour deflects the blow!",
            "The %s %s %s, but %s armour deflects the blow!",
            "The %s %s you with their %s, but you dodge!",
        ),
        msg(
            8307,
            "The %s %slunges at you, but your armour deflects the blow!",
            "The %s %slunges at %s, but %s armour deflects the blow!",
            "The %s %slunges at %syou, but you dodge out of the way!",
        ),
        msg(
            8309,
            "The %s %slashes at you, but your armour deflects the blow!",
            "The %s %slashes at %s, but %s armour deflects the blow!",
            "The %s %slashes at %syou, but you dodge out of the way!",
        ),
        msg(
            8310,
            "The %s %s %s with their %s, but %s dodges!",
            "The %s %s you with their %s!",
            "The %s %s %s with their %s!",
        ),
        msg(
            8228,
            "impale|skewer|slash",
            "impales|skewers|slashes",
            "impales|skewers|slashes",
        ),
        msg(8287, "lunge at", "lunges at", "lunges at"),
        msg(
            8237,
            "smack|whap|beat",
            "smacks|whaps|beats",
            "smacks|whaps|beats",
        ),
        msg(8289, "swipe at", "swipes at", "swipes at"),
    ]
}

/// One attack slot of a WCCKNMSR record.
#[allow(clippy::too_many_arguments)]
fn slot(
    kind: u16,
    accuracy: i16,
    pct: u8,
    min: i16,
    max: i16,
    msgs: (i32, i32, i32),
    energy: i16,
    hitspell: i16,
) -> MonsterAttack {
    MonsterAttack {
        kind: MonsterAttackType::from_code(kind),
        accuracy,
        pct,
        min,
        max,
        hitmsg: MessageNo(msgs.0),
        defmsg: MessageNo(msgs.1),
        missmsg: MessageNo(msgs.2),
        energy,
        hitspell: SpellNo(hitspell),
    }
}

/// A WCCKNMSR record with the fields the round reads. `grouprole` is Solo for all three
/// (the rat and the kobold are stock group followers, 2, which only matters to
/// `move_monster`'s group discipline, not tested here).
#[allow(clippy::too_many_arguments)]
fn known(
    n: i32,
    name: &str,
    weapon: ItemNo,
    dr: i16,
    ac: i16,
    exp: i32,
    maxhp: i16,
    abilities: &[Ability],
    followpct: i16,
    attacks: &[MonsterAttack],
) -> KnownMonster {
    let mut k = blank::clear_known_monster(MonsterNo(n));
    k.name = name.into();
    k.montype = MonsterType::Group1;
    k.level = 1;
    k.genindex = 1;
    k.weapon = weapon;
    k.dr = dr;
    k.ac = ac;
    k.exp = exp;
    k.maxhp = maxhp;
    k.energy = 1000;
    k.alignment = Alignment::ChaoticEvil;
    k.followpct = followpct;
    k.grouprole = GroupRole::Solo;
    for (i, a) in abilities.iter().enumerate() {
        k.abilities[i] = (*a, 0);
    }
    for (i, a) in attacks.iter().enumerate() {
        k.attacks[i] = *a;
    }
    k
}

fn exit(target: u32) -> Exit {
    Exit {
        target: RoomId(target),
        kind: ExitType::Normal,
        par1: 0,
        par2: 0,
        par3: 0,
        par4: 0,
    }
}

fn world() -> World {
    let mut s = tiny_snapshot();
    for n in [ARENA, WEST, MIDDLE, EAST] {
        let mut r = blank::clear_room(RoomKey::new(1, n));
        r.name = format!("Room {n}");
        r.mongroup = MonsterType::Group1.code() as i16;
        // east is exit 2, west exit 3
        if n == WEST {
            r.exits[2] = exit(MIDDLE);
        }
        if n == MIDDLE {
            r.exits[3] = exit(WEST);
            r.exits[2] = exit(EAST);
        }
        if n == EAST {
            r.exits[3] = exit(MIDDLE);
        }
        s.rooms.push(r);
    }
    s.messages = messages();
    let mut staff = blank::clear_item(QUARTERSTAFF);
    staff.name = "quarterstaff".into();
    staff.item_type = ItemType::Weapon;
    staff.weight = 100;
    staff.mindmg = 3;
    staff.maxdmg = 4;
    staff.speed = 1259;
    staff.strreq = 30;
    staff.hitmsg = MessageNo(8237);
    staff.missmsg = MessageNo(8289);
    s.items.push(staff);
    let mut sword = blank::clear_item(SHORTSWORD);
    sword.name = "shortsword".into();
    sword.item_type = ItemType::Weapon;
    sword.wpntype = WeaponType::from_code(2);
    sword.mindmg = 2;
    sword.maxdmg = 9;
    sword.speed = 1000;
    sword.hitmsg = MessageNo(8228);
    sword.missmsg = MessageNo(8287);
    sword.abilities[0] = (Ability::BackstabAv, 5);
    s.items.push(sword);
    let mut vest = blank::clear_item(VEST);
    vest.name = "padded vest".into();
    vest.item_type = ItemType::Armour;
    vest.ac = 4;
    vest.dr = 30;
    s.items.push(vest);
    s.monsters = vec![
        // giant rat: atktypes [1,0,0,229,0], atkpcts [100,20,30,0,0]: slot 0 always wins
        known(
            RAT,
            "giant rat",
            ItemNo(0),
            1,
            0,
            9,
            12,
            &[Ability::PoisonImmune, Ability::Animal],
            20,
            &[
                slot(1, 10, 100, 2, 10, (27, 8307, 8294), 1000, 0),
                slot(0, 70, 20, 4, 20, (27, 8307, 8294), 1000, 0),
                slot(0, 1, 30, 50, 0, (1, 1, 1), 1000, 0),
                slot(229, 0, 0, 0, 0, (0, 0, 0), 0, 0),
            ],
        ),
        known(
            SLIME,
            "acid slime",
            ItemNo(0),
            0,
            0,
            16,
            25,
            &[Ability::NonLiving],
            15,
            &[
                slot(1, 20, 100, 2, 9, (35, 8309, 8296), 1000, 126),
                slot(229, 0, 0, 0, 0, (0, 0, 0), 0, 0),
            ],
        ),
        known(
            KOBOLD,
            "kobold thief",
            SHORTSWORD,
            1,
            10,
            13,
            20,
            &[],
            20,
            &[
                slot(1, 15, 100, 1, 8, (41, 8297, 8310), 666, 0),
                slot(229, 0, 0, 0, 0, (0, 0, 0), 0, 0),
            ],
        ),
    ];
    let mut witchunter = blank::clear_class(WITCHUNTER);
    witchunter.name = "Witchunter".into();
    witchunter.exp = 30;
    witchunter.combat = 7;
    s.classes.push(witchunter);
    let mut kang = blank::clear_race(KANG);
    kang.name = "Kang".into();
    kang.exp = 35;
    s.races.push(kang);
    let mut w = World::load(test_config(), 16, s).expect("world loads");
    w.rng = SeededRng::new(36);
    w
}

/// A level-1 Kang Witchunter in `room` who does not go down.
fn hero(w: &mut World, name: &str, room: u32) -> SessionId {
    let id = enter(w, name, room, MessageLevel::High);
    let c = w.session_mut(id).unwrap().character.as_mut().unwrap();
    c.settings = PlayerSettings::VALIDATED | PlayerSettings::CANCAST;
    c.race = KANG;
    c.class = WITCHUNTER;
    c.level = 1;
    (c.str, c.intel, c.wil, c.hea, c.agi, c.cha) = (55, 30, 45, 50, 30, 30);
    (c.hp, c.maxhp) = (30000, 30000);
    (c.energy, c.maxenergy) = (1000, 1000);
    c.lives = 9;
    c.items[0] = QUARTERSTAFF;
    c.itemuses[0] = -1;
    c.weapon = QUARTERSTAFF;
    id
}

/// A stock monster forced into `room` (`generate_monster(..., force 1)`).
fn place(w: &mut World, n: i32, room: u32) -> ActiveMonsterId {
    let req = Request {
        areatype: -1,
        monnum: MonsterNo(n),
        minlevel: 0,
        maxlevel: 0,
        owner: None,
        ally: false,
        force: true,
    };
    let m = generate_monster(w, RoomKey::new(1, room), req).expect("placed");
    // the generator's own name pick and arrival texts are not this card's
    w.active_monsters.by_number_mut(m).unwrap().grouprole = GroupRole::Solo;
    m
}

fn mon(w: &mut World, n: ActiveMonsterId) -> &mut silvermere_types::records::ActiveMonster {
    w.active_monsters.by_number_mut(n).unwrap()
}

fn plr(w: &mut World, id: SessionId) -> &mut silvermere_types::records::Character {
    w.session_mut(id).unwrap().character.as_mut().unwrap()
}

/// Whether `id` was attacked since the flags were last cleared, clearing them:
/// `ST2_ATTACKED`, or `ST_SPOTTED`, which the end of `background_energy` turns it into.
fn attacked(w: &mut World, id: SessionId) -> bool {
    let p = plr(w, id);
    let was =
        p.status2.contains(PlayerStatus2::ATTACKED) || p.status.contains(PlayerStatus::SPOTTED);
    p.status2.remove(PlayerStatus2::ATTACKED);
    p.status.remove(PlayerStatus::SPOTTED);
    was
}

/// A fresh round for `m`: a full round of energy, following nobody, and every player's
/// output, flags and hit points as they were.
fn reset(w: &mut World, m: ActiveMonsterId, players: &[SessionId]) {
    let e = mon(w, m);
    e.energy = e.maxenergy as u16;
    e.following.clear();
    for &id in players {
        let p = plr(w, id);
        p.hp = p.maxhp;
        p.status2.remove(PlayerStatus2::ATTACKED);
        w.session_mut(id).unwrap().output.clear();
    }
}

/// The session's output since the last call, ANSI stripped, as lines.
fn text(w: &mut World, id: SessionId) -> Vec<String> {
    let palette = palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let out = s.output.render(palette, b"<P>");
    s.output.clear();
    String::from_utf8_lossy(&strip_ansi(&out))
        .split("\r\n")
        .map(|l| {
            l.replace("<P>", "")
                .trim_matches([' ', '\u{7}'])
                .to_string()
        })
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}

fn chi_square(hits: u64, n: u64, p: f64) -> f64 {
    let e1 = n as f64 * p;
    let e0 = n as f64 * (1.0 - p);
    (hits as f64 - e1).powi(2) / e1 + ((n - hits) as f64 - e0).powi(2) / e0
}

// --- the round through the engine ----------------------------------------------------

// A Chaotic Evil giant rat attacks the only player it sees: the
// `lastseen` fallback makes the first energy round certain.
#[test]
fn a_giant_rat_attacks_a_lone_player_in_the_first_round() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    place(&mut w, RAT, ARENA);
    text(&mut w, id);
    let (tx, rx): (_, Receiver<EngineOutput>) = std::sync::mpsc::channel();
    let clock = TestClock::new();
    let mut engine = Engine::new(w, clock.clone(), SeededRng::new(36), tx);
    let mut seen = String::new();
    let mut rounds = 0;
    for _ in 0..5 {
        clock.advance_ticks(1, &mut engine);
        while let Ok(o) = rx.try_recv() {
            if let EngineOutput::Write { id: to, bytes } = o
                && to == id
            {
                seen.push_str(&String::from_utf8_lossy(&strip_ansi(&bytes)));
            }
        }
        rounds += 1;
        if seen.contains("giant rat") {
            break;
        }
    }
    assert!(seen.contains("The giant rat"), "{seen:?}");
    assert!(rounds <= 5, "the first energy round (5 s)");
    assert!(
        engine
            .world
            .session(id)
            .unwrap()
            .player()
            .unwrap()
            .attackers
            >= 1
    );
    let _ = EngineInput::Tick;
}

// Lawful Good, Good and Neutral monsters never start a fight; and
// no monster in the round attacks another unless it hunts it.
#[test]
fn good_monsters_never_start_and_monsters_leave_monsters_alone() {
    for a in [Alignment::LawfulGood, Alignment::Good, Alignment::Neutral] {
        let mut w = world();
        let id = hero(&mut w, "Kasimir", ARENA);
        let rat = place(&mut w, RAT, ARENA);
        mon(&mut w, rat).alignment = a;
        for _ in 0..200 {
            background_energy(&mut w);
            assert!(!attacked(&mut w, id), "{a:?}");
        }
        assert_eq!(plr(&mut w, id).attackers, 0);
    }
    // a hostile rat and a good slime: the slime is never touched
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    place(&mut w, RAT, ARENA);
    let slime = place(&mut w, SLIME, ARENA);
    mon(&mut w, slime).alignment = Alignment::Good;
    let hp = mon(&mut w, slime).hp;
    for _ in 0..200 {
        background_energy(&mut w);
        plr(&mut w, id).hp = 30000;
    }
    assert_eq!(mon(&mut w, slime).hp, hp);
}

// Three players with 0, 1 and 2 attackers in `mappings` order: the
// rat's first pick is the first at 50%, the second at 50%*45%, the third at 50%*55%
// (its own 40% plus the fallback). (sic) The rat is visited once per player in the room,
// so every round raises the attackers counts three times, though only the first visit
// strikes (the others find it below a round of energy).
#[test]
fn three_players_spread_fifty_minus_five_per_attacker() {
    let mut w = world();
    let ids = [
        hero(&mut w, "Aldric", ARENA),
        hero(&mut w, "Bob", ARENA),
        hero(&mut w, "Cid", ARENA),
    ];
    w.cycles.order = ids.to_vec();
    let rat = place(&mut w, RAT, ARENA);
    let trials = 10_000u64;
    let mut first = [0u64; 3];
    for _ in 0..trials {
        reset(&mut w, rat, &ids);
        for (i, &id) in ids.iter().enumerate() {
            plr(&mut w, id).attackers = i as i32;
        }
        do_monster_combat(&mut w);
        let hit: Vec<usize> = (0..3).filter(|&i| attacked(&mut w, ids[i])).collect();
        assert_eq!(hit.len(), 1, "one real attack a round");
        first[hit[0]] += 1;
        let raised: i32 = ids.iter().map(|&id| plr(&mut w, id).attackers).sum::<i32>() - 3;
        assert_eq!(raised, 3, "three visits, three counts");
    }
    let p = [0.5, 0.5 * 0.45, 0.5 * 0.55];
    let chi: f64 = (0..3)
        .map(|i| {
            let e = p[i] * trials as f64;
            (first[i] as f64 - e).powi(2) / e
        })
        .sum();
    assert!(chi < CHI2_0001, "chi {chi} {first:?}");
}

// A Lawful Evil monster leaves a player with 40 evil points alone
// until he fights it; 39 points is no protection.
#[test]
fn lawful_evil_ignores_an_outlaw_until_attacked() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let rat = place(&mut w, RAT, ARENA);
    mon(&mut w, rat).alignment = Alignment::LawfulEvil;
    plr(&mut w, id).evil = 40;
    for _ in 0..200 {
        reset(&mut w, rat, &[id]);
        do_monster_combat(&mut w);
        assert!(!attacked(&mut w, id));
    }
    // he attacks it: queued against the rat
    engage(
        &mut w,
        id,
        Some(AutoTarget::Monster(rat)),
        Move::Attack,
        None,
    );
    assert!(is_inside_autocombat(&w, id));
    reset(&mut w, rat, &[id]);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, id));
    // a 39-point player is fair game at once
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let rat = place(&mut w, RAT, ARENA);
    mon(&mut w, rat).alignment = Alignment::LawfulEvil;
    plr(&mut w, id).evil = 39;
    reset(&mut w, rat, &[id]);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, id));
}

// A guard that is not Lawful Evil attacks a 40-point outlaw with
// (100 - 5*attackers)% and never a player under 40; a Lawful Evil guard the reverse.
#[test]
fn guards_go_after_outlaws_at_one_hundred_minus_five_n() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let guard = place(&mut w, RAT, ARENA);
    mon(&mut w, guard).montype = MonsterType::Guard;
    mon(&mut w, guard).alignment = Alignment::Good;
    plr(&mut w, id).evil = 40;
    let trials = 10_000u64;
    let mut hits = 0;
    for _ in 0..trials {
        reset(&mut w, guard, &[id]);
        plr(&mut w, id).attackers = 4;
        do_monster_combat(&mut w);
        hits += u64::from(attacked(&mut w, id));
    }
    let chi = chi_square(hits, trials, 0.80);
    assert!(chi < CHI1_0001, "chi {chi} hits {hits}");
    plr(&mut w, id).evil = 39;
    for _ in 0..1000 {
        reset(&mut w, guard, &[id]);
        plr(&mut w, id).attackers = 0;
        do_monster_combat(&mut w);
        assert!(!attacked(&mut w, id));
    }
    // already fighting it: attacked whatever the points
    engage(
        &mut w,
        id,
        Some(AutoTarget::Monster(guard)),
        Move::Attack,
        None,
    );
    reset(&mut w, guard, &[id]);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, id));
    // a Lawful Evil guard: the 40-point outlaw is safe, the 0-point player is not
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let guard = place(&mut w, RAT, ARENA);
    mon(&mut w, guard).montype = MonsterType::Guard;
    mon(&mut w, guard).alignment = Alignment::LawfulEvil;
    plr(&mut w, id).evil = 40;
    for _ in 0..200 {
        reset(&mut w, guard, &[id]);
        do_monster_combat(&mut w);
        assert!(!attacked(&mut w, id));
    }
    plr(&mut w, id).evil = 0;
    reset(&mut w, guard, &[id]);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, id));
}

// A hidden player is seen only by a monster with See Hidden.
#[test]
fn a_hidden_player_is_attacked_only_by_see_hidden() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let rat = place(&mut w, RAT, ARENA);
    plr(&mut w, id).hidden = 1;
    for _ in 0..200 {
        reset(&mut w, rat, &[id]);
        do_monster_combat(&mut w);
        assert!(!attacked(&mut w, id));
    }
    w.monsters.get_mut(&MonsterNo(RAT)).unwrap().abilities[2] = (Ability::SeeHidden, 1);
    reset(&mut w, rat, &[id]);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, id));
}

// A sneaking player is seen only with See Hidden, and not on his
// arrival round (ST_JUSTARRIVED). (sic) The energy round clears ST_JUSTARRIVED for
// everyone (energy_update_character) before do_monster_combat, its only caller,
// so through the round the arrival protects nobody: the See Hidden rat strikes at once.
#[test]
fn a_sneaking_arrival_and_the_arrival_flag() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let rat = place(&mut w, RAT, ARENA);
    let key = RoomKey::new(1, ARENA);
    plr(&mut w, id).status |= PlayerStatus::SNEAKING | PlayerStatus::JUSTARRIVED;
    let seen = |w: &World| {
        let m = w.active_monsters.by_number(rat).unwrap();
        monster_can_see_user(w, w.session(id).unwrap().player(), key, m)
    };
    assert!(!seen(&w), "no See Hidden");
    w.monsters.get_mut(&MonsterNo(RAT)).unwrap().abilities[2] = (Ability::SeeHidden, 1);
    assert!(!seen(&w), "See Hidden, arrival round");
    plr(&mut w, id).status.remove(PlayerStatus::JUSTARRIVED);
    assert!(seen(&w), "See Hidden, a round later");
    // another room is never seen
    assert!(!monster_can_see_user(
        &w,
        w.session(id).unwrap().player(),
        RoomKey::new(1, WEST),
        w.active_monsters.by_number(rat).unwrap()
    ));
    // through the energy round
    plr(&mut w, id).status |= PlayerStatus::JUSTARRIVED;
    reset(&mut w, rat, &[id]);
    background_energy(&mut w);
    assert!(attacked(&mut w, id));
    // and without See Hidden a sneaking player is never attacked
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let rat = place(&mut w, RAT, ARENA);
    plr(&mut w, id).status |= PlayerStatus::SNEAKING;
    for _ in 0..100 {
        reset(&mut w, rat, &[id]);
        background_energy(&mut w);
        assert!(!attacked(&mut w, id));
    }
}

// A charmed follower fights its master's target.
#[test]
fn a_charmed_follower_fights_its_masters_target() {
    let mut w = world();
    let master = hero(&mut w, "Aldric", ARENA);
    let foe = hero(&mut w, "Bob", ARENA);
    w.cycles.order = vec![master, foe];
    let pet = place(&mut w, RAT, ARENA);
    let setup = |w: &mut World| {
        let m = mon(w, pet);
        m.following = "Aldric".into();
        m.friendly = 1;
        m.flags.insert(ActiveMonsterFlags::CHARMED);
        m.energy = m.maxenergy as u16;
    };
    // not in auto-combat: nothing
    setup(&mut w);
    do_monster_combat(&mut w);
    assert!(!attacked(&mut w, foe) && !attacked(&mut w, master));
    // fighting Bob: the pet goes for Bob
    engage(
        &mut w,
        master,
        Some(AutoTarget::Player(foe)),
        Move::Attack,
        None,
    );
    setup(&mut w);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, foe));
    assert!(!attacked(&mut w, master));
    // fighting a slime: the pet bites the slime
    let slime = place(&mut w, SLIME, ARENA);
    mon(&mut w, slime).alignment = Alignment::Good;
    engage(
        &mut w,
        master,
        Some(AutoTarget::Monster(slime)),
        Move::Attack,
        None,
    );
    let full = mon(&mut w, slime).hp;
    let mut bitten = false;
    for _ in 0..20 {
        setup(&mut w);
        do_monster_combat(&mut w);
        bitten |= w
            .active_monsters
            .by_number(slime)
            .is_none_or(|m| m.hp < full);
        if bitten {
            break;
        }
    }
    assert!(bitten, "the slime was hit at least once in 20 rounds");
}

// An unfriendly follower attacks the player it follows (whatever
// its alignment) and nobody else.
#[test]
fn an_unfriendly_follower_attacks_its_master() {
    let mut w = world();
    let master = hero(&mut w, "Aldric", ARENA);
    let other = hero(&mut w, "Bob", ARENA);
    w.cycles.order = vec![other, master];
    let rat = place(&mut w, RAT, ARENA);
    mon(&mut w, rat).alignment = Alignment::Good;
    for _ in 0..50 {
        reset(&mut w, rat, &[master, other]);
        mon(&mut w, rat).following = "Aldric".into();
        do_monster_combat(&mut w);
        assert!(attacked(&mut w, master));
        assert!(!attacked(&mut w, other));
    }
}

// A friendly guard attacks whoever fights its master;
// An evil friend attacks the first player it sees other than its master.
#[test]
fn friendly_followers_defend_and_bully() {
    let mut w = world();
    let master = hero(&mut w, "Aldric", ARENA);
    let foe = hero(&mut w, "Bob", ARENA);
    let bystander = hero(&mut w, "Cid", ARENA);
    w.cycles.order = vec![bystander, master, foe];
    let guard = place(&mut w, RAT, ARENA);
    let ids = [master, foe, bystander];
    let setup = |w: &mut World| {
        reset(w, guard, &ids);
        let m = mon(w, guard);
        m.montype = MonsterType::Guard;
        m.following = "Aldric".into();
        m.friendly = 1;
    };
    setup(&mut w);
    do_monster_combat(&mut w);
    assert!(
        ids.iter().all(|&i| !attacked(&mut w, i)),
        "nobody fights him"
    );
    engage(
        &mut w,
        foe,
        Some(AutoTarget::Player(master)),
        Move::Attack,
        None,
    );
    setup(&mut w);
    do_monster_combat(&mut w);
    assert!(attacked(&mut w, foe));
    assert!(!attacked(&mut w, master) && !attacked(&mut w, bystander));
    // an evil friend that is no guard: the first stranger in mappings order
    let mut w = world();
    let master = hero(&mut w, "Aldric", ARENA);
    let first = hero(&mut w, "Bob", ARENA);
    let second = hero(&mut w, "Cid", ARENA);
    w.cycles.order = vec![master, first, second];
    let pet = place(&mut w, RAT, ARENA);
    for _ in 0..20 {
        reset(&mut w, pet, &[master, first, second]);
        let m = mon(&mut w, pet);
        m.following = "Aldric".into();
        m.friendly = 1;
        do_monster_combat(&mut w);
        assert!(attacked(&mut w, first));
        assert!(!attacked(&mut w, master) && !attacked(&mut w, second));
    }
    // a good friend does nothing
    reset(&mut w, pet, &[master, first, second]);
    let m = mon(&mut w, pet);
    m.following = "Aldric".into();
    m.friendly = 1;
    m.alignment = Alignment::Neutral;
    do_monster_combat(&mut w);
    assert!(
        [master, first, second]
            .iter()
            .all(|&i| !attacked(&mut w, i))
    );
}

// A hunter takes one step a turn along the hunted monster's trail
// (dir_monster_travelling_coord) and attacks once no step is left.
// Players stand in every room, visited east to west, so each round moves it one room.
#[test]
fn a_hunter_walks_two_rooms_and_attacks() {
    let mut w = world();
    let pe = hero(&mut w, "Aldric", EAST);
    let pm = hero(&mut w, "Bob", MIDDLE);
    let pw = hero(&mut w, "Cid", WEST);
    w.cycles.order = vec![pe, pm, pw];
    let prey = place(&mut w, SLIME, EAST);
    {
        let m = mon(&mut w, prey);
        m.alignment = Alignment::Good;
        m.trail[0] = RoomId(EAST);
        m.trail[1] = RoomId(MIDDLE);
        m.trail[2] = RoomId(WEST);
    }
    let hunter = place(&mut w, RAT, WEST);
    mon(&mut w, hunter).hunting = prey;
    let full = mon(&mut w, prey).hp;
    let room_of = |w: &mut World| mon(w, hunter).room.0;
    energy_update_monsters(&mut w);
    do_monster_combat(&mut w);
    assert_eq!(room_of(&mut w), MIDDLE, "round 1");
    assert_eq!(mon(&mut w, prey).hp, full);
    energy_update_monsters(&mut w);
    do_monster_combat(&mut w);
    assert_eq!(room_of(&mut w), EAST, "round 2");
    let mut rounds = 2;
    while mon(&mut w, prey).hp == full && rounds < 30 {
        energy_update_monsters(&mut w);
        do_monster_combat(&mut w);
        rounds += 1;
        assert_eq!(room_of(&mut w), EAST);
    }
    assert!(
        mon(&mut w, prey).hp < full,
        "attacked within {rounds} rounds"
    );
    // nobody was attacked by the hunter
    for id in [pe, pm, pw] {
        assert_eq!(plr(&mut w, id).attackers, 0);
    }
}

// The same walk with the players visited west to east (sic, the once-per-player visit):
// the hunter steps WEST->MIDDLE on Cid's visit, MIDDLE->EAST on Bob's and attacks on
// Aldric's, all in one round. `attack_monster_monster` spends its energy, hit or miss;
// moving costs none.
#[test]
fn a_hunter_double_steps_when_the_order_runs_its_way() {
    let mut w = world();
    let pe = hero(&mut w, "Aldric", EAST);
    let pm = hero(&mut w, "Bob", MIDDLE);
    let pw = hero(&mut w, "Cid", WEST);
    w.cycles.order = vec![pw, pm, pe];
    let prey = place(&mut w, SLIME, EAST);
    {
        let m = mon(&mut w, prey);
        m.alignment = Alignment::Good;
        m.trail[0] = RoomId(EAST);
        m.trail[1] = RoomId(MIDDLE);
        m.trail[2] = RoomId(WEST);
    }
    let hunter = place(&mut w, RAT, WEST);
    mon(&mut w, hunter).hunting = prey;
    energy_update_monsters(&mut w);
    let full = mon(&mut w, hunter).maxenergy as u16;
    assert_eq!(mon(&mut w, hunter).energy, full);
    do_monster_combat(&mut w);
    assert_eq!(mon(&mut w, hunter).room.0, EAST, "two steps in one round");
    assert!(
        mon(&mut w, hunter).energy < full,
        "and the attack in the same round"
    );
}

// The players go first when a roll of 0..=100 is below 60.
#[test]
fn players_first_sixty_percent_over_a_thousand_rounds() {
    let mut w = world();
    hero(&mut w, "Kasimir", ARENA);
    let n = 1000u64;
    let first = (0..n).filter(|_| background_energy(&mut w)).count() as u64;
    let chi = chi_square(first, n, 0.60);
    assert!(chi < CHI1_0001, "chi {chi} first {first}");
}

// The monsters' round runs in the 60/40 slot: before the players'
// auto-combat or after it. The rat's line comes before Kasimir's blow in a round the
// monsters lead, after it in one the players lead.
#[test]
fn the_monster_round_runs_before_or_after_the_players() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let rat = place(&mut w, RAT, ARENA);
    mon(&mut w, rat).hp = 30000;
    mon(&mut w, rat).maxhp = 30000;
    let mut saw = [false; 2];
    for _ in 0..40 {
        engage(
            &mut w,
            id,
            Some(AutoTarget::Monster(rat)),
            Move::Attack,
            None,
        );
        reset(&mut w, rat, &[id]);
        let players_first = background_energy(&mut w);
        let ls = text(&mut w, id);
        let mine = ls
            .iter()
            .position(|l| l.starts_with("You ") && l.contains("giant rat"));
        let its = ls.iter().position(|l| l.starts_with("The giant rat"));
        if let (Some(a), Some(b)) = (mine, its) {
            assert_eq!(a < b, players_first, "{ls:?}");
            saw[usize::from(players_first)] = true;
        }
    }
    assert!(saw[0] && saw[1], "both orders seen");
}

// Blows a round. The slot is the
// first whose atkpcts is above the roll (the rat's 100 makes slot 0 certain), each blow
// costs the slot's atkenergies, a blow the energy left cannot pay ends the round, and
// the remainder carries into the next round (below one round, a round is added). So a
// monster makes floor((left + energy) / atkenergies) blows, maxenergy/atkenergies on
// average: the rat and the slime one a round, the kobold thief 1, 2, 1, 2...
#[test]
fn monster_blows_per_round() {
    for (n, cost, expect) in [
        (RAT, 1000u16, vec![1u16; 8]),
        (SLIME, 1000, vec![1; 8]),
        (KOBOLD, 666, vec![1, 2, 1, 2, 1, 2, 1, 2]),
    ] {
        let mut w = world();
        let id = hero(&mut w, "Kasimir", ARENA);
        let m = place(&mut w, n, ARENA);
        let mut got = Vec::new();
        for _ in 0..expect.len() {
            energy_update_monsters(&mut w);
            plr(&mut w, id).hp = 30000;
            let before = mon(&mut w, m).energy;
            do_monster_combat(&mut w);
            let after = mon(&mut w, m).energy;
            assert_eq!((before - after) % cost, 0);
            got.push((before - after) / cost);
        }
        assert_eq!(got, expect, "monster {n}");
    }
}

// The T-043 timing clause: a party follower's own move gives the monsters their free
// attack (give_monsters_a_free_attack) before the
// ` -- Following your Party leader --` line.
#[test]
fn the_free_attack_comes_before_the_follow_line() {
    let mut w = world();
    let leader = hero(&mut w, "Aldric", WEST);
    let follower = hero(&mut w, "Bob", WEST);
    w.session_mut(follower).unwrap().party.leader = Some(leader);
    w.session_mut(leader).unwrap().party.group[0] = Some(follower);
    assert!(move_user(&mut w, leader, 2, 0));
    let rat = place(&mut w, RAT, WEST);
    // followpct 100 always takes the free-attack roll
    mon(&mut w, rat).followpct = 100;
    text(&mut w, follower);
    assert!(move_user(&mut w, follower, 2, 3));
    let ls = text(&mut w, follower);
    let rat_line = ls.iter().position(|l| l.contains("giant rat"));
    let follow = ls
        .iter()
        .position(|l| l.contains("Following your Party leader"));
    assert!(
        matches!((rat_line, follow), (Some(a), Some(b)) if a < b),
        "{ls:?}"
    );
}

// --- the texts --------------------------------------------------------------------------

/// `template_regex` of the reference test harness: `%s` a lazy
/// group, `%d` a number, the first letter either case, trailing `%s`s optional.
fn template_regex(t: &str) -> regex::Regex {
    let mut t = t.trim_end().to_string();
    let mut tail = String::new();
    let trailing = regex::Regex::new(r"(?:\s*%s)+$").unwrap();
    if let Some(m) = trailing.find(&t)
        && m.start() > 0
    {
        t.truncate(m.start());
        tail = r"(?:\s.*)?".into();
    }
    let mut out = String::new();
    let mut first = true;
    for part in regex::Regex::new(r"%[sd]|%%").unwrap().split(&t).zip(
        regex::Regex::new(r"%[sd]|%%")
            .unwrap()
            .find_iter(&t)
            .map(|m| Some(m.as_str()))
            .chain(std::iter::once(None)),
    ) {
        let (lit, tok) = part;
        if !lit.is_empty() {
            let mut cs = lit.chars();
            let c = cs.next().unwrap();
            if first && c.is_alphabetic() {
                out.push_str(&format!(
                    "[{}{}]{}",
                    c.to_ascii_uppercase(),
                    c.to_ascii_lowercase(),
                    regex::escape(cs.as_str())
                ));
            } else {
                out.push_str(&regex::escape(lit));
            }
        }
        first = false;
        match tok {
            Some("%s") => out.push_str(".*?"),
            Some("%d") => out.push_str(r"-?\d+"),
            Some("%%") => out.push('%'),
            _ => {}
        }
    }
    regex::Regex::new(&format!("^(?:{out}{tail})$")).unwrap()
}

/// The generated families of a monster's slot 0 (as the world load makes them): `you` and `other`.
fn families(w: &World, n: i32) -> (Vec<regex::Regex>, Vec<regex::Regex>) {
    let k = &w.monsters[&MonsterNo(n)];
    let a = &k.attacks[0];
    let m = |x: MessageNo| w.messages[&x].msgs.clone();
    let (h, d, s) = (m(a.hitmsg), m(a.defmsg), m(a.missmsg));
    // hit[0] mon_hits_you, hit[1] mon_hits_other, def[0] mon_deflect_you, def[1]
    // mon_deflect_other, def[2] mon_dodge_you, miss[0] mon_dodge_other, miss[1]
    // mon_miss_you, miss[2] mon_miss_other
    let you = [&h[0], &d[0], &d[2], &s[1]];
    let other = [&h[1], &d[1], &s[0], &s[2]];
    (
        you.iter().map(|t| template_regex(t)).collect(),
        other.iter().map(|t| template_regex(t)).collect(),
    )
}

fn static_pattern(name: &str) -> regex::Regex {
    let reg: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/patterns.json")).unwrap();
    let rx = reg["static"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no pattern {name}"))["rx"]
        .as_str()
        .unwrap()
        .to_owned();
    regex::Regex::new(&format!("^(?:{rx})$")).unwrap()
}

// The three stock monsters' blows, to the victim and the room, match
// the families generated from their WCCMSG records; the lines are snapshotted.
#[test]
fn monster_texts_match_the_generated_families() {
    let mut all = String::new();
    for n in [RAT, SLIME, KOBOLD] {
        let mut w = world();
        let id = hero(&mut w, "Kasimir", ARENA);
        let watcher = hero(&mut w, "Bob", ARENA);
        // Bob is out of sight: hidden, so only Kasimir is attacked
        plr(&mut w, watcher).hidden = 1;
        // hit chance 100 - dv^2/(av^2/140): against an unarmoured
        // player the kobold (15) and the slime (20) never miss, so Kasimir wears AC 4
        plr(&mut w, id).worn[0] = VEST;
        // and dodges now and then (a third of agility above 50)
        plr(&mut w, id).agi = 80;
        let m = place(&mut w, n, ARENA);
        text(&mut w, id);
        text(&mut w, watcher);
        let (you, other) = families(&w, n);
        let name = w.monsters[&MonsterNo(n)].name.clone();
        let mut kinds = std::collections::BTreeSet::new();
        let mut shown = Vec::new();
        for _ in 0..200 {
            reset(&mut w, m, &[]);
            plr(&mut w, id).hp = 30000;
            do_monster_combat(&mut w);
            for l in text(&mut w, id).into_iter().filter(|l| l.contains(&name)) {
                let i = you.iter().position(|r| r.is_match(&l));
                assert!(i.is_some(), "you: {l:?}");
                if kinds.insert(("you", i)) {
                    shown.push(format!("you   {l}"));
                }
            }
            for l in text(&mut w, watcher)
                .into_iter()
                .filter(|l| l.contains(&name))
            {
                let i = other.iter().position(|r| r.is_match(&l));
                assert!(i.is_some(), "other: {l:?}");
                if kinds.insert(("other", i)) {
                    shown.push(format!("other {l}"));
                }
            }
        }
        assert!(kinds.len() >= 4, "{name}: {kinds:?} {shown:?}");
        all.push_str(&format!("# {name}\n{}\n", shown.join("\n")));
    }
    insta::assert_snapshot!("monster_texts", all);
}

// `%s moves to attack %s.` to the room when a player starts on a
// monster; `%s appears!` when the attacking monster is missing from
// its room's list (it is put back).
#[test]
fn moves_to_attack_and_appears() {
    let mut w = world();
    let id = hero(&mut w, "Kasimir", ARENA);
    let watcher = hero(&mut w, "Bob", ARENA);
    plr(&mut w, watcher).hidden = 1;
    let rat = place(&mut w, RAT, ARENA);
    text(&mut w, id);
    text(&mut w, watcher);
    attack_user_monster(&mut w, id, rat, Move::Attack, Move::Attack, Mode::Command);
    let ls = text(&mut w, watcher);
    let moves = ls
        .iter()
        .find(|l| l.contains("moves to attack"))
        .unwrap_or_else(|| panic!("{ls:?}"))
        .clone();
    assert!(static_pattern("attacks_other").is_match(&moves));
    // out of the room's list
    let key = RoomKey::new(1, ARENA);
    let slots = &mut w.room_state.get_mut(&key).unwrap().monsters;
    *slots.iter_mut().find(|s| **s == rat).unwrap() = ActiveMonsterId(0);
    reset(&mut w, rat, &[]);
    text(&mut w, id);
    attack_monster_user(&mut w, rat, id);
    let ls = text(&mut w, id);
    let appears = ls
        .iter()
        .find(|l| l.ends_with("appears!"))
        .unwrap_or_else(|| panic!("{ls:?}"))
        .clone();
    assert!(static_pattern("summoned_appears").is_match(&appears));
    assert!(w.room_state[&key].monsters.contains(&rat), "put back");
    insta::assert_snapshot!("moves_to_attack_and_appears", format!("{moves}\n{appears}"));
}

// T-086: the energy round walks the monster cache, so a monster not in
// it keeps its energy and its target until something reads it in
#[test]
fn energy_skips_monsters_outside_the_cache() {
    let mut w = world();
    let m = place(&mut w, RAT, EAST);
    *w.active_monsters.cache_mut() = silvermere_engine::monsters::cache::MonsterCache::default();
    {
        let mon = w.active_monsters.peek_mut(m).unwrap();
        mon.energy = 0;
        mon.target = 7;
    }
    energy_update_monsters(&mut w);
    let mon = w.active_monsters.peek(m).unwrap();
    assert_eq!((mon.energy, mon.target), (0, 7));
    w.active_monsters.touch(m);
    energy_update_monsters(&mut w);
    let mon = w.active_monsters.peek(m).unwrap();
    assert_eq!((mon.energy, mon.target), (mon.maxenergy as u16, -1));
}
