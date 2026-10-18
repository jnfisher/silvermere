//! The timer queue runs each periodic routine at the period seen on the original and,
//! within a tick, in the order its effects appear there.

mod common;

use silvermere_engine::scheduler::{Periods, on_fast_tick};
use silvermere_engine::{EngineInput, Ran, Routine, Scheduler, World};
use silvermere_types::ids::RoomKey;

use Routine::*;

/// The routines due per tick over the first minute, as our queue's insertion rule
/// (a new node goes ahead of nodes due at the same tick unless the head is due then) and
/// start order produce them, matching the order seen on the original. Energy and
/// monster creation share a period, so their order alternates every five seconds.
const EXPECTED: &[(u64, &[Routine])] = &[
    (3, &[Medium]),
    (5, &[Energy, Spawn]),
    (6, &[Medium]),
    (9, &[Medium]),
    (10, &[Spawn, Energy]),
    (12, &[Medium]),
    (15, &[Medium, Energy, Spawn]),
    (18, &[Medium]),
    (20, &[Spawn, Energy]),
    (21, &[Medium]),
    (24, &[Medium]),
    (25, &[Energy, Spawn]),
    (27, &[Medium]),
    (30, &[Medium, Spawn, Energy, Slow]),
    (33, &[Medium]),
    (35, &[Energy, Spawn]),
    (36, &[Medium]),
    (39, &[Medium]),
    (40, &[Spawn, Energy]),
    (42, &[Medium]),
    (45, &[Medium, Energy, Spawn]),
    (48, &[Medium]),
    (50, &[Spawn, Energy]),
    (51, &[Medium]),
    (54, &[Medium]),
    (55, &[Energy, Spawn]),
    (57, &[Medium]),
    (60, &[Medium, Spawn, Energy, Slow, Save]),
];

// The fast cycle and the periods of every routine.
#[test]
fn sixty_ticks_run_the_periods_in_the_originals_order() {
    let (mut engine, clock, _rx) = common::engine(4);
    let mut log: Vec<(u64, Vec<Ran>)> = Vec::new();
    for _ in 0..60 {
        clock.advance_ticks(1, &mut engine);
        let report = engine.last_tick.clone().expect("a tick report");
        log.push((report.tick, report.ran));
    }
    assert_eq!(log.len(), 60);
    for (tick, ran) in &log {
        let routines: Vec<Routine> = ran
            .iter()
            .filter_map(|r| match r {
                Ran::Routine(x) => Some(*x),
                _ => None,
            })
            .collect();
        let expected: &[Routine] = EXPECTED
            .iter()
            .find(|(t, _)| t == tick)
            .map(|(_, r)| *r)
            .unwrap_or(&[]);
        assert_eq!(routines, expected, "routines at tick {tick}");
        // After the kicks, the character update, then the monster update, every tick.
        let tail = &ran[ran.len() - 2..];
        assert_eq!(
            tail,
            [Ran::CharacterUpdate, Ran::MonsterUpdate],
            "tail at tick {tick}"
        );
        assert_eq!(
            ran.len(),
            routines.len() + 2,
            "nothing else ran at tick {tick}: {ran:?}"
        );
    }
    let energy: Vec<u64> = ticks_of(&log, Energy);
    assert_eq!(energy, (1..=12).map(|n| n * 5).collect::<Vec<_>>());
    assert_eq!(
        ticks_of(&log, Medium),
        (1..=20).map(|n| n * 3).collect::<Vec<_>>()
    );
    assert_eq!(ticks_of(&log, Slow), [30, 60]);
    assert_eq!(
        ticks_of(&log, Spawn),
        (1..=12).map(|n| n * 5).collect::<Vec<_>>()
    );
    assert_eq!(ticks_of(&log, Save), [60]);
    assert!(ticks_of(&log, Inactivity).is_empty());
    // Inactivity is still pending for tick 3600.
    let inact = engine
        .world
        .scheduler
        .pending()
        .find(|k| k.routine == Inactivity)
        .unwrap();
    assert_eq!(inact.when, 3600);
}

fn ticks_of(log: &[(u64, Vec<Ran>)], r: Routine) -> Vec<u64> {
    log.iter()
        .filter(|(_, ran)| ran.contains(&Ran::Routine(r)))
        .map(|(t, _)| *t)
        .collect()
}

// The restock counter.
#[test]
fn restock_runs_on_every_21st_slow_cycle() {
    let mut world = common::tiny_world(1);
    let mut restocks = Vec::new();
    for _ in 0..(30 * 43) {
        let r = on_fast_tick(&mut world);
        if r.ran.contains(&Ran::Restock) {
            restocks.push(r.tick);
        }
    }
    assert_eq!(restocks, [21 * 30, 42 * 30]);
    // The restock follows the slow cycle body in the same tick.
    let mut world = common::tiny_world(1);
    let mut last = None;
    for _ in 0..(30 * 21) {
        last = Some(on_fast_tick(&mut world));
    }
    let ran = last.unwrap().ran;
    let slow = ran.iter().position(|r| *r == Ran::Routine(Slow)).unwrap();
    assert_eq!(ran[slow + 1], Ran::Restock);
}

// Each cycle re-arms at the end of its body
#[test]
fn re_arm_happens_after_the_body_and_never_double_fires() {
    let mut world = common::tiny_world(1);
    // A tick that advances inside a routine body (a slow routine) must not make that
    // routine fire twice: the re-arm is relative to the tick after the body.
    world.scheduler = Scheduler::new(Periods::from_config(&common::test_config()));
    world.scheduler.arm(2, Energy);
    let r = on_fast_tick(&mut world);
    assert!(r.ran.iter().all(|x| !matches!(x, Ran::Routine(_))));
    let r = on_fast_tick(&mut world);
    assert_eq!(r.ran[0], Ran::Routine(Energy));
    let next = world
        .scheduler
        .pending()
        .find(|k| k.routine == Energy)
        .unwrap();
    assert_eq!(next.when, 2 + 5);
    // Only one Energy node is pending after the run.
    assert_eq!(
        world
            .scheduler
            .pending()
            .filter(|k| k.routine == Energy)
            .count(),
        1
    );
}

// An exit's delay arms a one-shot exit update.
#[test]
fn a_one_shot_routine_runs_once_and_is_not_re_armed() {
    let mut world: World = common::tiny_world(1);
    let revert = ExitRevert {
        room: RoomKey::new(1, 2140),
        dir: 3,
    };
    world.scheduler.arm(2, revert);
    on_fast_tick(&mut world);
    let r = on_fast_tick(&mut world);
    assert!(r.ran.contains(&Ran::Routine(revert)));
    assert!(world.scheduler.pending().all(|k| k.routine != revert));
}

// The stock WCCMMUD.MSG SECOTH of 160 rounds down to 120
#[test]
fn the_stock_config_saves_every_120_seconds() {
    let p = Periods::from_config(&silvermere_types::config::GameConfig::default());
    assert_eq!(p.save, 120);
    assert_eq!(
        (p.energy, p.medium, p.slow, p.spawn, p.inactivity),
        (5, 3, 30, 5, 3600)
    );
}

#[test]
fn a_tick_input_drives_the_same_queue() {
    let (mut engine, _clock, _rx) = common::engine(1);
    for _ in 0..3 {
        engine.handle(EngineInput::Tick);
    }
    assert_eq!(engine.world.scheduler.tick(), 3);
    assert_eq!(
        engine.last_tick.as_ref().unwrap().ran[0],
        Ran::Routine(Medium)
    );
}
