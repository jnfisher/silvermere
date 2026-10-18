//! T-037: the background cycles on a hand-driven clock. Each test drives
//! `scheduler::on_fast_tick` (what `EngineInput::Tick` runs) one simulated second at a
//! time over the tiny world, and reads what the cycles queued for each session.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use silvermere_engine::commands::rest::{cmd_meditate, cmd_rest};
use silvermere_engine::cycles::{CycleHooks, MoveKind};
use silvermere_engine::output::Item;
use silvermere_engine::scheduler::on_fast_tick;
use silvermere_engine::{Ran, Routine, SeededRng, SessionId, World};
use silvermere_text::Msg;
use silvermere_types::enums::{
    Ability, Alignment, ExitType, GroupRole, Magery, MessageLevel, MonsterType,
};
use silvermere_types::flags::{ActiveMonsterFlags, PlayerStatus, PlayerStatus2};
use silvermere_types::ids::{
    ActiveMonsterId, ClassNo, ItemNo, MonsterNo, RoomId, RoomKey, SpellNo, TextBlockNo,
};
use silvermere_types::records::{ActiveMonster, ActiveSpell, Coins, TextBlock};
use silvermere_types::sizes::{NAMTRAIL, NMONITEMS, NMONSPELLS};

use common::{enter, special_rooms, tiny_world};

struct Run {
    world: World,
}

impl Run {
    fn new() -> Self {
        let mut world = tiny_world(16);
        world.rng = SeededRng::new(7);
        Self { world }
    }

    fn player(&mut self, name: &str) -> SessionId {
        enter(
            &mut self.world,
            name,
            special_rooms()[0],
            MessageLevel::Medium,
        )
    }

    /// Runs `n` fast ticks; returns the reports' routines in order.
    fn ticks(&mut self, n: u32) -> Vec<Ran> {
        let mut ran = Vec::new();
        for _ in 0..n {
            ran.extend(on_fast_tick(&mut self.world).ran);
        }
        ran
    }

    fn tick(&self) -> u64 {
        self.world.scheduler.tick()
    }

    fn plr(&mut self, id: SessionId) -> &mut silvermere_types::records::Character {
        self.world
            .sessions
            .get_mut(&id)
            .and_then(|s| s.character.as_mut())
            .unwrap()
    }

    /// Takes everything queued for `id`, prompts without their captured state.
    fn out(&mut self, id: SessionId) -> Vec<Item> {
        let items = self.world.sessions.get_mut(&id).unwrap().output.take().0;
        items
            .into_iter()
            .map(|i| match i {
                Item::Prompt(_) => Item::Prompt(None),
                i => i,
            })
            .collect()
    }

    fn msgs(&mut self, id: SessionId) -> Vec<Msg> {
        self.out(id)
            .into_iter()
            .filter_map(|i| match i {
                Item::Msg { msg, .. } => Some(msg),
                _ => None,
            })
            .collect()
    }
}

/// A L1 Kang (the KB's sheet) at 20/48 hit points, health 40.
fn kang(run: &mut Run) -> SessionId {
    let id = run.player("Kang");
    let p = run.plr(id);
    p.level = 1;
    p.hea = 40;
    p.hp = 20;
    p.maxhp = 48;
    id
}

// The slow tick (+max((1+20)*40/750,1) = +1 every 30 s) and the rest tick (rest
// +1*3 = +3 on every 21st fast tick of resting).
#[test]
fn resting_kang_heals_on_the_slow_and_rest_ticks() {
    let mut run = Run::new();
    let id = kang(&mut run);
    cmd_rest(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::RestStart]);
    let mut seen = Vec::new();
    for t in 1..=63 {
        run.ticks(1);
        seen.push((t, run.plr(id).hp));
    }
    let hp_at = |t: usize| seen[t - 1].1;
    assert_eq!(hp_at(20), 20);
    assert_eq!(hp_at(21), 23); // rest tick
    assert_eq!(hp_at(30), 24); // slow tick
    assert_eq!(hp_at(42), 27); // rest tick
    assert_eq!(hp_at(60), 28); // slow tick
    assert_eq!(hp_at(63), 31); // rest tick
    // Silent regen apart from the prompt.
    let out = run.out(id);
    assert!(out.iter().all(|i| *i == Item::Prompt(None)), "{out:?}");
}

