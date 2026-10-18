//! The evil law (T-038) through the engine: `add_evil_points` from
//! the attack and rob commands, the evil timers, `is_in_retaliation`,
//! FORGIVE (`cmd_forgive`; `attempt_to_forgive`), the star of
//! `Also here:`, the points kept at death and the
//! evil reincarnation room (`check_kill_user`).

mod common;

use std::sync::mpsc::channel;

use common::{TestClock, enter, test_config, tiny_snapshot};
use silvermere_engine::combat::{KillOutcome, check_kill_user};
use silvermere_engine::commands::input::execute_input;
use silvermere_engine::evil::{self, TimerKind};
use silvermere_engine::movement::move_user::user_allowed_in_room;
use silvermere_engine::world::{World, blank};
use silvermere_engine::{AccountRef, Engine, SessionId};
use silvermere_rules::SeededRng;
use silvermere_rules::evil::{Kind, Legal, legal_level};
use silvermere_text::strip_ansi;
use silvermere_types::enums::{Alignment, GroupRole, MessageLevel, MonsterType, RoomType};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerSettings, RoomFlags};
use silvermere_types::ids::{
    AccountId, ActiveMonsterId, ItemNo, MapId, MonsterNo, RoomId, RoomKey,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Character, Coins};

const ARENA: u32 = 2150;
const SAFE: u32 = 2151;
const GUARD: MonsterNo = MonsterNo(10);

fn world_with(align: Alignment) -> World {
    let mut s = tiny_snapshot();
    let mut arena = blank::clear_room(RoomKey::new(1, ARENA));
    arena.name = "Newhaven Arena".into();
    s.rooms.push(arena);
    let mut safe = blank::clear_room(RoomKey::new(1, SAFE));
    safe.name = "Temple".into();
    safe.flags.insert(RoomFlags::PROTECTED);
    s.rooms.push(safe);
    let mut k = blank::clear_known_monster(GUARD);
    k.name = "guardsman".into();
    k.level = 1;
    k.maxhp = 20;
    k.energy = 1000;
    k.alignment = align;
    k.montype = MonsterType::Lair;
    s.monsters.push(k);
    let mut config = test_config();
    config.death_hp = -10;
    let mut w = World::load(config, 8, s).expect("world loads");
    w.rng = SeededRng::new(7);
    w
}

fn world() -> World {
    world_with(Alignment::Good)
}

/// A level-5 player in the arena with an account and warnings off.
fn player_in(w: &mut World, name: &str) -> SessionId {
    let id = enter(w, name, ARENA, MessageLevel::High);
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
    c.settings = PlayerSettings::VALIDATED;
    c.level = 5;
    (c.str, c.intel, c.wil, c.hea, c.agi, c.cha) = (55, 30, 45, 50, 30, 30);
    (c.hp, c.maxhp) = (48, 48);
    (c.energy, c.maxenergy) = (1000, 1000);
    c.lives = 9;
    id
}

fn plr(w: &mut World, id: SessionId) -> &mut Character {
    w.session_mut(id).unwrap().character.as_mut().unwrap()
}

/// The guardsman, monster 1, in the arena.
fn spawn(w: &mut World) -> ActiveMonsterId {
    let k = w.monsters.get(&GUARD).unwrap().clone();
    let number = ActiveMonsterId(1);
    let mon = ActiveMonster {
        number,
        knownnum: k.number,
        level: k.level,
        map: MapId(1),
        room: RoomId(ARENA),
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
        genroom: RoomId(ARENA),
        inactive: 0,
        flags: ActiveMonsterFlags::empty(),
        montype: k.montype,
        target: -1,
        hpregen: k.hpregen,
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
    };
    w.active_monsters.insert(mon);
    let rs = w.room_state.get_mut(&RoomKey::new(1, ARENA)).unwrap();
    let slot = rs.monsters.iter_mut().find(|m| m.0 == 0).unwrap();
    *slot = number;
    number
}

