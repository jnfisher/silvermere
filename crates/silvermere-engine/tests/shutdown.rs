//! Graceful shutdown (T-059): the notice, the saves, the barrier, the closes, in order.

use crate::common;

use std::time::Duration;

use silvermere_engine::{
    AccountRef, EngineControl, EngineInput, EngineOutput, PersistRequest, ShutdownReport,
    shutdown::NOTICE,
};
use silvermere_types::enums::MessageLevel;
use silvermere_types::flags::PlayerStatus;
use silvermere_types::ids::AccountId;

fn bind_account(world: &mut silvermere_engine::World, id: silvermere_engine::SessionId, n: i64) {
    world.session_mut(id).unwrap().created = true;
    world
        .session_mut(id)
        .unwrap()
        .character
        .as_mut()
        .unwrap()
        .settings |= silvermere_types::flags::PlayerSettings::VALIDATED;
    world.session_mut(id).unwrap().account = Some(AccountRef {
        id: AccountId(n),
        username: format!("acct{n}"),
        sysop: false,
        gameop: false,
        sex: b'M',
    });
}

#[test]
fn shutdown_tells_saves_flushes_and_closes_without_the_hangup_penalty() {
    let (mut engine, _clock, rx) = common::engine(4);
    let a = common::enter(&mut engine.world, "Ann", 2140, MessageLevel::High);
    let b = common::enter(&mut engine.world, "Bob", 2140, MessageLevel::High);
    bind_account(&mut engine.world, a, 1);
    bind_account(&mut engine.world, b, 2);
    let (reply, report_rx) = std::sync::mpsc::channel();
    engine.handle(EngineInput::Control(EngineControl::Shutdown {
        grace: Duration::from_secs(10),
        reply,
    }));
    assert_eq!(
        report_rx.try_recv().unwrap(),
        ShutdownReport {
            notified: 2,
            saved: 2,
            closed: 2
        }
    );
    let out = common::drain(&rx);
    // Both sessions got the notice, in their last write.
    for id in [a, b] {
        let wrote = out.iter().any(|o| {
            matches!(o, EngineOutput::Write { id: i, bytes }
                if *i == id && String::from_utf8_lossy(bytes).contains(NOTICE))
        });
        assert!(wrote, "{id:?} was told");
    }
    // Two saves precede the barrier, which precedes the closes.
    let pos = |pred: &dyn Fn(&EngineOutput) -> bool| out.iter().position(pred).unwrap();
    let saves: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o, EngineOutput::Persist(PersistRequest::Character { .. })))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(saves.len(), 2);
    let barrier = pos(&|o| matches!(o, EngineOutput::Persist(PersistRequest::Flush)));
    assert!(saves.iter().all(|&i| i < barrier), "saves before barrier");
    // The sessions' last writes and closes go out in the event's first drain; the
    // `lofrou` saves and then the barrier follow (the process waits for the barrier).
    let last_close = out
        .iter()
        .rposition(|o| matches!(o, EngineOutput::Close { .. }))
        .unwrap();
    assert!(last_close < saves[0], "closes precede the logoff saves");
    assert_eq!(
        out.iter()
            .filter(|o| matches!(o, EngineOutput::Close { .. }))
            .count(),
        2
    );
    // Saved as a clean logoff: no hang-up mark on the characters that were saved.
    for o in &out {
        if let EngineOutput::Persist(PersistRequest::Character { character, .. }) = o {
            assert!(!character.status.contains(PlayerStatus::HUNGUP));
        }
    }
    assert!(engine.world.sessions.is_empty(), "sessions removed");
}