// The rest tick and the slow cycle: each regen formats its
// own prompt and sends it, so the two in one second show the hit points
// after each (T-099; the flush used to print the later figure twice).
#[test]
fn two_regen_prompts_in_one_tick_show_their_own_hit_points() {
    let mut run = Run::new();
    let id = kang(&mut run);
    run.plr(id).maxhp = 100;
    cmd_rest(&mut run.world, id);
    run.ticks(209);
    // nine rest ticks (+3) and six slow ticks (+1)
    assert_eq!(run.plr(id).hp, 53);
    run.out(id);
    // tick 210 is both the 10th rest tick and the 7th slow tick
    run.ticks(1);
    assert_eq!(run.plr(id).hp, 57);
    let palette = silvermere_engine::tell::palette_of(&run.world, id);
    let b = run
        .world
        .session(id)
        .unwrap()
        .output
        .render_in(&run.world, id, palette);
    // the slow cycle runs first in the second (+1), then the rest tick (+3)
    let prompt = |hp: u32| format!("\x1b[79D\x1b[K\x1b[0;37m[HP={hp}\x1b[0;37m (Resting) ]:");
    assert_eq!(
        String::from_utf8_lossy(&b),
        [prompt(54), prompt(57)].concat()
    );
}

// The slow cycle works out the secondary stats again before its prompt, so a
// figure that hangs on the abilities is worked out again (T-100).
#[test]
fn the_slow_tick_works_out_the_dynamic_stats() {
    let mut run = Run::new();
    let id = kang(&mut run);
    run.plr(id).avbonus = 99;
    run.ticks(29);
    assert_eq!(run.plr(id).avbonus, 99);
    run.ticks(1);
    assert_eq!(run.plr(id).avbonus, 0);
}

// Standing, the slow tick alone; capped at the maximum.
#[test]
fn standing_heals_only_on_the_slow_tick_and_stops_at_maxhp() {
    let mut run = Run::new();
    let id = kang(&mut run);
    run.plr(id).hp = 47;
    run.ticks(29);
    assert_eq!(run.plr(id).hp, 47);
    assert!(run.out(id).is_empty(), "no prompt before the slow tick");
    run.ticks(1);
    assert_eq!(run.plr(id).hp, 48);
    assert_eq!(run.out(id), [Item::Prompt(None)]);
    run.ticks(30);
    assert_eq!(run.plr(id).hp, 48);
    assert!(run.out(id).is_empty(), "nothing changed: no prompt");
}

fn mage(run: &mut Run) -> SessionId {
    let cls = run.world.classes.get_mut(&ClassNo(1)).unwrap();
    cls.magery = Magery::Mage;
    cls.magelvl = 3;
    cls.abilities[0] = (Ability::Meditate, 1);
    let id = run.player("Gandi");
    let p = run.plr(id);
    p.level = 1;
    p.intel = 60;
    p.mana = 0;
    p.maxmana = 30;
    id
}

// 60*(1+20)*(3+2)/1650 = 3 mana a slow tick; the same each 15th
// fast tick of meditating; the wake line when full.
#[test]
fn mage_mana_on_slow_and_meditate_ticks_then_wakes() {
    let mut run = Run::new();
    let id = mage(&mut run);
    run.ticks(30);
    assert_eq!(run.plr(id).mana, 3);
    run.out(id);
    cmd_meditate(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::MeditateStart]);
    run.ticks(15); // tick 45: meditate
    assert_eq!(run.plr(id).mana, 6);
    assert_eq!(run.out(id), [Item::Prompt(None)]);
    run.ticks(15); // tick 60: slow (+3) then meditate (+3)
    assert_eq!(run.plr(id).mana, 12);
    run.plr(id).mana = 28;
    run.ticks(15); // tick 75: +3, capped at 30, awake
    assert_eq!(run.plr(id).mana, 30);
    assert!(!run.plr(id).status2.contains(PlayerStatus2::MEDITATING));
    assert!(run.msgs(id).contains(&Msg::MeditateDone));
}