/// The screen lines written to `id` since the last call: ANSI off, prompts dropped.
fn lines(w: &mut World, id: SessionId) -> Vec<String> {
    let prompt = silvermere_engine::tell::prompt_bytes(w, id);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    let text = String::from_utf8_lossy(&b).replace("\x1b[79D\x1b[K", "\r\n");
    let plain = String::from_utf8_lossy(&strip_ansi(text.as_bytes())).into_owned();
    plain
        .split("\r\n")
        .map(|l| l.trim_matches([' ', '\u{7}']).to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with("[HP="))
        .collect()
}

/// Everything written to `id` since the last call, ANSI kept.
fn raw(w: &mut World, id: SessionId) -> String {
    let prompt = silvermere_engine::tell::prompt_bytes(w, id);
    let palette = silvermere_engine::tell::palette_of(w, id);
    let s = w.session_mut(id).unwrap();
    let b = s.output.render(palette, &prompt);
    s.output.clear();
    String::from_utf8_lossy(&b).into_owned()
}

fn run(w: &mut World, id: SessionId, line: &str) -> Vec<String> {
    lines(w, id);
    execute_input(w, id, line, false);
    lines(w, id)
}

const WARNING: &str = "To do this action, you must turn off your evil warnings.";
const TOO_EVIL: &str = "You have progressed too far to the evil side to do this action.";
const LAWFUL: &str = "You have chosen a way of life which does not allow this action.";
const CLOUD: &str = "A dark cloud passes over you";
const ENGAGED: &str = "*Combat Engaged*";

// A good monster with the warnings on
#[test]
fn a_good_monster_with_warnings_on_is_refused() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    spawn(&mut w);
    plr(&mut w, k).settings.insert(PlayerSettings::EVILWARN);
    assert_eq!(run(&mut w, k, "a guardsman"), [WARNING]);
    assert_eq!(plr(&mut w, k).evil, 0);
}

// Warnings off, the points and the fight
#[test]
fn a_good_monster_with_warnings_off_costs_ten() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    spawn(&mut w);
    let out = raw(&mut w, k);
    drop(out);
    execute_input(&mut w, k, "a guardsman", false);
    let out = raw(&mut w, k);
    // dark grey (ANSI_DKGRAY)
    assert!(
        out.contains("\x1b[1;30mA dark cloud passes over you\r"),
        "{out:?}"
    );
    let plain = String::from_utf8_lossy(&strip_ansi(out.as_bytes())).into_owned();
    assert!(plain.find(CLOUD).unwrap() < plain.find(ENGAGED).unwrap());
    let c = plr(&mut w, k);
    assert_eq!(c.evil, 10);
    assert!(c.settings.contains(PlayerSettings::EVILADDED));
    // no player victim: no timer
    assert!(w.evil_timers.is_empty());
}

// A lawful character, and the order of the refusals
#[test]
fn lawful_and_too_evil_are_refused() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    spawn(&mut w);
    plr(&mut w, k).lawful = -10;
    assert_eq!(run(&mut w, k, "a guardsman"), [LAWFUL]);
    // the warnings are asked first
    plr(&mut w, k).settings.insert(PlayerSettings::EVILWARN);
    assert_eq!(run(&mut w, k, "a guardsman"), [WARNING]);
    plr(&mut w, k).settings.remove(PlayerSettings::EVILWARN);
    // over 300 before lawful
    plr(&mut w, k).evil = 301;
    assert_eq!(run(&mut w, k, "a guardsman"), [TOO_EVIL]);
    plr(&mut w, k).lawful = 0;
    assert_eq!(run(&mut w, k, "a guardsman"), [TOO_EVIL]);
    assert_eq!(plr(&mut w, k).evil, 301);
    // ENABLAWF off: a lawful record is no bar
    w.config.enable_lawful = false;
    plr(&mut w, k).lawful = -10;
    plr(&mut w, k).evil = 0;
    let out = run(&mut w, k, "a guardsman");
    assert_eq!(out[..2], [CLOUD, ENGAGED]);
}

