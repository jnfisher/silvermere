//! Telnet CHARSET (RFC 2066): keeping the client's idea of the text's character set in step
//! with what the session sends.
//!
//! The transport asks every client that agrees to CHARSET to read `IBM437` (CP437) before the
//! first screen, so Mudlet and the like switch by themselves. The login dialogue is always
//! CP437; the game is UTF-8 when the session has `caps.utf8` (`SET UTF8 ON`,
//! `compat.charset = "utf8"`). [`sync`] asks again whenever that changes: entering from the
//! login, `SET UTF8`, going back to the login. A client that refused CHARSET (plain telnet,
//! MegaMud) is never asked anything, and its bytes are what they always were.

use silvermere_types::ids::SessionId;

use crate::event_loop::EngineOutput;
use crate::world::World;

/// The CP437 name the client is asked for.
pub const CP437: &str = "IBM437";
/// The UTF-8 name.
pub const UTF8: &str = "UTF-8";

/// The client accepted `name`, or refused the request (`None`).
pub fn on_accepted(world: &mut World, id: SessionId, name: Option<String>) {
    let Some(s) = world.session_mut(id) else {
        return;
    };
    s.charset = name.map(|n| n.to_ascii_uppercase());
    // logged in already (a slow answer): bring it in step with the game text
    if s.account.is_some() {
        let utf8 = s.caps.utf8;
        sync(world, id, utf8);
    }
}

/// Asks the client to read UTF-8 (`utf8`) or CP437 from now on, if it agreed to CHARSET and
/// reads the other one now. Goes out ahead of the event's text.
pub fn sync(world: &mut World, id: SessionId, utf8: bool) {
    let want = if utf8 { UTF8 } else { CP437 };
    let Some(s) = world.session_mut(id) else {
        return;
    };
    match &s.charset {
        Some(now) if now != want => {}
        _ => return,
    }
    s.charset = Some(want.to_owned());
    world.outbox.push(EngineOutput::Charset {
        id,
        name: want.to_owned(),
    });
}
