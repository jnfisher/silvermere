//! T-094: a line and a tick queued at the same moment. MBBSEmu's host loop ran every
//! channel's input in a pass before that pass's timers (`MbbsHost.cs`:339-459 at
//! v1.0-alpha-090826): the module's input routine ran in the pass that echoed the CR
//! (:426, :929), the slow cycle's prompt redraw only after it (:458-459).

mod common;

use silvermere_engine::{EngineInput, EngineOutput, Routine, SessionId};
use silvermere_types::enums::MessageLevel;

/// A wounded player whose slow cycle (and so its prompt redraw) runs on the next tick.
fn wounded() -> (
    silvermere_engine::Engine<common::TestClock, silvermere_engine::SeededRng>,
    std::sync::mpsc::Receiver<EngineOutput>,
    SessionId,
) {
    let (mut engine, _clock, rx) = common::engine(4);
    let id = common::enter(&mut engine.world, "Ann", 2140, MessageLevel::High);
    engine
        .world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .hp = 5;
    engine.world.scheduler.disarm(Routine::Slow);
    engine.world.scheduler.arm(1, Routine::Slow);
    (engine.acking_lines(), rx, id)
}

/// The session's outputs, in order, up to `Stopped`.
fn session_outputs(rx: &std::sync::mpsc::Receiver<EngineOutput>, id: SessionId) -> Vec<String> {
    let mut seen = Vec::new();
    for out in common::drain(rx) {
        match out {
            EngineOutput::LineStarted { id: i } if i == id => seen.push("LineStarted".into()),
            EngineOutput::LineDone { id: i } if i == id => seen.push("LineDone".into()),
            EngineOutput::Write { id: i, bytes } if i == id => {
                seen.push(String::from_utf8_lossy(&bytes).into_owned());
            }
            _ => {}
        }
    }
    seen
}

#[test]
fn a_line_queued_with_a_due_tick_runs_before_the_tick() {
    let (engine, rx, id) = wounded();
    let (tx, inputs) = common::channels();
    // the tick fell due first; the line completed at the same moment
    tx.send(EngineInput::Tick).unwrap();
    tx.send(EngineInput::Line {
        id,
        text: "xyzzy".into(),
    })
    .unwrap();
    drop(tx);
    let world = engine.run(inputs);
    assert!(world.cycles.slow_runs >= 1, "the tick ran");
    let seen = session_outputs(&rx, id);
    // the transport sends the held CR echo at `LineStarted`; the command's output follows it
    // directly, and the tick's redraw comes after the line is done
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert_eq!(seen[0], "LineStarted");
    assert!(seen[1].contains("Your command had no effect."), "{seen:?}");
    assert_eq!(seen[2], "LineDone");
    assert!(seen[3].contains("[HP="), "the tick's redraw: {seen:?}");
}

#[test]
fn a_tick_with_nothing_queued_behind_it_runs_at_once() {
    let (engine, rx, id) = wounded();
    let (tx, inputs) = common::channels();
    tx.send(EngineInput::Tick).unwrap();
    // a line that completes after the tick ran (a tick between typed characters)
    let (late_tx, late) = std::sync::mpsc::channel();
    let feeder = std::thread::spawn(move || {
        late.recv().unwrap();
        tx.send(EngineInput::Line {
            id,
            text: "xyzzy".into(),
        })
        .unwrap();
    });
    let runner = std::thread::spawn(move || engine.run(inputs));
    // wait for the tick's redraw before the line is sent
    let first = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(
        matches!(&first, EngineOutput::Write { id: i, bytes } if *i == id && String::from_utf8_lossy(bytes).contains("[HP=")),
        "{first:?}"
    );
    late_tx.send(()).unwrap();
    feeder.join().unwrap();
    runner.join().unwrap();
    let seen = session_outputs(&rx, id);
    assert_eq!(
        seen.first().map(String::as_str),
        Some("LineStarted"),
        "{seen:?}"
    );
}

#[test]
fn only_typed_input_goes_ahead_of_a_tick() {
    let (engine, _rx, _id) = wounded();
    let (tx, inputs) = common::channels();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let cell = seen.clone();
    tx.send(EngineInput::Tick).unwrap();
    // a barrier sent after the tick (the tests' `on_world`) still runs after it
    tx.send(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| *cell.lock().unwrap() = Some(w.cycles.slow_runs)),
    })
    .unwrap();
    drop(tx);
    engine.run(inputs);
    assert_eq!(*seen.lock().unwrap(), Some(1));
}