// Only Good and Lawful Good cost points
#[test]
fn neutral_and_evil_monsters_are_free() {
    for align in [
        Alignment::Neutral,
        Alignment::ChaoticEvil,
        Alignment::Evil,
        Alignment::LawfulEvil,
    ] {
        let mut w = world_with(align);
        let k = player_in(&mut w, "Kasimir");
        spawn(&mut w);
        plr(&mut w, k).settings.insert(PlayerSettings::EVILWARN);
        let out = run(&mut w, k, "a guardsman");
        assert_eq!(out.first().map(String::as_str), Some(ENGAGED), "{align:?}");
        assert!(!out.iter().any(|l| l == WARNING || l == CLOUD));
        assert_eq!(plr(&mut w, k).evil, 0);
    }
    let mut w = world_with(Alignment::LawfulGood);
    let k = player_in(&mut w, "Kasimir");
    spawn(&mut w);
    assert_eq!(run(&mut w, k, "a guardsman")[..2], [CLOUD, ENGAGED]);
}

// PvP timers
#[test]
fn pvp_starts_a_timer_and_the_window_is_free() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    assert_eq!(run(&mut w, k, "a bob"), [CLOUD, ENGAGED]);
    assert_eq!(plr(&mut w, k).evil, 10);
    assert_eq!(
        evil::already_evil(&w, k, bob),
        Some(TimerKind::Attack),
        "{:?}",
        w.evil_timers
    );
    assert_eq!(w.evil_timers[0].rounds, 11);
    assert_eq!(w.evil_timers[0].points, 10);
    assert!(evil::is_in_retaliation(&w, k));
    // the second attack in the window costs nothing
    let out = run(&mut w, k, "a bob");
    assert!(!out.iter().any(|l| l == CLOUD), "{out:?}");
    assert_eq!(plr(&mut w, k).evil, 10);
    // nor does the victim striking back
    lines(&mut w, bob);
    plr(&mut w, bob).settings.insert(PlayerSettings::EVILWARN);
    let out = run(&mut w, bob, "a kasimir");
    assert!(!out.iter().any(|l| l == CLOUD || l == WARNING), "{out:?}");
    assert_eq!(plr(&mut w, bob).evil, 0);
    assert_eq!(w.evil_timers.len(), 1);
    assert!(!evil::is_in_retaliation(&w, bob));
}

// The victim scales the cost; an evil victim is free
#[test]
fn the_victim_scales_the_cost() {
    for (victim_evil, lawful, cost) in [(-51, 0, 20), (-201, 0, 30), (0, -10, 30), (30, 0, 0)] {
        let mut w = world();
        let k = player_in(&mut w, "Kasimir");
        let bob = player_in(&mut w, "Bob");
        plr(&mut w, bob).evil = victim_evil;
        plr(&mut w, bob).lawful = lawful;
        let out = run(&mut w, k, "a bob");
        assert_eq!(out.contains(&CLOUD.to_owned()), cost != 0, "{out:?}");
        assert_eq!(plr(&mut w, k).evil, cost);
        assert_eq!(w.evil_timers[0].points, cost);
        // a 0-point timer is no retaliation period
        assert_eq!(evil::is_in_retaliation(&w, k), cost != 0);
    }
    // a good attacker lands on 10 at least
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    player_in(&mut w, "Bob");
    plr(&mut w, k).evil = -100;
    run(&mut w, k, "a bob");
    assert_eq!(plr(&mut w, k).evil, 10);
}

// Through slow_update_character: 11 slow ticks
#[test]
fn the_timer_runs_out_on_the_slow_tick() {
    let mut world = world();
    let k = player_in(&mut world, "Kasimir");
    let bob = player_in(&mut world, "Bob");
    assert_eq!(run(&mut world, k, "a bob"), [CLOUD, ENGAGED]);
    let (tx, _rx) = channel();
    let clock = TestClock::new();
    let mut engine = Engine::new(world, clock.clone(), SeededRng::new(1), tx);
    // ten slow ticks (30 s each) leave one round
    clock.advance_ticks(300, &mut engine);
    assert_eq!(engine.world.evil_timers.len(), 1);
    assert_eq!(engine.world.evil_timers[0].rounds, 1);
    assert!(evil::is_in_retaliation(&engine.world, k));
    clock.advance_ticks(30, &mut engine);
    assert!(engine.world.evil_timers.is_empty());
    assert!(!evil::is_in_retaliation(&engine.world, k));
    assert!(evil::already_evil(&engine.world, k, bob).is_none());
}