#[test]
fn meditate_command_needs_the_ability_and_room_to_gain() {
    let mut run = Run::new();
    let id = run.player("Plain");
    run.plr(id).maxmana = 10;
    cmd_meditate(&mut run.world, id);
    assert!(run.out(id).is_empty(), "no ability: nothing at all");
    let id = mage(&mut run);
    let other = run.player("Bob");
    run.plr(id).mana = 30;
    cmd_meditate(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::MeditateUseless]);
    run.plr(id).mana = 1;
    run.plr(id).poison = 2;
    cmd_meditate(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::MeditateSick]);
    run.plr(id).poison = 0;
    run.plr(id).status.insert(PlayerStatus::RESTING);
    cmd_meditate(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::MeditateStart]);
    assert!(!run.plr(id).status.contains(PlayerStatus::RESTING));
    assert_eq!(
        run.out(other),
        [
            Item::Msg {
                msg: Msg::MeditateOther {
                    who: "Gandi".into()
                },
                colour: None
            },
            Item::Prompt(None)
        ]
    );
    cmd_meditate(&mut run.world, id);
    assert!(run.out(other).is_empty(), "told only the first time");
}

#[test]
fn rest_command_lines() {
    let mut run = Run::new();
    let id = kang(&mut run);
    let other = run.player("Bob");
    run.plr(id).poison = 1;
    cmd_rest(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::RestSick]);
    run.plr(id).poison = 0;
    run.plr(id).status.insert(PlayerStatus::SNEAKING);
    run.plr(id).hidden = 1;
    cmd_rest(&mut run.world, id);
    assert_eq!(run.msgs(id), [Msg::RestStart]);
    assert_eq!(run.msgs(other), [Msg::RestOther { who: "Kang".into() }]);
    let p = run.plr(id);
    assert!(p.status.contains(PlayerStatus::RESTING));
    assert!(!p.status.contains(PlayerStatus::SNEAKING));
    assert_eq!(p.hidden, 0);
    cmd_rest(&mut run.world, id);
    assert!(run.out(other).is_empty());
}

/// The bytes queued for `id`, prompts left out (rendered empty).
fn raw(run: &mut Run, id: SessionId) -> Vec<u8> {
    let palette = silvermere_engine::tell::palette_of(&run.world, id);
    let s = run.world.sessions.get_mut(&id).unwrap();
    let b = s.output.render(palette, b"");
    s.output.clear();
    b
}

// REST and MEDITATE: `tell_room` sends the whole
// output buffer, so the dispatcher's ANSI_NORMAL goes to the room ahead of the line and the
// player's own line comes bare (m4/spells step 22 `rest` against the original, run
// 38904009536). Already resting, nobody else is told and the reset is the player's.
#[test]
fn rest_and_meditate_give_the_pending_reset_to_the_room() {
    use silvermere_engine::commands::input::execute_input;
    let mut run = Run::new();
    let id = kang(&mut run);
    let other = run.player("Bob");
    raw(&mut run, id);
    raw(&mut run, other);
    execute_input(&mut run.world, id, "rest", false);
    assert_eq!(raw(&mut run, id), b"You are now resting.\r\n");
    assert_eq!(
        raw(&mut run, other),
        b"\x1b[0;37;40m\x1b[79D\x1b[KKang stops to rest.\r\n"
    );
    execute_input(&mut run.world, id, "rest", false);
    assert_eq!(raw(&mut run, id), b"\x1b[0;37;40mYou are now resting.\r\n");
    assert!(raw(&mut run, other).is_empty());
    // MEDITATE, by the same rule (Kang has no Meditate: give him a mage's class)
    let mage = mage(&mut run);
    raw(&mut run, mage);
    raw(&mut run, other);
    raw(&mut run, id);
    execute_input(&mut run.world, mage, "meditate", false);
    assert_eq!(raw(&mut run, mage), b"You are now meditating.\r\n");
    assert!(
        raw(&mut run, other).starts_with(b"\x1b[0;37;40m\x1b[79D\x1b[K"),
        "the reset goes to the room"
    );
}

static DEATHS: AtomicUsize = AtomicUsize::new(0);

fn recording_kill(world: &mut World, id: SessionId) -> bool {
    let dead = silvermere_engine::cycles::default_check_kill_user(world, id);
    if dead {
        DEATHS.fetch_add(1, Ordering::SeqCst);
    }
    dead
}

