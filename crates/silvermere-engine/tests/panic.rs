//! A panicking handler interrupts one session's command and nothing else.

mod common;

use silvermere_engine::{EngineInput, EngineOutput, tell_user};
use silvermere_text::Msg;
use silvermere_types::enums::MessageLevel;

#[test]
fn a_panic_sends_command_interrupted_and_the_loop_goes_on() {
    // Keep the expected panic's report out of the test output; anything else prints.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let expected = info
            .payload()
            .downcast_ref::<&str>()
            .is_some_and(|m| *m == "handler bug");
        if !expected {
            default_hook(info);
        }
    }));
    let (mut engine, _clock, rx) = common::engine(4);
    let a = common::enter(&mut engine.world, "Ann", 2140, MessageLevel::High);
    let b = common::enter(&mut engine.world, "Bob", 2140, MessageLevel::High);
    engine.handle(EngineInput::Custom {
        session: Some(a),
        run: Box::new(move |w| {
            // Output queued before the panic is dropped with it.
            tell_user(w, a, Msg::GameFull);
            tell_user(w, b, Msg::GameFull);
            panic!("handler bug");
        }),
    });
    let out = common::drain(&rx);
    // Ann's queued line is replaced by the interruption; Bob's queued line still goes
    // out, since the world is left as the handler left it.
    assert_eq!(
        out,
        [
            EngineOutput::Write {
                id: a,
                bytes: b"Command interrupted.\r\n".to_vec()
            },
            EngineOutput::Write {
                id: b,
                bytes: silvermere_text::render(&Msg::GameFull, Default::default()).bytes
            }
        ]
    );
    // The next event is handled normally.
    engine.handle(EngineInput::Custom {
        session: Some(b),
        run: Box::new(move |w| tell_user(w, b, Msg::TournamentNotice)),
    });
    let out = common::drain(&rx);
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], EngineOutput::Write { id, .. } if *id == b));
    assert_eq!(engine.world.sessions.len(), 2);
}