// No protected room during a retaliation period
#[test]
fn retaliation_bars_a_protected_room() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    let safe = RoomKey::new(1, SAFE);
    assert!(user_allowed_in_room(
        &mut w,
        k,
        safe,
        RoomType::Normal,
        false
    ));
    evil::add_evil_points(&mut w, k, Some(bob), 10, 11, Kind::Attack);
    lines(&mut w, k);
    assert!(!user_allowed_in_room(
        &mut w,
        k,
        safe,
        RoomType::Normal,
        false
    ));
    assert_eq!(
        lines(&mut w, k),
        ["You may not enter that room during a retaliation time-period."]
    );
}

#[test]
fn forgive_removes_the_timer_and_the_points() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    assert_eq!(run(&mut w, k, "a bob"), [CLOUD, ENGAGED]);
    lines(&mut w, k);
    lines(&mut w, bob);
    // only the victim can forgive
    assert_eq!(
        run(&mut w, k, "forgive bob"),
        ["The gods refuse to forgive him for his actions."]
    );
    assert_eq!(
        run(&mut w, bob, "forgive kasimir"),
        ["The gods have forgiven him for his action."]
    );
    assert_eq!(
        lines(&mut w, k),
        ["The gods have forgiven you for your action."]
    );
    assert_eq!(plr(&mut w, k).evil, 0);
    assert!(w.evil_timers.is_empty());
    assert_eq!(
        run(&mut w, bob, "forgive kasimir"),
        ["The gods refuse to forgive him for his actions."]
    );
    assert_eq!(
        run(&mut w, bob, "forgive zed"),
        ["You do not see zed here!"]
    );
}

// The later timers of the list
#[test]
fn forgive_finds_a_later_timer() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    let eve = player_in(&mut w, "Eve");
    evil::add_evil_points(&mut w, k, Some(eve), 10, 11, Kind::Attack);
    evil::add_evil_points(&mut w, k, Some(bob), 10, 11, Kind::Attack);
    assert_eq!(plr(&mut w, k).evil, 20);
    assert!(evil::attempt_to_forgive(&mut w, k, bob));
    assert_eq!(plr(&mut w, k).evil, 10);
    assert_eq!(w.evil_timers.len(), 1);
    assert_eq!(w.evil_timers[0].victim, eve);
}

fn also_here(w: &mut World, id: SessionId) -> String {
    run(w, id, "look")
        .into_iter()
        .find(|l| l.starts_with("Also here:"))
        .unwrap_or_default()
}

// The star
#[test]
fn the_star_shows_while_a_timer_runs() {
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    let eve = player_in(&mut w, "Eve");
    assert_eq!(also_here(&mut w, bob), "Also here: Kasimir, Eve.");
    evil::add_evil_points(&mut w, k, Some(bob), 10, 11, Kind::Attack);
    // either way round
    assert_eq!(also_here(&mut w, bob), "Also here: Kasimir*, Eve.");
    assert_eq!(also_here(&mut w, k), "Also here: Bob*, Eve.");
    // a bystander sees no star at 10 points
    assert_eq!(also_here(&mut w, eve), "Also here: Kasimir, Bob.");
    // an Outlaw is starred for everyone
    plr(&mut w, k).evil = 40;
    assert_eq!(also_here(&mut w, eve), "Also here: Kasimir*, Bob.");
    // a robbery the victim did not see withholds it (KIND_NOSTAR)
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    evil::add_evil_points(&mut w, k, Some(bob), 1, 11, Kind::NoStar);
    assert_eq!(evil::already_evil(&w, k, bob), Some(TimerKind::RobNoStar));
    assert_eq!(also_here(&mut w, bob), "Also here: Kasimir.");
    assert_eq!(also_here(&mut w, k), "Also here: Bob*.");
    // a seen robbery keeps the star on a later unseen one
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let bob = player_in(&mut w, "Bob");
    evil::add_evil_points(&mut w, k, Some(bob), 1, 11, Kind::Robbery);
    evil::add_evil_points(&mut w, k, Some(bob), 1, 11, Kind::NoStar);
    assert_eq!(evil::already_evil(&w, k, bob), Some(TimerKind::Robbery));
    assert_eq!(plr(&mut w, k).evil, 2);
    // an attack replaces a robbery timer (should_give_evil 1)
    evil::add_evil_points(&mut w, k, Some(bob), 10, 11, Kind::Attack);
    assert_eq!(evil::already_evil(&w, k, bob), Some(TimerKind::Attack));
    assert_eq!(w.evil_timers.len(), 1);
    assert_eq!(plr(&mut w, k).evil, 12);
}