// "You feel ill." then the damage; knocked out going below 0;
// then (same tick) the bleed.
#[test]
fn poison_ticks_feel_ill_and_knocks_out() {
    let mut run = Run::new();
    let id = kang(&mut run);
    let other = run.player("Bob");
    run.plr(id).hp = 3;
    run.plr(id).poison = 5;
    run.ticks(30);
    let out = run.out(id);
    assert_eq!(
        out[0],
        Item::Msg {
            msg: Msg::PoisonTick,
            colour: None
        }
    );
    assert!(out.contains(&Item::Msg {
        msg: Msg::DropsUnconscious { who: "Kang".into() },
        colour: None
    }));
    assert_eq!(run.plr(id).hp, 3 - 5 - 1);
    assert!(
        run.msgs(other)
            .contains(&Msg::DropsUnconscious { who: "Kang".into() })
    );
}

// Down and alone, one point a slow tick to deathhp (T-033's
// check_kill_user); aided, one point back.
#[test]
fn bleeding_to_deathhp_and_aid() {
    let mut run = Run::new();
    run.world.cycles.hooks = CycleHooks {
        check_kill_user: recording_kill,
        ..CycleHooks::default()
    };
    let death_hp = run.world.config.death_hp as i16;
    let id = kang(&mut run);
    let aided = run.player("Aided");
    run.plr(id).hp = death_hp + 2;
    run.plr(aided).hp = -5;
    run.plr(aided).aided = 1;
    DEATHS.store(0, Ordering::SeqCst);
    run.ticks(30);
    assert_eq!(run.plr(id).hp, death_hp + 1);
    assert_eq!(DEATHS.load(Ordering::SeqCst), 0);
    assert_eq!(run.plr(aided).hp, -4);
    run.ticks(30);
    assert_eq!(run.plr(id).hp, death_hp);
    assert_eq!(DEATHS.load(Ordering::SeqCst), 1);
    assert_eq!(run.plr(aided).hp, -3);
    // Back above 0 the regen clears `aided`.
    run.plr(aided).hp = 0;
    run.ticks(30);
    assert_eq!(run.plr(aided).hp, 1);
    run.ticks(30);
    assert_eq!(run.plr(aided).aided, 0);
}

// The telepath, gossip and auction allowances every 3 s, and the
// move count unless over 66% encumbered.
#[test]
fn per_period_counters_reset_every_medium_cycle() {
    let mut run = Run::new();
    let id = kang(&mut run);
    let heavy = run.player("Heavy");
    for who in [id, heavy] {
        let p = run.plr(who);
        p.pagecnt = 3;
        p.gosscnt = 2;
        p.auctcnt = 1;
        p.moves = 2;
    }
    run.plr(heavy).encumpct = 67;
    run.ticks(2);
    assert_eq!(run.plr(id).pagecnt, 3);
    run.ticks(1);
    let p = run.plr(id);
    assert_eq!((p.pagecnt, p.gosscnt, p.auctcnt, p.moves), (0, 0, 0, 0));
    assert_eq!(run.plr(heavy).moves, 2);
    assert_eq!(run.plr(heavy).pagecnt, 0);
}

// One charge per medium pass; the standard lines when it goes out.
#[test]
fn a_torch_burns_a_charge_every_3_seconds() {
    let mut run = Run::new();
    let torch = run.world.items.get_mut(&ItemNo(1)).unwrap();
    torch.name = "torch".into();
    torch.weight = 1;
    let id = kang(&mut run);
    let other = run.player("Bob");
    {
        let p = run.plr(id);
        p.items[0] = ItemNo(1);
        p.itemuses[0] = 3;
        p.readied = 0;
        p.wgt = 1;
        p.light = 1;
    }
    run.ticks(3);
    assert_eq!(run.plr(id).itemuses[0], 2);
    assert_eq!(run.plr(id).lighttime, 1);
    run.ticks(3);
    assert_eq!(run.plr(id).itemuses[0], 1);
    run.msgs(other);
    run.ticks(3);
    let p = run.plr(id);
    assert_eq!(
        (p.readied, p.items[0], p.itemuses[0], p.wgt),
        (-1, ItemNo(0), -2, 0)
    );
    assert_eq!(
        run.msgs(id),
        [Msg::LightOut {
            item: "torch".into()
        }]
    );
    assert_eq!(
        run.msgs(other),
        [Msg::LightOutOther {
            who: "Kang".into(),
            item: "torch".into()
        }]
    );
    run.ticks(3);
    assert_eq!(run.plr(id).light, 0, "the timer ran out");
}

