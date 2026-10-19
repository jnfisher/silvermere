//! One event, one write per touched session, in emission order.

use crate::common;

use silvermere_engine::{EngineInput, EngineOutput, tell_user};
use silvermere_text::colour::Palette;
use silvermere_text::{Msg, render};
use silvermere_types::enums::MessageLevel;

#[test]
fn three_to_a_and_one_to_b_yield_two_writes_in_order() {
    let (mut engine, _clock, rx) = common::engine(4);
    let a = common::enter(&mut engine.world, "Ann", 2140, MessageLevel::High);
    let b = common::enter(&mut engine.world, "Bob", 2140, MessageLevel::High);
    engine.handle(EngineInput::Custom {
        session: Some(a),
        run: Box::new(move |w| {
            tell_user(w, a, Msg::TournamentNotice);
            tell_user(w, b, Msg::GameFull);
            tell_user(w, a, Msg::GameFull);
            tell_user(w, a, Msg::CommandInterrupted);
        }),
    });
    let out = common::drain(&rx);
    assert_eq!(out.len(), 2, "{out:?}");
    let p = Palette::new(0).unwrap();
    let a_bytes = [
        render(&Msg::TournamentNotice, p).bytes,
        render(&Msg::GameFull, p).bytes,
        render(&Msg::CommandInterrupted, p).bytes,
    ]
    .concat();
    assert_eq!(
        out[0],
        EngineOutput::Write {
            id: a,
            bytes: a_bytes
        }
    );
    assert_eq!(
        out[1],
        EngineOutput::Write {
            id: b,
            bytes: render(&Msg::GameFull, p).bytes
        }
    );
    // Nothing is left behind for the next event.
    engine.handle(EngineInput::Tick);
    assert!(common::drain(&rx).is_empty());
}

#[test]
fn a_reprompt_appends_the_sessions_prompt_last() {
    let (mut engine, _clock, rx) = common::engine(4);
    let a = common::enter(&mut engine.world, "Ann", 2140, MessageLevel::High);
    engine.handle(EngineInput::Custom {
        session: Some(a),
        run: Box::new(move |w| {
            let s = w.session_mut(a).unwrap();
            s.output.push(Msg::GameFull);
            s.output.reprompt = true;
        }),
    });
    let prompt = silvermere_engine::tell::prompt_bytes(&engine.world, a);
    assert!(!prompt.is_empty());
    let out = common::drain(&rx);
    let EngineOutput::Write { bytes, .. } = &out[0] else {
        panic!("{out:?}");
    };
    assert!(bytes.ends_with(&prompt));
}
