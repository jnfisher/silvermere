//! The forget list: first names a player will not hear telepaths or invitations from. Per
//! session, in memory, never saved; emptied when the player leaves the game and when the
//! channel hangs up (`session.rs` and `state::login::leave` do it with
//! `ust.forgets.clear()`).

use silvermere_text::Msg;
use silvermere_types::ids::SessionId;
use silvermere_types::records::ForgetEntry;

use crate::commands::{CommandLine, Handled};
use crate::tell::tell_user;
use crate::world::World;

/// The name field's size; a name must be shorter.
pub const FGTNAMSIZ: usize = 11;
/// Entries always allowed per list.
pub const FGTMINMAX: usize = 20;

/// `FORGET` alone lists the forget list; `FORGET <name>` toggles the rest of the line
/// (spaces included) on or off it. A name of `FGTNAMSIZ` bytes or more is not a command (0:
/// the line goes on to speech).
pub fn cmd_forget(world: &mut World, id: SessionId, line: &CommandLine) -> Handled {
    if line.argc() == 1 {
        show_forget_list(world, id);
        return Handled::Done;
    }
    let name = line.rest_from(1);
    if name.len() < FGTNAMSIZ {
        let name = name.to_string();
        toggle_forget_user(world, id, &name);
        return Handled::Done;
    }
    Handled::NotCommand
}

/// The heading and one name per line, newest first, or the "empty" line; one send to the player.
pub fn show_forget_list(world: &mut World, id: SessionId) {
    let Some(names) = world.session(id).map(|s| {
        s.ust
            .forgets
            .iter()
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    if names.is_empty() {
        tell_user(world, id, Msg::ForgetListEmpty);
        return;
    }
    tell_user(world, id, Msg::ForgetListHead);
    for name in names {
        tell_user(world, id, Msg::ForgetListName { name });
    }
}

/// A name already on the list (case not counting) comes off it; otherwise it goes on at the
/// front, unless the list holds `max(nterms, FGTMINMAX)` names already. The name is kept as
/// typed and never checked against the players.
pub fn toggle_forget_user(world: &mut World, id: SessionId, name: &str) {
    let max = usize::from(world.ids.max()).max(FGTMINMAX);
    let Some(s) = world.session_mut(id) else {
        return;
    };
    let list = &mut s.ust.forgets;
    if let Some(i) = list.iter().position(|f| f.name.eq_ignore_ascii_case(name)) {
        list.remove(i);
        s.output.push(Msg::ForgetRemoved {
            name: name.to_string(),
        });
        return;
    }
    if max <= list.len() {
        s.output.push(Msg::ForgetTooLong);
    } else {
        list.insert(
            0,
            ForgetEntry {
                name: name.to_string(),
            },
        );
        s.output.push(Msg::ForgetAdded {
            name: name.to_string(),
        });
    }
}

/// Has `listener` forgotten `other`'s first name? `false` when `other` has no character.
pub fn is_user_forgotten(world: &World, listener: SessionId, other: SessionId) -> bool {
    let Some(name) = world
        .session(other)
        .and_then(|s| s.player())
        .map(|p| p.firstname.as_str())
    else {
        return false;
    };
    world.session(listener).is_some_and(|s| {
        s.ust
            .forgets
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(name))
    })
}