/// A test `move_user`: through the exit to its target on the same map, the trail
/// pushed as T-026 does.
fn test_move(world: &mut World, id: SessionId, dir: u8, _kind: MoveKind) -> bool {
    let p = world.sessions.get(&id).unwrap().player().unwrap();
    let key = RoomKey {
        map: p.map,
        room: p.room,
    };
    let target = world.room(key).unwrap().exits[dir as usize].target;
    if target == RoomId(0) {
        return false;
    }
    let p = world
        .sessions
        .get_mut(&id)
        .unwrap()
        .character
        .as_mut()
        .unwrap();
    p.trailrooms.rotate_right(1);
    p.trailmaps.rotate_right(1);
    p.room = target;
    p.trailrooms[0] = target;
    true
}

// Followers move with direction_player_travelling.
#[test]
fn followers_move_the_tick_after_the_leader() {
    let mut run = Run::new();
    run.world.cycles.hooks = CycleHooks {
        move_user: test_move,
        ..CycleHooks::default()
    };
    let [a, b, ..] = special_rooms();
    let ra = RoomKey::new(1, a);
    run.world.rooms.get_mut(&ra).unwrap().exits[0].target = RoomId(b);
    run.world.rooms.get_mut(&ra).unwrap().exits[0].kind = ExitType::Normal;
    let leader = run.player("Lead");
    let follower = run.player("Foll");
    for id in [leader, follower] {
        let p = run.plr(id);
        p.trailrooms[0] = RoomId(a);
        p.trailmaps[0] = 1.into();
    }
    run.world.sessions.get_mut(&follower).unwrap().party.leader = Some(leader);
    // The leader walks north (T-026's move, here the test's).
    assert!(test_move(&mut run.world, leader, 0, MoveKind::Walk));
    run.plr(leader).trailmaps[0] = 1.into();
    assert_eq!(run.plr(follower).room, RoomId(a));
    run.ticks(1);
    assert_eq!(run.plr(follower).room, RoomId(b));
    run.ticks(1);
    assert_eq!(run.plr(follower).room, RoomId(b), "same room: no move");
    assert_eq!(
        run.world.session(follower).unwrap().party.leader,
        Some(leader)
    );
    // A follower who is down leaves the party line.
    run.plr(follower).hp = 0;
    run.ticks(1);
    assert_eq!(run.world.session(follower).unwrap().party.leader, None);
}

#[test]
fn stun_rise_after_the_timer() {
    let mut run = Run::new();
    let id = kang(&mut run);
    let other = run.player("Bob");
    run.plr(id).status2.insert(PlayerStatus2::KNOCKEDDOWN);
    run.plr(id).pvptimer = 3;
    run.ticks(2);
    assert!(run.msgs(id).is_empty());
    run.ticks(1);
    assert_eq!(run.msgs(id), [Msg::StunRise]);
    assert_eq!(
        run.out(other),
        [
            Item::Msg {
                msg: Msg::StunRiseOther { who: "Kang".into() },
                colour: None
            },
            Item::Prompt(None)
        ]
    );
    assert!(!run.plr(id).status2.contains(PlayerStatus2::KNOCKEDDOWN));
}

static DELAY_STEPS: AtomicUsize = AtomicUsize::new(0);

fn counting_delay(world: &mut World, id: SessionId) {
    // The stun has already been counted this tick when the delay step runs.
    let p = world.session(id).unwrap().player().unwrap();
    assert!(!p.status2.contains(PlayerStatus2::KNOCKEDDOWN) || p.pvptimer > 0);
    DELAY_STEPS.fetch_add(1, Ordering::SeqCst);
}

// The delay and queue step (T-024's) runs once per fast tick for
// each player in the realm, last in the player's update.
#[test]
fn delay_step_runs_once_per_tick_per_player() {
    let mut run = Run::new();
    run.world.cycles.hooks = CycleHooks {
        delay_step: counting_delay,
        ..CycleHooks::default()
    };
    kang(&mut run);
    run.player("Bob");
    DELAY_STEPS.store(0, Ordering::SeqCst);
    run.ticks(10);
    assert_eq!(DELAY_STEPS.load(Ordering::SeqCst), 20);
}