// ROB is refused with the warnings on or for a lawful character
#[test]
fn rob_refusals() {
    let rob_lawful = "You have chosen a way of life which prevents this action.";
    let mut w = world();
    let k = player_in(&mut w, "Kasimir");
    let _bob = player_in(&mut w, "Bob");
    plr(&mut w, k).settings.insert(PlayerSettings::EVILWARN);
    assert_eq!(run(&mut w, k, "rob bob"), [rob_lawful]);
    plr(&mut w, k).settings.remove(PlayerSettings::EVILWARN);
    plr(&mut w, k).lawful = -10;
    plr(&mut w, k).delay = 0;
    assert_eq!(run(&mut w, k, "rob bob"), [rob_lawful]);
    assert!(w.evil_timers.is_empty());
    // too evil: add_evil_points' own refusal
    plr(&mut w, k).lawful = 0;
    plr(&mut w, k).evil = 301;
    plr(&mut w, k).delay = 0;
    assert_eq!(run(&mut w, k, "rob bob"), [TOO_EVIL]);
    assert_eq!(plr(&mut w, k).evil, 301);
}

#[test]
fn legal_level_boundaries() {
    use Legal::*;
    let table = [
        (-201, Saint),
        (-200, Good),
        (-51, Good),
        (-50, Neutral),
        (29, Neutral),
        (30, Seedy),
        (39, Seedy),
        (40, Outlaw),
        (79, Outlaw),
        (80, Criminal),
        (119, Criminal),
        (120, Villain),
        (209, Villain),
        (210, Fiend),
    ];
    for (evil, level) in table {
        assert_eq!(legal_level(evil), level, "{evil}");
    }
}

// The kept points and the next character
#[test]
fn death_keeps_evilkeep_percent_for_the_next_character() {
    let mut w = world();
    w.config.evil_keep = 50;
    let k = player_in(&mut w, "Kasimir");
    let account = w.session(k).unwrap().account.as_ref().unwrap().id;
    plr(&mut w, k).evil = 100;
    plr(&mut w, k).deathdate = -1;
    plr(&mut w, k).lives = 1;
    plr(&mut w, k).hp = -10;
    assert_eq!(check_kill_user(&mut w, k), KillOutcome::Deleted);
    assert_eq!(w.evil_points.get(&account).map(|e| e.evil), Some(50));
    let next = silvermere_engine::player::create::create_player(&w, account, "KASIMIR", 0);
    assert_eq!(next.evil, 50);
}

// check_kill_user: 40 or more points wake in evilreincroom
#[test]
fn rebirth_in_the_evil_room_from_forty() {
    for (evil, room) in [(39, 0), (40, 1)] {
        let mut w = world();
        let k = player_in(&mut w, "Kasimir");
        plr(&mut w, k).evil = evil;
        plr(&mut w, k).hp = -10;
        assert_eq!(check_kill_user(&mut w, k), KillOutcome::Reborn);
        let want = [w.config.reinc_room, w.config.evil_reinc_room][room];
        assert_eq!(plr(&mut w, k).room, RoomId(want), "{evil}");
    }
}
