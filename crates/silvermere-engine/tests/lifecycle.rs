//! The connect and logoff boundaries of one event: what the transport sees, in order.

use crate::common;

use std::time::Instant;

use silvermere_engine::player_session::Caps;
use silvermere_engine::{
    ConnId, EngineInput, EngineOutput, GameEvent, SessionId, session, tell_user,
};
use silvermere_text::Msg;
use silvermere_types::enums::MessageLevel;

// The original's logoff tells the player his lines, then closes the channel.
#[test]
fn a_line_queued_before_logoff_arrives_before_close() {
    let (mut engine, _clock, rx) = common::engine(4);
    let a = common::enter(&mut engine.world, "Ann", 2140, MessageLevel::High);
    let b = common::enter(&mut engine.world, "Bob", 2140, MessageLevel::High);
    engine.handle(EngineInput::Custom {
        session: Some(a),
        run: Box::new(move |w| {
            tell_user(w, a, Msg::GameFull);
            w.logoff(a);
            tell_user(w, b, Msg::TournamentNotice);
        }),
    });
    let out = common::drain(&rx);
    let p = Default::default();
    assert_eq!(
        out,
        [
            EngineOutput::Write {
                id: a,
                bytes: silvermere_text::render(&Msg::GameFull, p).bytes
            },
            EngineOutput::Write {
                id: b,
                bytes: silvermere_text::render(&Msg::TournamentNotice, p).bytes
            },
            EngineOutput::Close { id: a },
            EngineOutput::Event(GameEvent::Closed { id: a }),
        ]
    );
    assert!(engine.world.session(a).is_none(), "removed after the flush");
    assert!(
        !engine.world.ids.is_allocated(a),
        "id freed after the flush"
    );
    assert_eq!(engine.world.ids.alloc().unwrap(), a, "and reusable");
}

#[test]
fn opened_precedes_the_first_write_of_the_same_event() {
    let (mut engine, clock, rx) = common::engine(4);
    let now = Instant::now();
    engine.handle(EngineInput::Custom {
        session: None,
        run: Box::new(move |w| {
            // What the Connected arm does, plus the login prompt T-021 will queue.
            let id = session::on_connect(w, Caps::default(), now).unwrap();
            w.opened(ConnId(9), id);
            w.session_mut(id)
                .unwrap()
                .output
                .push_raw(b"Username: ".to_vec());
        }),
    });
    let out = common::drain(&rx);
    assert_eq!(
        out,
        [
            EngineOutput::Opened {
                conn: ConnId(9),
                id: SessionId(0)
            },
            EngineOutput::Write {
                id: SessionId(0),
                bytes: b"Username: ".to_vec()
            },
            EngineOutput::Event(GameEvent::Connected { id: SessionId(0) }),
        ]
    );
    // The real input takes the same path.
    engine.handle(EngineInput::Connected {
        conn: ConnId(10),
        caps: Caps::default(),
    });
    let out = common::drain(&rx);
    assert_eq!(
        out[0],
        EngineOutput::Opened {
            conn: ConnId(10),
            id: SessionId(1)
        }
    );
    let _ = clock;
}

#[test]
fn a_full_server_refuses_before_anything_else() {
    let (mut engine, _clock, rx) = common::engine(1);
    engine.handle(EngineInput::Connected {
        conn: ConnId(1),
        caps: Caps::default(),
    });
    engine.handle(EngineInput::Connected {
        conn: ConnId(2),
        caps: Caps::default(),
    });
    let out = common::drain(&rx);
    assert_eq!(out[2], EngineOutput::Refused { conn: ConnId(2) });
    assert_eq!(engine.world.sessions.len(), 1);
}