// A roll of 'A' to 'Z' each slow tick gives 'A'..'Y'.
#[test]
fn direction_letters_change_on_the_slow_tick() {
    let mut run = Run::new();
    run.ticks(29);
    assert_eq!(run.world.cycles.direction_name(0, 0), "n \u{8}orth");
    run.ticks(1);
    let first = run.world.cycles.dir_letters;
    for dir in 0..10 {
        assert_eq!(
            run.world.display.dirnames.get(dir),
            run.world.cycles.direction_name(0, dir),
            "the room display reads the same letters"
        );
    }
    assert!(first.iter().flatten().all(|c| (b'A'..=b'Y').contains(c)));
    assert_eq!(
        run.world.cycles.direction_name(2, 9).len(),
        "dow \u{8}n".len()
    );
    run.ticks(29);
    assert_eq!(run.world.cycles.dir_letters, first);
    run.ticks(1);
    assert_ne!(run.world.cycles.dir_letters, first);
}

// The routine periods over ten simulated minutes.
#[test]
fn routine_periods_over_ten_minutes_are_exact() {
    let mut run = Run::new();
    run.player("Kang");
    let ran = run.ticks(600);
    let count = |r: Routine| ran.iter().filter(|x| **x == Ran::Routine(r)).count();
    assert_eq!(count(Routine::Medium), 200);
    assert_eq!(count(Routine::Slow), 20);
    assert_eq!(count(Routine::Energy), 120);
    assert_eq!(count(Routine::Spawn), 120);
    assert_eq!(count(Routine::Save), 10);
    assert_eq!(count(Routine::Inactivity), 0);
    assert_eq!(
        ran.iter().filter(|x| **x == Ran::CharacterUpdate).count(),
        600
    );
    assert_eq!(run.world.cycles.slow_runs, 20);
    assert_eq!(run.world.cycles.medium_runs, 200);
    assert_eq!(run.tick(), 600);
}

// T-021's two strikes, scheduled every `inactive` seconds.
#[test]
fn inactivity_check_is_scheduled_every_period() {
    let mut run = Run::new();
    let ran = run.ticks(7200);
    let at: Vec<usize> = ran
        .split(|r| *r == Ran::MonsterUpdate)
        .enumerate()
        .filter(|(_, tick)| tick.contains(&Ran::Routine(Routine::Inactivity)))
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(at, [3600, 7200]);
}

fn active_monster(number: i32, known: i32) -> ActiveMonster {
    ActiveMonster {
        number: ActiveMonsterId(number),
        knownnum: MonsterNo(known),
        level: 0,
        map: 1.into(),
        room: 0.into(),
        poison: 0,
        energy: 0,
        hp: 0,
        following: String::new(),
        trail: [0.into(); NAMTRAIL],
        hunters: [ActiveMonsterId(0); 10],
        hunting: ActiveMonsterId(0),
        name: "orc".into(),
        wornitem: ItemNo(0),
        weapon: ItemNo(0),
        items: [(ItemNo(0), 0); NMONITEMS],
        cash: Coins::default(),
        maxhp: 0,
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

// Poison, then hpregen up to the maximum, once per slow pass, for a
// monster in the cache while a player is in the game (T-086).
#[test]
fn monsters_take_poison_and_regenerate_on_the_slow_tick() {
    let mut run = Run::new();
    kang(&mut run);
    let mut m = active_monster(5, 1);
    m.hp = 10;
    m.maxhp = 20;
    m.hpregen = 4;
    m.poison = 1;
    m.lastdir = 3;
    let slot = run.world.active_monsters.insert(m);
    run.world.active_monsters.touch(ActiveMonsterId(5));
    run.ticks(30);
    let m = &run.world.active_monsters[slot];
    assert_eq!((m.hp, m.lastdir, m.lastupd), (13, -1, 1));
    run.world.active_monsters[slot].poison = 0;
    run.ticks(60);
    assert_eq!(run.world.active_monsters[slot].hp, 20);
}

// A monster with a random-event text in a player's room.
#[test]
fn random_event_text_goes_to_the_room() {
    let mut run = Run::new();
    let id = kang(&mut run);
    run.world.monsters.get_mut(&MonsterNo(1)).unwrap().randtext = TextBlockNo(9);
    run.world.text_blocks.insert(
        (0, TextBlockNo(9)),
        TextBlock {
            part: 0,
            number: TextBlockNo(9),
            link: TextBlockNo(0),
            text: "100 The orc grunts.".into(),
        },
    );
    run.world.active_monsters.insert(active_monster(5, 1));
    let key = RoomKey::new(1, special_rooms()[0]);
    run.world.room_state.get_mut(&key).unwrap().monsters[0] = ActiveMonsterId(5);
    // A stale entry goes.
    run.world.room_state.get_mut(&key).unwrap().monsters[1] = ActiveMonsterId(6);
    run.world.cycles.order = vec![id];
    silvermere_engine::cycles::slow::perform_random_event(&mut run.world, 94);
    assert_eq!(
        run.out(id),
        [
            Item::Msg {
                msg: Msg::RandomEvent {
                    text: "The orc grunts.".into()
                },
                colour: None
            },
            Item::Prompt(None)
        ]
    );
    silvermere_engine::cycles::slow::perform_random_event(&mut run.world, 95);
    assert!(run.out(id).is_empty(), "95 and up: no event");
    // Monster 5 has its event before the loop reaches slot 1; move it so it does not.
    let rs = run.world.room_state.get_mut(&key).unwrap();
    rs.monsters.swap(0, 1);
    run.world.text_blocks.clear();
    silvermere_engine::cycles::slow::perform_random_event(&mut run.world, 0);
    assert_eq!(
        run.world.room_state[&key].monsters[0],
        ActiveMonsterId(0),
        "the stale entry was removed"
    );
}

// A permutation of the sessions, new every slow cycle.
#[test]
fn shuffled_sessions_is_a_permutation() {
    let mut run = Run::new();
    let ids: Vec<SessionId> = (0..12).map(|i| run.player(&format!("P{i}"))).collect();
    run.ticks(30);
    let mut order = run.world.shuffled_sessions();
    assert_ne!(order, ids, "seeded shuffle moves something");
    order.sort();
    assert_eq!(order, ids);
}

#[test]
fn session_time_counts_slow_ticks() {
    let mut run = Run::new();
    let id = kang(&mut run);
    run.ticks(90);
    assert_eq!(run.world.session(id).unwrap().player().unwrap().sesstime, 3);
}

// The "other" cycle every 60 s (the test config), T-023's
// save_characters: five players a sweep, round-robin from where the
// last sweep stopped.
#[test]
fn the_save_sweep_writes_five_players_a_minute_round_robin() {
    use silvermere_engine::player_session::AccountRef;
    use silvermere_engine::{EngineOutput, PersistRequest};
    use silvermere_types::flags::PlayerSettings;
    use silvermere_types::ids::AccountId;

    let (mut engine, clock, rx) = common::engine(16);
    for i in 0..12 {
        let id = enter(
            &mut engine.world,
            &format!("P{i:02}"),
            special_rooms()[0],
            MessageLevel::Medium,
        );
        let s = engine.world.sessions.get_mut(&id).unwrap();
        s.created = true;
        s.account = Some(AccountRef {
            id: AccountId(i64::from(i) + 1),
            username: format!("p{i:02}"),
            sysop: false,
            gameop: false,
            sex: b'M',
        });
        s.character
            .as_mut()
            .unwrap()
            .settings
            .insert(PlayerSettings::VALIDATED);
    }
    let saved = |rx: &std::sync::mpsc::Receiver<EngineOutput>| -> Vec<i64> {
        common::drain(rx)
            .into_iter()
            .filter_map(|o| match o {
                EngineOutput::Persist(PersistRequest::Character { account, .. }) => Some(account.0),
                _ => None,
            })
            .collect()
    };
    clock.advance_ticks(59, &mut engine);
    assert!(saved(&rx).is_empty());
    clock.advance_ticks(1, &mut engine);
    assert_eq!(saved(&rx), [1, 2, 3, 4, 5]);
    clock.advance_ticks(60, &mut engine);
    assert_eq!(saved(&rx), [6, 7, 8, 9, 10]);
    // Slots 10..16 hold two players; the sweep ends at the last slot and starts over.
    clock.advance_ticks(60, &mut engine);
    assert_eq!(saved(&rx), [11, 12]);
    clock.advance_ticks(60, &mut engine);
    assert_eq!(saved(&rx), [1, 2, 3, 4, 5]);
}
